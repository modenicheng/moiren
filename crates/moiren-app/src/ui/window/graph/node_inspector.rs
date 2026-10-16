use super::*;
use crate::service::{GraphEdit, InputSelection};
use controls::*;
use moiren_core::graph::NodeKind;

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
