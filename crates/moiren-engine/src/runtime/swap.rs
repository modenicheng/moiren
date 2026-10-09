//! Non-RT preparation and bounded ownership transfer at the render boundary.
use super::*;
use crate::{buffer::IoMode, control::ParamSpec};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    fmt, mem,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PlanSwapError {
    #[error("retire queue capacity must be nonzero")]
    InvalidCapacity,
    #[error("plan switching is already enabled")]
    AlreadyEnabled,
    #[error("candidate must be an unrendered, live engine without a swap port")]
    InvalidCandidate,
    #[error("hot swaps require the same processing configuration")]
    IncompatibleConfig,
    #[error("hot swaps must preserve the timeline epoch")]
    StaleEpoch,
    #[error("candidate revision must exceed every previously published revision")]
    StaleRevision,
    #[error("one candidate is already pending")]
    PendingFull,
    #[error("the render owner has stopped")]
    Disconnected,
    #[error("reuse references a missing processor")]
    MissingProcessor,
    #[error("reuse maps an old or new processor more than once")]
    DuplicateReuse,
    #[error("reused processor type, role, latency or IO schema differs")]
    IncompatibleProcessor,
    #[error("reused processor parameter IDs or domains differ")]
    IncompatibleParameters,
    #[error("reuse was prepared against a runtime that is no longer active")]
    StaleBasis,
    #[error("control cancelled the pending candidate")]
    Cancelled,
}

#[derive(Clone, PartialEq, Eq)]
struct IoSchema {
    mode: IoMode,
    inputs: Box<[(u16, usize)]>,
    outputs: Box<[(u16, usize)]>,
}
struct ProcessorSchema {
    id: ProcessorId,
    state_type: TypeId,
    role: ProcessorRole,
    latency: u32,
    io: Option<IoSchema>,
}
struct Snapshot {
    processors: Box<[ProcessorSchema]>,
    parameters: Arc<[ParamSpec]>,
    config: EngineConfig,
    revision: u64,
    epoch: u64,
}

