//! A low-level linear executor, not a LogicalGraph compiler or liveness planner.
use crate::{
    buffer::{BufferArena, BufferError, PreparedIo},
    control::{ParameterBindings, ParameterRuntime},
    processor::{ProcessContext, ProcessorError, ProcessorRole, RtProcessor},
    sample::ProcessingSample,
};
use moiren_core::protocol::ProcessorId;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    pub processing_sr: f64,
    pub max_block_frames: usize,
    pub max_events_per_block: usize,
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
    #[error(transparent)]
    Buffer(#[from] BufferError),
    #[error(transparent)]
    Processor(#[from] ProcessorError),
}

pub struct ProcessorInstance<S: ProcessingSample> {
    id: ProcessorId,
    processor: Box<dyn RtProcessor<S>>,
}
impl<S: ProcessingSample> ProcessorInstance<S> {
    pub fn new(id: ProcessorId, processor: impl RtProcessor<S> + 'static) -> Self {
        Self {
            id,
            processor: Box::new(processor),
        }
    }
}
/// Owns persistent DSP state independently from the schedule. Existing slots
/// may be rebound by a future prepared plan without reinitializing processors.
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
struct PreparedOp {
    runtime: usize,
    io: PreparedIo,
    params: ParameterBindings,
}
pub struct ExecutionPlan<S: ProcessingSample> {
    arena: BufferArena<S>,
    ops: Box<[PreparedOp]>,
    resources: Arc<()>,
    config: EngineConfig,
    revision: u64,
    epoch: u64,
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
        if !config.processing_sr.is_finite()
            || config.processing_sr <= 0.0
            || config.max_block_frames == 0
            || config.max_block_frames > u32::MAX as usize
            || config.max_events_per_block == 0
        {
            return Err(RuntimeError::InvalidConfig);
        }
        let mut ops = Vec::<PreparedOp>::with_capacity(specs.len());
        for spec in specs {
            if !arena.owns(&spec.io) {
                return Err(RuntimeError::Buffer(BufferError::ForeignAccess));
            }
            if spec.io.max_frames() < config.max_block_frames {
                return Err(RuntimeError::InvalidFrames);
            }
            let index = resources
                .instances
                .iter()
                .position(|r| r.id == spec.processor)
                .ok_or(RuntimeError::MissingProcessor)?;
            if ops.iter().any(|op| op.runtime == index) {
                return Err(RuntimeError::DuplicateProcessor);
            }
            let processor = &resources.instances[index].processor;
            if processor.role() == ProcessorRole::Observer && spec.io.output_count() != 0 {
                return Err(RuntimeError::Processor(ProcessorError::InvalidIo));
            }
            processor.validate_io(&spec.io)?;
            let bindings = parameters.bindings(spec.processor);
            processor.validate_parameters(parameters.view(&bindings))?;
            ops.push(PreparedOp {
                runtime: index,
                io: spec.io,
                params: bindings,
            });
        }
        Ok(Self {
            arena,
            ops: ops.into_boxed_slice(),
            resources: Arc::clone(&resources.identity),
            config,
            revision: parameters.revision(),
            epoch: parameters.epoch(),
        })
    }
}

pub struct Engine<S: ProcessingSample> {
    plan: ExecutionPlan<S>,
    resources: RtResources<S>,
    parameters: ParameterRuntime,
    timeline: u64,
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
            || plan.ops.iter().any(|op| !parameters.owns(&op.params))
        {
            return Err(RuntimeError::ForeignParameters);
        }
        Ok(Self {
            plan,
            resources,
            parameters,
            timeline: 0,
        })
    }
    pub fn timeline(&self) -> u64 {
        self.timeline
    }
    pub fn epoch(&self) -> u64 {
        self.plan.epoch
    }
    pub fn render(&mut self, frames: usize) -> Result<RenderReport, RuntimeError> {
        if frames == 0 || frames > self.plan.config.max_block_frames {
            return Err(RuntimeError::InvalidFrames);
        }
        let start = self.timeline;
        let end = start
            .checked_add(frames as u64)
            .ok_or(RuntimeError::TimelineOverflow)?;
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
            for op in &self.plan.ops {
                let processor = &mut self.resources.instances[op.runtime].processor;
                let params = self.parameters.view(&op.params);
                self.plan.arena.with_io(&op.io, window.clone(), |mut io| {
                    // All samples were initialized at allocation. This clear also
                    // prevents stale output when a safe processor only writes a prefix.
                    io.clear_outputs();
                    processor.process(&ctx, io, params);
                })?;
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
    pub fn into_parts(self) -> (ExecutionPlan<S>, RtResources<S>, ParameterRuntime) {
        (self.plan, self.resources, self.parameters)
    }
}
