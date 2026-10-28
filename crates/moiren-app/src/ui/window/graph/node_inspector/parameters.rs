//! Complete node DSP controls, separate from its channel strip.
use super::*;

pub(super) fn parameters(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    entry: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let id = node.id();
    if node.kind() == NodeKind::Eq {
        return Disclosure::new(
            Section::Advanced,
            id,
            super::super::node_controls::eq_inspector(shell, id),
        )
        .into_any_element();
    }
    if node.kind() == NodeKind::Gate {
        return Disclosure::new(
            Section::Advanced,
            id,
            super::super::node_controls::gate_inspector(shell, id),
        )
        .into_any_element();
    }
    if node.kind() == NodeKind::ChannelMapper {
        return Disclosure::new(
            Section::Advanced,
            id,
            super::super::node_controls::mapper_inspector(shell, entry, cx),
        )
        .on_toggle(
            shell.node_controls.mapper_disclosure_open(id),
            shell.node_controls.mapper_disclosure_toggle(id, cx),
        )
        .into_any_element();
    }
    if node.kind() == NodeKind::Rack {
        return Disclosure::new(
            Section::Advanced,
            id,
            super::super::node_controls::rack_inspector(shell, entry, cx),
        )
        .into_any_element();
    }
    if node.kind() == NodeKind::SpectrumAnalyzer {
        return Disclosure::new(
            Section::Advanced,
            id,
            super::super::node_controls::spectrum_inspector(shell, entry, cx),
        )
        .into_any_element();
    }
    let mut view = body();
    if node.kind() == NodeKind::Recorder {
        view = view.child(super::super::node_controls::recording_format_controls(
            shell,
            id,
            "inspector-record-format",
            cx,
        ));
    }
    if node.kind() == NodeKind::Gain {
        view = view
            .child(heading(t!("Gain")))
            .child(detail_editable(
                &t!("Requested"),
                SharedString::from(format!("inspector-node-gain-{id:?}")),
                db_text(entry.gain),
                shell.mixer.editing_node_gain == Some(id),
                &shell.mixer.node_gain_inputs[&id],
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
                    .gap(px(4.))
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
                shell.mixer.editing_node_pan == Some(id),
                &shell.mixer.node_pan_inputs[&id],
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
                    .gap(px(4.))
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
        view =
            view.child(label(t!("Enter or leave a field to apply")).text_size(px(theme::TEXT_XS)));
        for (title, parameters) in compressor_groups() {
            let mut fields = div().flex().flex_wrap().gap(px(theme::SPACE_2));
            for &parameter in parameters {
                // Input::h only sizes multiline editors; use the Styled height
                // for these single-line precision fields.
                let input = Styled::h(
                    component::input::Input::new(&shell.compressor.inputs[&(id, parameter)])
                        .id(format!("compressor-input-{}", parameter.id().0)),
                    px(26.),
                )
                .px(px(theme::SPACE_1))
                .py(px(0.))
                .text_size(px(theme::TEXT_SM))
                .bg(theme::SURFACE_3)
                .text_color(theme::TEXT)
                .border_color(theme::BORDER);
                fields = fields.child(
                    div()
                        .id(format!("compressor-{:?}", parameter.id()))
                        .flex()
                        .flex_col()
                        .w(px(118.))
                        .min_w_0()
                        .gap(px(theme::SPACE_1))
                        .child(label(t!(parameter.label())).text_size(px(theme::TEXT_XS)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(theme::SPACE_1))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(div().flex_1().min_w_0().child(input))
                                .child(
                                    label(compressor_input_unit(parameter))
                                        .text_size(px(theme::TEXT_XS))
                                        .flex_none(),
                                ),
                        )
                        .gpui_test_observe(),
                );
            }
            let group = div()
                .flex()
                .flex_col()
                .gap(px(theme::SPACE_2))
                .pt(px(theme::SPACE_2))
                .child(label(t!(title)).font_weight(FontWeight::SEMIBOLD))
                .child(fields);
            view = view.child(group);
        }
    }
    Disclosure::new(
        Section::Advanced,
        id,
        view.child(builtin::builtin_controls(id, node, &entry.builtin, cx)),
    )
    .into_any_element()
}

/// Existing InputState subscriptions parse engine units. In particular wet mix
/// is a 0..1 ratio, not the percent used by its read-only display formatter.
fn compressor_input_unit(parameter: crate::service::CompressorParameter) -> &'static str {
    use crate::service::CompressorParameter;
    match parameter {
        CompressorParameter::Ratio => ":1",
        CompressorParameter::Attack | CompressorParameter::Hold | CompressorParameter::Release => {
            "ms"
        }
        CompressorParameter::Mix => "0..1",
        _ => "dB",
    }
}

/// Detector shape, envelope timing, and gain staging are different tasks;
/// keep the full parameter set without ten visually identical sections.
fn compressor_groups() -> [(&'static str, &'static [crate::service::CompressorParameter]); 3] {
    use crate::service::CompressorParameter::*;
    [
        ("Dynamics", &[InputGain, Threshold, Ratio, Knee]),
        ("Timing", &[Attack, Hold, Release]),
        ("Gain staging", &[MakeupGain, OutputGain, Mix]),
    ]
}

#[cfg(test)]
mod tests {
    use super::{compressor_groups, compressor_input_unit};

    #[::core::prelude::v1::test]
    fn compressor_groups_cover_every_parameter_exactly_once() {
        let grouped: Vec<_> = compressor_groups()
            .into_iter()
            .flat_map(|(_, fields)| fields)
            .copied()
            .collect();
        assert_eq!(
            grouped.len(),
            crate::service::CompressorParameter::ALL.len()
        );
        for parameter in crate::service::CompressorParameter::ALL {
            assert_eq!(
                grouped.iter().filter(|&&field| field == parameter).count(),
                1
            );
        }
    }

    #[::core::prelude::v1::test]
    fn precision_input_units_match_the_existing_engine_unit_parser() {
        use crate::service::CompressorParameter::*;
        assert_eq!(compressor_input_unit(Mix), "0..1");
        assert_eq!(compressor_input_unit(Ratio), ":1");
        for parameter in [Attack, Release, Hold] {
            assert_eq!(compressor_input_unit(parameter), "ms");
        }
        for parameter in [InputGain, Threshold, Knee, MakeupGain, OutputGain] {
            assert_eq!(compressor_input_unit(parameter), "dB");
        }
    }
}
