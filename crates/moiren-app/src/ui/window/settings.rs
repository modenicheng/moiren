use rust_i18n::t;

use super::*;
use controls::*;
use gpui_kit::base::FocusTrapElement;
use moiren_engine::audio_config::{
    AudioBackend, AudioSettings, BUFFER_SIZES, ProcessingPrecision, SAMPLE_RATES, SampleFormat,
    StreamSettings,
};
use moiren_engine::node::DeviceMode;

mod fields;
mod model;
#[cfg(test)]
mod model_tests;
mod persistence;
mod transaction;
mod transport;
use fields::choices;
pub(super) use fields::{field, footer, rate_title, stream_editor};
pub(super) use model::{Direction, Draft, Editor, Section, SettingsEditor, device_stream_mut};

pub(super) fn overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let this = cx.entity();
    let close = this.clone();
    let dismiss = this.clone();
    let mut tabs = div().flex().gap(px(6.));
    for (id, title, section) in [
        ("settings-audio", t!("Audio"), Section::Audio),
        (
            "settings-transport",
            t!("Input buffers"),
            Section::Transport,
        ),
        ("settings-appearance", t!("Appearance"), Section::Appearance),
        (
            "settings-diagnostics",
            t!("Diagnostics"),
            Section::Diagnostics,
        ),
    ] {
        let this = this.clone();
        tabs = tabs.child(
            button(id, title)
                .variant(if shell.settings.section == section {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.settings.section = section;
                        cx.notify();
                    })
                }),
        );
    }
    let content = match shell.settings.section {
        Section::Audio => audio(shell, cx),
        Section::Transport => transport::editor(shell, cx),
        Section::Appearance => appearance(shell, cx),
        Section::Diagnostics => diagnostics(shell, cx),
    };
    let height = (f32::from(window.viewport_size().height) - 40.).max(260.);
    div()
        .id("settings-page")
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_any_mouse_down(move |event, window, cx| {
            cx.stop_propagation();
            let viewport = window.viewport_size();
            let dialog_width = 920f32.min(f32::from(viewport.width) * 0.95);
            let dialog_height = (f32::from(viewport.height) - 40.).clamp(260., 800.);
            let left = (f32::from(viewport.width) - dialog_width) * 0.5;
            let top = (f32::from(viewport.height) - dialog_height) * 0.5;
            let inside_dialog = event.position.x >= px(left)
                && event.position.x <= px(left + dialog_width)
                && event.position.y >= px(top)
                && event.position.y <= px(top + dialog_height);
            if !inside_dialog {
                dismiss.update(cx, |shell, cx| shell.close_settings(window, cx));
            }
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("settings-backdrop")
                .absolute()
                .inset_0()
                .bg(theme::OVERLAY_SCRIM)
                .gpui_test_observe(),
        )
        .child(
            div()
                .id("settings-dialog")
                .relative()
                .flex()
                .flex_col()
                .w(px(920.))
                .max_w(relative(0.95))
                .h(px(height.min(800.)))
                .max_h_full()
                .rounded(px(theme::RADIUS_LG))
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .bg(theme::SURFACE)
                .overflow_hidden()
                .role(Role::Dialog)
                .child(
                    div()
                        .id("settings-surface")
                        .size_full()
                        .flex()
                        .flex_col()
                        .track_focus(&shell.settings.focus)
                        .on_key_down(move |event, window, cx| {
                            if event.keystroke.key == "escape" {
                                this.update(cx, |shell, cx| shell.close_settings(window, cx));
                            }
                            cx.stop_propagation();
                        })
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_between()
                                .p(px(20.))
                                .border_b_1()
                                .border_color(theme::BORDER)
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(4.))
                                        .child(heading(t!("Settings")))
                                        .child(label(t!("Audio defaults for this workstation"))),
                                )
                                .child(
                                    button("close-settings", t!("Close"))
                                        .variant(ButtonVariant::Ghost)
                                        .on_click(move |_, window, cx| {
                                            close.update(cx, |shell, cx| {
                                                shell.close_settings(window, cx)
                                            })
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px(px(16.))
                                .py(px(8.))
                                .border_b_1()
                                .border_color(theme::BORDER)
                                .child(tabs),
                        )
                        .child(
                            div()
                                .id("settings-scroll")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .p(px(if shell.settings.global.density == Density::Compact {
                                    16.
                                } else {
                                    20.
                                }))
                                .child(content)
                                .gpui_test_observe(),
                        )
                        .child(footer(shell, Editor::Global, cx))
                        .focus_trap("settings-focus-trap", &shell.settings.focus),
                )
                .gpui_test_observe(),
        )
        .gpui_test_observe()
        .into_any_element()
}

