//! gpui-kit presentation layer. The service runtime owns audio; this module
//! only projects snapshots into windows and dispatches [`AppRequest`]s back.
pub(crate) mod actions;
mod branding;
// Design-review surface with no production entry point; keeping it behind the
// feature (paired with the component_preview example's required-features)
// stops it from linking into ordinary application builds.
#[cfg(feature = "component-gallery")]
mod component_gallery;
pub mod monitoring;
mod observe;
mod pages;
pub mod presentation;
mod scroll_policy;
mod shared_frame;
mod shell;
mod state;
pub(crate) mod theme;
#[cfg(windows)]
#[allow(unsafe_code)]
mod touchpad;
mod tray;
mod widgets;

#[cfg(feature = "component-gallery")]
pub use component_gallery::run_component_preview;

pub use widgets::action_assets::ActionIcon;
pub use widgets::actions::{
    ActionButton, ButtonVariant, ControlSize, Density, IconButton, IconIntent, ToggleIconButton,
};
pub use widgets::navigation::{WindowAction, WindowControlButton, WorkspaceTab};

pub use pages::Page;
pub use pages::WorkspaceView;
pub use shell::PageCell;
#[cfg(feature = "gpui-test")]
pub use shell::run_ui_capture;
#[cfg(feature = "gpui-test")]
pub use shell::shell_for_tests;
pub use shell::{WindowPresentation, run};
#[cfg(any(test, feature = "gpui-test"))]
pub use shell::{finish_service, initial_projection, spawn_service, wait_for, wait_until};
#[cfg(feature = "gpui-test")]
pub use shell::{settle_panels, shell_with_state, test_shell};
pub use state::UiState;
