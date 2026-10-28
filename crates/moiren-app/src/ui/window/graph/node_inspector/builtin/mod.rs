//! Advanced controls for prepared builtin processors.
use super::*;

mod fields;
mod matrix;
mod step;
use fields::*;
mod utility;
use step::*;

struct BuiltinStep<F> {
    key: String,
    label_text: &'static str,
    value: f64,
    suffix: &'static str,
    min: f64,
    max: f64,
    step: f64,
    node: moiren_core::graph::NodeId,
    build: F,
}

/// Field descriptor driving the table-driven builtin step rows; `set` applies
/// one field's new value, replacing the implicit per-index match protocol.
struct BuiltinFieldSpec<S> {
    key: &'static str,
    label_text: &'static str,
    value: f64,
    suffix: &'static str,
    min: f64,
    max: f64,
    step: f64,
    set: fn(&mut S, f64),
}

pub(super) fn builtin_controls(
    id: moiren_core::graph::NodeId,
    _node: &moiren_core::graph::Node,
    state: &crate::service::BuiltinState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut view = div()
        .id("builtin-controls")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_2));
    match state {
        crate::service::BuiltinState::Eq(settings) => {
            let settings = *settings;
            view = view
                .child(heading(t!("Parametric EQ")))
                .child(label(t!("One peaking band")))
                .child(builtin_step(
                    BuiltinStep {
                        key: "eq-frequency".into(),
                        label_text: "Frequency",
                        value: settings.frequency_hz,
                        suffix: "Hz",
                        min: 20.,
                        max: 20_000.,
                        step: 100.,
                        node: id,
                        build: move |value| {
                            crate::service::BuiltinState::Eq(EqSettings {
                                frequency_hz: value,
                                ..settings
                            })
                        },
                    },
                    cx,
                ))
                .child(builtin_step(
                    BuiltinStep {
                        key: "eq-gain".into(),
                        label_text: "Gain",
                        value: settings.gain_db,
                        suffix: "dB",
                        min: -24.,
                        max: 24.,
                        step: 1.,
                        node: id,
                        build: move |value| {
                            crate::service::BuiltinState::Eq(EqSettings {
                                gain_db: value,
                                ..settings
                            })
                        },
                    },
                    cx,
                ))
                .child(builtin_step(
                    BuiltinStep {
                        key: "eq-q".into(),
                        label_text: "Q",
                        value: settings.q,
                        suffix: "",
                        min: 0.1,
                        max: 20.,
                        step: 0.1,
                        node: id,
                        build: move |value| {
                            crate::service::BuiltinState::Eq(EqSettings {
                                q: value,
                                ..settings
                            })
                        },
                    },
                    cx,
                ));
        }
        crate::service::BuiltinState::Gate(settings) => {
            view = view
                .child(heading(t!("Gate")))
                .child(label(t!("Linked downward expander")));
            for field in gate_fields(*settings) {
                view = view.child(builtin_field_step(
                    "gate",
                    *settings,
                    field,
                    crate::service::BuiltinState::Gate,
                    id,
                    cx,
                ));
            }
        }
        crate::service::BuiltinState::Limiter(settings) => {
            view = view
                .child(heading(t!("Limiter")))
                .child(label(t!("Zero-latency peak limiter")));
            for field in limiter_fields(*settings) {
                view = view.child(builtin_field_step(
                    "limiter",
                    *settings,
                    field,
                    crate::service::BuiltinState::Limiter,
                    id,
                    cx,
                ));
            }
        }
        crate::service::BuiltinState::Delay(settings) => {
            view = view
                .child(heading(t!("Delay")))
                .child(label(t!("Bounded feedback delay")));
            for field in delay_fields(*settings) {
                view = view.child(builtin_field_step(
                    "delay",
                    *settings,
                    field,
                    crate::service::BuiltinState::Delay,
                    id,
                    cx,
                ));
            }
        }
        crate::service::BuiltinState::ChannelMapper(settings) => {
            view = view.child(matrix::summary(settings));
        }
        crate::service::BuiltinState::Recorder(_)
        | crate::service::BuiltinState::Meter(_)
        | crate::service::BuiltinState::SpectrumAnalyzer(_)
        | crate::service::BuiltinState::Rack(_) => {
            view = view.child(utility::controls(id, state, cx));
        }
        crate::service::BuiltinState::None => {}
    }
    view.into_any_element()
}
