use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use moiren_core::{graph::NodeId, protocol::ProcessorId};
use thiserror::Error;

use super::{EngineProject, NodeConfig};
use crate::{
    compiler::{CompiledBindings, NodeBindings},
    runtime::Engine,
    sample::ProcessingSample,
};

type CancelHook = Box<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<Cancellation>,
}
#[derive(Default)]
struct Cancellation {
    cancelled: AtomicBool,
    hooks: Mutex<Vec<CancelHook>>,
    wake: Condvar,
}

impl CancellationToken {
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            let hooks = std::mem::take(
                &mut *self
                    .inner
                    .hooks
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()),
            );
            self.inner.wake.notify_all();
            for hook in hooks {
                let _ = catch_unwind(AssertUnwindSafe(hook));
            }
        }
    }
    /// Hooks run on a non-RT caller and must only wake native preparation/owners.
    pub fn on_cancel(&self, hook: impl Fn() + Send + Sync + 'static) -> Result<(), BackendError> {
        let mut hooks = self
            .inner
            .hooks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.is_cancelled() {
            drop(hooks);
            catch_unwind(AssertUnwindSafe(hook)).map_err(|_| BackendError::WorkerPanicked)?;
        } else if hooks.len() < 16 {
            hooks.push(Box::new(hook));
        } else {
            return Err(BackendError::Failed(
                "cancellation hook capacity exhausted".into(),
            ));
        }
        Ok(())
    }
    pub fn wait_cancelled(&self, timeout: Duration) -> bool {
        let hooks = self
            .inner
            .hooks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _guard = self
            .inner
            .wake
            .wait_timeout_while(hooks, timeout, |_| !self.is_cancelled())
            .unwrap_or_else(|poison| poison.into_inner());
        self.is_cancelled()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BackendError {
    #[error("resource unavailable: {0}")]
    ResourceUnavailable(String),
    #[error("unsupported backend configuration: {0}")]
    Unsupported(String),
    #[error("backend failed: {0}")]
    Failed(String),
    #[error("preparation cancelled")]
    Cancelled,
    #[error("backend worker panicked")]
    WorkerPanicked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareMode {
    Initial,
    Update,
}

pub struct PrepareRequest<'a> {
    pub project: &'a EngineProject,
    pub mode: PrepareMode,
    pub reuse: &'a [(NodeId, ProcessorId)],
    pub graph_revision: u64,
    pub owner_generation: u64,
    pub resource_generation: u64,
    pub signal_host: &'a moiren_signal::HostControl,
}

/// The backend prepares IO/observers, never compiles the logical graph.
/// Update preparation leaves reused IO nodes unbound and returns no transport.
pub trait EngineBackend<S: ProcessingSample>: Send + Sync + 'static {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<S>, BackendError>;
    /// Nonblocking, one event per call. Probes must be bounded and cancellable.
    fn poll_resource(&self) -> Option<ResourceEvent> {
        None
    }
}

/// Lets type-erased backends flow into `EngineHost::open(impl EngineBackend)`
/// without each caller wrapping them in a forwarding newtype.
impl<S: ProcessingSample> EngineBackend<S> for Box<dyn EngineBackend<S>> {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<S>, BackendError> {
        (**self).prepare(request, cancel)
    }
    fn poll_resource(&self) -> Option<ResourceEvent> {
        (**self).poll_resource()
    }
}

pub struct PreparedBackend<S: ProcessingSample> {
    pub bindings: NodeBindings<S>,
    pub transport: Option<Box<dyn PreparedTransport<S>>>,
}

/// COM objects stay on their native threads; this is only a movable handoff.
pub trait PreparedTransport<S: ProcessingSample>: Send + 'static {
    fn attach(
        self: Box<Self>,
        engine: Engine<S>,
        bindings: &CompiledBindings,
        cancel: CancellationToken,
    ) -> Result<Box<dyn BackendOwner<S>>, Box<AttachFailure<S>>>;
}

pub struct AttachFailure<S: ProcessingSample> {
    pub engine: Engine<S>,
    pub error: BackendError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerStatus {
    Starting,
    Running,
    Failed(BackendError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerTimeline {
    pub epoch: u64,
    pub revision: u64,
    pub frame: u64,
}

/// Must preserve Engine ownership on every native error and caught RT panic.
/// stop_and_join is non-RT, must not panic, and returns only after native stop.
pub trait BackendOwner<S: ProcessingSample>: Send + 'static {
    /// Opens the prepared owner's start gate. Attach must not start audio.
    /// Called once by the coordinator after current-basis/closing validation.
    fn activate(&self) -> Result<(), BackendError>;
    fn cancel(&self);
    fn status(&self) -> OwnerStatus;
    fn timeline(&self) -> Option<OwnerTimeline>;
    fn stop_and_join(self: Box<Self>) -> OwnerReturn<S>;
}

pub struct OwnerReturn<S: ProcessingSample> {
    pub engine: Engine<S>,
    pub error: Option<BackendError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceEvent {
    pub node: NodeId,
    pub configuration: NodeConfig,
    pub generation: u64,
    pub available: bool,
}
