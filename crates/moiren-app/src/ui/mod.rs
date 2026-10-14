//! gpui-kit presentation layer. The service runtime owns audio; this module
//! only projects snapshots into windows and dispatches [`AppRequest`]s back.
mod app_view;
mod state;
mod window;

pub mod signals;

pub use state::{SelectionState, UiState};
pub use window::{Page, WindowPresentation, run};
