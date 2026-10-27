use rust_i18n::t;

use super::*;

pub(in super::super) fn choices<T: Copy + PartialEq + 'static>(
    shell: &StudioShell,
    editor: Editor,
    id: &str,
    selected: T,
    options: impl IntoIterator<Item = (T, String)>,
    set: impl Fn(&mut Draft, T) + 'static,
    cx: &Context<StudioShell>,
) -> Div {
    let mut row = div().flex().flex_wrap().gap(px(4.));
    let set = std::rc::Rc::new(set);
    for (index, (value, title)) in options.into_iter().enumerate() {
        let this = cx.entity();
        let set = set.clone();
        row = row.child(
            button(SharedString::from(format!("{id}-{index}")), title)
                .density(shell.settings.draft(editor).density)
                .variant(if value == selected {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .disabled(shell.settings.busy())
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        let draft = shell.settings.draft_mut(editor);
                        if draft.pending.is_none() {
                            set(draft, value);
                            draft.changed();
                        }
                        cx.notify();
                    });
                }),
        );
    }
    row
}

pub(in super::super) fn field(title: &str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(label(title.to_owned()))
        .child(value)
}

pub(in super::super) fn rate_title(rate: u32) -> String {
    format!("{} kHz", rate as f32 / 1000.)
}

pub(in super::super) fn buffer_title(frames: u32, rate: u32) -> String {
    format!("{frames} / {:.2} ms", frames as f64 * 1000. / rate as f64)
}

pub(in super::super) fn stream_editor(
    shell: &StudioShell,
    editor: Editor,
    direction: Direction,
    device: Option<&str>,
    cx: &Context<StudioShell>,
) -> Div {
    let draft = shell.settings.draft(editor);
    let audio = &draft.audio;
    let stream = match (direction, device) {
        (Direction::Input, Some(id)) => audio.effective_input(id),
        (Direction::Output, Some(id)) => audio.effective_output(id),
        (Direction::Input, None) => audio.input,
        (Direction::Output, None) => audio.output,
    };
    let prefix = match (direction, device) {
        (Direction::Input, None) => "default-input",
        (Direction::Output, None) => "default-output",
        (Direction::Input, Some(_)) => "device-input",
        (Direction::Output, Some(_)) => "device-output",
    };
    let update = std::rc::Rc::new(StreamTarget {
        direction,
        device: device.map(str::to_owned),
    });
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(if draft.density == Density::Compact {
            10.
        } else {
            14.
        }));
    if audio.backend == AudioBackend::Wasapi {
        let update = update.clone();
        view = view.child(field(
            &t!("WASAPI mode"),
            choices(
                shell,
                editor,
                &format!("{prefix}-mode"),
                stream.mode,
                [
                    (DeviceMode::Shared, t!("Shared").into()),
                    (DeviceMode::Exclusive, t!("Exclusive").into()),
                ],
                move |draft, value| update.stream_mut(draft).mode = value,
                cx,
            ),
        ));
    }
    let setter = update.clone();
    view = view.child(field(
        &t!("Requested sample rate"),
        choices(
            shell,
            editor,
            &format!("{prefix}-rate"),
            stream.sample_rate,
            std::iter::once((None, t!("Follow processing").into()))
                .chain(SAMPLE_RATES.map(|rate| (Some(rate), rate_title(rate)))),
            move |draft, value| setter.stream_mut(draft).sample_rate = value,
            cx,
        ),
    ));
    if audio.backend == AudioBackend::Wasapi {
        let setter = update.clone();
        view = view.child(field(
            &t!("Client sample format"),
            choices(
                shell,
                editor,
                &format!("{prefix}-format"),
                stream.format,
                SampleFormat::ALL.map(|value| (value, t!(value.title()).into())),
                move |draft, value| setter.stream_mut(draft).format = value,
                cx,
            ),
        ));
    } else {
        view = view.child(detail(
            &t!("Sample encoding"),
            t!("Determined by ASIO driver"),
        ));
    }
    view.child(field(
        &t!("Requested device buffer (frames / ms)"),
        choices(
            shell,
            editor,
            &format!("{prefix}-buffer"),
            stream.buffer_frames,
            std::iter::once((None, t!("Driver default").into())).chain(BUFFER_SIZES.map(
                |frames| {
                    (
                        Some(frames),
                        buffer_title(
                            frames,
                            stream.sample_rate.unwrap_or(audio.processing.sample_rate),
                        ),
                    )
                },
            )),
            move |draft, value| update.stream_mut(draft).buffer_frames = value,
            cx,
        ),
    ))
}

