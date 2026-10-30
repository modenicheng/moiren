use super::*;
use crate::service::{
    ExportAuthorization, ExportDirection, ExportIntent, ExportResourceState, ExportStatus,
    GraphEdit, GraphSnapshot, validate_exports,
};
use moiren_core::graph::NodeId;
use moiren_engine::node::VirtualEndpointId;
use rust_i18n::t;
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

pub(in crate::ui::window) struct ExportControls {
    name: Entity<component::input::InputState>,
    direction: ExportDirection,
    channels: usize,
    names: BTreeMap<VirtualEndpointId, NameDraft>,
}

struct NameDraft {
    input: Entity<component::input::InputState>,
    committed: String,
}

impl ExportControls {
    /// Whether one of the export editor's text inputs owns focus, so global
    /// shortcuts (undo/redo) leave typing alone.
    pub(in crate::ui::window) fn owns_focus(
        &self,
        focus: &gpui_kit::FocusHandle,
        cx: &gpui_kit::App,
    ) -> bool {
        if self.name.read(cx).focus_handle(cx) == *focus {
            return true;
        }
        self.names
            .values()
            .any(|draft| draft.input.read(cx).focus_handle(cx) == *focus)
    }

    pub(in crate::ui::window) fn new(window: &mut Window, cx: &mut Context<StudioShell>) -> Self {
        let name = name_input("Moiren Playback", window, cx);
        Self {
            name,
            direction: ExportDirection::Playback,
            channels: 2,
            names: BTreeMap::new(),
        }
    }
}

fn name_input(
    value: &str,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) -> Entity<component::input::InputState> {
    let input = cx.new(|cx| component::input::InputState::new(window, cx).default_value(value));
    cx.subscribe(&input, |_, _, _: &component::input::InputEvent, cx| {
        cx.notify()
    })
    .detach();
    input
}

enum IntentEdit {
    Rename(String),
    Channels(usize),
    Enabled(bool),
    Remove,
}

fn create_export(
    graph: &GraphSnapshot,
    endpoint_id: VirtualEndpointId,
    direction: ExportDirection,
    name: &str,
    channels: usize,
) -> Result<Vec<ExportIntent>, String> {
    let mut exports = graph.exports.clone();
    exports.push(ExportIntent {
        endpoint_id,
        direction,
        name: name.into(),
        channels,
        enabled: true,
        binding: None,
    });
    validate_exports(&exports, &graph.graph)?;
    Ok(exports)
}

fn edit_export(
    graph: &GraphSnapshot,
    endpoint_id: VirtualEndpointId,
    edit: IntentEdit,
) -> Result<Vec<ExportIntent>, String> {
    let mut exports = graph.exports.clone();
    let export = exports
        .iter_mut()
        .find(|export| export.endpoint_id == endpoint_id)
        .ok_or(t!("Export is no longer in this project's candidate graph").to_string())?;
    match edit {
        IntentEdit::Rename(name) => export.name = name,
        IntentEdit::Channels(channels) => export.channels = channels,
        IntentEdit::Enabled(enabled) => export.enabled = enabled,
        IntentEdit::Remove => exports.retain(|export| export.endpoint_id != endpoint_id),
    }
    validate_exports(&exports, &graph.graph)?;
    Ok(exports)
}

