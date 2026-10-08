//! Non-RT reference compiler. Every signal has a dedicated slot; no in-place
//! processing or slot reuse can overwrite a value needed by another consumer.
use std::{collections::BTreeMap, mem::size_of};

use moiren_core::{
    graph::{EdgeId, GraphError, LogicalGraph, NodeId, SendTap},
    protocol::{ParameterKey, ProcessorId},
};
use thiserror::Error;

use crate::{
    buffer::{BufferArena, BufferError},
    control::{ControlError, ControlPort, parameter_channel},
    runtime::{Engine, EngineConfig, ExecutionPlan, RtResources, RuntimeError},
    sample::ProcessingSample,
};

mod bindings;
mod plan;
mod send;

pub use bindings::NodeBindings;

#[derive(Debug, Clone, Copy)]
pub struct CompileConfig {
    pub engine: EngineConfig,
    /// Audio slab only; excludes processors, queues and metadata.
    pub audio_byte_budget: usize,
    pub plan_revision: u64,
    pub timeline_epoch: u64,
    pub control_capacity: usize,
    pub control_horizon_frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CompileError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Buffer(#[from] BufferError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Control(#[from] ControlError),
    #[error("IO node {node:?} has no prepared source/sink binding")]
    MissingBinding { node: NodeId },
    #[error("node {node:?} was bound more than once")]
    DuplicateBinding { node: NodeId },
    #[error("binding does not match node {node:?}'s kind or IO role")]
    BindingKind { node: NodeId },
    #[error("edge {edge:?} uses PreFader without a defined channel-strip tap")]
    UnsupportedTap { edge: EdgeId },
    #[error(
        "edge {edge:?} has nonzero pan on {channels} channels; only stereo balance is supported"
    )]
    UnsupportedPan { edge: EdgeId, channels: usize },
    #[error("compiled processor identifier capacity exhausted")]
    TooManyProcessors,
}

#[derive(Debug, Clone, Copy)]
pub struct SendParameterKeys {
    pub gain: ParameterKey,
    pub pan: ParameterKey,
    pub mute: ParameterKey,
}
/// Plan-local bindings. Recompilation requires obtaining the new map and
/// submitting parameters with that plan's revision/epoch.
pub struct CompiledBindings {
    nodes: BTreeMap<NodeId, ProcessorId>,
    edges: BTreeMap<EdgeId, SendParameterKeys>,
}
impl CompiledBindings {
    pub fn node(&self, node: NodeId) -> Option<ProcessorId> {
        self.nodes.get(&node).copied()
    }
    pub fn edge(&self, edge: EdgeId) -> Option<SendParameterKeys> {
        self.edges.get(&edge).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub operation_count: usize,
    pub slot_count: usize,
    pub audio_bytes: usize,
}
pub struct CompiledGraph<S: ProcessingSample> {
    pub engine: Engine<S>,
    pub control: ControlPort,
    pub bindings: CompiledBindings,
    pub stats: CompileStats,
}

/// Compile and prepare entirely off the realtime thread. Unsupported edge
/// semantics are diagnosed before consuming any backend into an engine.
pub fn compile<S: ProcessingSample>(
    graph: &LogicalGraph,
    mut bindings: NodeBindings<S>,
    config: CompileConfig,
) -> Result<CompiledGraph<S>, CompileError> {
    let cfg = config.engine;
    if !cfg.processing_sr.is_finite()
        || cfg.processing_sr <= 0.0
        || cfg.max_block_frames == 0
        || cfg.max_block_frames > u32::MAX as usize
        || cfg.max_events_per_block == 0
    {
        return Err(RuntimeError::InvalidConfig.into());
    }
    graph.validate()?;
    for edge in graph.edges() {
        if edge.params().tap != SendTap::PostFader {
            return Err(CompileError::UnsupportedTap { edge: edge.id() });
        }
        let channels = graph.get_port(edge.src_port())?.channels();
        if channels != 2 && edge.params().pan != 0.0 {
            return Err(CompileError::UnsupportedPan {
                edge: edge.id(),
                channels,
            });
        }
    }
    bindings.validate(graph)?;
    let (builder, mapping) = plan::build(graph, &mut bindings, cfg.max_block_frames)?;
    let audio_bytes = builder.layouts.iter().try_fold(0usize, |total, layout| {
        layout
            .channels
            .checked_mul(layout.capacity_frames)
            .and_then(|samples| samples.checked_mul(size_of::<S>()))
            .and_then(|bytes| total.checked_add(bytes))
            .ok_or(BufferError::SizeOverflow)
    })?;
    let stats = CompileStats {
        node_count: graph.nodes().len(),
        edge_count: graph.edges().len(),
        operation_count: builder.ops.len(),
        slot_count: builder.layouts.len(),
        audio_bytes,
    };
    let arena = BufferArena::new(&builder.layouts, config.audio_byte_budget)?;
    let resources = RtResources::new(builder.instances)?;
    let (control, parameters) = parameter_channel(
        builder.params,
        config.plan_revision,
        config.timeline_epoch,
        config.control_capacity,
        config.control_horizon_frames,
    )?;
    let specs = plan::prepare_operations(builder.ops, &arena)?;
    let plan = ExecutionPlan::prepare(arena, specs, &resources, &parameters, cfg)?;
    let engine = Engine::new(plan, resources, parameters)?;
    Ok(CompiledGraph {
        engine,
        control,
        bindings: mapping,
        stats,
    })
}
