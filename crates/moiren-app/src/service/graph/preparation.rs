use super::*;

/// Why a project could not be prepared, and the node it points at — `None`
/// when the failure is genuinely graph-wide (settings, exports, topology
/// shape). Callers record this as a fault so a node-aimed cause lands on
/// that node's card and strip instead of the status bar.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparationFailure {
    pub node: Option<NodeId>,
    pub cause: String,
}

impl PreparationFailure {
    fn graph_wide(cause: impl Into<String>) -> Self {
        Self {
            node: None,
            cause: cause.into(),
        }
    }

    fn at(node: NodeId, cause: impl Into<String>) -> Self {
        Self {
            node: Some(node),
            cause: cause.into(),
        }
    }
}

/// The sink node that owns the native device clock: the lowest-id physical
/// render endpoint not bound to an enabled export. `None` leaves the project
/// paced by the single Engine realtime owner. Preparation and the status
/// bar's read-only clock panel share this one policy.
pub fn clock_master(
    graph: &moiren_core::graph::LogicalGraph,
    exports: &[crate::service::ExportIntent],
) -> Option<NodeId> {
    graph
        .nodes()
        .iter()
        .filter(|n| n.kind() == NodeKind::Sink)
        .filter(|n| {
            !exports
                .iter()
                .any(|export| export.enabled && export.binding == Some(n.id()))
        })
        .map(|n| n.id())
        .min()
}

impl GraphPreparation {
    /// Default-settings translation used only by the tests; production always
    /// carries the effective audio settings through
    /// [`Self::configured_engine_project`].
    #[cfg(test)]
    pub fn engine_project(
        &self,
        max_block_frames: usize,
    ) -> Result<moiren_engine::host::EngineProject, String> {
        self.configured_engine_project(
            max_block_frames,
            &crate::service::AudioSettings::default(),
            crate::service::AudioLatencyProfile::default(),
        )
        .map_err(|failure| failure.cause)
    }

    pub fn configured_engine_project(
        &self,
        max_block_frames: usize,
        settings: &crate::service::AudioSettings,
        profile: crate::service::AudioLatencyProfile,
    ) -> Result<moiren_engine::host::EngineProject, PreparationFailure> {
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
        settings
            .validate()
            .map_err(|error| PreparationFailure::graph_wide(error.to_string()))?;
        let profile_buffer = profile
            .requested_buffer_frames()
            .map(|frames| frames as usize);
        self.validate_state()
            .map_err(PreparationFailure::graph_wide)?;
        let mut project = EngineProject {
            graph: self.graph.clone(),
            ..EngineProject::default()
        };
        project.processing.engine.processing_sr = f64::from(settings.processing.sample_rate);
        project.processing.engine.max_block_frames = max_block_frames;
        project.processing.control_horizon_frames = u64::from(settings.processing.sample_rate);
        project.strips.extend(self.mixer_configs());
        let master = clock_master(&self.graph, &self.exports);
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
                    let source = match state.source.as_ref().ok_or_else(|| {
                        PreparationFailure::at(
                            node.id(),
                            "The Source node is not bound to an input source",
                        )
                    })? {
                        InputSelection::Physical { endpoint_id } => {
                            if !inputs.insert(format!("physical:{endpoint_id}")) {
                                return Err(PreparationFailure::at(
                                    node.id(),
                                    "Multiple Source nodes are bound to the same physical device",
                                ));
                            }
                            device = settings.effective_input(endpoint_id).device_options(
                                settings.backend,
                                settings.processing.sample_rate,
                                ClockRole::Follower,
                            );
                            if device.period_frames.is_none()
                                && let Some(frames) = profile_buffer
                            {
                                device.period_frames = Some(frames);
                            }
                            InputSource::Physical(pinned(endpoint_id))
                        }
                        InputSelection::Process {
                            pid,
                            creation_time_100ns,
                        } => {
                            if *creation_time_100ns == 0 {
                                return Err(PreparationFailure::at(
                                    node.id(),
                                    "The application source is missing a stable process identity",
                                ));
                            }
                            let executable = format!("moiren:process:{pid}:{creation_time_100ns}");
                            if !inputs.insert(executable.clone()) {
                                return Err(PreparationFailure::at(
                                    node.id(),
                                    "Multiple Source nodes are bound to the same application source",
                                ));
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
                    let endpoint = state.output_endpoint_id.as_ref().ok_or_else(|| {
                        PreparationFailure::at(
                            node.id(),
                            "The Sink node is not bound to an output device",
                        )
                    })?;
                    if !endpoints.insert(endpoint.clone()) {
                        return Err(PreparationFailure::at(
                            node.id(),
                            "Multiple Sink nodes are bound to the same device",
                        ));
                    }
                    let clock_role = if Some(node.id()) == master {
                        ClockRole::Master
                    } else {
                        ClockRole::Follower
                    };
                    let mut device = settings.effective_output(endpoint).device_options(
                        settings.backend,
                        settings.processing.sample_rate,
                        clock_role,
                    );
                    if device.period_frames.is_none()
                        && let Some(frames) = profile_buffer
                    {
                        device.period_frames = Some(frames);
                    }
                    NodeConfig::Output(OutputConfig {
                        target: OutputTarget::Physical(pinned(endpoint)),
                        channels: node.channels(),
                        device,
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
                    .node_config(node.kind())
                    .map_err(|error| PreparationFailure::at(node.id(), error))?
                    .ok_or_else(|| {
                        PreparationFailure::at(
                            node.id(),
                            format!("missing settings for {:?} node", node.kind()),
                        )
                    })?,
                NodeKind::Bus => NodeConfig::Bus,
                NodeKind::Plugin => {
                    // The assignment resolves against the service's current
                    // plugin library: preparation instantiates through the
                    // existing moiren-plugins host (engine-side
                    // NodeConfig::Plugin lowering), never a new one.
                    let Some(assignment) = &state.plugin else {
                        return Err(PreparationFailure::at(
                            node.id(),
                            "The Plugin node has no plugin assigned",
                        ));
                    };
                    let Some(entry) = self.plugins.get(&assignment.id) else {
                        return Err(PreparationFailure::at(
                            node.id(),
                            "The assigned plugin is missing from the library; scan plugins",
                        ));
                    };
                    if entry.descriptor.channels() as usize != node.channels() {
                        return Err(PreparationFailure::at(
                            node.id(),
                            format!(
                                "The plugin needs {} channels but the node has {}",
                                entry.descriptor.channels(),
                                node.channels()
                            ),
                        ));
                    }
                    NodeConfig::Plugin(moiren_engine::processor::PluginSettings {
                        descriptor: entry.descriptor.clone(),
                        parameters: entry.parameters.clone(),
                        preset: assignment.preset.clone(),
                    })
                }
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
        project
            .validate()
            .map_err(|e| PreparationFailure::graph_wide(e.to_string()))?;
        Ok(project)
    }
    /// The primary Source/Sink nodes when present. An output-free or
    /// input-free graph is a valid project: absence is information, not an
    /// error, so callers decide what a missing side means for them.
    pub fn primary_io_nodes(&self) -> Result<(Option<NodeId>, Option<NodeId>), String> {
        self.validate_state()?;
        let source = primary_node_of(&self.graph, NodeKind::Source).map(|n| n.id());
        let sink = primary_node_of(&self.graph, NodeKind::Sink).map(|n| n.id());
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
    pub fn gain_pan_nodes(&self) -> Option<(NodeId, NodeId)> {
        Some((
            primary_node_of(&self.graph, NodeKind::Gain)?.id(),
            primary_node_of(&self.graph, NodeKind::Pan)?.id(),
        ))
    }
}