pub(in super::super) fn footer(
    shell: &StudioShell,
    editor: Editor,
    cx: &Context<StudioShell>,
) -> Div {
    let draft = shell.settings.draft(editor);
    let state = shell.state.read(cx);
    let running = state.snapshot.phase == SessionPhase::Running;
    let needs_restart = state.snapshot.active_audio_settings.as_ref() != Some(&draft.audio)
        || state.snapshot.active_latency_profile
            != super::model::expected_active_profile(draft.profile);
    let busy = shell.settings.busy()
        || state.pending_request
        || state.snapshot.graph.pending_revision.is_some()
        || matches!(
            state.snapshot.phase,
            SessionPhase::Starting
                | SessionPhase::Stopping
                | SessionPhase::Exiting
                | SessionPhase::Exited
        );
    let prefix = if editor == Editor::Global {
        "settings"
    } else {
        "device-settings"
    };
    let this = cx.entity();
    let restart_this = this.clone();
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(16.))
        .border_t_1()
        .border_color(theme::BORDER)
        .bg(theme::SURFACE)
        .child(
            div()
                .id(SharedString::from(format!("{prefix}-feedback")))
                .text_size(px(theme::TEXT_SM))
                .text_color(if draft.error.is_some() { theme::DANGER } else { theme::TEXT_MUTED })
                .child(draft.error.clone().or_else(|| draft.message.clone()).unwrap_or_else(|| {
                    if draft.dirty() { t!("Unsaved changes").into() }
                    else if running { t!("Saved requests and active audio are shown separately in Diagnostics.").into() }
                    else { t!("Audio is stopped. Save keeps it stopped.").into() }
                }))
                .gpui_test_observe(),
        )
        .child(
            div().flex().flex_wrap().gap(px(8.)).justify_end()
                .child(
                    button(SharedString::from(format!("{prefix}-discard")), t!("Discard changes"))
                        .disabled(busy || !draft.dirty())
                        .on_click(move |_, _, cx| this.update(cx, |shell, cx| {
                            *shell.settings.draft_mut(editor) = shell.current_settings_draft(cx);
                            cx.notify();
                        })),
                )
                .child(
                    button(SharedString::from(format!("{prefix}-save")), t!("Save"))
                        .variant(if running { ButtonVariant::Secondary } else { ButtonVariant::Primary })
                        .disabled(busy || !draft.dirty())
                        .on_click({
                            let this = cx.entity();
                            move |_, _, cx| this.update(cx, |shell, cx| shell.save_audio_settings(editor, false, cx))
                        }),
                )
                .when(running, |view| view.child(
                    button(SharedString::from(format!("{prefix}-restart")), t!("Restart Audio"))
                        .variant(ButtonVariant::Primary)
                        .disabled(busy || !needs_restart)
                        .on_click(move |_, _, cx| restart_this.update(cx, |shell, cx| shell.save_audio_settings(editor, true, cx))),
                )),
        )
}

struct StreamTarget {
    direction: Direction,
    device: Option<String>,
}

impl StreamTarget {
    fn stream_mut<'a>(&self, draft: &'a mut Draft) -> &'a mut StreamSettings {
        match &self.device {
            Some(id) => device_stream_mut(&mut draft.audio, self.direction, id),
            None => match self.direction {
                Direction::Input => &mut draft.audio.input,
                Direction::Output => &mut draft.audio.output,
            },
        }
    }
}