fn binding_candidate(
    graph: &GraphSnapshot,
    endpoint_id: VirtualEndpointId,
    selected: Option<NodeId>,
) -> Option<NodeId> {
    let export = graph
        .exports
        .iter()
        .find(|export| export.endpoint_id == endpoint_id)?;
    selected.filter(|id| {
        graph.graph.get_node(*id).is_ok_and(|node| {
            node.kind() == export.direction.node_kind() && node.channels() == export.channels
        }) && !graph
            .exports
            .iter()
            .any(|other| other.endpoint_id != endpoint_id && other.binding == Some(*id))
    })
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_export_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.presentation.page != Page::Devices || self.device_tab != DeviceTab::Exports {
            return;
        }
        let exports = self.state.read(cx).snapshot.graph.exports.clone();
        self.export_controls
            .names
            .retain(|id, _| exports.iter().any(|export| export.endpoint_id == *id));
        for export in exports {
            let draft = self
                .export_controls
                .names
                .entry(export.endpoint_id)
                .or_insert_with(|| NameDraft {
                    input: name_input(&export.name, window, cx),
                    committed: export.name.clone(),
                });
            // Keep unsubmitted text through service polls and validation errors.
            if draft.committed != export.name
                && !draft.input.read(cx).focus_handle(cx).is_focused(window)
            {
                draft.input.update(cx, |input, cx| {
                    input.set_value(export.name.clone(), window, cx)
                });
                draft.committed = export.name;
            }
        }
    }

    fn submit_export_edit(
        &mut self,
        id: VirtualEndpointId,
        edit: IntentEdit,
        cx: &mut Context<Self>,
    ) {
        let result = edit_export(&self.state.read(cx).snapshot.graph, id, edit);
        self.dispatch_exports(result, cx);
    }

    fn submit_export_binding(
        &mut self,
        id: VirtualEndpointId,
        node: Option<NodeId>,
        cx: &mut Context<StudioShell>,
    ) {
        self.state.update(cx, |state, _| {
            if !state.pending_request {
                state.dispatch(AppRequest::EditGraph(GraphEdit::BindExport {
                    endpoint_id: id,
                    node,
                }));
            }
        });
        cx.notify();
    }

    fn dispatch_exports(
        &mut self,
        exports: Result<Vec<ExportIntent>, String>,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, _| {
            if state.pending_request {
                return;
            }
            match exports {
                Ok(exports) => {
                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetExports(exports)))
                }
                Err(error) => state.dispatch_error = Some(error),
            }
        });
        cx.notify();
    }
}

pub(super) fn list(shell: &StudioShell, cx: &Context<StudioShell>) -> Stateful<Div> {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let mut list = div()
        .id("virtual-export-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(12.))
        .overflow_y_scroll()
        .child(heading(t!("Virtual exports")))
        .child(label(t!("Endpoint presence is not evidence of audio flowing. Stop keeps endpoints present with gates closed.")))
        .child(creation_controls(shell, cx))
        .child(label(t!("Virtual speaker: other apps play into a Moiren input. Virtual microphone: a Moiren output feeds other apps. Bind matching channels; each IO belongs to one export.")))
        .child(div().id("export-default-help")
            .aria_label(t!(DEFAULT_ELIGIBILITY_HELP))
            .child(label(t!(DEFAULT_ELIGIBILITY_HELP)))
            .gpui_test_observe())
        .child(label(t!("Changing channels, a name, or eligibility safely retires and recreates the endpoint; unbind incompatible Graph IO before changing channels.")));
    if snapshot.exports.is_empty() {
        list = list.child(label(t!(
            "No virtual exports yet. Create one above; it starts unbound."
        )));
    }
    for status in &snapshot.exports {
        list = list.child(export_row(shell, status, cx));
    }
    list
}

const DEFAULT_ELIGIBILITY_HELP: &str = "Default eligibility is permission on this computer only, not saved in the project. Off prevents both Windows automatic and user manual default selection. On only permits eligibility; it does not set a Windows default.";

fn creation_controls(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let pending = shell.state.read(cx).pending_request;
    let mut choices = div().flex().flex_wrap().items_center().gap(px(6.));
    for (id, title, direction) in [
        (
            "export-create-playback",
            t!("Playback"),
            ExportDirection::Playback,
        ),
        (
            "export-create-microphone",
            t!("Microphone"),
            ExportDirection::Microphone,
        ),
    ] {
        let this = cx.entity();
        choices = choices.child(
            button(id, title)
                .variant(if shell.export_controls.direction == direction {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .disabled(pending)
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.export_controls.direction = direction;
                        cx.notify();
                    });
                }),
        );
    }
    for (id, title, channels) in [
        ("export-create-mono", t!("Mono"), 1),
        ("export-create-stereo", t!("Stereo"), 2),
    ] {
        let this = cx.entity();
        choices = choices.child(
            button(id, title)
                .variant(if shell.export_controls.channels == channels {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .disabled(pending)
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.export_controls.channels = channels;
                        cx.notify();
                    });
                }),
        );
    }
    let this = cx.entity();
    div()
        .id("export-create-panel")
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .bg(theme::SURFACE)
        .rounded(px(theme::RADIUS_MD))
        .child(heading(t!("Create export")))
        .child(choices)
        .child(label(t!(
            "Endpoint name (1-63 UTF-16 units; no control characters)"
        )))
        .child(
            component::input::Input::new(&shell.export_controls.name)
                .id("export-create-name")
                .bg(theme::SURFACE_3)
                .text_color(theme::TEXT)
                .border_color(theme::BORDER),
        )
        .child(
            div().flex().flex_wrap().items_center().gap(px(8.)).child(
                button("export-create", t!("Create export"))
                    .variant(ButtonVariant::Primary)
                    .disabled(
                        pending
                            || shell.state.read(cx).snapshot.graph.exports.len()
                                >= crate::service::MAX_EXPORTS,
                    )
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            // Use GPUI's public UUID type; persist its ordered bytes, never a native token.
                            let id = VirtualEndpointId::from_bytes(
                                *gpui_kit::accesskit::Uuid::new_v4().as_bytes(),
                            );
                            let name = shell.export_controls.name.read(cx).value();
                            let graph = &shell.state.read(cx).snapshot.graph;
                            let result = create_export(
                                graph,
                                id,
                                shell.export_controls.direction,
                                &name,
                                shell.export_controls.channels,
                            );
                            shell.dispatch_exports(result, cx);
                        });
                    }),
            ),
        )
        .gpui_test_observe()
}

