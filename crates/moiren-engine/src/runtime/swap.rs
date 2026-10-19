//! Non-RT preparation and bounded ownership transfer at the render boundary.
use super::*;
use crate::{buffer::IoMode, control::ParamSpec};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    fmt, mem,
    sync::atomic::{AtomicU64, Ordering},
};

mod reservation;

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
    #[error("a reserved or protected IO instance has no matching reuse transfer")]
    MissingReuse,
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
    #[error("reused processor signal topic, shape or capacity differs")]
    IncompatibleSignals,
    #[error("candidate requires a stable attached signal host")]
    MissingSignalHost,
    #[error("an attached host requires a prepared candidate signal update")]
    MissingSignalPlan,
    #[error(transparent)]
    SignalPreparation(#[from] crate::signal::SignalPlanError),
    #[error(transparent)]
    SignalActivation(#[from] moiren_signal::ActivateError),
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    use crate::{
        buffer::{BufferSlotLayout, PortAccess, ProcessIo},
        control::{ProcessParameters, parameter_channel},
    };
    use moiren_signal::{
        BlockInit, Bus, ErasedBinding, HostControl, LatestValue, PreparedPublisher, PublishError,
    };

    struct ManifestSource(Box<[ErasedBinding]>);
    impl RtProcessor<f64> for ManifestSource {
        fn role(&self) -> ProcessorRole {
            ProcessorRole::Source
        }
        fn validate_io(&self, _: &PreparedIo) -> Result<(), ProcessorError> {
            Ok(())
        }
        fn signal_bindings(&self) -> &[ErasedBinding] {
            &self.0
        }
        fn process(
            &mut self,
            _: &ProcessContext,
            _: ProcessIo<'_, f64>,
            _: ProcessParameters<'_>,
            _: ProcessSignals<'_>,
        ) {
        }
    }

    // Integration tests keep the parameterized twin of this scaffolding in
    // `tests/common/mod.rs::test_engine`; this copy stays because embedded
    // `#[cfg(test)]` modules cannot import from the test binaries.
    fn engine(
        host: &HostControl,
        bindings: Vec<ErasedBinding>,
        revision: u64,
        id: u64,
    ) -> Engine<f64> {
        let resources = RtResources::new(vec![ProcessorInstance::new(
            ProcessorId(id),
            ManifestSource(bindings.into_boxed_slice()),
        )])
        .unwrap();
        let (_, parameters) = parameter_channel(vec![], revision, 3, 4, 1024).unwrap();
        let arena = BufferArena::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 8,
            }],
            4096,
        )
        .unwrap();
        let io = arena
            .prepare_io(&[PortAccess::Write {
                port: 0,
                slot: arena.slot(0).unwrap(),
            }])
            .unwrap();
        let plan = ExecutionPlan::prepare_with_signals(
            arena,
            vec![OpSpec {
                processor: ProcessorId(id),
                io,
            }],
            &resources,
            &parameters,
            EngineConfig {
                processing_sr: 48000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
            host,
        )
        .unwrap();
        Engine::new(plan, resources, parameters).unwrap()
    }

    #[test]
    fn c3_rejected_update_moves_out_of_candidate_into_retirement() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_latest_value("retired-update", 0_u64).unwrap();
        let (control, host) = bus.create_host(1).unwrap();
        let publisher = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let mut active = engine(&control, vec![publisher.erase()], 1, 10);
        active.attach_signal_host(host).unwrap();
        let mut plans = active.enable_plan_switching(1).unwrap();
        let candidate =
            PreparedPlan::new(engine(&control, vec![publisher.erase()], 2, 20)).unwrap();
        plans.publish(candidate).unwrap();
        plans.cancel_pending();
        active.render(1).unwrap();
        let retired = plans.poll_retired().unwrap();
        assert_eq!(
            retired.outcome(),
            RetireOutcome::Rejected(PlanSwapError::Cancelled)
        );
        assert!(retired.plan.engine.plan.signal_update.is_none());
        assert!(retired._prepared_signals.is_some());
        drop(retired);
        let basis = active.plan_snapshot();
        let stale = PreparedPlan::new(engine(&control, vec![publisher.erase()], 4, 40)).unwrap();
        plans
            .publish(PreparedPlan::new(engine(&control, vec![publisher.erase()], 3, 30)).unwrap())
            .unwrap();
        active.render(1).unwrap();
        drop(plans.poll_retired().unwrap());
        let check = |retired: RetiredPlan<f64>, reason| {
            assert_eq!(retired.outcome(), RetireOutcome::Rejected(reason));
            assert!(retired.plan.engine.plan.signal_update.is_none());
            assert!(retired._prepared_signals.is_some());
        };
        plans.publish(stale).unwrap();
        active.render(1).unwrap();
        check(
            plans.poll_retired().unwrap(),
            PlanSwapError::SignalActivation(moiren_signal::ActivateError::StaleBasis),
        );
        let stale = PreparedPlan::new(engine(&control, vec![publisher.erase()], 5, 50))
            .unwrap()
            .with_reuse(
                &basis,
                &[ProcessorReuse {
                    old: ProcessorId(10),
                    new: ProcessorId(50),
                }],
            )
            .unwrap();
        plans.publish(stale).unwrap();
        active.render(1).unwrap();
        check(plans.poll_retired().unwrap(), PlanSwapError::StaleBasis);
        let other = bus
            .register_latest_value("retired-update/foreign", 0_u64)
            .unwrap();
        let (foreign_control, _foreign_host) = bus.create_host(1).unwrap();
        let foreign = foreign_control
            .prepare(&other, foreign_control.new_binding().unwrap())
            .unwrap();
        plans
            .publish(
                PreparedPlan::new(engine(&foreign_control, vec![foreign.erase()], 6, 60)).unwrap(),
            )
            .unwrap();
        active.render(1).unwrap();
        check(
            plans.poll_retired().unwrap(),
            PlanSwapError::SignalActivation(moiren_signal::ActivateError::ForeignHost),
        );
        assert_eq!(active.revision(), 3);
    }

    #[test]
    fn c3_old_operation_table_is_inactive_but_retained_handle_has_actual_new_authority() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_latest_value("authority", 0_u64).unwrap();
        let _reader = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, host) = bus.create_host(2).unwrap();
        let original: PreparedPublisher<LatestValue<u64>> = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let mut active = engine(&control, vec![original.erase()], 1, 10);
        active.attach_signal_host(host).unwrap();
        let old = active.plan.ops[0]
            .signals
            .as_ref()
            .unwrap()
            .for_operation(&[original.erase()])
            .unwrap();
        let basis = active.plan_snapshot();
        let disposable = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let mut plans = active.enable_plan_switching(1).unwrap();
        let candidate = PreparedPlan::new(engine(&control, vec![disposable.erase()], 2, 20))
            .unwrap()
            .with_reuse(
                &basis,
                &[ProcessorReuse {
                    old: ProcessorId(10),
                    new: ProcessorId(20),
                }],
            )
            .unwrap();
        assert_eq!(
            candidate.snapshot().0.processors[0].signals[0].key(),
            original.erase().key()
        );
        assert_ne!(
            candidate.snapshot().0.processors[0].signals[0].key(),
            disposable.erase().key()
        );
        let handle = original.handle();
        active.render(1).unwrap();
        let generation = active
            .signal_host
            .as_mut()
            .unwrap()
            .scope(&old)
            .demand(&handle)
            .unwrap()
            .generation;
        plans.publish(candidate).unwrap();
        active.render(1).unwrap();
        let host = active.signal_host.as_mut().unwrap();
        assert_eq!(
            host.scope(&old).publish(&handle, 11),
            Err(PublishError::InactiveBinding)
        );
        let mut scope = host.scope(active.plan.ops[0].signals.as_ref().unwrap());
        assert_eq!(scope.demand(&handle).unwrap().generation, generation);
        assert!(matches!(
            scope.publish(&handle, 12),
            Ok(moiren_signal::PublishOutcome::Published(_))
        ));
        assert_eq!(
            scope.publish(&disposable.handle(), 13),
            Err(PublishError::InactiveBinding)
        );
    }

    #[test]
    fn c3_reuse_checks_actual_topic_type_mode_capacity_and_binding_count() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "schema/original",
                BlockInit {
                    header: 0_u64,
                    element: 0_f32,
                    capacity: 2,
                },
            )
            .unwrap();
        let other_topic = bus
            .register_latest_block(
                "schema/other",
                BlockInit {
                    header: 0_u64,
                    element: 0_f32,
                    capacity: 2,
                },
            )
            .unwrap();
        let other_type = bus
            .register_latest_block(
                "schema/type",
                BlockInit {
                    header: 0_u64,
                    element: 0_f64,
                    capacity: 2,
                },
            )
            .unwrap();
        let other_mode = bus
            .register_stream_block(
                "schema/mode",
                BlockInit {
                    header: 0_u64,
                    element: 0_f32,
                    capacity: 2,
                },
                4,
            )
            .unwrap();
        let other_capacity = bus
            .register_latest_block(
                "schema/capacity",
                BlockInit {
                    header: 0_u64,
                    element: 0_f32,
                    capacity: 3,
                },
            )
            .unwrap();
        let (control, _host) = bus.create_host(8).unwrap();
        let original = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let active = engine(&control, vec![original.erase()], 1, 10);
        let basis = active.plan_snapshot();
        let different = [
            control
                .prepare(&other_topic, control.new_binding().unwrap())
                .unwrap()
                .erase(),
            control
                .prepare(&other_type, control.new_binding().unwrap())
                .unwrap()
                .erase(),
            control
                .prepare(&other_mode, control.new_binding().unwrap())
                .unwrap()
                .erase(),
            control
                .prepare(&other_capacity, control.new_binding().unwrap())
                .unwrap()
                .erase(),
        ];
        for binding in different {
            let candidate = PreparedPlan::new(engine(&control, vec![binding], 2, 20)).unwrap();
            assert!(matches!(
                candidate.with_reuse(
                    &basis,
                    &[ProcessorReuse {
                        old: ProcessorId(10),
                        new: ProcessorId(20)
                    }]
                ),
                Err(PlanSwapError::IncompatibleSignals)
            ));
        }
        let candidate = PreparedPlan::new(engine(&control, vec![], 2, 20)).unwrap();
        assert!(matches!(
            candidate.with_reuse(
                &basis,
                &[ProcessorReuse {
                    old: ProcessorId(10),
                    new: ProcessorId(20)
                }]
            ),
            Err(PlanSwapError::IncompatibleSignals)
        ));
    }

    #[test]
    fn c3_empty_update_clears_permissions_and_leases_retire_off_rt() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_latest_value("empty-update", 0_u64).unwrap();
        let (control, host) = bus.create_host(2).unwrap();
        let publisher = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let mut active = engine(&control, vec![publisher.erase()], 1, 10);
        active.attach_signal_host(host).unwrap();
        let basis = active.plan_snapshot();
        let mut plans = active.enable_plan_switching(1).unwrap();
        plans
            .publish(PreparedPlan::new(engine(&control, vec![], 2, 20)).unwrap())
            .unwrap();
        active.render(1).unwrap();
        assert_eq!(
            topic.status().publisher,
            moiren_signal::PublisherStatus::NoPublisher
        );
        assert_eq!(
            bus.unregister(&topic),
            Err(moiren_signal::UnregisterError::Busy)
        );
        let retired = plans.poll_retired().unwrap();
        drop(publisher);
        drop(retired);
        assert_eq!(
            bus.unregister(&topic),
            Err(moiren_signal::UnregisterError::Busy)
        );
        drop(basis);
        bus.unregister(&topic).unwrap();
    }
}

