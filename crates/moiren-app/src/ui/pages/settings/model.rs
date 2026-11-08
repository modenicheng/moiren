use rust_i18n::t;

use super::*;
use crate::service::{
    AppSnapshot, AudioLatencyProfile, MeasuredProcessSummary, RequestId, TransportLatencySettings,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Editor {
    Global,
    Device,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Direction {
    Input,
    Output,
}

/// Which stream a set of controls edits: the global defaults, or one
/// device's override. Carried by `stream_editor`, the dropdown triggers and
/// the open value menu so a menu keeps editing the stream it was opened for.
#[derive(Clone)]
pub(crate) struct StreamTarget {
    pub direction: Direction,
    pub device: Option<String>,
}

impl StreamTarget {
    /// The stream as it would run: the device override when one exists, the
    /// defaults otherwise. Owned because `effective_*` copies overrides out.
    pub(crate) fn stream(&self, audio: &AudioSettings) -> StreamSettings {
        match (&self.device, self.direction) {
            (Some(id), Direction::Input) => audio.effective_input(id),
            (Some(id), Direction::Output) => audio.effective_output(id),
            (None, Direction::Input) => audio.input,
            (None, Direction::Output) => audio.output,
        }
    }

    pub(crate) fn stream_mut<'a>(&self, draft: &'a mut Draft) -> &'a mut StreamSettings {
        match &self.device {
            Some(id) => device_stream_mut(&mut draft.audio, self.direction, id),
            None => match self.direction {
                Direction::Input => &mut draft.audio.input,
                Direction::Output => &mut draft.audio.output,
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Section {
    #[default]
    Audio,
    Transport,
    Interface,
    Diagnostics,
}

#[derive(Clone)]
pub(crate) struct Draft {
    pub audio: AudioSettings,
    pub profile: AudioLatencyProfile,
    pub transport: TransportLatencySettings,
    original: (AudioSettings, AudioLatencyProfile, TransportLatencySettings),
    pub error: Option<String>,
    pub message: Option<String>,
    pub pending: Option<Pending>,
}

impl PartialEq for Draft {
    fn eq(&self, other: &Self) -> bool {
        self.audio == other.audio
            && self.profile == other.profile
            && self.transport == other.transport
            && self.original == other.original
            && self.error == other.error
            && self.message == other.message
            && self.pending.is_some() == other.pending.is_some()
    }
}

/// Local presentation only: deadlines and receipt bookkeeping do not repaint.
#[derive(PartialEq)]
pub(super) struct MeasuredKey {
    pub node: Option<moiren_core::graph::NodeId>,
    pub bounds: [u32; 3],
    pub exact: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub message: Option<String>,
    #[cfg(windows)]
    pub retained_open: bool,
    #[cfg(windows)]
    pub retained: RetainedKey,
    #[cfg(windows)]
    pub live_open: bool,
    #[cfg(windows)]
    pub live_policy: [Option<u64>; 5],
}

#[cfg(windows)]
#[derive(PartialEq)]
pub(super) struct RetainedKey {
    pub policy: moiren_windows_audio::process_loopback::ReplayWindowPolicy,
    pub error: Option<String>,
    pub busy: bool,
    pub result_revision: u64,
}

#[derive(Clone)]
pub(crate) struct Pending {
    // NOTE: deliberately not `PartialEq`; the draft comparison below treats
    // any pending save as equal to any other so the panel key only rebuilds
    // on value changes (the pending stage renders through the shell's save
    // notifications via the `message`/`error` fields instead).
    pub request: RequestId,
    pub stage: Stage,
    pub restart: bool,
    pub submitted: Submission,
}

#[derive(Clone)]
pub(crate) struct Submission {
    pub audio: AudioSettings,
    pub profile: AudioLatencyProfile,
    pub transport: TransportLatencySettings,
    pub measured_process: Option<MeasuredProcessSummary>,
    pub save_transport: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Audio,
    Transport,
    Profile,
    Restart,
}

impl Draft {
    pub fn new(
        audio: AudioSettings,
        profile: AudioLatencyProfile,
        transport: TransportLatencySettings,
    ) -> Self {
        Self {
            original: (audio.clone(), profile, transport),
            audio,
            profile,
            transport,
            error: None,
            message: None,
            pending: None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.error.is_some()
            || self.audio != self.original.0
            || self.profile != self.original.1
            || self.transport != self.original.2
    }

    pub fn changed(&mut self) {
        self.error = None;
        self.message = None;
    }

    pub fn committed(&mut self) {
        self.original = (self.audio.clone(), self.profile, self.transport);
        self.pending = None;
        self.error = None;
        self.message = Some(t!("Saved. Audio settings apply on the next start.").into());
    }

    pub(super) fn submission(
        &self,
        current: &AudioSettings,
        profile: AudioLatencyProfile,
        transport: TransportLatencySettings,
    ) -> Result<Submission, String> {
        let merged_transport = TransportLatencySettings {
            render: merge_value(
                self.original.2.render,
                self.transport.render,
                transport.render,
                &t!("Render queue"),
            )?,
            process_max_ingress_elapsed_100ns: merge_value(
                self.original.2.process_max_ingress_elapsed_100ns,
                self.transport.process_max_ingress_elapsed_100ns,
                transport.process_max_ingress_elapsed_100ns,
                &t!("Process host ingress elapsed limit"),
            )?,
            process: merge_value(
                self.original.2.process,
                self.transport.process,
                transport.process,
                &t!("Process input buffer"),
            )?,
            vad: merge_value(
                self.original.2.vad,
                self.transport.vad,
                transport.vad,
                &t!("VAD input buffer"),
            )?,
            vad_recovery_cap_periods: merge_value(
                self.original.2.vad_recovery_cap_periods,
                self.transport.vad_recovery_cap_periods,
                transport.vad_recovery_cap_periods,
                &t!("VAD recovery cap"),
            )?,
        };
        Ok(Submission {
            audio: rebase_audio(&self.original.0, &self.audio, current)?,
            profile: merge_value(
                self.original.1,
                self.profile,
                profile,
                &t!("Device buffer policy"),
            )?,
            transport: merged_transport,
            measured_process: None,
            save_transport: merged_transport != transport,
        })
    }

    pub(super) fn capture(&self, snapshot: &AppSnapshot) -> Result<Submission, String> {
        let mut submitted = self.submission(
            &snapshot.audio_settings,
            snapshot.latency_profile,
            snapshot.transport_latency,
        )?;
        // SetTransportLatency intentionally clears measured intent, including
        // VAD-only and render-only changes. A changed consumer envelope needs
        // fresh admission; never reuse a finite-window proof for another one.
        // Otherwise preserve the latest service selection, not the old draft.
        if !submitted.save_transport {
            submitted.measured_process = snapshot.measured_process.clone();
        }
        Ok(submitted)
    }
}

impl Submission {
    pub(super) fn matches_requested(&self, snapshot: &AppSnapshot) -> bool {
        snapshot.audio_settings == self.audio
            && snapshot.transport_latency == self.transport
            && snapshot.latency_profile == self.profile
            && snapshot.measured_process == self.measured_process
    }

    pub(super) fn matches_active(&self, snapshot: &AppSnapshot) -> bool {
        snapshot.phase == SessionPhase::Running
            && snapshot.active_audio_settings.as_ref() == Some(&self.audio)
            && snapshot.active_transport_latency == Some(self.transport)
            && snapshot.active_latency_profile == expected_active_profile(self.profile)
            && snapshot.active_measured_process == self.measured_process
    }
}

pub(super) fn expected_active_profile(profile: AudioLatencyProfile) -> Option<AudioLatencyProfile> {
    Some(profile)
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

pub(crate) struct SettingsEditor {
    pub(super) period_inputs: super::native_period::PeriodInputs,
    pub(super) expert_inputs: super::transport::exact::ExpertInputs,
    pub(super) measured: super::measured::MeasuredEditor,
    pub global: Draft,
    pub device: Draft,
    pub focus: FocusHandle,
    pub previous_focus: Option<FocusHandle>,
    pub section: Section,
    pub save_revision: Option<(u64, Editor)>,
}

impl SettingsEditor {
    pub fn new(preferences: &crate::project::AppPreferences, cx: &mut App) -> Self {
        let draft = Draft::new(
            preferences.audio.clone(),
            preferences.latency_profile,
            preferences.transport_latency,
        );
        Self {
            period_inputs: Default::default(),
            expert_inputs: Default::default(),
            measured: Default::default(),
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
        self.global.pending.is_some() || self.device.pending.is_some() || self.measured.busy()
    }
}

/// A device edit materializes the entire inherited stream once. Further edits
/// stay independent of subsequent defaults until the override is removed.
pub(crate) fn device_stream_mut<'a>(
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
    pub(crate) fn set_language(
        &mut self,
        language: crate::i18n::UiLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.values.language == language {
            return;
        }
        crate::i18n::apply(language);
        self.preferences.values.language = language;
        self.save_preferences(cx);
        // Strings stored in component state never re-read the locale on
        // re-render; the search placeholder must follow the switch like the
        // render-time t! calls do.
        self.node_search.update(cx, |search, cx| {
            search.set_placeholder(t!("Search nodes"), window, cx);
        });
        // The untitled name is presentational, so it follows the locale too;
        // a loaded document's name is user data and stays as authored.
        if self.project_io.path.is_none()
            && !self.project_io.busy
            && self.project_io.pending.is_none()
        {
            self.project_io.name = t!("Untitled session").into();
        }
        cx.notify();
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.editor.global.dirty() && self.settings.editor.global.pending.is_none() {
            self.settings.editor.reset_period_inputs(Editor::Global);
            self.settings.editor.reset_expert_inputs();
            self.settings.editor.global = self.current_settings_draft(cx);
        }
        self.settings.editor.previous_focus = window.focused(cx);
        self.settings_open = true;
        self.project_menu = false;
        window.focus(&self.settings.editor.focus, cx);
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.settings.menu = None;
        if let Some(focus) = self.settings.editor.previous_focus.take() {
            window.focus(&focus, cx);
        } else {
            window.blur(cx);
        }
        cx.notify();
    }

    pub(crate) fn keep_settings_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The modal is a sibling of the retained workspace. Moving focus also
        // removes graph Delete/Escape handlers from the keyboard dispatch path.
        if self.settings_open && !self.settings.editor.focus.contains_focused(window, cx) {
            window.focus(&self.settings.editor.focus, cx);
        }
    }

    pub(crate) fn current_settings_draft(&self, cx: &App) -> Draft {
        let snapshot = &self.state.read(cx).snapshot;
        Draft::new(
            snapshot.audio_settings.clone(),
            snapshot.latency_profile,
            snapshot.transport_latency,
        )
    }

    pub(crate) fn inspect_device(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.settings.editor.device.dirty() && self.settings.editor.device.pending.is_none() {
            self.settings.editor.reset_period_inputs(Editor::Device);
            self.settings.editor.device = self.current_settings_draft(cx);
        }
        self.devices.selected_device = Some(id);
        // Configure is an explicit request for the override editor.
        self.open_inspector();
        cx.notify();
    }

    /// Immediate preferences: apply to the live shell state and persist
    /// through the quiet write chain. The interface speaks for itself — a
    /// flipped control is already in effect, so there is nothing to save.
    pub(crate) fn set_density(&mut self, density: Density, cx: &mut Context<Self>) {
        if self.density == density {
            return;
        }
        self.density = density;
        self.preferences.values.density = if density == Density::Compact {
            crate::project::UiDensity::Compact
        } else {
            crate::project::UiDensity::Comfortable
        };
        self.save_preferences_quiet(cx);
        cx.notify();
    }

    pub(crate) fn set_meter_falloff(
        &mut self,
        falloff: crate::project::MeterFalloff,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.values.meters.falloff == falloff {
            return;
        }
        self.preferences.values.meters.falloff = falloff;
        self.save_preferences_quiet(cx);
        cx.notify();
    }

    pub(crate) fn set_peak_hold(
        &mut self,
        peak_hold: crate::project::PeakHold,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.values.meters.peak_hold == peak_hold {
            return;
        }
        self.preferences.values.meters.peak_hold = peak_hold;
        self.save_preferences_quiet(cx);
        cx.notify();
    }

    pub(crate) fn set_restore_last_project(&mut self, restore: bool, cx: &mut Context<Self>) {
        if self.preferences.values.restore_last_project == restore {
            return;
        }
        self.preferences.values.restore_last_project = restore;
        self.save_preferences_quiet(cx);
        cx.notify();
    }
}
