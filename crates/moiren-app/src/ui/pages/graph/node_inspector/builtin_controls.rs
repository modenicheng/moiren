//! The builtin processor's controls section: one assembly function per
//! [`crate::service::BuiltinState`] variant, dispatched by
//! [`builtin_controls`]. The channel-mapper's matrix helpers travel with
//! their section; the step rows themselves come from
//! [`super::builtin_fields`].

use rust_i18n::t;

use super::*;
use crate::service::{AppRequest, GraphEdit, RecordingPhase};
use crate::ui::ButtonVariant;
use moiren_core::graph::NodeKind;
use moiren_engine::{
    meter::MeterSettings,
    processor::{ChannelMapperSettings, DelaySettings, EqSettings, GateSettings, LimiterSettings},
    rack::{RackProcessor, RackSettings, RackSlot},
    recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};

use super::builtin_fields::{
    BuiltinStep, builtin_field_step, builtin_step, delay_fields, gate_fields, limiter_fields,
};

const MAX_MATRIX_EDITOR_CHANNELS: usize = 8;

pub(super) fn builtin_controls(
    id: moiren_core::graph::NodeId,
    node: &moiren_core::graph::Node,
    state: &crate::service::BuiltinState,
    snapshot: &crate::service::AppSnapshot,
    shell: &StudioShell,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let mut view = div()
        .id("builtin-controls")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_2));
    match state {
        crate::service::BuiltinState::Eq(settings) => {
            view = eq_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::Gate(settings) => {
            view = gate_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::Limiter(settings) => {
            view = limiter_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::Delay(settings) => {
            view = delay_controls(view, id, *settings, cx);
        }
        crate::service::BuiltinState::ChannelMapper(settings) => {
            view = channel_mapper_controls(view, id, node, settings, cx);
        }
        crate::service::BuiltinState::Recorder(settings) => {
            view = recorder_controls(view, id, settings, snapshot, shell, cx);
        }
        crate::service::BuiltinState::Meter(settings) => {
            view = meter_controls(view, id, settings, cx);
        }
        crate::service::BuiltinState::SpectrumAnalyzer(settings) => {
            view = spectrum_controls(view, id, settings, cx);
        }
        crate::service::BuiltinState::Rack(settings) => {
            view = rack_controls(view, id, settings, cx);
        }
        crate::service::BuiltinState::None => {}
    }
    // The recorder's observation is the session itself, shown above; meters
    // and analyzers still become available only with a live graph.
    if matches!(node.kind(), NodeKind::Meter | NodeKind::SpectrumAnalyzer) {
        view = view.child(label(t!(
            "Managed observation becomes available once the graph runs live."
        )));
    }
    view.into_any_element()
}

fn eq_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: EqSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Parametric EQ")))
        .child(label(t!(
            "One peaking band · apply to prepare the new kernel"
        )))
        .child(builtin_step(
            BuiltinStep {
                key: "eq-frequency".into(),
                label_text: "Frequency",
                value: settings.frequency_hz,
                suffix: "Hz",
                min: 20.,
                max: 20_000.,
                step: 100.,
                node: id,
                build: move |value| {
                    crate::service::BuiltinState::Eq(EqSettings {
                        frequency_hz: value,
                        ..settings
                    })
                },
            },
            cx,
        ))
        .child(builtin_step(
            BuiltinStep {
                key: "eq-gain".into(),
                label_text: "Gain",
                value: settings.gain_db,
                suffix: "dB",
                min: -24.,
                max: 24.,
                step: 1.,
                node: id,
                build: move |value| {
                    crate::service::BuiltinState::Eq(EqSettings {
                        gain_db: value,
                        ..settings
                    })
                },
            },
            cx,
        ))
        .child(builtin_step(
            BuiltinStep {
                key: "eq-q".into(),
                label_text: "Q",
                value: settings.q,
                suffix: "",
                min: 0.1,
                max: 20.,
                step: 0.1,
                node: id,
                build: move |value| {
                    crate::service::BuiltinState::Eq(EqSettings {
                        q: value,
                        ..settings
                    })
                },
            },
            cx,
        ));
    view
}

fn gate_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: GateSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Gate")))
        .child(label(t!("Linked downward expander")));
    for field in gate_fields(settings) {
        view = view.child(builtin_field_step(
            "gate",
            settings,
            field,
            crate::service::BuiltinState::Gate,
            id,
            cx,
        ));
    }
    view
}

