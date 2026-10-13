//! Non-RT managed project, resource and execution ownership.
mod backend;
mod model;
mod parameters;
mod preparation;
mod project;
mod retirement;
mod signals;
mod supervisor;

pub use backend::*;
pub use model::*;
pub use project::*;
pub use signals::ManagedSignalSubscription;

use crate::sample::ProcessingSample;
use moiren_core::protocol::ParamValue;
use std::{
    marker::PhantomData,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub command_capacity: usize,
    pub receipt_capacity: usize,
    pub cleanup_capacity: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_parameters: usize,
    pub signal_bindings: usize,
    pub signals: moiren_signal::BusConfig,
    pub poll_interval: Duration,
}
impl Default for HostConfig {
    fn default() -> Self {
        Self {
            command_capacity: 64,
            receipt_capacity: 256,
            cleanup_capacity: 4,
            max_nodes: 128,
            max_edges: 512,
            max_parameters: 2048,
            signal_bindings: 256,
            signals: Default::default(),
            poll_interval: Duration::from_millis(5),
        }
    }
}
impl HostConfig {
    pub(crate) fn validate_project(&self, project: &EngineProject) -> Result<(), ProjectError> {
        if project.graph.nodes().len() > self.max_nodes
            || project.graph.edges().len() > self.max_edges
            || project.parameters.len() > self.max_parameters
        {
            return Err(ProjectError::Capacity);
        }
        project.validate()?;
        if project.parameter_values().len() > self.max_parameters {
            return Err(ProjectError::Capacity);
        }
        Ok(())
    }
}

type Edit = Box<dyn FnOnce(&mut EngineProject) -> Result<(), ProjectError> + Send>;
pub(crate) enum Command {
    Edit(Edit, EditReceipt),
    Parameter {
        id: u64,
        target: ParameterTarget,
        value: ParamValue,
        timing: ParameterTiming,
        ramp_frames: u32,
        receipt: ParameterReceipt,
    },
}
pub(crate) struct Shared {
    closing: AtomicBool,
    snapshot: Mutex<HostSnapshot>,
    project: Mutex<EngineProject>,
    wake_lock: Mutex<()>,
    wake: Condvar,
    permits: Arc<AtomicUsize>,
    parameter_ids: AtomicU64,
    signals: Arc<signals::SignalHub>,
}
impl Shared {
    pub(crate) fn notify(&self) {
        self.wake.notify_all();
    }
    pub(crate) fn next_parameter_id(&self) -> Result<u64, HostError> {
        self.parameter_ids
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| HostError::IdentityExhausted)
    }
}

