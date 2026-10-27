//! The service owns the editable topology. Pages receive immutable projections;
//! only validated transactions replace the candidate and native startup confirms it.
use super::{
    ExportIntent, InputSelection, RunLimit, SessionGeneration, SessionSpec, validate_exports,
};
use moiren_core::graph::{EdgeId, LogicalGraph, NodeId, NodeKind, PortId, SendParams};
use moiren_core::protocol::ParamValue;
use moiren_engine::host::ParameterTarget;
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
}
impl GraphSnapshot {
    pub fn node(&self, id: NodeId) -> Option<&GraphNodeState> {
        self.nodes.iter().find(|node| node.id == id)
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
            _ => None,
        }
    }
}

/// How long successive same-key edits stay merged into one undo step.
const UNDO_COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
/// Upper bound on remembered undo states; oldest entries drop off first.
const UNDO_DEPTH: usize = 100;

/// Snapshot journal for undo/redo. Entries hold only the editable candidate
/// content; revision and runtime confirmation fields stay with the live
/// snapshot so an undo can never fabricate or hide an owner confirmation.
#[derive(Default)]
struct UndoJournal {
    undo: Vec<GraphSnapshot>,
    redo: Vec<GraphSnapshot>,
    last: Option<(String, std::time::Instant)>,
}

impl UndoJournal {
    fn record(&mut self, before: GraphSnapshot, key: Option<String>) {
        let now = std::time::Instant::now();
        let coalesced = key.as_ref().is_some_and(|key| {
            self.last.as_ref().is_some_and(|(last, at)| {
                *last == *key && now.duration_since(*at) < UNDO_COALESCE_WINDOW
            })
        });
        if !coalesced {
            self.undo.push(before);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.last = key.map(|key| (key, now));
    }
}

/// Owned worker preparation input; no UI entity or native handle crosses here.
#[derive(Debug, Clone)]
pub struct GraphPreparation {
    pub exports: Vec<ExportIntent>,
    pub graph: LogicalGraph,
    pub nodes: Vec<GraphNodeState>,
    pub revision: u64,
}
pub struct ServiceGraph {
    snapshot: GraphSnapshot,
    journal: UndoJournal,
    pending_generation: Option<SessionGeneration>,
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
            },
            journal: UndoJournal::default(),
            pending_generation: None,
        }
    }
}
/// Default label for freshly created nodes. A Source/Sink node still carrying
/// its default label follows the bound device name in presentation instead.
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
    }
}
/// Real-time parameter intent derived from a graph edit. This mapping is the
/// single canonical source for every driver: the managed driver submits all
/// values through engine control, while the legacy driver admits only the
/// single node-addressed values its fixed session chain can address and must
/// explicitly ignore the edge- and strip-targeted values.
pub(crate) fn edit_parameter_values(edit: &GraphEdit) -> Vec<(ParameterTarget, ParamValue)> {
    use moiren_engine::{
        host::{SendParameter, StripParameter},
        processor::{Gain, Pan},
    };
    let node = |node, parameter, value| vec![(ParameterTarget::Node { node, parameter }, value)];
    match edit {
        GraphEdit::SetCompressor {
            node: id,
            parameter,
            value,
        } => node(*id, parameter.id(), ParamValue::Float(*value)),
        GraphEdit::SetGain { node: id, gain } => node(*id, Gain::LEVEL, ParamValue::Float(*gain)),
        GraphEdit::SetPan { node: id, pan } => node(*id, Pan::POSITION, ParamValue::Float(*pan)),
        GraphEdit::SetMono { node: id, mono } => node(*id, Pan::MONO, ParamValue::Bool(*mono)),
        GraphEdit::SetSwap { node: id, swap } => node(*id, Pan::SWAP, ParamValue::Bool(*swap)),
        // Direct strip fields. The effective gain and mute (dim fold, solo
        // audibility) depend on the whole graph and are submitted separately
        // by the managed driver's strip synchronization.
        GraphEdit::SetMixer { node: id, state } => {
            let strip = |parameter, value| {
                (
                    ParameterTarget::Strip {
                        node: *id,
                        parameter,
                    },
                    value,
                )
            };
            vec![
                strip(StripParameter::Balance, ParamValue::Float(state.balance)),
                strip(
                    StripParameter::Mode,
                    ParamValue::Enum(state.channel_mode as u32),
                ),
            ]
        }
        GraphEdit::SetSend { edge, params } => vec![
            (
                ParameterTarget::Edge {
                    edge: *edge,
                    parameter: SendParameter::Gain,
                },
                ParamValue::Float(params.gain),
            ),
            (
                ParameterTarget::Edge {
                    edge: *edge,
                    parameter: SendParameter::Pan,
                },
                ParamValue::Float(params.pan),
            ),
            (
                ParameterTarget::Edge {
                    edge: *edge,
                    parameter: SendParameter::Mute,
                },
                ParamValue::Bool(params.mute),
            ),
        ],
        _ => Vec::new(),
    }
}
/// The channel or route an edit names, for fault placement.
pub(crate) fn edit_subject(edit: &GraphEdit, graph: &LogicalGraph) -> GraphSubject {
    let node = |node: &NodeId| GraphSubject::Node(*node);
    match edit {
        GraphEdit::SetBuiltin { node: id, .. }
        | GraphEdit::SetCompressor { node: id, .. }
        | GraphEdit::ClearBinding(id)
        | GraphEdit::SetMixer { node: id, .. }
        | GraphEdit::RemoveNode(id)
        | GraphEdit::SetLabel { node: id, .. }
        | GraphEdit::SetGain { node: id, .. }
        | GraphEdit::SetPan { node: id, .. }
        | GraphEdit::SetMono { node: id, .. }
        | GraphEdit::SetSwap { node: id, .. }
        | GraphEdit::BindSource { node: id, .. }
        | GraphEdit::BindOutput { node: id, .. }
        | GraphEdit::AddBusInput(id)
        | GraphEdit::RemoveBusInput { node: id, .. } => node(id),
        GraphEdit::Connect {
            source,
            destination,
        } => GraphSubject::Route(*source, *destination),
        GraphEdit::Disconnect(edge) | GraphEdit::SetSend { edge, .. } => graph
            .get_edge(*edge)
            .map(|edge| GraphSubject::Route(edge.src_port(), edge.dst_port()))
            .unwrap_or(GraphSubject::Graph),
        GraphEdit::SetExports(_) | GraphEdit::AddNode { .. } => GraphSubject::Graph,
    }
}