fn audio(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let draft = &shell.settings.global;
    let config = &draft.audio;
    let mut system = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading(t!("Audio system")))
        .child(field(
            &t!("Backend"),
            choices(
                shell,
                Editor::Global,
                "audio-backend",
                config.backend,
                AudioBackend::ALL.map(|value| (value, t!(value.title()).into())),
                |draft, value| draft.audio.backend = value,
                cx,
            ),
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(24.))
                .child(field(
                    &t!("Processing sample rate"),
                    choices(
                        shell,
                        Editor::Global,
                        "processing-rate",
                        config.processing.sample_rate,
                        SAMPLE_RATES.map(|rate| (rate, rate_title(rate))),
                        |draft, value| draft.audio.processing.sample_rate = value,
                        cx,
                    ),
                ))
                .child(field(
                    &t!("Processing precision"),
                    choices(
                        shell,
                        Editor::Global,
                        "processing-precision",
                        config.processing.precision,
                        ProcessingPrecision::ALL.map(|value| (value, t!(value.title()).into())),
                        |draft, value| draft.audio.processing.precision = value,
                        cx,
                    ),
                )),
        )
        .child(field(
            &t!("Processing block (frames / ms)"),
            choices(
                shell,
                Editor::Global,
                "processing-buffer",
                config.processing.block_frames,
                BUFFER_SIZES.map(|value| {
                    (
                        value,
                        fields::buffer_title(value, config.processing.sample_rate),
                    )
                }),
                |draft, value| draft.audio.processing.block_frames = value,
                cx,
            ),
        ));
    system = system.child(label(t!(
        "Processing precision is independent of the device sample format. Buffers are requests; actual periods are reported in Diagnostics."
    )));
    let input = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading(t!("Input defaults")))
        .child(stream_editor(
            shell,
            Editor::Global,
            Direction::Input,
            None,
            cx,
        ));
    let output = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading(t!("Output defaults")))
        .child(stream_editor(
            shell,
            Editor::Global,
            Direction::Output,
            None,
            cx,
        ));
    div().flex().flex_col().gap(px(20.)).child(system)
        .child(div().pt(px(16.)).border_t_1().border_color(theme::BORDER).flex().gap(px(24.)).child(input).child(output))
        .child(label(if config.backend == AudioBackend::Wasapi {
            t!("Shared uses Windows client-format conversion without changing the system mix format. Exclusive requires device support and sole access. Unsupported requests report an error.")
        } else {
            t!("ASIO uses one driver session for input and output. Rate and buffer requests must agree across that session; encoding is determined by the driver. Select a driver in Devices.")
        }))
        .child(label(t!(
            "%{input_count} input / %{output_count} output overrides. Configure individual devices in Devices; overrides keep their complete settings when defaults change.",
            input_count = config.input_overrides.len(),
            output_count = config.output_overrides.len()
        )))
        .when(config == &AudioSettings::default(), |view| view.child(latency_policy(shell, cx)))
        .when(config != &AudioSettings::default(), |view| view.child(label(
            t!("Explicit audio settings use the requested device buffers. Legacy queue/recovery profiles apply to the default WASAPI shared configuration.")
        )))
}

