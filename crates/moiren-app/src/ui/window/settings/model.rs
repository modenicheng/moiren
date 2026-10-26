use rust_i18n::t;

use super::*;
use crate::service::{AudioLatencyProfile, RequestId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum Editor {
    Global,
    Device,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum Direction {
    Input,
    Output,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(in super::super) enum Section {
    #[default]
    Audio,
    Appearance,
    Diagnostics,
}

#[derive(Clone)]
pub(in super::super) struct Draft {
    pub audio: AudioSettings,
    pub profile: AudioLatencyProfile,
    pub density: Density,
    original: (AudioSettings, AudioLatencyProfile, Density),
    pub error: Option<String>,
    pub message: Option<String>,
    pub pending: Option<Pending>,
}

#[derive(Clone)]
pub(in super::super) struct Pending {
    pub request: RequestId,
    pub stage: Stage,
    pub restart: bool,
    pub submitted: Submission,
}

#[derive(Clone)]
pub(in super::super) struct Submission {
    pub audio: AudioSettings,
    pub profile: AudioLatencyProfile,
    pub density: Density,
}

#[derive(Clone, Copy)]
pub(in super::super) enum Stage {
    Audio,
    Profile,
    Restart,
}

impl Draft {
    pub fn new(audio: AudioSettings, profile: AudioLatencyProfile, density: Density) -> Self {
        Self {
            original: (audio.clone(), profile, density),
            audio,
            profile,
            density,
            error: None,
            message: None,
            pending: None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.error.is_some()
            || self.audio != self.original.0
            || self.profile != self.original.1
            || self.density != self.original.2
    }

    pub fn changed(&mut self) {
        self.error = None;
        self.message = None;
    }

    pub fn committed(&mut self) {
        self.original = (self.audio.clone(), self.profile, self.density);
        self.pending = None;
        self.error = None;
        self.message = Some(t!("Saved. Audio settings apply on the next start.").into());
    }

    pub(super) fn submission(
        &self,
        current: &AudioSettings,
        profile: AudioLatencyProfile,
        density: Density,
    ) -> Result<Submission, String> {
        Ok(Submission {
            audio: rebase_audio(&self.original.0, &self.audio, current)?,
            profile: merge_value(
                self.original.1,
                self.profile,
                profile,
                &t!("Recovery policy"),
            )?,
            density: merge_value(self.original.2, self.density, density, &t!("Density"))?,
        })
    }
}

pub(super) fn expected_active_profile(
    audio: &AudioSettings,
    profile: AudioLatencyProfile,
) -> Option<AudioLatencyProfile> {
    (audio == &AudioSettings::default()).then_some(profile)
}

fn merge_value<T: PartialEq>(original: T, edited: T, current: T, label: &str) -> Result<T, String> {
    if edited == original {
        Ok(current)
    } else if current == original || current == edited {
        Ok(edited)
    } else {
        Err(t!(
            "%{label} changed elsewhere. Your draft is retained. Discard changes to reload the saved settings.",
            label = label
        )
        .to_string())
    }
}

/// Merge changed domains only. A device's full stream override is atomic;
/// unrelated device edits and newer defaults remain owned by the service.
fn rebase_audio(
    original: &AudioSettings,
    edited: &AudioSettings,
    current: &AudioSettings,
) -> Result<AudioSettings, String> {
    let mut merged = current.clone();
    merged.backend = merge_value(
        original.backend,
        edited.backend,
        current.backend,
        &t!("Backend"),
    )?;
    merged.processing = merge_value(
        original.processing,
        edited.processing,
        current.processing,
        &t!("Processing"),
    )?;
    merged.input = merge_value(
        original.input,
        edited.input,
        current.input,
        &t!("Input defaults"),
    )?;
    merged.output = merge_value(
        original.output,
        edited.output,
        current.output,
        &t!("Output defaults"),
    )?;
    for (original, edited, current, target, label) in [
        (
            &original.input_overrides,
            &edited.input_overrides,
            &current.input_overrides,
            &mut merged.input_overrides,
            "Input",
        ),
        (
            &original.output_overrides,
            &edited.output_overrides,
            &current.output_overrides,
            &mut merged.output_overrides,
            "Output",
        ),
    ] {
        for id in original
            .keys()
            .chain(edited.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let value = merge_value(
                original.get(id),
                edited.get(id),
                current.get(id),
                &t!("%{label} override for %{id}", label = t!(label), id = id),
            )?;
            if let Some(value) = value {
                target.insert(id.clone(), *value);
            } else {
                target.remove(id);
            }
        }
    }
    Ok(merged)
}

pub(in super::super) struct SettingsEditor {
    pub global: Draft,
    pub device: Draft,
    pub focus: FocusHandle,
    pub previous_focus: Option<FocusHandle>,
    pub section: Section,
    pub save_revision: Option<(u64, Editor)>,
}

impl SettingsEditor {
    pub fn new(preferences: &crate::project::AppPreferences, cx: &mut App) -> Self {
        let density = if preferences.density == crate::project::UiDensity::Compact {
            Density::Compact
        } else {
            Density::Comfortable
        };
        let draft = Draft::new(
            preferences.audio.clone(),
            preferences.latency_profile,
            density,
        );
        Self {
            global: draft.clone(),
            device: draft,
            focus: cx.focus_handle(),
            previous_focus: None,
            section: Section::Audio,
            save_revision: None,
        }
    }

    pub fn draft(&self, editor: Editor) -> &Draft {
        match editor {
            Editor::Global => &self.global,
            Editor::Device => &self.device,
        }
    }

    pub fn draft_mut(&mut self, editor: Editor) -> &mut Draft {
        match editor {
            Editor::Global => &mut self.global,
            Editor::Device => &mut self.device,
        }
    }

    pub fn busy(&self) -> bool {
        self.global.pending.is_some() || self.device.pending.is_some()
    }
}

/// A device edit materializes the entire inherited stream once. Further edits
/// stay independent of subsequent defaults until the override is removed.
pub(in super::super) fn device_stream_mut<'a>(
    audio: &'a mut AudioSettings,
    direction: Direction,
    id: &str,
) -> &'a mut StreamSettings {
    match direction {
        Direction::Input => audio
            .input_overrides
            .entry(id.to_owned())
            .or_insert(audio.input),
        Direction::Output => audio
            .output_overrides
            .entry(id.to_owned())
            .or_insert(audio.output),
    }
}

impl StudioShell {
    /// Language applies immediately (not through the Save flow) because every
    /// rendered string reads the process-global locale; preferences persist in
    /// the background like density/profile writes.
    pub(in super::super) fn set_language(
        &mut self,
        language: crate::i18n::UiLanguage,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.language == language {
            return;
        }
        crate::i18n::apply(language);
        self.preferences.language = language;
        self.save_preferences(cx);
        cx.notify();
    }

    pub(in super::super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.global.dirty() && self.settings.global.pending.is_none() {
            self.settings.global = self.current_settings_draft(cx);
        }
        self.settings.previous_focus = window.focused(cx);
        self.settings_open = true;
        self.project_menu = false;
        window.focus(&self.settings.focus, cx);
        cx.notify();
    }

    pub(in super::super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        if let Some(focus) = self.settings.previous_focus.take() {
            window.focus(&focus, cx);
        } else {
            window.blur(cx);
        }
        cx.notify();
    }

    pub(in super::super) fn keep_settings_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The modal is a sibling of the retained workspace. Moving focus also
        // removes graph Delete/Escape handlers from the keyboard dispatch path.
        if self.settings_open && !self.settings.focus.contains_focused(window, cx) {
            window.focus(&self.settings.focus, cx);
        }
    }

    pub(in super::super) fn current_settings_draft(&self, cx: &App) -> Draft {
        let snapshot = &self.state.read(cx).snapshot;
        Draft::new(
            snapshot.audio_settings.clone(),
            snapshot.latency_profile,
            self.density,
        )
    }

    pub(in super::super) fn inspect_device(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.settings.device.dirty() && self.settings.device.pending.is_none() {
            self.settings.device = self.current_settings_draft(cx);
        }
        self.selected_device = Some(id);
        // Configure is an explicit request for the override editor.
        self.open_inspector();
        cx.notify();
    }
}
