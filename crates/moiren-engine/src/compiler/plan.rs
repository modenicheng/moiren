//! Lower logical topology to a schedule with explicit signal reads and writes.
use moiren_core::{
    graph::{GraphError, LogicalGraph, NodeId, NodeKind, SendTap},
    protocol::{ParameterId, ParameterKey, ProcessorId},
};
use std::collections::BTreeMap;

use super::{
    CompileError, CompiledBindings, NodeBindings, StripParameterKeys,
    bindings::{Binding, BoundIo},
    send::Send,
};
use crate::{
    buffer::{BufferArena, BufferError, BufferSlotLayout, PortAccess},
    control::ParamSpec,
    processor::{
        Bus, ChannelMapper, ChannelMapperSettings, Compressor, Delay, Eq, Gain, Gate, Limiter, Pan,
        PassThrough, RtProcessor,
    },
    runtime::{CompiledOpSpec, FusedMixInputSpec, FusedOpSpec, ProcessorInstance},
    sample::ProcessingSample,
};

pub(super) struct DraftOp {
    pub(super) processor: ProcessorId,
    pub(super) reads: Vec<(u16, usize)>,
    pub(super) outputs: Vec<(u16, usize)>,
    pub(super) fused: Option<FusedOpSpec>,
    pub(super) in_place_candidate: bool,
    pub(super) in_place: bool,
}
pub(super) struct Builder<S: ProcessingSample> {
    frames: usize,
    processing_sr: f64,
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

    fn strip(
        &mut self,
        source: usize,
        channels: usize,
        config: crate::processor::StripConfig,
        include_gain: bool,
    ) -> Result<(usize, StripParameterKeys), CompileError> {
        let slot = self.slot(channels);
        let strip = crate::processor::builtin::ChannelStrip { channels, config };
        let processor = self.processor(strip)?;
        let specs = crate::processor::builtin::ChannelStrip::parameters(
            processor,
            channels,
            config,
            include_gain,
        );
        let key = |parameter: ParameterId| ParameterKey {
            processor,
            parameter,
        };
        let keys = StripParameterKeys {
            gain: include_gain.then(|| key(crate::processor::builtin::ChannelStrip::GAIN)),
            balance: key(crate::processor::builtin::ChannelStrip::BALANCE),
            mode: key(crate::processor::builtin::ChannelStrip::MODE),
            effective_mute: key(crate::processor::builtin::ChannelStrip::EFFECTIVE_MUTE),
        };
        self.params.extend(specs);
        self.ops.push(DraftOp {
            processor,
            reads: vec![(0, source)],
            outputs: vec![(0, slot)],
            fused: None,
            in_place_candidate: false,
            in_place: false,
        });
        Ok((slot, keys))
    }
    pub(super) fn processor(
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
        channels: usize,
        input_channels: usize,
        processing_sr: f64,
        binding: Option<Binding<S>>,
    ) -> Result<ProcessorId, CompileError> {
        Ok(match kind {
            NodeKind::Source | NodeKind::Sink => match binding {
                Some(Binding::Io(processor)) => self.processor(BoundIo(processor))?,
                Some(Binding::ReusedIo(basis, previous)) => {
                    let id = ProcessorId(
                        u64::try_from(self.instances.len())
                            .map_err(|_| CompileError::TooManyProcessors)?,
                    );
                    let (instance, parameters) = basis
                        .reserve_io(previous, id)
                        .map_err(|_| CompileError::BindingKind { node })?;
                    self.instances.push(instance);
                    self.params.extend(parameters);
                    id
                }
                _ => return Err(CompileError::MissingBinding { node }),
            },
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
                let (position, mono, swap) = if let Some(Binding::Pan {
                    position,
                    mono,
                    swap,
                }) = binding
                {
                    (position, mono, swap)
                } else {
                    (0.0, false, false)
                };
                let processor = self.processor(Pan)?;
                self.params
                    .extend(Pan::parameters(processor, position, mono, swap));
                processor
            }
            NodeKind::Compressor => {
                let (settings, publisher) =
                    if let Some(Binding::Compressor(settings, publisher)) = binding {
                        (settings, publisher)
                    } else {
                        (Default::default(), None)
                    };
                let compressor = publisher.map_or_else(Compressor::new, Compressor::with_signal);
                let processor = self.processor(compressor)?;
                self.params
                    .extend(Compressor::parameters(processor, settings));
                processor
            }
            NodeKind::Eq => {
                let settings = match binding {
                    Some(Binding::Eq(settings)) => settings,
                    _ => Default::default(),
                };
                let processor = self.processor(Eq::new())?;
                self.params.extend(Eq::parameters(processor, settings));
                processor
            }
            NodeKind::Gate => {
                let settings = match binding {
                    Some(Binding::Gate(settings)) => settings,
                    _ => Default::default(),
                };
                let processor = self.processor(Gate::new())?;
                self.params.extend(Gate::parameters(processor, settings));
                processor
            }
            NodeKind::Limiter => {
                let settings = match binding {
                    Some(Binding::Limiter(settings)) => settings,
                    _ => Default::default(),
                };
                let processor = self.processor(Limiter::new())?;
                self.params.extend(Limiter::parameters(processor, settings));
                processor
            }
            NodeKind::Delay => {
                let settings = match binding {
                    Some(Binding::Delay(settings)) => settings,
                    _ => Default::default(),
                };
                let processor = self.processor(
                    Delay::new(channels, settings, processing_sr)
                        .map_err(|_| CompileError::BindingKind { node })?,
                )?;
                self.params.extend(Delay::parameters(processor, settings));
                processor
            }
            NodeKind::ChannelMapper => self.processor(
                ChannelMapper::with_layout(
                    input_channels,
                    channels,
                    match binding {
                        Some(Binding::ChannelMapper(settings)) => settings,
                        _ => ChannelMapperSettings::default(),
                    },
                )
                .map_err(|_| CompileError::BindingKind { node })?,
            )?,
            NodeKind::Recorder | NodeKind::Meter | NodeKind::SpectrumAnalyzer => {
                if !matches!(binding, Some(Binding::Unmanaged(_))) {
                    return Err(CompileError::MissingBinding { node });
                }
                self.processor(PassThrough)?
            }
            // Rack lowering prepares the explicit multi-port boundary separately.
            NodeKind::Rack => return Err(CompileError::MissingBinding { node }),
            NodeKind::Bus => self.processor(Bus)?,
        })
    }
}

