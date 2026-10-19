use super::*;
use crate::ui::state::{SourceMode, input_rows, output_name, output_rows, source_name};
use controls::*;

mod overrides;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeviceTab {
    Inputs,
    #[default]
    Outputs,
    Applications,
    Clock,
}

pub(super) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let catalog = &snapshot.catalog;
    let asio = snapshot.audio_settings.backend == moiren_engine::audio_config::AudioBackend::Asio;
    let mut tabs = div().flex().items_center().gap(px(6.));
    for (id, title, tab) in [
        ("devices-inputs", "Inputs", DeviceTab::Inputs),
        ("devices-outputs", "Outputs", DeviceTab::Outputs),
        ("devices-apps", "Applications", DeviceTab::Applications),
        ("devices-clock", "Clock", DeviceTab::Clock),
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
                        name: "Unavailable device (saved override)".into(),
                    });
                }
            }
            if rows.is_empty() {
                list = list
                    .child(heading("No devices available"))
                    .child(label(if asio {
                        "Refresh to discover installed ASIO drivers."
                    } else {
                        "Refresh to discover connected Windows audio endpoints."
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
                        .when(!present, |v| v.child(badge("Unavailable", theme::WARNING)))
                        .when(saved.contains_key(&id), |v| {
                            v.child(badge("Override", theme::PRIMARY))
                        })
                        .when(default && !asio, |v| {
                            v.child(badge("Default", theme::TEXT_MUTED))
                        })
                        .child(
                            button(SharedString::from(format!("inspect-{id}")), "Configure")
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
                        .when(active, |v| v.child(badge("Active", theme::SUCCESS)))
                        .child(
                            button(
                                SharedString::from(format!("choose-{id}")),
                                if selected {
                                    "Next start"
                                } else {
                                    "Use next start"
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
                list = list.child(label("Application loopback requires WASAPI. Choose WASAPI in Settings to use application sources."));
            }
            if catalog.processes.is_empty() {
                list=list.child(heading("No capturable applications")).child(label("Start an application that plays audio, then refresh. Protected applications may be unavailable."));
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
                                .child(label(format!(
                                    "PID {} · created {}",
                                    process.pid, process.creation_time_100ns
                                ))),
                        )
                        .child(
                            button(
                                SharedString::from(format!("process-{}-{}", key.0, key.1)),
                                if state.selection.process_key == Some(key) {
                                    "Next start"
                                } else {
                                    "Use next start"
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
            list = list.child(heading("Active audio clocks"));
            if let Some(diagnostic) = snapshot.monitor_diagnostics.filter(|d| {
                d.generation == snapshot.generation && snapshot.phase == SessionPhase::Running
            }) {
                let obs = diagnostic.observation;
                list=list.child(detail("Input sample rate",format!("{} Hz",obs.prepared.input.sample_rate)))
                    .child(detail("Output sample rate",format!("{} Hz",obs.prepared.output.sample_rate)))
                    .child(detail("Clock correction",format!("{:+.2} ppm",obs.correction_ppm)))
                    .child(detail("Input buffer fill",format!("{} frames",obs.input_fill_frames)))
                    .child(detail("Processed",format!("{} frames",obs.processed_frames)))
                    .child(label("Clock correction and buffer inventory come from the active monitor. Buffer fill is not end-to-end latency."));
            } else {
                list=list.child(label("No active monitor clock observation. Start a physical input or application session to inspect clock adaptation."));
            }
        }
    }
    let mut inspector = panel("device-inspector").child(heading("Session resources"));
    if let Some(active) = snapshot
        .desired
        .as_ref()
        .filter(|_| snapshot.phase == SessionPhase::Running)
    {
        inspector = inspector
            .child(badge("Active session", theme::SUCCESS))
            .child(detail("Source", source_name(snapshot, &active.source)))
            .child(detail(
                "Output",
                output_name(snapshot, &active.output_endpoint_id),
            ));
    } else {
        inspector = inspector.child(label("No active session"));
    }
    inspector=inspector.child(badge("Next start",theme::WARNING))
        .child(detail("Source mode",state.source_mode.title()))
        .child(detail("Source",state.candidate_spec().map(|s|source_name(snapshot,&s.source)).unwrap_or_else(||"Select available resources".into())))
        .child(detail("Output",state.selection.output_id.as_ref().map(|id|output_name(snapshot,id)).unwrap_or_else(||"Not selected".into())))
        .child(label("Selecting another resource leaves the running session unchanged. Stop and start to switch devices."));
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
                .child(tabs)
                .child(inspector_toggle("toggle-details", cx))
                .child(
                    button("refresh-devices", "Refresh devices")
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
                    .child(format!(
                        "ASIO discovery failed: {error}. Refresh devices to retry."
                    ))
                    .gpui_test_observe(),
            )
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .p(px(16.))
                .gap(px(16.))
                .child(list)
                .when(shell.inspector_open, |v| {
                    if matches!(shell.device_tab, DeviceTab::Inputs | DeviceTab::Outputs)
                        && shell.selected_device.is_some()
                    {
                        v.child(overrides::inspector(shell, cx))
                    } else {
                        v.child(inspector)
                    }
                }),
        )
        .gpui_test_observe()
        .into_any_element()
}
