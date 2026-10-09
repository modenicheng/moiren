//! Portable control owner for a prepared, independently gated multi-source graph.
//! Graph edits and reclamation run here; only `HostRenderer` crosses to render.
mod graph;
mod model;
mod publication;
mod renderer;

pub use graph::{GraphCommand, GraphEdit};
pub use model::*;
pub use renderer::HostRenderer;

use std::{collections::BTreeMap, sync::Arc};

use moiren_core::{graph::*, protocol::*};
use moiren_engine::{
    boundary::{AudioReader, RtAudioSource, audio_bridge},
    compiler::{self, CompileConfig, CompileStats, CompiledBindings, NodeBindings},
    control::ControlPort,
    processor::{CompressorSettings, Gain, Pan},
    runtime::{Engine, EngineConfig, PlanControlPort, PlanSnapshot},
};

use model::{Active, Pending, SourceState, State, Telemetry};

pub struct AudioHost {
    config: HostConfig,
    desired: State,
    active: Active,
    pending: Option<Pending>,
    backends: BTreeMap<SourceId, Box<dyn RtAudioSource<f32>>>,
    plans: PlanControlPort<f32>,
    telemetry: Arc<Telemetry>,
    next_source: u64,
    next_revision: u64,
    next_request: u64,
    dirty: bool,
}

impl AudioHost {
    pub fn prepare(config: HostConfig) -> Result<(Self, HostRenderer), HostError> {
        if config.channels != 2
            || config.max_block_frames == 0
            || !config.processing_sr.is_finite()
            || config.processing_sr <= 0.0
            || config.control_capacity == 0
        {
            return Err(HostError::InvalidConfig);
        }
        let mut graph = LogicalGraph::new();
        let bus = graph.create_node(NodeKind::Bus, config.channels)?;
        let sink = graph.create_node(NodeKind::Sink, config.channels)?;
        graph.connect(
            output(&graph, bus)?,
            input(&graph, sink)?,
            SendParams::default(),
        )?;
        let desired = State {
            graph,
            bus,
            sink,
            sources: BTreeMap::new(),
            compressors: BTreeMap::new(),
        };
        let (writer, reader) = audio_bridge(
            config.channels,
            config.max_block_frames,
            config.audio_byte_budget,
        )?;
        let mut bindings = NodeBindings::new();
        bindings.bind_sink(sink, writer)?;
        let compiled = compiler::compile(&desired.graph, bindings, compile_config(config, 1))?;
        let mut engine = compiled.engine;
        let snapshot = engine.plan_snapshot();
        let plans = engine.enable_plan_switching(1)?;
        let telemetry = Arc::new(Telemetry::default());
        let renderer = HostRenderer::new(engine, reader, Arc::clone(&telemetry));
        let host = Self {
            config,
            active: Active {
                state: desired.clone(),
                control: compiled.control,
                bindings: compiled.bindings,
                snapshot,
                stats: compiled.stats,
            },
            desired,
            pending: None,
            backends: BTreeMap::new(),
            plans,
            telemetry,
            next_source: 1,
            next_revision: 2,
            next_request: 1,
            dirty: false,
        };
        Ok((host, renderer))
    }

    fn editable(&self) -> Result<(), HostError> {
        if self.pending.is_some() {
            Err(HostError::Busy)
        } else {
            Ok(())
        }
    }

    /// Stage one channel strip. `publish` makes it audible at a block boundary.
    pub fn add_source(
        &mut self,
        source: impl RtAudioSource<f32> + 'static,
    ) -> Result<SourceId, HostError> {
        self.add_source_with_settings(source, SourceSettings::default())
    }