pub(super) fn build<S: ProcessingSample>(
    graph: &LogicalGraph,
    bindings: &mut NodeBindings<S>,
    frames: usize,
    processing_sr: f64,
    optimized: bool,
) -> Result<(Builder<S>, CompiledBindings), CompileError> {
    let incoming: BTreeMap<_, _> = graph.edges().iter().map(|e| (e.dst_port(), e)).collect();
    let mut outputs = BTreeMap::new();
    let mut pre_fader = BTreeMap::new();
    let mut mapping = CompiledBindings {
        nodes: BTreeMap::new(),
        observers: BTreeMap::new(),
        edges: BTreeMap::new(),
        rack_slots: BTreeMap::new(),
        strips: BTreeMap::new(),
    };
    let mut builder = Builder {
        frames,
        processing_sr,
        layouts: Vec::new(),
        instances: Vec::new(),
        params: Vec::new(),
        ops: Vec::new(),
    };
    for id in graph.topological_order()? {
        let node = graph.get_node(id)?;
        let strip = bindings.strips.get(&id).copied();
        let mut reads = Vec::with_capacity(node.inputs().len());
        let use_mix = optimized
            && node.kind() == NodeKind::Bus
            && !node
                .inputs()
                .iter()
                .any(|port| bindings.has_input_observers(port.id()));
        let mut mix_inputs = Vec::new();
        for (index, port) in node.inputs().iter().enumerate() {
            // An unconnected Bus input contributes zero. A fused mix does not
            // need to materialize that silence; observed inputs use the fallback.
            if use_mix && !incoming.contains_key(&port.id()) {
                continue;
            }
            let port_index = u16::try_from(index).map_err(|_| GraphError::TooManyPorts)?;
            let slot = if let Some(edge) = incoming.get(&port.id()) {
                let source = if edge.params().tap == SendTap::PreFader {
                    pre_fader[&edge.src_port()]
                } else {
                    outputs[&edge.src_port()]
                };
                let send = Send {
                    channels: port.channels(),
                };
                let processor = builder.processor(send)?;
                let (specs, keys) = Send::parameters(processor, port.channels(), *edge.params());
                builder.params.extend(specs);
                mapping.edges.insert(edge.id(), keys);
                if use_mix {
                    mix_inputs.push(FusedMixInputSpec {
                        processor,
                        port: port_index,
                    });
                    source
                } else {
                    let slot = builder.slot(port.channels());
                    builder.ops.push(DraftOp {
                        processor,
                        reads: vec![(0, source)],
                        outputs: vec![(0, slot)],
                        fused: None,
                        in_place_candidate: false,
                        in_place: false,
                    });
                    slot
                }
            } else {
                // Dedicated initialized silence. No operation ever writes here.
                builder.slot(port.channels())
            };
            let slot = if node.kind() == NodeKind::Sink
                && let Some(config) = strip
            {
                let (slot, keys) = builder.strip(slot, node.channels(), config, true)?;
                mapping.strips.entry(id).or_default().push(keys);
                slot
            } else {
                slot
            };
            reads.push((port_index, slot));
            if let Some(observers) = bindings.take_input_observers(port.id()) {
                for observer in observers {
                    super::observers::push(&mut builder, observer, slot)?;
                }
            }
        }
        let binding = bindings.take(id);
        // Resolve widths before allocating kernels; slot identity remains stable,
        // while ProcessorIds and buffer slots belong only to this prepared plan.
        let rack_boundary = if let Some(Binding::Rack(settings)) = &binding {
            Some(lower_rack(
                &mut builder,
                &mut mapping,
                id,
                node,
                settings,
                &mut reads,
            )?)
        } else {
            None
        };
        let (binding, observer) = match binding {
            Some(Binding::Observed(kind, observer)) => {
                (Some(Binding::Unmanaged(kind)), Some(observer))
            }
            binding => (binding, None),
        };
        let processor = if let Some(processor) = rack_boundary {
            processor
        } else {
            builder.node_processor(
                id,
                node.kind(),
                node.channels(),
                node.inputs().first().map_or(0, |port| port.channels()),
                builder.processing_sr,
                binding,
            )?
        };
        if node.kind() == NodeKind::Gain {
            pre_fader.insert(node.outputs()[0].id(), reads[0].1);
        }
        mapping.nodes.insert(id, processor);
        let writes: Vec<_> = node
            .outputs()
            .iter()
            .enumerate()
            .map(|(index, port)| {
                let slot = builder.slot(port.channels());
                outputs.insert(port.id(), slot);
                (index as u16, slot)
            })
            .collect();
        builder.ops.push(DraftOp {
            processor,
            reads,
            outputs: writes.clone(),
            fused: if use_mix {
                Some(FusedOpSpec::Mix {
                    inputs: mix_inputs.into_boxed_slice(),
                    input_count: node.inputs().len(),
                })
            } else {
                None
            },
            in_place_candidate: !use_mix && matches!(node.kind(), NodeKind::Gain | NodeKind::Pan),
            in_place: false,
        });
        // A strip is a node control shared across its output streams, but each
        // stream retains its own width and pre-fader tap (never a NodeId alias).
        for ((_, raw), port) in writes.into_iter().zip(node.outputs()) {
            if let Some(config) = strip {
                // PRE keeps the monitoring transform at unity gain.
                let (before, pre) = builder.strip(
                    raw,
                    port.channels(),
                    crate::processor::StripConfig { gain: 1., ..config },
                    false,
                )?;
                pre_fader.insert(port.id(), before);
                let (after, post) = builder.strip(raw, port.channels(), config, true)?;
                outputs.insert(port.id(), after);
                mapping.strips.entry(id).or_default().extend([pre, post]);
            }
        }
        for port in node.outputs() {
            let Some(observers) = bindings.take_observers(port.id()) else {
                continue;
            };
            let slot = outputs[&port.id()];
            for observer in observers {
                super::observers::push(&mut builder, observer, slot)?;
            }
        }
        if let Some(observer) = observer {
            let port = node
                .outputs()
                .first()
                .ok_or(CompileError::BindingKind { node: id })?;
            let processor = super::observers::push(&mut builder, observer, outputs[&port.id()])?;
            mapping.observers.insert(id, processor);
        }
    }
    Ok((builder, mapping))
}

