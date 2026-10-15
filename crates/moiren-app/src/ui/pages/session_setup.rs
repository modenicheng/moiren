//! Session I/O panel: source mode, device pickers, refresh — ported from
//! session-setup.slint. Uses a lightweight dropdown built from base parts.

use gpui_kit::prelude::*;
use gpui_kit::*;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

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
        gpui_kit::base::Button::new(self.id)
            .disabled(self.disabled)
            .accessibility_label(format!("{}: {}", self.label, self.value))
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
            .when_some(self.on_click.filter(|_| !self.disabled), |s, on_click| {
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
                    .min_w_0()
                    .truncate()
                    .font_family(theme::FONT_UI)
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

#[cfg(all(test, feature = "gpui-test"))]
mod tests {
    use crate::ui::ActionButton;
    use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Window, div, point, px};
    use std::{cell::Cell, rc::Rc};

    struct DisabledButton(Rc<Cell<usize>>);

    impl Render for DisabledButton {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.0.clone();
            div().size_full().child(
                ActionButton::new("disabled", "Disabled")
                    .disabled(true)
                    .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
            )
        }
    }

    #[gpui_kit::test]
    fn disabled_action_does_not_dispatch(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view(|_, _| DisabledButton(clicks.clone()));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_click(point(px(15.), px(15.)), gpui_kit::Modifiers::default());
        assert_eq!(
            clicks.get(),
            0,
            "disabled actions must never reach the service"
        );
    }
}
