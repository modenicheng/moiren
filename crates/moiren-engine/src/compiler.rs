//! Non-RT reference compiler. Every signal has a dedicated slot; no in-place
//! processing or slot reuse can overwrite a value needed by another consumer.
use std::{collections::BTreeMap, mem::size_of};

use moiren_core::{
    graph::*,
    protocol::{ParameterKey, ProcessorId},
};
use thiserror::Error;

use crate::{
    boundary::{RtAudioSink, RtAudioSource, SinkAdapter, SourceAdapter},
    buffer::{BufferArena, BufferError, BufferSlotLayout, PortAccess},
    control::{ControlError, ControlPort, ParamSpec, parameter_channel},
    processor::{Bus, Gain, Pan, ProcessorRole, RtProcessor},
    runtime::{
        Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources, RuntimeError,
    },
    sample::ProcessingSample,
};

mod send;
use send::Send;

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

enum Binding<S: ProcessingSample> {
    Io(Box<dyn RtProcessor<S>>),
    Gain(f64),
    Pan(f64),
}

/// Prepared backend ownership and initial DSP values, separate from editable
/// topology. All methods and all destruction are control-side operations.
pub struct NodeBindings<S: ProcessingSample> {
    nodes: BTreeMap<NodeId, Binding<S>>,
}
impl<S: ProcessingSample> Default for NodeBindings<S> {
    fn default() -> Self {
        Self::new()
    }
}
impl<S: ProcessingSample> NodeBindings<S> {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
        }
    }

    fn insert(&mut self, node: NodeId, binding: Binding<S>) -> Result<(), CompileError> {
        if self.nodes.contains_key(&node) {
            return Err(CompileError::DuplicateBinding { node });
        }
        self.nodes.insert(node, binding);
        Ok(())
    }
    pub fn bind_source(
        &mut self,
        node: NodeId,
        source: impl RtAudioSource<S> + 'static,
    ) -> Result<(), CompileError> {
        self.bind_io(node, SourceAdapter(source))
    }
    pub fn bind_sink(
        &mut self,
        node: NodeId,
        sink: impl RtAudioSink<S> + 'static,
    ) -> Result<(), CompileError> {
        self.bind_io(node, SinkAdapter(sink))
    }
    /// Also accepts already prepared InputNode/OutputNode with their telemetry.
    pub fn bind_io(
        &mut self,
        node: NodeId,
        processor: impl RtProcessor<S> + 'static,
    ) -> Result<(), CompileError> {
        self.insert(node, Binding::Io(Box::new(processor)))
    }
    pub fn bind_gain(&mut self, node: NodeId, initial: f64) -> Result<(), CompileError> {
        self.insert(node, Binding::Gain(initial))
    }
    pub fn bind_pan(&mut self, node: NodeId, initial: f64) -> Result<(), CompileError> {
        self.insert(node, Binding::Pan(initial))
    }
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

struct DraftOp {
    processor: ProcessorId,
    reads: Vec<(u16, usize)>,
    output: Option<usize>,
}
struct Builder<S: ProcessingSample> {
    frames: usize,
    layouts: Vec<BufferSlotLayout>,
    instances: Vec<ProcessorInstance<S>>,
    params: Vec<ParamSpec>,
    ops: Vec<DraftOp>,
}
impl<S: ProcessingSample> Builder<S> {
    fn slot(&mut self, channels: usize) -> usize {
        let slot = self.layouts.len();
        self.layouts.push(BufferSlotLayout {
            channels,
            capacity_frames: self.frames,
        });
        slot
    }
    fn processor(
        &mut self,
        processor: impl RtProcessor<S> + 'static,
    ) -> Result<ProcessorId, CompileError> {
        let id = ProcessorId(
            u64::try_from(self.instances.len()).map_err(|_| CompileError::TooManyProcessors)?,
        );
        self.instances.push(ProcessorInstance::new(id, processor));
        Ok(id)
    }
}

