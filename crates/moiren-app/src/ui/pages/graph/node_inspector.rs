use rust_i18n::t;

use super::*;
use crate::service::{GraphEdit, InputSelection};
use crate::ui::presentation::{db_text, dbfs_text, step_db};
use crate::ui::shell::mixer::{
    EditableValueFamily, gain_db_text, reset_clip_on_click, start_value_edit,
};
use crate::ui::widgets::fader::hslider;
use controls::*;
use moiren_core::graph::NodeKind;
use moiren_engine::{
    meter::MeterSettings,
    processor::{ChannelMapperSettings, DelaySettings, EqSettings, GateSettings, LimiterSettings},
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

/// Field descriptor driving the table-driven builtin step rows; `set` applies
/// one field's new value, replacing the implicit per-index match protocol.
struct BuiltinFieldSpec<S> {
    key: &'static str,
    label_text: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    set: fn(&mut S, f64),
}

const MAX_MATRIX_EDITOR_CHANNELS: usize = 8;

pub(crate) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let selected = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|node| shell.selection.node == Some(node.id()));
    let mut view =
        panel("graph-inspector", &shell.inspector.scroll).child(heading(t!("Inspector")));
    let Some(node) = selected else {
        let view = view
            .child(
                div()
                    .id("inspector-empty")
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(theme::SPACE_4))
                    .child(
                        div()
                            .id("inspector-empty-art")
                            .flex()
                            .flex_none()
                            .w(px(64.))
                            .h(px(64.))
                            .items_center()
                            .justify_center()
                            .rounded(px(theme::RADIUS_LG))
                            .bg(theme::SURFACE_3)
                            .child(
                                component::Icon::default()
                                    .path("icons/zoom-scan.svg")
                                    .size(px(28.))
                                    .text_color(theme::TEXT_FAINT),
                            )
                            .gpui_test_observe(),
                    )
                    .child(label(t!("Select a node to inspect its properties.")))
                    .gpui_test_observe(),
            )
            .gpui_test_observe();
        return panel_host("graph-inspector", &shell.inspector.scroll, view).into_any_element();
    };
    let id = node.id();
    let entry = snapshot.graph.node(id).expect("service node metadata");
    let display = crate::ui::presentation::display_label(entry);
    let editing = shell.inspector.node_name_editing == Some(id);
    // One inline name control: a plain heading until clicked, then the editor
    // in the same place. There is no separate rename row.
    let name_control: AnyElement = if editing {
        div()
            .id("node-name-editor")
            .flex()
            .items_center()
            .gap(px(theme::SPACE_1))
            .child(
                component::input::Input::new(&shell.inspector.node_name)
                    .id("node-name")
                    .bg(theme::SURFACE_3)
                    .text_color(theme::TEXT)
                    .border_color(theme::BORDER),
            )
            .child(button("node-name-confirm", "✓").on_click({
                let this = cx.entity();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.confirm_node_rename(cx));
                }
            }))
            .child(button("node-name-cancel", "✗").on_click({
                let this = cx.entity();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.cancel_node_rename(cx));
                }
            }))
            .into_any_element()
    } else {
        div()
            .id("node-name-display")
            .flex()
            .items_center()
            .gap(px(theme::SPACE_1_5))
            .cursor_pointer()
            .child(heading(display.clone()))
            .child(
                div()
                    .id("node-name-edit-hint")
                    .text_size(px(theme::TEXT_XS))
                    .text_color(theme::TEXT_FAINT)
                    .child(t!("rename")),
            )
            .on_click({
                let this = cx.entity();
                let initial = display.clone();
                move |_, window, cx| {
                    this.update(cx, |shell, cx| {
                        shell.inspector.node_name_editing = Some(id);
                        shell
                            .inspector
                            .node_name
                            .update(cx, |input, cx| input.set_value(initial.clone(), window, cx));
                        cx.notify();
                    });
                    let input = this.read(cx).inspector.node_name.clone();
                    let handle = input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                }
            })
            .gpui_test_observe()
            .into_any_element()
    };
    view = view
        .child(
            div()
                .pb(px(theme::SPACE_3))
                .border_b_1()
                .border_color(theme::BORDER)
                .child(name_control)
                .child(label(node_catalog::kind_title(node.kind()))),
        )
        .child(detail(&t!("Channels"), node.channels().to_string()));
    // The headline visualization (design 18 B4) leads the kind-specific
    // sections: the EQ's draggable control point and the dynamics family's
    // transfer curve render here.
    view = match node.kind() {
        NodeKind::Eq => view.child(headline::eq_section(
            id,
            entry,
            cx,
            headline::EQ_INSPECTOR_HEIGHT,
        )),
        NodeKind::Compressor | NodeKind::Gate | NodeKind::Limiter => view.child(
            headline::dynamics_section(node.kind(), entry, headline::DYNAMICS_INSPECTOR_HEIGHT),
        ),
        _ => view,
    };
    view = view
        .child(heading(t!("Channel strip")))
        .child(detail_editable(
            &t!("Fader"),
            SharedString::from(format!("inspector-fader-{id:?}")),
            db_text(entry.mixer.gain),
            shell.mixer.presentation.editing_value == Some(id),
            &shell.mixer.presentation.value_inputs[&id],
            start_value_edit(cx.entity(), EditableValueFamily::Fader, id, move |graph| {
                graph
                    .node(id)
                    .map(|node| gain_db_text(node.mixer.gain))
                    .unwrap_or_default()
            }),
        ))
        .child(hslider(
            &shell.mixer.presentation.inspector_faders[&id].state,
            &shell.mixer.presentation.inspector_faders[&id].behavior,
            cx,
        ))
        // Live bars are the sidebar hot surface's job; this painting-free
        // canvas only registers where this node's meter tracks sit.
        .child(inspector_meter_track(
            shell,
            cx.entity(),
            id,
            node.channels(),
        ))
        .when(node.channels() == 2, |view| {
            view.child(crate::ui::shell::mixer::strip_controls::balance(
                shell,
                entry,
                node.channels(),
                cx,
            ))
        })
        .child(crate::ui::shell::mixer::node_controls(
            shell,
            entry,
            node.channels(),
            cx,
        ));
    if node.kind() == NodeKind::Source {
        let bound_export = snapshot
            .graph
            .exports
            .iter()
            .find(|export| export.enabled && export.binding == Some(id));
        view = view.child(heading(t!("Source"))).child(label(
            bound_export
                .map(|export| export.name.clone())
                .or_else(|| {
                    entry
                        .source
                        .as_ref()
                        .map(|source| crate::ui::presentation::source_name(snapshot, source))
                })
                .unwrap_or_else(|| t!("Not bound · choose a resource").into()),
        ));
        // The one shared choice list (issue #119): the same ordered entries
        // the chip menu offers, rendered as this panel's pick buttons.
        for choice in binding_choices::source_choices(snapshot, entry) {
            let is_tone = matches!(choice.target, binding_choices::BindingTarget::Tone { .. });
            view = view.child(binding_choice_button(cx.entity(), choice));
            // The bound tone's frequency editor rides directly under the
            // tone choice that selected it.
            if is_tone && let Some(InputSelection::Tone { frequency_hz }) = entry.source {
                view = view
                    .child(detail(&t!("Frequency"), format!("{frequency_hz:.1} Hz")))
                    .child(
                        div()
                            .flex()
                            .gap(px(theme::SPACE_1))
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
        }
    }
    if node.kind() == NodeKind::Sink {
        let bound_export = snapshot
            .graph
            .exports
            .iter()
            .find(|export| export.enabled && export.binding == Some(id));
        view = view.child(heading(t!("Output device"))).child(label(
            bound_export
                .map(|export| export.name.clone())
                .or_else(|| {
                    entry
                        .output_endpoint_id
                        .as_ref()
                        .map(|endpoint| crate::ui::presentation::output_name(snapshot, endpoint))
                })
                .unwrap_or_else(|| t!("Not bound · choose an output").into()),
        ));
        // The shared sink choice list, same as the chip menu (issue #119).
        for choice in binding_choices::sink_choices(snapshot, entry) {
            view = view.child(binding_choice_button(cx.entity(), choice));
        }
    }
    if node.kind() == NodeKind::Gain {
        view = view
            .child(heading(t!("Gain")))
            .child(detail_editable(
                &t!("Requested"),
                SharedString::from(format!("inspector-node-gain-{id:?}")),
                db_text(entry.gain),
                shell.mixer.presentation.editing_node_gain == Some(id),
                &shell.mixer.presentation.node_gain_inputs[&id],
                start_value_edit(
                    cx.entity(),
                    EditableValueFamily::NodeGain,
                    id,
                    move |graph| {
                        graph
                            .node(id)
                            .map(|node| gain_db_text(node.gain))
                            .unwrap_or_default()
                    },
                ),
            ))
            .child(
                div()
                    .flex()
                    .gap(px(theme::SPACE_1))
                    .child(button("node-gain-down", "−1 dB").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetGain {
                            node: id,
                            gain: step_db(entry.gain, -1., 0., 16.),
                        },
                    )))
                    .child(button("node-gain-up", "+1 dB").on_click(edit(
                        cx.entity(),
                        GraphEdit::SetGain {
                            node: id,
                            // Silence cannot be multiplied into something
                            // audible: the first up-step seeds a finite value.
                            gain: if entry.gain == 0. {
                                0.01
                            } else {
                                step_db(entry.gain, 1., 0., 16.)
                            },
                        },
                    )))
                    .child(
                        button("node-unity", t!("Unity"))
                            .on_click(edit(cx.entity(), GraphEdit::SetGain { node: id, gain: 1. })),
                    ),
            )
            .child(
                button("node-silence", t!("Silence"))
                    .on_click(edit(cx.entity(), GraphEdit::SetGain { node: id, gain: 0. })),
            );
    }
    if node.kind() == NodeKind::Pan {
        view = view
            .child(heading(t!("Stereo controls")))
            .child(detail_editable(
                &t!("Balance"),
                SharedString::from(format!("inspector-node-pan-{id:?}")),
                format!("{:+.2}", entry.pan),
                shell.mixer.presentation.editing_node_pan == Some(id),
                &shell.mixer.presentation.node_pan_inputs[&id],
                start_value_edit(
                    cx.entity(),
                    EditableValueFamily::NodePan,
                    id,
                    move |graph| {
                        graph
                            .node(id)
                            .map(|node| format!("{:+.2}", node.pan))
                            .unwrap_or_default()
                    },
                ),
            ))
            .child(
                div()
                    .flex()
                    .gap(px(theme::SPACE_1))
                    .child(button("node-left", t!("Left")).on_click(edit(
                        cx.entity(),
                        GraphEdit::SetPan {
                            node: id,
                            pan: (entry.pan - 0.1).max(-1.),
                        },
                    )))
                    .child(
                        button("node-center", t!("Center"))
                            .on_click(edit(cx.entity(), GraphEdit::SetPan { node: id, pan: 0. })),
                    )
                    .child(button("node-right", t!("Right")).on_click(edit(
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
        view = view.child(heading(t!("Compressor"))).child(label(t!(
            "Linked peak detector · Enter or leave a field to apply"
        )));
        for parameter in crate::service::CompressorParameter::ALL {
            view = view.child(
                div()
                    .id(format!("compressor-{:?}", parameter.id()))
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_1))
                    .child(detail(
                        &t!(parameter.label()),
                        parameter.display(entry.compressor.value(parameter)),
                    ))
                    .child(
                        component::input::Input::new(
                            &shell.inspector.compressor.inputs[&(id, parameter)],
                        )
                        .id(format!("compressor-input-{}", parameter.id().0))
                        .bg(theme::SURFACE_3)
                        .text_color(theme::TEXT)
                        .border_color(theme::BORDER),
                    )
                    .child({
                        let slot = &shell.inspector.compressor.sliders[&(id, parameter)];
                        hslider(&slot.state, &slot.behavior, cx)
                    })
                    .gpui_test_observe(),
            );
        }
    }
    view = view.child(builtin_controls(id, node, &entry.builtin, cx));
    view = view.child(node_observation(shell, node, cx));
    view = view.child(heading(t!("Ports")));
    for (index, port) in node.inputs().iter().enumerate() {
        view = view.child(detail(
            &t!("Input %{index}", index = index + 1),
            t!("%{channels} ch", channels = port.channels()).into_owned(),
        ));
    }
    for (index, port) in node.outputs().iter().enumerate() {
        view = view.child(detail(
            &t!("Output %{index}", index = index + 1),
            t!("%{channels} ch", channels = port.channels()).into_owned(),
        ));
    }
    if node.kind() == NodeKind::Bus {
        view = view.child(
            button("add-bus-input", t!("Add bus input"))
                .on_click(edit(cx.entity(), GraphEdit::AddBusInput(id))),
        );
        if let Some(port) = node.inputs().last() {
            view = view.child(
                button("remove-bus-input", t!("Remove last input")).on_click(edit(
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
        button("remove-node", t!("Delete node"))
            .variant(ButtonVariant::Danger)
            .on_click(edit(cx.entity(), GraphEdit::RemoveNode(id))),
    )
    .gpui_test_observe()
    .into_any_element()
}

/// One shared-list choice rendered as the inspector's pick button
/// (issue #119): the same entry the chip menu offers, under this panel's
/// element-id namespace.
fn binding_choice_button(
    this: Entity<StudioShell>,
    choice: binding_choices::BindingChoice,
) -> impl IntoElement {
    button(choice.target.key(), choice.label)
        .variant(ButtonVariant::Ghost)
        .on_click(move |_, _, cx| {
            this.update(cx, |shell, cx| {
                match choice.action.clone() {
                    binding_choices::BindingAction::Edit(edit) => {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::EditGraph(edit)));
                    }
                    // The refresh keeps the section mounted: the list
                    // re-renders from the refreshed catalog.
                    binding_choices::BindingAction::Refresh => {
                        shell
                            .state
                            .update(cx, |state, _| state.dispatch(AppRequest::RefreshCatalog));
                    }
                }
                cx.notify();
            })
        })
}

/// The inspector's runtime-observation readout, sampled for the cache key
/// (issue #118). The status transitions are discrete and re-key immediately;
/// the Peak/RMS texts move with the ballistics at the meter frame rate, so
/// they re-key on the sampling cadence instead — keying the raw display
/// levels would rebuild the whole cached inspector panel per display frame.
/// The snapshot stores the formatted texts because those are exactly what a
/// viewer can see: two readings that spell the same text never wake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservationSnapshot {
    pub(crate) status: crate::ui::monitoring::MeterStatus,
    pub(crate) texts: Vec<String>,
}

pub(crate) fn observation_snapshot(
    meters: &crate::ui::monitoring::NodeMeters,
    node: moiren_core::graph::NodeId,
) -> ObservationSnapshot {
    let texts = meters
        .channel_levels(node)
        .map(|levels| {
            levels
                .iter()
                .flat_map(|level| [dbfs_text(level.marker), dbfs_text(level.rms)])
                .collect()
        })
        .unwrap_or_default();
    ObservationSnapshot {
        status: meters.status(node),
        texts,
    }
}

/// How often the observation texts are re-sampled into the inspector key.
/// Text readouts need no more than a few refreshes a second, and every
/// re-key rebuilds the cached inspector panel.
pub(crate) const OBSERVATION_SAMPLE_CADENCE: std::time::Duration =
    std::time::Duration::from_millis(200);

fn node_observation(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = shell.state.read(cx);
    let running = state.snapshot.phase == crate::service::SessionPhase::Running;
    let managed = state
        .snapshot
        .signal_topics
        .node_levels
        .contains_key(&node.id());
    let eligible = running && managed;
    let status = if eligible {
        let meter_status = shell.state.read(cx).node_meters.status(node.id());
        t!(meter_status.label()).to_string()
    } else if managed {
        t!("Not running").to_string()
    } else {
        t!("Unavailable for this node").to_string()
    };
    let mut view = div()
        .id("node-observation")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_1_5))
        .child(heading(t!("Runtime observation")))
        .child(
            div()
                .id("node-observation-status")
                .child(detail(&t!("Status"), status))
                .gpui_test_observe(),
        );
    if !running && managed {
        view = view.child(label(t!("Start audio to observe this node.")));
    } else if !managed {
        view = view.child(label(t!(
            "This node has no managed level topic in the current service projection."
        )));
    }
    if let Some(levels) = eligible
        .then(|| shell.state.read(cx).node_meters.channel_levels(node.id()))
        .flatten()
    {
        let mut readings = div()
            .id("node-observation-levels")
            .flex()
            .flex_col()
            .gpui_test_observe();
        for (channel, level) in levels.iter().enumerate() {
            readings = readings
                .child(detail(
                    &t!("Peak %{index}", index = channel + 1),
                    dbfs_text(level.marker),
                ))
                .child(detail(
                    &t!("RMS %{index}", index = channel + 1),
                    dbfs_text(level.rms),
                ));
        }
        view = view.child(readings);
    }
    panel_host("graph-inspector", &shell.inspector.scroll, view).into_any_element()
}

