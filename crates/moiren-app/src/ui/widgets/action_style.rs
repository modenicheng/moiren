//! Paint rules shared by the action family. Selection remains caller-owned.

use gpui_kit::prelude::*;
use gpui_kit::{Hsla, StatefulInteractiveElement, Styled, px};

use crate::ui::theme::controls as t;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum SampleState {
    #[default]
    Default,
    Hover,
    Pressed,
    Focus,
    Disabled,
}

#[derive(Clone, Copy)]
pub(crate) struct Paint {
    pub bg: Hsla,
    pub fg: Hsla,
    pub border: Hsla,
}

impl Paint {
    pub const fn new(bg: Hsla, fg: Hsla, border: Hsla) -> Self {
        Self { bg, fg, border }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Skin {
    pub normal: Paint,
    pub hover: Paint,
    pub pressed: Paint,
    pub disabled: Paint,
    pub focus: Paint,
}

impl Skin {
    pub fn paint(self, disabled: bool, sample: Option<SampleState>) -> Paint {
        if disabled {
            return self.disabled;
        }
        match sample.unwrap_or_default() {
            SampleState::Hover => self.hover,
            SampleState::Pressed => self.pressed,
            SampleState::Focus => self.focus,
            SampleState::Disabled => self.disabled,
            SampleState::Default => self.normal,
        }
    }
}

pub(crate) fn styled_control<T>(
    control: T,
    skin: Skin,
    disabled: bool,
    sample: Option<SampleState>,
) -> T
where
    T: Styled + StatefulInteractiveElement + FluentBuilder,
{
    let paint = skin.paint(disabled, sample);
    control
        .flex_none()
        .relative()
        .rounded(px(t::RADIUS))
        .bg(paint.bg)
        .text_color(paint.fg)
        .border_1()
        .border_color(paint.border)
        .when(matches!(sample, Some(SampleState::Focus)), |s| {
            s.border_2().border_color(t::FOCUS)
        })
        .when(!disabled && sample.is_none(), |s| {
            s.cursor_pointer()
                .hover(move |s| {
                    s.bg(skin.hover.bg)
                        .text_color(skin.hover.fg)
                        .border_color(skin.hover.border)
                })
                .active(move |s| {
                    s.bg(skin.pressed.bg)
                        .text_color(skin.pressed.fg)
                        .border_color(skin.pressed.border)
                })
                .focus_visible(move |s| {
                    s.bg(skin.focus.bg)
                        .text_color(skin.focus.fg)
                        .border_2()
                        .border_color(t::FOCUS)
                })
        })
}
