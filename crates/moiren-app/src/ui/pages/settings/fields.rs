use rust_i18n::t;

use super::menu::ValueField;
use super::*;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A connected segmented group: the shell carries the inset well and its
/// border, the segments stay square so the selected fill reads as one
/// control rather than a row of separate buttons.
fn segmented_group(
    shell: &StudioShell,
    id: &str,
    disabled: bool,
    items: Vec<(String, bool, ClickHandler)>,
) -> Div {
    let mut group = div()
        .flex()
        .items_stretch()
        .max_w_full()
        .self_start()
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(theme::BORDER)
        .bg(theme::WINDOW_BG)
        .overflow_hidden();
    for (index, (title, selected, handler)) in items.into_iter().enumerate() {
        group = group.child(
            button(SharedString::from(format!("{id}-{index}")), title)
                .density(shell.chrome.density)
                .variant(if selected {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .square()
                .disabled(disabled)
                .on_click(handler),
        );
    }
    group
}

/// Draft-backed segmented group for the audio transaction: clicks land in the
/// draft and stay there until Save/Restart Audio.
pub(crate) fn choices<T: Copy + PartialEq + 'static>(
    shell: &StudioShell,
    editor: Editor,
    id: &str,
    selected: T,
    options: impl IntoIterator<Item = (T, String)>,
    set: impl Fn(&mut Draft, T) + 'static,
    cx: &Context<StudioShell>,
) -> Div {
    let set = std::rc::Rc::new(set);
    let items = options
        .into_iter()
        .map(|(value, title)| {
            let this = cx.entity();
            let set = set.clone();
            (
                title,
                value == selected,
                Box::new(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    this.update(cx, |shell, cx| {
                        let draft = shell.settings.editor.draft_mut(editor);
                        if draft.pending.is_none() {
                            set(draft, value);
                            draft.changed();
                        }
                        cx.notify();
                    });
                }) as ClickHandler,
            )
        })
        .collect::<Vec<_>>();
    segmented_group(shell, id, shell.settings.editor.busy(), items)
}

/// Preference-backed segmented group: clicks apply immediately (and persist
/// through the quiet preference chain), so there is nothing to save.
pub(crate) fn pref_choices<T: Copy + PartialEq + 'static>(
    shell: &StudioShell,
    id: &str,
    selected: T,
    options: impl IntoIterator<Item = (T, String)>,
    set: impl Fn(&mut StudioShell, T, &mut Window, &mut Context<StudioShell>) + 'static,
    cx: &Context<StudioShell>,
) -> Div {
    let set = std::rc::Rc::new(set);
    let items = options
        .into_iter()
        .map(|(value, title)| {
            let this = cx.entity();
            let set = set.clone();
            (
                title,
                value == selected,
                Box::new(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    this.update(cx, |shell, cx| set(shell, value, window, cx));
                }) as ClickHandler,
            )
        })
        .collect::<Vec<_>>();
    segmented_group(shell, id, shell.settings.editor.busy(), items)
}

pub(crate) fn field(title: &str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_1_5))
        .child(label(title.to_owned()))
        .child(value)
}

/// A [`field`] whose label carries the one note a value can't say for
/// itself. The note is a tooltip: it answers a question the user asks by
/// hovering, instead of a paragraph they must read past.
pub(crate) fn field_with_note(
    title: &str,
    note: impl Into<SharedString>,
    value: impl IntoElement,
) -> Div {
    let note: SharedString = note.into();
    div()
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_1_5))
        .child(
            div()
                .id(SharedString::from(format!("field-note-{title}")))
                .self_start()
                .tooltip(move |window, cx| {
                    component::tooltip::Tooltip::new(note.clone()).build(window, cx)
                })
                .child(label(title.to_owned())),
        )
        .child(value)
}

/// The settings row's vertical inset, deliberately off the spacing grid:
/// measured against the 1 px hairline dividers between rows ([`rows`]) so
/// the row's label and control sit at the page's reading rhythm.
const ROW_PAD_Y: f32 = 7.;

/// A settings row: name on the left, the control that changes it on the
/// right. Dividers between rows come from [`rows`].
pub(crate) fn row(title: &str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACE_3))
        .py(px(ROW_PAD_Y))
        .child(label(title.to_owned()))
        .child(control)
}

