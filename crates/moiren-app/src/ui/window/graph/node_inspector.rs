use super::*;
use crate::service::{GraphEdit, InputSelection};
use controls::*;
use moiren_core::graph::NodeKind;
use moiren_engine::{
    meter::MeterSettings,
    processor::{ChannelMapperSettings, EqSettings},
    rack::{RackProcessor, RackSettings, RackSlot},
    recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};

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

const MAX_MATRIX_EDITOR_CHANNELS: usize = 8;

pub(in crate::ui::window) fn inspector(
    shell: &StudioShell,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let selected =
        snapshot.graph.graph.nodes().iter().find(|node| {
            shell.selected_node.as_deref() == Some(format!("{:?}", node.id()).as_str())
        });
    let mut view = panel("graph-inspector").child(heading("Inspector"));
    let Some(node) = selected else {
        return view
            .child(
                div()
                    .id("inspector-empty")
                    .child(label("Select a node to inspect its properties."))
                    .gpui_test_observe(),
            )
            .gpui_test_observe()
            .into_any_element();
    };
    let id = node.id();
    let entry = snapshot.graph.node(id).expect("service node metadata");
    view = view
        .child(
            div()
                .pb(px(12.))
                .border_b_1()
                .border_color(theme::BORDER)
                .child(heading(entry.label.clone()))
                .child(label(format!("{:?}", node.kind()))),
        )
        .child(label("Name"))
        .child(
            component::input::Input::new(&shell.node_name)
                .id("node-name")
                .bg(theme::SURFACE_3)
                .text_color(theme::TEXT)
                .border_color(theme::BORDER),
        )
        .child(button("rename-node", "Rename").on_click({
            let this = cx.entity();
            move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    let label = shell.node_name.read(cx).value().to_string();
                    shell.state.update(cx, |state, _| {
                        state.dispatch(AppRequest::EditGraph(GraphEdit::SetLabel {
                            node: id,
                            label,
                        }))
                    });
                    cx.notify();
                })
            }
        }))
        .child(detail("Channels", node.channels().to_string()));
    view = view
        .child(heading("Channel strip"))
        .child(detail("Fader", db(entry.mixer.gain)))
        .child(component::slider::Slider::new(&shell.mixer.faders[&id]))
        .child(super::super::mixer::node_controls(
            entry,
            node.channels(),
            cx,
        ));
    if node.kind() == NodeKind::Source {
        view = view
            .child(heading("Source"))
            .child(label(
                entry
                    .source
                    .as_ref()
                    .map(|source| crate::ui::state::source_name(snapshot, source))
                    .unwrap_or_else(|| "Not bound · choose a resource".into()),
            ))
            .child(
                button("unbind-node-source", "Clear binding")
                    .on_click(edit(cx.entity(), GraphEdit::ClearBinding(id))),
            )
            .child(
                button("bind-node-tone", "Tone generator · 440 Hz").on_click(edit(
                    cx.entity(),
                    GraphEdit::BindSource {
                        node: id,
                        source: InputSelection::Tone { frequency_hz: 440. },
                    },
                )),
            );
        if let Some(InputSelection::Tone { frequency_hz }) = entry.source {
            view = view
                .child(detail("Frequency", format!("{frequency_hz:.1} Hz")))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(button("tone-down", "−10 Hz").on_click(edit(
                            cx.entity(),
                            GraphEdit::BindSource {
                                node: id,
                                source: InputSelection::Tone {
                                    frequency_hz: (frequency_hz - 10.).max(1.),
                                },
                            },
                        )))
                        .child(button("tone-up", "+10 Hz").on_click(edit(
                            cx.entity(),
                            GraphEdit::BindSource {
                                node: id,
                                source: InputSelection::Tone {
                                    frequency_hz: (frequency_hz + 10.).min(23999.),
                                },
                            },
                        ))),
                );
        }
        for row in &snapshot.catalog.inputs {
            view = view.child(
                button(
                    SharedString::from(format!("bind-input-{}", row.endpoint_id)),
                    row.name.clone(),
                )
                .variant(ButtonVariant::Ghost)
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::BindSource {
                        node: id,
                        source: InputSelection::Physical {
                            endpoint_id: row.endpoint_id.clone(),
                        },
                    },
                )),
            );
        }
        for row in &snapshot.catalog.processes {
            view = view.child(
                button(
                    SharedString::from(format!("bind-process-{}", row.pid)),
                    row.name.clone(),
                )
                .variant(ButtonVariant::Ghost)
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::BindSource {
                        node: id,
                        source: InputSelection::Process {
                            pid: row.pid,
                            creation_time_100ns: row.creation_time_100ns,
                        },
                    },
                )),
            );
        }
        view = view.child(
            button("refresh-node-resources", "Refresh sources").on_click({
                let this = cx.entity();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::RefreshCatalog));
                        cx.notify();
                    })
                }
            }),
        );
    }
    if node.kind() == NodeKind::Sink {
        view = view.child(heading("Output device")).child(label(
            entry
                .output_endpoint_id
                .as_ref()
                .map(|endpoint| crate::ui::state::output_name(snapshot, endpoint))
                .unwrap_or_else(|| "Not bound · choose an output".into()),
        ));
        view = view.child(
            button("unbind-node-output", "Clear binding")
                .on_click(edit(cx.entity(), GraphEdit::ClearBinding(id))),
        );
        for row in &snapshot.catalog.outputs {
            view = view.child(
                button(
                    SharedString::from(format!("bind-output-{}", row.endpoint_id)),
                    row.name.clone(),
                )
                .variant(ButtonVariant::Ghost)
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::BindOutput {
                        node: id,
                        endpoint_id: row.endpoint_id.clone(),
                    },
                )),
            );
        }
    }
    if node.kind() == NodeKind::Gain {
        view = view
            .child(heading("Gain"))
            .child(detail("Requested", db(entry.gain)))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(button("node-gain-down", "−1 dB").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetGain {
                            node: id,
                            gain: entry.gain * 10f64.powf(-1. / 20.),
                        },
                    )))
                    .child(button("node-gain-up", "+1 dB").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetGain {
                            node: id,
                            gain: if entry.gain == 0. {
                                0.01
                            } else {
                                (entry.gain * 10f64.powf(1. / 20.)).min(16.)
                            },
                        },
                    )))
                    .child(
                        button("node-unity", "Unity")
                            .on_click(edit(cx.entity(), GraphEdit::SetGain { node: id, gain: 1. })),
                    ),
            )
            .child(
                button("node-silence", "Silence")
                    .on_click(edit(cx.entity(), GraphEdit::SetGain { node: id, gain: 0. })),
            );
    }
    if node.kind() == NodeKind::Pan {
        view = view
            .child(heading("Stereo controls"))
            .child(detail("Balance", format!("{:+.2}", entry.pan)))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(button("node-left", "Left").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetPan {
                            node: id,
                            pan: (entry.pan - 0.1).max(-1.),
                        },
                    )))
                    .child(
                        button("node-center", "Center")
                            .on_click(edit(cx.entity(), GraphEdit::SetPan { node: id, pan: 0. })),
                    )
                    .child(button("node-right", "Right").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetPan {
                            node: id,
                            pan: (entry.pan + 0.1).min(1.),
                        },
                    ))),
            )
            .child(stereo_controls(entry, cx));
    }
    if node.kind() == NodeKind::Compressor {
        view = view.child(heading("Compressor")).child(label(
            "Linked peak detector · Enter or leave a field to apply",
        ));
        for parameter in crate::service::CompressorParameter::ALL {
            view = view.child(
                div()
                    .id(format!("compressor-{:?}", parameter.id()))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(detail(
                        parameter.label(),
                        parameter.display(entry.compressor.value(parameter)),
                    ))
                    .child(
                        component::input::Input::new(&shell.compressor.inputs[&(id, parameter)])
                            .id(format!("compressor-input-{}", parameter.id().0))
                            .bg(theme::SURFACE_3)
                            .text_color(theme::TEXT)
                            .border_color(theme::BORDER),
                    )
                    .child(component::slider::Slider::new(
                        &shell.compressor.sliders[&(id, parameter)],
                    ))
                    .gpui_test_observe(),
            );
        }
    }
    view = view.child(builtin_controls(id, node, &entry.builtin, cx));
    view = view.child(node_observation(shell, node, cx));
    view = view.child(heading("Ports"));
    for (index, port) in node.inputs().iter().enumerate() {
        view = view.child(detail(
            &format!("Input {}", index + 1),
            format!("{} ch", port.channels()),
        ));
    }
    for (index, port) in node.outputs().iter().enumerate() {
        view = view.child(detail(
            &format!("Output {}", index + 1),
            format!("{} ch", port.channels()),
        ));
    }
    if node.kind() == NodeKind::Bus {
        view = view.child(
            button("add-bus-input", "Add bus input")
                .on_click(edit(cx.entity(), GraphEdit::AddBusInput(id))),
        );
        if let Some(port) = node.inputs().last() {
            view = view.child(
                button("remove-bus-input", "Remove last input").on_click(edit(
                    cx.entity(),
                    GraphEdit::RemoveBusInput {
                        node: id,
                        port: port.id(),
                    },
                )),
            );
        }
    }
    view.child(
        button("remove-node", "Delete node")
            .variant(ButtonVariant::Danger)
            .on_click(edit(cx.entity(), GraphEdit::RemoveNode(id))),
    )
    .gpui_test_observe()
    .into_any_element()
}

