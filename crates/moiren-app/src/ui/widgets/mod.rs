//! Shared native components and status text helpers.

pub mod action_assets;
pub(super) mod action_style;
pub mod actions;
pub mod meters;
pub mod navigation;

#[cfg(all(test, feature = "gpui-test"))]
mod action_tests;

use super::theme;
use gpui_kit::{Div, Hsla, ParentElement, SharedString, Styled, div, px};

/// Small mono-font status text for the engine status bar.
pub fn status_text(value: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(theme::TEXT_XS))
        .text_color(color)
        .child(value.into())
}
