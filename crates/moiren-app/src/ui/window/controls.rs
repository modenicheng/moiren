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

/// A [`detail`] row whose value side is a click-to-type editable readout.
pub(super) fn detail_editable(
    key: &str,
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    editing: bool,
    input: &Entity<component::input::InputState>,
    on_start: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .justify_between()
        .gap(px(theme::SPACE_3))
        .py(px(6.))
        .child(label(key.to_string()))
        .child(
            editable_value(id, text, editing, input, on_start, px(12.), TextAlign::Left)
                .min_w_0()
                .max_w(px(190.))
                .text_color(theme::TEXT),
        )
}

/// The click-to-type editor chrome: a transparent, borderless
/// [`component::input::Input`] with a single 1px underline drawn along the
/// bottom of its wrapper. The component's own chrome — filled background,
/// full border, corner radius, 10px/8px padding, fixed 32px height,
/// 0.875rem text at a fixed 1.25rem line height — is stripped so the text
/// sits exactly where the readout it replaced sat: the caller's font
/// cascades in, the size and line box (`phi` × font size, gpui's text
/// default) match the label, padding is zero, and the field spans the
/// wrapper's width with the requested alignment. The wrapper stops mouse
/// presses from propagating so a click inside the editor never reaches the
/// window root's outside-click commit. The editor box is observed under
/// `editable-editor` (keyed by the input state) so tests can assert the
/// no-jump geometry.
pub(super) fn underline_editor(
    input: &Entity<component::input::InputState>,
    text_size: Pixels,
    align: TextAlign,
) -> impl IntoElement {
    div()
        .id(("editable-editor", input.entity_id()))
        .relative()
        .w_full()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(1.))
                .bg(theme::BORDER),
        )
        .child(
            component::input::Input::new(input)
                .appearance(false)
                .px(px(0.))
                .py(px(0.))
                .h_auto()
                .text_size(text_size)
                .line_height(phi())
                .when(align == TextAlign::Center, |input| input.text_center())
                .when(align == TextAlign::Right, |input| input.text_right()),
        )
        .gpui_test_observe()
}

/// Click-to-type numeric readout shared by the mixer strip, the balance
/// label and the inspector rows: a plain text value until clicked, then an
/// underline-only inline editor (`underline_editor`) in the same box.
/// `text_size` and `align` style the label and the editor alike so the text
/// neither moves nor resizes when the editor opens. The helper owns no
/// parsing — the call site's `on_start` enters edit mode and the value
/// family's input subscription commits on Enter/blur. The site keeps its
/// layout styling and `gpui_test_observe` by chaining on the returned
/// `Stateful<Div>`.
pub(super) fn editable_value(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    editing: bool,
    input: &Entity<component::input::InputState>,
    on_start: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    text_size: Pixels,
    align: TextAlign,
) -> Stateful<Div> {
    if editing {
        return div()
            .id(id)
            .child(underline_editor(input, text_size, align));
    }
    div()
        .id(id)
        .cursor_pointer()
        .text_size(text_size)
        .when(align == TextAlign::Center, |label| label.text_center())
        .when(align == TextAlign::Right, |label| label.text_right())
        .child(text.into())
        .tooltip(|window, cx| {
            component::tooltip::Tooltip::new(t!("Click to type a value").into_owned())
                .build(window, cx)
        })
        .on_click(on_start)
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
