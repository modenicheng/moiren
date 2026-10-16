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
    Bus,
    Output,
    Analysis,
}

impl NodeTemplate {
    pub const ALL: [Self; 9] = [
        Self::Input,
        Self::Application,
        Self::Tone,
        Self::Gain,
        Self::Balance,
        Self::Compressor,
        Self::Bus,
        Self::Output,
        Self::Analysis,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Input => "Physical input",
            Self::Application => "Application audio",
            Self::Tone => "Tone generator",
            Self::Gain => "Gain",
            Self::Balance => "Balance",
            Self::Compressor => "Compressor",
            Self::Bus => "Mix bus",
            Self::Output => "Device output",
            Self::Analysis => "FFT analysis",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::Input | Self::Application | Self::Tone => "SOURCES",
            Self::Gain | Self::Balance | Self::Compressor => "PROCESSING",
            Self::Bus => "ROUTING",
            Self::Output => "DESTINATIONS",
            Self::Analysis => "ANALYSIS",
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
            Self::Bus => "Sum dynamic input ports",
            Self::Output => "WASAPI render endpoint",
            Self::Analysis => "Unavailable · analysis is not implemented",
        }
    }

    pub fn kind(self) -> Option<NodeKind> {
        Some(match self {
            Self::Input | Self::Application | Self::Tone => NodeKind::Source,
            Self::Gain => NodeKind::Gain,
            Self::Balance => NodeKind::Pan,
            Self::Compressor => NodeKind::Compressor,
            Self::Bus => NodeKind::Bus,
            Self::Output => NodeKind::Sink,
            Self::Analysis => return None,
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Input => "mic",
            Self::Application => "app",
            Self::Tone => "wave",
            Self::Bus => "git-merge",
            Self::Output => "speaker",
            Self::Analysis => "wave",
            _ => "sliders",
        }
    }

    pub fn tint(self) -> Hsla {
        match self {
            Self::Input | Self::Application | Self::Tone => theme::NODE_SOURCE,
            Self::Bus => theme::NODE_UTILITY,
            Self::Output => theme::NODE_SINK,
            Self::Analysis => theme::NODE_ANALYSIS,
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
    fn analysis_is_visible_but_cannot_create_an_unsupported_logical_node() {
        assert!(NodeTemplate::ALL.contains(&NodeTemplate::Analysis));
        assert_eq!(NodeTemplate::Analysis.kind(), None);
        assert_eq!(
            NodeTemplate::Gain.kind(),
            Some(moiren_core::graph::NodeKind::Gain)
        );
    }
}