fn language_choices(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let mut row = div().flex().flex_wrap().gap(px(4.));
    for (index, language) in crate::i18n::UiLanguage::ALL.into_iter().enumerate() {
        let this = cx.entity();
        row = row.child(
            button(
                SharedString::from(format!("language-{index}")),
                language.title(),
            )
            .variant(if shell.preferences.language == language {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .disabled(shell.settings.busy())
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| shell.set_language(language, cx));
            }),
        );
    }
    row
}

fn latency_policy(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    use crate::service::AudioLatencyProfile;
    field(
        &t!("Queue / recovery policy"),
        choices(
            shell,
            Editor::Global,
            "latency",
            shell.settings.global.profile,
            [
                AudioLatencyProfile::Compatible,
                AudioLatencyProfile::Balanced,
                AudioLatencyProfile::Low,
            ]
            .map(|profile| (profile, t!(profile.title()).into())),
            |draft, profile| draft.profile = profile,
            cx,
        ),
    )
    .child(label(t!(shell.settings.global.profile.description())))
}

fn appearance(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(heading(t!("Appearance")))
        .child(field(&t!("Language"), language_choices(shell, cx)))
        .child(detail(&t!("Theme"), t!("Dark workstation")))
        .child(field(
            &t!("Density"),
            choices(
                shell,
                Editor::Global,
                "density",
                shell.settings.global.density,
                [
                    (Density::Comfortable, t!("Comfortable").into()),
                    (Density::Compact, t!("Compact").into()),
                ],
                |draft, value| draft.density = value,
                cx,
            ),
        ))
        .child(label(t!(
            "Save to use this density throughout the workstation."
        )))
        .child(heading(t!("Keyboard")))
        .child(detail(&t!("Next / previous control"), "Tab / Shift + Tab"))
        .child(detail(&t!("Activate focused action"), "Enter / Space"))
        .child(detail(&t!("Dismiss settings"), "Escape"))
}

