//! Exact session-local intent. Text validity never falls back to an older value.
use super::*;

#[derive(Default)]
pub(super) struct ExactInputs {
    pub enabled: bool,
    fields: Vec<(Entity<component::input::InputState>, u32)>,
}

fn parse(text: &str, index: usize) -> Result<u32, String> {
    let range = match index {
        0 => 4..=262_144,
        1 => 2..=262_143,
        _ => 0..=262_143,
    };
    text.trim()
        .parse::<u32>()
        .ok()
        .filter(|value| range.contains(value))
        .ok_or_else(|| t!("Measured input bounds must be integers in the supported frame range. Native preparation validates the complete budget.").into())
}

impl ExactInputs {
    pub(super) fn validate(&self, cx: &App) -> Result<(), String> {
        if self.enabled {
            if self.fields.len() != 3 {
                return Err(t!("Exact input fields are not ready.").into());
            }
            for (index, (input, _)) in self.fields.iter().enumerate() {
                parse(input.read(cx).value().as_str(), index)?;
            }
        }
        Ok(())
    }

    pub(in crate::ui::window) fn owns_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.fields
            .iter()
            .any(|(input, _)| input.read(cx).focus_handle(cx) == *focused)
    }
}

impl SettingsEditor {
    pub(in crate::ui::window) fn exact_owns_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.measured.exact.owns_focus(focused, cx)
    }
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_measured_exact_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let measured = &mut self.settings.measured;
        if !measured.exact.enabled {
            measured.exact.fields.clear();
            return;
        }
        let values = [
            measured.draft.capacity_frames,
            measured.draft.target_frames,
            measured.draft.jitter_frames,
        ];
        for (index, value) in values.into_iter().enumerate() {
            if measured.exact.fields.len() <= index {
                let input = cx.new(|cx| {
                    component::input::InputState::new(window, cx).default_value(value.to_string())
                });
                cx.subscribe(
                    &input,
                    move |shell, input, event: &component::input::InputEvent, cx| {
                        if shell.settings.busy()
                            || !shell.settings.measured.exact.enabled
                            || !matches!(event, component::input::InputEvent::Change)
                        {
                            return;
                        }
                        let measured = &mut shell.settings.measured;
                        if measured
                            .exact
                            .fields
                            .get(index)
                            .is_none_or(|(current, _)| current != &input)
                        {
                            return;
                        }
                        measured.changed();
                        if let Ok(value) = parse(input.read(cx).value().as_str(), index) {
                            measured.exact.fields[index].1 = value;
                            match index {
                                0 => measured.draft.capacity_frames = value,
                                1 => measured.draft.target_frames = value,
                                _ => measured.draft.jitter_frames = value,
                            }
                        }
                        cx.notify();
                    },
                )
                .detach();
                measured.exact.fields.push((input, value));
            }
            let (input, previous) = &mut measured.exact.fields[index];
            // Preserve invalid text while the last valid draft is unchanged.
            if *previous != value {
                *previous = value;
                input.update(cx, |input, cx| {
                    input.set_value(value.to_string(), window, cx)
                });
            }
        }
    }
}

pub(super) fn toggle(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let enabled = shell.settings.measured.exact.enabled;
    let this = cx.entity();
    button(
        if enabled {
            "measured-exact-close"
        } else {
            "measured-exact-open"
        },
        if enabled {
            t!("Use input presets")
        } else {
            t!("Enter exact input bounds")
        },
    )
    .disabled(shell.settings.busy())
    .on_click(move |_, _, cx| {
        this.update(cx, |shell, cx| {
            if !shell.settings.busy() {
                shell.settings.measured.exact = ExactInputs {
                    enabled: !enabled,
                    ..Default::default()
                };
                shell.settings.measured.changed();
                cx.notify();
            }
        });
    })
}

pub(super) fn editor(shell: &StudioShell) -> Div {
    let mut view = div().flex().flex_col().gap(px(8.));
    for (index, (id, title)) in [
        ("measured-exact-capacity", "Ring capacity"),
        ("measured-exact-target", "Normal input target"),
        ("measured-exact-jitter", "Jitter reserve"),
    ]
    .into_iter()
    .enumerate()
    {
        if let Some((input, _)) = shell.settings.measured.exact.fields.get(index) {
            view = view.child(field(
                &t!(title),
                component::input::Input::new(input)
                    .id(id)
                    .disabled(shell.settings.busy())
                    .bg(theme::SURFACE_3)
                    .text_color(theme::TEXT)
                    .border_color(theme::BORDER),
            ));
        }
    }
    view.child(label(t!("Exact input bounds use 48 kHz frames. Invalid text blocks Apply; no rounding or fallback to the preceding value.")))
}
