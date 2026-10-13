use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use moiren_core::protocol::{ControlReply, ReplyCode};
use thiserror::Error;

use super::{BackendError, OwnerTimeline, ProjectError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostState {
    Opening,
    Open,
    Closing,
    Closed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Dormant,
    Preparing,
    Running,
    WaitingForResources,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    Backend(BackendError),
    Compile(String),
    Fault(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Preparing(u64),
    Pending(u64),
    Blocked {
        graph_revision: u64,
        reason: Diagnostic,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSnapshot {
    pub host: HostState,
    pub execution: ExecutionState,
    pub desired_revision: u64,
    pub active_revision: Option<u64>,
    pub plan_revision: Option<u64>,
    pub owner_generation: u64,
    pub resource_generation: u64,
    pub timeline: Option<OwnerTimeline>,
    pub update: UpdateState,
    pub fault: Option<Diagnostic>,
    pub cleanup_error: Option<BackendError>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditOutcome {
    Queued,
    Accepted {
        graph_revision: u64,
    },
    Activated {
        graph_revision: u64,
        plan_revision: Option<u64>,
    },
    Superseded {
        by_graph_revision: u64,
    },
    Rejected(ProjectError),
    Closed,
}
impl EditOutcome {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Queued | Self::Accepted { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterFailure {
    InvalidTarget,
    InvalidValue,
    TargetRemoved,
    StaleEpoch,
    Runtime(ReplyCode),
    Capacity,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterOutcome {
    Queued,
    Accepted,
    Applied(ControlReply),
    Rejected(ParameterFailure),
    Superseded,
    Closed,
}
impl ParameterOutcome {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Queued | Self::Accepted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterTiming {
    NextBlock,
    Frame { epoch: u64, frame: u64 },
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum HostError {
    #[error("host is closed")]
    Closed,
    #[error("host command/receipt capacity exhausted")]
    Capacity,
    #[error("host identity capacity exhausted")]
    IdentityExhausted,
    #[error("invalid host capacity or poll interval")]
    InvalidConfig,
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("could not start host worker: {0}")]
    Worker(String),
}

pub(crate) struct ReceiptCell<Outcome> {
    outcome: Mutex<Outcome>,
    wake: Condvar,
    permits: Arc<AtomicUsize>,
}
impl<Outcome: Clone> ReceiptCell<Outcome> {
    pub(crate) fn reserve(
        initial: Outcome,
        permits: &Arc<AtomicUsize>,
        capacity: usize,
    ) -> Result<Arc<Self>, HostError> {
        permits
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count < capacity {
                    count.checked_add(1)
                } else {
                    None
                }
            })
            .map_err(|_| HostError::Capacity)?;
        Ok(Arc::new(Self {
            outcome: Mutex::new(initial),
            wake: Condvar::new(),
            permits: Arc::clone(permits),
        }))
    }
    pub(crate) fn set(&self, outcome: Outcome) {
        *self
            .outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = outcome;
        self.wake.notify_all();
    }
    fn get(&self) -> Outcome {
        self.outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
    fn wait(&self, timeout: Duration, terminal: impl Fn(&Outcome) -> bool) -> Outcome {
        let outcome = self
            .outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (outcome, _) = self
            .wake
            .wait_timeout_while(outcome, timeout, |outcome| !terminal(outcome))
            .unwrap_or_else(|poison| poison.into_inner());
        outcome.clone()
    }
}
impl<Outcome> Drop for ReceiptCell<Outcome> {
    fn drop(&mut self) {
        self.permits.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub struct EditReceipt {
    pub(crate) cell: Arc<ReceiptCell<EditOutcome>>,
}
impl EditReceipt {
    pub fn outcome(&self) -> EditOutcome {
        self.cell.get()
    }
    pub fn wait(&self, timeout: Duration) -> EditOutcome {
        self.cell.wait(timeout, EditOutcome::is_terminal)
    }
}
#[derive(Clone)]
pub struct ParameterReceipt {
    pub(crate) cell: Arc<ReceiptCell<ParameterOutcome>>,
}
impl ParameterReceipt {
    pub fn outcome(&self) -> ParameterOutcome {
        self.cell.get()
    }
    pub fn wait(&self, timeout: Duration) -> ParameterOutcome {
        self.cell.wait(timeout, ParameterOutcome::is_terminal)
    }
}
