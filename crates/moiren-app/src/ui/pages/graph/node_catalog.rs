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

/// The single NodeKind presentation registry (issue #122): every surface
/// that colors or icons a node — canvas cards, drag ghosts, catalog
/// tiles, mixer/tray color bars — resolves through here, so a new kind
/// lands everywhere by extending one match.
pub(crate) fn kind_tint(kind: NodeKind) -> Hsla {
    use moiren_core::graph::NodeKind::*;
    match kind {
        Source => theme::NODE_SOURCE,
        Sink => theme::NODE_SINK,
        Bus | Rack | ChannelMapper => theme::NODE_UTILITY,
        Recorder | Meter | SpectrumAnalyzer => theme::NODE_ANALYSIS,
        _ => theme::NODE_DSP,
    }
}

/// Human display title for a kind (issue #121): the inspector's subtitle
/// and the mixer/tray strip subtitles show this instead of the debug
/// format. Wording follows the catalog template titles where a template
/// exists ("Parametric EQ", "Mix bus", ...); kinds without a template of
/// their own get a plain name — Source and Sink each cover several
/// templates, and Plugin has none.
///
/// Plain English today, like the template titles: per design 21 §4 these
/// strings are runtime i18n table keys, so later translation swaps the
/// lookup here without touching the call sites.
pub(crate) fn kind_title(kind: NodeKind) -> &'static str {
    use moiren_core::graph::NodeKind::*;
    match kind {
        Source => "Input",
        Sink => "Output",
        Gain => "Gain",
        Pan => "Balance",
        Eq => "Parametric EQ",
        Compressor => "Compressor",
        Gate => "Gate",
        Limiter => "Limiter",
        Delay => "Delay",
        ChannelMapper => "Channel mapper",
        Recorder => "Recorder",
        Meter => "Level meter",
        SpectrumAnalyzer => "Spectrum analyzer",
        Rack => "Processor rack",
        Bus => "Mix bus",
        Plugin => "Plugin",
    }
}

/// The card icon for a kind; a Source's icon follows its live input
/// selection (physical mic, application loopback, or the tone generator).
pub(crate) fn kind_icon(
    kind: NodeKind,
    source: Option<&crate::service::InputSelection>,
) -> &'static str {
    use moiren_core::graph::NodeKind::*;
    match kind {
        Source => match source {
            Some(crate::service::InputSelection::Process { .. }) => "app",
            Some(crate::service::InputSelection::Tone { .. }) => "wave",
            Some(crate::service::InputSelection::Physical { .. }) | None => "mic",
        },
        Sink => "speaker",
        Bus => "git-merge",
        Recorder => "radio",
        Meter => "activity",
        SpectrumAnalyzer => "chart-histogram",
        Delay => "clock",
        Gain => "volume",
        Pan => "arrows-horizontal",
        Eq => "wave-square",
        Compressor => "arrows-minimize",
        Gate => "door",
        Limiter => "wall",
        Rack => "list-details",
        ChannelMapper => "route",
        Plugin => "sliders",
    }
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
            Self::Bus => "Bus dynamic input ports",
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
            Self::Spectrum => "chart-histogram",
            Self::Delay => "clock",
            Self::ChannelMapper => "route",
            Self::Rack => "list-details",
            Self::Bus => "git-merge",
            Self::Output => "speaker",
            Self::Gain => "volume",
            Self::Balance => "arrows-horizontal",
            Self::Compressor => "arrows-minimize",
            Self::Eq => "wave-square",
            Self::Gate => "door",
            Self::Limiter => "wall",
        }
    }

    pub fn tint(self) -> Hsla {
        self.kind().map_or(theme::NODE_DSP, kind_tint)
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