fn node_observation(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = shell.state.read(cx);
    let graph = &state.snapshot.graph;
    let active = state.snapshot.phase == crate::service::SessionPhase::Running
        && graph.active_revision == Some(graph.candidate_revision);
    let managed = state
        .snapshot
        .signal_topics
        .node_levels
        .contains_key(&node.id());
    let eligible = active && managed;
    let status = if eligible {
        shell.node_meters.status(node.id()).label().to_string()
    } else if managed {
        "Waiting for Apply graph".to_string()
    } else {
        "Unavailable for this node".to_string()
    };
    let mut view = div()
        .id("node-observation")
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(heading("Runtime observation"))
        .child(
            div()
                .id("node-observation-status")
                .child(detail("Status", status))
                .gpui_test_observe(),
        );
    if !active && managed {
        view = view.child(label(
            "The candidate graph is not the confirmed runtime. Apply graph to observe this node.",
        ));
    } else if !managed {
        view = view.child(label(
            "This node has no managed level topic in the current service projection.",
        ));
    }
    if let Some(levels) = eligible
        .then(|| shell.node_meters.channel_levels(node.id()))
        .flatten()
    {
        let mut readings = div().id("node-observation-levels").flex().flex_col();
        for (channel, level) in levels.iter().enumerate() {
            readings = readings
                .child(detail(
                    &format!("Peak {}", channel + 1),
                    level_db(level.peak),
                ))
                .child(detail(&format!("RMS {}", channel + 1), level_db(level.rms)));
        }
        view = view.child(readings);
    }
    if !active {
        view = view.child(
            div()
                .id("node-observation-candidate")
                .child(label("Candidate graph · audio owner unchanged"))
                .gpui_test_observe(),
        );
    }
    view.into_any_element()
}

