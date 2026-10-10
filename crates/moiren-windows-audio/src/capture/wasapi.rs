//! Only kernel handles, prepared ring endpoints and scalars cross owner threads.
use super::{CaptureError, CaptureOptions, CaptureReport};
use crate::clock_bridge::{BridgeObserver, ClockSource};
use crate::{ActivationGate, StopSignal};
use std::{
    os::windows::io::{AsRawHandle, OwnedHandle as StdHandle},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};
use windows::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{SetEvent, WaitForMultipleObjects},
};

mod endpoint;
mod format;
mod packet;
mod physical;
pub(crate) mod stream;
#[cfg(test)]
mod tests;
pub use endpoint::{CaptureEndpoint, list_capture_endpoints};

pub(crate) fn api<T>(
    stage: &'static str,
    result: windows::core::Result<T>,
) -> Result<T, CaptureError> {
    result.map_err(|error| CaptureError::Api {
        stage,
        hresult: error.code().0,
    })
}
pub(crate) fn handle(value: &StdHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    Stop,
    TargetExited,
    Audio,
    Timeout,
}
#[cfg(test)]
fn wait(stop: HANDLE, audio: HANDLE, timeout_ms: u32) -> Result<Wake, CaptureError> {
    wait_target(stop, audio, None, timeout_ms)
}
fn wait_target(
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
    match unsafe { WaitForMultipleObjects(handles, false, timeout_ms) } {
        event if event == WAIT_OBJECT_0 => Ok(Wake::Stop),
        event if event.0 == WAIT_OBJECT_0.0 + 1 && target.is_some() => Ok(Wake::TargetExited),
        event if event.0 == WAIT_OBJECT_0.0 + handles.len() as u32 - 1 => Ok(Wake::Audio),
        WAIT_TIMEOUT => Ok(Wake::Timeout),
        _ => Err(CaptureError::Api {
            stage: "WaitForMultipleObjects(capture)",
            hresult: windows::core::Error::from_thread().code().0,
        }),
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
    pub fn join(mut self) -> Result<CaptureReport, CaptureError> {
        self.worker
            .take()
            .expect("owned capture worker")
            .join()
            .map_err(|_| CaptureError::WorkerPanicked)
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
pub(crate) struct Startup {
    source: ClockSource,
    observer: BridgeObserver,
    sample_rate: u32,
    channels: usize,
}
/// Non-RT startup handshake. Native negotiation happens on the worker, so a
/// rejected format can never be attached to a supposedly running graph.
pub fn start_capture(options: CaptureOptions) -> Result<PreparedCapture, CaptureError> {
    options.validate()?;
    let stop = api("CreateEvent(capture stop)", StopSignal::new())?;
    start_capture_with_stop(options, stop)
}
pub fn start_capture_with_stop(
    options: CaptureOptions,
    stop: StopSignal,
) -> Result<PreparedCapture, CaptureError> {
    let gate = ActivationGate::new(stop.clone())?;
    let prepared = prepare_capture_with_gate(options, stop, gate.clone())?;
    gate.activate()?;
    Ok(prepared)
}
/// Blocking worker-side negotiation; no audio starts until the shared gate opens.
pub fn prepare_capture_with_gate(
    options: CaptureOptions,
    stop: StopSignal,
    gate: ActivationGate,
) -> Result<PreparedCapture, CaptureError> {
    options.validate()?;
    gate.validate_stop(&stop)?;
    start_owner(stop, move |stop, sender, started| {
        physical::run_owner(options, stop, sender, gate, started)
    })
}
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
    let stop = signal.event;
    let worker_stop = Arc::clone(&stop);
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("moiren-shared-capture".into())
        .spawn(move || owner(worker_stop, tx, worker_started))
        .map_err(|error| CaptureError::WorkerSpawn {
            code: error.raw_os_error(),
        })?;
    let session = CaptureSession {
        stop,
        started,
        worker: Some(worker),
    };
    match rx.recv() {
        Ok(Ok(ready)) => Ok(PreparedCapture {
            session,
            source: ready.source,
            observer: ready.observer,
            sample_rate: ready.sample_rate,
            channels: ready.channels,
        }),
        Ok(Err(error)) => {
            drop(session);
            Err(error)
        }
        Err(_) => {
            // A panic before the handshake must be distinguished from an API
            // failure, and joining must not detach a partially opened stream.
            let _ = session.request_stop();
            session.join()?;
            Err(CaptureError::StartupLost)
        }
    }
}