fn diagnostics(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let requested = &snapshot.audio_settings;
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(heading(t!("Saved request")))
        .child(detail(&t!("Backend"), t!(requested.backend.title())))
        .child(detail(&t!("Processing"), processing_summary(requested)))
        .child(detail(
            &t!("Input"),
            stream_summary(requested.backend, requested.input),
        ))
        .child(detail(
            &t!("Output"),
            stream_summary(requested.backend, requested.output),
        ))
        .child(detail(
            &t!("Legacy recovery policy"),
            t!(snapshot.latency_profile.title()),
        ))
        .child(label(t!(
            "Unsaved edits stay in the Audio draft until Save or Restart Audio."
        )))
        .child(detail(
            &t!("Requested process input"),
            transport::summary(snapshot.transport_latency.process),
        ))
        .child(detail(
            &t!("Requested VAD input"),
            transport::summary(snapshot.transport_latency.vad),
        ))
        .child(div().mt(px(12.)).child(heading(t!("Active session"))))
        .child(detail(
            &t!("State"),
            crate::ui::presentation::phase_text(snapshot),
        ));
    if let Some(active) = snapshot
        .active_audio_settings
        .as_ref()
        .filter(|_| snapshot.phase == SessionPhase::Running)
    {
        view = view
            .child(detail(&t!("Active backend"), t!(active.backend.title())))
            .child(detail(&t!("Active processing"), processing_summary(active)))
            .child(detail(
                &t!("Active input request"),
                stream_summary(active.backend, active.input),
            ))
            .child(detail(
                &t!("Active output request"),
                stream_summary(active.backend, active.output),
            ));
    } else {
        view = view.child(label(t!("No active audio configuration")));
    }
    if let Some(active) = snapshot.active_transport_latency {
        view = view
            .child(detail(
                &t!("Active process request"),
                transport::summary(active.process),
            ))
            .child(detail(
                &t!("Active VAD request"),
                transport::summary(active.vad),
            ));
    }
    view = view.child(label(t!(
        "Input targets are estimates. Physical end-to-end latency is unqualified."
    )));
    view = view.child(heading(t!("Actual native timing")));
    for (index, (key, value)) in crate::ui::presentation::native_timing_rows(snapshot)
        .into_iter()
        .enumerate()
    {
        view = view.child(
            detail(&key, value.clone())
                .id(("native-timing-row", index))
                .aria_label(format!("{key}: {value}"))
                .gpui_test_observe(),
        );
    }
    let mut exports = div()
        .id("provider-export-diagnostics")
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(8.))
        .child(heading(t!("Provider export diagnostics")));
    if snapshot.exports.is_empty() {
        exports = exports.child(label(t!("No native export observations")));
    }
    for status in &snapshot.exports {
        let id = SharedString::from(format!("provider-export-{}", status.intent.endpoint_id));
        let mut endpoint = div()
            .id(id.clone())
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(4.));
        for (index, (key, value)) in crate::ui::presentation::export_diagnostic_rows(status)
            .into_iter()
            .enumerate()
        {
            endpoint = endpoint.child(
                detail(&key, value.clone())
                    .id((id.clone(), index))
                    .aria_label(format!("{key}: {value}"))
                    .gpui_test_observe(),
            );
        }
        exports = exports.child(endpoint.gpui_test_observe());
    }
    view = view.child(exports.gpui_test_observe());
    view.child(label(crate::ui::presentation::control_feedback(
        snapshot,
        state.pending_request,
    )))
    .child(detail(
        &t!("Catalog"),
        t!(
            "%{inputs} inputs / %{outputs} outputs / %{asio} ASIO drivers / %{apps} applications",
            inputs = snapshot.catalog.inputs.len(),
            outputs = snapshot.catalog.outputs.len(),
            asio = snapshot.catalog.asio_drivers.len(),
            apps = snapshot.catalog.processes.len()
        ),
    ))
    .when_some(snapshot.catalog.asio_error.clone(), |view, error| {
        view.child(
            div()
                .id("settings-asio-error")
                .text_color(theme::DANGER)
                .child(t!("ASIO discovery failed: %{error}", error = error).to_string())
                .gpui_test_observe(),
        )
    })
    .child(detail(&t!("CPU load"), t!("Unavailable")))
    .child(
        detail(
            &t!("Physical end-to-end latency"),
            t!("Unknown / unqualified"),
        )
        .id("diagnostics-physical-latency")
        .aria_label(format!(
            "{}: {}",
            t!("Physical end-to-end latency"),
            t!("Unknown / unqualified")
        ))
        .gpui_test_observe(),
    )
    .child(label(t!(
        "Queue fill and packet timing do not measure CPU load or physical end-to-end latency."
    )))
    .when_some(shell.project_message.clone(), |view, text| {
        view.child(label(text))
    })
    .child(
        button("quit-application", t!("Quit Moiren"))
            .variant(ButtonVariant::Danger)
            .on_click(|_, _, cx| cx.quit()),
    )
}

pub(super) fn processing_summary(audio: &AudioSettings) -> String {
    format!(
        "{} / {} / {} frames",
        rate_title(audio.processing.sample_rate),
        t!(audio.processing.precision.title()),
        audio.processing.block_frames
    )
}

pub(super) fn stream_summary(backend: AudioBackend, stream: StreamSettings) -> String {
    let encoding = if backend == AudioBackend::Asio {
        t!("Driver encoding").into()
    } else {
        format!(
            "{} / {}",
            if stream.mode == DeviceMode::Shared {
                t!("Shared")
            } else {
                t!("Exclusive")
            },
            t!(stream.format.title())
        )
    };
    format!(
        "{} / {} / {}",
        stream
            .sample_rate
            .map(rate_title)
            .unwrap_or_else(|| t!("Follow processing").into()),
        encoding,
        stream
            .buffer_frames
            .map(|frames| format!("{frames} frames"))
            .unwrap_or_else(|| t!("Driver default").into())
    )
}
