use super::*;
use crate::ui::state::{SourceMode, input_rows, output_name, output_rows, source_name};
use controls::*;
use rust_i18n::t;

mod exports;
pub(super) use exports::ExportControls;
mod overrides;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeviceTab {
    Inputs,
    #[default]
    Outputs,
    Applications,
    Exports,
    Clock,
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
            let available = if input {
                input_rows(catalog, snapshot.audio_settings.backend)
            } else {
                output_rows(catalog, snapshot.audio_settings.backend)
            };
            let mut rows = available.to_vec();
            let saved = if input {
                &shell.settings.device.audio.input_overrides
            } else {
                &shell.settings.device.audio.output_overrides
            };
            for id in saved.keys() {
                if !rows.iter().any(|row| &row.endpoint_id == id) {
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
                let selected = if input {
                    state.selection.input_id.as_ref() == Some(&row.endpoint_id)
                } else {
                    state.selection.output_id.as_ref() == Some(&row.endpoint_id)
                };
                let default = if input {
                    catalog.default_input_endpoint_id.as_ref() == Some(&row.endpoint_id)
                } else {
                    catalog.default_output_endpoint_id.as_ref() == Some(&row.endpoint_id)
                };
                let active=snapshot.phase==SessionPhase::Running&&snapshot.desired.as_ref().is_some_and(|s|if input{matches!(&s.source,crate::service::InputSelection::Physical{endpoint_id} if endpoint_id==&row.endpoint_id)}else{s.output_endpoint_id==row.endpoint_id});
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
                        .child(
                            button(SharedString::from(format!("inspect-{id}")), t!("Configure"))
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
                        .when(active, |v| v.child(badge(t!("Active"), theme::SUCCESS)))
                        .child(
                            button(
                                SharedString::from(format!("choose-{id}")),
                                if selected {
                                    t!("Next start")
                                } else {
                                    t!("Use next start")
                                },
                            )
                            .disabled(!present || state.pending_request)
                            .on_click(move |_, _, cx| {
                                this.update(cx, |s, cx| {
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
            if let Some(diagnostic) = snapshot.monitor_diagnostics.filter(|d| {
                d.generation == snapshot.generation && snapshot.phase == SessionPhase::Running
            }) {
                let obs = diagnostic.observation;
                list=list.child(detail(&t!("Input sample rate"),format!("{} Hz",obs.prepared.input.sample_rate)))
                    .child(detail(&t!("Output sample rate"),format!("{} Hz",obs.prepared.output.sample_rate)))
                    .child(detail(&t!("Clock correction"),format!("{:+.2} ppm",obs.correction_ppm)))
                    .child(detail(&t!("Input buffer fill"),format!("{} frames",obs.input_fill_frames)))
                    .child(detail(&t!("Processed"),format!("{} frames",obs.processed_frames)))
                    .child(label(t!("Clock correction and buffer inventory come from the active monitor. Buffer fill is not end-to-end latency.")));
            } else {
                list=list.child(label(t!("No active monitor clock observation. Start a physical input or application session to inspect clock adaptation.")));
            }
        }
    }
    div()
        .id("devices-page")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
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
        .child(div().flex().flex_1().min_h_0().p(px(16.)).child(list))
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
            "Changing resources rebinds the running session immediately."
        )))
        .gpui_test_observe()
        .into_any_element()
}
