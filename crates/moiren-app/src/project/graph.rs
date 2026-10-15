use std::collections::{BTreeMap, BTreeSet};

use moiren_core::graph::{LogicalGraph, NodeId, NodeKind, SendParams, SendTap};
use serde::{Deserialize, Serialize};

use super::{PersistenceError, SourceDocument, model::validate_identity};
use crate::service::{GraphNodeState, GraphSnapshot};

const MAX_PROJECT_NODES: usize = 4096;
const MAX_PROJECT_EDGES: usize = 32768;
const MAX_PROJECT_PORTS: usize = 65536;

/// Document-local IDs permit restoring a graph exclusively through its public
/// checked edits. Prepared engine addresses and runtime revisions are omitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GraphDocument {
    pub nodes: Vec<GraphNodeDocument>,
    pub edges: Vec<GraphEdgeDocument>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNodeDocument {
    pub id: u64,
    pub kind: GraphNodeKind,
    pub channels: usize,
    pub input_count: usize,
    pub label: String,
    pub gain: f64,
    pub pan: f64,
    #[serde(default)]
    pub mono: bool,
    #[serde(default)]
    pub swap: bool,
    pub source: Option<SourceDocument>,
    pub output_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    Source,
    Sink,
    Gain,
    Compressor,
    Bus,
    Pan,
}

impl From<NodeKind> for GraphNodeKind {
    fn from(kind: NodeKind) -> Self {
        match kind {
            NodeKind::Source => Self::Source,
            NodeKind::Sink => Self::Sink,
            NodeKind::Gain => Self::Gain,
            NodeKind::Compressor => Self::Compressor,
            NodeKind::Bus => Self::Bus,
            NodeKind::Pan => Self::Pan,
        }
    }
}

