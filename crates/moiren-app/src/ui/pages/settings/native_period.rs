//! Control-side exact device periods; Engine block choices remain independent.
use super::*;
use std::collections::BTreeMap;

type Key = (Editor, Direction, Option<String>);

pub(super) struct PeriodInput {
    pub input: Entity<component::input::InputState>,
    value: Option<u32>,
}

pub(super) type PeriodInputs = BTreeMap<Key, PeriodInput>;

fn stream<'a>(draft: &'a Draft, key: &Key) -> Option<&'a StreamSettings> {
    match (key.1, key.2.as_deref()) {
        (Direction::Input, None) => Some(&draft.audio.input),
        (Direction::Output, None) => Some(&draft.audio.output),
        (Direction::Input, Some(id)) => draft.audio.input_overrides.get(id),
        (Direction::Output, Some(id)) => draft.audio.output_overrides.get(id),
    }
}

fn parse(text: &str) -> Result<Option<u32>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    text.trim().parse::<u32>().ok()
        .filter(|frames| moiren_engine::audio_config::SHARED_PERIOD_RANGE.contains(frames))
        .map(Some)
        .ok_or_else(|| t!("Shared device period must be an integer within 64..=4096 frames; leave blank for driver default.").into())
}

impl SettingsEditor {
    pub(crate) fn device_period_inputs(
        &self,
    ) -> impl Iterator<Item = &Entity<component::input::InputState>> {
        self.period_inputs
            .iter()
            .filter(|(key, _)| key.0 == Editor::Device)
            .map(|(_, entry)| &entry.input)
    }

    pub(crate) fn period_owns_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.period_inputs
            .values()
            .any(|entry| entry.input.read(cx).focus_handle(cx) == *focused)
    }

    pub(super) fn validate_period_inputs(&self, editor: Editor, cx: &App) -> Result<(), String> {
        let draft = self.draft(editor);
        for (key, entry) in &self.period_inputs {
            if key.0 == editor
                && draft.audio.backend == AudioBackend::Wasapi
                && stream(draft, key).is_some_and(|stream| stream.mode == DeviceMode::Shared)
            {
                parse(entry.input.read(cx).value().as_str())?;
            }
        }
        Ok(())
    }

    pub(super) fn reset_period_inputs(&mut self, editor: Editor) {
        self.period_inputs.retain(|key, _| key.0 != editor);
    }
}