/// Unique non-RT lifecycle owner. Drop cancels preparation, stops native owners,
/// joins workers and finishes control/plan/signal reclamation before returning.
/// When released inside its own edit callback, closure happens after that
/// callback returns; the coordinator cannot synchronously join itself.
pub struct EngineHost<S: ProcessingSample = f64> {
    shared: Arc<Shared>,
    commands: SyncSender<Command>,
    worker: Option<JoinHandle<()>>,
    config: HostConfig,
    marker: PhantomData<S>,
}
impl<S: ProcessingSample> EngineHost<S> {
    pub fn open(
        config: HostConfig,
        project: EngineProject,
        backend: impl EngineBackend<S>,
    ) -> Result<Self, HostError> {
        if config.command_capacity == 0
            || config.receipt_capacity == 0
            || config.cleanup_capacity < 4
            || config.max_nodes == 0
            || config.max_edges == 0
            || config.max_parameters == 0
            || config.signal_bindings == 0
            || config.poll_interval.is_zero()
            || config.poll_interval > Duration::from_secs(1)
        {
            return Err(HostError::InvalidConfig);
        }
        config.validate_project(&project)?;
        let signals = Arc::new(signals::SignalHub::new(config.signals).map_err(HostError::Worker)?);
        let shared = Arc::new(Shared {
            closing: AtomicBool::new(false),
            snapshot: Mutex::new(HostSnapshot {
                host: HostState::Opening,
                execution: ExecutionState::Dormant,
                desired_revision: 1,
                active_revision: None,
                plan_revision: None,
                owner_generation: 0,
                resource_generation: 0,
                timeline: None,
                update: UpdateState::Idle,
                fault: None,
                cleanup_error: None,
            }),
            project: Mutex::new(project),
            wake_lock: Mutex::new(()),
            wake: Condvar::new(),
            permits: Arc::new(AtomicUsize::new(0)),
            parameter_ids: AtomicU64::new(1),
            signals,
        });
        let (commands, receiver) = mpsc::sync_channel(config.command_capacity);
        let worker = supervisor::spawn(
            config.clone(),
            Arc::clone(&shared),
            receiver,
            Arc::new(backend),
        )?;
        Ok(Self {
            shared,
            commands,
            worker: Some(worker),
            config,
            marker: PhantomData,
        })
    }
    pub fn control(&self) -> EngineControl {
        EngineControl {
            shared: Arc::clone(&self.shared),
            commands: self.commands.clone(),
            receipt_capacity: self.config.receipt_capacity,
        }
    }
    pub fn observer(&self) -> EngineObserver {
        EngineObserver {
            shared: Arc::clone(&self.shared),
        }
    }
}
impl<S: ProcessingSample> Drop for EngineHost<S> {
    fn drop(&mut self) {
        let gate = self
            .shared
            .wake_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.shared.closing.store(true, Ordering::Release);
        drop(gate);
        self.shared.notify();
        if let Some(worker) = self.worker.take()
            && worker.thread().id() != std::thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

/// Cloneable command endpoint. Retaining it never keeps audio execution alive.
#[derive(Clone)]
pub struct EngineControl {
    shared: Arc<Shared>,
    commands: SyncSender<Command>,
    receipt_capacity: usize,
}
impl EngineControl {
    pub fn edit(
        &self,
        edit: impl FnOnce(&mut EngineProject) -> Result<(), ProjectError> + Send + 'static,
    ) -> Result<EditReceipt, HostError> {
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        let receipt = EditReceipt {
            cell: ReceiptCell::reserve(
                EditOutcome::Queued,
                &self.shared.permits,
                self.receipt_capacity,
            )?,
        };
        self.send(Command::Edit(Box::new(edit), receipt.clone()))?;
        Ok(receipt)
    }
    pub fn parameter(
        &self,
        target: ParameterTarget,
        value: ParamValue,
        timing: ParameterTiming,
        ramp_frames: u32,
    ) -> Result<ParameterReceipt, HostError> {
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        let receipt = ParameterReceipt {
            cell: ReceiptCell::reserve(
                ParameterOutcome::Queued,
                &self.shared.permits,
                self.receipt_capacity,
            )?,
        };
        let id = self.shared.next_parameter_id()?;
        self.send(Command::Parameter {
            id,
            target,
            value,
            timing,
            ramp_frames,
            receipt: receipt.clone(),
        })?;
        Ok(receipt)
    }
    fn send(&self, command: Command) -> Result<(), HostError> {
        let _gate = self
            .shared
            .wake_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => HostError::Capacity,
                mpsc::TrySendError::Disconnected(_) => HostError::Closed,
            })?;
        self.shared.notify();
        Ok(())
    }
}

/// Read-only snapshots/subscriptions survive the lifecycle owner for diagnostics.
#[derive(Clone)]
pub struct EngineObserver {
    shared: Arc<Shared>,
}
impl EngineObserver {
    pub fn snapshot(&self) -> HostSnapshot {
        self.shared
            .snapshot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
    pub fn project(&self) -> EngineProject {
        self.shared
            .project
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
    pub fn subscribe_compressor(
        &self,
        node: moiren_core::graph::NodeId,
    ) -> Result<ManagedSignalSubscription, String> {
        self.shared.signals.subscribe(node)
    }
}
