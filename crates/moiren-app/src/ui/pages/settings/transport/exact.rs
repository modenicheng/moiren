//! Text intent is read atomically at Save, never replaced with a last-valid fallback.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(in crate::ui::pages::settings) struct ExpertInputs {
    enabled: BTreeSet<Input>,
    fields: BTreeMap<(Input, Frames), Entry>,
}

struct Entry {
    input: Entity<component::input::InputState>,
    value: u32,
}

fn parse(text: &str, frames: Frames) -> Result<u32, String> {
    let range = match frames {
        Frames::Capacity => 4..=262_144,
        Frames::Target => 2..=262_143,
        Frames::Jitter => 0..=262_143,
        Frames::Packet | Frames::Gap => 1..=262_143,
    };
    text.trim().parse::<u32>().ok().filter(|value| range.contains(value)).ok_or_else(||
        t!("Expert input bounds must be integers in the supported frame range. Native preparation validates the complete budget.").into())
}

impl ExpertInputs {
    pub(in crate::ui::pages::settings) fn modes(&self) -> [bool; 2] {
        [self.enabled(Input::Process), self.enabled(Input::Vad)]
    }

    pub(in crate::ui::pages::settings) fn inputs(
        &self,
    ) -> impl Iterator<Item = &Entity<component::input::InputState>> {
        self.fields.values().map(|entry| &entry.input)
    }

    pub(super) fn enabled(&self, input: Input) -> bool {
        self.enabled.contains(&input)
    }

    fn reset(&mut self, input: Input) {
        self.enabled.remove(&input);
        self.fields.retain(|key, _| key.0 != input);
    }

    fn current(
        &self,
        mut transport: crate::service::TransportLatencySettings,
        cx: &App,
    ) -> Result<crate::service::TransportLatencySettings, String> {
        for input in [Input::Process, Input::Vad] {
            let target = match input {
                Input::Process => &mut transport.process,
                Input::Vad => &mut transport.vad,
            };
            if !self.enabled(input) {
                continue;
            }
            let Some(target) = target else { continue };
            for frames in Frames::ALL {
                let entry = self
                    .fields
                    .get(&(input, frames))
                    .ok_or_else(|| t!("Exact input fields are not ready.").to_string())?;
                *frames.value(target) = parse(entry.input.read(cx).value().as_str(), frames)?;
            }
        }
        Ok(transport)
    }
}

impl SettingsEditor {
    pub(crate) fn reset_expert_inputs(&mut self) {
        self.expert_inputs = ExpertInputs::default();
    }

    pub(crate) fn expert_owns_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.expert_inputs
            .fields
            .values()
            .any(|entry| entry.input.read(cx).focus_handle(cx) == *focused)
    }

    pub(in crate::ui::pages::settings) fn capture_expert_inputs(
        &mut self,
        cx: &App,
    ) -> Result<(), String> {
        // Parse the complete current edit before mutating the submission draft.
        self.global.transport = self.expert_inputs.current(self.global.transport, cx)?;
        Ok(())
    }
}

impl StudioShell {
    pub(crate) fn sync_expert_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [Input::Process, Input::Vad] {
            let Some(mut selected) = input.get(&self.settings.editor.global) else {
                self.settings.editor.expert_inputs.reset(input);
                continue;
            };
            if !self.settings.editor.expert_inputs.enabled(input) || !self.settings_open {
                continue;
            }
            for frames in Frames::ALL {
                let key = (input, frames);
                let value = *frames.value(&mut selected);
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.settings.editor.expert_inputs.fields.entry(key)
                {
                    let entity = cx.new(|cx| {
                        component::input::InputState::new(window, cx)
                            .default_value(value.to_string())
                    });
                    cx.subscribe(
                        &entity,
                        move |shell, entity, event: &component::input::InputEvent, cx| {
                            if shell.settings.editor.busy()
                                || !matches!(event, component::input::InputEvent::Change)
                                || !shell.settings.editor.expert_inputs.enabled(input)
                                || shell
                                    .settings
                                    .editor
                                    .expert_inputs
                                    .fields
                                    .get(&key)
                                    .is_none_or(|entry| entry.input != entity)
                                || input.get(&shell.settings.editor.global).is_none()
                            {
                                return;
                            }
                            let draft = &mut shell.settings.editor.global;
                            draft.changed();
                            if let Ok(value) = parse(entity.read(cx).value().as_str(), frames) {
                                shell
                                    .settings
                                    .editor
                                    .expert_inputs
                                    .fields
                                    .get_mut(&key)
                                    .unwrap()
                                    .value = value;
                                *frames.value(
                                    input
                                        .edit(&mut shell.settings.editor.global)
                                        .as_mut()
                                        .unwrap(),
                                ) = value;
                            }
                            shell.settings.editor.global.error = shell
                                .settings
                                .editor
                                .expert_inputs
                                .current(shell.settings.editor.global.transport, cx)
                                .and_then(|transport| transport.validate())
                                .err();
                            cx.notify();
                        },
                    )
                    .detach();
                    entry.insert(Entry {
                        input: entity,
                        value,
                    });
                }
                let entry = self
                    .settings
                    .editor
                    .expert_inputs
                    .fields
                    .get_mut(&key)
                    .unwrap();
                // Do not normalize invalid text while its last valid value is unchanged.
                if entry.value != value {
                    entry.value = value;
                    entry.input.update(cx, |input, cx| {
                        input.set_value(value.to_string(), window, cx)
                    });
                }
            }
        }
    }
}