fn limiter_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: LimiterSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Limiter")))
        .child(label(t!("Zero-latency peak limiter")));
    for field in limiter_fields(settings) {
        view = view.child(builtin_field_step(
            "limiter",
            settings,
            field,
            crate::service::BuiltinState::Limiter,
            id,
            cx,
        ));
    }
    view
}

fn delay_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: DelaySettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Delay")))
        .child(label(t!("Bounded feedback delay")));
    for field in delay_fields(settings) {
        view = view.child(builtin_field_step(
            "delay",
            settings,
            field,
            crate::service::BuiltinState::Delay,
            id,
            cx,
        ));
    }
    view
}

fn channel_mapper_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    node: &moiren_core::graph::Node,
    settings: &ChannelMapperSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let mapping = if settings.mapping.is_empty() && settings.matrix.is_empty() {
        t!("Identity routing")
    } else if !settings.matrix.is_empty() {
        t!("Weighted matrix")
    } else {
        t!("Explicit channel map")
    };
    view = view
        .child(heading(t!("Channel mapper")))
        .child(detail(&t!("Mode"), mapping))
        .child(label(t!("Static routing is prepared before audio starts.")));
    let input_channels = node.inputs().first().map_or(0, |port| port.channels());
    let output_channels = node.channels();
    if input_channels == 2 && output_channels == 2 {
        view = view.child(button("channel-map-swap", t!("Swap L/R")).on_click(edit(
            cx.entity(),
            GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
                    mapping: vec![1, 0],
                    matrix: Vec::new(),
                }),
            },
        )));
    }
    view =
        view.child(
            button("channel-map-identity", t!("Identity")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::ChannelMapper(
                        ChannelMapperSettings::default(),
                    ),
                },
            )),
        );
    if input_channels <= MAX_MATRIX_EDITOR_CHANNELS && output_channels <= MAX_MATRIX_EDITOR_CHANNELS
    {
        let matrix = channel_mapper_matrix(settings, input_channels, output_channels);
        if settings.matrix.is_empty() {
            view = view.child(
                button("channel-map-edit-matrix", t!("Edit weighted matrix")).on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
                            mapping: Vec::new(),
                            matrix: matrix.clone(),
                        }),
                    },
                )),
            );
        } else {
            view = view
                .child(heading(t!("Weight matrix")))
                .child(detail(
                    &t!("Layout"),
                    t!(
                        "%{outputs} outputs x %{inputs} inputs",
                        outputs = output_channels,
                        inputs = input_channels
                    ),
                ))
                .child(label(t!(
                    "Adjust one output row at a time. Values are clamped to -1..1."
                )));
            for output in 0..output_channels {
                let mut row = div()
                    .id(format!("channel-map-matrix-row-{output}"))
                    .flex()
                    .items_center()
                    .gap(px(theme::SPACE_1));
                row = row.child(label(t!("Out %{index}", index = output + 1)));
                for input in 0..input_channels {
                    let value = matrix[output][input];
                    let down = matrix_cell_edit(id, &matrix, output, input, -0.25);
                    let up = matrix_cell_edit(id, &matrix, output, input, 0.25);
                    row = row.child(
                        div()
                            .id(format!("channel-map-matrix-cell-{output}-{input}"))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(theme::SPACE_0_5))
                            .child(label(t!("In %{index}", index = input + 1)))
                            .child(
                                button(
                                    format!("channel-map-matrix-out-{output}-in-{input}-up"),
                                    "+",
                                )
                                .on_click(edit(cx.entity(), up)),
                            )
                            .child(detail("", format!("{value:+.2}")))
                            .child(
                                button(
                                    format!("channel-map-matrix-out-{output}-in-{input}-down"),
                                    "−",
                                )
                                .on_click(edit(cx.entity(), down)),
                            ),
                    );
                }
                view = view.child(row);
            }
        }
    } else {
        view = view.child(label(
            t!("Matrix editor is limited to 8 input and output channels; use a project file for larger layouts."),
        ));
    }
    view
}