/// Plain-language fixes for the common validation causes. Causes and hints
/// are English catalog keys; the UI translates them when rendering, so the
/// matching here keys on stable English substrings regardless of locale.
fn edit_hint(error: &str) -> Option<&'static str> {
    if error.contains("not bound to an input source") {
        Some("Choose an input source at the top of the channel, or delete the channel")
    } else if error.contains("not bound to an output device") {
        Some("Choose an output device at the top of the channel, or delete the channel")
    } else if error.contains("Mixer") {
        Some("Faders cover −∞ to +24 dB; cycle the channel-mode button back to a valid setting")
    } else if error.contains("Mono and Swap") {
        Some("These two switches only apply to Balance channels")
    } else if error.contains("the same") {
        Some("Each device or application source can feed one channel; unbind the other one first")
    } else {
        None
    }
}

/// Whether an edit changes what a prepared plan must rebuild. Value edits of
/// parameters with live mappings (gain/pan/mono/swap/compressor, sends,
/// strips) route through engine control instead; a send tap change always
/// rebuilds because it rewires prepared routing. `previous` is the graph
/// before the edit. Drivers whose backend lacks the live strip mapping
/// (the legacy fixed session chain) treat `SetMixer` as structural.
pub(crate) fn edit_affects_prepared_structure(edit: &GraphEdit, previous: &LogicalGraph) -> bool {
    match edit {
        GraphEdit::AddNode { .. }
        | GraphEdit::RemoveNode(_)
        | GraphEdit::Connect { .. }
        | GraphEdit::Disconnect(_)
        | GraphEdit::AddBusInput(_)
        | GraphEdit::RemoveBusInput { .. }
        | GraphEdit::SetExports(_)
        | GraphEdit::SetBuiltin { .. }
        | GraphEdit::BindSource { .. }
        | GraphEdit::BindOutput { .. }
        | GraphEdit::ClearBinding(_) => true,
        GraphEdit::SetSend { edge, params } => previous
            .get_edge(*edge)
            .is_ok_and(|edge| edge.params().tap != params.tap),
        GraphEdit::SetMixer { .. }
        | GraphEdit::SetCompressor { .. }
        | GraphEdit::SetGain { .. }
        | GraphEdit::SetPan { .. }
        | GraphEdit::SetMono { .. }
        | GraphEdit::SetSwap { .. }
        | GraphEdit::SetLabel { .. } => false,
    }
}
/// Single source for per-node candidate-state domains. Every layer that
/// admits graph state — candidate edits, session preparation, and project
/// documents — routes through here; callers keep their own location context
/// for error reporting but never restate the rules or their messages.
pub fn validate_node_state(
    state: &GraphNodeState,
    kind: NodeKind,
    input_channels: usize,
    channels: usize,
) -> Result<(), String> {
    state.builtin.validate(kind, input_channels, channels)?;
    if kind != NodeKind::Pan && (state.mono || state.swap) {
        return Err("Mono and Swap only apply to Balance nodes".into());
    }
    if !state.compressor.valid() {
        return Err("Compressor parameters are outside their valid range".into());
    }
    if !state.mixer.valid(channels) {
        return Err("Mixer gain or channel mode is invalid".into());
    }
    if state.label.trim().is_empty() || state.label.len() > 256 || state.label.contains('\0') {
        return Err("Node names cannot be empty and are limited to 256 bytes".into());
    }
    if !state.gain.is_finite() || !(0.0..=16.0).contains(&state.gain) {
        return Err("The Gain node's gain must be a finite value in 0..16".into());
    }
    if !state.pan.is_finite() || !(-1.0..=1.0).contains(&state.pan) {
        return Err("The Balance node's position must be a finite value in -1..1".into());
    }
    if let Some(source) = &state.source {
        if kind != NodeKind::Source {
            return Err("Only Source nodes can bind an input source".into());
        }
        source
            .validate()
            .map_err(|_| "The input source configuration is invalid".to_string())?;
    }
    if let Some(output) = &state.output_endpoint_id
        && (kind != NodeKind::Sink || output.is_empty() || output.contains('\0'))
    {
        return Err("Only Sink nodes can bind a valid output device".into());
    }
    Ok(())
}
impl ServiceGraph {
    pub fn snapshot(&self) -> GraphSnapshot {
        self.snapshot.clone()
    }
    pub fn preparation(&self) -> GraphPreparation {
        GraphPreparation {
            exports: self.snapshot.exports.clone(),
            graph: self.snapshot.graph.clone(),
            nodes: self.snapshot.nodes.clone(),
            revision: self.snapshot.candidate_revision,
        }
    }
    pub fn record_error(&mut self, error: impl ToString) {
        let error = error.to_string();
        self.record_fault(GraphFault {
            subject: GraphSubject::Graph,
            cause: error.clone(),
            hint: None,
        });
        self.snapshot.last_error = Some(error);
    }
    /// Record a fault whose subject is already known (a rejected edit names
    /// its own channel or route).
    pub fn record_error_with_subject(&mut self, subject: GraphSubject, error: &str) {
        self.record_fault(GraphFault {
            subject,
            cause: error.to_string(),
            hint: edit_hint(error).map(str::to_string),
        });
        self.snapshot.last_error = Some(error.to_string());
    }
    pub fn record_fault(&mut self, fault: GraphFault) {
        if !self
            .snapshot
            .faults
            .iter()
            .any(|existing| existing.subject == fault.subject && existing.cause == fault.cause)
        {
            self.snapshot.faults.push(fault);
        }
    }
    /// Applies a validated edit and reports whether the prepared structure
    /// changed and a rebuild is required (see
    /// [`edit_affects_prepared_structure`]).
    pub fn edit(&mut self, edit: GraphEdit) -> Result<bool, String> {
        let before = self.snapshot.clone();
        let previous = self.snapshot.graph.clone();
        let subject = edit_subject(&edit, &previous);
        let mut next = self.snapshot.clone();
        let result = edit_candidate(&mut next, edit.clone());
        if let Err(error) = result {
            self.record_error_with_subject(subject, &error);
            return Err(error);
        }
        if let Err(error) = next.graph.validate().map_err(|e| e.to_string()) {
            self.record_error_with_subject(subject, &error);
            return Err(error);
        }
        if let Err(error) = validate_exports(&next.exports, &next.graph) {
            self.record_error_with_subject(subject, &error);
            return Err(error);
        }
        next.candidate_revision = next
            .candidate_revision
            .checked_add(1)
            .ok_or("The graph revision space is exhausted")?;
        next.last_error = None;
        next.faults.clear();
        let structural = edit_affects_prepared_structure(&edit, &previous);
        self.journal.record(before, edit.undo_key());
        next.can_undo = !self.journal.undo.is_empty();
        next.can_redo = !self.journal.redo.is_empty();
        self.snapshot = next;
        Ok(structural)
    }
    /// Restore the editable content recorded before the last edit. Revision
    /// and runtime confirmation fields stay live; the restored content gets a
    /// fresh candidate revision so consumers never see a revision go back.
    pub fn undo(&mut self) -> bool {
        self.step_journal(true)
    }
    pub fn redo(&mut self) -> bool {
        self.step_journal(false)
    }
    fn step_journal(&mut self, back: bool) -> bool {
        let entry = if back {
            self.journal.undo.pop()
        } else {
            self.journal.redo.pop()
        };
        let Some(entry) = entry else {
            return false;
        };
        if back {
            self.journal.redo.push(self.snapshot.clone());
        } else {
            self.journal.undo.push(self.snapshot.clone());
        }
        self.journal.last = None;
        let revision = self
            .snapshot
            .candidate_revision
            .checked_add(1)
            .expect("revision headroom retained by undo depth");
        let can_undo = !self.journal.undo.is_empty();
        let can_redo = !self.journal.redo.is_empty();
        self.snapshot.exports = entry.exports;
        self.snapshot.graph = entry.graph;
        self.snapshot.nodes = entry.nodes;
        self.snapshot.last_error = None;
        self.snapshot.faults.clear();
        self.snapshot.candidate_revision = revision;
        self.snapshot.can_undo = can_undo;
        self.snapshot.can_redo = can_redo;
        true
    }
    /// A loaded document creates a new candidate. Disk contents cannot confirm
    /// native activity, and stale in-flight preparations never confirm this edit.
    pub fn replace(
        &mut self,
        graph: LogicalGraph,
        nodes: Vec<GraphNodeState>,
    ) -> Result<(), String> {
        self.replace_with_exports(graph, nodes, Vec::new())
    }

