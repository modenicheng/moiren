//! gpui-kit presentation layer. The service runtime owns audio; this module
//! only projects snapshots into windows and dispatches [`AppRequest`]s back.
mod app_view;
mod pages;
mod state;
mod theme;
mod widgets;
mod window;

pub mod signals;

pub use pages::Page;
pub use state::{SelectionState, UiState};
pub use window::{WindowPresentation, run};
