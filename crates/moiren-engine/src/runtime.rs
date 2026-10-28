//! A low-level linear executor, not a LogicalGraph compiler or liveness planner.
use crate::{
    buffer::{BufferArena, BufferError, PreparedIo},
    control::{ParameterBindings, ParameterRuntime},
    processor::{ProcessContext, ProcessorError, ProcessorRole, RtProcessor},
    sample::ProcessingSample,
    signal::{PreparedSignalPlan, ProcessSignals},
};
use moiren_core::protocol::ProcessorId;
use std::{any::TypeId, sync::Arc};
use thiserror::Error;

mod mix;
mod swap;
use mix::{PreparedMixInput, process_mix};
pub use swap::{
    PlanControlPort, PlanSnapshot, PlanSwapError, PreparedPlan, ProcessorReuse, PublishFailure,
    RetireOutcome, RetiredPlan,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineConfig {
    pub processing_sr: f64,
    pub max_block_frames: usize,
    pub max_events_per_block: usize,
}
impl EngineConfig {
    /// Single owner of the engine bounds. Compilation and plan preparation
    /// route through here so neither entry point can accept a config the
    /// other rejects.
    pub(crate) fn validated(&self) -> Result<(), RuntimeError> {
        if !self.processing_sr.is_finite()
            || self.processing_sr <= 0.0
            || self.max_block_frames == 0
            || self.max_block_frames > u32::MAX as usize
            || self.max_events_per_block == 0
        {
            Err(RuntimeError::InvalidConfig)
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RuntimeError {
    #[error("engine configuration has invalid sample rate or block/event bounds")]
    InvalidConfig,
    #[error("requested frames are zero or exceed the prepared block maximum")]
    InvalidFrames,
    #[error("timeline frame counter would overflow u64")]
    TimelineOverflow,
    #[error("two processor instances share one identifier")]
    DuplicateProcessor,
    #[error("op spec references a processor absent from the resources")]
    MissingProcessor,
    #[error("plan was prepared against different resources")]
    ForeignResources,
    #[error("plan was prepared against a different parameter table")]
    ForeignParameters,
    #[error("a reuse reservation must be activated before rendering")]
    UnboundResources,
    #[error("signal bindings require a host control during preparation")]
    SignalsRequireHost,
    #[error("a prepared signal plan requires its host before rendering")]
    MissingSignalHost,
    #[error(transparent)]
    Signal(#[from] crate::signal::SignalPlanError),
    #[error(transparent)]
    Buffer(#[from] BufferError),
    #[error(transparent)]
    Processor(#[from] ProcessorError),
}

pub struct ProcessorInstance<S: ProcessingSample> {
    id: ProcessorId,
    state_type: TypeId,
    processor: Box<dyn RtProcessor<S>>,
    reservation: Option<(PlanSnapshot, ProcessorId)>,
}
impl<S: ProcessingSample> ProcessorInstance<S> {
    pub fn new(id: ProcessorId, processor: impl RtProcessor<S> + 'static) -> Self {
        Self {
            id,
            state_type: processor.state_type_id(),
            processor: Box::new(processor),
            reservation: None,
        }
    }

    /// Single source for "both transports are equivalent for this kernel",
    /// queried by the compiler's in-place candidacy and the swap snapshot's
    /// IO-mode normalization. Control-side only, like every other schema
    /// query; it never touches rendering.
    pub(crate) fn transport_interchangeable(&self) -> bool {
        self.processor.transport_interchangeable()
    }
}
/// Owns persistent DSP state independently from the schedule. Existing slots
/// may be migrated into a compatible prepared plan without reinitialization.
pub struct RtResources<S: ProcessingSample> {
    identity: Arc<()>,
    instances: Box<[ProcessorInstance<S>]>,
}
impl<S: ProcessingSample> RtResources<S> {
    pub fn new(instances: Vec<ProcessorInstance<S>>) -> Result<Self, RuntimeError> {
        for (i, instance) in instances.iter().enumerate() {
            if instances[..i]
                .iter()
                .any(|previous| previous.id == instance.id)
            {
                return Err(RuntimeError::DuplicateProcessor);
            }
        }
        Ok(Self {
            identity: Arc::new(()),
            instances: instances.into_boxed_slice(),
        })
    }
}
pub struct OpSpec {
    pub processor: ProcessorId,
    pub io: PreparedIo,
}
pub(crate) struct FusedMixInputSpec {
    pub processor: ProcessorId,
    pub port: u16,
}
pub(crate) enum FusedOpSpec {
    Mix {
        inputs: Box<[FusedMixInputSpec]>,
        input_count: usize,
    },
}
pub(crate) struct CompiledOpSpec {
    pub processor: ProcessorId,
    pub io: PreparedIo,
    pub fused: Option<FusedOpSpec>,
}
enum PreparedOpKind {
    Processor {
        runtime: usize,
        params: ParameterBindings,
    },
    Mix {
        runtime: usize,
        params: ParameterBindings,
        inputs: Box<[PreparedMixInput]>,
        input_count: usize,
    },
}
struct PreparedOp {
    io: PreparedIo,
    kind: PreparedOpKind,
    // Signal permissions belong to the scheduled operation and are prepared
    // off-thread alongside the processor bindings during plan migration.
    signals: Option<moiren_signal::OpSignalBindings>,
}
impl PreparedOp {
    fn runtime_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let (runtime, inputs): (_, &[PreparedMixInput]) = match &self.kind {
            PreparedOpKind::Processor { runtime, .. } => (*runtime, &[]),
            PreparedOpKind::Mix {
                runtime, inputs, ..
            } => (*runtime, inputs),
        };
        std::iter::once(runtime).chain(inputs.iter().map(|input| input.runtime))
    }
}
pub struct ExecutionPlan<S: ProcessingSample> {
    arena: BufferArena<S>,
    ops: Box<[PreparedOp]>,
    resources: Arc<()>,
    config: EngineConfig,
    revision: u64,
    epoch: u64,
    signal_update: Option<moiren_signal::PreparedHostUpdate>,
    signal_control: Option<moiren_signal::HostControl>,
    signal_bindings: Box<[Box<[moiren_signal::ErasedBinding]>]>,
}
impl<S: ProcessingSample> ExecutionPlan<S> {
    /// Non-RT only. Validates access, schema and table identity, NOT whole-graph
    /// signal liveness. A compiler must still preserve each value through last use.
    pub fn prepare(
        arena: BufferArena<S>,
        specs: Vec<OpSpec>,
        resources: &RtResources<S>,
        parameters: &ParameterRuntime,
        config: EngineConfig,
    ) -> Result<Self, RuntimeError> {
        Self::prepare_inner(arena, specs, resources, parameters, config, None)
    }

    pub fn prepare_with_signals(
        arena: BufferArena<S>,
        specs: Vec<OpSpec>,
        resources: &RtResources<S>,
        parameters: &ParameterRuntime,
        config: EngineConfig,
        host: &moiren_signal::HostControl,
    ) -> Result<Self, RuntimeError> {
        Self::prepare_inner(arena, specs, resources, parameters, config, Some(host))
    }

    pub(crate) fn prepare_inner(
        arena: BufferArena<S>,
        specs: Vec<OpSpec>,
        resources: &RtResources<S>,
        parameters: &ParameterRuntime,
        config: EngineConfig,
        host: Option<&moiren_signal::HostControl>,
    ) -> Result<Self, RuntimeError> {
        let specs = specs
            .into_iter()
            .map(|spec| CompiledOpSpec {
                processor: spec.processor,
                io: spec.io,
                fused: None,
            })
            .collect();
        Self::prepare_compiled_inner(arena, specs, resources, parameters, config, host)
    }

    pub(crate) fn prepare_compiled_inner(
        arena: BufferArena<S>,
        specs: Vec<CompiledOpSpec>,
        resources: &RtResources<S>,
        parameters: &ParameterRuntime,
        config: EngineConfig,
        host: Option<&moiren_signal::HostControl>,
    ) -> Result<Self, RuntimeError> {
        config.validated()?;
        let mut ops = Vec::<PreparedOp>::with_capacity(specs.len());
        let mut signal_bindings = Vec::with_capacity(specs.len());
        for spec in specs {
            if !arena.owns(&spec.io) {
                return Err(RuntimeError::Buffer(BufferError::ForeignAccess));
            }
            if spec.io.max_frames() < config.max_block_frames {
                return Err(RuntimeError::InvalidFrames);
            }
            let used = |index| {
                ops.iter()
                    .any(|op| op.runtime_indices().any(|runtime| runtime == index))
            };
            match spec.fused {
                None => {
                    let processor_id = spec.processor;
                    let index = resources
                        .instances
                        .iter()
                        .position(|r| r.id == processor_id)
                        .ok_or(RuntimeError::MissingProcessor)?;
                    if used(index) {
                        return Err(RuntimeError::DuplicateProcessor);
                    }
                    let processor = &resources.instances[index].processor;
                    if processor.role() == ProcessorRole::Observer && spec.io.output_count() != 0 {
                        return Err(RuntimeError::Processor(ProcessorError::InvalidIo));
                    }
                    processor.validate_io(&spec.io)?;
                    let bindings = parameters.bindings(processor_id);
                    processor.validate_parameters(parameters.view(&bindings))?;
                    signal_bindings.push(processor.signal_bindings().to_vec().into_boxed_slice());
                    ops.push(PreparedOp {
                        kind: PreparedOpKind::Processor {
                            runtime: index,
                            params: bindings,
                        },
                        io: spec.io,
                        signals: None,
                    });
                }
                Some(FusedOpSpec::Mix {
                    inputs,
                    input_count,
                }) => {
                    let bus_id = spec.processor;
                    let bus_index = resources
                        .instances
                        .iter()
                        .position(|r| r.id == bus_id)
                        .ok_or(RuntimeError::MissingProcessor)?;
                    if used(bus_index)
                        || resources.instances[bus_index].processor.role()
                            != ProcessorRole::Transform
                    {
                        return Err(RuntimeError::DuplicateProcessor);
                    }
                    let bus = &resources.instances[bus_index].processor;
                    bus.validate_io(&spec.io)?;
                    let bus_bindings = parameters.bindings(bus_id);
                    bus.validate_parameters(parameters.view(&bus_bindings))?;
                    if input_count > usize::from(u16::MAX) + 1
                        || inputs
                            .iter()
                            .any(|input| usize::from(input.port) >= input_count)
                        || !inputs
                            .iter()
                            .map(|input| input.port)
                            .eq(spec.io.input_ports().map(|(port, _)| port))
                    {
                        return Err(RuntimeError::Processor(ProcessorError::InvalidIo));
                    }
                    let mut prepared: Vec<PreparedMixInput> = Vec::with_capacity(inputs.len());
                    for input in inputs {
                        let index = resources
                            .instances
                            .iter()
                            .position(|r| r.id == input.processor)
                            .ok_or(RuntimeError::MissingProcessor)?;
                        if used(index)
                            || index == bus_index
                            || prepared.iter().any(|input| input.runtime == index)
                            || resources.instances[index].processor.role()
                                != ProcessorRole::Transform
                        {
                            return Err(RuntimeError::DuplicateProcessor);
                        }
                        let processor = &resources.instances[index].processor;
                        if !processor.signal_bindings().is_empty() {
                            return Err(RuntimeError::SignalsRequireHost);
                        }
                        let bindings = parameters.bindings(input.processor);
                        processor.validate_parameters(parameters.view(&bindings))?;
                        prepared.push(PreparedMixInput::new(index, input.port, bindings));
                    }
                    signal_bindings.push(bus.signal_bindings().to_vec().into_boxed_slice());
                    ops.push(PreparedOp {
                        io: spec.io,
                        kind: PreparedOpKind::Mix {
                            runtime: bus_index,
                            params: bus_bindings,
                            inputs: prepared.into_boxed_slice(),
                            input_count,
                        },
                        signals: None,
                    });
                }
            }
        }
        let signal_update = match host {
            Some(host) => {
                let prepared = PreparedSignalPlan::prepare(host, &signal_bindings)?;
                let (update, operations) = prepared.into_parts();
                for (op, signals) in ops.iter_mut().zip(operations) {
                    op.set_signals(signals);
                }
                Some(update)
            }
            None if signal_bindings.iter().any(|bindings| !bindings.is_empty()) => {
                return Err(RuntimeError::SignalsRequireHost);
            }
            None => None,
        };
        Ok(Self {
            arena,
            ops: ops.into_boxed_slice(),
            resources: Arc::clone(&resources.identity),
            config,
            revision: parameters.revision(),
            epoch: parameters.epoch(),
            signal_update,
            signal_control: host.cloned(),
            signal_bindings: signal_bindings.into_boxed_slice(),
        })
    }
}

pub struct Engine<S: ProcessingSample> {
    plan: ExecutionPlan<S>,
    resources: RtResources<S>,
    parameters: ParameterRuntime,
    timeline: u64,
    snapshot: PlanSnapshot,
    swaps: Option<swap::RtPlanPort<S>>,
    signal_host: Option<moiren_signal::RtHost>,
}

/// Stopped runtime ownership returned together for non-RT reclamation.
pub struct EngineParts<S: ProcessingSample> {
    pub plan: ExecutionPlan<S>,
    pub resources: RtResources<S>,
    pub parameters: ParameterRuntime,
    pub signals: Option<moiren_signal::RtHost>,
}

pub struct SignalAttachFailure {
    pub host: moiren_signal::RtHost,
    pub error: SignalAttachError,
}

impl std::fmt::Debug for SignalAttachFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SignalAttachFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SignalAttachError {
    #[error("a signal host is already attached")]
    AlreadyAttached,
    #[error("the engine has no prepared signal plan")]
    NoSignalPlan,
    #[error("the engine has already rendered")]
    AlreadyRendered,
    #[error(transparent)]
    Activate(#[from] moiren_signal::ActivateError),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderReport {
    pub start: u64,
    pub end: u64,
    pub segments: usize,
    pub applied_events: usize,
}

impl<S: ProcessingSample> Engine<S> {
    pub fn new(
        plan: ExecutionPlan<S>,
        resources: RtResources<S>,
        parameters: ParameterRuntime,
    ) -> Result<Self, RuntimeError> {
        if !Arc::ptr_eq(&plan.resources, &resources.identity) {
            return Err(RuntimeError::ForeignResources);
        }
        if plan.revision != parameters.revision()
            || plan.epoch != parameters.epoch()
            || plan
                .ops
                .iter()
                .flat_map(PreparedOp::parameter_bindings)
                .any(|bindings| !parameters.owns(bindings))
        {
            return Err(RuntimeError::ForeignParameters);
        }
        let snapshot = PlanSnapshot::capture(&plan, &resources, &parameters);
        Ok(Self {
            plan,
            resources,
            parameters,
            timeline: 0,
            snapshot,
            swaps: None,
            signal_host: None,
        })
    }
    pub fn timeline(&self) -> u64 {
        self.timeline
    }
    /// Attaches the stable signal host before rendering, on a non-RT thread.
    /// Successful activation releases the host's previous prepared ownership.
    pub fn attach_signal_host(
        &mut self,
        mut host: moiren_signal::RtHost,
    ) -> Result<(), SignalAttachFailure> {
        if self.signal_host.is_some() {
            return Err(SignalAttachFailure {
                host,
                error: SignalAttachError::AlreadyAttached,
            });
        }
        if self.timeline != 0 {
            return Err(SignalAttachFailure {
                host,
                error: SignalAttachError::AlreadyRendered,
            });
        }
        let Some(update) = self.plan.signal_update.take() else {
            return Err(SignalAttachFailure {
                host,
                error: SignalAttachError::NoSignalPlan,
            });
        };
        match host.activate(update) {
            Ok(_retired) => {
                self.signal_host = Some(host);
                Ok(())
            }
            Err(failure) => {
                self.plan.signal_update = Some(failure.update);
                Err(SignalAttachFailure {
                    host,
                    error: failure.error.into(),
                })
            }
        }
    }
    /// Non-RT only, after the output owner has joined. Poll existing replies,
    /// then retire and retry after draining replies until this returns zero.
    /// Keep both this engine and its ControlPort alive throughout reclamation.
    pub fn retire_controls(&mut self) -> usize {
        self.parameters.retire();
        self.parameters.reject_pending()
    }
    /// Prepared processing configuration; does not query devices or allocate.
    pub fn config(&self) -> EngineConfig {
        self.plan.config
    }
    pub fn epoch(&self) -> u64 {
        self.plan.epoch
    }
    pub fn revision(&self) -> u64 {
        self.plan.revision
    }
    pub fn plan_switching_enabled(&self) -> bool {
        self.swaps.is_some()
    }
    pub fn render(&mut self, frames: usize) -> Result<RenderReport, RuntimeError> {
        if self
            .resources
            .instances
            .iter()
            .any(|instance| instance.reservation.is_some())
        {
            return Err(RuntimeError::UnboundResources);
        }
        if frames == 0 || frames > self.plan.config.max_block_frames {
            return Err(RuntimeError::InvalidFrames);
        }
        let start = self.timeline;
        let end = start
            .checked_add(frames as u64)
            .ok_or(RuntimeError::TimelineOverflow)?;
        if self.plan.signal_update.is_some() && self.signal_host.is_none() {
            return Err(RuntimeError::MissingSignalHost);
        }
        self.apply_pending_plan();
        if let Some(host) = &mut self.signal_host {
            host.begin_block();
        }
        // Snapshot availability once: concurrent producer traffic cannot make
        // callback work unbounded or sneak new events into the current batch.
        let budget = self
            .parameters
            .queued()
            .min(self.plan.config.max_events_per_block);
        let mut remaining = budget;
        let mut cursor = start;
        let mut segments = 0;
        while cursor < end {
            let next = self
                .parameters
                .next_segment_end(cursor, end, &mut remaining);
            let count = (next - cursor) as usize;
            let ctx = ProcessContext {
                timeline_epoch: self.plan.epoch,
                timeline_start: cursor,
                frames: count,
                processing_sr: self.plan.config.processing_sr,
            };
            let window = (cursor - start) as usize..(next - start) as usize;
            for op in &mut self.plan.ops {
                match &mut op.kind {
                    PreparedOpKind::Processor {
                        runtime,
                        params: bindings,
                    } => {
                        let processor = &mut self.resources.instances[*runtime].processor;
                        let params = self.parameters.view(bindings);
                        let mut empty = ();
                        let signals = match (&mut self.signal_host, &op.signals) {
                            (Some(host), Some(bindings)) => {
                                ProcessSignals::from_scope(host.scope(bindings))
                            }
                            _ => ProcessSignals::empty(&mut empty),
                        };
                        self.plan.arena.with_io(&op.io, window.clone(), |mut io| {
                            io.clear_outputs();
                            processor.process(&ctx, io, params, signals);
                        })?;
                    }
                    PreparedOpKind::Mix { inputs, .. } => {
                        self.plan.arena.with_io(&op.io, window.clone(), |io| {
                            process_mix(io, inputs, &self.parameters);
                        })?;
                    }
                }
            }
            self.parameters.advance(count);
            cursor = next;
            segments += 1;
        }
        self.timeline = end;
        Ok(RenderReport {
            start,
            end,
            segments,
            applied_events: budget - remaining,
        })
    }
    /// Call after the render loop stops; transfer these owned objects back to
    /// control for destruction. No live device callback may retain the engine.
    pub fn into_parts(self) -> EngineParts<S> {
        EngineParts {
            plan: self.plan,
            resources: self.resources,
            parameters: self.parameters,
            signals: self.signal_host,
        }
    }
}

impl PreparedOp {
    fn parameter_bindings(&self) -> impl Iterator<Item = &ParameterBindings> {
        let (params, inputs): (_, &[PreparedMixInput]) = match &self.kind {
            PreparedOpKind::Processor { params, .. } => (params, &[]),
            // Even an empty Bus retains the table identity proof.
            PreparedOpKind::Mix { params, inputs, .. } => (params, inputs),
        };
        std::iter::once(params).chain(inputs.iter().map(|input| &input.params))
    }

    pub(crate) fn contains_runtime(&self, runtime: usize) -> bool {
        self.runtime_indices().any(|index| index == runtime)
    }

    pub(crate) fn set_signals(&mut self, signals: Option<moiren_signal::OpSignalBindings>) {
        self.signals = signals;
    }
}
