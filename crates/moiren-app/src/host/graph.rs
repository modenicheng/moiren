use super::*;
use moiren_engine::{control::ParamDomain, processor::Compressor};

/// Typed DAG commands. Host-owned source/sink and strip nodes are managed by
/// the source API; arbitrary prepared IO cannot be invented by a graph command.
#[derive(Debug, Clone, Copy)]
pub enum GraphCommand {
    CreateNode {
        kind: NodeKind,
        channels: usize,
    },
    RemoveNode(NodeId),
    AddInput {
        node: NodeId,
        channels: usize,
    },
    RemoveInput {
        node: NodeId,
        port: PortId,
    },
    Connect {
        source: PortId,
        destination: PortId,
        params: SendParams,
    },
    ConnectToNewBusInput {
        source: PortId,
        bus: NodeId,
        params: SendParams,
    },
    Disconnect(EdgeId),
    SetSend {
        edge: EdgeId,
        params: SendParams,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphEdit {
    Node(NodeId),
    Port(PortId),
    Edge(EdgeId),
    Done,
}

impl AudioHost {
    /// Each command is transactional. Disconnecting an ordinary path is legal;
    /// the host's final Bus->output edge must remain connected at all times.
    pub fn graph_command(&mut self, command: GraphCommand) -> Result<GraphEdit, HostError> {
        self.editable()?;
        let mut state = self.desired.clone();
        let result = match command {
            GraphCommand::CreateNode { kind, channels } => {
                if matches!(kind, NodeKind::Source | NodeKind::Sink) {
                    return Err(HostError::UnsupportedGraph);
                }
                let node = state.graph.create_node(kind, channels)?;
                if kind == NodeKind::Compressor {
                    state
                        .compressors
                        .insert(node, CompressorSettings::default());
                }
                GraphEdit::Node(node)
            }
            GraphCommand::RemoveNode(node) => {
                if state.is_owned(node) {
                    return Err(HostError::ProtectedInfrastructure);
                }
                state.graph.remove_node(node)?;
                state.compressors.remove(&node);
                GraphEdit::Done
            }
            GraphCommand::AddInput { node, channels } => {
                GraphEdit::Port(state.graph.add_input_port(node, channels)?)
            }
            GraphCommand::RemoveInput { node, port } => {
                if state.sources.values().any(|source| source.bus_port == port) {
                    return Err(HostError::ProtectedInfrastructure);
                }
                state.graph.remove_input_port(node, port)?;
                GraphEdit::Done
            }
            GraphCommand::Connect {
                source,
                destination,
                params,
            } => GraphEdit::Edge(state.graph.connect(source, destination, params)?),
            GraphCommand::ConnectToNewBusInput {
                source,
                bus,
                params,
            } => GraphEdit::Edge(edit::connect_to_new_bus_input(
                &mut state.graph,
                source,
                bus,
                params,
            )?),
            GraphCommand::Disconnect(edge) => {
                state.graph.disconnect(edge)?;
                GraphEdit::Done
            }
            GraphCommand::SetSend { edge, params } => {
                state.graph.set_send_params(edge, params)?;
                GraphEdit::Done
            }
        };
        state.validate()?;
        self.desired = state;
        self.dirty = true;
        Ok(result)
    }

    /// Insert/remove the strip compressor between Gain and Pan. Settings changes
    /// deliberately replace its DSP state at publication, so reuse cannot copy
    /// old parameter values over the requested settings.
    pub fn set_compressor(
        &mut self,
        id: SourceId,
        settings: Option<CompressorSettings>,
    ) -> Result<(), HostError> {
        self.editable()?;
        if let Some(settings) = settings {
            for spec in Compressor::parameters(ProcessorId(0), settings) {
                let valid = match (spec.domain, spec.initial) {
                    (ParamDomain::Float { min, max }, ParamValue::Float(value)) => {
                        value.is_finite() && (min..=max).contains(&value)
                    }
                    _ => false,
                };
                if !valid {
                    return Err(HostError::InvalidCompressor);
                }
            }
        }
        let mut state = self.desired.clone();
        let source = state
            .sources
            .get(&id)
            .ok_or(HostError::UnknownSource(id))?
            .clone();
        match (source.compressor, settings) {
            (Some(node), Some(settings)) => {
                state.compressors.insert(node, settings);
            }
            (None, None) => return Ok(()),
            (None, Some(settings)) => {
                let gain_output = output(&state.graph, source.gain)?;
                let pan_input = input(&state.graph, source.pan)?;
                let edge = state
                    .graph
                    .edges()
                    .iter()
                    .find(|edge| edge.src_port() == gain_output && edge.dst_port() == pan_input)
                    .ok_or(HostError::UnsupportedGraph)?
                    .clone();
                state.graph.disconnect(edge.id())?;
                let node = state
                    .graph
                    .create_node(NodeKind::Compressor, self.config.channels)?;
                state
                    .graph
                    .connect(gain_output, input(&state.graph, node)?, *edge.params())?;
                state.graph.connect(
                    output(&state.graph, node)?,
                    pan_input,
                    SendParams::default(),
                )?;
                state.compressors.insert(node, settings);
                state.sources.get_mut(&id).unwrap().compressor = Some(node);
            }
            (Some(node), None) => {
                // Preserve sends if an editor has added fan-out to the compressor.
                // Removal is only unambiguous for the ordinary channel strip.
                let incident: Vec<_> = state
                    .graph
                    .edges()
                    .iter()
                    .filter(|e| e.src() == node || e.dst() == node)
                    .cloned()
                    .collect();
                if incident.len() != 2 {
                    return Err(HostError::UnsupportedGraph);
                }
                let incoming = incident
                    .iter()
                    .find(|e| e.dst() == node && e.src() == source.gain)
                    .ok_or(HostError::UnsupportedGraph)?;
                let outgoing = incident
                    .iter()
                    .find(|e| {
                        e.src() == node
                            && e.dst() == source.pan
                            && *e.params() == SendParams::default()
                    })
                    .ok_or(HostError::UnsupportedGraph)?;
                let (src, dst, params) =
                    (incoming.src_port(), outgoing.dst_port(), *incoming.params());
                state.graph.remove_node(node)?;
                state.graph.connect(src, dst, params)?;
                state.compressors.remove(&node);
                state.sources.get_mut(&id).unwrap().compressor = None;
            }
        }
        state.validate()?;
        self.desired = state;
        self.dirty = true;
        Ok(())
    }
}

impl State {
    fn is_owned(&self, node: NodeId) -> bool {
        node == self.bus
            || node == self.sink
            || self.sources.values().any(|s| {
                [Some(s.node), Some(s.gain), Some(s.pan), s.compressor].contains(&Some(node))
            })
    }
    pub(super) fn validate(&self) -> Result<(), HostError> {
        self.graph.validate()?;
        let sink_input = input(&self.graph, self.sink)?;
        if !self
            .graph
            .edges()
            .iter()
            .any(|e| e.src() == self.bus && e.dst_port() == sink_input)
        {
            return Err(HostError::ProtectedInfrastructure);
        }
        for node in self.graph.nodes() {
            if (node.kind() == NodeKind::Source
                && !self.sources.values().any(|s| s.node == node.id()))
                || (node.kind() == NodeKind::Sink && node.id() != self.sink)
            {
                return Err(HostError::UnsupportedGraph);
            }
        }
        for edge in self.graph.edges() {
            if edge.params().tap != SendTap::PostFader
                || (self.graph.get_port(edge.src_port())?.channels() != 2
                    && edge.params().pan != 0.0)
            {
                return Err(HostError::UnsupportedGraph);
            }
        }
        Ok(())
    }
}
