//! Only kernel handles, prepared ring endpoints and scalars cross owner threads.
use super::{CaptureError, CaptureReport, CaptureStatus};
#[cfg(test)]
use crate::ActivationGate;
use crate::StopSignal;
use crate::clock_bridge::{BridgeObserver, ClockSource};
use crate::owner::{WaitOutcome, wait_for_handles};
use std::{
    os::windows::io::{AsRawHandle, OwnedHandle as StdHandle},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};
use windows::Win32::{Foundation::HANDLE, System::Threading::SetEvent};

mod packet;
pub(crate) mod preparation;
pub(crate) mod stream;
#[cfg(test)]
mod tests;

pub(crate) fn api<T>(
    stage: &'static str,
    result: windows::core::Result<T>,
) -> Result<T, CaptureError> {
    crate::negotiation::stage(stage, result).map_err(CaptureError::from)
}

impl From<crate::negotiation::StageError> for CaptureError {
    fn from(failure: crate::negotiation::StageError) -> Self {
        CaptureError::Api {
            stage: failure.stage,
            hresult: failure.hresult,
        }
    }
}

pub(crate) fn handle(value: &StdHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wake {
    Stop,
    TargetExited,
    Audio,
    Timeout,
}
#[cfg(test)]
fn wait(stop: HANDLE, audio: HANDLE, timeout_ms: u32) -> Result<Wake, CaptureError> {
    wait_target(stop, audio, None, timeout_ms)
}
pub(crate) fn wait_target(
    stop: HANDLE,
    audio: HANDLE,
    target: Option<HANDLE>,
    timeout_ms: u32,
) -> Result<Wake, CaptureError> {
    // Lowest signaled index wins. Stop must also work without any audio wake.
    let handles = [stop, target.unwrap_or(audio), audio];
    let handles = if target.is_some() {
        &handles[..]
    } else {
        &handles[..2]
    };
    match wait_for_handles(handles, timeout_ms).map_err(|error| CaptureError::Api {
        stage: "WaitForMultipleObjects(capture)",
        hresult: error.code().0,
    })? {
        WaitOutcome::Signaled(0) => Ok(Wake::Stop),
        WaitOutcome::Signaled(1) if target.is_some() => Ok(Wake::TargetExited),
        WaitOutcome::Signaled(_) => Ok(Wake::Audio),
        WaitOutcome::Timeout => Ok(Wake::Timeout),
    }
}

pub struct CaptureSession {
    started: Arc<AtomicBool>,
    stop: Arc<StdHandle>,
    worker: Option<JoinHandle<CaptureReport>>,
}
impl CaptureSession {
    /// True only after the native client successfully starts.
    pub fn has_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
    /// Link a render owner to this stream before starting output. Both observe
    /// cancellation immediately, independently of final COM cleanup/join.
    pub fn stop_signal(&self) -> StopSignal {
        StopSignal {
            event: Arc::clone(&self.stop),
        }
    }
    pub fn request_stop(&self) -> Result<(), CaptureError> {
        api("SetEvent(capture stop)", unsafe {
            SetEvent(handle(&self.stop))
        })
    }
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
    /// The report is returned after Stop and owner-thread COM destruction.
    pub fn join(self) -> Result<CaptureReport, CaptureError> {
        let (report, error) = self.join_returning();
        match error {
            Some(error) => Err(error),
            None => Ok(report.expect("joined capture report")),
        }
    }
    fn join_returning(mut self) -> (Option<CaptureReport>, Option<CaptureError>) {
        match self.worker.take().expect("owned capture worker").join() {
            Ok(report) => (Some(report), None),
            // Process preparation preserves its report as the unwind payload,
            // but a panic must still be an Err for every legacy join caller.
            Err(payload) => (
                payload
                    .downcast::<CaptureReport>()
                    .ok()
                    .map(|report| *report),
                Some(CaptureError::WorkerPanicked),
            ),
        }
    }
}
impl Drop for CaptureSession {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = self.request_stop();
            let _ = worker.join();
        }
    }
}
pub struct PreparedCapture {
    pub session: CaptureSession,
    pub source: ClockSource,
    pub observer: BridgeObserver,
    pub sample_rate: u32,
    pub channels: usize,
}

/// A control-side preparation failure, including independently retained cleanup.
#[derive(Debug)]
pub struct CapturePreparationAbortFailure<A> {
    pub error: CaptureError,
    pub report: Option<CaptureReport>,
    pub stop_error: Option<CaptureError>,
    pub join_error: Option<CaptureError>,
    pub abort: Option<A>,
    pub abort_panicked: bool,
    // Explicit startup errors and a closed startup channel have different
    // historical join-panic precedence. Do not infer it from the error value.
    pub(crate) legacy_error: CaptureError,
}

struct CaptureAbortGuard<F: FnOnce() -> A, A> {
    stop: StopSignal,
    session: Option<CaptureSession>,
    before_join: Option<F>,
    stop_without_owner: bool,
}

impl<F: FnOnce() -> A, A> CaptureAbortGuard<F, A> {
    fn failure(&mut self, error: CaptureError) -> Box<CapturePreparationAbortFailure<A>> {
        let stop_error = if self.session.is_some() || self.stop_without_owner {
            self.stop
                .request_stop()
                .err()
                .map(|error| CaptureError::Api {
                    stage: "SetEvent(capture stop)",
                    hresult: error.code().0,
                })
        } else {
            None
        };
        // This closure borrows caller-owned peers; it never crosses an owner/RT boundary.
        let aborted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            self.before_join.take().expect("armed capture abort"),
        ));
        let (report, join_error) = self
            .session
            .take()
            .map(CaptureSession::join_returning)
            .unwrap_or((None, None));
        let (abort, abort_panicked) = match aborted {
            Ok(value) => (Some(value), false),
            Err(_) => (None, true),
        };
        Box::new(CapturePreparationAbortFailure {
            error,
            report,
            stop_error,
            join_error,
            abort,
            abort_panicked,
            legacy_error: error,
        })
    }
}

