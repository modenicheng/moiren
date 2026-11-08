//! The workspace pages: shared identities plus each page's module tree.

pub(crate) mod devices;
pub(crate) mod graph;
pub(crate) mod matrix;
pub(crate) mod mixer;
pub(crate) mod settings;

// The pages render against the shell's shared surface (gpui prelude, theme,
// service types, StudioShell). Re-exported here so page modules keep their
// `use super::*` glob after moving out of the shell module.
pub(crate) use crate::ui::shell::*;

/// Which workspace page is active (toolbar tabs).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    #[default]
    Graph,
    Mixer,
    Matrix,
    Devices,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Graph => "Graph",
            Page::Mixer => "Mixer",
            Page::Devices => "Devices",
            Page::Matrix => "Matrix",
        }
    }
}
