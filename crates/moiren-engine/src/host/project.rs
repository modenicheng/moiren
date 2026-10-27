use std::collections::BTreeMap;

use moiren_core::{
    graph::{EdgeId, GraphError, LogicalGraph, NodeId, NodeKind},
    protocol::{ParamValue, ParameterId, ProcessorId},
};
use thiserror::Error;

use crate::{
    compiler::CompileConfig,
    control::{ParamSpec, accepts_parameter, specs_accept_initials},
    node::{InputConfig, IoConfigError, OutputConfig},
    processor::{
        ChannelMapperSettings, Compressor, CompressorSettings, Delay, DelaySettings,
        Eq as EqProcessor, EqSettings, Gate, GateSettings, Limiter, LimiterSettings,
        builtin::MAX_CHANNELS,
    },
    runtime::EngineConfig,
};

#[derive(Debug, Clone, PartialEq)]
pub enum NodeConfig {
    Input(InputConfig),
    Output(OutputConfig),
    Gain { initial: f64 },
    Pan { initial: f64 },
    Compressor(CompressorSettings),
    Eq(EqSettings),
    Gate(GateSettings),
    Limiter(LimiterSettings),
    Delay(DelaySettings),
    ChannelMapper(ChannelMapperSettings),
    Recorder(crate::recorder::RecorderSettings),
    Meter(crate::meter::MeterSettings),
    SpectrumAnalyzer(crate::spectrum::SpectrumSettings),
    Rack(crate::rack::RackSettings),
    Bus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendParameter {
    Gain,
    Pan,
    Mute,
}

/// Live controls of a node's channel strip. `Mode` is an index into the
/// strip's mode cycle (0 = Normal, 1 = Mono, `2..width+2` = channel-only,
/// `width+2` = Swap) — the same index the app stores per node, interpreted
/// against each port's width exactly like a compiled `StripConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripParameter {
    Gain,
    Balance,
    Mode,
    EffectiveMute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterTarget {
    Node {
        node: NodeId,
        parameter: ParameterId,
    },
    /// Logical slot identity; runtime ProcessorIds are resolved per active plan.
    RackSlot {
        node: NodeId,
        slot: u32,
        parameter: ParameterId,
    },
    Edge {
        edge: EdgeId,
        parameter: SendParameter,
    },
    /// The node's channel strip, fanning out to every compiled instance
    /// (PRE and POST) of the active plan.
    Strip {
        node: NodeId,
        parameter: StripParameter,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesiredParameter {
    pub target: ParameterTarget,
    pub value: ParamValue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessingConfig {
    pub engine: EngineConfig,
    pub audio_byte_budget: usize,
    pub control_capacity: usize,
    pub control_horizon_frames: u64,
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 256,
                max_events_per_block: 64,
            },
            audio_byte_budget: 16 * 1024 * 1024,
            control_capacity: 64,
            control_horizon_frames: 48_000,
        }
    }
}

impl ProcessingConfig {
    pub fn compile_config(self, plan_revision: u64, timeline_epoch: u64) -> CompileConfig {
        CompileConfig {
            engine: self.engine,
            audio_byte_budget: self.audio_byte_budget,
            plan_revision,
            timeline_epoch,
            control_capacity: self.control_capacity,
            control_horizon_frames: self.control_horizon_frames,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineProject {
    pub strips: BTreeMap<NodeId, crate::processor::StripConfig>,
    pub graph: LogicalGraph,
    pub nodes: BTreeMap<NodeId, NodeConfig>,
    pub parameters: Vec<DesiredParameter>,
    pub processing: ProcessingConfig,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProjectError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error("node {0:?} has no configuration")]
    MissingConfiguration(NodeId),
    #[error("node {0:?} configuration does not match its graph kind")]
    ConfigurationKind(NodeId),
    #[error("node {0:?} configuration is not in the graph")]
    UnknownConfiguration(NodeId),
    #[error("node {0:?} IO channel count differs from its graph port")]
    IoChannels(NodeId),
    #[error("invalid IO configuration for {node:?}: {error}")]
    Io { node: NodeId, error: IoConfigError },
    #[error("invalid processor configuration for {0:?}")]
    InvalidProcessor(NodeId),
    #[error("invalid processing configuration")]
    InvalidProcessing,
    #[error("invalid parameter target or value")]
    InvalidParameter,
    #[error("project exceeds the host capacity")]
    Capacity,
}

/// Single control-side source for a node kernel's live parameter schema: every
/// table routes through the kernel's own `parameters` constructor, so validate,
/// validate_parameter and parameter_values cannot drift from the kernels (or
/// from each other). Strip and edge schemas route through their kernel tables
/// at the `validate_parameter` call sites, which own the node width.
fn node_specs(config: &NodeConfig) -> Vec<ParamSpec> {
    // Schema identity only; the caller's plan never sees this placeholder id.
    const ID: ProcessorId = ProcessorId(0);
    match config {
        NodeConfig::Gain { initial } => vec![crate::processor::Gain::parameter(ID, *initial)],
        NodeConfig::Pan { initial } => {
            crate::processor::Pan::parameters(ID, *initial, false, false).to_vec()
        }
        NodeConfig::Compressor(settings) => Compressor::parameters(ID, *settings).to_vec(),
        NodeConfig::Eq(settings) => EqProcessor::parameters(ID, *settings).to_vec(),
        NodeConfig::Gate(settings) => Gate::parameters(ID, *settings).to_vec(),
        NodeConfig::Limiter(settings) => Limiter::parameters(ID, *settings).to_vec(),
        NodeConfig::Delay(settings) => Delay::parameters(ID, *settings).to_vec(),
        _ => Vec::new(),
    }
}

impl EngineProject {
    pub fn validate(&self) -> Result<(), ProjectError> {
        self.graph.validate()?;
        for (&node, config) in &self.strips {
            let graph_node = self.graph.get_node(node)?;
            let ports = if graph_node.kind() == NodeKind::Sink {
                graph_node.inputs()
            } else {
                graph_node.outputs()
            };
            if !ports.iter().all(|port| config.valid(port.channels())) {
                return Err(ProjectError::InvalidProcessor(node));
            }
        }
        let config = self.processing;
        if !config.engine.processing_sr.is_finite()
            || config.engine.processing_sr <= 0.0
            || config.engine.max_block_frames == 0
            || config.engine.max_block_frames > u32::MAX as usize
            || config.engine.max_events_per_block == 0
            || config.control_capacity == 0
            || config.control_horizon_frames == 0
        {
            return Err(ProjectError::InvalidProcessing);
        }
        for &node in self.nodes.keys() {
            if self.graph.get_node(node).is_err() {
                return Err(ProjectError::UnknownConfiguration(node));
            }
        }
        for node in self.graph.nodes() {
            let config = self
                .nodes
                .get(&node.id())
                .ok_or(ProjectError::MissingConfiguration(node.id()))?;
            let kind = match config {
                NodeConfig::Input(_) => NodeKind::Source,
                NodeConfig::Output(_) => NodeKind::Sink,
                NodeConfig::Gain { .. } => NodeKind::Gain,
                NodeConfig::Pan { .. } => NodeKind::Pan,
                NodeConfig::Compressor(_) => NodeKind::Compressor,
                NodeConfig::Eq(_) => NodeKind::Eq,
                NodeConfig::Gate(_) => NodeKind::Gate,
                NodeConfig::Limiter(_) => NodeKind::Limiter,
                NodeConfig::Delay(_) => NodeKind::Delay,
                NodeConfig::ChannelMapper(_) => NodeKind::ChannelMapper,
                NodeConfig::Recorder(_) => NodeKind::Recorder,
                NodeConfig::Meter(_) => NodeKind::Meter,
                NodeConfig::SpectrumAnalyzer(_) => NodeKind::SpectrumAnalyzer,
                NodeConfig::Rack(_) => NodeKind::Rack,
                NodeConfig::Bus => NodeKind::Bus,
            };
            if kind != node.kind() {
                return Err(ProjectError::ConfigurationKind(node.id()));
            }
            let io = match config {
                NodeConfig::Input(config) => Some((config.channels, config.validate())),
                NodeConfig::Output(config) => Some((config.channels, config.validate())),
                _ => None,
            };
            if let Some((channels, result)) = io {
                result.map_err(|error| ProjectError::Io {
                    node: node.id(),
                    error,
                })?;
                if channels != node.channels() {
                    return Err(ProjectError::IoChannels(node.id()));
                }
            }
            let valid = match config {
                NodeConfig::Gain { .. }
                | NodeConfig::Pan { .. }
                | NodeConfig::Compressor(_)
                | NodeConfig::Eq(_)
                | NodeConfig::Gate(_)
                | NodeConfig::Limiter(_) => specs_accept_initials(&node_specs(config)),
                NodeConfig::Delay(settings) => {
                    settings.storage_valid() && specs_accept_initials(&node_specs(config))
                }
                NodeConfig::ChannelMapper(settings) => {
                    settings.valid_layout(node.inputs()[0].channels(), node.outputs()[0].channels())
                }
                NodeConfig::Recorder(settings) => {
                    settings.valid() && node.channels() <= MAX_CHANNELS
                }
                NodeConfig::Meter(settings) => settings.valid() && node.channels() <= MAX_CHANNELS,
                NodeConfig::SpectrumAnalyzer(settings) => {
                    settings.valid() && node.channels() <= MAX_CHANNELS
                }
                NodeConfig::Rack(settings) => settings.valid_layout(
                    &node
                        .inputs()
                        .iter()
                        .map(|port| port.channels())
                        .collect::<Vec<_>>(),
                    &node
                        .outputs()
                        .iter()
                        .map(|port| port.channels())
                        .collect::<Vec<_>>(),
                ),
                _ => true,
            };
            if !valid {
                return Err(ProjectError::InvalidProcessor(node.id()));
            }
        }
        for (index, parameter) in self.parameters.iter().enumerate() {
            if self.parameters[..index]
                .iter()
                .any(|previous| previous.target == parameter.target)
            {
                return Err(ProjectError::InvalidParameter);
            }
            self.validate_parameter(parameter.target, parameter.value)?;
        }
        Ok(())
    }

    pub fn has_output(&self) -> bool {
        self.nodes
            .values()
            .any(|config| matches!(config, NodeConfig::Output(_)))
    }

    pub fn edit(
        &mut self,
        edit: impl FnOnce(&mut Self) -> Result<(), ProjectError>,
    ) -> Result<(), ProjectError> {
        self.edit_with_result(edit)
    }

    /// Return a control-side result only after the entire candidate validates.
    /// The unit-result `edit` entry point retains its original inference rules.
    pub fn edit_with_result<T>(
        &mut self,
        edit: impl FnOnce(&mut Self) -> Result<T, ProjectError>,
    ) -> Result<T, ProjectError> {
        let mut candidate = self.clone();
        let result = edit(&mut candidate)?;
        for node in self.graph.nodes() {
            if candidate.graph.get_node(node.id()).is_err() {
                candidate.nodes.remove(&node.id());
                candidate.strips.remove(&node.id());
            }
        }
        // Keep newly introduced invalid targets for validation, but remove old
        // intent when its node, edge or active rack slot disappears. Moving the
        // vector lets target checks borrow the complete control-side candidate.
        let mut parameters = std::mem::take(&mut candidate.parameters);
        parameters.retain(|parameter| match parameter.target {
            ParameterTarget::Node { node, .. } => {
                candidate.graph.get_node(node).is_ok() || self.graph.get_node(node).is_err()
            }
            ParameterTarget::RackSlot { .. } => {
                candidate.target_exists(parameter.target) || !self.target_exists(parameter.target)
            }
            ParameterTarget::Edge { edge, .. } => {
                candidate.graph.get_edge(edge).is_ok() || self.graph.get_edge(edge).is_err()
            }
            ParameterTarget::Strip { node, .. } => {
                candidate.graph.get_node(node).is_ok() || self.graph.get_node(node).is_err()
            }
        });
        candidate.parameters = parameters;
        candidate.validate()?;
        *self = candidate;
        Ok(result)
    }

    pub fn target_exists(&self, target: ParameterTarget) -> bool {
        match target {
            ParameterTarget::Node { node, .. } => self.graph.get_node(node).is_ok(),
            ParameterTarget::RackSlot { node, slot, .. } => self.rack_slot(node, slot).is_some(),
            ParameterTarget::Edge { edge, .. } => self.graph.get_edge(edge).is_ok(),
            ParameterTarget::Strip { node, .. } => self.strips.contains_key(&node),
        }
    }

    /// The narrowest port width a strip must satisfy, mirroring the per-port
    /// validity rule of [`EngineProject::validate`].
    pub(crate) fn strip_width(&self, node: NodeId) -> Option<usize> {
        let graph_node = self.graph.get_node(node).ok()?;
        let ports = if graph_node.kind() == NodeKind::Sink {
            graph_node.inputs()
        } else {
            graph_node.outputs()
        };
        ports.iter().map(|port| port.channels()).min()
    }

    fn rack_slot(&self, node: NodeId, slot: u32) -> Option<(usize, &crate::rack::RackSlot)> {
        let graph_node = self.graph.get_node(node).ok()?;
        if graph_node.kind() != NodeKind::Rack {
            return None;
        }
        let NodeConfig::Rack(settings) = self.nodes.get(&node)? else {
            return None;
        };
        let slot = settings
            .slots
            .iter()
            .find(|candidate| candidate.id == slot && !candidate.bypass)?;
        let layout = settings.resolve_layout(
            &graph_node
                .inputs()
                .iter()
                .map(|port| port.channels())
                .collect::<Vec<_>>(),
            &graph_node
                .outputs()
                .iter()
                .map(|port| port.channels())
                .collect::<Vec<_>>(),
        )?;
        Some((layout.slots.get(&slot.id)?.input_channels, slot))
    }

    pub fn validate_parameter(
        &self,
        target: ParameterTarget,
        value: ParamValue,
    ) -> Result<(), ProjectError> {
        let valid = match target {
            ParameterTarget::Node { node, parameter } => match self.nodes.get(&node) {
                Some(
                    config @ (NodeConfig::Gain { .. }
                    | NodeConfig::Pan { .. }
                    | NodeConfig::Compressor(_)
                    | NodeConfig::Eq(_)
                    | NodeConfig::Gate(_)
                    | NodeConfig::Limiter(_)
                    | NodeConfig::Delay(_)),
                ) => accepts_parameter(&node_specs(config), parameter, value),
                _ => false,
            },
            ParameterTarget::RackSlot {
                node,
                slot,
                parameter,
            } => self.rack_slot(node, slot).is_some_and(|(channels, slot)| {
                slot.processor
                    .checked_parameters(channels, ProcessorId(0))
                    .is_ok_and(|specs| accepts_parameter(&specs, parameter, value))
            }),
            ParameterTarget::Strip { node, parameter } => {
                // Logical slot identity maps onto the strip kernel's own ids;
                // width only shapes the Mode domain. PRE strips omit the gain
                // slot, but live strip intent is always POST (include_gain).
                let slot = match parameter {
                    StripParameter::Gain => crate::processor::builtin::ChannelStrip::GAIN,
                    StripParameter::Balance => crate::processor::builtin::ChannelStrip::BALANCE,
                    StripParameter::Mode => crate::processor::builtin::ChannelStrip::MODE,
                    StripParameter::EffectiveMute => {
                        crate::processor::builtin::ChannelStrip::EFFECTIVE_MUTE
                    }
                };
                let width = self.strip_width(node).unwrap_or(usize::MAX);
                self.strips.contains_key(&node)
                    && accepts_parameter(
                        &crate::processor::builtin::ChannelStrip::parameters(
                            ProcessorId(0),
                            width,
                            crate::processor::StripConfig::default(),
                            true,
                        ),
                        slot,
                        value,
                    )
            }
            ParameterTarget::Edge { edge, parameter } => match self.graph.get_edge(edge) {
                Ok(edge) => {
                    let slot = match parameter {
                        SendParameter::Gain => crate::compiler::send::Send::GAIN,
                        SendParameter::Pan => crate::compiler::send::Send::PAN,
                        SendParameter::Mute => crate::compiler::send::Send::MUTE,
                    };
                    // A mono route narrows the send kernel's pan domain to
                    // center; an unresolvable port behaves the same way.
                    let channels = self
                        .graph
                        .get_port(edge.src_port())
                        .map_or(1, |port| port.channels());
                    accepts_parameter(
                        &crate::compiler::send::Send::parameters(
                            ProcessorId(0),
                            channels,
                            *edge.params(),
                        )
                        .0,
                        slot,
                        value,
                    )
                }
                Err(_) => false,
            },
        };
        if valid {
            Ok(())
        } else {
            Err(ProjectError::InvalidParameter)
        }
    }

    pub(crate) fn set_parameter(&mut self, target: ParameterTarget, value: ParamValue) {
        if let Some(parameter) = self
            .parameters
            .iter_mut()
            .find(|parameter| parameter.target == target)
        {
            parameter.value = value;
        } else {
            self.parameters.push(DesiredParameter { target, value });
        }
    }

    /// Effective desired values: node/slot/edge configuration defaults overridden
    /// by explicitly recorded parameter intent. This allocating control-side
    /// query does not report instantaneous runtime values or apply parameters.
    pub fn parameter_values(&self) -> Vec<DesiredParameter> {
        let mut values = Vec::new();
        for (&node, config) in &self.nodes {
            if let NodeConfig::Rack(settings) = config {
                for slot in settings.slots.iter().filter(|slot| !slot.bypass) {
                    let Some((channels, slot)) = self.rack_slot(node, slot.id) else {
                        continue;
                    };
                    // Control-side only: use the same checked schema as lowering,
                    // without allocating a kernel or leaking plan-local identities.
                    if let Ok(specs) = slot.processor.checked_parameters(channels, ProcessorId(0)) {
                        values.extend(specs.into_iter().map(|spec| DesiredParameter {
                            target: ParameterTarget::RackSlot {
                                node,
                                slot: slot.id,
                                parameter: spec.key.parameter,
                            },
                            value: spec.initial,
                        }));
                    }
                }
                continue;
            }
            let specs = match config {
                NodeConfig::Gain { .. }
                | NodeConfig::Pan { .. }
                | NodeConfig::Compressor(_)
                | NodeConfig::Eq(_)
                | NodeConfig::Gate(_)
                | NodeConfig::Limiter(_)
                | NodeConfig::Delay(_) => node_specs(config),
                _ => Vec::new(),
            };
            for spec in specs {
                values.push(DesiredParameter {
                    target: ParameterTarget::Node {
                        node,
                        parameter: spec.key.parameter,
                    },
                    value: spec.initial,
                });
            }
        }
        for edge in self.graph.edges() {
            for (parameter, value) in [
                (SendParameter::Gain, ParamValue::Float(edge.params().gain)),
                (SendParameter::Pan, ParamValue::Float(edge.params().pan)),
                (SendParameter::Mute, ParamValue::Bool(edge.params().mute)),
            ] {
                values.push(DesiredParameter {
                    target: ParameterTarget::Edge {
                        edge: edge.id(),
                        parameter,
                    },
                    value,
                });
            }
        }
        for (&node, config) in &self.strips {
            let Some(width) = self.strip_width(node) else {
                continue;
            };
            for (parameter, value) in [
                (StripParameter::Gain, ParamValue::Float(config.gain)),
                (StripParameter::Balance, ParamValue::Float(config.balance)),
                (
                    StripParameter::Mode,
                    ParamValue::Enum(crate::processor::builtin::ChannelStrip::mode_index(
                        config.mode,
                        width,
                    )),
                ),
                (
                    StripParameter::EffectiveMute,
                    ParamValue::Bool(config.muted),
                ),
            ] {
                values.push(DesiredParameter {
                    target: ParameterTarget::Strip { node, parameter },
                    value,
                });
            }
        }
        for parameter in &self.parameters {
            if let Some(value) = values
                .iter_mut()
                .find(|value| value.target == parameter.target)
            {
                value.value = parameter.value;
            }
        }
        values
    }
}
