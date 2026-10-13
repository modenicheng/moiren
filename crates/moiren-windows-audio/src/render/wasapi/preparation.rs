//! COM preparation is independent of graph/renderer ownership and activation.
use super::{NativeOwnerExit, PreparedRenderSession, RenderSession, api, handle, stream};
use crate::{
    ActivationGate, StopSignal,
    latency::{NativeStreamTiming, PreparedRenderQueue, RenderQueuePolicy, prepare_render_queue},
    preparation,
    render::{DemandRenderer, RenderError, RenderOptions},
};
use std::{
    os::windows::io::OwnedHandle,
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
    pub info: NativeRenderInfo,
    session: RenderSession,
    gate: ActivationGate,
    renderer: preparation::Sender<(DemandRenderer, PreparedRenderQueue)>,
    attached: mpsc::Receiver<Result<(), RenderError>>,
}
impl PreparedNativeRender {
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
            run_owner(
                options,
                worker_stop,
                worker_gate,
                worker_started,
                ready,
                attached,
                receiver,
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
fn run_owner(
    options: RenderOptions,
    stop: Arc<OwnedHandle>,
    gate: ActivationGate,
    started: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<NativeRenderInfo, RenderError>>,
    attached: mpsc::SyncSender<Result<(), RenderError>>,
    receiver: preparation::Receiver<(DemandRenderer, PreparedRenderQueue)>,
) -> NativeOwnerExit {
    let mut report = stream::new_report(&options);
    let mut renderer = None;
    let mut initial_frame = 0;
    let result = stream::with_native(&options, &stop, &mut report, |native, report| {
        ready
            .send(Ok(NativeRenderInfo {
                timing: native.timing(report),
                shared_period: report.shared_period,
            }))
            .map_err(|_| RenderError::StartupLost)?;
        let (prepared, queue) =
            api("Wait for renderer", receiver.receive(&stop))?.ok_or(RenderError::Cancelled)?;
        initial_frame = prepared.timeline();
        renderer = Some(prepared);
        report.queue = Some(queue);
        stream::run_stream(
            native,
            renderer.as_mut().expect("installed renderer"),
            report,
            stream::StreamContext {
                options: &options,
                stop: &stop,
                gate: &gate,
                acknowledged: &started,
            },
            || attached.send(Ok(())).map_err(|_| RenderError::StartupLost),
        )
    });
    let pending = receiver.close_and_take();
    if renderer.is_none() {
        renderer = pending.map(|(renderer, _)| renderer);
    }
    let _ = unsafe { SetEvent(handle(&stop)) };
    match result {
        Ok(status) => report.status = status,
        Err(error) => {
            report.failure = Some(error.to_string());
            let _ = ready.try_send(Err(error));
            let _ = attached.try_send(Err(error));
        }
    }
    report.stats.processed_frames = renderer.as_ref().map_or(0, |renderer| {
        renderer.timeline().saturating_sub(initial_frame)
    });
    NativeOwnerExit { report, renderer }
}