impl StudioShell {
    pub(crate) fn sync_native_period_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Removed overrides and hidden backend/mode editors must not retain
        // invalid text if the user later creates a fresh override.
        self.settings.editor.period_inputs.retain(|key, _| {
            let draft = match key.0 {
                Editor::Global => &self.settings.editor.global,
                Editor::Device => &self.settings.editor.device,
            };
            draft.audio.backend == AudioBackend::Wasapi
                && stream(draft, key).is_some_and(|stream| stream.mode == DeviceMode::Shared)
        });
        let mut targets = Vec::new();
        if self.presentation.workspace.is_settings() {
            for direction in [Direction::Input, Direction::Output] {
                targets.push((Editor::Global, direction, None));
            }
        }
        if let Some(id) = &self.devices.selected_device {
            let direction = if self.devices.device_tab == devices::DeviceTab::Inputs {
                Direction::Input
            } else {
                Direction::Output
            };
            targets.push((Editor::Device, direction, Some(id.clone())));
        }
        for key in targets {
            let draft = self.settings.editor.draft(key.0);
            let Some(value) = stream(draft, &key)
                .filter(|stream| {
                    draft.audio.backend == AudioBackend::Wasapi && stream.mode == DeviceMode::Shared
                })
                .map(|stream| stream.buffer_frames)
            else {
                continue;
            };
            let entry = self
                .settings
                .editor
                .period_inputs
                .entry(key.clone())
                .or_insert_with(|| {
                    let input = cx.new(|cx| {
                        component::input::InputState::new(window, cx)
                            .placeholder(t!("Driver default"))
                            .default_value(
                                value.map_or_else(String::new, |value| value.to_string()),
                            )
                    });
                    let key = key.clone();
                    cx.subscribe(
                        &input,
                        move |shell, input, event: &component::input::InputEvent, cx| {
                            if shell.settings.editor.busy()
                                || shell
                                    .settings
                                    .editor
                                    .period_inputs
                                    .get(&key)
                                    .is_none_or(|entry| entry.input != input)
                                || !matches!(
                                    event,
                                    component::input::InputEvent::Change
                                        | component::input::InputEvent::PressEnter { .. }
                                        | component::input::InputEvent::Blur
                                )
                            {
                                return;
                            }
                            let draft = shell.settings.editor.draft(key.0);
                            if draft.audio.backend != AudioBackend::Wasapi
                                || !stream(draft, &key)
                                    .is_some_and(|stream| stream.mode == DeviceMode::Shared)
                            {
                                return;
                            }
                            let text = input.read(cx).value();
                            match parse(text.as_str()) {
                                Ok(value) => {
                                    if let Some(entry) =
                                        shell.settings.editor.period_inputs.get_mut(&key)
                                    {
                                        entry.value = value;
                                    }
                                    let draft = shell.settings.editor.draft_mut(key.0);
                                    let target = StreamTarget {
                                        direction: key.1,
                                        device: key.2.clone(),
                                    };
                                    target.stream_mut(draft).buffer_frames = value;
                                    draft.changed();
                                }
                                Err(error) => {
                                    let draft = shell.settings.editor.draft_mut(key.0);
                                    // The cached inspector keys on the draft revision rather
                                    // than comparing its full value every notification.
                                    // `changed` would clear this diagnostic, so report it
                                    // while making the feedback itself a repaint input.
                                    draft.revision = draft.revision.wrapping_add(1);
                                    draft.feedback =
                                        Some(DraftFeedback::Diagnostic { cause: error });
                                }
                            }
                            cx.notify();
                        },
                    )
                    .detach();
                    PeriodInput { input, value }
                });
            if entry.value != value {
                entry.value = value;
                entry.input.update(cx, |input, cx| {
                    input.set_value(
                        value.map_or_else(String::new, |value| value.to_string()),
                        window,
                        cx,
                    );
                });
            }
        }
    }
}

pub(super) fn editor(
    shell: &StudioShell,
    editor: Editor,
    direction: Direction,
    device: Option<&str>,
    prefix: &str,
) -> Div {
    let key = (editor, direction, device.map(str::to_owned));
    let mut row = div().flex().flex_col().gap(px(6.));
    if let Some(entry) = shell.settings.editor.period_inputs.get(&key) {
        row = row.child(
            component::input::Input::new(&entry.input)
                .id(SharedString::from(format!("{prefix}-period-input")))
                .disabled(shell.settings.editor.busy())
                .bg(theme::SURFACE)
                .text_color(theme::TEXT)
                .border_color(theme::BORDER),
        );
    }
    row.child(label(t!("Exact Shared device period in frames (64..=4096). Blank uses driver default. Native preparation checks support; this is independent of the Engine block.")))
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests {
    #[gpui_kit::test]
    fn shared_native_period_replaced_input_cannot_mutate_the_new_draft(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::*;
        use crate::ui::shell::test_harness::{finish_service, test_shell};
        let (view, cx, runtime) = test_shell(cx);
        let key = (Editor::Global, Direction::Output, None);
        let old = cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.open_settings(window, cx);
                shell.settings.editor.global.audio = AudioSettings::default();
                shell.sync_native_period_inputs(window, cx);
                shell.settings.editor.period_inputs[&key].input.clone()
            })
        });
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.settings.editor.reset_period_inputs(Editor::Global);
                shell.sync_native_period_inputs(window, cx);
                assert_ne!(&shell.settings.editor.period_inputs[&key].input, &old);
                old.update(cx, |input, cx| {
                    input.set_value("336", window, cx);
                    cx.emit(crate::ui::shell::component::input::InputEvent::Change);
                });
            });
        });
        cx.run_until_parked();
        view.read_with(cx, |shell, cx| {
            assert_eq!(
                shell.settings.editor.global.audio.output.buffer_frames,
                None
            );
            assert_eq!(
                shell.settings.editor.period_inputs[&key]
                    .input
                    .read(cx)
                    .value(),
                ""
            );
        });
        finish_service(runtime);
    }
}
