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
        .ok_or("节点不存在")?;
    validate_node_state(
        state,
        topology.kind(),
        topology.inputs().first().map_or(0, |port| port.channels()),
        topology.channels(),
    )
}

pub(super) fn edit_candidate(next: &mut GraphSnapshot, edit: GraphEdit) -> Result<(), String> {
    match edit {
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
                .ok_or("Node missing")?
                .builtin = state.resolved(kind);
        }
        GraphEdit::SetCompressor {
            node,
            parameter,
            value,
        } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Compressor
            {
                return Err("Compressor 参数只能用于 Compressor 节点".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("节点不存在")?
                .compressor
                .set(parameter, value);
            validate_edited_node(next, node)?;
        }
        GraphEdit::AddNode { kind, channels } => {
            if next.graph.nodes().len() >= 4096 || channels > 256 {
                return Err("图最多包含 4096 个节点，每节点最多 256 声道".into());
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
                .ok_or("Node missing")?
                .mixer = state;
            validate_edited_node(next, node)?;
        }
        GraphEdit::ClearBinding(node) => {
            let entry = next
                .nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("Node missing")?;
            entry.source = None;
            entry.output_endpoint_id = None;
        }
        GraphEdit::RemoveNode(node) => {
            next.graph.remove_node(node).map_err(|e| e.to_string())?;
            next.nodes.retain(|s| s.id != node);
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
                .ok_or("节点不存在")?
                .label = label;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetGain { node, gain } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Gain {
                return Err("只能调整 Gain 节点的增益".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("节点不存在")?
                .gain = gain;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetPan { node, pan } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Pan {
                return Err("只能调整 Balance 节点的位置".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("节点不存在")?
                .pan = pan;
            validate_edited_node(next, node)?;
        }
        GraphEdit::BindSource { node, source } => {
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("节点不存在")?
                .source = Some(source);
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetMono { node, mono } => {
            next.nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("节点不存在")?
                .mono = mono;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetSwap { node, swap } => {
            next.nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("节点不存在")?
                .swap = swap;
            validate_edited_node(next, node)?;
        }
        GraphEdit::BindOutput { node, endpoint_id } => {
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("节点不存在")?
                .output_endpoint_id = Some(endpoint_id);
            validate_edited_node(next, node)?;
        }
    }
    Ok(())
}