    /// Initial settings are compiled into the first audible plan, avoiding an
    /// initial unity-gain block before a live parameter request can be applied.
    pub fn add_source_with_settings(
        &mut self,
        source: impl RtAudioSource<f32> + 'static,
        settings: SourceSettings,
    ) -> Result<SourceId, HostError> {
        self.editable()?;
        if !settings.gain.is_finite()
            || !(0.0..=16.0).contains(&settings.gain)
            || !settings.pan.is_finite()
            || !(-1.0..=1.0).contains(&settings.pan)
        {
            return Err(HostError::InvalidSourceSettings);
        }
        if source.channel_count() != self.config.channels {
            return Err(HostError::ChannelMismatch);
        }
        let id = SourceId(self.next_source);
        let next = self
            .next_source
            .checked_add(1)
            .ok_or(HostError::IdOverflow)?;
        let mut state = self.desired.clone();
        let source_node = state
            .graph
            .create_node(NodeKind::Source, self.config.channels)?;
        let gain = state
            .graph
            .create_node(NodeKind::Gain, self.config.channels)?;
        let pan = state
            .graph
            .create_node(NodeKind::Pan, self.config.channels)?;
        state.graph.connect(
            output(&state.graph, source_node)?,
            input(&state.graph, gain)?,
            SendParams::default(),
        )?;
        state.graph.connect(
            output(&state.graph, gain)?,
            input(&state.graph, pan)?,
            SendParams::default(),
        )?;
        let pan_output = output(&state.graph, pan)?;
        let edge = edit::connect_to_new_bus_input(
            &mut state.graph,
            pan_output,
            state.bus,
            SendParams::default(),
        )?;
        let bus_port = state.graph.get_edge(edge)?.dst_port();
        let gate = SourceGate::default();
        gate.set_available(settings.available);
        state.sources.insert(
            id,
            SourceState {
                id,
                node: source_node,
                gain,
                pan,
                bus_port,
                generation: 1,
                gate,
                level: settings.gain,
                position: settings.pan,
                compressor: None,
            },
        );
        self.desired = state;
        self.backends.insert(id, Box::new(source));
        self.next_source = next;
        self.dirty = true;
        Ok(id)
    }

    pub fn remove_source(&mut self, id: SourceId) -> Result<(), HostError> {
        self.editable()?;
        let mut state = self.desired.clone();
        let source = state
            .sources
            .remove(&id)
            .ok_or(HostError::UnknownSource(id))?;
        for node in [
            Some(source.node),
            Some(source.gain),
            Some(source.pan),
            source.compressor,
        ]
        .into_iter()
        .flatten()
        {
            state.graph.remove_node(node)?;
            state.compressors.remove(&node);
        }
        // Graph commands may have rerouted the strip; remove every remaining
        // edge into its reserved Bus input before deleting that input.
        if let Some(edge) = state
            .graph
            .edges()
            .iter()
            .find(|edge| edge.dst_port() == source.bus_port)
            .map(Edge::id)
        {
            state.graph.disconnect(edge)?;
        }
        state.graph.remove_input_port(state.bus, source.bus_port)?;
        self.desired = state;
        self.backends.remove(&id);
        self.dirty = true;
        Ok(())
    }

    /// Replacement retains the UI identity but never reuses the previous backend.
    pub fn replace_source(
        &mut self,
        id: SourceId,
        source: impl RtAudioSource<f32> + 'static,
    ) -> Result<(), HostError> {
        self.editable()?;
        if source.channel_count() != self.config.channels {
            return Err(HostError::ChannelMismatch);
        }
        let current = self
            .desired
            .sources
            .get_mut(&id)
            .ok_or(HostError::UnknownSource(id))?;
        current.generation = current
            .generation
            .checked_add(1)
            .ok_or(HostError::IdOverflow)?;
        current.gate = SourceGate::default();
        self.backends.insert(id, Box::new(source));
        self.dirty = true;
        Ok(())
    }

    pub fn source_gate(&self, id: SourceId) -> Result<SourceGate, HostError> {
        Ok(self
            .desired
            .sources
            .get(&id)
            .ok_or(HostError::UnknownSource(id))?
            .gate
            .clone())
    }

    pub fn set_gain(
        &mut self,
        id: SourceId,
        value: f64,
        ramp_frames: u32,
    ) -> Result<ControlReply, HostError> {
        self.set_strip_parameter(id, value, ramp_frames, true)
    }
    pub fn set_pan(
        &mut self,
        id: SourceId,
        value: f64,
        ramp_frames: u32,
    ) -> Result<ControlReply, HostError> {
        self.set_strip_parameter(id, value, ramp_frames, false)
    }
    fn set_strip_parameter(
        &mut self,
        id: SourceId,
        value: f64,
        ramp: u32,
        gain: bool,
    ) -> Result<ControlReply, HostError> {
        let source = self
            .active
            .state
            .sources
            .get(&id)
            .ok_or(HostError::SourceNotActive(id))?;
        let node = if gain { source.gain } else { source.pan };
        let reply = self.submit_parameter(
            node,
            if gain { Gain::LEVEL } else { Pan::POSITION },
            ParamValue::Float(value),
            ApplyAt::NextBlock,
            ramp,
        )?;
        if reply.code == ReplyCode::Accepted {
            for state in std::iter::once(&mut self.active.state)
                .chain(std::iter::once(&mut self.desired))
                .chain(self.pending.iter_mut().map(|p| &mut p.active.state))
            {
                if let Some(source) = state.sources.get_mut(&id) {
                    if gain {
                        source.level = value;
                    } else {
                        source.position = value;
                    }
                }
            }
        }
        Ok(reply)
    }