/// Rack lowering: resolve the rack's layout against the node's port widths,
/// materialize one prepared op per non-bypass slot in the order declared by
/// `RackSettings`, and rewire the node's `reads` onto the routed output
/// streams. Returns the boundary processor that applies the rack's routing.
fn lower_rack<S: ProcessingSample>(
    builder: &mut Builder<S>,
    mapping: &mut CompiledBindings,
    id: NodeId,
    node: &moiren_core::graph::Node,
    settings: &crate::rack::RackSettings,
    reads: &mut Vec<(u16, usize)>,
) -> Result<ProcessorId, CompileError> {
    let input_widths: Vec<_> = node.inputs().iter().map(|port| port.channels()).collect();
    let output_widths: Vec<_> = node.outputs().iter().map(|port| port.channels()).collect();
    let layout = settings
        .resolve_layout(&input_widths, &output_widths)
        .ok_or(CompileError::BindingKind { node: id })?;
    let mut streams: Vec<_> = layout
        .routes
        .iter()
        .map(|route| reads[usize::from(route.input_port)].1)
        .collect();
    // Never iterate routes here: slots on distinct streams must still
    // execute in the single global order declared by RackSettings.
    for slot in settings.slots.iter().filter(|slot| !slot.bypass) {
        let io = layout.slots[&slot.id];
        let processor = ProcessorId(
            u64::try_from(builder.instances.len()).map_err(|_| CompileError::TooManyProcessors)?,
        );
        let (kernel, parameters) = crate::rack::prepare_slot::<S>(
            &slot.processor,
            io.input_channels,
            builder.processing_sr,
            processor,
        )
        .map_err(|_| CompileError::BindingKind { node: id })?;
        let prepared = builder.processor(BoundIo(kernel))?;
        builder.params.extend(parameters);
        let output = builder.slot(io.output_channels);
        builder.ops.push(DraftOp {
            processor: prepared,
            reads: vec![(0, streams[io.route])],
            outputs: vec![(0, output)],
            fused: None,
            in_place_candidate: false,
            in_place: false,
        });
        streams[io.route] = output;
        mapping.rack_slots.insert((id, slot.id), prepared);
    }
    *reads = streams
        .into_iter()
        .enumerate()
        .map(|(port, slot)| (port as u16, slot))
        .collect();
    builder.processor(crate::rack::RackRouting::new(&layout, &output_widths))
}