// Forwarding a boxed user processor preserves its role and validation hooks.
// This wrapper stays private, avoiding a blanket public Box implementation.
struct BoundIo<S: ProcessingSample>(Box<dyn RtProcessor<S>>);
impl<S: ProcessingSample> RtProcessor<S> for BoundIo<S> {
    fn role(&self) -> ProcessorRole {
        self.0.role()
    }
    fn latency_frames(&self) -> u32 {
        self.0.latency_frames()
    }
    fn validate_io(
        &self,
        io: &crate::buffer::PreparedIo,
    ) -> Result<(), crate::processor::ProcessorError> {
        self.0.validate_io(io)
    }
    fn validate_parameters(
        &self,
        params: crate::control::ProcessParameters<'_>,
    ) -> Result<(), crate::processor::ProcessorError> {
        self.0.validate_parameters(params)
    }
    fn process(
        &mut self,
        ctx: &crate::processor::ProcessContext,
        io: crate::buffer::ProcessIo<'_, S>,
        params: crate::control::ProcessParameters<'_>,
    ) {
        self.0.process(ctx, io, params);
    }
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
    for (&id, binding) in &bindings.nodes {
        let kind = graph.get_node(id)?.kind();
        let valid = match (kind, binding) {
            (NodeKind::Source, Binding::Io(p)) => p.role() == ProcessorRole::Source,
            (NodeKind::Sink, Binding::Io(p)) => p.role() == ProcessorRole::Sink,
            (NodeKind::Gain, Binding::Gain(_)) | (NodeKind::Pan, Binding::Pan(_)) => true,
            _ => false,
        };
        if !valid {
            return Err(CompileError::BindingKind { node: id });
        }
    }
    let incoming: BTreeMap<_, _> = graph.edges().iter().map(|e| (e.dst_port(), e)).collect();
    let mut outputs = BTreeMap::new();
    let mut mapping = CompiledBindings {
        nodes: BTreeMap::new(),
        edges: BTreeMap::new(),
    };
    let mut builder = Builder {
        frames: cfg.max_block_frames,
        layouts: Vec::new(),
        instances: Vec::new(),
        params: Vec::new(),
        ops: Vec::new(),
    };
    for id in graph.topological_order()? {
        let node = graph.get_node(id)?;
        let mut reads = Vec::with_capacity(node.inputs().len());
        for (index, port) in node.inputs().iter().enumerate() {
            let slot = if let Some(edge) = incoming.get(&port.id()) {
                let source = outputs[&edge.src_port()];
                let slot = builder.slot(port.channels());
                let send = Send {
                    channels: port.channels(),
                };
                let processor = builder.processor(send)?;
                let (specs, keys) = Send::parameters(processor, port.channels(), *edge.params());
                builder.params.extend(specs);
                mapping.edges.insert(edge.id(), keys);
                builder.ops.push(DraftOp {
                    processor,
                    reads: vec![(0, source)],
                    output: Some(slot),
                });
                slot
            } else {
                // Dedicated initialized silence. No operation ever writes here.
                builder.slot(port.channels())
            };
            let port_index = u16::try_from(index).map_err(|_| GraphError::TooManyPorts)?;
            reads.push((port_index, slot));
        }
        let binding = bindings.nodes.remove(&id);
        let processor = match node.kind() {
            NodeKind::Source | NodeKind::Sink => {
                let Some(Binding::Io(processor)) = binding else {
                    return Err(CompileError::MissingBinding { node: id });
                };
                builder.processor(BoundIo(processor))?
            }
            NodeKind::Gain => {
                let initial = if let Some(Binding::Gain(initial)) = binding {
                    initial
                } else {
                    1.0
                };
                let processor = builder.processor(Gain)?;
                builder.params.push(Gain::parameter(processor, initial));
                processor
            }
            NodeKind::Pan => {
                let initial = if let Some(Binding::Pan(initial)) = binding {
                    initial
                } else {
                    0.0
                };
                let processor = builder.processor(Pan)?;
                builder.params.push(Pan::parameter(processor, initial));
                processor
            }
            NodeKind::Bus => builder.processor(Bus)?,
        };
        mapping.nodes.insert(id, processor);
        let output = node.outputs().first().map(|port| {
            let slot = builder.slot(port.channels());
            outputs.insert(port.id(), slot);
            slot
        });
        builder.ops.push(DraftOp {
            processor,
            reads,
            output,
        });
    }
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
    let specs = builder
        .ops
        .into_iter()
        .map(|op| {
            let mut access: Vec<_> = op
                .reads
                .into_iter()
                .map(|(port, slot)| PortAccess::Read {
                    port,
                    slot: arena.slot(slot).expect("compiler allocated slot"),
                })
                .collect();
            if let Some(slot) = op.output {
                access.push(PortAccess::Write {
                    port: 0,
                    slot: arena.slot(slot).expect("compiler allocated slot"),
                });
            }
            Ok(OpSpec {
                processor: op.processor,
                io: arena.prepare_io(&access)?,
            })
        })
        .collect::<Result<Vec<_>, BufferError>>()?;
    let plan = ExecutionPlan::prepare(arena, specs, &resources, &parameters, cfg)?;
    let engine = Engine::new(plan, resources, parameters)?;
    Ok(CompiledGraph {
        engine,
        control,
        bindings: mapping,
        stats,
    })
}