fn level_db(value: f32) -> String {
    if value <= 0. {
        "-inf dBFS".into()
    } else {
        format!("{:+.1} dBFS", 20. * value.log10())
    }
}

fn builtin_controls(
    id: moiren_core::graph::NodeId,
    node: &moiren_core::graph::Node,
    state: &crate::service::BuiltinState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut view = div().id("builtin-controls").flex().flex_col().gap(px(8.));
    match state {
        crate::service::BuiltinState::Eq(settings) => {
            let settings = *settings;
            view = view
                .child(heading("Parametric EQ"))
                .child(label("One peaking band · apply to prepare the new kernel"))
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
                .child(heading("Gate"))
                .child(label("Linked downward expander"));
            for (key, label_text, value, min, max, step, update) in [
                (
                    "threshold",
                    "Threshold",
                    settings.threshold_db,
                    -120.,
                    0.,
                    1.,
                    0,
                ),
                ("attack", "Attack", settings.attack_ms, 0., 5_000., 5., 1),
                (
                    "release",
                    "Release",
                    settings.release_ms,
                    0.,
                    5_000.,
                    10.,
                    2,
                ),
                ("hold", "Hold", settings.hold_ms, 0., 5_000., 10., 3),
                ("range", "Range", settings.range_db, -120., 0., 1., 4),
            ] {
                let entry = *settings;
                view = view.child(builtin_step(
                    BuiltinStep {
                        key: format!("gate-{key}"),
                        label_text,
                        value,
                        suffix: "",
                        min,
                        max,
                        step,
                        node: id,
                        build: move |value| {
                            let mut next = entry;
                            match update {
                                0 => next.threshold_db = value,
                                1 => next.attack_ms = value,
                                2 => next.release_ms = value,
                                3 => next.hold_ms = value,
                                _ => next.range_db = value,
                            }
                            crate::service::BuiltinState::Gate(next)
                        },
                    },
                    cx,
                ));
            }
        }
        crate::service::BuiltinState::Limiter(settings) => {
            view = view
                .child(heading("Limiter"))
                .child(label("Zero-latency peak limiter"));
            for (key, label_text, value, min, max, step, update) in [
                (
                    "threshold",
                    "Threshold",
                    settings.threshold_db,
                    -60.,
                    0.,
                    1.,
                    0,
                ),
                (
                    "release",
                    "Release",
                    settings.release_ms,
                    0.,
                    5_000.,
                    10.,
                    1,
                ),
                (
                    "input-gain",
                    "Input gain",
                    settings.input_gain_db,
                    -60.,
                    60.,
                    1.,
                    2,
                ),
            ] {
                let entry = *settings;
                view = view.child(builtin_step(
                    BuiltinStep {
                        key: format!("limiter-{key}"),
                        label_text,
                        value,
                        suffix: "",
                        min,
                        max,
                        step,
                        node: id,
                        build: move |value| {
                            let mut next = entry;
                            match update {
                                0 => next.threshold_db = value,
                                1 => next.release_ms = value,
                                _ => next.input_gain_db = value,
                            }
                            crate::service::BuiltinState::Limiter(next)
                        },
                    },
                    cx,
                ));
            }
        }
        crate::service::BuiltinState::Delay(settings) => {
            view = view
                .child(heading("Delay"))
                .child(label("Bounded feedback delay"));
            for (key, label_text, value, min, max, step, update) in [
                (
                    "time",
                    "Time",
                    settings.delay_ms,
                    0.,
                    settings.max_delay_ms,
                    10.,
                    0,
                ),
                (
                    "feedback",
                    "Feedback",
                    settings.feedback,
                    -0.999,
                    0.999,
                    0.05,
                    1,
                ),
                ("mix", "Wet mix", settings.mix, 0., 1., 0.05, 2),
            ] {
                let entry = *settings;
                view = view.child(builtin_step(
                    BuiltinStep {
                        key: format!("delay-{key}"),
                        label_text,
                        value,
                        suffix: "",
                        min,
                        max,
                        step,
                        node: id,
                        build: move |value| {
                            let mut next = entry;
                            match update {
                                0 => next.delay_ms = value,
                                1 => next.feedback = value,
                                _ => next.mix = value,
                            }
                            crate::service::BuiltinState::Delay(next)
                        },
                    },
                    cx,
                ));
            }
        }
        crate::service::BuiltinState::ChannelMapper(settings) => {
            let mapping = if settings.mapping.is_empty() && settings.matrix.is_empty() {
                "Identity routing"
            } else if !settings.matrix.is_empty() {
                "Weighted matrix"
            } else {
                "Explicit channel map"
            };
            view = view
                .child(heading("Channel mapper"))
                .child(detail("Mode", mapping))
                .child(label("Static routing is prepared before audio starts."));
            let input_channels = node.inputs().first().map_or(0, |port| port.channels());
            let output_channels = node.channels();
            if input_channels == 2 && output_channels == 2 {
                view = view.child(button("channel-map-swap", "Swap L/R").on_click(edit(
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
            view = view.child(button("channel-map-identity", "Identity").on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::ChannelMapper(
                        ChannelMapperSettings::default(),
                    ),
                },
            )));
            if input_channels <= MAX_MATRIX_EDITOR_CHANNELS
                && output_channels <= MAX_MATRIX_EDITOR_CHANNELS
            {
                let matrix = channel_mapper_matrix(settings, input_channels, output_channels);
                if settings.matrix.is_empty() {
                    view = view.child(
                        button("channel-map-edit-matrix", "Edit weighted matrix").on_click(edit(
                            cx.entity(),
                            GraphEdit::SetBuiltin {
                                node: id,
                                state: crate::service::BuiltinState::ChannelMapper(
                                    ChannelMapperSettings {
                                        mapping: Vec::new(),
                                        matrix: matrix.clone(),
                                    },
                                ),
                            },
                        )),
                    );
                } else {
                    view = view
                        .child(heading("Weight matrix"))
                        .child(detail(
                            "Layout",
                            format!("{} outputs x {} inputs", output_channels, input_channels),
                        ))
                        .child(label(
                            "Adjust one output row at a time. Values are clamped to -1..1.",
                        ));
                    for output in 0..output_channels {
                        let mut row = div()
                            .id(format!("channel-map-matrix-row-{output}"))
                            .flex()
                            .items_center()
                            .gap(px(4.));
                        row = row.child(label(format!("Out {}", output + 1)));
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
                                    .gap(px(2.))
                                    .child(label(format!("In {}", input + 1)))
                                    .child(
                                        button(
                                            format!(
                                                "channel-map-matrix-out-{output}-in-{input}-up"
                                            ),
                                            "+",
                                        )
                                        .on_click(edit(cx.entity(), up)),
                                    )
                                    .child(detail("", format!("{value:+.2}")))
                                    .child(
                                        button(
                                            format!(
                                                "channel-map-matrix-out-{output}-in-{input}-down"
                                            ),
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
                    "Matrix editor is limited to 8 input and output channels; use a project file for larger layouts.",
                ));
            }
        }
        crate::service::BuiltinState::Recorder(settings) => {
            view = view
                .child(heading("Recorder"))
                .child(detail(
                    "Buffer",
                    format!("{} frames", settings.capacity_frames),
                ))
                .child(
                    button("recorder-expand", "Add 1 second").on_click(edit(
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
                .child(button("recorder-shrink", "Remove 1 second").on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::Recorder(RecorderSettings {
                            capacity_frames: settings.capacity_frames.saturating_sub(48_000).max(1),
                        }),
                    },
                )));
        }
        crate::service::BuiltinState::Meter(settings) => {
            view = view
                .child(heading("Level meter"))
                .child(detail(
                    "Window",
                    format!("{} frames", settings.window_frames),
                ))
                .child(
                    button("meter-window", "Use 2048 frame window").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetBuiltin {
                            node: id,
                            state: crate::service::BuiltinState::Meter(MeterSettings {
                                window_frames: 2048,
                            }),
                        },
                    )),
                );
        }
        crate::service::BuiltinState::SpectrumAnalyzer(settings) => {
            view = view
                .child(heading("Spectrum analyzer"))
                .child(detail("FFT", settings.fft_size.to_string()))
                .child(detail("Hop", settings.hop_frames.to_string()))
                .child(button("spectrum-size", "Use 2048 point FFT").on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::SpectrumAnalyzer(SpectrumSettings {
                            fft_size: 2048,
                            hop_frames: 1024,
                        }),
                    },
                )));
        }
        crate::service::BuiltinState::Rack(settings) => {
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
                .child(heading("Processor rack"))
                .child(detail("Stages", settings.slots.len().to_string()))
                .child(button("rack-add-gain", "Add gain stage").on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::Rack(RackSettings {
                            slots: next_slots,
                            routes: next_routes,
                        }),
                    },
                )))
                .child(button("rack-clear", "Clear stages").on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::Rack(RackSettings {
                            slots: Vec::new(),
                            routes: settings.routes.clone(),
                        }),
                    },
                )));
        }
        crate::service::BuiltinState::None => {}
    }
    if matches!(
        node.kind(),
        NodeKind::Recorder | NodeKind::Meter | NodeKind::SpectrumAnalyzer
    ) {
        view = view.child(label(
            "Managed observation becomes available after Apply graph.",
        ));
    }
    view.into_any_element()
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

fn builtin_step(
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
        .gap(px(6.))
        .child(label(label_text))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(button(format!("{key}-down"), "−").on_click({
                    let this = this.clone();
                    let build = build_down;
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                                    node,
                                    state: build((value - step).clamp(min, max)),
                                }))
                            });
                            cx.notify();
                        })
                    }
                }))
                .child(detail("", format!("{value:.2} {suffix}")))
                .child(button(format!("{key}-up"), "+").on_click({
                    let this = this.clone();
                    let build = build_up;
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(GraphEdit::SetBuiltin {
                                    node,
                                    state: build((value + step).clamp(min, max)),
                                }))
                            });
                            cx.notify();
                        })
                    }
                })),
        )
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_node_name(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.node_name_selection == self.selected_node {
            return;
        }
        self.node_name_selection = self.selected_node.clone();
        let label = self
            .state
            .read(cx)
            .snapshot
            .graph
            .nodes
            .iter()
            .find(|entry| self.selected_node.as_deref() == Some(format!("{:?}", entry.id).as_str()))
            .map(|entry| entry.label.clone())
            .unwrap_or_default();
        self.node_name
            .update(cx, |input, cx| input.set_value(label, window, cx));
    }
}