pub(super) fn prepare_operations<S: ProcessingSample>(
    ops: Vec<DraftOp>,
    arena: &BufferArena<S>,
) -> Result<Vec<CompiledOpSpec>, BufferError> {
    ops.into_iter()
        .map(|op| {
            let access = if op.in_place {
                let (input, input_slot) = op.reads[0];
                let (output, output_slot) = op.outputs[0];
                debug_assert_eq!(input_slot, output_slot);
                vec![PortAccess::InPlace {
                    input,
                    output,
                    slot: arena.slot(input_slot).expect("compiler allocated slot"),
                }]
            } else {
                let mut access: Vec<_> = op
                    .reads
                    .into_iter()
                    .map(|(port, slot)| PortAccess::Read {
                        port,
                        slot: arena.slot(slot).expect("compiler allocated slot"),
                    })
                    .collect();
                for (port, slot) in op.outputs {
                    access.push(PortAccess::Write {
                        port,
                        slot: arena.slot(slot).expect("compiler allocated slot"),
                    });
                }
                access
            };
            let io = arena.prepare_io(&access)?;
            let processor = op.processor;
            Ok(CompiledOpSpec {
                processor,
                io,
                fused: op.fused,
            })
        })
        .collect::<Result<Vec<_>, BufferError>>()
}
