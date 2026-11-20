//! The service owns the editable topology. Pages receive immutable projections;
//! only validated transactions replace the candidate and native startup confirms it.
use super::{
    ExportIntent, InputSelection, PluginAssignment, PluginLibrary, RecordingCard,
    SessionGeneration, SessionSpec, validate_exports,
};
use moiren_core::graph::{EdgeId, LogicalGraph, NodeId, NodeKind, PortId, SendParams};
mod builtin;
mod candidate;
mod compressor;
mod edit;
mod journal;
mod mixer;
mod preparation;
mod revisions;
pub use builtin::BuiltinState;
pub(crate) use candidate::edit_parameter_values;
pub use candidate::validate_node_state;
use candidate::{edit_hint, node_of_port};
pub use compressor::{CompressorParameter, CompressorState};
use journal::UndoJournal;
pub use mixer::MixerState;
pub use preparation::clock_master;

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
    /// The Plugin node's assignment, resolved against the service's plugin
    /// library at preparation time; `None` on every other kind.
    pub plugin: Option<PluginAssignment>,
    /// User-chosen card tint as an index into the fixed theme palette
    /// (see [`NODE_TINT_COUNT`]); `None` keeps the category tint.
    pub tint: Option<u8>,
}

/// How many entries the node tint palette has. The palette itself is a
/// UI concern; the service only guards the index.
pub const NODE_TINT_COUNT: u8 = 8;

/// Where a fault points, in user-visible terms: a channel strip, a route
/// between two ports, or the project as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphSubject {
    Node(NodeId),
    Route(PortId, PortId),
    Graph,
}

/// A rustc-style fault: what is wrong, where, and how to fix it. Causes are
/// user language and never describe internal runtime state.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphFault {
    pub subject: GraphSubject,
    pub cause: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphSnapshot {
    pub exports: Vec<ExportIntent>,
    pub graph: LogicalGraph,
    pub nodes: Vec<GraphNodeState>,
    pub candidate_revision: u64,
    /// Present only while the confirmed owner is running this topology.
    pub active_revision: Option<u64>,
    pub pending_revision: Option<u64>,
    pub last_error: Option<String>,
    /// Unresolved faults; the group clears on the next successful edit
    /// landing or activation.
    pub faults: Vec<GraphFault>,
    /// Whether the undo/redo journal can step back / forward right now.
    pub can_undo: bool,
    pub can_redo: bool,
    /// Nodes touched by structural edits since the last activation; the
    /// engine is re-preparing these, so cards show the loading state. An
    /// edit failure or activation clears the set.
    pub preparing_nodes: Vec<NodeId>,
    /// Discrete recording card state attached by the snapshot assembler:
    /// phase and file name only. The ticking duration is deliberately absent
    /// so graph-page keys stay free of per-second values.
    pub recording: Vec<RecordingCard>,
}
impl GraphSnapshot {
    pub fn node(&self, id: NodeId) -> Option<&GraphNodeState> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Whether any fault points at `id`: a `Node` subject names it directly,
    /// a `Route` subject through both of its endpoint ports. Mixer strip keys
    /// project this per capture, so it stays allocation-free.
    pub fn node_faulted(&self, id: NodeId) -> bool {
        self.faults
            .iter()
            .any(|fault| self.subject_points_at(fault.subject, id))
    }

    /// All faults pointing at `id`, in snapshot order. The one projection the
    /// inline error UI shares (graph card tooltip and badge popover, mixer
    /// strip hover banner), so subject→node resolution lives in exactly one
    /// place.
    pub fn faults_for_node(&self, id: NodeId) -> Vec<GraphFault> {
        self.faults
            .iter()
            .filter(|fault| self.subject_points_at(fault.subject, id))
            .cloned()
            .collect()
    }

    fn subject_points_at(&self, subject: GraphSubject, id: NodeId) -> bool {
        match subject {
            GraphSubject::Node(node) => node == id,
            GraphSubject::Route(source, destination) => {
                node_of_port(&self.graph, source) == Some(id)
                    || node_of_port(&self.graph, destination) == Some(id)
            }
            GraphSubject::Graph => false,
        }
    }

