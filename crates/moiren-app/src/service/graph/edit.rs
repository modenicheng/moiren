use super::*;

/// After a state-mutating edit, re-run the single-source node domains on that
/// node. The caller discards `next` wholesale on failure, so no revert is
/// needed here.
fn validate_edited_node(next: &GraphSnapshot, node: NodeId) -> Result<(), String> {
    let topology = next.graph.get_node(node).map_err(|e| e.to_string())?;
    let state = next
        .nodes
        .iter()
        .find(|entry| entry.id == node)
        .ok_or("The node does not exist")?;
    validate_node_state(
        state,
        topology.kind(),
        topology.inputs().first().map_or(0, |port| port.channels()),
        topology.channels(),
    )
}

pub(super) fn edit_candidate(next: &mut GraphSnapshot, edit: GraphEdit) -> Result<(), String> {
    match edit {
        GraphEdit::SetExports(exports) => {
            validate_exports(&exports, &next.graph)?;
            next.exports = exports;
        }
        GraphEdit::SetBuiltin { node, state } => {
            let (kind, input_channels, output_channels) = {
                let topology = next
                    .graph
                    .get_node(node)
                    .map_err(|error| error.to_string())?;
                (
                    topology.kind(),
                    topology.inputs().first().map_or(0, |port| port.channels()),
                    topology.channels(),
                )
            };
            state.validate(kind, input_channels, output_channels)?;
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?
                .builtin = state.resolved(kind);
        }
        GraphEdit::SetCompressor {
            node,
            parameter,
            value,
        } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Compressor
            {
                return Err("Compressor parameters only apply to Compressor nodes".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?
                .compressor
                .set(parameter, value);
            validate_edited_node(next, node)?;
        }
        GraphEdit::AddNode { kind, channels } => {
            if next.graph.nodes().len() >= 4096 || channels > 256 {
                return Err(
                    "The graph is limited to 4096 nodes with at most 256 channels each".into(),
                );
            }
            let id = next
                .graph
                .create_node(kind, channels)
                .map_err(|e| e.to_string())?;
            next.nodes.push(GraphNodeState {
                id,
                label: default_label(kind).into(),
                gain: 1.0,
                pan: 0.0,
                mono: false,
                swap: false,
                mixer: super::MixerState::default(),
                compressor: super::CompressorState::default(),
                builtin: super::BuiltinState::for_kind(kind),
                source: None,
                output_endpoint_id: None,
            });
        }
        GraphEdit::SetMixer { node, state } => {
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?
                .mixer = state;
            validate_edited_node(next, node)?;
        }
        GraphEdit::ClearBinding(node) => {
            let entry = next
                .nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?;
            entry.source = None;
            entry.output_endpoint_id = None;
            for export in &mut next.exports {
                if export.binding == Some(node) {
                    export.binding = None;
                }
            }
        }
        GraphEdit::RemoveNode(node) => {
            next.graph.remove_node(node).map_err(|e| e.to_string())?;
            next.nodes.retain(|s| s.id != node);
            for export in &mut next.exports {
                if export.binding == Some(node) {
                    export.binding = None;
                }
            }
        }
        GraphEdit::Connect {
            source,
            destination,
        } => {
            next.graph
                .connect(source, destination, SendParams::default())
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::Disconnect(edge) => {
            next.graph.disconnect(edge).map_err(|e| e.to_string())?;
        }
        GraphEdit::SetSend { edge, params } => {
            next.graph
                .set_send_params(edge, params)
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::AddBusInput(node) => {
            let channels = next
                .graph
                .get_node(node)
                .map_err(|e| e.to_string())?
                .channels();
            next.graph
                .add_input_port(node, channels)
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::RemoveBusInput { node, port } => {
            next.graph
                .remove_input_port(node, port)
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::SetLabel { node, label } => {
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("The node does not exist")?
                .label = label;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetGain { node, gain } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Gain {
                return Err("Only Gain nodes have an adjustable gain".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?
                .gain = gain;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetPan { node, pan } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Pan {
                return Err("Only Balance nodes have an adjustable position".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?
                .pan = pan;
            validate_edited_node(next, node)?;
        }
        GraphEdit::BindSource { node, source } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Source {
                return Err("Only input nodes can bind a source".into());
            }
            for export in &mut next.exports {
                if export.binding == Some(node) {
                    export.binding = None;
                }
            }
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("The node does not exist")?
                .source = Some(source);
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetMono { node, mono } => {
            next.nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("The node does not exist")?
                .mono = mono;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetSwap { node, swap } => {
            next.nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("The node does not exist")?
                .swap = swap;
            validate_edited_node(next, node)?;
        }
        GraphEdit::BindOutput { node, endpoint_id } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Sink {
                return Err("Only output nodes can bind a device".into());
            }
            for export in &mut next.exports {
                if export.binding == Some(node) {
                    export.binding = None;
                }
            }
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("The node does not exist")?
                .output_endpoint_id = Some(endpoint_id);
            validate_edited_node(next, node)?;
        }
        GraphEdit::BindExport { endpoint_id, node } => {
            let (direction, channels) = next
                .exports
                .iter()
                .find(|export| export.endpoint_id == endpoint_id)
                .map(|export| (export.direction, export.channels))
                .ok_or("The virtual export does not exist")?;
            if let Some(node) = node {
                let topology = next.graph.get_node(node).map_err(|e| e.to_string())?;
                if topology.kind() != direction.node_kind() {
                    return Err(match direction {
                        super::super::ExportDirection::Playback => {
                            "A Playback export can only bind an input node".into()
                        }
                        super::super::ExportDirection::Microphone => {
                            "A Microphone export can only bind an output node".into()
                        }
                    });
                }
                if topology.channels() != channels {
                    return Err(
                        "The virtual export and graph node have different channel counts".into(),
                    );
                }
                for other in &mut next.exports {
                    if other.endpoint_id != endpoint_id && other.binding == Some(node) {
                        other.binding = None;
                    }
                }
                let state = next
                    .nodes
                    .iter_mut()
                    .find(|state| state.id == node)
                    .ok_or("The node does not exist")?;
                match direction {
                    super::super::ExportDirection::Playback => state.source = None,
                    super::super::ExportDirection::Microphone => state.output_endpoint_id = None,
                }
            }
            next.exports
                .iter_mut()
                .find(|export| export.endpoint_id == endpoint_id)
                .expect("export validated above")
                .binding = node;
        }
    }
    Ok(())
}