/// Immutable control-side metadata. Cloning this snapshot is non-RT; it never
/// exposes or borrows the live processor or parameter state.
#[derive(Clone)]
pub struct PlanSnapshot(Arc<Snapshot>);
impl PlanSnapshot {
    pub fn revision(&self) -> u64 {
        self.0.revision
    }
    pub fn epoch(&self) -> u64 {
        self.0.epoch
    }
    pub fn config(&self) -> EngineConfig {
        self.0.config
    }
    pub(super) fn capture<S: ProcessingSample>(
        plan: &ExecutionPlan<S>,
        resources: &RtResources<S>,
        parameters: &ParameterRuntime,
    ) -> Self {
        let processors = resources
            .instances
            .iter()
            .enumerate()
            .map(|(index, instance)| {
                let io = plan.ops.iter().find(|op| op.runtime == index).map(|op| {
                    let mut inputs: Vec<_> = op.io.input_ports().collect();
                    let mut outputs: Vec<_> = op.io.output_ports().collect();
                    inputs.sort_unstable();
                    outputs.sort_unstable();
                    IoSchema {
                        mode: op.io.mode(),
                        inputs: inputs.into_boxed_slice(),
                        outputs: outputs.into_boxed_slice(),
                    }
                });
                ProcessorSchema {
                    id: instance.id,
                    state_type: instance.state_type,
                    role: instance.processor.role(),
                    latency: instance.processor.latency_frames(),
                    io,
                }
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self(Arc::new(Snapshot {
            processors,
            parameters: parameters.schema(),
            config: plan.config,
            revision: plan.revision,
            epoch: plan.epoch,
        }))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessorReuse {
    pub old: ProcessorId,
    pub new: ProcessorId,
}
struct Transfer {
    basis: PlanSnapshot,
    processors: Box<[(usize, usize)]>,
    parameters: Box<[(usize, usize)]>,
}

/// Complete prepared ownership package. Construct and validate off RT, then
/// publish. The candidate's new ControlPort stays with its control owner.
pub struct PreparedPlan<S: ProcessingSample> {
    engine: Box<Engine<S>>,
    transfer: Option<Transfer>,
}
impl<S: ProcessingSample> PreparedPlan<S> {
    pub fn new(engine: Engine<S>) -> Result<Self, PlanSwapError> {
        if engine.timeline != 0 || engine.swaps.is_some() || engine.parameters.is_retired() {
            return Err(PlanSwapError::InvalidCandidate);
        }
        Ok(Self {
            engine: Box::new(engine),
            transfer: None,
        })
    }
    pub fn revision(&self) -> u64 {
        self.engine.revision()
    }
    pub fn snapshot(&self) -> PlanSnapshot {
        self.engine.plan_snapshot()
    }
    /// Explicit reuse asserts compatible static configuration, including IO
    /// backend identity. Type/role/port/schema checks cannot establish that two
    /// user processors with the same Rust type have equivalent configuration.
    /// Reused processors retain their current parameters and ramp progress;
    /// candidate events can then override them at the first active block.
    pub fn with_reuse(
        mut self,
        basis: &PlanSnapshot,
        mappings: &[ProcessorReuse],
    ) -> Result<Self, PlanSwapError> {
        let target = &self.engine.snapshot.0;
        if basis.config() != target.config {
            return Err(PlanSwapError::IncompatibleConfig);
        }
        if basis.epoch() != target.epoch {
            return Err(PlanSwapError::StaleEpoch);
        }
        let mut processors = Vec::with_capacity(mappings.len());
        let mut parameters = Vec::new();
        for mapping in mappings {
            let old = basis
                .0
                .processors
                .iter()
                .position(|p| p.id == mapping.old)
                .ok_or(PlanSwapError::MissingProcessor)?;
            let new = target
                .processors
                .iter()
                .position(|p| p.id == mapping.new)
                .ok_or(PlanSwapError::MissingProcessor)?;
            if processors.iter().any(|&(a, b)| a == old || b == new) {
                return Err(PlanSwapError::DuplicateReuse);
            }
            let a = &basis.0.processors[old];
            let b = &target.processors[new];
            if a.state_type != b.state_type
                || a.role != b.role
                || a.latency != b.latency
                || a.io != b.io
            {
                return Err(PlanSwapError::IncompatibleProcessor);
            }
            let old_specs: Vec<_> = basis
                .0
                .parameters
                .iter()
                .enumerate()
                .filter(|(_, p)| p.key.processor == mapping.old)
                .collect();
            let new_specs: Vec<_> = target
                .parameters
                .iter()
                .enumerate()
                .filter(|(_, p)| p.key.processor == mapping.new)
                .collect();
            if old_specs.len() != new_specs.len() {
                return Err(PlanSwapError::IncompatibleParameters);
            }
            for (old_slot, old_spec) in old_specs {
                let &(new_slot, new_spec) = new_specs
                    .iter()
                    .find(|(_, p)| p.key.parameter == old_spec.key.parameter)
                    .ok_or(PlanSwapError::IncompatibleParameters)?;
                if old_spec.domain != new_spec.domain {
                    return Err(PlanSwapError::IncompatibleParameters);
                }
                parameters.push((old_slot, new_slot));
            }
            processors.push((old, new));
        }
        self.transfer = if processors.is_empty() {
            None
        } else {
            Some(Transfer {
                basis: basis.clone(),
                processors: processors.into_boxed_slice(),
                parameters: parameters.into_boxed_slice(),
            })
        };
        Ok(self)
    }
}

pub struct PublishFailure<S: ProcessingSample> {
    pub reason: PlanSwapError,
    pub plan: PreparedPlan<S>,
}
impl<S: ProcessingSample> fmt::Debug for PublishFailure<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublishFailure")
            .field("reason", &self.reason)
            .field("revision", &self.plan.revision())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetireOutcome {
    Replaced { active_revision: u64, frame: u64 },
    Rejected(PlanSwapError),
}
/// Drop only on the control thread. Rejected candidates use the same queue so
/// even cancellation or a stale reuse basis cannot destroy objects in render.
pub struct RetiredPlan<S: ProcessingSample> {
    plan: PreparedPlan<S>,
    outcome: RetireOutcome,
}
impl<S: ProcessingSample> RetiredPlan<S> {
    pub fn revision(&self) -> u64 {
        self.plan.revision()
    }
    pub fn outcome(&self) -> RetireOutcome {
        self.outcome
    }
    /// Number of accepted requests still waiting for StaleRevision replies.
    /// Drain the old ControlPort and retry until this returns zero before drop.
    pub fn reject_pending(&mut self) -> usize {
        self.plan.engine.parameters.reject_pending()
    }
}

struct Status {
    revision: AtomicU64,
    cancel: AtomicU64,
}
pub(super) struct RtPlanPort<S: ProcessingSample> {
    pending: Consumer<PreparedPlan<S>>,
    retired: Producer<RetiredPlan<S>>,
    status: Arc<Status>,
}
pub struct PlanControlPort<S: ProcessingSample> {
    pending: Producer<PreparedPlan<S>>,
    retired: Consumer<RetiredPlan<S>>,
    status: Arc<Status>,
    config: EngineConfig,
    epoch: u64,
    last_revision: u64,
}
impl<S: ProcessingSample> PlanControlPort<S> {
    pub fn active_revision(&self) -> u64 {
        self.status.revision.load(Ordering::Acquire)
    }
    /// Failure retains the candidate for control-side retry or destruction.
    pub fn publish(&mut self, plan: PreparedPlan<S>) -> Result<(), PublishFailure<S>> {
        let reason = if self.pending.is_abandoned() {
            Some(PlanSwapError::Disconnected)
        } else if self.config != plan.engine.config() {
            Some(PlanSwapError::IncompatibleConfig)
        } else if self.epoch != plan.engine.epoch() {
            Some(PlanSwapError::StaleEpoch)
        } else if plan.revision() <= self.last_revision {
            Some(PlanSwapError::StaleRevision)
        } else if self.pending.is_full() {
            Some(PlanSwapError::PendingFull)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(PublishFailure { reason, plan });
        }
        self.last_revision = plan.revision();
        self.pending
            .push(plan)
            .unwrap_or_else(|_| unreachable!("single producer reserved pending slot"));
        Ok(())
    }
    /// Requests cancellation of the most recently published revision. An
    /// already committed swap wins the race; inspect RetireOutcome to confirm.
    pub fn cancel_pending(&mut self) {
        self.status
            .cancel
            .store(self.last_revision, Ordering::Release);
    }
    pub fn poll_retired(&mut self) -> Option<RetiredPlan<S>> {
        self.retired.pop().ok()
    }
}

impl<S: ProcessingSample> Engine<S> {
    /// Non-RT only; take this before transferring Engine to the render owner.
    pub fn plan_snapshot(&self) -> PlanSnapshot {
        self.snapshot.clone()
    }
    /// Non-RT preparation. After render stops, move this Engine back to control
    /// before dropping it or calling into_parts, including when control closed.
    pub fn enable_plan_switching(
        &mut self,
        retire_capacity: usize,
    ) -> Result<PlanControlPort<S>, PlanSwapError> {
        if retire_capacity == 0 {
            return Err(PlanSwapError::InvalidCapacity);
        }
        if self.swaps.is_some() {
            return Err(PlanSwapError::AlreadyEnabled);
        }
        let (tx, rx) = RingBuffer::new(1);
        let (retired_tx, retired_rx) = RingBuffer::new(retire_capacity);
        let status = Arc::new(Status {
            revision: AtomicU64::new(self.revision()),
            cancel: AtomicU64::new(self.revision()),
        });
        self.swaps = Some(RtPlanPort {
            pending: rx,
            retired: retired_tx,
            status: Arc::clone(&status),
        });
        Ok(PlanControlPort {
            pending: tx,
            retired: retired_rx,
            status,
            config: self.config(),
            epoch: self.epoch(),
            last_revision: self.revision(),
        })
    }
    pub(super) fn apply_pending_plan(&mut self) {
        let Some(port) = &mut self.swaps else {
            return;
        };
        // Never take ownership without a guaranteed return slot. When control
        // disappears, leave both queues owned until Engine returns after stop.
        if port.retired.is_abandoned() || port.retired.is_full() {
            return;
        }
        let Ok(mut candidate) = port.pending.pop() else {
            return;
        };
        let rejection = if port.status.cancel.load(Ordering::Acquire) == candidate.revision() {
            Some(PlanSwapError::Cancelled)
        } else if candidate
            .transfer
            .as_ref()
            .is_some_and(|transfer| !Arc::ptr_eq(&transfer.basis.0, &self.snapshot.0))
        {
            Some(PlanSwapError::StaleBasis)
        } else {
            None
        };
        let outcome = if let Some(reason) = rejection {
            RetireOutcome::Rejected(reason)
        } else {
            if let Some(transfer) = &candidate.transfer {
                for &(old, new) in &transfer.processors {
                    mem::swap(
                        &mut self.resources.instances[old].processor,
                        &mut candidate.engine.resources.instances[new].processor,
                    );
                }
                candidate
                    .engine
                    .parameters
                    .copy_states_from(&self.parameters, &transfer.parameters);
            }
            mem::swap(&mut self.plan, &mut candidate.engine.plan);
            mem::swap(&mut self.resources, &mut candidate.engine.resources);
            mem::swap(&mut self.parameters, &mut candidate.engine.parameters);
            mem::swap(&mut self.snapshot, &mut candidate.engine.snapshot);
            candidate.engine.timeline = self.timeline;
            port.status
                .revision
                .store(self.plan.revision, Ordering::Release);
            RetireOutcome::Replaced {
                active_revision: self.plan.revision,
                frame: self.timeline,
            }
        };
        candidate.engine.parameters.retire();
        port.retired
            .push(RetiredPlan {
                plan: candidate,
                outcome,
            })
            .unwrap_or_else(|_| unreachable!("single producer reserved retire slot"));
    }
}