/// The recorder's detail surface: session state, duration, the output file,
/// the directory control, and the buffer — the card strip carries the quick
/// transport, this column carries everything else.
/// FLAC encoding knobs: one compression level walking the 0 (fastest)
/// through 8 (best) presets. Preference-level setting applied to the next
/// start, never to a running session.
fn recorder_flac_controls(shell: &StudioShell, this: &Entity<StudioShell>) -> Stateful<Div> {
    use moiren_engine::recorder::{FLAC_MAX_LEVEL, FlacEncoding};

    let flac = shell.recording_flac();
    let persist = |next: FlacEncoding| {
        let this = this.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            this.update(cx, |shell, cx| {
                shell.preferences.values.recording_flac = next;
                shell.save_preferences_quiet(cx);
                cx.notify();
            });
        }
    };
    let mut section = div()
        .id("recorder-flac")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_2));
    section = section.child(detail(
        &t!("Compression level"),
        t!(
            "%{level} of %{max}",
            level = flac.level,
            max = FLAC_MAX_LEVEL
        )
        .to_string(),
    ));
    section = section.child(
        div()
            .id("recorder-flac-level-hint")
            .text_size(px(theme::TEXT_SM))
            .text_color(theme::TEXT_MUTED)
            .child(t!("0 is fastest, 8 is smallest.").to_owned()),
    );
    let steps = div()
        .id("recorder-flac-steps")
        .flex()
        .flex_row()
        .gap(px(theme::SPACE_2))
        .child(
            button("recorder-flac-level-down", "\u{2212}")
                .disabled(flac.level == 0)
                .on_click(persist(FlacEncoding {
                    level: flac.level.saturating_sub(1),
                })),
        )
        .child(
            button("recorder-flac-level-up", "+")
                .disabled(flac.level == FLAC_MAX_LEVEL)
                .on_click(persist(FlacEncoding {
                    level: (flac.level + 1).min(FLAC_MAX_LEVEL),
                })),
        );
    section = section.child(steps).child(
        div()
            .id("recorder-flac-apply-hint")
            .text_size(px(theme::TEXT_SM))
            .text_color(theme::TEXT_MUTED)
            .child(t!("Applies to the next recording.").to_owned()),
    );
    section
}