#[derive(Clone, PartialEq, Eq)]
struct IoSchema {
    mode: IoMode,
    inputs: Box<[(u16, usize)]>,
    outputs: Box<[(u16, usize)]>,
}

/// Single source for the (config, epoch, revision) compatibility ordering
/// shared by the three swap gates: control-side [`PlanControlPort::publish`],
/// RT-side [`Engine::apply_pending_plan`], and the reuse-basis check in
/// [`PreparedPlan::prepare_reuse`]. A replacing plan must preserve the exact
/// processing configuration and timeline epoch of the plan it replaces and
/// carry a strictly greater revision. The control-side and RT-side checks are
/// deliberately both kept: control validates before queueing, RT re-validates
/// before committing, so a racing or stale control side can never activate an
/// incompatible candidate.
fn swap_rejection(
    active: (&EngineConfig, u64, u64),
    replacement: (&EngineConfig, u64, u64),
) -> Option<PlanSwapError> {
    let (config, epoch, revision) = active;
    let (replacement_config, replacement_epoch, replacement_revision) = replacement;
    if config != replacement_config {
        Some(PlanSwapError::IncompatibleConfig)
    } else if epoch != replacement_epoch {
        Some(PlanSwapError::StaleEpoch)
    } else if replacement_revision <= revision {
        Some(PlanSwapError::StaleRevision)
    } else {
        None
    }
}