fn export_row(
    shell: &StudioShell,
    status: &ExportStatus,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let state = shell.state.read(cx);
    let graph = &state.snapshot.graph;
    let id = status.intent.endpoint_id;
    let desired = graph.exports.iter().find(|export| export.endpoint_id == id);
    let mut row = div()
        .id(SharedString::from(format!("export-row-{id}")))
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .bg(theme::SURFACE)
        .rounded(px(theme::RADIUS_MD))
        .child(heading(status.intent.name.clone()))
        .child(label(format!(
            "{:?} / {} / {id}",
            status.intent.direction,
            if status.intent.channels == 1 {
                t!("Mono")
            } else {
                t!("Stereo")
            }
        )))
        .child(detail(
            &t!("Endpoint"),
            match status.resource {
                ExportResourceState::Unavailable => t!("Unavailable"),
                ExportResourceState::Absent => t!("Absent"),
                ExportResourceState::Creating => t!("Creating"),
                ExportResourceState::Present => t!("Present (not a flow signal)"),
                ExportResourceState::Removing => t!("Removing / awaiting confirmation"),
            },
        ))
        .child(detail(
            &t!("Gate admission"),
            if status.gate_open {
                t!("Open (not a flow signal)")
            } else {
                t!("Closed")
            },
        ))
        .child(detail(
            &t!("Engine-confirmed binding"),
            status
                .active_revision
                .map(|revision| t!("Active revision %{revision}", revision = revision).to_string())
                .unwrap_or_else(|| t!("Inactive").into()),
        ))
        .child(detail(
            &t!("Candidate graph binding"),
            desired
                .and_then(|intent| intent.binding)
                .and_then(|node| graph.node(node))
                .map(|node| node.label.clone())
                .unwrap_or_else(|| t!("Unbound").into()),
        ));
    if let Some(desired) = desired {
        row = row.child(detail(
            &t!("Candidate intent"),
            if desired.enabled {
                t!("Enabled")
            } else {
                t!("Disabled")
            },
        ));
        let mut shape = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .child(label(t!("Channels")));
        for (key, title, channels) in [("mono", t!("Mono"), 1), ("stereo", t!("Stereo"), 2)] {
            let this = cx.entity();
            shape = shape.child(
                button(SharedString::from(format!("export-{key}-{id}")), title)
                    .variant(if desired.channels == channels {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .disabled(state.pending_request || desired.channels == channels)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.submit_export_edit(id, IntentEdit::Channels(channels), cx);
                        });
                    }),
            );
        }
        row = row.child(shape);
        if let Some(draft) = shell.export_controls.names.get(&id) {
            let this = cx.entity();
            row = row
                .child(label(t!("Endpoint name")))
                .child(
                    component::input::Input::new(&draft.input)
                        .id(SharedString::from(format!("export-name-{id}")))
                        .bg(theme::SURFACE_3)
                        .text_color(theme::TEXT)
                        .border_color(theme::BORDER),
                )
                .child(
                    button(
                        SharedString::from(format!("export-rename-{id}")),
                        t!("Rename"),
                    )
                    .disabled(
                        state.pending_request
                            || draft.input.read(cx).value().as_ref() == desired.name,
                    )
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            if let Some(draft) = shell.export_controls.names.get(&id) {
                                let name = draft.input.read(cx).value().to_string();
                                shell.submit_export_edit(id, IntentEdit::Rename(name), cx);
                            }
                        });
                    }),
                );
        }
        let selected = binding_candidate(graph, id, shell.selected_node);
        let enabled = !desired.enabled;
        let this = cx.entity();
        let bind = cx.entity();
        let unbind = cx.entity();
        let remove = cx.entity();
        row = row.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .child(
                    button(
                        SharedString::from(format!("export-toggle-{id}")),
                        if desired.enabled {
                            t!("Disable intent")
                        } else {
                            t!("Enable intent")
                        },
                    )
                    .disabled(state.pending_request)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.submit_export_edit(id, IntentEdit::Enabled(enabled), cx)
                        });
                    }),
                )
                .child(
                    button(
                        SharedString::from(format!("export-bind-{id}")),
                        t!("Bind selected IO"),
                    )
                    .disabled(
                        state.pending_request || selected.is_none() || selected == desired.binding,
                    )
                    .on_click(move |_, _, cx| {
                        bind.update(cx, |shell, cx| {
                            // Re-resolve current selection and intent instead of replaying a stale render snapshot.
                            let selected = binding_candidate(
                                &shell.state.read(cx).snapshot.graph,
                                id,
                                shell.selected_node,
                            );
                            if let Some(node) = selected {
                                shell.submit_export_binding(id, Some(node), cx);
                            }
                        });
                    }),
                )
                .child(
                    button(
                        SharedString::from(format!("export-unbind-{id}")),
                        t!("Unbind"),
                    )
                    .disabled(state.pending_request || desired.binding.is_none())
                    .on_click(move |_, _, cx| {
                        unbind.update(cx, |shell, cx| shell.submit_export_binding(id, None, cx));
                    }),
                )
                .child(
                    button(
                        SharedString::from(format!("export-remove-{id}")),
                        t!("Remove intent"),
                    )
                    .disabled(state.pending_request)
                    .on_click(move |_, _, cx| {
                        remove.update(cx, |shell, cx| {
                            shell.submit_export_edit(id, IntentEdit::Remove, cx)
                        });
                    }),
                ),
        );
        row = row.child(default_control(shell, status, cx));
    } else {
        row = row.child(label(t!("Removed from candidate. The service retains lifecycle tracking until safe retirement is confirmed.")));
    }
    row.gpui_test_observe()
}

