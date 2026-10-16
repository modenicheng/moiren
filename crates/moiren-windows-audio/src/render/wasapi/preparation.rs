//! COM preparation is independent of graph/renderer ownership and activation.
use super::{NativeOwnerExit, PreparedRenderSession, RenderSession, api, handle, stream};
use crate::{
    ActivationGate, StopSignal,
    latency::{NativeStreamTiming, PreparedRenderQueue, RenderQueuePolicy, prepare_render_queue},
    preparation,
    render::{
        DemandRenderer, FollowerRenderer, NativeRenderer, RenderError, RenderOptions, RenderReport,
        RenderStatus, SampleRenderer,
    },
};
use std::{
    os::windows::io::OwnedHandle,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
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
    renderer: preparation::Sender<(RendererHandoff, PreparedRenderQueue)>,
    attached: mpsc::Receiver<Result<(), RenderError>>,
}
type RendererHandoff = Arc<Mutex<Option<NativeRenderer>>>;

pub struct RenderAttachFailure {
    pub error: RenderError,
    pub renderer: DemandRenderer,
    pub report: Option<RenderReport>,
}

struct NativeAttachFailure {
    error: RenderError,
    renderer: NativeRenderer,
    report: Option<RenderReport>,
}
pub(crate) struct FollowerAttachFailure {
    pub error: RenderError,
    pub report: Option<RenderReport>,
}

impl PreparedNativeRender {
    pub fn info(&self) -> NativeRenderInfo {
        self.info
    }
    pub fn cancel_with_report(self) -> Result<RenderReport, RenderError> {
        let _ = self.session.request_stop();
        self.session.join_owner().map(|exit| exit.report)
    }
    pub fn attach(
        self,
        renderer: DemandRenderer,
        queue: RenderQueuePolicy,
    ) -> Result<PreparedRenderSession, RenderError> {
        self.attach_returning(renderer, queue)
            .map_err(|failure| failure.error)
    }

    /// A failed handoff joins the native owner before returning the renderer.
    pub fn attach_returning(
        self,
        renderer: DemandRenderer,
        queue: RenderQueuePolicy,
    ) -> Result<PreparedRenderSession, Box<RenderAttachFailure>> {
        self.attach_source(NativeRenderer::Master(Box::new(renderer)), queue)
            .map_err(|failure| {
                Box::new(RenderAttachFailure {
                    error: failure.error,
                    renderer: failure.renderer.into_master().expect("master handoff"),
                    report: failure.report,
                })
            })
    }
    pub(crate) fn attach_follower(
        self,
        source: crate::clock_bridge::ClockSource,
        queue: RenderQueuePolicy,
    ) -> Result<PreparedRenderSession, Box<FollowerAttachFailure>> {
        self.attach_source(
            NativeRenderer::Follower(Box::new(FollowerRenderer::new(source))),
            queue,
        )
        .map_err(|failure| {
            Box::new(FollowerAttachFailure {
                error: failure.error,
                report: failure.report,
            })
        })
    }
    fn attach_source(
        self,
        renderer: NativeRenderer,
        queue: RenderQueuePolicy,
    ) -> Result<PreparedRenderSession, Box<NativeAttachFailure>> {
        let queue = match prepare_render_queue(self.info.timing, queue) {
            Ok(queue) => queue,
            Err(error) => {
                let report = self.cancel_with_report().ok();
                return Err(Box::new(NativeAttachFailure {
                    error: error.into(),
                    renderer,
                    report,
                }));
            }
        };
        let retained = self
            .session
            .recovery
            .as_ref()
            .expect("native recovery mailbox")
            .clone();
        *retained.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(renderer);
        let sent = api(
            "Send prepared renderer",
            self.renderer.send((retained.clone(), queue)),
        );
        let result =
            sent.and_then(|()| self.attached.recv().map_err(|_| RenderError::StartupLost)?);
        if let Err(error) = result {
            let _ = self.session.request_stop();
            let exit = self.session.join_owner();
            let (renderer, report) = match exit {
                Ok(exit) => (exit.renderer, Some(exit.report)),
                Err(_) => (None, None),
            };
            let renderer = renderer
                .or_else(|| {
                    retained
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take()
                })
                .expect("native owner retains renderer outside its unwind scope");
            return Err(Box::new(NativeAttachFailure {
                error,
                renderer,
                report,
            }));
        }
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
    let fallback = stream::new_report(&options);
    start_native_owner(
        stop,
        gate,
        fallback,
        move |stop, gate, started, preparation| {
            run_owner(options, stop, gate, started, preparation)
        },
    )
}
fn start_native_owner(
    stop: StopSignal,
    gate: ActivationGate,
    fallback: RenderReport,
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
    let recovery = Arc::new(Mutex::new(None));
    let worker_recovery = recovery.clone();
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
                    recovery: worker_recovery,
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
        recovery: Some(recovery),
        fallback: Some(fallback),
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
    receiver: preparation::Receiver<(RendererHandoff, PreparedRenderQueue)>,
    renderer: Option<NativeRenderer>,
    initial_frame: u64,
    recovery: RendererHandoff,
}
impl Drop for RenderPreparation {
    fn drop(&mut self) {
        // Even an unexpected owner bookkeeping panic leaves the engine in the
        // control-side recovery mailbox, after the borrowed native loop unwinds.
        if let Some(renderer) = self.renderer.take() {
            *self
                .recovery
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(renderer);
        }
    }
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
        let (handoff, queue) =
            api("Wait for renderer", self.receiver.receive(stop))?.ok_or(RenderError::Cancelled)?;
        let renderer = handoff
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .ok_or(RenderError::StartupLost)?;
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
    // Keep the Engine and report-owned diagnostic allocation outside the unwind
    // scope so the cleanup joiner reclaims both after COM has unwound.
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(&stop, &gate, &started, &mut preparation, &mut report)
    }))
    .unwrap_or(Err(RenderError::WorkerPanicked));
    report.finish_diagnostics();
    let pending = preparation.receiver.close_and_take();
    if let Some((handoff, _)) = pending {
        let renderer = handoff
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if let Some(renderer) = renderer {
            preparation.initial_frame = renderer.timeline();
            preparation.renderer = Some(renderer);
        }
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
        renderer: None,
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::{
    Outcome as TestOutcome, fake_prepared as prepare_test_native,
    fake_prepared_with_info as prepare_test_native_with_info,
};
