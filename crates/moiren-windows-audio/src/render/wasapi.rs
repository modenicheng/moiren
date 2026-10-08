//! Thread-owned WASAPI objects. Only the stop kernel handle is shared.
use super::{DemandRenderer, RenderError, RenderOptions, RenderReport};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle as StdHandle},
    sync::Arc,
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

pub use endpoint::{RenderEndpoint, list_render_endpoints};
use stream::run_owner;

#[cfg(test)]
mod tests;

fn api<T>(stage: &'static str, result: windows::core::Result<T>) -> Result<T, RenderError> {
    result.map_err(|error| RenderError::Api {
        stage,
        hresult: error.code().0,
    })
}
fn handle(value: &StdHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}
fn stop_event() -> Result<StdHandle, RenderError> {
    let raw = api("CreateEvent(stop)", unsafe {
        CreateEventW(None, true, false, None)
    })?;
    // SAFETY: CreateEvent returned a uniquely owned valid kernel handle.
    Ok(unsafe { StdHandle::from_raw_handle(raw.0) })
}

pub struct RenderSession {
    stop: Arc<StdHandle>,
    worker: Option<JoinHandle<RenderReport>>,
}
impl RenderSession {
    pub fn request_stop(&self) -> Result<(), RenderError> {
        api("SetEvent(stop)", unsafe { SetEvent(handle(&self.stop)) })
    }
    /// Waits for normal duration completion or a requested stop. The returned
    /// report is created after all stream/COM objects have been released.
    pub fn join(mut self) -> Result<RenderReport, RenderError> {
        self.worker
            .take()
            .expect("owned worker")
            .join()
            .map_err(|_| RenderError::WorkerPanicked)
    }
}
impl Drop for RenderSession {
    fn drop(&mut self) {
        // Joining keeps cleanup on the COM owner and avoids detaching a live stream.
        if let Some(worker) = self.worker.take() {
            let _ = self.request_stop();
            let _ = worker.join();
        }
    }
}
pub fn start_render(
    options: RenderOptions,
    renderer: DemandRenderer,
) -> Result<RenderSession, RenderError> {
    options.validate()?;
    let stop = Arc::new(stop_event()?);
    let worker_stop = Arc::clone(&stop);
    let worker = thread::Builder::new()
        .name("moiren-shared-render".into())
        .spawn(move || run_owner(options, renderer, worker_stop))
        .map_err(|error| RenderError::WorkerSpawn {
            code: error.raw_os_error(),
        })?;
    Ok(RenderSession {
        stop,
        worker: Some(worker),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    Stop,
    Audio,
    Timeout,
}
fn wait(stop: HANDLE, audio: HANDLE, timeout_ms: u32) -> Result<Wake, RenderError> {
    // Windows returns the lowest signaled index; stop must win over queued audio
    // so shutdown remains responsive even when both events are already signaled.
    match unsafe { WaitForMultipleObjects(&[stop, audio], false, timeout_ms) } {
        event if event == WAIT_OBJECT_0 => Ok(Wake::Stop),
        event if event.0 == WAIT_OBJECT_0.0 + 1 => Ok(Wake::Audio),
        WAIT_TIMEOUT => Ok(Wake::Timeout),
        _ => Err(RenderError::Api {
            stage: "WaitForMultipleObjects",
            hresult: windows::core::Error::from_thread().code().0,
        }),
    }
}