impl From<GraphNodeKind> for NodeKind {
    fn from(kind: GraphNodeKind) -> Self {
        match kind {
            GraphNodeKind::Source => Self::Source,
            GraphNodeKind::Sink => Self::Sink,
            GraphNodeKind::Gain => Self::Gain,
            GraphNodeKind::Compressor => Self::Compressor,
            GraphNodeKind::Bus => Self::Bus,
            GraphNodeKind::Pan => Self::Pan,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdgeDocument {
    pub source_node: u64,
    pub source_output: usize,
    pub destination_node: u64,
    pub destination_input: usize,
    pub gain: f64,
    pub pan: f64,
    pub mute: bool,
    pub tap: SendTapDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendTapDocument {
    PreFader,
    PostFader,
}

/// The mapping lets the presentation restore saved positions without keeping
/// a second topology or manufacturing LogicalGraph object identities.
pub struct RestoredGraph {
    pub graph: LogicalGraph,
    pub node_ids: BTreeMap<u64, NodeId>,
}

impl GraphDocument {
    pub fn from_snapshot(snapshot: &GraphSnapshot) -> Result<Self, PersistenceError> {
        let ids: BTreeMap<_, _> = snapshot
            .graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id(), index as u64))
            .collect();
        let nodes = snapshot
            .graph
            .nodes()
            .iter()
            .map(|node| {
                let state = snapshot.node(node.id()).ok_or_else(|| {
                    PersistenceError::invalid("graph.nodes", "service node metadata is missing")
                })?;
                Ok(GraphNodeDocument {
                    id: ids[&node.id()],
                    kind: node.kind().into(),
                    channels: node.channels(),
                    input_count: node.inputs().len(),
                    label: state.label.clone(),
                    gain: state.gain,
                    pan: state.pan,
                    mono: state.mono,
                    swap: state.swap,
                    source: state.source.as_ref().map(Into::into),
                    output_endpoint_id: state.output_endpoint_id.clone(),
                })
            })
            .collect::<Result<Vec<_>, PersistenceError>>()?;
        let edges =
            snapshot
                .graph
                .edges()
                .iter()
                .map(|edge| {
                    let source = snapshot.graph.get_node(edge.src()).map_err(|error| {
                        PersistenceError::invalid("graph.edges", error.to_string())
                    })?;
                    let destination = snapshot.graph.get_node(edge.dst()).map_err(|error| {
                        PersistenceError::invalid("graph.edges", error.to_string())
                    })?;
                    let source_output = source
                        .outputs()
                        .iter()
                        .position(|port| port.id() == edge.src_port())
                        .ok_or_else(|| {
                            PersistenceError::invalid("graph.edges", "source output is missing")
                        })?;
                    let destination_input = destination
                        .inputs()
                        .iter()
                        .position(|port| port.id() == edge.dst_port())
                        .ok_or_else(|| {
                            PersistenceError::invalid("graph.edges", "destination input is missing")
                        })?;
                    let params = edge.params();
                    Ok(GraphEdgeDocument {
                        source_node: ids[&source.id()],
                        source_output,
                        destination_node: ids[&destination.id()],
                        destination_input,
                        gain: params.gain,
                        pan: params.pan,
                        mute: params.mute,
                        tap: match params.tap {
                            SendTap::PreFader => SendTapDocument::PreFader,
                            SendTap::PostFader => SendTapDocument::PostFader,
                        },
                    })
                })
                .collect::<Result<Vec<_>, PersistenceError>>()?;
        let document = Self { nodes, edges };
        document.to_graph()?;
        Ok(document)
    }

    pub fn to_graph(&self) -> Result<RestoredGraph, PersistenceError> {
        if self.nodes.len() > MAX_PROJECT_NODES || self.edges.len() > MAX_PROJECT_EDGES {
            return Err(PersistenceError::invalid(
                "graph",
                "project exceeds 4096 nodes or 32768 edges",
            ));
        }
        let mut graph = LogicalGraph::new();
        let mut node_ids = BTreeMap::new();
        let mut ids = BTreeSet::new();
        let mut total_ports = 0usize;
        for (index, node) in self.nodes.iter().enumerate() {
            let field = format!("graph.nodes[{index}]");
            if !ids.insert(node.id) {
                return Err(PersistenceError::invalid(
                    &field,
                    "duplicate document node ID",
                ));
            }
            if node.channels > 256 || node.input_count > MAX_PROJECT_PORTS {
                return Err(PersistenceError::invalid(
                    &field,
                    "channels exceed 256 or input count exceeds 65536",
                ));
            }
            total_ports = total_ports
                .saturating_add(node.input_count)
                .saturating_add(1);
            if total_ports > MAX_PROJECT_PORTS {
                return Err(PersistenceError::invalid(
                    "graph",
                    "total port count exceeds 65536",
                ));
            }
            if node.label.trim().is_empty() || node.label.len() > 256 || node.label.contains('\0') {
                return Err(PersistenceError::invalid(
                    &field,
                    "label must contain 1 to 256 bytes without NUL",
                ));
            }
            if !node.gain.is_finite()
                || !(0.0..=16.0).contains(&node.gain)
                || !node.pan.is_finite()
                || !(-1.0..=1.0).contains(&node.pan)
            {
                return Err(PersistenceError::invalid(
                    &field,
                    "gain must be in 0..=16 and pan in -1..=1",
                ));
            }
            if node.kind != GraphNodeKind::Pan && (node.mono || node.swap) {
                return Err(PersistenceError::invalid(
                    &field,
                    "only a balance node may enable mono or swap",
                ));
            }
            if let Some(source) = &node.source {
                source.validate()?;
                if node.kind != GraphNodeKind::Source {
                    return Err(PersistenceError::invalid(
                        &field,
                        "only a source node may bind an input",
                    ));
                }
            }
            if let Some(output) = &node.output_endpoint_id {
                validate_identity(output, &field)?;
                if node.kind != GraphNodeKind::Sink {
                    return Err(PersistenceError::invalid(
                        &field,
                        "only a sink node may bind an output",
                    ));
                }
            }
            let id = graph
                .create_node(node.kind.into(), node.channels)
                .map_err(|error| PersistenceError::invalid(&field, error.to_string()))?;
            if node.kind == GraphNodeKind::Bus {
                for _ in 0..node.input_count {
                    graph
                        .add_input_port(id, node.channels)
                        .map_err(|error| PersistenceError::invalid(&field, error.to_string()))?;
                }
            } else if graph.get_node(id).expect("just created").inputs().len() != node.input_count {
                return Err(PersistenceError::invalid(
                    &field,
                    "input count does not match the node kind",
                ));
            }
            node_ids.insert(node.id, id);
        }
        for (index, edge) in self.edges.iter().enumerate() {
            let field = format!("graph.edges[{index}]");
            let source_id = node_ids
                .get(&edge.source_node)
                .ok_or_else(|| PersistenceError::invalid(&field, "source node is missing"))?;
            let destination_id = node_ids
                .get(&edge.destination_node)
                .ok_or_else(|| PersistenceError::invalid(&field, "destination node is missing"))?;
            let source = graph
                .get_node(*source_id)
                .expect("document node was created")
                .outputs()
                .get(edge.source_output)
                .ok_or_else(|| {
                    PersistenceError::invalid(&field, "source output index is out of range")
                })?
                .id();
            let destination = graph
                .get_node(*destination_id)
                .expect("document node was created")
                .inputs()
                .get(edge.destination_input)
                .ok_or_else(|| {
                    PersistenceError::invalid(&field, "destination input index is out of range")
                })?
                .id();
            graph
                .connect(
                    source,
                    destination,
                    SendParams {
                        gain: edge.gain,
                        pan: edge.pan,
                        mute: edge.mute,
                        tap: match edge.tap {
                            SendTapDocument::PreFader => SendTap::PreFader,
                            SendTapDocument::PostFader => SendTap::PostFader,
                        },
                    },
                )
                .map_err(|error| PersistenceError::invalid(&field, error.to_string()))?;
        }
        graph
            .validate()
            .map_err(|error| PersistenceError::invalid("graph", error.to_string()))?;
        Ok(RestoredGraph { graph, node_ids })
    }

    pub fn restore(&self) -> Result<(LogicalGraph, Vec<GraphNodeState>), PersistenceError> {
        let restored = self.to_graph()?;
        let nodes = self
            .nodes
            .iter()
            .map(|node| GraphNodeState {
                id: restored.node_ids[&node.id],
                label: node.label.clone(),
                gain: node.gain,
                pan: node.pan,
                mono: node.mono,
                swap: node.swap,
                source: node.source.as_ref().map(SourceDocument::to_selection),
                output_endpoint_id: node.output_endpoint_id.clone(),
            })
            .collect();
        Ok((restored.graph, nodes))
    }
}