/// A [`row`] whose name carries a hover note (see [`field_with_note`]).
pub(crate) fn row_with_note(
    title: &str,
    note: impl Into<SharedString>,
    control: impl IntoElement,
) -> Div {
    let note: SharedString = note.into();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(theme::SPACE_3))
        .py(px(ROW_PAD_Y))
        .child(
            div()
                .id(SharedString::from(format!("field-note-{title}")))
                .self_start()
                .tooltip(move |window, cx| {
                    component::tooltip::Tooltip::new(note.clone()).build(window, cx)
                })
                .child(label(title.to_owned())),
        )
        .child(control)
}

/// Lays out [`row`]s with hairline dividers between them (never on the
/// group's outer edges).
pub(crate) fn rows(items: impl IntoIterator<Item = Div>) -> Div {
    let mut group = div().flex().flex_col();
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            group = group.child(div().h(px(1.)).bg(theme::HAIRLINE));
        }
        group = group.child(item);
    }
    group
}

pub(crate) fn rate_title(rate: u32) -> String {
    format!("{} kHz", rate as f32 / 1000.)
}

pub(crate) fn buffer_title(frames: u32, rate: u32) -> String {
    format!("{frames} / {:.2} ms", frames as f64 * 1000. / rate as f64)
}

/// The dropdown affordance: a control-styled trigger showing the current
/// value; clicking opens the value menu at the click position (see
/// [`ValueMenu`]). The trigger itself shows no open state — the menu
/// covering the page is the open state.
pub(crate) fn select_trigger(
    shell: &StudioShell,
    editor: Editor,
    id: &str,
    target: StreamTarget,
    field: ValueField,
    title: String,
    cx: &Context<StudioShell>,
) -> ActionButton {
    let this = cx.entity();
    button(SharedString::from(format!("{id}-trigger")), title)
        .density(shell.chrome.density)
        .variant(ButtonVariant::Secondary)
        .trailing_icon(ActionIcon::ChevronDown)
        .disabled(shell.settings.editor.busy())
        .on_click(move |event, _, cx| {
            this.update(cx, |shell, cx| {
                shell.settings.menu = Some(ValueMenu {
                    editor,
                    target: target.clone(),
                    field,
                    at: event.position(),
                });
                cx.notify();
            });
        })
}

fn rate_value(rate: Option<u32>) -> String {
    rate.map(rate_title)
        .unwrap_or_else(|| t!("Follow processing").into_owned())
}

fn buffer_value(buffer: Option<u32>, effective: u32) -> String {
    buffer
        .map(|frames| buffer_title(frames, effective))
        .unwrap_or_else(|| t!("Driver default").into_owned())
}

pub(crate) fn stream_editor(
    shell: &StudioShell,
    editor: Editor,
    direction: Direction,
    device: Option<&str>,
    cx: &Context<StudioShell>,
) -> Div {
    let draft = shell.settings.editor.draft(editor);
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
    let target = StreamTarget {
        direction,
        device: device.map(str::to_owned),
    };
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(if shell.chrome.density == Density::Compact {
            10.
        } else {
            14.
        }));
    if audio.backend == AudioBackend::Wasapi {
        view = view.child(field_with_note(
            &t!("WASAPI mode"),
            t!("Exclusive requests sole access to the device and needs its support.").into_owned(),
            choices(
                shell,
                editor,
                &format!("{prefix}-mode"),
                stream.mode,
                [
                    (DeviceMode::Shared, t!("Shared").into()),
                    (DeviceMode::Exclusive, t!("Exclusive").into()),
                ],
                {
                    let target = target.clone();
                    move |draft, value| target.stream_mut(draft).mode = value
                },
                cx,
            ),
        ));
    }
    let rate_target = target.clone();
    view = view.child(field(
        &t!("Requested sample rate"),
        select_trigger(
            shell,
            editor,
            &format!("{prefix}-rate"),
            rate_target,
            ValueField::StreamRate,
            rate_value(stream.sample_rate),
            cx,
        ),
    ));
    if audio.backend == AudioBackend::Wasapi {
        let format_target = target.clone();
        view = view.child(field(
            &t!("Client sample format"),
            select_trigger(
                shell,
                editor,
                &format!("{prefix}-format"),
                format_target,
                ValueField::StreamFormat,
                t!(stream.format.title()).into_owned(),
                cx,
            ),
        ));
    } else {
        view = view.child(detail(
            &t!("Sample encoding"),
            t!("Determined by ASIO driver"),
        ));
    }
    let buffer_target = target.clone();
    view.child(field(
        &t!("Requested device buffer"),
        select_trigger(
            shell,
            editor,
            &format!("{prefix}-buffer"),
            buffer_target,
            ValueField::StreamBuffer,
            buffer_value(
                stream.buffer_frames,
                stream.sample_rate.unwrap_or(audio.processing.sample_rate),
            ),
            cx,
        ),
    ))
}