/// MP3 encoding knobs: rate-control mode plus the matching bitrate ladder or
/// VBR quality stepper. Preference-level settings applied to the next start,
/// never to a running session.
fn recorder_mp3_controls(shell: &StudioShell, this: &Entity<StudioShell>) -> Stateful<Div> {
    use moiren_engine::recorder::{MP3_BITRATE_KBPS, Mp3Encoding, Mp3Mode};

    let mp3 = shell.recording_mp3();
    let persist = |next: Mp3Encoding| {
        let this = this.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            this.update(cx, |shell, cx| {
                shell.preferences.values.recording_mp3 = next;
                shell.save_preferences_quiet(cx);
                cx.notify();
            });
        }
    };
    let mut section = div()
        .id("recorder-mp3")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_2));
    let mut mode_row = div()
        .id("recorder-mp3-mode")
        .flex()
        .flex_row()
        .gap(px(theme::SPACE_2));
    mode_row = mode_row.child(
        button("recorder-mp3-cbr", t!("CBR"))
            .variant(if matches!(mp3.mode, Mp3Mode::Cbr) {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .on_click(persist(Mp3Encoding {
                mode: Mp3Mode::Cbr,
                ..mp3
            })),
    );
    let quality = match mp3.mode {
        Mp3Mode::Vbr { quality } => quality,
        Mp3Mode::Cbr => 2,
    };
    mode_row = mode_row.child(
        button("recorder-mp3-vbr", t!("VBR"))
            .variant(if matches!(mp3.mode, Mp3Mode::Vbr { .. }) {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .on_click(persist(Mp3Encoding {
                mode: Mp3Mode::Vbr { quality },
                ..mp3
            })),
    );
    section = section.child(mode_row);
    let mut steps = div()
        .id("recorder-mp3-steps")
        .flex()
        .flex_row()
        .gap(px(theme::SPACE_2));
    match mp3.mode {
        Mp3Mode::Cbr => {
            let index = MP3_BITRATE_KBPS
                .iter()
                .position(|&kbps| kbps == mp3.bitrate_kbps)
                .unwrap_or_default();
            let top = MP3_BITRATE_KBPS.len() - 1;
            section = section.child(detail(
                &t!("Bitrate"),
                t!("%{kbps} kbps", kbps = mp3.bitrate_kbps).to_string(),
            ));
            steps = steps
                .child(
                    button("recorder-mp3-bitrate-down", "\u{2212}")
                        .disabled(index == 0)
                        .on_click(persist(Mp3Encoding {
                            bitrate_kbps: MP3_BITRATE_KBPS[index.saturating_sub(1)],
                            ..mp3
                        })),
                )
                .child(
                    button("recorder-mp3-bitrate-up", "+")
                        .disabled(index == top)
                        .on_click(persist(Mp3Encoding {
                            bitrate_kbps: MP3_BITRATE_KBPS[(index + 1).min(top)],
                            ..mp3
                        })),
                );
        }
        Mp3Mode::Vbr { quality } => {
            section = section
                .child(detail(&t!("VBR quality"), quality.to_string()))
                .child(
                    div()
                        .id("recorder-mp3-quality-hint")
                        .text_size(px(theme::TEXT_SM))
                        .text_color(theme::TEXT_MUTED)
                        .child(t!("0 is best.").to_owned()),
                );
            steps = steps
                .child(
                    button("recorder-mp3-quality-down", "\u{2212}")
                        .disabled(quality == 0)
                        .on_click(persist(Mp3Encoding {
                            mode: Mp3Mode::Vbr {
                                quality: quality.saturating_sub(1),
                            },
                            ..mp3
                        })),
                )
                .child(
                    button("recorder-mp3-quality-up", "+")
                        .disabled(quality == 9)
                        .on_click(persist(Mp3Encoding {
                            mode: Mp3Mode::Vbr {
                                quality: (quality + 1).min(9),
                            },
                            ..mp3
                        })),
                );
        }
    }
    section = section.child(steps).child(
        div()
            .id("recorder-mp3-apply-hint")
            .text_size(px(theme::TEXT_SM))
            .text_color(theme::TEXT_MUTED)
            .child(t!("Applies to the next recording.").to_owned()),
    );
    section
}

fn recorder_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &RecorderSettings,
    snapshot: &crate::service::AppSnapshot,
    shell: &StudioShell,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let session = snapshot.recordings.iter().find(|entry| entry.node == id);
    let live = session.is_some_and(|entry| !entry.phase.is_terminal());
    let phase = session.map(|entry| entry.phase);
    let sample_rate = snapshot.audio_settings.processing.sample_rate as usize;
    let this = cx.entity();
    view = view.child(heading(t!("Recorder")));
    if let Some(entry) = session {
        view = view
            .child(detail(
                &t!("Status"),
                crate::ui::presentation::recording_phase_text(Some(entry.phase)),
            ))
            .child(detail(
                &t!("Duration"),
                crate::ui::presentation::recording_duration_text(entry.recorded_secs),
            ));
        if !entry.path.as_os_str().is_empty() {
            view = view.child(detail(&t!("File"), entry.path.display().to_string()));
        }
        if entry.dropped_frames > 0 {
            view = view.child(detail(
                &t!("Dropped frames"),
                entry.dropped_frames.to_string(),
            ));
        }
        if let Some(error) = &entry.error {
            // Raw diagnostic, like the catalog error string: engine errors
            // are not catalog text and never localize.
            view = view.child(
                div()
                    .id("recorder-error")
                    .text_size(px(theme::TEXT_SM))
                    .text_color(theme::DANGER)
                    .child(error.as_str().to_owned()),
            );
        }
    }
    // Transport mirror: the same requests the card's strip dispatches.
    let transport = |request: AppRequest| {
        let this = this.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            this.update(cx, |shell, cx| {
                shell
                    .state
                    .update(cx, |state, _| state.dispatch(request.clone()));
            });
        }
    };
    let mut row = div()
        .id("recorder-transport")
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(theme::SPACE_2));
    if !live {
        // The inspector renders inside the shell's own update; reading the
        // entity back here would double-lease and panic, so preferences
        // travel in through `shell`.
        let directory = shell.recording_output_directory();
        let format = shell.recording_format();
        let mp3 = shell.recording_mp3();
        let flac = shell.recording_flac();
        row = row.child(
            button("recorder-start", t!("Start recording")).on_click(transport(
                AppRequest::StartRecording {
                    node: id,
                    directory,
                    format,
                    mp3,
                    flac,
                },
            )),
        );
    }
    if phase == Some(RecordingPhase::Recording) {
        row = row.child(
            button("recorder-pause", t!("Pause recording"))
                .on_click(transport(AppRequest::PauseRecording { node: id })),
        );
    }
    if phase == Some(RecordingPhase::Paused) {
        row = row.child(
            button("recorder-resume", t!("Resume recording"))
                .on_click(transport(AppRequest::ResumeRecording { node: id })),
        );
    }
    if live {
        row = row.child(
            button("recorder-stop", t!("Stop recording"))
                .on_click(transport(AppRequest::StopRecording { node: id })),
        );
    }
    view = view.child(row);
    // Output directory: a machine-level preference, resolved at start time.
    view = view
        .child(detail(
            &t!("Output directory"),
            shell.recording_output_directory().display().to_string(),
        ))
        .child(
            button("recorder-directory", t!("Change directory…")).on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.change_recording_directory(cx));
                }
            }),
        );
    // Format: like the directory, a machine-level preference resolved at
    // start time. The selected button reflects what the next file uses.
    let format = shell.recording_format();
    let mut format_row = div()
        .id("recorder-format")
        .flex()
        .flex_row()
        .gap(px(theme::SPACE_2));
    format_row = format_row.child(
        button("recorder-format-wav", t!("WAV"))
            .variant(if format == crate::service::RecordingFormat::Wav {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.preferences.values.recording_format =
                            crate::service::RecordingFormat::Wav;
                        shell.save_preferences_quiet(cx);
                        cx.notify();
                    });
                }
            }),
    );
    format_row = format_row.child(
        button("recorder-format-mp3", t!("MP3"))
            .variant(if format == crate::service::RecordingFormat::Mp3 {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.preferences.values.recording_format =
                            crate::service::RecordingFormat::Mp3;
                        shell.save_preferences_quiet(cx);
                        cx.notify();
                    });
                }
            }),
    );
    format_row = format_row.child(
        button("recorder-format-flac", t!("FLAC"))
            .variant(if format == crate::service::RecordingFormat::Flac {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .on_click({
                let this = this.clone();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.preferences.values.recording_format =
                            crate::service::RecordingFormat::Flac;
                        shell.save_preferences_quiet(cx);
                        cx.notify();
                    });
                }
            }),
    );
    view = view.child(format_row);
    // Encoding knobs apply to the next encoded start, like the format itself.
    if format == crate::service::RecordingFormat::Mp3 {
        view = view.child(recorder_mp3_controls(shell, &this));
    }
    if format == crate::service::RecordingFormat::Flac {
        view = view.child(recorder_flac_controls(shell, &this));
    }
    // Buffer: one second steps at the graph's processing rate. A live
    // session would be revoked by the reconfiguration, so the steps lock
    // while one runs.
    let second = sample_rate.max(1);
    view = view
        .child(detail(
            &t!("Buffer"),
            t!("%{frames} frames", frames = settings.capacity_frames).to_string(),
        ))
        .child(
            button("recorder-expand", t!("Add 1 second"))
                .disabled(live)
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::Recorder(RecorderSettings {
                            capacity_frames: settings
                                .capacity_frames
                                .saturating_add(second)
                                .min(RecorderSettings::MAX_CAPACITY_FRAMES),
                        }),
                    },
                )),
        )
        .child(
            button("recorder-shrink", t!("Remove 1 second"))
                .disabled(live)
                .on_click(edit(
                    cx.entity(),
                    GraphEdit::SetBuiltin {
                        node: id,
                        state: crate::service::BuiltinState::Recorder(RecorderSettings {
                            capacity_frames: settings.capacity_frames.saturating_sub(second).max(1),
                        }),
                    },
                )),
        );
    if live {
        view = view.child(label(t!("The buffer is fixed while a recording runs.")));
    }
    view
}

