//! Workspace identities shared by shell and persistence.

pub mod graph_canvas;
pub mod graph_canvas_view;
pub mod session_setup;

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
            Page::Matrix => "Matrix",
            Page::Devices => "Devices",
        }
    }
}
