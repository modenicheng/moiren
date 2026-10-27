use super::*;
use moiren_engine::compiler::{CompileError, NodeBindings};

impl GraphPreparation {
    /// Translate resource intent for every IO node. The first stable sink owns
    /// the render clock; further sinks follow it through native output bridges.
    pub fn engine_project(
        &self,
        max_block_frames: usize,
    ) -> Result<moiren_engine::host::EngineProject, String> {
        self.configured_engine_project(max_block_frames, &crate::service::AudioSettings::default())
    }

    pub fn configured_engine_project(
        &self,
        max_block_frames: usize,
        settings: &crate::service::AudioSettings,
    ) -> Result<moiren_engine::host::EngineProject, String> {
        use moiren_core::protocol::ParamValue;
        use moiren_engine::{
            boundary::{CaptureIntent, ClockRole},
            host::{DesiredParameter, EngineProject, NodeConfig, ParameterTarget},
            node::{
                DeviceOptions, EndpointSelector, InputConfig, InputSource, OutputConfig,
                OutputTarget,
            },
            processor::{Gain, Pan},
        };
        settings.validate().map_err(|error| error.to_string())?;
        // The legacy default backend negotiates native Shared mix rates and
        // adapts capture to 48 kHz itself. Keep its unspecified hardware intent;
        // every professional configuration resolves explicit processing intent.
        let legacy_defaults = settings == &crate::service::AudioSettings::default();
        self.primary_io_nodes()?;
        let mut project = EngineProject {
            graph: self.graph.clone(),
            ..EngineProject::default()
        };
        project.processing.engine.processing_sr = f64::from(settings.processing.sample_rate);
        project.processing.engine.max_block_frames = max_block_frames;
        project.processing.control_horizon_frames = u64::from(settings.processing.sample_rate);
        project.strips.extend(self.mixer_configs());
        // Only a physical render endpoint can own the native device clock.
        // An all-virtual project is paced by the single Engine realtime owner.
        let master = self
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Sink)
            .filter(|n| {
                !self
                    .exports
                    .iter()
                    .any(|export| export.enabled && export.binding == Some(n.id()))
            })
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
                NodeKind::Source if self.enabled_export(node.id()).is_some() => {
                    let export = self.enabled_export(node.id()).expect("matched export");
                    NodeConfig::Input(InputConfig {
                        source: InputSource::VirtualPlayback {
                            endpoint_id: export.endpoint_id,
                        },
                        channels: export.channels,
                        device: DeviceOptions::default(),
                    })
                }
                NodeKind::Sink if self.enabled_export(node.id()).is_some() => {
                    let export = self.enabled_export(node.id()).expect("matched export");
                    NodeConfig::Output(OutputConfig {
                        target: OutputTarget::VirtualMicrophone {
                            endpoint_id: export.endpoint_id,
                        },
                        channels: export.channels,
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
                NodeKind::Source => {
                    let mut device = DeviceOptions::default();
                    let source = match state
                        .source
                        .as_ref()
                        .ok_or("The Source node is not bound to an input source")?
                    {
                        InputSelection::Physical { endpoint_id } => {
                            if !inputs.insert(format!("physical:{endpoint_id}")) {
                                return Err(
                                    "Multiple Source nodes are bound to the same physical device"
                                        .into(),
                                );
                            }
                            if !legacy_defaults {
                                device = settings.effective_input(endpoint_id).device_options(
                                    settings.backend,
                                    settings.processing.sample_rate,
                                    ClockRole::Follower,
                                );
                            }
                            InputSource::Physical(pinned(endpoint_id))
                        }
                        InputSelection::Process {
                            pid,
                            creation_time_100ns,
                        } => {
                            if *creation_time_100ns == 0 {
                                return Err(
                                    "The application source is missing a stable process identity"
                                        .into(),
                                );
                            }
                            let executable = format!("moiren:process:{pid}:{creation_time_100ns}");
                            if !inputs.insert(executable.clone()) {
                                return Err("Multiple Source nodes are bound to the same application source".into());
                            }
                            // Process loopback owns a fixed WASAPI Shared float
                            // stream. Physical input defaults cannot turn it
                            // into an exclusive or ASIO capture endpoint.
                            device.hardware_sample_rate = Some(48_000);
                            device.sample_format = crate::service::SampleFormat::Float32;
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
                        device,
                    })
                }
                NodeKind::Sink => {
                    let endpoint = state
                        .output_endpoint_id
                        .as_ref()
                        .ok_or("The Sink node is not bound to an output device")?;
                    if !endpoints.insert(endpoint.clone()) {
                        return Err("Multiple Sink nodes are bound to the same device".into());
                    }
                    let clock_role = if Some(node.id()) == master {
                        ClockRole::Master
                    } else {
                        ClockRole::Follower
                    };
                    NodeConfig::Output(OutputConfig {
                        target: OutputTarget::Physical(pinned(endpoint)),
                        channels: node.channels(),
                        device: if legacy_defaults {
                            DeviceOptions {
                                clock_role,
                                ..DeviceOptions::default()
                            }
                        } else {
                            settings.effective_output(endpoint).device_options(
                                settings.backend,
                                settings.processing.sample_rate,
                                clock_role,
                            )
                        },
                    })
                }
                NodeKind::Gain => NodeConfig::Gain { initial: 1.0 },
                NodeKind::Pan => NodeConfig::Pan { initial: 0.0 },
                // Compressor values are live parameters. Keep the prepared
                // processor's static shape at its default so editing a
                // compressor field does not force a topology rebuild.
                NodeKind::Compressor => NodeConfig::Compressor(Default::default()),
                NodeKind::Eq
                | NodeKind::Gate
                | NodeKind::Limiter
                | NodeKind::Delay
                | NodeKind::ChannelMapper
                | NodeKind::Recorder
                | NodeKind::Meter
                | NodeKind::SpectrumAnalyzer
                | NodeKind::Rack => state
                    .builtin
                    .node_config(node.kind())?
                    .ok_or_else(|| format!("missing settings for {:?} node", node.kind()))?,
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
        let source = primary_node_of(&self.graph, NodeKind::Source)
            .map(|n| n.id())
            .ok_or("The graph needs a Source node")?;
        let sink = primary_node_of(&self.graph, NodeKind::Sink)
            .map(|n| n.id())
            .ok_or("The graph needs a Sink node")?;
        Ok((source, sink))
    }
    pub(super) fn validate_state(&self) -> Result<(), String> {
        self.graph.validate().map_err(|e| e.to_string())?;
        validate_exports(&self.exports, &self.graph)?;
        if self.graph.nodes().len() > 4096 || self.nodes.len() != self.graph.nodes().len() {
            return Err(
                "Node state count does not match the graph, or the graph exceeds 4096 nodes".into(),
            );
        }
        for node in self.graph.nodes() {
            if self
                .nodes
                .iter()
                .filter(|state| state.id == node.id())
                .count()
                != 1
            {
                return Err("Project node states do not match the topology identity".into());
            }
            let state = self
                .nodes
                .iter()
                .find(|state| state.id == node.id())
                .expect("unique state");
            validate_node_state(
                state,
                node.kind(),
                node.inputs().first().map_or(0, |port| port.channels()),
                node.channels(),
            )?;
        }
        Ok(())
    }
    fn enabled_export(&self, node: NodeId) -> Option<&ExportIntent> {
        self.exports
            .iter()
            .find(|export| export.enabled && export.binding == Some(node))
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
            return Err("The current audio backend needs exactly one Source and one Sink node; multi-device graphs are not supported yet".into());
        }
        if self.graph.get_node(sources[0].id()).unwrap().channels() != 2
            || self.graph.get_node(sinks[0].id()).unwrap().channels() != 2
        {
            return Err("The current device bridge only supports stereo nodes".into());
        }
        Ok((sources[0].id(), sinks[0].id()))
    }
    pub fn gain_pan_nodes(&self) -> Option<(NodeId, NodeId)> {
        Some((
            primary_node_of(&self.graph, NodeKind::Gain)?.id(),
            primary_node_of(&self.graph, NodeKind::Pan)?.id(),
        ))
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn bind_processors(
        &self,
        bindings: &mut NodeBindings<f32>,
    ) -> Result<(), CompileError> {
        // This legacy route cannot return a recorder reader or prepare a typed
        // spectrum publisher. Reject before consuming any existing bindings.
        for node in self.graph.nodes() {
            match node.kind() {
                NodeKind::Recorder => return Err(CompileError::BindingKind { node: node.id() }),
                NodeKind::Meter => {}
                NodeKind::SpectrumAnalyzer => return Err(CompileError::SignalsRequireHost),
                _ => {}
            }
            let state = self
                .nodes
                .iter()
                .find(|state| state.id == node.id())
                .ok_or(CompileError::BindingKind { node: node.id() })?;
            state
                .builtin
                .validate(
                    node.kind(),
                    node.inputs().first().map_or(0, |port| port.channels()),
                    node.channels(),
                )
                .map_err(|_| CompileError::BindingKind { node: node.id() })?;
        }
        for (node, config) in self.mixer_configs() {
            bindings.bind_strip(node, config);
        }
        for node in self.graph.nodes() {
            let state = self
                .nodes
                .iter()
                .find(|state| state.id == node.id())
                .expect("validated node state");
            match node.kind() {
                NodeKind::Gain => bindings.bind_gain(node.id(), state.gain)?,
                NodeKind::Pan => {
                    bindings.bind_pan_with_stereo(node.id(), state.pan, state.mono, state.swap)?
                }
                NodeKind::Compressor => {
                    bindings.bind_compressor(node.id(), state.compressor.settings())?
                }
                _ => match state.builtin.resolved(node.kind()) {
                    BuiltinState::Eq(settings) => bindings.bind_eq(node.id(), settings)?,
                    BuiltinState::Gate(settings) => bindings.bind_gate(node.id(), settings)?,
                    BuiltinState::Limiter(settings) => {
                        bindings.bind_limiter(node.id(), settings)?
                    }
                    BuiltinState::Delay(settings) => bindings.bind_delay(node.id(), settings)?,
                    BuiltinState::ChannelMapper(settings) => {
                        bindings.bind_channel_mapper(node.id(), settings)?;
                    }
                    BuiltinState::Rack(settings) => bindings.bind_rack(node.id(), settings)?,
                    BuiltinState::Meter(settings) => {
                        let _reader = bindings.bind_meter(node.id(), settings)?;
                    }
                    BuiltinState::None => {}
                    BuiltinState::Recorder(_) | BuiltinState::SpectrumAnalyzer(_) => {
                        unreachable!("managed utilities rejected before binding")
                    }
                },
            }
        }
        Ok(())
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn output_meter_port(&self, sink: NodeId) -> Result<PortId, GraphError> {
        // Sink input includes the final edge's send gain/mute and the compiler's
        // dedicated silence slot for an unplugged output.
        Ok(self.graph.get_node(sink)?.inputs()[0].id())
    }
}