fn meter_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &MeterSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Level meter")))
        .child(detail(
            &t!("Window"),
            t!("%{frames} frames", frames = settings.window_frames).to_string(),
        ))
        .child(
            button("meter-window", t!("Use 2048 frame window")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::Meter(MeterSettings {
                        window_frames: 2048,
                    }),
                },
            )),
        );
    view
}

fn spectrum_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &SpectrumSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    view = view
        .child(heading(t!("Spectrum analyzer")))
        .child(detail(&t!("FFT"), settings.fft_size.to_string()))
        .child(detail(&t!("Hop"), settings.hop_frames.to_string()))
        .child(
            button("spectrum-size", t!("Use 2048 point FFT")).on_click(edit(
                cx.entity(),
                GraphEdit::SetBuiltin {
                    node: id,
                    state: crate::service::BuiltinState::SpectrumAnalyzer(SpectrumSettings {
                        fft_size: 2048,
                        hop_frames: 1024,
                    }),
                },
            )),
        );
    view
}

fn rack_controls(
    mut view: Stateful<Div>,
    id: moiren_core::graph::NodeId,
    settings: &RackSettings,
    cx: &Context<StudioShell>,
) -> Stateful<Div> {
    let next_slot_id = settings
        .slots
        .iter()
        .map(|slot| slot.id)
        .max()
        .map_or(0, |id| id.saturating_add(1));
    let mut next_slots = settings.slots.clone();
    next_slots.push(RackSlot {
        id: next_slot_id,
        bypass: false,
        processor: RackProcessor::Gain { initial: 1.0 },
    });
    let mut next_routes = settings.routes.clone();
    if let Some(route) = next_routes.first_mut() {
        route.slots.push(next_slot_id);
    }
    view = view
        .child(heading(t!("Processor rack")))
        .child(detail(&t!("Stages"), settings.slots.len().to_string()))
        .child(button("rack-add-gain", t!("Add gain stage")).on_click(edit(
            cx.entity(),
            GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::Rack(RackSettings {
                    slots: next_slots,
                    routes: next_routes,
                }),
            },
        )))
        .child(button("rack-clear", t!("Clear stages")).on_click(edit(
            cx.entity(),
            GraphEdit::SetBuiltin {
                node: id,
                state: crate::service::BuiltinState::Rack(RackSettings {
                    slots: Vec::new(),
                    routes: settings.routes.clone(),
                }),
            },
        )));
    view
}

fn channel_mapper_matrix(
    settings: &ChannelMapperSettings,
    input_channels: usize,
    output_channels: usize,
) -> Vec<Vec<f64>> {
    if !settings.matrix.is_empty() {
        return settings.matrix.clone();
    }
    (0..output_channels)
        .map(|output| {
            let source = settings.mapping.get(output).copied().unwrap_or(output);
            (0..input_channels)
                .map(|input| f64::from((input == source) as u8))
                .collect()
        })
        .collect()
}

fn matrix_cell_edit(
    node: moiren_core::graph::NodeId,
    matrix: &[Vec<f64>],
    output: usize,
    input: usize,
    delta: f64,
) -> GraphEdit {
    let mut matrix = matrix.to_vec();
    matrix[output][input] = (matrix[output][input] + delta).clamp(-1., 1.);
    GraphEdit::SetBuiltin {
        node,
        state: crate::service::BuiltinState::ChannelMapper(ChannelMapperSettings {
            mapping: Vec::new(),
            matrix,
        }),
    }
}
