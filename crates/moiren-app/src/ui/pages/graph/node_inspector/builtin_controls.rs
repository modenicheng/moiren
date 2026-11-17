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

/// The recorder's detail surface: the session header (the card strip's
/// status line, mirrored), the transport mirror, and the SESSION / OUTPUT /
/// BUFFER sections. The card carries the quick transport; this column
/// carries everything else. Every control keeps its hit-test id, so tests
/// and the paint agree.
///
/// The step rows share one rhythm with the builtin field rows: label left,
/// value and the ± steppers right.
fn recorder_step_row(
    key: &'static str,
    label_text: impl Into<SharedString>,
    value: String,
    down: ActionButton,
    up: ActionButton,
) -> Stateful<Div> {
    div()
        .id(key)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACE_1_5))
        .child(label(label_text))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_1))
                .child(down)
                .child(detail("", value))
                .child(up),
        )
}

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
    div()
        .id("recorder-flac")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_1_5))
        .child(recorder_step_row(
            "recorder-flac-level",
            t!("Compression level"),
            t!(
                "%{level} of %{max}",
                level = flac.level,
                max = FLAC_MAX_LEVEL
            )
            .to_string(),
            button("recorder-flac-level-down", "\u{2212}")
                .disabled(flac.level == 0)
                .on_click(persist(FlacEncoding {
                    level: flac.level.saturating_sub(1),
                })),
            button("recorder-flac-level-up", "+")
                .disabled(flac.level == FLAC_MAX_LEVEL)
                .on_click(persist(FlacEncoding {
                    level: (flac.level + 1).min(FLAC_MAX_LEVEL),
                })),
        ))
        .child(
            div()
                .id("recorder-flac-level-hint")
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT_MUTED)
                .child(t!("0 is fastest, 8 is smallest.").to_owned()),
        )
}

