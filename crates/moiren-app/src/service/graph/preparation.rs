use super::*;
use moiren_engine::{
    compiler::{CompileError, NodeBindings},
    processor::CompressorSettings,
};

impl GraphPreparation {
    pub(super) fn validate_state(&self) -> Result<(), String> {
        self.graph.validate().map_err(|e| e.to_string())?;
        if self.graph.nodes().len() > 4096 || self.nodes.len() != self.graph.nodes().len() {
            return Err("节点状态数量不匹配或图超过 4096 个节点".into());
        }
        for node in self.graph.nodes() {
            if self
                .nodes
                .iter()
                .filter(|state| state.id == node.id())
                .count()
                != 1
            {
                return Err("项目节点状态与拓扑身份不一致".into());
            }
            let state = self
                .nodes
                .iter()
                .find(|state| state.id == node.id())
                .expect("unique state");
            if node.kind() != NodeKind::Pan && (state.mono || state.swap) {
                return Err("Mono 和 Swap 只能用于 Balance 节点".into());
            }
            if state.label.trim().is_empty()
                || state.label.len() > 256
                || state.label.contains('\0')
                || !state.gain.is_finite()
                || !(0.0..=16.0).contains(&state.gain)
                || !state.pan.is_finite()
                || !(-1.0..=1.0).contains(&state.pan)
            {
                return Err("节点名称或参数无效".into());
            }
            if let Some(source) = &state.source {
                if node.kind() != NodeKind::Source {
                    return Err("只有输入节点可以绑定音源".into());
                }
                SessionSpec {
                    source: source.clone(),
                    output_endpoint_id: "validation".into(),
                    limit: RunLimit::UntilStopped,
                    gain: 1.0,
                    pan: 0.0,
                    max_block_frames: 256,
                }
                .validate()
                .map_err(|e| format!("{e:?}"))?;
            }
            if let Some(output) = &state.output_endpoint_id
                && (node.kind() != NodeKind::Sink || output.is_empty() || output.contains('\0'))
            {
                return Err("只有输出节点可以绑定有效设备".into());
            }
        }
        Ok(())
    }
    pub fn io_nodes(&self) -> Result<(NodeId, NodeId), String> {
        self.validate_state()?;
        let sources: Vec<_> = self
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Source)
            .collect();
        let sinks: Vec<_> = self
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Sink)
            .collect();
        if sources.len() != 1 || sinks.len() != 1 {
            return Err("当前音频驱动需要 1 个输入节点和 1 个输出节点；多设备图尚未实现".into());
        }
        if self.graph.nodes().iter().any(|node| node.channels() != 2) {
            return Err("当前设备桥仅支持双声道节点".into());
        }
        Ok((sources[0].id(), sinks[0].id()))
    }
    pub fn gain_pan_nodes(&self) -> Option<(NodeId, NodeId)> {
        Some((
            self.graph
                .nodes()
                .iter()
                .find(|n| n.kind() == NodeKind::Gain)?
                .id(),
            self.graph
                .nodes()
                .iter()
                .find(|n| n.kind() == NodeKind::Pan)?
                .id(),
        ))
    }
    pub(crate) fn bind_processors(
        &self,
        bindings: &mut NodeBindings<f32>,
    ) -> Result<(), CompileError> {
        for node in self.graph.nodes() {
            let state = self.nodes.iter().find(|state| state.id == node.id());
            match node.kind() {
                NodeKind::Gain => bindings.bind_gain(node.id(), state.map_or(1.0, |s| s.gain))?,
                NodeKind::Pan => bindings.bind_pan_with_stereo(
                    node.id(),
                    state.map_or(0.0, |s| s.pan),
                    state.is_some_and(|s| s.mono),
                    state.is_some_and(|s| s.swap),
                )?,
                NodeKind::Compressor => {
                    bindings.bind_compressor(node.id(), CompressorSettings::default())?
                }
                NodeKind::Source | NodeKind::Sink | NodeKind::Bus => {}
            }
        }
        Ok(())
    }
    pub(crate) fn output_meter_port(&self, sink: NodeId) -> Result<PortId, GraphError> {
        // Sink input includes the final edge's send gain/mute and the compiler's
        // dedicated silence slot for an unplugged output.
        Ok(self.graph.get_node(sink)?.inputs()[0].id())
    }
}
