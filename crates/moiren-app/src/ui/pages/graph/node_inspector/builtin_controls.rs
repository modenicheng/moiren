//! The builtin processor's controls section: one assembly function per
//! [`crate::service::BuiltinState`] variant, dispatched by
//! [`builtin_controls`]. The channel-mapper's matrix helpers travel with
//! their section; the step rows themselves come from
//! [`super::builtin_fields`].

use rust_i18n::t;

use super::*;
use crate::service::GraphEdit;
use moiren_core::graph::NodeKind;
use moiren_engine::{
    meter::MeterSettings,
    processor::{ChannelMapperSettings, DelaySettings, EqSettings, GateSettings, LimiterSettings},
    rack::{RackProcessor, RackSettings, RackSlot},
    recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};

use super::builtin_fields::{
    BuiltinStep, builtin_field_step, builtin_step, delay_fields, gate_fields, limiter_fields,
};

const MAX_MATRIX_EDITOR_CHANNELS: usize = 8;

pub(super) fn builtin_controls(
    id: moiren_core::graph::NodeId,
    node: &moiren_core::graph::Node,
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
            view = eq_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::Gate(settings) => {
            view = gate_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::Limiter(settings) => {
            view = limiter_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::Delay(settings) => {
            view = delay_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::ChannelMapper(settings) => {
            view = channel_mapper_controls(view, id, node, settings, cx);
        }
        crate::service::BuiltinState::Recorder(settings) => {
            view = recorder_controls(view, id, settings, cx);
        }
        crate::service::BuiltinState::Meter(settings) => {
            view = meter_controls(view, id, settings, cx);
        }
        crate::service::BuiltinState::SpectrumAnalyzer(settings) => {
            view = spectrum_controls(view, id, settings, cx);
        }
        crate::service::BuiltinState::Rack(settings) => {
            view = rack_controls(view, id, settings, cx);
        }
        crate::service::BuiltinState::None => {}
    }
    if matches!(
        node.kind(),
        NodeKind::Recorder | NodeKind::Meter | NodeKind::SpectrumAnalyzer
    ) {
        view = view.child(label(t!(
            "Managed observation becomes available once the graph runs live."
        )));
    }
    view.into_any_element()
}

fn eq_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: EqSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Parametric EQ")))
        .child(label(t!(
            "One peaking band · apply to prepare the new kernel"
        )))
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
    view
}

fn gate_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: GateSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Gate")))
        .child(label(t!("Linked downward expander")));
    for field in gate_fields(settings) {
        view = view.child(builtin_field_step(
            "gate",
            settings,
            field,
            crate::service::BuiltinState::Gate,
            id,
            cx,
        ));
    }
    view
}

fn limiter_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: LimiterSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Limiter")))
        .child(label(t!("Zero-latency peak limiter")));
    for field in limiter_fields(settings) {
        view = view.child(builtin_field_step(
            "limiter",
            settings,
            field,
            crate::service::BuiltinState::Limiter,
            id,
            cx,
        ));
    }
    view
}

fn delay_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: DelaySettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Delay")))
        .child(label(t!("Bounded feedback delay")));
    for field in delay_fields(settings) {
        view = view.child(builtin_field_step(
            "delay",
            settings,
            field,
            crate::service::BuiltinState::Delay,
            id,
            cx,
        ));
    }
    view
}

fn channel_mapper_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    node: &moiren_core::graph::Node,
    settings: &ChannelMapperSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let mapping = if settings.mapping.is_empty() && settings.matrix.is_empty() {
        t!("Identity routing")
    } else if !settings.matrix.is_empty() {
        t!("Weighted matrix")
    } else {
        t!("Explicit channel map")
    };
    view = view
        .child(heading(t!("Channel mapper")))
        .child(detail(&t!("Mode"), mapping))
        .child(label(t!("Static routing is prepared before audio starts.")));
    let input_channels = node.inputs().first().map_or(0, |port| port.channels());
    let output_channels = node.channels();
    if input_channels == 2 && output_channels == 2 {
        view = view.child(button("channel-map-swap", t!("Swap L/R")).on_click(edit(
            cx.entity(),
            GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
                    mapping: vec![1, 0],
                    matrix: Vec::new(),
                }),
            },
        )));
    }
    view =
        view.child(
            button("channel-map-identity", t!("Identity")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::ChannelMapper(
                        ChannelMapperSettings::default(),
                    ),
                },
            )),
        );
    if input_channels <= MAX_MATRIX_EDITOR_CHANNELS && output_channels <= MAX_MATRIX_EDITOR_CHANNELS
    {
        let matrix = channel_mapper_matrix(settings, input_channels, output_channels);
        if settings.matrix.is_empty() {
            view = view.child(
                button("channel-map-edit-matrix", t!("Edit weighted matrix")).on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
                            mapping: Vec::new(),
                            matrix: matrix.clone(),
                        }),
                    },
                )),
            );
        } else {
            view = view
                .child(heading(t!("Weight matrix")))
                .child(detail(
                    &t!("Layout"),
                    t!(
                        "%{outputs} outputs x %{inputs} inputs",
                        outputs = output_channels,
                        inputs = input_channels
                    ),
                ))
                .child(label(t!(
                    "Adjust one output row at a time. Values are clamped to -1..1."
                )));
            for output in 0..output_channels {
                let mut row = div()
                    .id(format!("channel-map-matrix-row-{output}"))
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_1));
                row = row.child(label(t!("Out %{index}", index = output + 1)));
                for input in 0..input_channels {
                    let value = matrix[output][input];
                    let down = matrix_cell_edit(id, &matrix, output, input, -0.25);
                    let up = matrix_cell_edit(id, &matrix, output, input, 0.25);
                    row = row.child(
                        div()
                            .id(format!("channel-map-matrix-cell-{output}-{input}"))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(theme::SPACE_0_5))
                            .child(label(t!("In %{index}", index = input + 1)))
                            .child(
                                button(
                                    format!("channel-map-matrix-out-{output}-in-{input}-up"),
                                    "+",
                                )
                                .on_click(edit(cx.entity(), up)),
                            )
                            .child(detail("", format!("{value:+.2}")))
                            .child(
                                button(
                                    format!("channel-map-matrix-out-{output}-in-{input}-down"),
                                    "−",
                                )
                                .on_click(edit(cx.entity(), down)),
                            ),
                    );
                }
                view = view.child(row);
            }
        }
    } else {
        view = view.child(label(
            t!("Matrix editor is limited to 8 input and output channels; use a project file for larger layouts."),
        ));
    }
    view
}

