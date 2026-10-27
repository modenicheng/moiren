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
    padding: crate::render::RenderPaddingObserver,
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

#[derive(Debug)]
pub struct RenderPreparationAbortFailure<A> {
    pub error: RenderError,
    pub report: Option<RenderReport>,
    pub stop_error: Option<RenderError>,
    pub join_error: Option<RenderError>,
    pub abort: Option<A>,
    pub abort_panicked: bool,
    // Preserve the old explicit-error versus closed-channel adapter rule.
    legacy_error: RenderError,
}

pub struct RenderAttachAbortFailure<A> {
    pub error: RenderError,
    pub renderer: DemandRenderer,
    pub report: Option<RenderReport>,
    pub stop_error: Option<RenderError>,
    pub join_error: Option<RenderError>,
    pub abort: Option<A>,
    pub abort_panicked: bool,
}

struct RenderAbortGuard<F: FnOnce() -> A, A> {
    stop: StopSignal,
    session: Option<RenderSession>,
    before_join: Option<F>,
    stop_without_owner: bool,
}
struct RenderCleanup<A> {
    exit: Option<NativeOwnerExit>,
    stop_error: Option<RenderError>,
    join_error: Option<RenderError>,
    abort: Option<A>,
    abort_panicked: bool,
}
impl<F: FnOnce() -> A, A> RenderAbortGuard<F, A> {
    fn cleanup(&mut self) -> RenderCleanup<A> {
        let stop_error = if self.session.is_some() || self.stop_without_owner {
            api("SetEvent(stop)", self.stop.request_stop()).err()
        } else {
            None
        };
        // Caller borrows stay on this control thread, including during unwind.
        let aborted = catch_unwind(AssertUnwindSafe(
            self.before_join.take().expect("armed render abort"),
        ));
        let (exit, join_error) = self
            .session
            .take()
            .map(RenderSession::join_owner_returning)
            .unwrap_or((None, None));
        let (abort, abort_panicked) = match aborted {
            Ok(value) => (Some(value), false),
            Err(_) => (None, true),
        };
        RenderCleanup {
            exit,
            stop_error,
            join_error,
            abort,
            abort_panicked,
        }
    }
    fn failure(&mut self, error: RenderError) -> Box<RenderPreparationAbortFailure<A>> {
        let cleanup = self.cleanup();
        Box::new(RenderPreparationAbortFailure {
            error,
            report: cleanup.exit.map(|exit| exit.report),
            stop_error: cleanup.stop_error,
            join_error: cleanup.join_error,
            abort: cleanup.abort,
            abort_panicked: cleanup.abort_panicked,
            legacy_error: error,
        })
    }
}
impl<F: FnOnce() -> A, A> Drop for RenderAbortGuard<F, A> {
    fn drop(&mut self) {
        if self.before_join.is_some() {
            let _ = self.cleanup();
        }
    }
}

struct NativeAttachAbortFailure<A> {
    error: RenderError,
    renderer: NativeRenderer,
    report: Option<RenderReport>,
    stop_error: Option<RenderError>,
    join_error: Option<RenderError>,
    abort: Option<A>,
    abort_panicked: bool,
}
pub(crate) struct FollowerAttachFailure {
    pub error: RenderError,
    pub report: Option<RenderReport>,
}

