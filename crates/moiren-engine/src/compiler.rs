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
    signal::SignalPlanError,
};

mod bindings;
mod observers;
mod plan;
mod send;
mod update;

pub use update::{
    CompiledUpdate, UpdateCompileError, UpdateReuse, compile_update, compile_update_with_reuse,
};

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
    #[error("signal bindings require compile_with_signals and a host control")]
    SignalsRequireHost,
    #[error("observer target port {port:?} does not exist")]
    ObserverPortNotFound { port: moiren_core::graph::PortId },
    #[error("observer target port {port:?} is an input port")]
    ObserverInputPort { port: moiren_core::graph::PortId },
    #[error("input observer targets an output port: {port:?}")]
    ObserverOutputPort { port: moiren_core::graph::PortId },
    #[error(transparent)]
    Signal(#[from] SignalPlanError),
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
    observers: BTreeMap<NodeId, ProcessorId>,
    edges: BTreeMap<EdgeId, SendParameterKeys>,
    rack_slots: BTreeMap<(NodeId, u32), ProcessorId>,
}
impl CompiledBindings {
    /// The independent observer owned by a Meter, Recorder or SpectrumAnalyzer
    /// node, distinct from its pass-through audio processor.
    pub fn observer(&self, node: NodeId) -> Option<ProcessorId> {
        self.observers.get(&node).copied()
    }

    pub fn node(&self, node: NodeId) -> Option<ProcessorId> {
        self.nodes.get(&node).copied()
    }
    pub fn edge(&self, edge: EdgeId) -> Option<SendParameterKeys> {
        self.edges.get(&edge).copied()
    }
    /// Stable slot identity resolved to this plan's processor and parameter keys.
    pub fn rack_slot(&self, node: NodeId, slot: u32) -> Option<ProcessorId> {
        self.rack_slots.get(&(node, slot)).copied()
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
    bindings: NodeBindings<S>,
    config: CompileConfig,
) -> Result<CompiledGraph<S>, CompileError> {
    compile_inner(graph, bindings, config, None)
}

pub fn compile_with_signals<S: ProcessingSample>(
    graph: &LogicalGraph,
    bindings: NodeBindings<S>,
    config: CompileConfig,
    host: &moiren_signal::HostControl,
) -> Result<CompiledGraph<S>, CompileError> {
    compile_inner(graph, bindings, config, Some(host))
}

fn compile_inner<S: ProcessingSample>(
    graph: &LogicalGraph,
    mut bindings: NodeBindings<S>,
    config: CompileConfig,
    host: Option<&moiren_signal::HostControl>,
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
        if edge.params().tap == SendTap::PreFader
            && graph.get_node(edge.src())?.kind() != moiren_core::graph::NodeKind::Gain
            && !bindings.strips.contains_key(&edge.src())
        {
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
    let (builder, mapping) = plan::build(
        graph,
        &mut bindings,
        cfg.max_block_frames,
        cfg.processing_sr,
    )?;
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
    let specs = plan::prepare_operations(builder.ops, &arena)?;
    let resources = RtResources::new(builder.instances)?;
    let (control, parameters) = parameter_channel(
        builder.params,
        config.plan_revision,
        config.timeline_epoch,
        config.control_capacity,
        config.control_horizon_frames,
    )?;
    let plan = ExecutionPlan::prepare_inner(arena, specs, &resources, &parameters, cfg, host)
        .map_err(|error| match error {
            RuntimeError::SignalsRequireHost => CompileError::SignalsRequireHost,
            RuntimeError::Signal(error) => CompileError::Signal(error),
            error => CompileError::Runtime(error),
        })?;
    let engine = Engine::new(plan, resources, parameters)?;
    Ok(CompiledGraph {
        engine,
        control,
        bindings: mapping,
        stats,
    })
}