fn recorder_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &RecorderSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Recorder")))
        .child(detail(
            &t!("Buffer"),
            format!("{} frames", settings.capacity_frames),
        ))
        .child(
            button("recorder-expand", t!("Add 1 second")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::Recorder(RecorderSettings {
                        capacity_frames: settings
                            .capacity_frames
                            .saturating_add(48_000)
                            .min(1_000_000),
                    }),
                },
            )),
        )
        .child(
            button("recorder-shrink", t!("Remove 1 second")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::Recorder(RecorderSettings {
                        capacity_frames: settings.capacity_frames.saturating_sub(48_000).max(1),
                    }),
                },
            )),
        );
    view
}

fn meter_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &MeterSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Level meter")))
        .child(detail(
            &t!("Window"),
            t!("%{frames} frames", frames = settings.window_frames).to_string(),
        ))
        .child(
            button("meter-window", t!("Use 2048 frame window")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::Meter(MeterSettings {
                        window_frames: 2048,
                    }),
                },
            )),
        );
    view
}

fn spectrum_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &SpectrumSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Spectrum analyzer")))
        .child(detail(&t!("FFT"), settings.fft_size.to_string()))
        .child(detail(&t!("Hop"), settings.hop_frames.to_string()))
        .child(
            button("spectrum-size", t!("Use 2048 point FFT")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::SpectrumAnalyzer(SpectrumSettings {
                        fft_size: 2048,
                        hop_frames: 1024,
                    }),
                },
            )),
        );
    view
}

fn rack_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &RackSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let next_slot_id = settings
        .slots
        .iter()
        .map(|slot| slot.id)
        .max()
        .map_or(0, |id| id.saturating_add(1));
    let mut next_slots = settings.slots.clone();
    next_slots.push(RackSlot {
        id: next_slot_id,
        bypass: false,
        processor: RackProcessor::Gain { initial: 1.0 },
    });
    let mut next_routes = settings.routes.clone();
    if let Some(route) = next_routes.first_mut() {
        route.slots.push(next_slot_id);
    }
    view = view
        .child(heading(t!("Processor rack")))
        .child(detail(&t!("Stages"), settings.slots.len().to_string()))
        .child(button("rack-add-gain", t!("Add gain stage")).on_click(edit(
            cx.entity(),
            GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::Rack(RackSettings {
                    slots: next_slots,
                    routes: next_routes,
                }),
            },
        )))
        .child(button("rack-clear", t!("Clear stages")).on_click(edit(
            cx.entity(),
            GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::Rack(RackSettings {
                    slots: Vec::new(),
                    routes: settings.routes.clone(),
                }),
            },
        )));
    view
}

fn channel_mapper_matrix(
    settings: &ChannelMapperSettings,
    input_channels: usize,
    output_channels: usize,
) -> Vec<Vec<f64>> {
    if !settings.matrix.is_empty() {
        return settings.matrix.clone();
    }
    (0..output_channels)
        .map(|output| {
            let source = settings.mapping.get(output).copied().unwrap_or(output);
            (0..input_channels)
                .map(|input| f64::from((input == source) as u8))
                .collect()
        })
        .collect()
}

fn matrix_cell_edit(
    node: moiren_core::graph::NodeId,
    matrix: &[Vec<f64>],
    output: usize,
    input: usize,
    delta: f64,
) -> GraphEdit {
    let mut matrix = matrix.to_vec();
    matrix[output][input] = (matrix[output][input] + delta).clamp(-1., 1.);
    GraphEdit::SetBuiltin {
        node,
        state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
            mapping: Vec::new(),
            matrix,
        }),
    }
}
