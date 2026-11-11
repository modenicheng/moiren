//! The builtin processor's table-driven step rows: the [`BuiltinFieldSpec`]
//! schema, the gate/limiter/delay field tables it populates, and the shared
//! ± step machinery ([`builtin_step`] / [`step_button`]) that dispatches one
//! rebuilt [`crate::service::BuiltinState`] per click.

use rust_i18n::t;

use super::*;
use crate::service::GraphEdit;
use moiren_engine::processor::{DelaySettings, GateSettings, LimiterSettings};

pub(super) struct BuiltinStep<F> {
    pub(super) key: String,
    pub(super) label_text: &'static str,
    pub(super) value: f64,
    pub(super) suffix: &'static str,
    pub(super) min: f64,
    pub(super) max: f64,
    pub(super) step: f64,
    pub(super) node: moiren_core::graph::NodeId,
    pub(super) build: F,
}

/// Field descriptor driving the table-driven builtin step rows; `set` applies
/// one field's new value, replacing the implicit per-index match protocol.
pub(super) struct BuiltinFieldSpec<S> {
    key: &'static str,
    label_text: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    set: fn(&mut S, f64),
}

pub(super) fn gate_fields(settings: GateSettings) -> [BuiltinFieldSpec<GateSettings>; 5] {
    [
        BuiltinFieldSpec {
            key: "threshold",
            label_text: "Threshold",
            value: settings.threshold_db,
            min: -120.,
            max: 0.,
            step: 1.,
            set: |next, value| next.threshold_db = value,
        },
        BuiltinFieldSpec {
            key: "attack",
            label_text: "Attack",
            value: settings.attack_ms,
            min: 0.,
            max: 5_000.,
            step: 5.,
            set: |next, value| next.attack_ms = value,
        },
        BuiltinFieldSpec {
            key: "release",
            label_text: "Release",
            value: settings.release_ms,
            min: 0.,
            max: 5_000.,
            step: 10.,
            set: |next, value| next.release_ms = value,
        },
        BuiltinFieldSpec {
            key: "hold",
            label_text: "Hold",
            value: settings.hold_ms,
            min: 0.,
            max: 5_000.,
            step: 10.,
            set: |next, value| next.hold_ms = value,
        },
        BuiltinFieldSpec {
            key: "range",
            label_text: "Range",
            value: settings.range_db,
            min: -120.,
            max: 0.,
            step: 1.,
            set: |next, value| next.range_db = value,
        },
    ]
}

pub(super) fn limiter_fields(settings: LimiterSettings) -> [BuiltinFieldSpec<LimiterSettings>; 3] {
    [
        BuiltinFieldSpec {
            key: "threshold",
            label_text: "Threshold",
            value: settings.threshold_db,
            min: -60.,
            max: 0.,
            step: 1.,
            set: |next, value| next.threshold_db = value,
        },
        BuiltinFieldSpec {
            key: "release",
            label_text: "Release",
            value: settings.release_ms,
            min: 0.,
            max: 5_000.,
            step: 10.,
            set: |next, value| next.release_ms = value,
        },
        BuiltinFieldSpec {
            key: "input-gain",
            label_text: "Input gain",
            value: settings.input_gain_db,
            min: -60.,
            max: 60.,
            step: 1.,
            set: |next, value| next.input_gain_db = value,
        },
    ]
}

pub(super) fn delay_fields(settings: DelaySettings) -> [BuiltinFieldSpec<DelaySettings>; 3] {
    [
        BuiltinFieldSpec {
            key: "time",
            label_text: "Time",
            value: settings.delay_ms,
            min: 0.,
            max: settings.max_delay_ms,
            step: 10.,
            set: |next, value| next.delay_ms = value,
        },
        BuiltinFieldSpec {
            key: "feedback",
            label_text: "Feedback",
            value: settings.feedback,
            min: -0.999,
            max: 0.999,
            step: 0.05,
            set: |next, value| next.feedback = value,
        },
        BuiltinFieldSpec {
            key: "mix",
            label_text: "Wet mix",
            value: settings.mix,
            min: 0.,
            max: 1.,
            step: 0.05,
            set: |next, value| next.mix = value,
        },
    ]
}

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
            suffix: "",
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
        .gap(px(theme::SPACE_1_5))
        .child(label(t!(label_text)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_1))
                .child(step_button(
                    format!("{key}-down"),
                    "−",
                    this.clone(),
                    node,
                    build_down,
                    (value - step).clamp(min, max),
                ))
                .child(detail("", format!("{value:.2} {suffix}")))
                .child(step_button(
                    format!("{key}-up"),
                    "+",
                    this,
                    node,
                    build_up,
                    (value + step).clamp(min, max),
                )),
        )
}

/// The shared ± step button: dispatches `build(next)` as one SetBuiltin edit.
fn step_button(
    key: String,
    text: &'static str,
    this: Entity<StudioShell>,
    node: moiren_core::graph::NodeId,
    build: impl Fn(f64) -> crate::service::BuiltinState + 'static,
    next: f64,
) -> impl IntoElement {
    button(key, text).on_click(move |_, _, cx| {
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