impl IoSchema {
    /// Canonical port snapshot: sorted so schemas captured at preparation and
    /// re-derived by a reservation's IO check compare structurally.
    fn from_io(io: &PreparedIo) -> Self {
        let mut inputs: Vec<_> = io.input_ports().collect();
        let mut outputs: Vec<_> = io.output_ports().collect();
        inputs.sort_unstable();
        outputs.sort_unstable();
        Self {
            mode: io.mode(),
            inputs: inputs.into_boxed_slice(),
            outputs: outputs.into_boxed_slice(),
        }
    }
}
#[derive(Clone)]
struct ProcessorSchema {
    id: ProcessorId,
    state_type: TypeId,
    role: ProcessorRole,
    latency: u32,
    io: Option<IoSchema>,
    signals: Box<[moiren_signal::ErasedBinding]>,
}
#[derive(Clone)]
struct Snapshot {
    processors: Box<[ProcessorSchema]>,
    parameters: Arc<[ParamSpec]>,
    config: EngineConfig,
    revision: u64,
    epoch: u64,
    signal_host: Option<moiren_signal::HostId>,
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
                let io = plan
                    .ops
                    .iter()
                    .find(|op| op.runtime == index)
                    .map(|op| IoSchema::from_io(&op.io));
                ProcessorSchema {
                    id: instance.id,
                    state_type: instance.state_type,
                    role: instance.processor.role(),
                    latency: instance.processor.latency_frames(),
                    io,
                    signals: plan
                        .ops
                        .iter()
                        .position(|op| op.runtime == index)
                        .map(|op| plan.signal_bindings[op].clone())
                        .unwrap_or_else(|| {
                            instance
                                .processor
                                .signal_bindings()
                                .to_vec()
                                .into_boxed_slice()
                        }),
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
            signal_host: plan
                .signal_control
                .as_ref()
                .map(moiren_signal::HostControl::id),
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
    original_snapshot: PlanSnapshot,
}
impl<S: ProcessingSample> PreparedPlan<S> {
    pub fn new(engine: Engine<S>) -> Result<Self, PlanSwapError> {
        if engine.timeline != 0
            || engine.swaps.is_some()
            || engine.parameters.is_retired()
            || engine.signal_host.is_some()
        {
            return Err(PlanSwapError::InvalidCandidate);
        }
        let original_snapshot = engine.plan_snapshot();
        Ok(Self {
            engine: Box::new(engine),
            transfer: None,
            original_snapshot,
        })
    }
    pub fn revision(&self) -> u64 {
        self.engine.revision()
    }
    pub fn snapshot(&self) -> PlanSnapshot {
        self.engine.plan_snapshot()
    }

