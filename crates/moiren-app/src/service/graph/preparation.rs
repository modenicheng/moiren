use super::*;
use moiren_engine::{
    compiler::{CompileError, NodeBindings},
    processor::{CompressorSettings, DelaySettings, EqSettings, GateSettings, LimiterSettings},
};

impl GraphPreparation {
    /// Translate resource intent for every IO node. The first stable sink owns
    /// the render clock; further sinks follow it through native output bridges.
    pub fn engine_project(
        &self,
        max_block_frames: usize,
    ) -> Result<moiren_engine::host::EngineProject, String> {
        use moiren_core::protocol::ParamValue;
        use moiren_engine::{
            boundary::{CaptureIntent, ClockRole},
            host::{DesiredParameter, EngineProject, NodeConfig, ParameterTarget},
            node::{
                DeviceOptions, EndpointSelector, InputConfig, InputSource, OutputConfig,
                OutputTarget,
            },
            processor::{DelaySettings, EqSettings, Gain, GateSettings, LimiterSettings, Pan},
        };
        self.primary_io_nodes()?;
        let mut project = EngineProject {
            graph: self.graph.clone(),
            ..EngineProject::default()
        };
        project.processing.engine.max_block_frames = max_block_frames;
        project.strips.extend(self.mixer_configs());
        let master = self
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Sink)
            .map(|n| n.id())
            .min();
        let mut endpoints = std::collections::BTreeSet::new();
        let mut inputs = std::collections::BTreeSet::new();
        let pinned = |endpoint_id: &String| EndpointSelector::PinnedEndpoint {
            endpoint_id: endpoint_id.clone(),
            stable_id: None,
        };
        for node in self.graph.nodes() {
            let state = self
                .nodes
                .iter()
                .find(|s| s.id == node.id())
                .expect("validated node state");
            let config = match node.kind() {
                NodeKind::Source => {
                    let source = match state.source.as_ref().ok_or("输入节点尚未绑定音源")?
                    {
                        InputSelection::Physical { endpoint_id } => {
                            if !inputs.insert(format!("physical:{endpoint_id}")) {
                                return Err("多个输入节点绑定了同一物理设备".into());
                            }
                            InputSource::Physical(pinned(endpoint_id))
                        }
                        InputSelection::Process {
                            pid,
                            creation_time_100ns,
                        } => {
                            if *creation_time_100ns == 0 {
                                return Err("应用音源缺少稳定进程身份".into());
                            }
                            let executable = format!("moiren:process:{pid}:{creation_time_100ns}");
                            if !inputs.insert(executable.clone()) {
                                return Err("多个输入节点绑定了同一应用音源".into());
                            }
                            InputSource::Application {
                                executable,
                                intent: CaptureIntent::PassiveTap,
                            }
                        }
                        InputSelection::Tone { frequency_hz } => InputSource::Software {
                            name: format!("moiren:tone:{frequency_hz}"),
                        },
                    };
                    NodeConfig::Input(InputConfig {
                        source,
                        channels: node.channels(),
                        device: DeviceOptions::default(),
                    })
                }
                NodeKind::Sink => {
                    let endpoint = state
                        .output_endpoint_id
                        .as_ref()
                        .ok_or("输出节点尚未绑定设备")?;
                    if !endpoints.insert(endpoint.clone()) {
                        return Err("多个输出节点绑定了同一设备".into());
                    }
                    NodeConfig::Output(OutputConfig {
                        target: OutputTarget::Physical(pinned(endpoint)),
                        channels: node.channels(),
                        device: DeviceOptions {
                            clock_role: if Some(node.id()) == master {
                                ClockRole::Master
                            } else {
                                ClockRole::Follower
                            },
                            ..DeviceOptions::default()
                        },
                    })
                }
                NodeKind::Gain => NodeConfig::Gain { initial: 1.0 },
                NodeKind::Pan => NodeConfig::Pan { initial: 0.0 },
                NodeKind::Compressor => NodeConfig::Compressor(CompressorSettings::default()),
                NodeKind::Eq => NodeConfig::Eq(EqSettings::default()),
                NodeKind::Gate => NodeConfig::Gate(GateSettings::default()),
                NodeKind::Limiter => NodeConfig::Limiter(LimiterSettings::default()),
                NodeKind::Delay => NodeConfig::Delay(DelaySettings::default()),
                NodeKind::ChannelMapper => NodeConfig::ChannelMapper,
                NodeKind::Recorder => NodeConfig::Recorder,
                NodeKind::Meter => NodeConfig::Meter,
                NodeKind::SpectrumAnalyzer => NodeConfig::SpectrumAnalyzer,
                NodeKind::Rack => NodeConfig::Rack,
                NodeKind::Bus => NodeConfig::Bus,
            };
            project.nodes.insert(node.id(), config);
            let values = match node.kind() {
                NodeKind::Gain => vec![(Gain::LEVEL, ParamValue::Float(state.gain))],
                NodeKind::Pan => vec![
                    (Pan::POSITION, ParamValue::Float(state.pan)),
                    (Pan::MONO, ParamValue::Bool(state.mono)),
                    (Pan::SWAP, ParamValue::Bool(state.swap)),
                ],
                NodeKind::Compressor => CompressorParameter::ALL
                    .into_iter()
                    .map(|parameter| parameter.desired(state.compressor))
                    .collect(),
                _ => Vec::new(),
            };
            project
                .parameters
                .extend(
                    values
                        .into_iter()
                        .map(|(parameter, value)| DesiredParameter {
                            target: ParameterTarget::Node {
                                node: node.id(),
                                parameter,
                            },
                            value,
                        }),
                );
        }
        project.validate().map_err(|e| e.to_string())?;
        Ok(project)
    }
    pub fn primary_io_nodes(&self) -> Result<(NodeId, NodeId), String> {
        self.validate_state()?;
        if self.graph.nodes().iter().any(|node| node.channels() != 2) {
            return Err("当前设备桥仅支持双声道节点".into());
        }
        let source = self
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Source)
            .map(|n| n.id())
            .min()
            .ok_or("图需要输入节点")?;
        let sink = self
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Sink)
            .map(|n| n.id())
            .min()
            .ok_or("图需要输出节点")?;
        Ok((source, sink))
    }
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
            if !state.compressor.valid()
                || !state.mixer.valid(node.channels())
                || state.label.trim().is_empty()
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
        for (node, config) in self.mixer_configs() {
            bindings.bind_strip(node, config);
        }
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
                NodeKind::Compressor => bindings.bind_compressor(
                    node.id(),
                    state.map_or_else(CompressorSettings::default, |state| {
                        state.compressor.settings()
                    }),
                )?,
                NodeKind::Eq => bindings.bind_eq(node.id(), EqSettings::default())?,
                NodeKind::Gate => bindings.bind_gate(node.id(), GateSettings::default())?,
                NodeKind::Limiter => {
                    bindings.bind_limiter(node.id(), LimiterSettings::default())?
                }
                NodeKind::Delay => bindings.bind_delay(node.id(), DelaySettings::default())?,
                NodeKind::ChannelMapper
                | NodeKind::Recorder
                | NodeKind::Meter
                | NodeKind::SpectrumAnalyzer
                | NodeKind::Rack => bindings.bind_utility(node.id())?,
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
