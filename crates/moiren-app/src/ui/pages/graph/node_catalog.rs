use gpui_kit::Hsla;
use moiren_core::graph::NodeKind;

use crate::ui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTemplate {
    Input,
    VirtualDevice,
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
    Plugin,
    Output,
}

/// Which kind of capture resource a Source node presents, OBS-style: the
/// library spawns a node per class, and its pickers list only that class's
/// resources (2026-11-13 decision — template splitting, not dropdown
/// grouping). A binding/UI-layer concept only: every class is still one
/// `NodeKind::Source`, and the live binding re-derives the class so a
/// restored project keeps filtering without stored presentation state
/// wherever it is bound. `Tone` keeps the generator's own picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceClass {
    /// Microphones and line-in endpoints the active backend offers
    /// (WASAPI capture endpoints, or ASIO drivers under ASIO).
    Physical,
    /// App-owned virtual (VAD) endpoints — playback exports and their
    /// Windows device rows.
    Virtual,
    /// Per-process loopback capture.
    Application,
    /// The prepared sine-wave generator.
    Tone,
}

impl From<SourceClass> for crate::project::SourceClassDocument {
    fn from(class: SourceClass) -> Self {
        match class {
            SourceClass::Physical => Self::Physical,
            SourceClass::Virtual => Self::Virtual,
            SourceClass::Application => Self::Application,
            SourceClass::Tone => Self::Tone,
        }
    }
}

impl From<crate::project::SourceClassDocument> for SourceClass {
    fn from(class: crate::project::SourceClassDocument) -> Self {
        match class {
            crate::project::SourceClassDocument::Physical => SourceClass::Physical,
            crate::project::SourceClassDocument::Virtual => SourceClass::Virtual,
            crate::project::SourceClassDocument::Application => SourceClass::Application,
            crate::project::SourceClassDocument::Tone => SourceClass::Tone,
        }
    }
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
        Bus | Rack | ChannelMapper => theme::TEXT_FAINT,
        Recorder | Meter | SpectrumAnalyzer => theme::SIGNAL_DEVICE,
        // DSP (and any future kind without a dedicated hue) wears the shared
        // accent: `controls::FOCUS` doubles as the DSP node tint.
        _ => theme::controls::FOCUS,
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

/// The card icon for a kind. A Source's icon follows its presentation
/// class — the stored identity, never the live binding: what a source node
/// *is* comes from [`SourceClass`] alone, and the binding only names where
/// it is wired (see the binding chip and `source_name`).
pub(crate) fn kind_icon(kind: NodeKind, source_class: Option<SourceClass>) -> &'static str {
    use moiren_core::graph::NodeKind::*;
    match kind {
        Source => match source_class {
            Some(SourceClass::Virtual) => VIRTUAL_DEVICE_ICON,
            Some(SourceClass::Application) => "app",
            Some(SourceClass::Tone) => "wave",
            // `None` only occurs on the frame before the render sync
            // materializes a class for a node that arrived without one;
            // the physical glyph matches what materialization will pick
            // for an unclassable node.
            Some(SourceClass::Physical) | None => "mic",
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

/// The icon name every virtual-device surface wears (template card,
/// virtual-class source cards).
pub(crate) const VIRTUAL_DEVICE_ICON: &str = "repeat";

impl NodeTemplate {
    pub const ALL: [Self; 19] = [
        Self::Input,
        Self::VirtualDevice,
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
        Self::Plugin,
        Self::Output,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Input => "Physical input",
            Self::VirtualDevice => "Virtual device",
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
            Self::Plugin => "Plugin",
            Self::Output => "Device output",
        }
    }

    /// The source class a node created from this template presents; only
    /// source templates carry one.
    pub fn source_class(self) -> Option<SourceClass> {
        match self {
            Self::Input => Some(SourceClass::Physical),
            Self::VirtualDevice => Some(SourceClass::Virtual),
            Self::Application => Some(SourceClass::Application),
            Self::Tone => Some(SourceClass::Tone),
            _ => None,
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::Input | Self::VirtualDevice | Self::Application | Self::Tone => "SOURCES",
            Self::Gain
            | Self::Balance
            | Self::Compressor
            | Self::Eq
            | Self::Gate
            | Self::Limiter
            | Self::Delay
            | Self::ChannelMapper => "PROCESSING",
            Self::Recorder | Self::Meter | Self::Spectrum => "OBSERVATION",
            Self::Bus | Self::Rack => "ROUTING",
            Self::Plugin => "PROCESSING",
            Self::Output => "DESTINATIONS",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Input => "WASAPI capture endpoint",
            Self::VirtualDevice => "VAD virtual capture endpoint",
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
            Self::Plugin => "Third-party effect from the plugin library",
            Self::Output => "WASAPI render endpoint",
        }
    }

    pub fn kind(self) -> Option<NodeKind> {
        Some(match self {
            Self::Input | Self::VirtualDevice | Self::Application | Self::Tone => NodeKind::Source,
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
            Self::Plugin => NodeKind::Plugin,
            Self::Output => NodeKind::Sink,
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Input => "mic",
            Self::VirtualDevice => "repeat",
            Self::Application => "app",
            Self::Tone => "wave",
            Self::Recorder => "radio",
            Self::Meter => "activity",
            Self::Spectrum => "chart-histogram",
            Self::Delay => "clock",
            Self::ChannelMapper => "route",
            Self::Rack => "list-details",
            Self::Bus => "git-merge",
            Self::Plugin => "sliders",
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
        self.kind().map_or(theme::controls::FOCUS, kind_tint)
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
        assert!(NodeTemplate::VirtualDevice.matches("vad"));
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

    /// The OBS-style split (2026-11-13): every source template carries a
    /// class, no other template does, and the three device classes plus the
    /// generator each have exactly one template.
    #[test]
    fn source_templates_map_one_to_one_onto_classes() {
        let classes: Vec<_> = NodeTemplate::ALL
            .into_iter()
            .filter_map(NodeTemplate::source_class)
            .collect();
        for class in [
            SourceClass::Physical,
            SourceClass::Virtual,
            SourceClass::Application,
            SourceClass::Tone,
        ] {
            assert_eq!(
                classes.iter().filter(|&&c| c == class).count(),
                1,
                "{class:?} must have exactly one template"
            );
        }
        assert_eq!(
            NodeTemplate::Input.source_class(),
            Some(SourceClass::Physical)
        );
        assert_eq!(
            NodeTemplate::VirtualDevice.source_class(),
            Some(SourceClass::Virtual)
        );
        assert_eq!(
            NodeTemplate::Application.source_class(),
            Some(SourceClass::Application)
        );
        assert_eq!(NodeTemplate::Tone.source_class(), Some(SourceClass::Tone));
        assert_eq!(NodeTemplate::Gain.source_class(), None);
        assert_eq!(NodeTemplate::VirtualDevice.kind(), Some(NodeKind::Source));
    }

    #[test]
    fn plugin_template_creates_a_plugin_node_in_the_processing_group() {
        assert!(NodeTemplate::ALL.contains(&NodeTemplate::Plugin));
        assert_eq!(
            NodeTemplate::Plugin.kind(),
            Some(NodeKind::Plugin),
            "the template lowers to the closed NodeKind enum"
        );
        assert_eq!(NodeTemplate::Plugin.group(), "PROCESSING");
        assert!(NodeTemplate::Plugin.matches("third-party"));
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