impl<F: FnOnce() -> A, A> Drop for CaptureAbortGuard<F, A> {
    fn drop(&mut self) {
        if self.before_join.is_some() {
            // Unwind must broadcast to peers before CaptureSession's implicit join.
            let _ = self.failure(CaptureError::StartupLost);
        }
    }
}

pub(crate) fn capture_failure_before_owner<A>(
    error: CaptureError,
    stop: StopSignal,
    before_join: impl FnOnce() -> A,
) -> Box<CapturePreparationAbortFailure<A>> {
    CaptureAbortGuard {
        stop,
        session: None,
        before_join: Some(before_join),
        stop_without_owner: true,
    }
    .failure(error)
}
pub(crate) struct Startup {
    source: ClockSource,
    observer: BridgeObserver,
    sample_rate: u32,
    channels: usize,
}
/// Non-RT startup handshake. Native negotiation happens on the worker, so a
/// rejected format can never be attached to a supposedly running graph.
#[cfg(test)]
#[cfg(test)]
pub(crate) fn start_owner(
    signal: StopSignal,
    owner: impl FnOnce(
        Arc<StdHandle>,
        mpsc::SyncSender<Result<Startup, CaptureError>>,
        Arc<AtomicBool>,
    ) -> CaptureReport
    + Send
    + 'static,
) -> Result<PreparedCapture, CaptureError> {
    start_owner_returning_with_abort_policy(signal, owner, || (), false)
        .map_err(|failure| failure.legacy_error)
}

pub(crate) fn start_owner_returning_with_abort<A>(
    signal: StopSignal,
    owner: impl FnOnce(
        Arc<StdHandle>,
        mpsc::SyncSender<Result<Startup, CaptureError>>,
        Arc<AtomicBool>,
    ) -> CaptureReport
    + Send
    + 'static,
    before_join: impl FnOnce() -> A,
) -> Result<PreparedCapture, Box<CapturePreparationAbortFailure<A>>> {
    start_owner_returning_with_abort_policy(signal, owner, before_join, true)
}

pub(crate) fn start_owner_returning_with_abort_policy<A>(
    signal: StopSignal,
    owner: impl FnOnce(
        Arc<StdHandle>,
        mpsc::SyncSender<Result<Startup, CaptureError>>,
        Arc<AtomicBool>,
    ) -> CaptureReport
    + Send
    + 'static,
    before_join: impl FnOnce() -> A,
    stop_without_owner: bool,
) -> Result<PreparedCapture, Box<CapturePreparationAbortFailure<A>>> {
    start_owner_with_spawn(signal, owner, before_join, stop_without_owner, |owner| {
        thread::Builder::new()
            .name("moiren-shared-capture".into())
            .spawn(owner)
    })
}

fn start_owner_with_spawn<A>(
    signal: StopSignal,
    owner: impl FnOnce(
        Arc<StdHandle>,
        mpsc::SyncSender<Result<Startup, CaptureError>>,
        Arc<AtomicBool>,
    ) -> CaptureReport
    + Send
    + 'static,
    before_join: impl FnOnce() -> A,
    stop_without_owner: bool,
    spawn: impl FnOnce(
        Box<dyn FnOnce() -> CaptureReport + Send>,
    ) -> std::io::Result<JoinHandle<CaptureReport>>,
) -> Result<PreparedCapture, Box<CapturePreparationAbortFailure<A>>> {
    let mut guard = CaptureAbortGuard {
        stop: signal.clone(),
        session: None,
        before_join: Some(before_join),
        stop_without_owner,
    };
    let stop = signal.event;
    let worker_stop = Arc::clone(&stop);
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker =
        spawn(Box::new(move || owner(worker_stop, tx, worker_started))).map_err(|error| {
            guard.failure(CaptureError::WorkerSpawn {
                code: error.raw_os_error(),
            })
        })?;
    guard.session = Some(CaptureSession {
        stop,
        started,
        worker: Some(worker),
    });
    match rx.recv() {
        Ok(Ok(ready)) => {
            let session = guard.session.take().expect("owned capture session");
            guard.before_join = None;
            Ok(PreparedCapture {
                session,
                source: ready.source,
                observer: ready.observer,
                sample_rate: ready.sample_rate,
                channels: ready.channels,
            })
        }
        Ok(Err(error)) => Err(guard.failure(error)),
        Err(_) => {
            let mut failure = guard.failure(CaptureError::StartupLost);
            failure.legacy_error = failure.join_error.unwrap_or(failure.error);
            Err(failure)
        }
    }
}

/// Retain the exact partial report without changing the worker's panic outcome.
pub(crate) fn finish_owner_retaining_report(
    mut report: CaptureReport,
    stop: &StdHandle,
    sender: &mpsc::SyncSender<Result<Startup, CaptureError>>,
    run: impl FnOnce(&mut CaptureReport) -> Result<CaptureStatus, CaptureError>,
) -> CaptureReport {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut report))) {
        Ok(result) => stream::finish(result, stop, sender, report),
        Err(_) => {
            let report = stream::finish(Err(CaptureError::WorkerPanicked), stop, sender, report);
            std::panic::resume_unwind(Box::new(report));
        }
    }
}
