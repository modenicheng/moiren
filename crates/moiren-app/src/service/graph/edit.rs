use super::*;

pub(super) fn edit_candidate(next: &mut GraphSnapshot, edit: GraphEdit) -> Result<(), String> {
    let graph = &mut next.graph;
    match edit {
        GraphEdit::SetBuiltin { node, state } => {
            let (kind, input_channels, output_channels) = {
                let topology = graph.get_node(node).map_err(|error| error.to_string())?;
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
            if graph
                .get_node(node)
                .map_err(|error| error.to_string())?
                .kind()
                != NodeKind::Compressor
                || !parameter.accepts(value)
            {
                return Err("Compressor 参数超出有效范围或节点类型不匹配".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("节点不存在")?
                .compressor
                .set(parameter, value);
        }
        GraphEdit::AddNode { kind, channels } => {
            if graph.nodes().len() >= 4096 || channels > 256 {
                return Err("图最多包含 4096 个节点，每节点最多 256 声道".into());
            }
            let id = graph
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
            if !state.valid(
                graph
                    .get_node(node)
                    .map_err(|error| error.to_string())?
                    .channels(),
            ) {
                return Err("Mixer gain or channel mode is invalid".into());
            }
            next.nodes
                .iter_mut()
                .find(|entry| entry.id == node)
                .ok_or("Node missing")?
                .mixer = state;
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
            graph.remove_node(node).map_err(|e| e.to_string())?;
            next.nodes.retain(|s| s.id != node);
        }
        GraphEdit::Connect {
            source,
            destination,
        } => {
            graph
                .connect(source, destination, SendParams::default())
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::Disconnect(edge) => {
            graph.disconnect(edge).map_err(|e| e.to_string())?;
        }
        GraphEdit::SetSend { edge, params } => {
            graph
                .set_send_params(edge, params)
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::AddBusInput(node) => {
            let channels = graph.get_node(node).map_err(|e| e.to_string())?.channels();
            graph
                .add_input_port(node, channels)
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::RemoveBusInput { node, port } => {
            graph
                .remove_input_port(node, port)
                .map_err(|e| e.to_string())?;
        }
        GraphEdit::SetLabel { node, label } => {
            if label.trim().is_empty() || label.len() > 256 || label.contains('\0') {
                return Err("节点名称不能为空且最多 256 字节".into());
            }
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("节点不存在")?
                .label = label;
        }
        GraphEdit::SetGain { node, gain } => {
            if graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Gain
                || !gain.is_finite()
                || !(0.0..=16.0).contains(&gain)
            {
                return Err("Gain 节点增益必须为 0..16 的有限值".into());
            }
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("节点不存在")?
                .gain = gain;
        }
        GraphEdit::SetPan { node, pan } => {
            if graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Pan
                || !pan.is_finite()
                || !(-1.0..=1.0).contains(&pan)
            {
                return Err("Balance 节点位置必须为 -1..1 的有限值".into());
            }
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("节点不存在")?
                .pan = pan;
        }
        GraphEdit::BindSource { node, source } => {
            if graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Source {
                return Err("只能为输入节点选择资源".into());
            }
            let spec = SessionSpec {
                source: source.clone(),
                output_endpoint_id: "validation".into(),
                limit: RunLimit::UntilStopped,
                gain: 1.0,
                pan: 0.0,
                max_block_frames: 256,
            };
            spec.validate().map_err(|e| format!("{e:?}"))?;
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("节点不存在")?
                .source = Some(source);
        }
        GraphEdit::SetMono { node, mono } => {
            if graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Pan {
                return Err("Mono 只能用于 Balance 节点".into());
            }
            next.nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("节点不存在")?
                .mono = mono;
        }
        GraphEdit::SetSwap { node, swap } => {
            if graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Pan {
                return Err("Swap 只能用于 Balance 节点".into());
            }
            next.nodes
                .iter_mut()
                .find(|state| state.id == node)
                .ok_or("节点不存在")?
                .swap = swap;
        }
        GraphEdit::BindOutput { node, endpoint_id } => {
            if graph.get_node(node).map_err(|e| e.to_string())?.kind() != NodeKind::Sink
                || endpoint_id.is_empty()
                || endpoint_id.contains('\0')
            {
                return Err("请选择有效输出设备".into());
            }
            next.nodes
                .iter_mut()
                .find(|s| s.id == node)
                .ok_or("节点不存在")?
                .output_endpoint_id = Some(endpoint_id);
        }
    }
    Ok(())
}
