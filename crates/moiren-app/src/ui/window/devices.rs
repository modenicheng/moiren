use super::*;
use crate::ui::state::{SourceMode, output_name, source_name};
use crate::ui::window::panel::{CachedPanel, PanelHost, PublishRef, cached_splice};
use controls::*;
use rust_i18n::t;

mod exports;
pub(super) use exports::ExportControls;
mod overrides;

/// Quick-bind candidate for a virtual export row: the first graph node whose
/// kind and channel count match the export, preferring one no other export
/// holds so the shortcut never steals an occupied IO while a free match
/// exists. The service edit still validates the final choice.
fn export_bind_candidate(
    graph: &crate::service::GraphSnapshot,
    direction: crate::service::ExportDirection,
    channels: usize,
) -> Option<moiren_core::graph::NodeId> {
    let kind = direction.node_kind();
    let mut occupied = None;
    for node in graph.graph.nodes() {
        if node.kind() != kind || node.channels() != channels {
            continue;
        }
        let id = node.id();
        if graph
            .exports
            .iter()
            .all(|export| export.binding != Some(id))
        {
            return Some(id);
        }
        occupied.get_or_insert(id);
    }
    occupied
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeviceTab {
    Inputs,
    #[default]
    Outputs,
    Applications,
    Exports,
    Clock,
}

/// The devices page behind a [`PanelHost`]: rebuilt only when the published
/// snapshot, the active tab, the selected device, or an export draft input
/// changed.
pub(in crate::ui::window) struct DevicesPanel;

#[derive(PartialEq)]
pub(in crate::ui::window) struct DevicesKey {
    snapshot: PublishRef<crate::service::AppSnapshot>,
    device_tab: DeviceTab,
    selected_device: Option<String>,
    exports_scroll: Point<Pixels>,
}

pub(in crate::ui::window) type DevicesPanelHost = PanelHost<DevicesPanel>;

impl CachedPanel for DevicesPanel {
    type Owner = StudioShell;
    type Key = DevicesKey;

    fn key(&self, shell: &Self::Owner, cx: &App) -> Self::Key {
        Self::Key {
            snapshot: PublishRef::new(std::sync::Arc::clone(&shell.state.read(cx).snapshot)),
            device_tab: shell.device_tab,
            selected_device: shell.selected_device.clone(),
            // The export list scrolls inside the cached page; the offset
            // rides in the key so wheel steps rebuild it (the mixer's
            // precedent for scrollable containers under a cached panel).
            exports_scroll: shell.exports_scroll.offset(),
        }
    }

    fn render(
        &self,
        shell: &mut Self::Owner,
        _window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement {
        page(shell, cx)
    }

    fn observe_nested(
        &self,
        owner: &Entity<Self::Owner>,
        cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        // The export drafts' inputs are model entities nested in this cached
        // tree; typing must re-render through explicit observers.
        let inputs = owner.read_with(cx, |shell, _| shell.export_controls.input_entities());
        inputs
            .into_iter()
            .map(|input| cx.observe(&input, |host, _, cx| host.notify_nested(cx)))
            .collect()
    }
}

/// The devices page's cached splice. The Exports tab stays live: its list
/// scrolls under the cached subtree, and a rebuild's scroll clamp resets the
/// tracked offset between frames (the scroll-under-cache hazard in design
/// 17) — until that has a framework-level answer, the tab keeps the
/// rebuild-every-notify behavior it always had.
pub(in crate::ui::window) fn page_for(
    shell: &StudioShell,
    _window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    if shell.device_tab == DeviceTab::Exports {
        return page(shell, cx);
    }
    cached_splice(shell.devices_panel.clone(), |entity| {
        entity
            .cached(StyleRefinement::default().flex_1().min_w_0().min_h_0())
            .into_any_element()
    })
}

pub(super) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let catalog = &snapshot.catalog;
    let asio = snapshot.audio_settings.backend == moiren_engine::audio_config::AudioBackend::Asio;
    let mut tabs = div().flex().items_center().gap(px(6.));
    for (id, title, tab) in [
        ("devices-inputs", t!("Inputs"), DeviceTab::Inputs),
        ("devices-outputs", t!("Outputs"), DeviceTab::Outputs),
        ("devices-apps", t!("Applications"), DeviceTab::Applications),
        ("devices-exports", t!("Exports"), DeviceTab::Exports),
        ("devices-clock", t!("Clock"), DeviceTab::Clock),
    ] {
        let this = cx.entity();
        tabs = tabs.child(
            button(id, title)
                .variant(if shell.device_tab == tab {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .disabled(asio && tab == DeviceTab::Applications)
                .on_click(move |_, _, cx| {
                    this.update(cx, |s, cx| {
                        s.device_tab = tab;
                        s.selected_device = None;
                        cx.notify();
                    })
                }),
        );
    }
    let mut list = div()
        .id("device-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(8.))
        .overflow_y_scroll();
    match shell.device_tab {
        DeviceTab::Exports => {
            list = exports::list(shell, cx);
        }
        DeviceTab::Inputs | DeviceTab::Outputs => {
            let input = shell.device_tab == DeviceTab::Inputs;
            let available = crate::ui::state::device_rows(
                catalog,
                &snapshot.exports,
                snapshot.audio_settings.backend,
                input,
            );
            let direction = if input {
                crate::service::ExportDirection::Playback
            } else {
                crate::service::ExportDirection::Microphone
            };
            let mut rows = available.clone();
            let saved = if input {
                &shell.settings.device.audio.input_overrides
            } else {
                &shell.settings.device.audio.output_overrides
            };
            for id in saved.keys() {
                if !catalog.vad_endpoints.values().any(|owned| owned == id)
                    && !rows.iter().any(|row| &row.endpoint_id == id)
                {
                    rows.push(crate::service::DeviceRow {
                        endpoint_id: id.clone(),
                        name: t!("Unavailable device (saved override)").into(),
                    });
                }
            }
            if rows.is_empty() {
                list = list
                    .child(heading(t!("No devices available")))
                    .child(label(if asio {
                        t!("Refresh to discover installed ASIO drivers.")
                    } else {
                        t!("Refresh to discover connected Windows audio endpoints.")
                    }));
            }
            for row in &rows {
                let present = available
                    .iter()
                    .any(|entry| entry.endpoint_id == row.endpoint_id);
                let inspected = shell.selected_device.as_ref() == Some(&row.endpoint_id);
                let vad_export = snapshot.exports.iter().find(|status| {
                    status.intent.direction == direction
                        && status
                            .windows_name
                            .as_ref()
                            .and_then(|name| name.mmdevice_id.as_deref())
                            == Some(row.endpoint_id.as_str())
                });
                let selected = if let Some(status) = vad_export {
                    status.intent.binding.is_some()
                } else if input {
                    state.selection.input_id.as_ref() == Some(&row.endpoint_id)
                } else {
                    state.selection.output_id.as_ref() == Some(&row.endpoint_id)
                };
                let default = if input {
                    catalog.default_input_endpoint_id.as_ref() == Some(&row.endpoint_id)
                } else {
                    catalog.default_output_endpoint_id.as_ref() == Some(&row.endpoint_id)
                };
                let active = vad_export.map_or_else(
                    || snapshot.phase == SessionPhase::Running
                        && snapshot.desired.as_ref().is_some_and(|s| if input {
                            matches!(&s.source, crate::service::InputSelection::Physical { endpoint_id } if endpoint_id == &row.endpoint_id)
                        } else {
                            s.output_endpoint_id == row.endpoint_id
                        }),
                    |status| status.active_revision.is_some(),
                );
                let this = cx.entity();
                let id = row.endpoint_id.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("device-row-{id}")))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(12.))
                        .p(px(12.))
                        .rounded(px(theme::RADIUS_MD))
                        .bg(theme::SURFACE)
                        .border_1()
                        .border_color(if inspected {
                            theme::PRIMARY
                        } else {
                            theme::BORDER
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(4.))
                                .child(heading(row.name.clone())),
                        )
                        .when(!present, |v| {
                            v.child(badge(t!("Unavailable"), theme::WARNING))
                        })
                        .when(saved.contains_key(&id), |v| {
                            v.child(badge(t!("Override"), theme::PRIMARY))
                        })
                        .when(default && !asio, |v| {
                            v.child(badge(t!("Default"), theme::TEXT_MUTED))
                        })
                        .when(vad_export.is_none(), |row| {
                            row.child(
                                button(
                                    SharedString::from(format!("inspect-{id}")),
                                    t!("Configure"),
                                )
                                .on_click({
                                    let this = cx.entity();
                                    let id = id.clone();
                                    move |_, _, cx| {
                                        this.update(cx, |shell, cx| {
                                            shell.inspect_device(id.clone(), cx)
                                        })
                                    }
                                }),
                            )
                        })
                        .when(active, |v| v.child(badge(t!("Active"), theme::SUCCESS)))
                        .child(
                            button(
                                SharedString::from(format!("choose-{id}")),
                                if vad_export.is_some() {
                                    if selected { t!("Bound") } else { t!("Bind") }
                                } else if selected {
                                    t!("Next start")
                                } else {
                                    t!("Use next start")
                                },
                            )
                            .disabled(
                                !present
                                    || state.pending_request
                                    || vad_export.is_some_and(|status| {
                                        selected
                                            || export_bind_candidate(
                                                &snapshot.graph,
                                                direction,
                                                status.intent.channels,
                                            )
                                            .is_none()
                                    }),
                            )
                            .on_click(move |_, _, cx| {
                                this.update(cx, |s, cx| {
                                    if let Some(status) =
                                        s.state.read(cx).snapshot.exports.iter().find(|status| {
                                            status.intent.enabled
                                                && status.intent.direction == direction
                                                && status
                                                    .windows_name
                                                    .as_ref()
                                                    .and_then(|name| name.mmdevice_id.as_deref())
                                                    == Some(id.as_str())
                                        })
                                    {
                                        let endpoint_id = status.intent.endpoint_id;
                                        let node = export_bind_candidate(
                                            &s.state.read(cx).snapshot.graph,
                                            direction,
                                            status.intent.channels,
                                        );
                                        if let Some(node) = node {
                                            s.state.update(cx, |state, _| {
                                                state.dispatch(AppRequest::EditGraph(
                                                    crate::service::GraphEdit::BindExport {
                                                        endpoint_id,
                                                        node: Some(node),
                                                    },
                                                ));
                                            });
                                        }
                                    } else {
                                        if s.state
                                            .read(cx)
                                            .snapshot
                                            .catalog
                                            .vad_endpoints
                                            .values()
                                            .any(|owned| owned == &id)
                                        {
                                            return;
                                        }
                                        s.state.update(cx, |state, _| {
                                            if input {
                                                state.selection.input_id = Some(id.clone());
                                                state.selection.process_key = None;
                                                state.source_mode = SourceMode::Input;
                                            } else {
                                                state.selection.output_id = Some(id.clone());
                                            }
                                            state.bind_candidate_resources();
                                        });
                                    }
                                    cx.notify();
                                })
                            }),
                        )
                        .gpui_test_observe(),
                );
            }
        }
        DeviceTab::Applications => {
            if asio {
                list = list.child(label(t!("Application loopback requires WASAPI. Choose WASAPI in Settings to use application sources.")));
            }
            if catalog.processes.is_empty() {
                list=list.child(heading(t!("No capturable applications"))).child(label(t!("Start an application that plays audio, then refresh. Protected applications may be unavailable.")));
            }
            for process in catalog.processes.iter().filter(|_| !asio) {
                let key = (process.pid, process.creation_time_100ns);
                let this = cx.entity();
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .p(px(12.))
                        .bg(theme::SURFACE)
                        .rounded(px(theme::RADIUS_MD))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(heading(process.name.clone()))
                                .child(label(t!(
                                    "PID %{pid} · created %{created}",
                                    pid = process.pid,
                                    created = process.creation_time_100ns
                                ))),
                        )
                        .child(
                            button(
                                SharedString::from(format!("process-{}-{}", key.0, key.1)),
                                if state.selection.process_key == Some(key) {
                                    t!("Next start")
                                } else {
                                    t!("Use next start")
                                },
                            )
                            .on_click(move |_, _, cx| {
                                this.update(cx, |s, cx| {
                                    s.state.update(cx, |state, _| {
                                        state.source_mode = SourceMode::Application;
                                        state.selection.process_key = Some(key);
                                        state.bind_candidate_resources();
                                    });
                                    cx.notify();
                                })
                            }),
                        ),
                );
            }
        }
        DeviceTab::Clock => {
            list = list.child(heading(t!("Active audio clocks")));
            let rows = crate::ui::presentation::native_timing_rows(snapshot);
            if rows.is_empty() {
                list = list.child(label(t!(
                    "No active native timing. Start audio to inspect the actual stream clocks."
                )));
            } else {
                for (key, value) in rows {
                    list = list.child(detail(&key, value));
                }
            }
        }
    }
    div()
        .id("devices-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .flex_none()
                .h(px(48.))
                .items_center()
                .justify_between()
                .px(px(16.))
                .gap(px(8.))
                .child(tabs)
                .child(
                    button("refresh-devices", t!("Refresh devices"))
                        .disabled(state.pending_request)
                        .on_click({
                            let this = cx.entity();
                            move |_, _, cx| {
                                this.update(cx, |s, cx| {
                                    s.state.update(cx, |state, _| {
                                        state.dispatch(AppRequest::RefreshCatalog)
                                    });
                                    cx.notify();
                                })
                            }
                        }),
                ),
        )
        .when_some(catalog.asio_error.clone(), |view, error| {
            view.child(
                div()
                    .id("devices-asio-error")
                    .flex_none()
                    .px(px(16.))
                    .py(px(8.))
                    .bg(theme::soft(theme::DANGER))
                    .text_color(theme::DANGER)
                    .child(t!(
                        "ASIO discovery failed: %{error}. Refresh devices to retry.",
                        error = error
                    ))
                    .gpui_test_observe(),
            )
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .p(px(16.))
                .child(list),
        )
        .gpui_test_observe()
        .into_any_element()
}