fn default_control(
    shell: &StudioShell,
    status: &ExportStatus,
    cx: &Context<StudioShell>,
) -> ActionButton {
    let this = cx.entity();
    let id = status.intent.endpoint_id;
    button(
        SharedString::from(format!("export-default-{id}")),
        if status.allow_default {
            t!("Revoke local default eligibility")
        } else {
            t!("Allow default eligibility on this computer")
        },
    )
    .disabled(shell.state.read(cx).pending_request)
    .on_click(move |_, _, cx| {
        this.update(cx, |shell, cx| {
            if shell.state.read(cx).pending_request {
                return;
            }
            let Some(status) = shell
                .state
                .read(cx)
                .snapshot
                .exports
                .iter()
                .find(|status| status.intent.endpoint_id == id)
            else {
                return;
            };
            let allow_default = !status.allow_default;
            let mut authorizations = shell.preferences.export_authorizations.clone();
            if let Some(value) = authorizations
                .iter_mut()
                .find(|value| value.endpoint_id == id)
            {
                value.allow_default = allow_default;
            } else {
                authorizations.push(ExportAuthorization {
                    endpoint_id: id,
                    allow_default,
                });
            }
            let result = shell.state.update(cx, |state, _| {
                state.try_dispatch(AppRequest::SetExportAuthorizations(authorizations.clone()))
            });
            if result.is_ok() {
                shell.preferences.export_authorizations = authorizations;
                shell.save_preferences(cx);
            }
            cx.notify();
        });
    })
}
