//! Applying a validated candidate edit: fault placement, live parameter
//! mapping, per-node state validation, and the transaction that commits a
//! new candidate revision.
use super::edit::edit_candidate;
use super::{GraphEdit, GraphNodeState, GraphSubject, ServiceGraph};
use moiren_core::graph::{LogicalGraph, NodeId, NodeKind, PortId};
use moiren_core::protocol::ParamValue;
use moiren_engine::host::ParameterTarget;
use std::collections::BTreeSet;

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

/// The node a port belongs to, for preparing-state placement.
pub(super) fn node_of_port(graph: &LogicalGraph, port: PortId) -> Option<NodeId> {
    graph
        .nodes()
        .iter()
        .find(|node| {
            node.inputs().iter().any(|candidate| candidate.id() == port)
                || node
                    .outputs()
                    .iter()
                    .any(|candidate| candidate.id() == port)
        })
        .map(|node| node.id())
}

/// The channel or route an edit names, for fault placement.
fn edit_subject(edit: &GraphEdit, graph: &LogicalGraph) -> GraphSubject {
    let node = |node: &NodeId| GraphSubject::Node(*node);
    match edit {
        GraphEdit::SetBuiltin { node: id, .. }
        | GraphEdit::SetCompressor { node: id, .. }
        | GraphEdit::ClearBinding(id)
        | GraphEdit::SetMixer { node: id, .. }
        | GraphEdit::RemoveNode(id)
        | GraphEdit::SetLabel { node: id, .. }
        | GraphEdit::SetNodeTint { node: id, .. }
        | GraphEdit::SetGain { node: id, .. }
        | GraphEdit::SetPan { node: id, .. }
        | GraphEdit::SetMono { node: id, .. }
        | GraphEdit::SetSwap { node: id, .. }
        | GraphEdit::BindSource { node: id, .. }
        | GraphEdit::BindOutput { node: id, .. }
        | GraphEdit::BindPlugin { node: id, .. }
        | GraphEdit::AddBusInput(id)
        | GraphEdit::RemoveBusInput { node: id, .. } => node(id),
        GraphEdit::Connect {
            source,
            destination,
        }
        | GraphEdit::ConnectReplacing {
            source,
            destination,
        } => GraphSubject::Route(*source, *destination),
        GraphEdit::Disconnect(edge) | GraphEdit::SetSend { edge, .. } => graph
            .get_edge(*edge)
            .map(|edge| GraphSubject::Route(edge.src_port(), edge.dst_port()))
            .unwrap_or(GraphSubject::Graph),
        GraphEdit::Reconnect { edge, destination } => graph
            .get_edge(*edge)
            .map(|moved| GraphSubject::Route(moved.src_port(), *destination))
            .unwrap_or(GraphSubject::Graph),
        GraphEdit::SetExports(_) | GraphEdit::BindExport { .. } | GraphEdit::AddNode { .. } => {
            GraphSubject::Graph
        }
    }
}

/// Plain-language fixes for the common validation causes. Causes and hints
/// are English catalog keys; the UI translates them when rendering, so the
/// matching here keys on stable English substrings regardless of locale.
pub(super) fn edit_hint(error: &str) -> Option<&'static str> {
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
fn edit_affects_prepared_structure(edit: &GraphEdit, previous: &LogicalGraph) -> bool {
    match edit {
        GraphEdit::AddNode { .. }
        | GraphEdit::RemoveNode(_)
        | GraphEdit::Connect { .. }
        | GraphEdit::ConnectReplacing { .. }
        | GraphEdit::Reconnect { .. }
        | GraphEdit::Disconnect(_)
        | GraphEdit::AddBusInput(_)
        | GraphEdit::RemoveBusInput { .. }
        | GraphEdit::SetExports(_)
        | GraphEdit::BindExport { .. }
        | GraphEdit::SetBuiltin { .. }
        | GraphEdit::BindSource { .. }
        | GraphEdit::BindOutput { .. }
        | GraphEdit::BindPlugin { .. }
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
        | GraphEdit::SetLabel { .. }
        | GraphEdit::SetNodeTint { .. } => false,
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
    if let Some(plugin) = &state.plugin {
        if kind != NodeKind::Plugin {
            return Err("Only Plugin nodes can carry a plugin assignment".into());
        }
        // Library keys are short printable composites; the bound is generous.
        if plugin.id.trim().is_empty() || plugin.id.len() > 1024 || plugin.id.contains('\0') {
            return Err("The plugin assignment id is invalid".into());
        }
        if plugin
            .preset
            .as_ref()
            .is_some_and(|preset| preset.len() > 16 * 1024 * 1024)
        {
            return Err("The plugin preset is too large".into());
        }
        // The two preset forms resolve differently (state chunk vs factory
        // program index); both set is a corrupt assignment, not a fallback.
        if plugin.preset.is_some() && plugin.preset_program.is_some() {
            return Err(
                "A plugin assignment carries either a state chunk or a factory preset, not both"
                    .into(),
            );
        }
    }
    Ok(())
}

impl ServiceGraph {
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
        if let Err(error) = super::validate_exports(&next.exports, &next.graph) {
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
        if structural {
            // The engine will re-prepare these nodes; keep any still
            // pending ones so cards stay in the loading state.
            let mut pending: BTreeSet<NodeId> = next.preparing_nodes.drain(..).collect();
            match &subject {
                GraphSubject::Node(id) => {
                    pending.insert(*id);
                }
                GraphSubject::Route(source, destination) => {
                    pending.extend(node_of_port(&previous, *source));
                    pending.extend(node_of_port(&previous, *destination));
                    // A reroute also re-prepares the node it departs from.
                    if let GraphEdit::Reconnect { edge, .. } = &edit
                        && let Ok(moved) = previous.get_edge(*edge)
                    {
                        pending.extend(node_of_port(&previous, moved.dst_port()));
                    }
                }
                GraphSubject::Graph => {
                    // AddNode and export changes: the topology diff.
                    let before: BTreeSet<NodeId> =
                        previous.nodes().iter().map(|node| node.id()).collect();
                    pending.extend(
                        next.graph
                            .nodes()
                            .iter()
                            .map(|node| node.id())
                            .filter(|id| !before.contains(id)),
                    );
                }
            }
            next.preparing_nodes = pending.into_iter().collect();
        }
        self.journal.record(before, edit.undo_key());
        next.can_undo = !self.journal.undo.is_empty();
        next.can_redo = !self.journal.redo.is_empty();
        self.snapshot = next;
        // The accepted edit always advances the candidate revision, so this
        // is a real content change.
        self.bump();
        Ok(structural)
    }
}
