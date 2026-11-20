//! The workspace pages: shared identities plus each page's module tree.

pub(crate) mod devices;
pub(crate) mod graph;
pub(crate) mod matrix;
pub(crate) mod mixer;
pub(crate) mod settings;

// The pages render against the shell's shared surface (gpui prelude, theme,
// service types, shared widgets). Re-exported here — explicitly, without a
// glob back into the shell — so page modules keep their `use super::*`
// after moving out of the shell module.
pub(crate) use crate::service::{AppRequest, SessionPhase};
pub(crate) use gpui_kit::prelude::*;
pub(crate) use gpui_kit::*;

pub(crate) use super::shell::{StudioShell, controls, project, sidebar};
pub(crate) use super::theme;
pub(crate) use super::widgets::action_assets::ActionIcon;
pub(crate) use super::widgets::actions::{
    ActionButton, ButtonVariant, ControlSize, Density, IconButton,
};
pub(crate) use crate::ui::observe::TestObserve;
pub(crate) use graph::graph_canvas::Camera;

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

/// What the workspace area of the window shows: one of the tabbed pages, or
/// the whole-workspace Settings page that replaces it. Settings carries the
/// page to resume when it closes, so the view is one value instead of a
/// page plus a separate open flag that every consumer must consult in
/// tandem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceView {
    Page(Page),
    Settings {
        /// The page shown again when Settings closes.
        resume: Page,
    },
}

impl Default for WorkspaceView {
    fn default() -> Self {
        WorkspaceView::Page(Page::default())
    }
}

impl WorkspaceView {
    /// The page the workspace is on: the shown page, or — while Settings
    /// replaces it — the one Settings resumes to. This is the page the tab
    /// strip, window state, and autosave fingerprint speak about.
    pub fn page(self) -> Page {
        match self {
            WorkspaceView::Page(page) => page,
            WorkspaceView::Settings { resume } => resume,
        }
    }

    /// Whether the Settings page currently replaces the workspace.
    pub fn is_settings(self) -> bool {
        matches!(self, WorkspaceView::Settings { .. })
    }
}