    /// Node kind of `id`, for surfaces that degrade gracefully when the
    /// topology row behind a state entry has already vanished.
    pub fn kind_of(&self, id: NodeId) -> Option<NodeKind> {
        self.graph.get_node(id).ok().map(|node| node.kind())
    }

    /// The primary node of `kind` paired with its state. Every layer that
    /// needs "the one Source/Sink/Gain/Pan" (UI selection, observation
    /// targets, meter wiring, preparation) must agree on this choice; see
    /// [`primary_node_of`] and [`GraphPreparation::primary_io_nodes`].
    pub fn node_of_kind(
        &self,
        kind: NodeKind,
    ) -> Option<(&moiren_core::graph::Node, &GraphNodeState)> {
        let node = primary_node_of(&self.graph, kind)?;
        Some((node, self.node(node.id())?))
    }
}

/// The primary node of `kind`: the first match in graph order. Node ids are
/// only ever appended, so this is also the lowest id — the same node the
/// meter bridge's former `min_by_key` scan and preparation's `min` resolved.
pub(crate) fn primary_node_of(
    graph: &LogicalGraph,
    kind: NodeKind,
) -> Option<&moiren_core::graph::Node> {
    graph.nodes().iter().find(|node| node.kind() == kind)
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraphEdit {
    SetExports(Vec<ExportIntent>),
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
    /// Connect that first displaces whatever currently feeds
    /// `destination`. Input ports are single-connection, so a drop on an
    /// occupied input reroutes it instead of failing: one candidate
    /// revision, one undo step, and if the connect half fails the whole
    /// candidate rolls back with the displaced edge intact.
    ConnectReplacing {
        source: PortId,
        destination: PortId,
    },
    /// Move an existing edge's destination, displacing whatever feeds the
    /// new one: the drag-a-wire-head-off-an-input gesture commits as one
    /// atomic edit, so a reroute is a single undo step and a failed
    /// connect half rolls the moved edge (and the displaced one) back
    /// exactly where they were.
    Reconnect {
        edge: EdgeId,
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
    SetNodeTint {
        node: NodeId,
        tint: Option<u8>,
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
    /// Assign a Plugin node's plugin (resolved by stable id against the
    /// service's plugin library at preparation time) or clear the
    /// assignment. Assignment is structural: it rebuilds the node's
    /// processor. `adapt_channels` also resizes the node to the assigned
    /// plugin's bus width as part of the same edit and undo step — the
    /// picker only proposes it when no connected route would strand (the
    /// core rejects a resize that would leave an edge width-mismatched), so
    /// a bind that cannot adapt safely keeps the preparation width fault as
    /// the presentation.
    BindPlugin {
        node: NodeId,
        assignment: Option<PluginAssignment>,
        adapt_channels: Option<usize>,
    },
    BindExport {
        endpoint_id: moiren_engine::node::VirtualEndpointId,
        node: Option<NodeId>,
    },
}

impl GraphEdit {
    /// Undo coalescing identity: rapid edits of the same control collapse
    /// into one undo step. `None` marks structural edits, which never
    /// coalesce with anything.
    fn undo_key(&self) -> Option<String> {
        match self {
            Self::SetCompressor {
                node, parameter, ..
            } => Some(format!("compressor:{node:?}:{}", parameter.label())),
            Self::SetGain { node, .. } => Some(format!("gain:{node:?}")),
            Self::SetPan { node, .. } => Some(format!("pan:{node:?}")),
            Self::SetMono { node, .. } => Some(format!("mono:{node:?}")),
            Self::SetSwap { node, .. } => Some(format!("swap:{node:?}")),
            Self::SetSend { edge, .. } => Some(format!("send:{edge:?}")),
            Self::SetMixer { node, .. } => Some(format!("mixer:{node:?}")),
            Self::SetBuiltin { node, .. } => Some(format!("builtin:{node:?}")),
            Self::SetLabel { node, .. } => Some(format!("label:{node:?}")),
            Self::SetNodeTint { node, .. } => Some(format!("tint:{node:?}")),
            Self::BindPlugin { node, .. } => Some(format!("plugin:{node:?}")),
            _ => None,
        }
    }
}

/// Owned worker preparation input; no UI entity or native handle crosses here.
#[derive(Debug, Clone)]
pub struct GraphPreparation {
    pub exports: Vec<ExportIntent>,
    pub graph: LogicalGraph,
    pub nodes: Vec<GraphNodeState>,
    pub revision: u64,
    /// The plugin library Plugin-node assignments resolve against; the
    /// lifecycle caller stamps the service's current snapshot in before
    /// translating to an engine project.
    pub plugins: std::sync::Arc<PluginLibrary>,
}
pub struct ServiceGraph {
    snapshot: GraphSnapshot,
    journal: UndoJournal,
    pending_generation: Option<SessionGeneration>,
    /// Content token for the snapshot publisher: bumped by every mutation
    /// that can change the projected [`GraphSnapshot`]. The service loop
    /// compares this instead of rebuilding the graph projection every
    /// round, so each mutation path in this file must keep its bump in
    /// step with the writes it performs.
    state_epoch: u64,
}
impl Default for ServiceGraph {
    fn default() -> Self {
        // The fixed stereo session graph: input -> gain -> pan -> output.
        let mut graph = moiren_core::graph::LogicalGraph::new();
        let chain = [
            (NodeKind::Source, None),
            (NodeKind::Gain, Some(0.0)),
            (NodeKind::Pan, None),
            (NodeKind::Sink, None),
        ];
        let mut chain_ids = Vec::new();
        for (kind, _) in chain {
            chain_ids.push(graph.create_node(kind, 2).expect("fixed stereo chain"));
        }
        for pair in chain_ids.windows(2) {
            let (source, destination) = (pair[0], pair[1]);
            graph
                .connect(
                    graph.get_node(source).unwrap().outputs()[0].id(),
                    graph.get_node(destination).unwrap().inputs()[0].id(),
                    moiren_core::graph::SendParams::default(),
                )
                .expect("fixed stereo chain");
        }
        let nodes = graph
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
                plugin: None,
                tint: None,
            })
            .collect();
        Self {
            snapshot: GraphSnapshot {
                exports: Vec::new(),
                graph,
                nodes,
                candidate_revision: 1,
                active_revision: None,
                pending_revision: None,
                last_error: None,
                faults: Vec::new(),
                can_undo: false,
                can_redo: false,
                preparing_nodes: Vec::new(),
                recording: Vec::new(),
            },
            journal: UndoJournal::default(),
            pending_generation: None,
            state_epoch: 0,
        }
    }
}
/// Default label for freshly created nodes. A Source/Sink node still carrying
/// its default label follows the bound device name in presentation instead.
///
/// Ratified: labels are user-editable data, not UI chrome, so the canonical
/// stored strings are intentionally English and are never localized — only
/// genuinely translatable chrome goes through `t!`.
pub fn default_label(kind: NodeKind) -> &'static str {
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
        NodeKind::Plugin => "Plugin",
    }
}
impl ServiceGraph {
    pub fn snapshot(&self) -> GraphSnapshot {
        self.snapshot.clone()
    }
    /// Shared view of the current snapshot for read-only projections. Poll
    /// paths that only inspect a field or feed `statuses` must take this
    /// instead of the deep [`ServiceGraph::snapshot`] copy.
    pub fn snapshot_ref(&self) -> &GraphSnapshot {
        &self.snapshot
    }
    /// Light revision accessors: reading one field must not pay the full
    /// snapshot deep copy.
    pub fn candidate_revision(&self) -> u64 {
        self.snapshot.candidate_revision
    }
    pub fn active_revision(&self) -> Option<u64> {
        self.snapshot.active_revision
    }
    pub fn pending_revision(&self) -> Option<u64> {
        self.snapshot.pending_revision
    }
    /// Content token advanced by every snapshot mutation below.
    pub fn state_epoch(&self) -> u64 {
        self.state_epoch
    }
    fn bump(&mut self) {
        self.state_epoch = self.state_epoch.wrapping_add(1);
    }
    pub fn preparation(&self) -> GraphPreparation {
        GraphPreparation {
            exports: self.snapshot.exports.clone(),
            graph: self.snapshot.graph.clone(),
            nodes: self.snapshot.nodes.clone(),
            revision: self.snapshot.candidate_revision,
            plugins: super::plugins::empty_library(),
        }
    }
    pub fn record_error(&mut self, error: impl ToString) {
        let error = error.to_string();
        self.record_fault(GraphFault {
            subject: GraphSubject::Graph,
            cause: error.clone(),
            hint: None,
        });
        if self.snapshot.last_error.as_deref() != Some(error.as_str()) {
            self.snapshot.last_error = Some(error);
            self.bump();
        }
    }
    /// Record a fault whose subject is already known (a rejected edit names
    /// its own channel or route).
    pub fn record_error_with_subject(&mut self, subject: GraphSubject, error: &str) {
        self.record_fault(GraphFault {
            subject,
            cause: error.to_string(),
            hint: edit_hint(error).map(str::to_string),
        });
        if self.snapshot.last_error.as_deref() != Some(error) {
            self.snapshot.last_error = Some(error.to_string());
            self.bump();
        }
    }
    /// Returns whether the fault was new content; duplicate faults are
    /// dropped without changing the snapshot.
    pub fn record_fault(&mut self, fault: GraphFault) -> bool {
        let fresh = !self
            .snapshot
            .faults
            .iter()
            .any(|existing| existing.subject == fault.subject && existing.cause == fault.cause);
        if fresh {
            self.snapshot.faults.push(fault);
            self.bump();
        }
        fresh
    }
    pub fn configure_session(&mut self, spec: &SessionSpec) -> Result<(), String> {
        spec.validate().map_err(|e| format!("{e:?}"))?;
        let preparation = self.preparation();
        let (source, output) = preparation.primary_io_nodes()?;
        // The legacy session summary binds both endpoints; a one-sided graph
        // cannot carry it (the always-on graph path has no such requirement).
        let (Some(source), Some(output)) = (source, output) else {
            return Err("The graph needs a Source node and a Sink node".into());
        };
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
                .ok_or("The graph revision space is exhausted")?;
            next.last_error = None;
            self.snapshot = next;
            self.bump();
        }
        Ok(())
    }
    /// Only legacy sessions have a two-endpoint summary. The managed host uses
    /// the validated EngineProject for virtual or mixed graph resource intent.
    /// A project paced by the virtual clock (no physical master output), a
    /// one-sided graph, or one whose primary IO is a virtual export boundary
    /// has no summary at all.
    pub(crate) fn managed_session_spec(
        &self,
        current: Option<&SessionSpec>,
    ) -> Result<Option<SessionSpec>, String> {
        let prep = self.preparation();
        let (source, output) = prep.primary_io_nodes()?;
        if clock_master(&prep.graph, &prep.exports).is_none() {
            return Ok(None);
        }
        let (Some(source), Some(output)) = (source, output) else {
            return Ok(None);
        };
        if prep.exports.iter().any(|export| {
            export.enabled
                && matches!(export.binding, Some(node) if node == source || node == output)
        }) {
            return Ok(None);
        }
        Ok(Some(self.session_spec(current)?))
    }
    pub fn session_spec(&self, current: Option<&SessionSpec>) -> Result<SessionSpec, String> {
        let prep = self.preparation();
        let (source, output) = prep.primary_io_nodes()?;
        let source = source
            .and_then(|node| self.snapshot.node(node))
            .and_then(|s| s.source.clone())
            .ok_or("Choose an input resource")?;
        let output_endpoint_id = output
            .and_then(|node| self.snapshot.node(node))
            .and_then(|s| s.output_endpoint_id.clone())
            .ok_or("Choose an output device")?;
        let mut spec = current.cloned().unwrap_or(SessionSpec {
            source: source.clone(),
            output_endpoint_id: output_endpoint_id.clone(),
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
}
#[cfg(test)]
mod tests;
