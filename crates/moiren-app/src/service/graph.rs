//! The service owns the editable topology. Pages receive immutable projections;
//! only validated transactions replace the candidate and native startup confirms it.
use super::{InputSelection, RunLimit, SessionGeneration, SessionSpec};
use moiren_core::graph::{EdgeId, GraphError, LogicalGraph, NodeId, NodeKind, PortId, SendParams};
mod builtin;
mod compressor;
mod edit;
mod mixer;
mod preparation;
pub use builtin::BuiltinState;
pub use compressor::{CompressorParameter, CompressorState};
use edit::edit_candidate;
pub use mixer::MixerState;

#[derive(Debug, Clone, PartialEq)]
pub struct GraphNodeState {
    pub id: NodeId,
    pub label: String,
    pub gain: f64,
    pub pan: f64,
    pub mono: bool,
    pub swap: bool,
    pub mixer: MixerState,
    pub compressor: CompressorState,
    pub builtin: BuiltinState,
    pub source: Option<InputSelection>,
    pub output_endpoint_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphSnapshot {
    pub graph: LogicalGraph,
    pub nodes: Vec<GraphNodeState>,
    pub candidate_revision: u64,
    /// Present only while the confirmed owner is running this topology.
    pub active_revision: Option<u64>,
    pub pending_revision: Option<u64>,
    pub last_error: Option<String>,
}
impl GraphSnapshot {
    pub fn node(&self, id: NodeId) -> Option<&GraphNodeState> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraphEdit {
    SetBuiltin {
        node: NodeId,
        state: BuiltinState,
    },
    SetCompressor {
        node: NodeId,
        parameter: CompressorParameter,
        value: f64,
    },
    ClearBinding(NodeId),
    SetMixer {
        node: NodeId,
        state: MixerState,
    },
    AddNode {
        kind: NodeKind,
        channels: usize,
    },
    RemoveNode(NodeId),
    Connect {
        source: PortId,
        destination: PortId,
    },
    Disconnect(EdgeId),
    SetSend {
        edge: EdgeId,
        params: SendParams,
    },
    AddBusInput(NodeId),
    RemoveBusInput {
        node: NodeId,
        port: PortId,
    },
    SetLabel {
        node: NodeId,
        label: String,
    },
    SetGain {
        node: NodeId,
        gain: f64,
    },
    SetPan {
        node: NodeId,
        pan: f64,
    },
    SetMono {
        node: NodeId,
        mono: bool,
    },
    SetSwap {
        node: NodeId,
        swap: bool,
    },
    BindSource {
        node: NodeId,
        source: InputSelection,
    },
    BindOutput {
        node: NodeId,
        endpoint_id: String,
    },
}

/// Owned worker preparation input; no UI entity or native handle crosses here.
#[derive(Debug, Clone)]
pub struct GraphPreparation {
    pub graph: LogicalGraph,
    pub nodes: Vec<GraphNodeState>,
    pub revision: u64,
}
pub struct ServiceGraph {
    snapshot: GraphSnapshot,
    pending_generation: Option<SessionGeneration>,
}
impl Default for ServiceGraph {
    fn default() -> Self {
        let session =
            crate::session_graph::SessionGraph::new().expect("fixed stereo session graph");
        let nodes = session
            .graph
            .nodes()
            .iter()
            .map(|node| GraphNodeState {
                id: node.id(),
                label: default_label(node.kind()).into(),
                gain: if node.kind() == NodeKind::Gain {
                    0.0
                } else {
                    1.0
                },
                pan: 0.0,
                mono: false,
                swap: false,
                mixer: MixerState::default(),
                compressor: CompressorState::default(),
                builtin: BuiltinState::for_kind(node.kind()),
                source: (node.kind() == NodeKind::Source).then_some(InputSelection::Tone {
                    frequency_hz: 440.0,
                }),
                output_endpoint_id: None,
            })
            .collect();
        Self {
            snapshot: GraphSnapshot {
                graph: session.graph,
                nodes,
                candidate_revision: 1,
                active_revision: None,
                pending_revision: None,
                last_error: None,
            },
            pending_generation: None,
        }
    }
}
fn default_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Source => "Input",
        NodeKind::Sink => "Output",
        NodeKind::Gain => "Gain",
        NodeKind::Pan => "Balance",
        NodeKind::Bus => "Bus",
        NodeKind::Compressor => "Compressor",
        NodeKind::Eq => "EQ",
        NodeKind::Gate => "Gate",
        NodeKind::Limiter => "Limiter",
        NodeKind::Delay => "Delay",
        NodeKind::ChannelMapper => "Channel Mapper",
        NodeKind::Recorder => "Recorder",
        NodeKind::Meter => "Meter",
        NodeKind::SpectrumAnalyzer => "Spectrum Analyzer",
        NodeKind::Rack => "Rack",
    }
}
impl ServiceGraph {
    pub fn snapshot(&self) -> GraphSnapshot {
        self.snapshot.clone()
    }
    pub fn preparation(&self) -> GraphPreparation {
        GraphPreparation {
            graph: self.snapshot.graph.clone(),
            nodes: self.snapshot.nodes.clone(),
            revision: self.snapshot.candidate_revision,
        }
    }
    pub fn record_error(&mut self, error: impl ToString) {
        self.snapshot.last_error = Some(error.to_string());
    }
    pub fn edit(&mut self, edit: GraphEdit) -> Result<(), String> {
        let mut next = self.snapshot.clone();
        let result = edit_candidate(&mut next, edit);
        if let Err(error) = result {
            self.record_error(&error);
            return Err(error);
        }
        next.graph.validate().map_err(|e| e.to_string())?;
        next.candidate_revision = next
            .candidate_revision
            .checked_add(1)
            .ok_or("图版本空间已耗尽")?;
        next.last_error = None;
        self.snapshot = next;
        Ok(())
    }
    /// A loaded document creates a new candidate. Disk contents cannot confirm
    /// native activity, and stale in-flight preparations never confirm this edit.
    pub fn replace(
        &mut self,
        graph: LogicalGraph,
        nodes: Vec<GraphNodeState>,
    ) -> Result<(), String> {
        GraphPreparation {
            graph: graph.clone(),
            nodes: nodes.clone(),
            revision: 0,
        }
        .validate_state()?;
        let revision = self
            .snapshot
            .candidate_revision
            .checked_add(1)
            .ok_or("图版本空间已耗尽")?;
        self.snapshot.graph = graph;
        self.snapshot.nodes = nodes;
        self.snapshot.candidate_revision = revision;
        self.snapshot.last_error = None;
        Ok(())
    }
    pub fn configure_session(&mut self, spec: &SessionSpec) -> Result<(), String> {
        spec.validate().map_err(|e| format!("{e:?}"))?;
        let preparation = self.preparation();
        let (source, output) = preparation.primary_io_nodes()?;
        let mut next = self.snapshot.clone();
        for state in &mut next.nodes {
            if state.id == source {
                state.source = Some(spec.source.clone());
            }
            if state.id == output {
                state.output_endpoint_id = Some(spec.output_endpoint_id.clone());
            }
        }
        if let Some((gain, pan)) = preparation.gain_pan_nodes() {
            next.nodes
                .iter_mut()
                .find(|s| s.id == gain)
                .expect("gain state")
                .gain = spec.gain;
            next.nodes
                .iter_mut()
                .find(|s| s.id == pan)
                .expect("pan state")
                .pan = spec.pan;
        }
        if next != self.snapshot {
            next.candidate_revision = next
                .candidate_revision
                .checked_add(1)
                .ok_or("图版本空间已耗尽")?;
            next.last_error = None;
            self.snapshot = next;
        }
        Ok(())
    }
    pub fn session_spec(&self, current: Option<&SessionSpec>) -> Result<SessionSpec, String> {
        let prep = self.preparation();
        let (source, output) = prep.primary_io_nodes()?;
        let source = self
            .snapshot
            .node(source)
            .and_then(|s| s.source.clone())
            .ok_or("请选择输入资源")?;
        let output_endpoint_id = self
            .snapshot
            .node(output)
            .and_then(|s| s.output_endpoint_id.clone())
            .ok_or("请选择输出设备")?;
        let mut spec = current.cloned().unwrap_or(SessionSpec {
            source: source.clone(),
            output_endpoint_id: output_endpoint_id.clone(),
            limit: RunLimit::UntilStopped,
            gain: 1.0,
            pan: 0.0,
            max_block_frames: 256,
        });
        spec.source = source;
        spec.output_endpoint_id = output_endpoint_id;
        if let Some((gain, pan)) = prep.gain_pan_nodes() {
            spec.gain = self.snapshot.node(gain).expect("gain state").gain.min(1.0);
            spec.pan = self.snapshot.node(pan).expect("pan state").pan;
        }
        spec.validate().map_err(|e| format!("{e:?}"))?;
        Ok(spec)
    }
    pub fn preparing(&mut self, generation: SessionGeneration, revision: u64) {
        self.pending_generation = Some(generation);
        self.snapshot.pending_revision = Some(revision);
    }
    pub fn running(&mut self, generation: SessionGeneration, revision: u64) {
        if self.pending_generation == Some(generation)
            && self.snapshot.pending_revision == Some(revision)
        {
            self.snapshot.active_revision = Some(revision);
            self.snapshot.pending_revision = None;
            self.pending_generation = None;
        }
    }
    pub fn stopped(&mut self) {
        self.snapshot.active_revision = None;
    }
    /// Only a managed owner may call this after host/RT confirmation. An older
    /// applied topology may become active while a newer candidate is pending.
    pub(crate) fn confirmed(&mut self, generation: SessionGeneration, revision: u64) {
        if self.snapshot.candidate_revision == revision
            && (self.snapshot.active_revision != Some(revision)
                || self.snapshot.pending_revision == Some(revision))
        {
            self.snapshot.last_error = None;
        }
        self.snapshot.active_revision = Some(revision);
        if self.pending_generation == Some(generation)
            && self.snapshot.pending_revision == Some(revision)
        {
            self.clear_pending();
        }
    }
    pub fn clear_pending(&mut self) {
        self.pending_generation = None;
        self.snapshot.pending_revision = None;
    }
    pub fn preparation_failed(&mut self, generation: SessionGeneration, error: impl ToString) {
        if self.pending_generation == Some(generation) {
            self.snapshot.pending_revision = None;
            self.pending_generation = None;
            self.record_error(error);
        }
    }
}
#[cfg(test)]
mod tests;
