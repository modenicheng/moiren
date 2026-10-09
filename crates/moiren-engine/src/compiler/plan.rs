//! Dedicated signal slots preserve fan-out and make preparation deterministic.
use moiren_core::{
    graph::{GraphError, LogicalGraph, NodeId, NodeKind},
    protocol::ProcessorId,
};
use std::collections::BTreeMap;

use super::{
    CompileError, CompiledBindings, NodeBindings,
    bindings::{Binding, BoundIo},
    send::Send,
};
use crate::{
    buffer::{BufferArena, BufferError, BufferSlotLayout, PortAccess},
    control::ParamSpec,
    processor::{Bus, Compressor, Gain, Pan, RtProcessor},
    runtime::{OpSpec, ProcessorInstance},
    sample::ProcessingSample,
};

pub(super) struct DraftOp {
    processor: ProcessorId,
    reads: Vec<(u16, usize)>,
    output: Option<usize>,
}
pub(super) struct Builder<S: ProcessingSample> {
    frames: usize,
    pub(super) layouts: Vec<BufferSlotLayout>,
    pub(super) instances: Vec<ProcessorInstance<S>>,
    pub(super) params: Vec<ParamSpec>,
    pub(super) ops: Vec<DraftOp>,
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
    fn node_processor(
        &mut self,
        node: NodeId,
        kind: NodeKind,
        binding: Option<Binding<S>>,
    ) -> Result<ProcessorId, CompileError> {
        Ok(match kind {
            NodeKind::Source | NodeKind::Sink => {
                let Some(Binding::Io(processor)) = binding else {
                    return Err(CompileError::MissingBinding { node });
                };
                self.processor(BoundIo(processor))?
            }
            NodeKind::Gain => {
                let initial = if let Some(Binding::Gain(initial)) = binding {
                    initial
                } else {
                    1.0
                };
                let processor = self.processor(Gain)?;
                self.params.push(Gain::parameter(processor, initial));
                processor
            }
            NodeKind::Pan => {
                let initial = if let Some(Binding::Pan(initial)) = binding {
                    initial
                } else {
                    0.0
                };
                let processor = self.processor(Pan)?;
                self.params.push(Pan::parameter(processor, initial));
                processor
            }
            NodeKind::Compressor => {
                let settings = if let Some(Binding::Compressor(settings)) = binding {
                    settings
                } else {
                    Default::default()
                };
                let processor = self.processor(Compressor::new())?;
                self.params
                    .extend(Compressor::parameters(processor, settings));
                processor
            }
            NodeKind::Bus => self.processor(Bus)?,
        })
    }
}

pub(super) fn build<S: ProcessingSample>(
    graph: &LogicalGraph,
    bindings: &mut NodeBindings<S>,
    frames: usize,
) -> Result<(Builder<S>, CompiledBindings), CompileError> {
    let incoming: BTreeMap<_, _> = graph.edges().iter().map(|e| (e.dst_port(), e)).collect();
    let mut outputs = BTreeMap::new();
    let mut mapping = CompiledBindings {
        nodes: BTreeMap::new(),
        edges: BTreeMap::new(),
    };
    let mut builder = Builder {
        frames,
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
        let processor = builder.node_processor(id, node.kind(), bindings.take(id))?;
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
    Ok((builder, mapping))
}

pub(super) fn prepare_operations<S: ProcessingSample>(
    ops: Vec<DraftOp>,
    arena: &BufferArena<S>,
) -> Result<Vec<OpSpec>, BufferError> {
    ops.into_iter()
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
        .collect::<Result<Vec<_>, BufferError>>()
}