pub(super) fn policy(shell: &StudioShell, input: Input, cx: &Context<StudioShell>) -> Div {
    let mut row = div().flex().flex_wrap().gap(px(4.));
    for (index, (enabled, title)) in [(false, t!("Compatible")), (true, t!("Expert estimate"))]
        .into_iter()
        .enumerate()
    {
        let this = cx.entity();
        row = row.child(
            button(
                SharedString::from(format!("{}-policy-{index}", input.prefix())),
                title,
            )
            .density(shell.density)
            .variant(
                if input.get(&shell.settings.editor.global).is_some() == enabled {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                },
            )
            .disabled(shell.settings.editor.busy())
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    if shell.settings.editor.busy() {
                        return;
                    }
                    if input.get(&shell.settings.editor.global).is_some() != enabled {
                        shell.settings.editor.expert_inputs.reset(input);
                        let value = input.edit(&mut shell.settings.editor.global);
                        *value = enabled.then(|| value.unwrap_or_default());
                        shell.settings.editor.global.changed();
                    }
                    cx.notify();
                })
            }),
        );
    }
    row
}

pub(super) fn toggle(
    shell: &StudioShell,
    input: Input,
    cx: &Context<StudioShell>,
) -> impl IntoElement {
    let enabled = shell.settings.editor.expert_inputs.enabled(input);
    let this = cx.entity();
    button(
        SharedString::from(format!(
            "{}-exact-{}",
            input.prefix(),
            if enabled { "close" } else { "open" }
        )),
        if enabled {
            t!("Use input presets")
        } else {
            t!("Enter exact input bounds")
        },
    )
    .disabled(shell.settings.editor.busy())
    .on_click(move |_, window, cx| {
        this.update(cx, |shell, cx| {
            if shell.settings.editor.busy() || input.get(&shell.settings.editor.global).is_none() {
                return;
            }
            shell.settings.editor.expert_inputs.reset(input);
            if !enabled {
                shell.settings.editor.expert_inputs.enabled.insert(input);
            }
            shell.settings.editor.global.changed();
            shell.sync_expert_inputs(window, cx);
            cx.notify();
        })
    })
}

pub(super) fn editor(shell: &StudioShell, input: Input) -> Div {
    let mut view = div().flex().flex_col().gap(px(8.));
    for frames in Frames::ALL {
        if let Some(entry) = shell
            .settings
            .editor
            .expert_inputs
            .fields
            .get(&(input, frames))
        {
            view = view.child(field(
                &t!(frames.title()),
                component::input::Input::new(&entry.input)
                    .id(SharedString::from(format!(
                        "{}-exact-{}",
                        input.prefix(),
                        frames.key()
                    )))
                    .disabled(shell.settings.editor.busy())
                    .bg(theme::SURFACE_3)
                    .text_color(theme::TEXT)
                    .border_color(theme::BORDER),
            ));
        }
    }
    view.child(label(t!("Exact expert bounds use 48 kHz frames. Invalid text blocks Save and Restart Audio; no rounding or fallback to the preceding value.")))
}

#[cfg(all(test, windows, feature = "gpui-test"))]
mod tests {
    use super::{ExpertInputBuffer, Frames, Input};
    use crate::ui::shell::test_harness::{finish_service, test_shell};

    #[gpui_kit::test]
    fn transport_exact_replaced_input_cannot_dispatch_a_delayed_change(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (view, cx, runtime) = test_shell(cx);
        let old = cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.settings_open = true;
                shell.settings.editor.global.transport.vad = Some(ExpertInputBuffer::default());
                shell
                    .settings
                    .editor
                    .expert_inputs
                    .enabled
                    .insert(Input::Vad);
                shell.sync_expert_inputs(window, cx);
                shell.settings.editor.expert_inputs.fields[&(Input::Vad, Frames::Target)]
                    .input
                    .clone()
            })
        });
        cx.update(|window, cx| {
            view.update(cx, |shell, cx| {
                shell.settings.editor.expert_inputs.reset(Input::Vad);
                shell
                    .settings
                    .editor
                    .expert_inputs
                    .enabled
                    .insert(Input::Vad);
                shell.sync_expert_inputs(window, cx);
                let fresh = &shell.settings.editor.expert_inputs.fields
                    [&(Input::Vad, Frames::Target)]
                    .input;
                assert_ne!(fresh, &old);
                old.update(cx, |input, cx| {
                    input.set_value("945", window, cx);
                    cx.emit(crate::ui::shell::component::input::InputEvent::Change);
                });
            })
        });
        cx.run_until_parked();
        view.update(cx, |shell, cx| {
            assert_eq!(
                shell
                    .settings
                    .editor
                    .global
                    .transport
                    .vad
                    .unwrap()
                    .target_frames,
                2048
            );
            assert_eq!(
                shell
                    .settings
                    .editor
                    .expert_inputs
                    .current(shell.settings.editor.global.transport, cx)
                    .unwrap()
                    .vad
                    .unwrap()
                    .target_frames,
                2048
            );
        });
        finish_service(runtime);
    }
}
