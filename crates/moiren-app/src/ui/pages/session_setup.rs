//! Session I/O panel: source mode, device pickers, refresh — ported from
//! session-setup.slint. Uses a lightweight dropdown built from base parts.

use gpui_kit::prelude::*;
use gpui_kit::*;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

use crate::service::{InputSelection, RunLimit, SessionSpec};

use crate::ui::theme;

/// A picker row: label + current value; click cycles through options.
/// (A full dropdown lands with focus/popover work; cycling preserves all
/// selection semantics and stays fully testable.)
#[derive(IntoElement)]
pub struct PickerRow {
    id: SharedString,
    label: &'static str,
    value: SharedString,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl PickerRow {
    pub fn new(
        id: impl Into<SharedString>,
        label: &'static str,
        value: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label,
            value: value.into(),
            disabled: false,
            on_click: None,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for PickerRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_between()
            .h(px(30.))
            .px(px(theme::SPACE_3))
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::SURFACE_3)
            .border_1()
            .border_color(theme::BORDER)
            .when(!self.disabled, |s| {
                s.cursor_pointer().hover(|s| s.bg(theme::SURFACE_4))
            })
            .when(self.disabled, |s| s.opacity(0.55))
            .when_some(self.on_click, |s, on_click| {
                s.on_click(move |e, w, cx| on_click(e, w, cx))
            })
            .child(
                div()
                    .text_size(px(theme::TEXT_SM))
                    .text_color(theme::TEXT_MUTED)
                    .child(self.label.to_string()),
            )
            .child(
                div()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(theme::TEXT_SM))
                    .text_color(if self.disabled {
                        theme::TEXT_FAINT
                    } else {
                        theme::TEXT
                    })
                    .child(self.value),
            )
    }
}

/// Section heading with tracking, ported from SectionHeading.
pub fn section_heading(title: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .h(px(24.))
        .items_center()
        .text_size(px(theme::TEXT_XS))
        .text_color(theme::TEXT_FAINT)
        .child(title.to_string())
}

/// Primary studio button (StudioButton port).
#[derive(IntoElement)]
pub struct StudioButton {
    id: SharedString,
    label: SharedString,
    disabled: bool,
    active: bool,
    on_click: Option<ClickHandler>,
}

impl StudioButton {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
            active: false,
            on_click: None,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for StudioButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let fg = if self.active {
            theme::PRIMARY
        } else {
            theme::TEXT
        };
        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .h(px(30.))
            .px(px(theme::SPACE_3))
            .rounded(px(theme::RADIUS_SM))
            .bg(if self.active {
                theme::soft(theme::PRIMARY)
            } else {
                theme::SURFACE_3
            })
            .border_1()
            .border_color(if self.active {
                theme::PRIMARY
            } else {
                theme::BORDER
            })
            .text_size(px(theme::TEXT_SM))
            .text_color(fg)
            .when(!self.disabled, |s| {
                s.cursor_pointer().hover(|s| s.bg(theme::SURFACE_4))
            })
            .when(self.disabled, |s| s.opacity(0.55))
            .when_some(self.on_click, |s, on_click| {
                s.on_click(move |e, w, cx| on_click(e, w, cx))
            })
            .child(self.label)
    }
}

/// Placeholder spec builder so the shell can start sessions once the setup
/// panel wires selection state (kept next to the ported semantics).
pub fn tone_spec(output_endpoint_id: String, seconds: u16, gain: f64, pan: f64) -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id,
        limit: RunLimit::Seconds(seconds),
        gain,
        pan,
        max_block_frames: 256,
    }
}