/// MP3 encoding knobs: the rate-control mode as one segmented selector, then
/// the matching bitrate ladder or VBR quality stepper. Preference-level
/// settings applied to the next start, never to a running session.
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
    let quality = match mp3.mode {
        Mp3Mode::Vbr { quality } => quality,
        Mp3Mode::Cbr => 2,
    };
    let mut section = div()
        .id("recorder-mp3")
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_1_5));
    let mut mode_row = div().id("recorder-mp3-mode").flex().gap(px(theme::SPACE_1));
    mode_row = mode_row.child(
        div().flex_1().child(
            button("recorder-mp3-cbr", t!("CBR"))
                .variant(if matches!(mp3.mode, Mp3Mode::Cbr) {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .full_width()
                .on_click(persist(Mp3Encoding {
                    mode: Mp3Mode::Cbr,
                    ..mp3
                })),
        ),
    );
    mode_row = mode_row.child(
        div().flex_1().child(
            button("recorder-mp3-vbr", t!("VBR"))
                .variant(if matches!(mp3.mode, Mp3Mode::Vbr { .. }) {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .full_width()
                .on_click(persist(Mp3Encoding {
                    mode: Mp3Mode::Vbr { quality },
                    ..mp3
                })),
        ),
    );
    section = section.child(mode_row);
    match mp3.mode {
        Mp3Mode::Cbr => {
            let index = MP3_BITRATE_KBPS
                .iter()
                .position(|&kbps| kbps == mp3.bitrate_kbps)
                .unwrap_or_default();
            let top = MP3_BITRATE_KBPS.len() - 1;
            section = section.child(recorder_step_row(
                "recorder-mp3-bitrate",
                t!("Bitrate"),
                t!("%{kbps} kbps", kbps = mp3.bitrate_kbps).to_string(),
                button("recorder-mp3-bitrate-down", "\u{2212}")
                    .disabled(index == 0)
                    .on_click(persist(Mp3Encoding {
                        bitrate_kbps: MP3_BITRATE_KBPS[index.saturating_sub(1)],
                        ..mp3
                    })),
                button("recorder-mp3-bitrate-up", "+")
                    .disabled(index == top)
                    .on_click(persist(Mp3Encoding {
                        bitrate_kbps: MP3_BITRATE_KBPS[(index + 1).min(top)],
                        ..mp3
                    })),
            ));
        }
        Mp3Mode::Vbr { quality } => {
            section = section.child(recorder_step_row(
                "recorder-mp3-quality",
                t!("VBR quality"),
                quality.to_string(),
                button("recorder-mp3-quality-down", "\u{2212}")
                    .disabled(quality == 0)
                    .on_click(persist(Mp3Encoding {
                        mode: Mp3Mode::Vbr {
                            quality: quality.saturating_sub(1),
                        },
                        ..mp3
                    })),
                button("recorder-mp3-quality-up", "+")
                    .disabled(quality == 9)
                    .on_click(persist(Mp3Encoding {
                        mode: Mp3Mode::Vbr {
                            quality: (quality + 1).min(9),
                        },
                        ..mp3
                    })),
            ));
            section = section.child(
                div()
                    .id("recorder-mp3-quality-hint")
                    .text_size(px(theme::TEXT_SM))
                    .text_color(theme::TEXT_MUTED)
                    .child(t!("0 is best.").to_owned()),
            );
        }
    }
    section
}

/// One recorder section: a hairline rule over its uppercase caption, then
/// the section's rows SPACE_1_5 apart. The rule keeps SPACE_3
/// of air above it — the column's own SPACE_2 gap plus this top padding.
fn recorder_group(id: &'static str, title: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .pt(px(theme::SPACE_1))
        .gap(px(theme::SPACE_1_5))
        .child(div().w_full().h(px(theme::HAIRLINE_W)).bg(theme::HAIRLINE))
        .child(
            div()
                .id(id)
                .gpui_test_observe()
                .text_size(px(theme::TEXT_TINY))
                .text_color(theme::TEXT_FAINT)
                .child(title.to_owned()),
        )
}

/// The failed session's diagnostic: the raw engine error never
/// localizes, like the catalog error string, so the wash container carries
/// the danger styling the bare text used to lack.
fn recorder_error_block(error: &str) -> impl IntoElement {
    div()
        .id("recorder-error")
        .gpui_test_observe()
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::DANGER_WASH)
        .p(px(theme::SPACE_2))
        .text_size(px(theme::TEXT_SM))
        .text_color(theme::DANGER)
        .child(error.to_owned())
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
    // Session header: the card strip's status line, mirrored. The dot's
    // color is the phase's one-word summary; the clock is the live datum,
    // same size and mono family as the card strip's clock.
    let (dot, word) = match phase {
        None => (theme::TEXT_FAINT, theme::TEXT_MUTED),
        Some(RecordingPhase::Recording) => (theme::DANGER, theme::TEXT),
        Some(RecordingPhase::Paused) => (theme::WARNING, theme::WARNING),
        Some(RecordingPhase::Rebinding) | Some(RecordingPhase::Stopping) => {
            (theme::WARNING, theme::TEXT_MUTED)
        }
        Some(RecordingPhase::Finished) => (theme::TEXT_FAINT, theme::TEXT_MUTED),
        Some(RecordingPhase::Failed) => (theme::DANGER, theme::DANGER),
    };
    let mut status = div()
        .id("recorder-status")
        .gpui_test_observe()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACE_3))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::SPACE_2))
                .child(
                    div()
                        .w(px(theme::card::REC_DOT + 1.))
                        .h(px(theme::card::REC_DOT + 1.))
                        .rounded_full()
                        .bg(dot),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_MD))
                        .text_color(word)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(crate::ui::presentation::recording_phase_text(phase)),
                ),
        );
    if let Some(entry) = session {
        status = status.child(
            div()
                .text_size(px(theme::card::REC_CLOCK_SIZE))
                .text_color(theme::TEXT)
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(theme::FONT_MONO)
                .child(crate::ui::presentation::recording_duration_text(
                    entry.recorded_secs,
                )),
        );
    }
    view = view.child(status);
    if let Some(error) = session.and_then(|entry| entry.error.as_ref()) {
        view = view.child(recorder_error_block(error.as_str()));
    }
    // Transport mirror, up front after the status: the quick actions sit
    // before the sections so start/pause/stop are one glance away. The icons
    // put the transport's vocabulary where the eye already looks for it; the
    // destructive stop wears the danger skin.
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
        .gpui_test_observe()
        .flex()
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
            div().flex_1().child(
                button("recorder-start", t!("Start recording"))
                    .variant(ButtonVariant::Primary)
                    .icon(ActionIcon::Record)
                    .full_width()
                    .on_click(transport(AppRequest::StartRecording {
                        node: id,
                        directory,
                        format,
                        mp3,
                        flac,
                    })),
            ),
        );
    }
    if phase == Some(RecordingPhase::Recording) {
        row = row.child(
            div().flex_1().child(
                button("recorder-pause", t!("Pause recording"))
                    .icon(ActionIcon::Pause)
                    .full_width()
                    .on_click(transport(AppRequest::PauseRecording { node: id })),
            ),
        );
    }
    if phase == Some(RecordingPhase::Paused) {
        row = row.child(
            div().flex_1().child(
                button("recorder-resume", t!("Resume recording"))
                    .variant(ButtonVariant::Primary)
                    .icon(ActionIcon::Play)
                    .full_width()
                    .on_click(transport(AppRequest::ResumeRecording { node: id })),
            ),
        );
    }
    if live {
        row = row.child(
            div().flex_1().child(
                button("recorder-stop", t!("Stop recording"))
                    .variant(ButtonVariant::Danger)
                    .icon(ActionIcon::Stop)
                    .full_width()
                    .on_click(transport(AppRequest::StopRecording { node: id })),
            ),
        );
    }
    view = view.child(row);
    // SESSION: the session's own rows — its file and how much the engine
    // had to drop. The header itself always renders, so the section
    // skeleton stays put whether or not a session has run yet.
    let mut session_group = recorder_group("recorder-group-session", t!("SESSION").as_ref());
    if let Some(entry) = session {
        if !entry.path.as_os_str().is_empty() {
            session_group =
                session_group.child(detail(&t!("File"), entry.path.display().to_string()));
        }
        if entry.dropped_frames > 0 {
            session_group = session_group.child(detail(
                &t!("Dropped frames"),
                entry.dropped_frames.to_string(),
            ));
        }
    }
    view = view.child(session_group);
    // OUTPUT: directory, format, and the encoding knobs — every
    // machine-level preference, all resolved at start time.
    let mut output = recorder_group("recorder-group-output", t!("OUTPUT").as_ref());
    // The path needs the column's whole width to wrap legibly, so the change
    // action stays on its own row beneath it.
    output = output.child(detail(
        &t!("Output directory"),
        shell.recording_output_directory().display().to_string(),
    ));
    output = output.child(
        button("recorder-directory", t!("Change directory…")).on_click({
            let this = this.clone();
            move |_, _, cx| {
                this.update(cx, |shell, cx| shell.change_recording_directory(cx));
            }
        }),
    );
    // Format: one segmented selector; the selected option carries the
    // primary skin, and the encoding knobs mount beneath it.
    let format = shell.recording_format();
    let mut format_row = div().id("recorder-format").flex().gap(px(theme::SPACE_1));
    for (key, selected, next_format) in [
        (
            "recorder-format-wav",
            format == crate::service::RecordingFormat::Wav,
            crate::service::RecordingFormat::Wav,
        ),
        (
            "recorder-format-mp3",
            format == crate::service::RecordingFormat::Mp3,
            crate::service::RecordingFormat::Mp3,
        ),
        (
            "recorder-format-flac",
            format == crate::service::RecordingFormat::Flac,
            crate::service::RecordingFormat::Flac,
        ),
    ] {
        let label = match next_format {
            crate::service::RecordingFormat::Wav => t!("WAV"),
            crate::service::RecordingFormat::Mp3 => t!("MP3"),
            crate::service::RecordingFormat::Flac => t!("FLAC"),
        };
        let this = this.clone();
        format_row = format_row.child(
            div().flex_1().child(
                button(key, label)
                    .variant(if selected {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .full_width()
                    .on_click(
                        move |_: &gpui_kit::ClickEvent,
                              _: &mut gpui_kit::Window,
                              cx: &mut gpui_kit::App| {
                            this.update(cx, |shell, cx| {
                                shell.preferences.values.recording_format = next_format;
                                shell.save_preferences_quiet(cx);
                                cx.notify();
                            });
                        },
                    ),
            ),
        );
    }
    output = output.child(format_row);
    // Encoding knobs apply to the next encoded start, like the format itself.
    if format == crate::service::RecordingFormat::Mp3 {
        output = output.child(recorder_mp3_controls(shell, &this));
    }
    if format == crate::service::RecordingFormat::Flac {
        output = output.child(recorder_flac_controls(shell, &this));
    }
    if format != crate::service::RecordingFormat::Wav {
        // One apply hint closes the format group: the format and the knobs
        // above are preference-level settings, never mid-session changes.
        output = output.child(
            div()
                .id(if format == crate::service::RecordingFormat::Mp3 {
                    "recorder-mp3-apply-hint"
                } else {
                    "recorder-flac-apply-hint"
                })
                .text_size(px(theme::TEXT_SM))
                .text_color(theme::TEXT_MUTED)
                .child(t!("Applies to the next recording.").to_owned()),
        );
    }
    view = view.child(output);
    // BUFFER: one second steps at the graph's processing rate, seconds as
    // the primary reading. A live session would be revoked by the
    // reconfiguration, so the steps lock while one runs.
    let second = sample_rate.max(1);
    let mut buffer = recorder_group("recorder-group-buffer", t!("BUFFER").as_ref());
    buffer = buffer.child(recorder_step_row(
        "recorder-buffer",
        t!("Buffer"),
        crate::ui::presentation::buffer_summary_text(
            settings.capacity_frames as u32,
            sample_rate as u32,
        ),
        button("recorder-shrink", "\u{2212}")
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
        button("recorder-expand", "+").disabled(live).on_click(edit(
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
    ));
    if live {
        buffer = buffer.child(label(t!("The buffer is fixed while a recording runs.")));
    }
    view = view.child(buffer);
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
