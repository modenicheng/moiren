//! Shared bounded step controls for prepared builtin parameters.
use super::*;

/// One table-driven step row for a `Copy` settings struct; `wrap` rebuilds the
/// [`BuiltinState`] variant after `set` applies the field's new value.
pub(super) fn builtin_field_step<S: Copy + 'static>(
    prefix: &str,
    settings: S,
    field: BuiltinFieldSpec<S>,
    wrap: fn(S) -> crate::service::BuiltinState,
    node: moiren_core::graph::NodeId,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let set = field.set;
    builtin_step(
        BuiltinStep {
            key: format!("{prefix}-{}", field.key),
            label_text: field.label_text,
            value: field.value,
            suffix: field.suffix,
            min: field.min,
            max: field.max,
            step: field.step,
            node,
            build: move |value| {
                let mut next = settings;
                set(&mut next, value);
                wrap(next)
            },
        },
        cx,
    )
}

pub(super) fn builtin_step(
    spec: BuiltinStep<impl Fn(f64) -> crate::service::BuiltinState + Clone + 'static>,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let BuiltinStep {
        key,
        label_text,
        value,
        suffix,
        min,
        max,
        step,
        node,
        build,
    } = spec;
    let this = cx.entity();
    let build_down = build.clone();
    let build_up = build;
    div()
        .id(key.clone())
        .flex()
        .items_center()
        .justify_between()
        .py(px(theme::SPACE_1))
        .gap(px(theme::SPACE_1))
        .child(label(t!(label_text)).flex_1().min_w_0())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(step_button(
                    format!("{key}-down"),
                    "−",
                    this.clone(),
                    node,
                    build_down,
                    (value - step).clamp(min, max),
                    value <= min,
                ))
                .child(
                    div()
                        .min_w(px(68.))
                        .text_right()
                        .text_size(px(theme::TEXT_SM))
                        .text_color(theme::TEXT)
                        .child(field_value(value, suffix)),
                )
                .child(step_button(
                    format!("{key}-up"),
                    "+",
                    this,
                    node,
                    build_up,
                    (value + step).clamp(min, max),
                    value >= max,
                )),
        )
        .gpui_test_observe()
}

/// Percent is presentation only; edits retain the engine's signed ratio units.
pub(super) fn field_value(value: f64, suffix: &str) -> String {
    match suffix {
        "%" => format!("{:.1} %", value * 100.),
        "Hz" | "ms" => format!("{value:.1} {suffix}"),
        "dB" => format!("{value:.1} {suffix}"),
        "" => format!("{value:.2}"),
        _ => format!("{value:.2} {suffix}"),
    }
}

/// The shared ± step button: dispatches `build(next)` as one SetBuiltin edit.
fn step_button(
    key: String,
    text: &'static str,
    this: Entity<StudioShell>,
    node: moiren_core::graph::NodeId,
    build: impl Fn(f64) -> crate::service::BuiltinState + 'static,
    next: f64,
    disabled: bool,
) -> impl IntoElement {
    button(key, text)
        .disabled(disabled)
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| {
                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                        node,
                        state: build(next),
                    }))
                });
                cx.notify();
            })
        })
}

#[cfg(test)]
mod tests {
    use super::field_value;

    #[::core::prelude::v1::test]
    fn readouts_include_units_and_preserve_signed_feedback() {
        assert_eq!(field_value(1100., "Hz"), "1100.0 Hz");
        assert_eq!(field_value(-18., "dB"), "-18.0 dB");
        assert_eq!(field_value(25., "ms"), "25.0 ms");
        assert_eq!(field_value(-0.999, "%"), "-99.9 %");
        assert_eq!(field_value(0.5, "%"), "50.0 %");
        assert_eq!(field_value(0.7, ""), "0.70");
    }
}
