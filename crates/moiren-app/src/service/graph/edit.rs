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
    validate_node_topology(state, topology)
}

pub(super) fn edit_candidate(next: &mut GraphSnapshot, edit: GraphEdit) -> Result<(), String> {
    match edit {
        GraphEdit::SetExports(exports) => {
            validate_exports(&exports, &next.graph)?;
            next.exports = exports;
        }
        GraphEdit::SetBuiltin { node, state } => {
            let topology = next
                .graph
                .get_node(node)
                .map_err(|error| error.to_string())?;
            state.validate_for_node(topology)?;
            let kind = topology.kind();
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?
                .builtin = state.resolved(kind);
        }
        GraphEdit::SetMapperWeight {
            node,
            input,
            output,
            weight,
        } => {
            field_edit::set_mapper_weight(next, node, input, output, weight)?;
            validate_edited_node(next, node)?;
        }
        GraphEdit::MapperPreset { node, swap } => {
            field_edit::mapper_preset(next, node, swap)?;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetBuiltinParameter {
            node,
            parameter,
            value,
        } => {
            field_edit::set_builtin_parameter(next, node, parameter, value)?;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetEqBand {
            node,
            frequency_hz,
            gain_db,
        } => {
            field_edit::set_eq_band(next, node, frequency_hz, gain_db)?;
            validate_edited_node(next, node)?;
        }
        GraphEdit::AdjustEqQ { node, factor } => {
            field_edit::adjust_eq_q(next, node, factor)?;
            validate_edited_node(next, node)?;
        }
        GraphEdit::SetSpectrumFft { node, fft_size } => {
            edit_spectrum(next, node, |settings| {
                settings.fft_size = fft_size;
                settings.hop_frames = settings.hop_frames.min(fft_size);
            })?;
        }
        GraphEdit::SetSpectrumHop { node, hop_frames } => {
            edit_spectrum(next, node, |settings| settings.hop_frames = hop_frames)?;
        }
        GraphEdit::EditRack { node, epoch, edit } => {
            let topology = next
                .graph
                .get_node(node)
                .map_err(|error| error.to_string())?;
            if topology.kind() != NodeKind::Rack {
                return Err("Rack edits only apply to Rack nodes".into());
            }
            if next.rack_epochs.get(&node) != Some(&epoch) {
                return Err("The Rack editor is stale; refresh it before editing".into());
            }
            let inputs: Vec<_> = topology
                .inputs()
                .iter()
                .map(|port| port.channels())
                .collect();
            let outputs: Vec<_> = topology
                .outputs()
                .iter()
                .map(|port| port.channels())
                .collect();
            let state = next
                .nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("The node does not exist")?;
            let BuiltinState::Rack(settings) = state.builtin.resolved(NodeKind::Rack) else {
                return Err("Rack settings are missing".into());
            };
            state.builtin = BuiltinState::Rack(edit.preview(&settings, &inputs, &outputs)?);
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
        GraphEdit::EditMixer { node, edit } => {
            let channels = next
                .graph
                .get_node(node)
                .map_err(|e| e.to_string())?
                .channels();
            let state = next
                .nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("The node does not exist")?;
            edit.apply(&mut state.mixer, channels)?;
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
        GraphEdit::ToggleMono { node } | GraphEdit::ToggleSwap { node } => {
            if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Pan {
                return Err("Mono and Swap only apply to Balance nodes".into());
            }
            let state = next
                .nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("The node does not exist")?;
            if matches!(edit, GraphEdit::ToggleMono { .. }) {
                state.mono = !state.mono;
            } else {
                state.swap = !state.swap;
            }
            validate_edited_node(next, node)?;
        }
        GraphEdit::BindOutput { node, endpoint_id } => {
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("The node does not exist")?
                .output_endpoint_id = Some(endpoint_id);
            validate_edited_node(next, node)?;
        }
    }
    Ok(())
}

fn edit_spectrum(
    next: &mut GraphSnapshot,
    node: NodeId,
    edit: impl FnOnce(&mut moiren_engine::spectrum::SpectrumSettings),
) -> Result<(), String> {
    if next.graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::SpectrumAnalyzer {
        return Err("Spectrum settings only apply to Spectrum Analyzer nodes".into());
    }
    let state = next
        .nodes
        .iter_mut()
        .find(|state| state.id == node)
        .ok_or("The node does not exist")?;
    let BuiltinState::SpectrumAnalyzer(mut settings) =
        state.builtin.resolved(NodeKind::SpectrumAnalyzer)
    else {
        return Err("Spectrum Analyzer settings are missing".into());
    };
    edit(&mut settings);
    state.builtin = BuiltinState::SpectrumAnalyzer(settings);
    validate_edited_node(next, node)
}