/// Session-resource summary for the global inspector; a configured device on
/// the input/output tabs swaps in the override editor instead.
pub(super) fn inspector(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    if matches!(shell.device_tab, DeviceTab::Inputs | DeviceTab::Outputs)
        && shell.selected_device.is_some()
    {
        return div()
            .id("device-override-host")
            .flex()
            .flex_none()
            .w_full()
            .h_full()
            .p(px(theme::SPACE_3))
            .child(overrides::inspector(shell, cx))
            .into_any_element();
    }
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let mut view = panel("device-inspector").child(heading(t!("Session resources")));
    if let Some(active) = snapshot
        .desired
        .as_ref()
        .filter(|_| snapshot.phase == SessionPhase::Running)
    {
        view = view
            .child(badge(t!("Active session"), theme::SUCCESS))
            .child(detail(&t!("Source"), source_name(snapshot, &active.source)))
            .child(detail(
                &t!("Output"),
                output_name(snapshot, &active.output_endpoint_id),
            ));
    } else {
        view = view.child(label(t!("No active session")));
    }
    view.child(badge(t!("Next start"), theme::WARNING))
        .child(detail(&t!("Source mode"), t!(state.source_mode.title())))
        .child(detail(
            &t!("Source"),
            state
                .candidate_spec()
                .map(|spec| source_name(snapshot, &spec.source))
                .unwrap_or_else(|| t!("Select available resources").into()),
        ))
        .child(detail(
            &t!("Output"),
            state
                .selection
                .output_id
                .as_ref()
                .map(|id| output_name(snapshot, id))
                .unwrap_or_else(|| t!("Not selected").into()),
        ))
        .child(label(t!(
            "Selecting another resource leaves the running session unchanged. Stop and start to switch devices."
        )))
        .gpui_test_observe()
        .into_any_element()
}

#[cfg(test)]
mod tests;