pub(crate) fn footer(shell: &StudioShell, editor: Editor, cx: &Context<StudioShell>) -> Div {
    let draft = shell.settings.editor.draft(editor);
    let state = shell.state.read(cx);
    let running = state.snapshot.phase == SessionPhase::Running;
    let needs_restart = state.snapshot.active_audio_settings.as_ref() != Some(&draft.audio)
        || state.snapshot.active_latency_profile
            != super::model::expected_active_profile(draft.profile);
    let busy = shell.settings.editor.busy()
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
        .gap(px(theme::SPACE_2))
        .px(px(theme::SPACE_4))
        .py(px(theme::SPACE_3))
        .border_t_1()
        .border_color(theme::BORDER)
        // No fill of its own: the bar sits on whatever surface carries it
        // (canvas on the settings page, the inspector panel for device
        // overrides) instead of drawing a second band across the bottom.
        // Its contents share the reading measure of the cards above.
        .child(
            div()
                .w_full()
                .max_w(px(680.))
                .mx_auto()
                .flex()
                .flex_col()
                .gap(px(theme::SPACE_2))
                .child(
                    div()
                        .id(SharedString::from(format!("{prefix}-feedback")))
                        .text_size(px(theme::TEXT_SM))
                        .text_color(if draft.error.is_some() {
                            theme::DANGER
                        } else {
                            theme::TEXT_MUTED
                        })
                        .child(draft.error.clone().or_else(|| draft.message.clone()).unwrap_or_else(|| {
                            if draft.dirty() {
                                t!("Unsaved changes").into()
                            } else if running {
                                t!("Saved requests and active audio are shown separately in Diagnostics.")
                                    .into()
                            } else {
                                t!("Audio is stopped. Save keeps it stopped.").into()
                            }
                        }))
                        .gpui_test_observe(),
                )
                .child(
                    div().flex().flex_wrap().gap(px(theme::SPACE_2)).justify_end()
                        .child(
                            button(SharedString::from(format!("{prefix}-discard")), t!("Discard changes"))
                                .disabled(busy || !draft.dirty())
                                .on_click(move |_, _, cx| {
                                    this.update(cx, |shell, cx| {
                                        *shell.settings.editor.draft_mut(editor) =
                                            shell.current_settings_draft(cx);
                                        cx.notify();
                                    })
                                }),
                        )
                        .child(
                            button(SharedString::from(format!("{prefix}-save")), t!("Save"))
                                .variant(if running {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Primary
                                })
                                .disabled(busy || !draft.dirty())
                                .on_click({
                                    let this = cx.entity();
                                    move |_, _, cx| {
                                        this.update(cx, |shell, cx| {
                                            shell.save_audio_settings(editor, false, cx)
                                        })
                                    }
                                }),
                        )
                        .when(running, |view| {
                            view.child(
                                button(
                                    SharedString::from(format!("{prefix}-restart")),
                                    t!("Restart Audio"),
                                )
                                .variant(ButtonVariant::Primary)
                                .disabled(busy || !needs_restart)
                                .on_click(move |_, _, cx| {
                                    restart_this.update(cx, |shell, cx| {
                                        shell.save_audio_settings(editor, true, cx)
                                    })
                                }),
                            )
                        }),
                ),
        )
}
