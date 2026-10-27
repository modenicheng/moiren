//! Resource binding uses the same transactions as node cards.
use super::*;
use crate::ui::state::{input_rows, output_rows};
use sections::resource_list;

pub(super) fn binding(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    entry: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let id = node.id();
    let summary = if node.kind() == NodeKind::Source {
        entry
            .source
            .as_ref()
            .map(|source| crate::ui::state::source_name(snapshot, source))
            .unwrap_or_else(|| t!("Not bound · choose a resource").into_owned())
    } else {
        entry
            .output_endpoint_id
            .as_ref()
            .map(|endpoint| crate::ui::state::output_name(snapshot, endpoint))
            .unwrap_or_else(|| t!("Not bound · choose an output").into_owned())
    };
    let mut view = body();
    if node.kind() == NodeKind::Source {
        view = view
            .child(
                button("unbind-node-source", t!("Clear binding"))
                    .disabled(entry.source.is_none())
                    .on_click(edit(cx.entity(), GraphEdit::ClearBinding(id))),
            )
            .child(
                button("bind-node-tone", t!("Tone generator · 440 Hz")).on_click(edit(
                    cx.entity(),
                    GraphEdit::BindSource {
                        node: id,
                        source: InputSelection::Tone { frequency_hz: 440. },
                    },
                )),
            );
        if let Some(InputSelection::Tone { frequency_hz }) = entry.source {
            view = view
                .child(detail(&t!("Frequency"), format!("{frequency_hz:.1} Hz")))
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
        let inputs = input_rows(&snapshot.catalog, snapshot.audio_settings.backend);
        let mut devices = resource_list("inspector-input-resources", t!("Input devices"));
        if inputs.is_empty() {
            devices = devices.child(label(t!("No input devices available")));
        }
        for row in inputs {
            let selected = matches!(&entry.source, Some(InputSelection::Physical { endpoint_id }) if *endpoint_id == row.endpoint_id);
            devices = devices.child(
                button(
                    SharedString::from(format!("bind-input-{}", row.endpoint_id)),
                    row.name.clone(),
                )
                .full_width()
                .justify_start()
                .truncate_label()
                .tooltip(row.name.clone())
                .variant(if selected {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .trailing(if selected {
                    t!("Selected").into_owned()
                } else {
                    String::new()
                })
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
        view = view.child(devices);
        let mut processes = resource_list("inspector-process-resources", t!("Applications"));
        if snapshot.catalog.processes.is_empty() {
            processes = processes.child(label(t!("No application sources available")));
        }
        for row in &snapshot.catalog.processes {
            let selected = matches!(&entry.source, Some(InputSelection::Process { pid, creation_time_100ns }) if *pid == row.pid && *creation_time_100ns == row.creation_time_100ns);
            processes = processes.child(
                button(
                    SharedString::from(format!("bind-process-{}", row.pid)),
                    row.name.clone(),
                )
                .full_width()
                .justify_start()
                .truncate_label()
                .tooltip(row.name.clone())
                .variant(if selected {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .trailing(if selected {
                    t!("Selected").into_owned()
                } else {
                    String::new()
                })
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
        view = view.child(processes);
    }
    if node.kind() == NodeKind::Sink {
        view = view.child(
            button("unbind-node-output", t!("Clear binding"))
                .disabled(entry.output_endpoint_id.is_none())
                .on_click(edit(cx.entity(), GraphEdit::ClearBinding(id))),
        );
        let outputs = output_rows(&snapshot.catalog, snapshot.audio_settings.backend);
        let mut devices = resource_list("inspector-output-resources", t!("Output devices"));
        if outputs.is_empty() {
            devices = devices.child(label(t!("No output devices available")));
        }
        for row in outputs {
            let selected = entry.output_endpoint_id.as_ref() == Some(&row.endpoint_id);
            devices = devices.child(
                button(
                    SharedString::from(format!("bind-output-{}", row.endpoint_id)),
                    row.name.clone(),
                )
                .full_width()
                .justify_start()
                .truncate_label()
                .tooltip(row.name.clone())
                .variant(if selected {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .trailing(if selected {
                    t!("Selected").into_owned()
                } else {
                    String::new()
                })
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::BindOutput {
                        node: id,
                        endpoint_id: row.endpoint_id.clone(),
                    },
                )),
            );
        }
        view = view.child(devices);
    }
    if snapshot.catalog_error.is_some() {
        view = view.child(
            label(t!(
                "Resource refresh failed; showing the last available list."
            ))
            .text_color(theme::WARNING),
        );
    }
    let content = view.child(
        button("refresh-node-resources", t!("Refresh sources"))
            .full_width()
            .on_click({
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
    Disclosure::new(Section::Binding, id, content)
        .summary(summary)
        .into_any_element()
}
