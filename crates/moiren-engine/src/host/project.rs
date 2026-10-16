use std::collections::BTreeMap;

use moiren_core::{
    graph::{EdgeId, GraphError, LogicalGraph, NodeId, NodeKind},
    protocol::{ParamValue, ParameterId, ProcessorId},
};
use thiserror::Error;

use crate::{
    compiler::CompileConfig,
    node::{InputConfig, IoConfigError, OutputConfig},
    processor::{Compressor, CompressorSettings},
    runtime::EngineConfig,
};

#[derive(Debug, Clone, PartialEq)]
pub enum NodeConfig {
    Input(InputConfig),
    Output(OutputConfig),
    Gain { initial: f64 },
    Pan { initial: f64 },
    Compressor(CompressorSettings),
    Bus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendParameter {
    Gain,
    Pan,
    Mute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterTarget {
    Node {
        node: NodeId,
        parameter: ParameterId,
    },
    Edge {
        edge: EdgeId,
        parameter: SendParameter,
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

impl EngineProject {
    pub fn validate(&self) -> Result<(), ProjectError> {
        self.graph.validate()?;
        for (&node, config) in &self.strips {
            if !config.valid(self.graph.get_node(node)?.channels()) {
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
                NodeConfig::Gain { initial } => {
                    crate::processor::Gain::parameter(ProcessorId(0), *initial)
                        .accepts(ParamValue::Float(*initial), 0)
                }
                NodeConfig::Pan { initial } => {
                    initial.is_finite() && (-1.0..=1.0).contains(initial)
                }
                NodeConfig::Compressor(settings) => {
                    Compressor::parameters(ProcessorId(0), *settings)
                        .iter()
                        .all(|spec| spec.accepts(spec.initial, 0))
                }
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
        let graph = &candidate.graph;
        candidate
            .parameters
            .retain(|parameter| match parameter.target {
                ParameterTarget::Node { node, .. } => {
                    graph.get_node(node).is_ok() || self.graph.get_node(node).is_err()
                }
                ParameterTarget::Edge { edge, .. } => {
                    graph.get_edge(edge).is_ok() || self.graph.get_edge(edge).is_err()
                }
            });
        candidate.validate()?;
        *self = candidate;
        Ok(result)
    }

    pub fn target_exists(&self, target: ParameterTarget) -> bool {
        match target {
            ParameterTarget::Node { node, .. } => self.graph.get_node(node).is_ok(),
            ParameterTarget::Edge { edge, .. } => self.graph.get_edge(edge).is_ok(),
        }
    }

    pub fn validate_parameter(
        &self,
        target: ParameterTarget,
        value: ParamValue,
    ) -> Result<(), ProjectError> {
        let valid = match target {
            ParameterTarget::Node { node, parameter } => match self.nodes.get(&node) {
                Some(NodeConfig::Gain { .. }) => {
                    parameter == crate::processor::Gain::LEVEL
                        && crate::processor::Gain::parameter(ProcessorId(0), 1.0).accepts(value, 0)
                }
                Some(NodeConfig::Pan { .. }) => {
                    (parameter == crate::processor::Pan::POSITION
                        && matches!(value, ParamValue::Float(value) if value.is_finite() && (-1.0..=1.0).contains(&value)))
                        || (matches!(
                            parameter,
                            crate::processor::Pan::MONO | crate::processor::Pan::SWAP
                        ) && matches!(value, ParamValue::Bool(_)))
                }
                Some(NodeConfig::Compressor(settings)) => {
                    Compressor::parameters(ProcessorId(0), *settings)
                        .iter()
                        .any(|spec| spec.key.parameter == parameter && spec.accepts(value, 0))
                }
                _ => false,
            },
            ParameterTarget::Edge { edge, parameter } => {
                self.graph.get_edge(edge).is_ok()
                    && match (parameter, value) {
                        (SendParameter::Gain, ParamValue::Float(value)) => {
                            value.is_finite() && value >= 0.0
                        }
                        (SendParameter::Pan, ParamValue::Float(value)) => {
                            value.is_finite()
                                && (-1.0..=1.0).contains(&value)
                                && (value == 0.0
                                    || self
                                        .graph
                                        .get_edge(edge)
                                        .and_then(|edge| self.graph.get_port(edge.src_port()))
                                        .is_ok_and(|port| port.channels() == 2))
                        }
                        (SendParameter::Mute, ParamValue::Bool(_)) => true,
                        _ => false,
                    }
            }
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

    /// Effective desired values: node/edge configuration defaults overridden
    /// by explicitly recorded parameter intent. This allocating control-side
    /// query does not report instantaneous runtime values or apply parameters.
    pub fn parameter_values(&self) -> Vec<DesiredParameter> {
        let mut values = Vec::new();
        for (&node, config) in &self.nodes {
            let specs = match config {
                NodeConfig::Gain { initial } => {
                    vec![crate::processor::Gain::parameter(ProcessorId(0), *initial)]
                }
                NodeConfig::Pan { initial } => {
                    vec![crate::processor::Pan::parameter(ProcessorId(0), *initial)]
                }
                NodeConfig::Compressor(settings) => {
                    Compressor::parameters(ProcessorId(0), *settings).to_vec()
                }
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
            if matches!(config, NodeConfig::Pan { .. }) {
                for parameter in [crate::processor::Pan::MONO, crate::processor::Pan::SWAP] {
                    values.push(DesiredParameter {
                        target: ParameterTarget::Node { node, parameter },
                        value: ParamValue::Bool(false),
                    });
                }
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
