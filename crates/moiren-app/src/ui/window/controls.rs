//! Small, shared workstation building blocks. Actions always use ActionButton.
use rust_i18n::t;

use super::*;

pub(super) fn label(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(theme::TEXT_SM))
        .text_color(theme::TEXT_MUTED)
        .child(text.into())
}

pub(super) fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(theme::TEXT_LG))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

pub(super) fn badge(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(3.))
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::soft(color))
        .text_color(color)
        .text_size(px(11.))
        .child(text.into())
}

pub(super) fn detail(key: &str, value: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .justify_between()
        .gap(px(theme::SPACE_3))
        .py(px(6.))
        .child(label(key.to_string()))
        .child(
            div()
                .min_w_0()
                .max_w(px(190.))
                .text_size(px(12.))
                .text_color(theme::TEXT)
                .child(value.into()),
        )
}

pub(super) fn panel(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_none()
        .w(px(theme::INSPECTOR_WIDTH))
        // Fill the sidebar column so short content (the empty state) never
        // exposes the workspace behind it; overflow scrolls inside instead.
        .h_full()
        .p(px(theme::SPACE_4))
        .gap(px(theme::SPACE_3))
        .bg(theme::SURFACE)
        .border_l_1()
        .border_color(theme::BORDER)
        .overflow_y_scroll()
}

pub(super) fn button(id: impl Into<ElementId>, text: impl Into<SharedString>) -> ActionButton {
    ActionButton::new(id, text).size(ControlSize::Small)
}

pub(super) fn db(gain: f64) -> String {
    if gain == 0. {
        "−∞ dB".into()
    } else {
        format!("{:+.1} dB", 20. * gain.log10())
    }
}

pub(super) fn edit(
    this: Entity<StudioShell>,
    edit: crate::service::GraphEdit,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        this.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(edit.clone()))
            });
            cx.notify();
        })
    }
}

pub(super) fn stereo_controls(
    node: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> Div {
    use crate::service::GraphEdit;
    div()
        .flex()
        .gap(px(6.))
        .child(
            button("mix-mono", t!("Mono"))
                .variant(if node.mono {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::SetMono {
                        node: node.id,
                        mono: !node.mono,
                    },
                )),
        )
        .child(
            button("mix-swap", t!("Swap"))
                .variant(if node.swap {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::SetSwap {
                        node: node.id,
                        swap: !node.swap,
                    },
                )),
        )
}