    pub fn replace_with_exports(
        &mut self,
        graph: LogicalGraph,
        nodes: Vec<GraphNodeState>,
        exports: Vec<ExportIntent>,
    ) -> Result<(), String> {
        GraphPreparation {
            exports: exports.clone(),
            graph: graph.clone(),
            nodes: nodes.clone(),
            revision: 0,
        }
        .validate_state()?;
        let revision = self
            .snapshot
            .candidate_revision
            .checked_add(1)
            .ok_or("The graph revision space is exhausted")?;
        self.journal.record(self.snapshot.clone(), None);
        self.snapshot.graph = graph;
        self.snapshot.exports = exports;
        self.snapshot.nodes = nodes;
        self.snapshot.candidate_revision = revision;
        self.snapshot.last_error = None;
        self.snapshot.can_undo = !self.journal.undo.is_empty();
        self.snapshot.can_redo = !self.journal.redo.is_empty();
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
                .ok_or("The graph revision space is exhausted")?;
            next.last_error = None;
            self.snapshot = next;
        }
        Ok(())
    }
    /// Only legacy sessions have a two-endpoint summary. The managed host uses
    /// the validated EngineProject for virtual or mixed graph resource intent.
    pub(crate) fn managed_session_spec(
        &self,
        current: Option<&SessionSpec>,
        limit: Option<RunLimit>,
    ) -> Result<Option<SessionSpec>, String> {
        let prep = self.preparation();
        let (source, output) = prep.primary_io_nodes()?;
        if prep.exports.iter().any(|export| {
            export.enabled
                && matches!(export.binding, Some(node) if node == source || node == output)
        }) {
            return Ok(None);
        }
        let mut summary = self.session_spec(current)?;
        if let Some(limit) = limit {
            summary.limit = limit;
        }
        Ok(Some(summary))
    }
    pub fn session_spec(&self, current: Option<&SessionSpec>) -> Result<SessionSpec, String> {
        let prep = self.preparation();
        let (source, output) = prep.primary_io_nodes()?;
        let source = self
            .snapshot
            .node(source)
            .and_then(|s| s.source.clone())
            .ok_or("Choose an input resource")?;
        let output_endpoint_id = self
            .snapshot
            .node(output)
            .and_then(|s| s.output_endpoint_id.clone())
            .ok_or("Choose an output device")?;
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
            self.snapshot.faults.clear();
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
    /// A rebuild rejected by the engine: keep the old topology running and
    /// report the located fault.
    pub fn preparation_rejected(
        &mut self,
        generation: SessionGeneration,
        fault: GraphFault,
        error: String,
    ) {
        if self.pending_generation == Some(generation) {
            self.snapshot.pending_revision = None;
            self.pending_generation = None;
            self.record_fault(fault);
            self.snapshot.last_error = Some(error);
        }
    }
}
#[cfg(test)]
mod tests;
