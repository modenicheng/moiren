//! Only kernel handles, prepared ring endpoints and scalars cross owner threads.
use super::{CaptureError, CaptureOptions, CaptureReport};
use crate::StopSignal;
use crate::clock_bridge::{BridgeObserver, ClockSource};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle as StdHandle},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};
use windows::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects},
};

mod endpoint;
mod format;
mod packet;
mod stream;
#[cfg(test)]
mod tests;
pub use endpoint::{CaptureEndpoint, list_capture_endpoints};

fn api<T>(stage: &'static str, result: windows::core::Result<T>) -> Result<T, CaptureError> {
    result.map_err(|error| CaptureError::Api {
        stage,
        hresult: error.code().0,
    })
}
fn handle(value: &StdHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}
fn stop_event() -> Result<StdHandle, CaptureError> {
    let raw = api("CreateEvent(capture stop)", unsafe {
        CreateEventW(None, true, false, None)
    })?;
    // SAFETY: CreateEvent returned a uniquely owned valid kernel handle.
    Ok(unsafe { StdHandle::from_raw_handle(raw.0) })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    Stop,
    Audio,
    Timeout,
}
fn wait(stop: HANDLE, audio: HANDLE, timeout_ms: u32) -> Result<Wake, CaptureError> {
    // Lowest signaled index wins. Stop must also work without any audio wake.
    match unsafe { WaitForMultipleObjects(&[stop, audio], false, timeout_ms) } {
        event if event == WAIT_OBJECT_0 => Ok(Wake::Stop),
        event if event.0 == WAIT_OBJECT_0.0 + 1 => Ok(Wake::Audio),
        WAIT_TIMEOUT => Ok(Wake::Timeout),
        _ => Err(CaptureError::Api {
            stage: "WaitForMultipleObjects(capture)",
            hresult: windows::core::Error::from_thread().code().0,
        }),
    }
}

pub struct CaptureSession {
    stop: Arc<StdHandle>,
    worker: Option<JoinHandle<CaptureReport>>,
}
impl CaptureSession {
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
pub(super) struct Startup {
    source: ClockSource,
    observer: BridgeObserver,
    sample_rate: u32,
    channels: usize,
}
/// Non-RT startup handshake. Native negotiation happens on the worker, so a
/// rejected format can never be attached to a supposedly running graph.
pub fn start_capture(options: CaptureOptions) -> Result<PreparedCapture, CaptureError> {
    options.validate()?;
    let stop = Arc::new(stop_event()?);
    let worker_stop = Arc::clone(&stop);
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("moiren-shared-capture".into())
        .spawn(move || stream::run_owner(options, worker_stop, tx))
        .map_err(|error| CaptureError::WorkerSpawn {
            code: error.raw_os_error(),
        })?;
    let session = CaptureSession {
        stop,
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
