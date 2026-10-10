//! Thread-owned WASAPI objects. Only the stop kernel handle is shared.
use super::{DemandRenderer, RenderError, RenderOptions, RenderReport};
use crate::{Activation, ActivationGate, StopSignal};
#[cfg(test)]
use std::os::windows::io::FromRawHandle;
use std::{
    os::windows::io::{AsRawHandle, OwnedHandle as StdHandle},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};
#[cfg(test)]
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{SetEvent, WaitForMultipleObjects},
};

mod endpoint;
mod format;
mod packet;
mod stream;

pub use endpoint::{RenderEndpoint, list_render_endpoints};

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
#[cfg(test)]
fn stop_event() -> Result<StdHandle, RenderError> {
    let raw = api("CreateEvent(stop)", unsafe {
        CreateEventW(None, true, false, None)
    })?;
    // SAFETY: CreateEvent returned a uniquely owned valid kernel handle.
    Ok(unsafe { StdHandle::from_raw_handle(raw.0) })
}

/// Native interfaces have already been released on the output owner.
pub struct RenderOwnerExit {
    pub report: RenderReport,
    pub renderer: DemandRenderer,
}
pub struct RenderSession {
    stop: Arc<StdHandle>,
    started: Arc<AtomicBool>,
    worker: Option<JoinHandle<RenderOwnerExit>>,
}
impl RenderSession {
    /// A post-Start acknowledgement, independent of preparation readiness.
    pub fn has_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub fn request_stop(&self) -> Result<(), RenderError> {
        api("SetEvent(stop)", unsafe { SetEvent(handle(&self.stop)) })
    }
    /// Blocking cleanup-worker operation. The pure Rust renderer transfers to
    /// the joiner even when native preparation or streaming reported failure.
    pub fn join_with_renderer(mut self) -> Result<RenderOwnerExit, RenderError> {
        self.worker
            .take()
            .expect("owned worker")
            .join()
            .map_err(|_| RenderError::WorkerPanicked)
    }
    /// CLI convenience: releases the renderer on the calling thread.
    pub fn join(self) -> Result<RenderReport, RenderError> {
        let exit = self.join_with_renderer()?;
        Ok(exit.report)
    }
}
impl Drop for RenderSession {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = self.request_stop();
            // The join result owns renderer destruction on this non-RT caller.
            let _ = worker.join();
        }
    }
}
pub struct PreparedRenderSession {
    session: RenderSession,
    gate: ActivationGate,
}
impl PreparedRenderSession {
    pub fn activate(&self) -> Result<(), RenderError> {
        self.gate.activate().map_err(Into::into)
    }
    pub fn request_stop(&self) -> Result<(), RenderError> {
        self.session.request_stop()
    }
    pub fn has_started(&self) -> bool {
        self.session.has_started()
    }
    pub fn into_session(self) -> RenderSession {
        self.session
    }
}
pub fn start_render(
    options: RenderOptions,
    renderer: DemandRenderer,
) -> Result<RenderSession, RenderError> {
    options.validate()?;
    let stop = api("CreateEvent(render stop)", StopSignal::new())?;
    start_render_with_stop(options, renderer, stop)
}
pub fn start_render_with_stop(
    options: RenderOptions,
    renderer: DemandRenderer,
    stop: StopSignal,
) -> Result<RenderSession, RenderError> {
    let prepared = start_render_prepared(options, renderer, stop)?;
    prepared.activate()?;
    Ok(prepared.into_session())
}
/// Blocking preparation belongs on a worker, never on the UI/service loop.
pub fn start_render_prepared(
    options: RenderOptions,
    renderer: DemandRenderer,
    stop: StopSignal,
) -> Result<PreparedRenderSession, RenderError> {
    let gate = ActivationGate::new(stop.clone())?;
    start_render_prepared_with_gate(options, renderer, stop, gate)
}
pub fn start_render_prepared_with_gate(
    options: RenderOptions,
    renderer: DemandRenderer,
    stop: StopSignal,
    gate: ActivationGate,
) -> Result<PreparedRenderSession, RenderError> {
    options.validate()?;
    gate.validate_stop(&stop)?;
    start_owner(
        renderer,
        stop,
        gate,
        move |renderer, stop, gate, ready, started| {
            stream::run_owner_with(options, renderer, stop, gate, ready, started, stream::owner)
        },
    )
}
fn start_owner(
    renderer: DemandRenderer,
    signal: StopSignal,
    gate: ActivationGate,
    owner: impl FnOnce(
        DemandRenderer,
        Arc<StdHandle>,
        ActivationGate,
        mpsc::SyncSender<Result<(), RenderError>>,
        Arc<AtomicBool>,
    ) -> RenderOwnerExit
    + Send
    + 'static,
) -> Result<PreparedRenderSession, RenderError> {
    let stop = signal.event;
    let worker_stop = Arc::clone(&stop);
    let worker_gate = gate.clone();
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("moiren-shared-render".into())
        .spawn(move || owner(renderer, worker_stop, worker_gate, tx, worker_started))
        .map_err(|error| RenderError::WorkerSpawn {
            code: error.raw_os_error(),
        })?;
    let session = RenderSession {
        stop,
        started,
        worker: Some(worker),
    };
    match rx.recv() {
        Ok(Ok(())) => Ok(PreparedRenderSession { session, gate }),
        Ok(Err(error)) => {
            session.join_with_renderer()?;
            Err(error)
        }
        Err(_) => {
            let _ = session.request_stop();
            session.join_with_renderer()?;
            Err(RenderError::StartupLost)
        }
    }
}
fn ready_and_wait(
    ready: &mpsc::SyncSender<Result<(), RenderError>>,
    gate: &ActivationGate,
) -> Result<Activation, RenderError> {
    ready.send(Ok(())).map_err(|_| RenderError::StartupLost)?;
    Ok(gate.wait()?)
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