impl PreparedNativeRender {
    pub fn info(&self) -> NativeRenderInfo {
        self.info
    }
    pub fn padding_observer(&self) -> crate::render::RenderPaddingObserver {
        self.padding.clone()
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
        self.attach_returning_with_abort(renderer, queue, || ())
            .map_err(|failure| {
                Box::new(RenderAttachFailure {
                    error: failure.error,
                    renderer: failure.renderer,
                    report: failure.report,
                })
            })
    }
    /// Abort peers before any failed attachment's local cleanup join.
    pub fn attach_returning_with_abort<A>(
        self,
        renderer: DemandRenderer,
        queue: RenderQueuePolicy,
        before_join: impl FnOnce() -> A,
    ) -> Result<PreparedRenderSession, Box<RenderAttachAbortFailure<A>>> {
        self.attach_source_with_abort(
            NativeRenderer::Master(Box::new(renderer)),
            queue,
            before_join,
        )
        .map_err(|failure| {
            Box::new(RenderAttachAbortFailure {
                error: failure.error,
                renderer: failure.renderer.into_master().expect("master handoff"),
                report: failure.report,
                stop_error: failure.stop_error,
                join_error: failure.join_error,
                abort: failure.abort,
                abort_panicked: failure.abort_panicked,
            })
        })
    }
    pub(crate) fn attach_follower(
        self,
        source: crate::clock_bridge::ClockSource,
        queue: RenderQueuePolicy,
    ) -> Result<PreparedRenderSession, Box<FollowerAttachFailure>> {
        self.attach_source_with_abort(
            NativeRenderer::Follower(Box::new(FollowerRenderer::new(source))),
            queue,
            || (),
        )
        .map_err(|failure| {
            Box::new(FollowerAttachFailure {
                error: failure.error,
                report: failure.report,
            })
        })
    }
    fn attach_source_with_abort<A>(
        self,
        renderer: NativeRenderer,
        queue: RenderQueuePolicy,
        before_join: impl FnOnce() -> A,
    ) -> Result<PreparedRenderSession, Box<NativeAttachAbortFailure<A>>> {
        let Self {
            info,
            session,
            gate,
            renderer: sender,
            attached,
            ..
        } = self;
        let mut guard = RenderAbortGuard {
            stop: StopSignal {
                event: session.stop.clone(),
            },
            session: Some(session),
            before_join: Some(before_join),
            stop_without_owner: true,
        };
        let queue = match prepare_render_queue(info.timing, queue) {
            Ok(queue) => queue,
            Err(error) => {
                let cleanup = guard.cleanup();
                return Err(Box::new(NativeAttachAbortFailure {
                    error: error.into(),
                    renderer,
                    report: cleanup.exit.map(|exit| exit.report),
                    stop_error: cleanup.stop_error,
                    join_error: cleanup.join_error,
                    abort: cleanup.abort,
                    abort_panicked: cleanup.abort_panicked,
                }));
            }
        };
        let retained = guard
            .session
            .as_ref()
            .expect("owned render session")
            .recovery
            .as_ref()
            .expect("native recovery mailbox")
            .clone();
        *retained.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(renderer);
        let sent = api(
            "Send prepared renderer",
            sender.send((retained.clone(), queue)),
        );
        let result = sent.and_then(|()| attached.recv().map_err(|_| RenderError::StartupLost)?);
        if let Err(error) = result {
            let cleanup = guard.cleanup();
            let (renderer, report) = match cleanup.exit {
                Some(exit) => (exit.renderer, Some(exit.report)),
                None => (None, None),
            };
            let renderer = renderer
                .or_else(|| {
                    retained
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take()
                })
                .expect("native owner retains renderer outside its unwind scope");
            return Err(Box::new(NativeAttachAbortFailure {
                error,
                renderer,
                report,
                stop_error: cleanup.stop_error,
                join_error: cleanup.join_error,
                abort: cleanup.abort,
                abort_panicked: cleanup.abort_panicked,
            }));
        }
        let session = guard.session.take().expect("owned render session");
        guard.before_join = None;
        Ok(PreparedRenderSession { session, gate })
    }
}
pub fn prepare_render_native_with_gate(
    options: RenderOptions,
    stop: StopSignal,
    gate: ActivationGate,
) -> Result<PreparedNativeRender, RenderError> {
    options.validate()?;
    gate.validate_stop(&stop)?;
    prepare_render_native_with_abort_policy(options, stop, gate, || (), false)
        .map_err(|failure| failure.legacy_error)
}
/// Retain native preparation evidence while aborting peers before local reaping.
pub fn prepare_render_native_with_gate_returning_with_abort<A>(
    options: RenderOptions,
    stop: StopSignal,
    gate: ActivationGate,
    before_join: impl FnOnce() -> A,
) -> Result<PreparedNativeRender, Box<RenderPreparationAbortFailure<A>>> {
    prepare_render_native_with_abort_policy(options, stop, gate, before_join, true)
}
fn prepare_render_native_with_abort_policy<A>(
    options: RenderOptions,
    stop: StopSignal,
    gate: ActivationGate,
    before_join: impl FnOnce() -> A,
    stop_without_owner: bool,
) -> Result<PreparedNativeRender, Box<RenderPreparationAbortFailure<A>>> {
    if let Err(error) = options
        .validate()
        .and_then(|()| gate.validate_stop(&stop).map_err(Into::into))
    {
        return Err(RenderAbortGuard {
            stop,
            session: None,
            before_join: Some(before_join),
            stop_without_owner: true,
        }
        .failure(error));
    }
    let fallback = stream::new_report(&options);
    let owner = move |stop, gate, started, preparation| {
        run_owner(options, stop, gate, started, preparation)
    };
    if stop_without_owner {
        start_native_owner_returning_with_abort(stop, gate, fallback, owner, before_join)
    } else {
        start_native_owner_returning_with_abort_policy(
            stop,
            gate,
            fallback,
            owner,
            before_join,
            false,
        )
    }
}
#[cfg(test)]
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
    start_native_owner_returning_with_abort_policy(stop, gate, fallback, owner, || (), false)
        .map_err(|failure| failure.legacy_error)
}
fn start_native_owner_returning_with_abort<A>(
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
    before_join: impl FnOnce() -> A,
) -> Result<PreparedNativeRender, Box<RenderPreparationAbortFailure<A>>> {
    start_native_owner_returning_with_abort_policy(stop, gate, fallback, owner, before_join, true)
}
fn start_native_owner_returning_with_abort_policy<A>(
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
    before_join: impl FnOnce() -> A,
    stop_without_owner: bool,
) -> Result<PreparedNativeRender, Box<RenderPreparationAbortFailure<A>>> {
    start_native_owner_with_spawn(
        stop,
        gate,
        fallback,
        owner,
        before_join,
        stop_without_owner,
        |owner| {
            thread::Builder::new()
                .name("moiren-shared-render".into())
                .spawn(owner)
        },
    )
}
fn start_native_owner_with_spawn<A>(
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
    before_join: impl FnOnce() -> A,
    stop_without_owner: bool,
    spawn: impl FnOnce(
        Box<dyn FnOnce() -> NativeOwnerExit + Send>,
    ) -> std::io::Result<thread::JoinHandle<NativeOwnerExit>>,
) -> Result<PreparedNativeRender, Box<RenderPreparationAbortFailure<A>>> {
    let mut guard = RenderAbortGuard {
        stop: stop.clone(),
        session: None,
        before_join: Some(before_join),
        stop_without_owner,
    };
    let (sender, receiver) = api("Create render handoff", preparation::channel())
        .map_err(|error| guard.failure(error))?;
    let (ready, metadata) = mpsc::sync_channel(1);
    let (attached, attachment) = mpsc::sync_channel(1);
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_stop = stop.event.clone();
    let worker_gate = gate.clone();
    let recovery = Arc::new(Mutex::new(None));
    let worker_recovery = recovery.clone();
    let padding = crate::render::RenderPaddingObserver::new();
    let padding_owner = padding.new_owner();
    let worker = spawn(Box::new(move || {
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
                padding: Some(padding_owner),
            },
        )
    }))
    .map_err(|error| {
        guard.failure(RenderError::WorkerSpawn {
            code: error.raw_os_error(),
        })
    })?;
    guard.session = Some(RenderSession {
        started,
        stop: stop.event,
        worker: Some(worker),
        recovery: Some(recovery),
        fallback: Some(fallback),
        padding: Some(padding.clone()),
    });
    let info = match metadata.recv() {
        Ok(Ok(info)) => info,
        Ok(Err(error)) => return Err(guard.failure(error)),
        Err(_) => {
            let mut failure = guard.failure(RenderError::StartupLost);
            if failure.report.is_none() {
                failure.legacy_error = failure.join_error.unwrap_or(failure.error);
            }
            return Err(failure);
        }
    };
    let session = guard.session.take().expect("owned render session");
    guard.before_join = None;
    Ok(PreparedNativeRender {
        info,
        padding,
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
    padding: Option<crate::render::RenderPaddingOwner>,
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
                        padding: preparation.padding.as_mut(),
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
    if let Some(padding) = preparation.padding.as_mut() {
        padding.invalidate();
    }
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
        _padding: preparation.padding.take(),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::{
    Outcome as TestOutcome, fake_prepared as prepare_test_native,
    fake_prepared_with_info as prepare_test_native_with_info,
};
