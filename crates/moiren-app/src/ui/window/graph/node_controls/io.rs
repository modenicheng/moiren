//! Resource selection and digital input/output monitoring.
use super::*;

/// Binding stays full width; the monitoring strip stretches to the adjacent
/// controls rather than deciding the card's height itself.
pub(super) fn io_body(
    shell: &StudioShell,
    card: &CanvasNode,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let id = node.id;
    let channels = snapshot
        .graph
        .graph
        .get_node(id)
        .map_or(2, |n| n.channels());
    let meter_width = crate::ui::widgets::meters::node_meter_width(channels);
    let this = cx.entity();
    let details = this.clone();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .w_full()
        .child(resource_selector(shell, card, node, cx));
    if snapshot.phase == SessionPhase::Running
        && snapshot.graph.active_revision != Some(snapshot.graph.candidate_revision)
    {
        body = body
            .child(note("Edits pending; meters show the running graph").text_color(theme::WARNING));
    }
    body.child(
        div()
            .relative()
            .w_full()
            .pr(px(meter_width + 16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(parameter_control(shell, node, Parameter::Level, true, cx))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(5.))
                                    .flex_1()
                                    .min_w_0()
                                    .child(note(format!("{channels} ch")))
                                    .child(note(if card.kind == NodeKind::Source {
                                        "Digital trim · post-fader meter"
                                    } else {
                                        "Post-fader · Peak / RMS"
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                controls::button(format!("card-mute-{id:?}"), t!("Mute"))
                                    .variant(if node.mixer.muted {
                                        ButtonVariant::Danger
                                    } else {
                                        ButtonVariant::Secondary
                                    })
                                    .on_click(controls::edit(
                                        this,
                                        GraphEdit::EditMixer {
                                            node: id,
                                            edit: MixerEdit::ToggleMute,
                                        },
                                    )),
                            )
                            .child(
                                controls::button(format!("card-details-{id:?}"), t!("Details"))
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(move |_, _, cx| {
                                        details.update(cx, |shell, cx| {
                                            shell.selected_node = Some(id);
                                            shell.selected_nodes.clear();
                                            shell.selected_nodes.insert(id);
                                            shell.selected_route = None;
                                            shell.open_inspector();
                                            cx.notify();
                                        })
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .w(px(meter_width))
                    .child(vertical_meter(shell, node, cx)),
            ),
    )
    .into_any_element()
}

fn resource_selector(
    shell: &StudioShell,
    card: &CanvasNode,
    node: &GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    use component::popover::Popover;
    let snapshot = &shell.state.read(cx).snapshot;
    let id = node.id;
    let output = card.kind == NodeKind::Sink;
    let process = shell
        .node_controls
        .application_picker
        .get(&id)
        .copied()
        .unwrap_or_else(|| {
            matches!(node.source, Some(InputSelection::Process { .. }))
                || node.label == "Application audio"
        });
    let mut choices: Vec<(String, GraphEdit)> = Vec::new();
    if output {
        for d in crate::ui::state::output_rows(&snapshot.catalog, snapshot.audio_settings.backend) {
            choices.push((
                d.name.clone(),
                GraphEdit::BindOutput {
                    node: id,
                    endpoint_id: d.endpoint_id.clone(),
                },
            ));
        }
    } else if process {
        for p in &snapshot.catalog.processes {
            choices.push((
                p.name.clone(),
                GraphEdit::BindSource {
                    node: id,
                    source: InputSelection::Process {
                        pid: p.pid,
                        creation_time_100ns: p.creation_time_100ns,
                    },
                },
            ));
        }
    } else {
        for d in crate::ui::state::input_rows(&snapshot.catalog, snapshot.audio_settings.backend) {
            choices.push((
                d.name.clone(),
                GraphEdit::BindSource {
                    node: id,
                    source: InputSelection::Physical {
                        endpoint_id: d.endpoint_id.clone(),
                    },
                },
            ));
        }
    }
    let title = if output {
        node.output_endpoint_id
            .as_ref()
            .map(|id| crate::ui::state::output_name(snapshot, id))
    } else {
        node.source
            .as_ref()
            .map(|s| crate::ui::state::source_name(snapshot, s))
    }
    .unwrap_or_else(|| t!("Choose a device").to_string());
    let this = cx.entity();
    let mut menu = div()
        .id(format!("card-resource-list-{id:?}"))
        .flex()
        .flex_col()
        .gap(px(4.))
        .w(px(300.))
        .max_h(px(320.))
        .overflow_y_scroll();
    if choices.is_empty() {
        menu = menu.child(note("No available devices. Refresh to discover."));
    }
    for (index, (name, edit)) in choices.into_iter().enumerate() {
        menu = menu.child(
            controls::button(format!("card-resource-{id:?}-{index}"), name.clone())
                .full_width()
                .justify_start()
                .truncate_label()
                .tooltip(name)
                .on_click({
                    let this = this.clone();
                    move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.state.update(cx, |state, _| {
                                state.dispatch(AppRequest::EditGraph(edit.clone()))
                            });
                            shell.node_controls.resource_popup = None;
                            cx.notify();
                        })
                    }
                }),
        );
    }
    menu = menu.child(
        controls::button(format!("card-refresh-{id:?}"), t!("Refresh")).on_click(
            move |_, _, cx| {
                this.update(cx, |s, cx| {
                    s.state
                        .update(cx, |s, _| s.dispatch(AppRequest::RefreshCatalog));
                    cx.notify();
                })
            },
        ),
    );
    let mut view = div().flex().flex_col().gap(px(4.));
    if !output {
        let mut modes = div().flex().gap(px(4.));
        for (application, label) in [(false, "Physical input"), (true, "Application audio")] {
            let this = cx.entity();
            modes = modes.child(
                controls::button(format!("card-source-mode-{id:?}-{application}"), t!(label))
                    .variant(if process == application {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell
                                .node_controls
                                .application_picker
                                .insert(id, application);
                            shell.node_controls.resource_popup = Some(id);
                            cx.notify();
                        })
                    }),
            );
        }
        view = view.child(modes);
    }
    let this = cx.entity();
    view.child(note(if output {
        "Output device"
    } else if process {
        "Application"
    } else {
        "Input device"
    }))
    .child(
        Popover::new(format!("card-device-popup-{id:?}"))
            .open(shell.node_controls.resource_popup == Some(id))
            .on_open_change(move |open, _, cx| {
                this.update(cx, |shell, cx| {
                    shell.node_controls.resource_popup = open.then_some(id);
                    cx.notify();
                })
            })
            .trigger(ResourceTrigger(
                controls::button(format!("card-device-{id:?}"), title.clone())
                    .full_width()
                    .justify_start()
                    .truncate_label()
                    .tooltip(title)
                    .trailing("v"),
            ))
            .child(menu),
    )
    .into_any_element()
}

/// Adapt the existing action primitive to Popover without changing its library.
#[derive(IntoElement)]
struct ResourceTrigger(ActionButton);
impl component::Selectable for ResourceTrigger {
    fn selected(self, _selected: bool) -> Self {
        self
    }
    fn is_selected(&self) -> bool {
        false
    }
}
impl RenderOnce for ResourceTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}
