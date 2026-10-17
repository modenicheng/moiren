//! Persisted processor intent. Engine validation remains the authority for
//! parameter domains, channel layouts, and nested rack settings.
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::{
    host::{EngineProject, NodeConfig},
    meter::MeterSettings,
    processor::{ChannelMapperSettings, DelaySettings, EqSettings, GateSettings, LimiterSettings},
    rack::RackSettings,
    recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BuiltinState {
    #[default]
    None,
    Eq(EqSettings),
    Gate(GateSettings),
    Limiter(LimiterSettings),
    Delay(DelaySettings),
    ChannelMapper(ChannelMapperSettings),
    Recorder(RecorderSettings),
    Meter(MeterSettings),
    SpectrumAnalyzer(SpectrumSettings),
    Rack(RackSettings),
}

impl BuiltinState {
    pub fn for_kind(kind: NodeKind) -> Self {
        match kind {
            NodeKind::Eq => Self::Eq(EqSettings::default()),
            NodeKind::Gate => Self::Gate(GateSettings::default()),
            NodeKind::Limiter => Self::Limiter(LimiterSettings::default()),
            NodeKind::Delay => Self::Delay(DelaySettings::default()),
            NodeKind::ChannelMapper => Self::ChannelMapper(ChannelMapperSettings::default()),
            NodeKind::Recorder => Self::Recorder(RecorderSettings::default()),
            NodeKind::Meter => Self::Meter(MeterSettings::default()),
            NodeKind::SpectrumAnalyzer => Self::SpectrumAnalyzer(SpectrumSettings::default()),
            NodeKind::Rack => Self::Rack(RackSettings::default()),
            _ => Self::None,
        }
    }

    /// Older documents omitted builtin state. Resolve that absence only against
    /// the topology kind; an explicitly different variant is always an error.
    pub fn resolved(&self, kind: NodeKind) -> Self {
        if matches!(self, Self::None) {
            Self::for_kind(kind)
        } else {
            self.clone()
        }
    }

    pub fn eq(&self) -> Option<&EqSettings> {
        match self {
            Self::Eq(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn gate(&self) -> Option<&GateSettings> {
        match self {
            Self::Gate(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn limiter(&self) -> Option<&LimiterSettings> {
        match self {
            Self::Limiter(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn delay(&self) -> Option<&DelaySettings> {
        match self {
            Self::Delay(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn channel_mapper(&self) -> Option<&ChannelMapperSettings> {
        match self {
            Self::ChannelMapper(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn recorder(&self) -> Option<&RecorderSettings> {
        match self {
            Self::Recorder(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn meter(&self) -> Option<&MeterSettings> {
        match self {
            Self::Meter(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn spectrum(&self) -> Option<&SpectrumSettings> {
        match self {
            Self::SpectrumAnalyzer(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn rack(&self) -> Option<&RackSettings> {
        match self {
            Self::Rack(settings) => Some(settings),
            _ => None,
        }
    }

    pub(super) fn node_config(&self, kind: NodeKind) -> Result<Option<NodeConfig>, String> {
        let (expected, config) = match self.resolved(kind) {
            Self::None => return Ok(None),
            Self::Eq(settings) => (NodeKind::Eq, NodeConfig::Eq(settings)),
            Self::Gate(settings) => (NodeKind::Gate, NodeConfig::Gate(settings)),
            Self::Limiter(settings) => (NodeKind::Limiter, NodeConfig::Limiter(settings)),
            Self::Delay(settings) => (NodeKind::Delay, NodeConfig::Delay(settings)),
            Self::ChannelMapper(settings) => {
                (NodeKind::ChannelMapper, NodeConfig::ChannelMapper(settings))
            }
            Self::Recorder(settings) => (NodeKind::Recorder, NodeConfig::Recorder(settings)),
            Self::Meter(settings) => (NodeKind::Meter, NodeConfig::Meter(settings)),
            Self::SpectrumAnalyzer(settings) => (
                NodeKind::SpectrumAnalyzer,
                NodeConfig::SpectrumAnalyzer(settings),
            ),
            Self::Rack(settings) => (NodeKind::Rack, NodeConfig::Rack(settings)),
        };
        if kind != expected {
            return Err(format!(
                "builtin settings for {expected:?} cannot configure a {kind:?} node"
            ));
        }
        Ok(Some(config))
    }

    pub fn validate(
        &self,
        kind: NodeKind,
        input_channels: usize,
        output_channels: usize,
    ) -> Result<(), String> {
        let Some(config) = self.node_config(kind)? else {
            return Ok(());
        };
        // Isolated validation accepts unbound editable projects without
        // manufacturing IO resources or allocating any processor's RT storage.
        let mut graph = LogicalGraph::new();
        let id = if kind == NodeKind::ChannelMapper {
            graph.create_channel_mapper(input_channels, output_channels)
        } else {
            graph.create_node(kind, output_channels)
        }
        .map_err(|error| error.to_string())?;
        let mut project = EngineProject {
            graph,
            ..EngineProject::default()
        };
        project.nodes.insert(id, config);
        project.validate().map_err(|error| error.to_string())
    }
}