    /// Submit against the active revision. Accepted is not an applied receipt;
    /// every accepted request produces a terminal event through `poll`/`finish`.
    pub fn submit_parameter(
        &mut self,
        node: NodeId,
        parameter: ParameterId,
        value: ParamValue,
        at: ApplyAt,
        ramp_frames: u32,
    ) -> Result<ControlReply, HostError> {
        let processor = self
            .active
            .bindings
            .node(node)
            .ok_or(HostError::Graph(GraphError::NodeNotFound))?;
        let request_id = self.next_request;
        self.next_request = request_id.checked_add(1).ok_or(HostError::IdOverflow)?;
        Ok(self.active.control.submit(
            ParameterRequest {
                request_id,
                plan_revision: self.active.snapshot.revision(),
                timeline_epoch: self.active.snapshot.epoch(),
                target: ParameterKey {
                    processor,
                    parameter,
                },
                at,
                value,
                ramp_frames,
            },
            self.runtime_snapshot().timeline,
        ))
    }

    /// Backend render observers can publish their timeline without exposing an
    /// Engine through normal control methods. Called on the control owner.
    pub fn observe_timeline(&self, timeline: u64) {
        self.telemetry
            .timeline
            .fetch_max(timeline, std::sync::atomic::Ordering::Release);
    }
    pub fn runtime_snapshot(&self) -> RuntimeSnapshot {
        use std::sync::atomic::Ordering::Acquire;
        RuntimeSnapshot {
            timeline: self.telemetry.timeline.load(Acquire),
            peak: f32::from_bits(self.telemetry.peak.load(Acquire)),
            rendered_blocks: self.telemetry.blocks.load(Acquire),
            active_revision: self.plans.active_revision(),
            desired_revision: self.pending.as_ref().map_or_else(
                || {
                    if self.dirty {
                        self.next_revision
                    } else {
                        self.active.snapshot.revision()
                    }
                },
                |p| p.active.snapshot.revision(),
            ),
            pending: self.pending.is_some(),
            dirty: self.dirty,
        }
    }
    pub fn graph_snapshot(&self) -> GraphSnapshot {
        GraphSnapshot {
            desired: self.desired.graph.clone(),
            active: self.active.state.graph.clone(),
            sources: self
                .desired
                .sources
                .values()
                .map(SourceState::snapshot)
                .collect(),
            bus: self.desired.bus,
            sink: self.desired.sink,
            plan: PlanInfo {
                active_revision: self.active.snapshot.revision(),
                pending_revision: self.pending.as_ref().map(|p| p.active.snapshot.revision()),
                stats: self.active.stats,
            },
        }
    }
}

fn input(graph: &LogicalGraph, node: NodeId) -> Result<PortId, GraphError> {
    graph
        .get_node(node)?
        .inputs()
        .first()
        .map(Port::id)
        .ok_or(GraphError::PortNotFound)
}
fn output(graph: &LogicalGraph, node: NodeId) -> Result<PortId, GraphError> {
    graph
        .get_node(node)?
        .outputs()
        .first()
        .map(Port::id)
        .ok_or(GraphError::PortNotFound)
}
fn compile_config(config: HostConfig, revision: u64) -> CompileConfig {
    CompileConfig {
        engine: EngineConfig {
            processing_sr: config.processing_sr,
            max_block_frames: config.max_block_frames,
            max_events_per_block: config.control_capacity,
        },
        audio_byte_budget: config.audio_byte_budget,
        plan_revision: revision,
        timeline_epoch: 1,
        control_capacity: config.control_capacity,
        control_horizon_frames: config.control_horizon_frames,
    }
}
