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
        ("settings-audio", "Audio", Section::Audio),
        ("settings-appearance", "Appearance", Section::Appearance),
        ("settings-diagnostics", "Diagnostics", Section::Diagnostics),
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
                                        .child(heading("Settings"))
                                        .child(label("Audio defaults for this workstation")),
                                )
                                .child(
                                    button("close-settings", "Close")
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
        .child(heading("Audio system"))
        .child(field(
            "Backend",
            choices(
                shell,
                Editor::Global,
                "audio-backend",
                config.backend,
                AudioBackend::ALL.map(|value| (value, value.title().into())),
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
                    "Processing sample rate",
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
                    "Processing precision",
                    choices(
                        shell,
                        Editor::Global,
                        "processing-precision",
                        config.processing.precision,
                        ProcessingPrecision::ALL.map(|value| (value, value.title().into())),
                        |draft, value| draft.audio.processing.precision = value,
                        cx,
                    ),
                )),
        )
        .child(field(
            "Processing block (frames / ms)",
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
    system = system.child(label("Processing precision is independent of the device sample format. Buffers are requests; actual periods are reported in Diagnostics."));
    let input = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(heading("Input defaults"))
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
        .child(heading("Output defaults"))
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
            "Shared uses Windows client-format conversion without changing the system mix format. Exclusive requires device support and sole access. Unsupported requests report an error."
        } else {
            "ASIO uses one driver session for input and output. Rate and buffer requests must agree across that session; encoding is determined by the driver. Select a driver in Devices."
        }))
        .child(label(format!("{} input / {} output overrides. Configure individual devices in Devices; overrides keep their complete settings when defaults change.", config.input_overrides.len(), config.output_overrides.len())))
        .when(config == &AudioSettings::default(), |view| view.child(latency_policy(shell, cx)))
        .when(config != &AudioSettings::default(), |view| view.child(label(
            "Explicit audio settings use the requested device buffers. Legacy queue/recovery profiles apply to the default WASAPI shared configuration."
        )))
}

fn latency_policy(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    use crate::service::AudioLatencyProfile;
    field(
        "Queue / recovery policy",
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
            .map(|profile| (profile, profile.title().into())),
            |draft, profile| draft.profile = profile,
            cx,
        ),
    )
    .child(label(shell.settings.global.profile.description()))
}

fn appearance(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(heading("Appearance"))
        .child(detail("Theme", "Dark workstation"))
        .child(field(
            "Density",
            choices(
                shell,
                Editor::Global,
                "density",
                shell.settings.global.density,
                [
                    (Density::Comfortable, "Comfortable".into()),
                    (Density::Compact, "Compact".into()),
                ],
                |draft, value| draft.density = value,
                cx,
            ),
        ))
        .child(label(
            "Save to use this density throughout the workstation.",
        ))
        .child(heading("Keyboard"))
        .child(detail("Next / previous control", "Tab / Shift + Tab"))
        .child(detail("Activate focused action", "Enter / Space"))
        .child(detail("Dismiss settings", "Escape"))
}

fn diagnostics(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let requested = &snapshot.audio_settings;
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(heading("Saved request"))
        .child(detail("Backend", requested.backend.title()))
        .child(detail("Processing", processing_summary(requested)))
        .child(detail(
            "Input",
            stream_summary(requested.backend, requested.input),
        ))
        .child(detail(
            "Output",
            stream_summary(requested.backend, requested.output),
        ))
        .child(detail(
            "Legacy recovery policy",
            snapshot.latency_profile.title(),
        ))
        .child(label(
            "Unsaved edits stay in the Audio draft until Save or Restart Audio.",
        ))
        .child(div().mt(px(12.)).child(heading("Active session")))
        .child(detail("State", state.phase_text()));
    if let Some(active) = snapshot
        .active_audio_settings
        .as_ref()
        .filter(|_| snapshot.phase == SessionPhase::Running)
    {
        view = view
            .child(detail("Active backend", active.backend.title()))
            .child(detail("Active processing", processing_summary(active)))
            .child(detail(
                "Active input request",
                stream_summary(active.backend, active.input),
            ))
            .child(detail(
                "Active output request",
                stream_summary(active.backend, active.output),
            ));
    } else {
        view = view.child(label("No active audio configuration"));
    }
    view = view.child(heading("Actual native timing"));
    for (key, value) in state.native_timing_rows() {
        view = view.child(detail(&key, value));
    }
    view.child(label(state.control_feedback()))
        .child(detail("Catalog", format!("{} inputs / {} outputs / {} ASIO drivers / {} applications",
            snapshot.catalog.inputs.len(), snapshot.catalog.outputs.len(), snapshot.catalog.asio_drivers.len(), snapshot.catalog.processes.len())))
        .when_some(snapshot.catalog.asio_error.clone(), |view, error| view.child(
            div().id("settings-asio-error").text_color(theme::DANGER).child(format!("ASIO discovery failed: {error}")).gpui_test_observe()
        ))
        .child(detail("CPU / physical latency / XRUN", "Unavailable"))
        .child(label("Queue fill and callback timing do not measure CPU load, physical end-to-end latency or XRUN counts."))
        .when_some(shell.project_message.clone(), |view, text| view.child(label(text)))
        .child(button("quit-application", "Quit Moiren").variant(ButtonVariant::Danger).on_click(|_, _, cx| cx.quit()))
}

pub(super) fn processing_summary(audio: &AudioSettings) -> String {
    format!(
        "{} / {} / {} frames",
        rate_title(audio.processing.sample_rate),
        audio.processing.precision.title(),
        audio.processing.block_frames
    )
}

pub(super) fn stream_summary(backend: AudioBackend, stream: StreamSettings) -> String {
    let encoding = if backend == AudioBackend::Asio {
        "Driver encoding".into()
    } else {
        format!(
            "{} / {}",
            if stream.mode == DeviceMode::Shared {
                "Shared"
            } else {
                "Exclusive"
            },
            stream.format.title()
        )
    };
    format!(
        "{} / {} / {}",
        stream
            .sample_rate
            .map(rate_title)
            .unwrap_or_else(|| "Follow processing".into()),
        encoding,
        stream
            .buffer_frames
            .map(|frames| format!("{frames} frames"))
            .unwrap_or_else(|| "Driver default".into())
    )
}
