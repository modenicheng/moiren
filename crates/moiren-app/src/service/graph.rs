//! The service owns the editable topology. Pages receive immutable projections;
//! only validated transactions replace the candidate and native startup confirms it.
use super::{
    ExportIntent, InputSelection, RunLimit, SessionGeneration, SessionSpec, validate_exports,
};
use moiren_core::graph::{EdgeId, GraphError, LogicalGraph, NodeId, NodeKind, PortId, SendParams};
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
    /// Whether the undo/redo journal can step back / forward right now.
    pub can_undo: bool,
    pub can_redo: bool,
}
impl GraphSnapshot {
    pub fn node(&self, id: NodeId) -> Option<&GraphNodeState> {
        self.nodes.iter().find(|node| node.id == id)
    }
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
    /// Value-like edits a user emits in rapid bursts while dragging a control.
    /// The UI debounces the instant-apply follow-up for these; structural
    /// edits apply the moment they land.
    pub fn is_continuous_adjustment(&self) -> bool {
        matches!(
            self,
            Self::SetCompressor { .. }
                | Self::SetGain { .. }
                | Self::SetPan { .. }
                | Self::SetMono { .. }
                | Self::SetSwap { .. }
                | Self::SetSend { .. }
                | Self::SetMixer { .. }
                | Self::SetBuiltin { .. }
        )
    }

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
                exports: Vec::new(),
                graph: session.graph,
                nodes,
                candidate_revision: 1,
                active_revision: None,
                pending_revision: None,
                last_error: None,
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
/// explicitly ignore the edge-targeted `SetSend` values.
pub(crate) fn edit_parameter_values(edit: &GraphEdit) -> Vec<(ParameterTarget, ParamValue)> {
    use moiren_engine::{
        host::SendParameter,
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
        return Err("Mono 和 Swap 只能用于 Balance 节点".into());
    }
    if !state.compressor.valid() {
        return Err("Compressor 参数超出有效范围".into());
    }
    if !state.mixer.valid(channels) {
        return Err("Mixer 增益或声道模式无效".into());
    }
    if state.label.trim().is_empty() || state.label.len() > 256 || state.label.contains('\0') {
        return Err("节点名称不能为空且最多 256 字节".into());
    }
    if !state.gain.is_finite() || !(0.0..=16.0).contains(&state.gain) {
        return Err("Gain 节点增益必须为 0..16 的有限值".into());
    }
    if !state.pan.is_finite() || !(-1.0..=1.0).contains(&state.pan) {
        return Err("Balance 节点位置必须为 -1..1 的有限值".into());
    }
    if let Some(source) = &state.source {
        if kind != NodeKind::Source {
            return Err("只有输入节点可以绑定音源".into());
        }
        source
            .validate()
            .map_err(|_| "输入音源配置无效".to_string())?;
    }
    if let Some(output) = &state.output_endpoint_id
        && (kind != NodeKind::Sink || output.is_empty() || output.contains('\0'))
    {
        return Err("只有输出节点可以绑定有效设备".into());
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
        self.snapshot.last_error = Some(error.to_string());
    }
    pub fn edit(&mut self, edit: GraphEdit) -> Result<(), String> {
        let before = self.snapshot.clone();
        let key = edit.undo_key();
        let mut next = self.snapshot.clone();
        let result = edit_candidate(&mut next, edit);
        if let Err(error) = result {
            self.record_error(&error);
            return Err(error);
        }
        next.graph.validate().map_err(|e| e.to_string())?;
        if let Err(error) = validate_exports(&next.exports, &next.graph) {
            self.record_error(&error);
            return Err(error);
        }
        next.candidate_revision = next
            .candidate_revision
            .checked_add(1)
            .ok_or("图版本空间已耗尽")?;
        next.last_error = None;
        self.journal.record(before, key);
        next.can_undo = !self.journal.undo.is_empty();
        next.can_redo = !self.journal.redo.is_empty();
        self.snapshot = next;
        Ok(())
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
            .ok_or("图版本空间已耗尽")?;
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
                .ok_or("图版本空间已耗尽")?;
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