    /// Keep the reused kernel but start selected parameters from their prepared
    /// values. Apply after `with_reuse`, off RT. Other parameters retain ramps.
    pub fn with_initial_parameters(mut self, keys: &[moiren_core::protocol::ParameterKey]) -> Self {
        if let Some(transfer) = &mut self.transfer {
            let schema = self.engine.parameters.schema();
            transfer.parameters = transfer
                .parameters
                .iter()
                .copied()
                .filter(|&(_, new)| !keys.contains(&schema[new].key))
                .collect();
        }
        self
    }

    /// Non-RT reclamation for an unpublished or stopped pending candidate.
    /// Keep its ControlPort alive and drain replies until this returns zero.
    pub fn retire_controls(&mut self) -> usize {
        self.engine.retire_controls()
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
        if let Err(error) = self.prepare_reuse(basis, mappings) {
            let remaining = self.engine.retire_controls();
            debug_assert_eq!(
                remaining, 0,
                "unrendered candidate replies fit their paired queue"
            );
            return Err(error);
        }
        Ok(self)
    }

    fn prepare_reuse(
        &mut self,
        basis: &PlanSnapshot,
        mappings: &[ProcessorReuse],
    ) -> Result<(), PlanSwapError> {
        if self.engine.parameters.is_retired() {
            return Err(PlanSwapError::InvalidCandidate);
        }
        for instance in &self.engine.resources.instances {
            if let Some((reserved_basis, previous)) = &instance.reservation {
                if !Arc::ptr_eq(&reserved_basis.0, &basis.0) {
                    return Err(PlanSwapError::StaleBasis);
                }
                if !mappings
                    .iter()
                    .any(|mapping| mapping.old == *previous && mapping.new == instance.id)
                {
                    return Err(PlanSwapError::MissingReuse);
                }
            }
        }
        let target = &self.original_snapshot.0;
        // Same compatibility ordering as publish/apply; any candidate that
        // passed an unordered revision here would still be rejected before
        // activation, so checking it with the shared source adds no new
        // reachable rejection.
        if let Some(reason) = swap_rejection(
            (&basis.0.config, basis.0.epoch, basis.0.revision),
            (&target.config, target.epoch, target.revision),
        ) {
            return Err(reason);
        }
        if basis.0.signal_host != target.signal_host {
            return Err(PlanSwapError::IncompatibleSignals);
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
            if !PreparedSignalPlan::compatible_bindings(&a.signals, &b.signals) {
                return Err(PlanSwapError::IncompatibleSignals);
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
        {
            let mut snapshot = (**target).clone();
            for &(old, new) in &processors {
                snapshot.processors[new].signals = basis.0.processors[old].signals.clone();
            }
            if let Some(host) = &self.engine.plan.signal_control {
                let bindings: Box<[_]> = self
                    .engine
                    .plan
                    .ops
                    .iter()
                    .map(|op| snapshot.processors[op.runtime].signals.clone())
                    .collect();
                let prepared = PreparedSignalPlan::prepare(host, &bindings)?;
                let (update, operations) = prepared.into_parts();
                self.engine.plan.signal_update = Some(update);
                self.engine.plan.signal_bindings = bindings;
                for (op, signals) in self.engine.plan.ops.iter_mut().zip(operations) {
                    op.signals = signals;
                }
            }
            self.engine.snapshot = PlanSnapshot(Arc::new(snapshot));
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
        Ok(())
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
    _signals: Option<moiren_signal::RetiredHostUpdate>,
    _prepared_signals: Option<moiren_signal::PreparedHostUpdate>,
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
    protected_sinks: Vec<usize>,
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
        } else if plan.engine.parameters.is_retired() {
            Some(PlanSwapError::InvalidCandidate)
        } else if let Some(reason) = swap_rejection(
            (&self.config, self.epoch, self.last_revision),
            (&plan.engine.config(), plan.engine.epoch(), plan.revision()),
        ) {
            Some(reason)
        } else if plan
            .engine
            .resources
            .instances
            .iter()
            .enumerate()
            .any(|(slot, instance)| {
                instance.reservation.is_some()
                    && !plan.transfer.as_ref().is_some_and(|transfer| {
                        transfer.processors.iter().any(|&(_, new)| new == slot)
                    })
            })
        {
            Some(PlanSwapError::MissingReuse)
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

    /// Call only after the audio owner has joined. Pending candidates still
    /// own accepted controls and must be returned intact for terminal replies.
    pub fn take_pending_plan(&mut self) -> Option<PreparedPlan<S>> {
        let candidate = self.swaps.as_mut()?.pending.pop().ok()?;
        candidate.engine.parameters.retire();
        Some(candidate)
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
        if self
            .resources
            .instances
            .iter()
            .any(|instance| instance.reservation.is_some())
        {
            return Err(PlanSwapError::InvalidCandidate);
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
            protected_sinks: Vec::new(),
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

    /// Non-RT setup for a demand-driven renderer whose consumer stays fixed.
    /// Every later activation must migrate this exact live sink instance.
    /// Calls accumulate, protecting all native output consumers in a fan-out.
    pub fn require_sink_reuse(&mut self, sink: ProcessorId) -> Result<(), PlanSwapError> {
        if self.timeline != 0 {
            return Err(PlanSwapError::InvalidCandidate);
        }
        let slot = self
            .resources
            .instances
            .iter()
            .position(|instance| instance.id == sink)
            .ok_or(PlanSwapError::MissingProcessor)?;
        if self.resources.instances[slot].processor.role() != ProcessorRole::Sink {
            return Err(PlanSwapError::IncompatibleProcessor);
        }
        let sinks = &mut self
            .swaps
            .as_mut()
            .ok_or(PlanSwapError::InvalidCandidate)?
            .protected_sinks;
        if !sinks.contains(&slot) {
            sinks.push(slot);
        }
        Ok(())
    }
    pub(super) fn apply_pending_plan(&mut self) {
        // The port moves out for the duration so each phase below can borrow
        // the engine directly; it is restored on every exit path. Nothing
        // else runs while this audio callback holds `&mut self`.
        let Some(mut port) = self.swaps.take() else {
            return;
        };
        // Never take ownership without a guaranteed return slot. When control
        // disappears, leave both queues owned until Engine returns after stop.
        if !port.retired.is_abandoned()
            && !port.retired.is_full()
            && let Ok(mut candidate) = port.pending.pop()
        {
            let rejection = Self::plan_swap_rejection(self, &port, &candidate);
            let activation = Self::activate_plan_signals(self, rejection, &mut candidate);
            let (outcome, retired_signals) = match activation {
                Err(reason) => (RetireOutcome::Rejected(reason), None),
                Ok(retired_signals) => {
                    Self::commit_plan_swap(self, &mut port, &mut candidate, retired_signals)
                }
            };
            retire_swapped_plan(&mut port, candidate, outcome, retired_signals);
        }
        self.swaps = Some(port);
    }

    /// Phase 1 — guard evaluation. Reads the shared cancel word and re-checks
    /// the candidate against the live plan; this is the RT-side twin of the
    /// compatibility ordering control already applied in `publish`, kept as a
    /// deliberate second line of defense.
    #[inline]
    fn plan_swap_rejection(
        &self,
        port: &RtPlanPort<S>,
        candidate: &PreparedPlan<S>,
    ) -> Option<PlanSwapError> {
        if port.status.cancel.load(Ordering::Acquire) == candidate.revision() {
            Some(PlanSwapError::Cancelled)
        } else if let Some(reason) = swap_rejection(
            (&self.plan.config, self.plan.epoch, self.plan.revision),
            (
                &candidate.engine.plan.config,
                candidate.engine.plan.epoch,
                candidate.engine.plan.revision,
            ),
        ) {
            Some(reason)
        } else if candidate
            .transfer
            .as_ref()
            .is_some_and(|transfer| !Arc::ptr_eq(&transfer.basis.0, &self.snapshot.0))
        {
            Some(PlanSwapError::StaleBasis)
        } else if port.protected_sinks.iter().any(|&sink| {
            !candidate
                .transfer
                .as_ref()
                .is_some_and(|transfer| transfer.processors.iter().any(|&(old, _)| old == sink))
        }) {
            Some(PlanSwapError::MissingReuse)
        } else {
            None
        }
    }

    /// Phase 2 — signal activation. Commits the candidate's prepared signal
    /// update against the live host; the retired half of the activation
    /// transaction is carried out with the retired plan.
    #[inline]
    fn activate_plan_signals(
        &mut self,
        rejection: Option<PlanSwapError>,
        candidate: &mut PreparedPlan<S>,
    ) -> Result<Option<moiren_signal::RetiredHostUpdate>, PlanSwapError> {
        if let Some(reason) = rejection {
            return Err(reason);
        }
        match (
            &mut self.signal_host,
            &mut candidate.engine.plan.signal_update,
        ) {
            (Some(host), Some(update)) => host
                .activation(update)
                .map(Some)
                .map_err(PlanSwapError::SignalActivation)
                .map(|transaction| transaction.map(moiren_signal::ActivationTransaction::commit)),
            (Some(_), None) => Err(PlanSwapError::MissingSignalPlan),
            (None, Some(_)) => Err(PlanSwapError::MissingSignalHost),
            (None, None) => Ok(None),
        }
    }

    /// Phase 3 — transfer and swap. Migrates reused kernels and parameter
    /// ramp state in place, then swaps plan ownership wholesale. Everything
    /// moved was prepared off RT; the index surgery on protected sinks exists
    /// so a sink migration never allocates on the audio thread.
    #[inline]
    fn commit_plan_swap(
        &mut self,
        port: &mut RtPlanPort<S>,
        candidate: &mut PreparedPlan<S>,
        retired_signals: Option<moiren_signal::RetiredHostUpdate>,
    ) -> (RetireOutcome, Option<moiren_signal::RetiredHostUpdate>) {
        if let Some(transfer) = &candidate.transfer {
            for sink in &mut port.protected_sinks {
                *sink = transfer
                    .processors
                    .iter()
                    .find_map(|&(old, new)| (old == *sink).then_some(new))
                    .expect("all protected sinks validated above");
            }
            for &(old, new) in &transfer.processors {
                mem::swap(
                    &mut self.resources.instances[old].processor,
                    &mut candidate.engine.resources.instances[new].processor,
                );
                mem::swap(
                    &mut self.resources.instances[old].reservation,
                    &mut candidate.engine.resources.instances[new].reservation,
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
        (
            RetireOutcome::Replaced {
                active_revision: self.plan.revision,
                frame: self.timeline,
            },
            retired_signals,
        )
    }
}

/// Phase 4 — retirement enqueue. The replaced engine retires its accepted
/// controls and both signal-update halves leave the audio thread with it;
/// the queue slot was reserved before the candidate was ever popped.
#[inline]
fn retire_swapped_plan<S: ProcessingSample>(
    port: &mut RtPlanPort<S>,
    mut candidate: PreparedPlan<S>,
    outcome: RetireOutcome,
    retired_signals: Option<moiren_signal::RetiredHostUpdate>,
) {
    candidate.engine.parameters.retire();
    let prepared_signals = candidate.engine.plan.signal_update.take();
    port.retired
        .push(RetiredPlan {
            plan: candidate,
            outcome,
            _signals: retired_signals,
            _prepared_signals: prepared_signals,
        })
        .unwrap_or_else(|_| unreachable!("single producer reserved retire slot"));
}
