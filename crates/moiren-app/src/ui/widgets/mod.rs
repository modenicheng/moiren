//! Shared widgets: window buttons, workspace tabs, statusbar text helpers.

pub mod icon;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

use gpui_kit::prelude::*;
use gpui_kit::*;
use icon::icon_tinted;

use super::theme;

/// Hoverable, focusless window-control button (minimize/maximize/close).
pub fn window_button(
    id: &'static str,
    icon_name: &'static str,
    danger: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let normal = if danger {
        theme::DANGER
    } else {
        theme::TEXT_MUTED
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .size(px(34.))
        .items_center()
        .justify_center()
        .when(danger, |s| s.hover(|s| s.bg(theme::DANGER)))
        .when(!danger, |s| s.hover(|s| s.bg(theme::SURFACE_4)))
        .child(icon_tinted(icon_name, normal, 13.))
        .on_click(move |event, window, cx| on_click(event, window, cx))
}

/// One workspace tab in the application toolbar (Graph/Mixer/…).
#[derive(IntoElement)]
pub struct WorkspaceTab {
    id: SharedString,
    label: &'static str,
    icon: &'static str,
    active: bool,
    enabled: bool,
    on_click: Option<ClickHandler>,
}

impl WorkspaceTab {
    pub fn new(id: impl Into<SharedString>, label: &'static str, icon: &'static str) -> Self {
        Self {
            id: id.into(),
            label,
            icon,
            active: false,
            enabled: true,
            on_click: None,
        }
    }
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
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

impl RenderOnce for WorkspaceTab {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let fg = if self.active {
            theme::PRIMARY
        } else {
            theme::TEXT_MUTED
        };
        div()
            .id(self.id.clone())
            .flex()
            .flex_none()
            .h_full()
            .px_3()
            .gap_2()
            .items_center()
            .justify_center()
            .relative()
            .when(!self.enabled, |s| s.opacity(0.42))
            .when(self.enabled, |s| {
                s.hover(|s| s.bg(theme::SURFACE_3))
                    .when_some(self.on_click, |s, on_click| {
                        s.cursor_pointer()
                            .on_click(move |event, window, cx| on_click(event, window, cx))
                    })
            })
            .child(icon_tinted(self.icon, fg, 14.))
            .child(
                div()
                    .text_size(px(theme::TEXT_SM))
                    .text_color(fg)
                    .child(self.label),
            )
            .when(self.active, |s| {
                s.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(px(2.))
                        .bg(theme::PRIMARY),
                )
            })
    }
}

/// Small mono-font status text for the engine status bar.
pub fn status_text(value: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(theme::TEXT_XS))
        .text_color(color)
        .child(value.into())
}