fn builtin_controls(
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
            let settings = *settings;
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
            view = view.child(
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
            if input_channels <= MAX_MATRIX_EDITOR_CHANNELS
                && output_channels <= MAX_MATRIX_EDITOR_CHANNELS
            {
                let matrix = channel_mapper_matrix(settings, input_channels, output_channels);
                if settings.matrix.is_empty() {
                    view = view.child(
                        button("channel-map-edit-matrix", t!("Edit weighted matrix")).on_click(
                            edit(
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
                            ),
                        ),
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
                    t!("Matrix editor is limited to 8 input and output channels; use a project file for larger layouts."),
                ));
            }
        }
        crate::service::BuiltinState::Recorder(settings) => {
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
                                capacity_frames: settings
                                    .capacity_frames
                                    .saturating_sub(48_000)
                                    .max(1),
                            }),
                        },
                    )),
                );
        }
        crate::service::BuiltinState::Meter(settings) => {
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
        }
        crate::service::BuiltinState::SpectrumAnalyzer(settings) => {
            view = view
                .child(heading(t!("Spectrum analyzer")))
                .child(detail(&t!("FFT"), settings.fft_size.to_string()))
                .child(detail(&t!("Hop"), settings.hop_frames.to_string()))
                .child(
                    button("spectrum-size", t!("Use 2048 point FFT")).on_click(edit(
                        cx.entity(),
                        GraphEdit::SetBuiltin {
                            node: id,
                            state: crate::service::BuiltinState::SpectrumAnalyzer(
                                SpectrumSettings {
                                    fft_size: 2048,
                                    hop_frames: 1024,
                                },
                            ),
                        },
                    )),
                );
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

fn gate_fields(settings: GateSettings) -> [BuiltinFieldSpec<GateSettings>; 5] {
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

fn limiter_fields(settings: LimiterSettings) -> [BuiltinFieldSpec<LimiterSettings>; 3] {
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

fn delay_fields(settings: DelaySettings) -> [BuiltinFieldSpec<DelaySettings>; 3] {
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
fn builtin_field_step<S: Copy + 'static>(
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

impl StudioShell {
    /// Per-render graph-page housekeeping: exit a name edit whose node is no
    /// longer selected, drop selection members the graph no longer has, and
    /// keep the primary selection inside the canvas multi-selection.
    pub(crate) fn sync_node_name(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.inspector.node_name_editing
            && self.selection.node != Some(id)
        {
            self.inspector.node_name_editing = None;
        }
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.selection.nodes.retain(|id| graph.node(*id).is_some());
        if let Some(id) = self.selection.node {
            if graph.node(id).is_some() {
                self.selection.nodes.insert(id);
            } else {
                self.selection.node = None;
            }
        }
    }

    /// Commits the inline name edit: dispatches SetLabel unless the value is
    /// empty (treated as cancel). Enter, ✓ and focus loss land here.
    pub(crate) fn confirm_node_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.inspector.node_name_editing.take() else {
            return;
        };
        let label = self.inspector.node_name.read(cx).value().trim().to_string();
        if !label.is_empty() {
            self.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetLabel {
                    node: id,
                    label,
                }))
            });
        }
        cx.notify();
    }

    /// Discards the inline name edit without touching the stored label.
    pub(crate) fn cancel_node_rename(&mut self, cx: &mut Context<Self>) {
        if self.inspector.node_name_editing.take().is_some() {
            cx.notify();
        }
    }
}

/// The inspector's meter footprint: a painting-free canvas that registers
/// where this node's meter tracks sit, for the sidebar hot surface to paint
/// live bars at. Sized like the DOM meter it replaced.
fn inspector_meter_track(
    shell: &StudioShell,
    this: Entity<StudioShell>,
    id: moiren_core::graph::NodeId,
    channels: usize,
) -> impl IntoElement {
    let geometry = shell.inspector.meters.clone();
    // Clicking the meter resets the clip latch, like every meter surface.
    div()
        .id(SharedString::from(format!("inspector-meter-{id:?}")))
        .w_full()
        .h(px(if channels >= 2 { 15. } else { 9. }))
        .flex_none()
        .on_mouse_down(
            MouseButton::Left,
            reset_clip_on_click(shell.state.clone(), this, id),
        )
        .child(
            canvas(
                move |bounds, window, _| {
                    geometry
                        .update(|geometry| geometry.register(id, bounds, window.content_mask()));
                },
                |_, _, _, _| {},
            )
            .size_full(),
        )
}
