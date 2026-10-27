//! gpui-kit presentation layer. The service runtime owns audio; this module
//! only projects snapshots into windows and dispatches [`AppRequest`]s back.
mod app_view;
mod branding;
mod component_gallery;
pub mod monitoring;
mod pages;
pub mod presentation;
mod scroll_policy;
mod state;
mod theme;
#[cfg(windows)]
#[allow(unsafe_code)]
mod touchpad;
mod tray;
mod widgets;
mod window;

pub mod signals;
pub use component_gallery::run_component_preview;

pub use widgets::action_assets::ActionIcon;
pub use widgets::actions::{
    ActionButton, ButtonVariant, ControlSize, Density, IconButton, IconIntent, ToggleIconButton,
};
pub use widgets::navigation::{WindowAction, WindowControlButton, WorkspaceTab};

pub use pages::Page;
pub use state::{SelectionState, UiState};
pub use window::PageCell;
#[cfg(feature = "gpui-test")]
pub use window::run_ui_capture;
#[cfg(feature = "gpui-test")]
pub use window::shell_for_tests;
pub use window::{WindowPresentation, run};
#[cfg(any(test, feature = "gpui-test"))]
pub use window::{finish_service, initial_projection, spawn_service, wait_for, wait_until};
#[cfg(feature = "gpui-test")]
pub use window::{settle_panels, shell_with_state, test_shell};
