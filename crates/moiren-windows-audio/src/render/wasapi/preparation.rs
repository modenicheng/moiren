//! COM preparation is independent of graph/renderer ownership and activation.
use super::{NativeOwnerExit, PreparedRenderSession, RenderSession, api, handle, stream};
use crate::{
    ActivationGate, StopSignal,
    latency::{NativeStreamTiming, PreparedRenderQueue, RenderQueuePolicy, prepare_render_queue},
    preparation,
    render::{DemandRenderer, RenderError, RenderOptions, RenderReport, RenderStatus},
};
use std::{
    os::windows::io::OwnedHandle,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
};
use windows::Win32::System::Threading::SetEvent;

#[derive(Debug, Clone, Copy)]
pub struct NativeRenderInfo {
    pub timing: NativeStreamTiming,
    pub shared_period: crate::shared_stream::SharedPeriodReport,
}
pub struct PreparedNativeRender {
    info: NativeRenderInfo,
    session: RenderSession,
    gate: ActivationGate,
    renderer: preparation::Sender<(DemandRenderer, PreparedRenderQueue)>,
    attached: mpsc::Receiver<Result<(), RenderError>>,
}
impl PreparedNativeRender {
    pub fn info(&self) -> NativeRenderInfo {
        self.info
    }
    pub fn attach(
        self,
        renderer: DemandRenderer,
        queue: RenderQueuePolicy,
    ) -> Result<PreparedRenderSession, RenderError> {
        let queue = prepare_render_queue(self.info.timing, queue)?;
        api(
            "Send prepared renderer",
            self.renderer.send((renderer, queue)),
        )?;
        self.attached
            .recv()
            .map_err(|_| RenderError::StartupLost)??;
        Ok(PreparedRenderSession {
            session: self.session,
            gate: self.gate,
        })
    }
}
pub fn prepare_render_native_with_gate(
    options: RenderOptions,
    stop: StopSignal,
    gate: ActivationGate,
) -> Result<PreparedNativeRender, RenderError> {
    options.validate()?;
    gate.validate_stop(&stop)?;
    start_native_owner(stop, gate, move |stop, gate, started, preparation| {
        run_owner(options, stop, gate, started, preparation)
    })
}
fn start_native_owner(
    stop: StopSignal,
    gate: ActivationGate,
    owner: impl FnOnce(
        Arc<OwnedHandle>,
        ActivationGate,
        Arc<AtomicBool>,
        RenderPreparation,
    ) -> NativeOwnerExit
    + Send
    + 'static,
) -> Result<PreparedNativeRender, RenderError> {
    let (sender, receiver) = api("Create render handoff", preparation::channel())?;
    let (ready, metadata) = mpsc::sync_channel(1);
    let (attached, attachment) = mpsc::sync_channel(1);
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_stop = stop.event.clone();
    let worker_gate = gate.clone();
    let worker = thread::Builder::new()
        .name("moiren-shared-render".into())
        .spawn(move || {
            let _stop_on_exit = preparation::OwnerStop(worker_stop.clone());
            owner(
                worker_stop,
                worker_gate,
                worker_started,
                RenderPreparation {
                    ready,
                    attached,
                    receiver,
                    renderer: None,
                    initial_frame: 0,
                },
            )
        })
        .map_err(|error| RenderError::WorkerSpawn {
            code: error.raw_os_error(),
        })?;
    let session = RenderSession {
        started,
        stop: stop.event,
        worker: Some(worker),
    };
    let info = match metadata.recv() {
        Ok(result) => result?,
        Err(_) => {
            let _ = session.request_stop();
            session.join_owner()?;
            return Err(RenderError::StartupLost);
        }
    };
    Ok(PreparedNativeRender {
        info,
        session,
        gate,
        renderer: sender,
        attached: attachment,
    })
}
struct RenderPreparation {
    ready: mpsc::SyncSender<Result<NativeRenderInfo, RenderError>>,
    attached: mpsc::SyncSender<Result<(), RenderError>>,
    receiver: preparation::Receiver<(DemandRenderer, PreparedRenderQueue)>,
    renderer: Option<DemandRenderer>,
    initial_frame: u64,
}
impl RenderPreparation {
    fn receive(
        &mut self,
        info: NativeRenderInfo,
        stop: &OwnedHandle,
        report: &mut RenderReport,
    ) -> Result<(), RenderError> {
        self.ready
            .send(Ok(info))
            .map_err(|_| RenderError::StartupLost)?;
        let (renderer, queue) =
            api("Wait for renderer", self.receiver.receive(stop))?.ok_or(RenderError::Cancelled)?;
        self.initial_frame = renderer.timeline();
        self.renderer = Some(renderer);
        report.queue = Some(queue);
        Ok(())
    }
}
fn run_owner(
    options: RenderOptions,
    stop: Arc<OwnedHandle>,
    gate: ActivationGate,
    started: Arc<AtomicBool>,
    preparation: RenderPreparation,
) -> NativeOwnerExit {
    run_owner_with(
        &options,
        stop,
        gate,
        started,
        preparation,
        |stop, gate, started, preparation, report| {
            stream::with_native(&options, stop, report, |native, report| {
                preparation.receive(
                    NativeRenderInfo {
                        timing: native.timing(report),
                        shared_period: report.shared_period,
                    },
                    stop,
                    report,
                )?;
                stream::run_stream(
                    native,
                    preparation.renderer.as_mut().expect("installed renderer"),
                    report,
                    stream::StreamContext {
                        options: &options,
                        stop,
                        gate,
                        acknowledged: started,
                    },
                    || {
                        preparation
                            .attached
                            .send(Ok(()))
                            .map_err(|_| RenderError::StartupLost)
                    },
                )
            })
        },
    )
}
fn run_owner_with(
    options: &RenderOptions,
    stop: Arc<OwnedHandle>,
    gate: ActivationGate,
    started: Arc<AtomicBool>,
    mut preparation: RenderPreparation,
    run: impl FnOnce(
        &OwnedHandle,
        &ActivationGate,
        &AtomicBool,
        &mut RenderPreparation,
        &mut RenderReport,
    ) -> Result<RenderStatus, RenderError>,
) -> NativeOwnerExit {
    let mut report = stream::new_report(options);
    // Keep the Engine outside the unwind scope so native/DSP failures still
    // transfer its destruction to the cleanup joiner after COM has unwound.
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(&stop, &gate, &started, &mut preparation, &mut report)
    }))
    .unwrap_or(Err(RenderError::WorkerPanicked));
    let pending = preparation.receiver.close_and_take();
    if let Some((renderer, _)) = pending {
        preparation.initial_frame = renderer.timeline();
        preparation.renderer = Some(renderer);
    }
    let _ = unsafe { SetEvent(handle(&stop)) };
    match result {
        Ok(status) => report.status = status,
        Err(error) => {
            report.failure = Some(error.to_string());
            let _ = preparation.ready.try_send(Err(error));
            let _ = preparation.attached.try_send(Err(error));
        }
    }
    report.stats.processed_frames = preparation.renderer.as_ref().map_or(0, |renderer| {
        renderer
            .timeline()
            .saturating_sub(preparation.initial_frame)
    });
    NativeOwnerExit {
        report,
        renderer: preparation.renderer,
    }
}

#[cfg(test)]
mod tests;
