use gpui_kit::Hsla;
use moiren_core::graph::NodeKind;

use crate::ui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTemplate {
    Input,
    Application,
    Tone,
    Gain,
    Balance,
    Compressor,
    Eq,
    Gate,
    Limiter,
    Delay,
    ChannelMapper,
    Recorder,
    Meter,
    Spectrum,
    Rack,
    Bus,
    Output,
}

impl NodeTemplate {
    pub const ALL: [Self; 17] = [
        Self::Input,
        Self::Application,
        Self::Tone,
        Self::Gain,
        Self::Balance,
        Self::Compressor,
        Self::Eq,
        Self::Gate,
        Self::Limiter,
        Self::Delay,
        Self::ChannelMapper,
        Self::Recorder,
        Self::Meter,
        Self::Spectrum,
        Self::Rack,
        Self::Bus,
        Self::Output,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Input => "Physical input",
            Self::Application => "Application audio",
            Self::Tone => "Tone generator",
            Self::Gain => "Gain",
            Self::Balance => "Balance",
            Self::Compressor => "Compressor",
            Self::Eq => "Parametric EQ",
            Self::Gate => "Gate",
            Self::Limiter => "Limiter",
            Self::Delay => "Delay",
            Self::ChannelMapper => "Channel mapper",
            Self::Recorder => "Recorder",
            Self::Meter => "Level meter",
            Self::Spectrum => "Spectrum analyzer",
            Self::Rack => "Processor rack",
            Self::Bus => "Mix bus",
            Self::Output => "Device output",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::Input | Self::Application | Self::Tone => "SOURCES",
            Self::Gain
            | Self::Balance
            | Self::Compressor
            | Self::Eq
            | Self::Gate
            | Self::Limiter
            | Self::Delay
            | Self::ChannelMapper => "PROCESSING",
            Self::Recorder | Self::Meter | Self::Spectrum => "OBSERVATION",
            Self::Bus => "ROUTING",
            Self::Output => "DESTINATIONS",
            Self::Rack => "ROUTING",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Input => "WASAPI capture endpoint",
            Self::Application => "Process loopback source",
            Self::Tone => "Prepared sine-wave source",
            Self::Gain => "Adjust linear amplitude",
            Self::Balance => "Stereo balance, mono and swap",
            Self::Compressor => "Linked peak dynamics",
            Self::Eq => "One parametric peaking band",
            Self::Gate => "Linked downward expander",
            Self::Limiter => "Zero-latency peak limiter",
            Self::Delay => "Bounded feedback delay",
            Self::ChannelMapper => "Static channel routing matrix",
            Self::Recorder => "Managed recording tap",
            Self::Meter => "Peak and RMS observation",
            Self::Spectrum => "Managed FFT observation",
            Self::Rack => "Ordered processor chain",
            Self::Bus => "Sum dynamic input ports",
            Self::Output => "WASAPI render endpoint",
        }
    }

    pub fn kind(self) -> Option<NodeKind> {
        Some(match self {
            Self::Input | Self::Application | Self::Tone => NodeKind::Source,
            Self::Gain => NodeKind::Gain,
            Self::Balance => NodeKind::Pan,
            Self::Compressor => NodeKind::Compressor,
            Self::Eq => NodeKind::Eq,
            Self::Gate => NodeKind::Gate,
            Self::Limiter => NodeKind::Limiter,
            Self::Delay => NodeKind::Delay,
            Self::ChannelMapper => NodeKind::ChannelMapper,
            Self::Recorder => NodeKind::Recorder,
            Self::Meter => NodeKind::Meter,
            Self::Spectrum => NodeKind::SpectrumAnalyzer,
            Self::Rack => NodeKind::Rack,
            Self::Bus => NodeKind::Bus,
            Self::Output => NodeKind::Sink,
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Input => "mic",
            Self::Application => "app",
            Self::Tone => "wave",
            Self::Recorder => "radio",
            Self::Meter => "activity",
            Self::Spectrum => "wave",
            Self::Delay => "clock",
            Self::ChannelMapper | Self::Rack => "route",
            Self::Bus => "git-merge",
            Self::Output => "speaker",
            _ => "sliders",
        }
    }

    pub fn tint(self) -> Hsla {
        match self {
            Self::Input | Self::Application | Self::Tone => theme::NODE_SOURCE,
            Self::Bus => theme::NODE_UTILITY,
            Self::Output => theme::NODE_SINK,
            Self::Recorder | Self::Meter | Self::Spectrum => theme::NODE_ANALYSIS,
            Self::Rack => theme::NODE_UTILITY,
            _ => theme::NODE_DSP,
        }
    }

    pub fn matches(self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        [self.title(), self.description(), self.group()]
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_names_groups_and_descriptions() {
        assert!(NodeTemplate::Gain.matches("GAIN"));
        assert!(NodeTemplate::Input.matches("source"));
        assert!(NodeTemplate::Application.matches("process"));
        assert!(!NodeTemplate::Output.matches("compressor"));
        assert!(NodeTemplate::Bus.matches("  "));
    }

    #[test]
    fn spectrum_is_visible_and_creates_a_supported_logical_node() {
        assert!(NodeTemplate::ALL.contains(&NodeTemplate::Spectrum));
        assert_eq!(
            NodeTemplate::Spectrum.kind(),
            Some(NodeKind::SpectrumAnalyzer)
        );
        assert_eq!(NodeTemplate::Gain.kind(), Some(NodeKind::Gain));
    }

    #[test]
    fn catalog_exposes_every_engine_builtin_with_a_real_graph_kind() {
        let expected = [
            NodeKind::Eq,
            NodeKind::Gate,
            NodeKind::Limiter,
            NodeKind::Delay,
            NodeKind::ChannelMapper,
            NodeKind::Recorder,
            NodeKind::Meter,
            NodeKind::SpectrumAnalyzer,
            NodeKind::Rack,
        ];
        let actual = NodeTemplate::ALL
            .into_iter()
            .filter_map(NodeTemplate::kind)
            .collect::<Vec<_>>();

        assert!(expected.into_iter().all(|kind| actual.contains(&kind)));
        assert!(NodeTemplate::ALL.into_iter().any(|template| {
            template.title() == "Spectrum analyzer" && template.kind().is_some()
        }));
        assert!(
            NodeTemplate::ALL
                .into_iter()
                .any(|template| template.title() == "Recorder" && template.matches("managed"))
        );
    }
}
