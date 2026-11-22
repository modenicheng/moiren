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
    /// The draft's status line payload (design 21 §4): stored typed and
    /// locale-free, translated at paint by [`feedback_text`]. One field
    /// serves both the failure prose and the progress notes the old
    /// `error`/`message` string pair froze — the two never coexisted, and
    /// error-ness is a property of the variant ([`DraftFeedback::is_error`]).
    pub feedback: Option<DraftFeedback>,
    pub pending: Option<Pending>,
    /// Edit revision for the cached-panel keys (issues #117/#118): the
    /// Devices and Inspector panels render draft-derived content, but
    /// keying the draft by value would clone (and compare) a whole
    /// `AudioSettings` — the override maps make `PartialEq` a full-value
    /// walk — on every owner notification. Every draft-mutating method
    /// bumps this counter (`changed`, `committed`, and the whole-draft
    /// replacement in `inspect_device`, which carries the successor); the
    /// keys compare the u64. Smallest correct option: no Arc wraps the
    /// draft, so `PublishRef` identity was not available. Deliberately
    /// excluded from [`PartialEq`] — content equality still drives
    /// `SettingsKey`, and a bump without a content change is one harmless
    /// extra rebuild, never staleness.
    pub revision: u64,
}

impl PartialEq for Draft {
    fn eq(&self, other: &Self) -> bool {
        self.audio == other.audio
            && self.profile == other.profile
            && self.original == other.original
            && self.feedback == other.feedback
            && self.pending.is_some() == other.pending.is_some()
            && self.transport == other.transport
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
    // notifications via the `feedback` field instead).
    pub request: RequestId,
    pub stage: Stage,
    pub restart: bool,
    pub(super) submitted: Submission,
}

#[derive(Clone)]
pub(super) struct Submission {
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

/// The settings draft's status payload (design 21 §4): [`Draft`] stores what
/// happened — typed and locale-free — and the paint sites translate it
/// through [`feedback_text`] with the current locale, so a language switch
/// relabels a stored status on the next repaint instead of freezing it.
/// Closed sets (`UiFailure`, [`AudioConfigError`]) are stored whole; the
/// device ids in [`Domain`] stay raw data; free-form service prose rides as
/// diagnostic data inside typed variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DraftFeedback {
    /// The settings were committed; they take effect at the next start.
    Saved,
    /// The save request is in flight.
    SavingRequested,
    /// The latency-policy request is in flight.
    SavingLatencyPolicy,
    /// Audio is restarting; the active session is being waited on.
    RestartingAudio,
    /// Audio restarted with the saved settings.
    Restarted,
    /// The user configuration directory is unavailable; the draft is kept.
    PreferencesDirUnavailable,
    /// The service accepted the request but kept its previous settings and
    /// reported no diagnostic.
    Rejected { what: Rejection },
    /// The service reported a typed control-plane failure while the save was
    /// pending (captured from `UiState.dispatch_error`); translated at paint
    /// through `crate::ui::presentation::failure_text`.
    Failure {
        failure: crate::ui::state::UiFailure,
    },
    /// The service reported free-form diagnostic prose (`snapshot.error` /
    /// `graph.last_error`); kept as data and rendered untranslated like the
    /// other service diagnostics.
    Diagnostic { cause: String },
    /// The settings-save dispatch was declined outright; the failure is the
    /// typed service refusal and the wrapper sentence is chosen at paint.
    SaveDispatchFailed {
        failure: crate::ui::state::UiFailure,
    },
    /// A follow-up dispatch (latency policy or restart) was declined
    /// outright; the failure is the typed service refusal and the wrapper
    /// sentence is chosen at paint.
    ApplyDispatchFailed {
        failure: crate::ui::state::UiFailure,
    },
    /// A value changed elsewhere while editing; `domain` names the
    /// conflicting field and its label composes at paint.
    DomainChanged { domain: Domain },
    /// The submitted settings failed validation; the closed
    /// [`AudioConfigError`] set is stored whole and translated at paint.
    ValidateInvalid {
        error: crate::service::AudioConfigError,
    },
    /// The background preferences write failed; `cause` is the persistence
    /// prose (diagnostic data).
    PreferencesSaveFailed { cause: String },
}

impl DraftFeedback {
    /// Whether this status is a failure: danger-colored in the footer and
    /// draft-keeping for `dirty` (Save/Discard stay enabled), exactly where
    /// the old `error`/`message` field split decided it.
    pub(crate) fn is_error(&self) -> bool {
        !matches!(
            self,
            Self::Saved
                | Self::SavingRequested
                | Self::SavingLatencyPolicy
                | Self::RestartingAudio
                | Self::Restarted
                // Diagnostic prose is data, but a malformed period is still
                // actionable input feedback and must keep the draft dirty.
                | Self::Diagnostic { .. }
        )
    }
}

/// Which request the service ignored without a diagnostic, or which restart
/// outcome failed ([`Rejection::NoRestart`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    AudioSettings,
    TransportLatency,
    LatencyPolicy,
    NoRestart,
}

/// The saved domain a conflicting edit touches; free of rendered labels so
/// the footer composes its own locale's wording at paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Domain {
    Backend,
    Processing,
    LatencyPolicy,
    RenderQueue,
    ProcessIngressLimit,
    ProcessInput,
    VadInput,
    VadRecoveryCap,
    InputDefaults,
    OutputDefaults,
    /// A device id as raw data, never a pre-rendered label.
    InputOverride(String),
    OutputOverride(String),
}

/// Paint-time translation of the stored status: one literal `t!` sentence
/// per case, payloads interpolated as data, reading the process locale like
/// every other `t!` paint site.
pub(crate) fn feedback_text(feedback: &DraftFeedback) -> String {
    feedback_text_in(feedback, &rust_i18n::locale())
}

/// [`feedback_text`] under an explicit locale — the locale-flip test's seam,
/// so parallel tests never touch the process-global locale.
pub(super) fn feedback_text_in(feedback: &DraftFeedback, locale: &str) -> String {
    match feedback {
        DraftFeedback::Saved => {
            t!("Saved. Audio settings apply on the next start.", locale = locale).to_string()
        }
        DraftFeedback::SavingRequested => {
            t!("Saving requested settings...", locale = locale).to_string()
        }
        DraftFeedback::SavingLatencyPolicy => {
            t!("Saving latency policy...", locale = locale).to_string()
        }
        DraftFeedback::RestartingAudio => {
            t!("Restarting audio; waiting for the active session...", locale = locale).to_string()
        }
        DraftFeedback::Restarted => {
            t!("Audio restarted with the saved settings.", locale = locale).to_string()
        }
        DraftFeedback::PreferencesDirUnavailable => t!(
            "User configuration directory is unavailable. Your draft is retained.",
            locale = locale
        )
        .to_string(),
        DraftFeedback::Rejected { what } => match what {
            Rejection::AudioSettings => t!("Audio settings were rejected.", locale = locale),
            Rejection::TransportLatency => {
                t!("Input buffer settings were rejected.", locale = locale)
            }
            Rejection::LatencyPolicy => t!("Latency policy was rejected.", locale = locale),
            Rejection::NoRestart => t!("Audio did not restart. Your draft is retained.", locale = locale),
        }
        .to_string(),
        DraftFeedback::Failure { failure } => {
            crate::ui::presentation::failure_text_in(failure, locale)
        }
        DraftFeedback::Diagnostic { cause } => cause.clone(),
        DraftFeedback::SaveDispatchFailed { failure } => t!(
            "Could not save audio settings: %{error}",
            locale = locale,
            error = crate::ui::presentation::failure_text_in(failure, locale)
        )
        .to_string(),
        DraftFeedback::ApplyDispatchFailed { failure } => t!(
            "Could not apply audio settings: %{error}",
            locale = locale,
            error = crate::ui::presentation::failure_text_in(failure, locale)
        )
        .to_string(),
        DraftFeedback::DomainChanged { domain } => t!(
            "%{label} changed elsewhere. Your draft is retained. Discard changes to reload the saved settings.",
            locale = locale,
            label = domain_label(domain, locale)
        )
        .to_string(),
        DraftFeedback::ValidateInvalid { error } => t!(error.to_string(), locale = locale).to_string(),
        DraftFeedback::PreferencesSaveFailed { cause } => t!(
            "Could not save preferences: %{error}",
            locale = locale,
            error = cause
        )
        .to_string(),
    }
}

/// The conflicting domain's display label, translated at paint; the override
/// cases compose the shared "%{label} override for %{id}" shape with the
/// device id as data.
fn domain_label(domain: &Domain, locale: &str) -> String {
    match domain {
        Domain::Backend => t!("Backend", locale = locale).to_string(),
        Domain::Processing => t!("Processing", locale = locale).to_string(),
        Domain::LatencyPolicy => t!("Device buffer policy", locale = locale).to_string(),
        Domain::RenderQueue => t!("Render queue", locale = locale).to_string(),
        Domain::ProcessIngressLimit => {
            t!("Process host ingress elapsed limit", locale = locale).to_string()
        }
        Domain::ProcessInput => t!("Process input buffer", locale = locale).to_string(),
        Domain::VadInput => t!("VAD input buffer", locale = locale).to_string(),
        Domain::VadRecoveryCap => t!("VAD recovery cap", locale = locale).to_string(),
        Domain::InputDefaults => t!("Input defaults", locale = locale).to_string(),
        Domain::OutputDefaults => t!("Output defaults", locale = locale).to_string(),
        Domain::InputOverride(id) => t!(
            "%{label} override for %{id}",
            locale = locale,
            label = t!("Input", locale = locale),
            id = id
        )
        .to_string(),
        Domain::OutputOverride(id) => t!(
            "%{label} override for %{id}",
            locale = locale,
            label = t!("Output", locale = locale),
            id = id
        )
        .to_string(),
    }
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
            feedback: None,
            pending: None,
            revision: 0,
        }
    }

    pub fn dirty(&self) -> bool {
        self.feedback.as_ref().is_some_and(DraftFeedback::is_error)
            || self.audio != self.original.0
            || self.profile != self.original.1
            || self.transport != self.original.2
    }

    /// Marks the draft user-edited. Every interactive write (segmented
    /// choices, value-menu items, the Override/Use-defaults buttons, the
    /// save entry point) funnels through here right after mutating, so the
    /// revision bump rides the one call they already share.
    pub fn changed(&mut self) {
        self.revision += 1;
        self.feedback = None;
    }

    pub fn committed(&mut self) {
        self.revision += 1;
        self.original = (self.audio.clone(), self.profile, self.transport);
        self.pending = None;
        self.feedback = Some(DraftFeedback::Saved);
    }

    pub(super) fn submission(
        &self,
        current: &AudioSettings,
        profile: AudioLatencyProfile,
        transport: TransportLatencySettings,
    ) -> Result<Submission, DraftFeedback> {
        let merged_transport = self.merge_transport(transport)?;
        Ok(Submission {
            audio: rebase_audio(&self.original.0, &self.audio, current)?,
            profile: merge_value(
                self.original.1,
                self.profile,
                profile,
                Domain::LatencyPolicy,
            )?,
            transport: merged_transport,
            measured_process: None,
            save_transport: merged_transport != transport,
        })
    }

    fn merge_transport(
        &self,
        current: TransportLatencySettings,
    ) -> Result<TransportLatencySettings, DraftFeedback> {
        Ok(TransportLatencySettings {
            render: merge_value(
                self.original.2.render,
                self.transport.render,
                current.render,
                Domain::RenderQueue,
            )?,
            process_max_ingress_elapsed_100ns: merge_value(
                self.original.2.process_max_ingress_elapsed_100ns,
                self.transport.process_max_ingress_elapsed_100ns,
                current.process_max_ingress_elapsed_100ns,
                Domain::ProcessIngressLimit,
            )?,
            process: merge_value(
                self.original.2.process,
                self.transport.process,
                current.process,
                Domain::ProcessInput,
            )?,
            vad: merge_value(
                self.original.2.vad,
                self.transport.vad,
                current.vad,
                Domain::VadInput,
            )?,
            vad_recovery_cap_periods: merge_value(
                self.original.2.vad_recovery_cap_periods,
                self.transport.vad_recovery_cap_periods,
                current.vad_recovery_cap_periods,
                Domain::VadRecoveryCap,
            )?,
        })
    }

    pub(super) fn capture(&self, snapshot: &AppSnapshot) -> Result<Submission, DraftFeedback> {
        let mut submitted = self.submission(
            &snapshot.audio_settings,
            snapshot.latency_profile,
            snapshot.transport_latency,
        )?;
        // A changed consumer envelope needs fresh measured admission; never
        // reuse a finite-window proof for another envelope. An unchanged save
        // preserves the service's newer selection instead of an old draft.
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

/// Merge one domain. The error carries the conflicting domain as data; the
/// footer composes its label at paint ([`domain_label`]).
fn merge_value<T: PartialEq>(
    original: T,
    edited: T,
    current: T,
    domain: Domain,
) -> Result<T, DraftFeedback> {
    if edited == original {
        Ok(current)
    } else if current == original || current == edited {
        Ok(edited)
    } else {
        Err(DraftFeedback::DomainChanged { domain })
    }
}

/// Merge changed domains only. A device's full stream override is atomic;
/// unrelated device edits and newer defaults remain owned by the service.
fn rebase_audio(
    original: &AudioSettings,
    edited: &AudioSettings,
    current: &AudioSettings,
) -> Result<AudioSettings, DraftFeedback> {
    let mut merged = current.clone();
    merged.backend = merge_value(
        original.backend,
        edited.backend,
        current.backend,
        Domain::Backend,
    )?;
    merged.processing = merge_value(
        original.processing,
        edited.processing,
        current.processing,
        Domain::Processing,
    )?;
    merged.input = merge_value(
        original.input,
        edited.input,
        current.input,
        Domain::InputDefaults,
    )?;
    merged.output = merge_value(
        original.output,
        edited.output,
        current.output,
        Domain::OutputDefaults,
    )?;
    for (original, edited, current, target, direction) in [
        (
            &original.input_overrides,
            &edited.input_overrides,
            &current.input_overrides,
            &mut merged.input_overrides,
            Direction::Input,
        ),
        (
            &original.output_overrides,
            &edited.output_overrides,
            &current.output_overrides,
            &mut merged.output_overrides,
            Direction::Output,
        ),
    ] {
        for id in original
            .keys()
            .chain(edited.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let domain = match direction {
                Direction::Input => Domain::InputOverride(id.clone()),
                Direction::Output => Domain::OutputOverride(id.clone()),
            };
            let value = merge_value(original.get(id), edited.get(id), current.get(id), domain)?;
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
        self.graph_view.node_search.update(cx, |search, cx| {
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
        self.presentation.workspace = WorkspaceView::Settings {
            resume: self.presentation.workspace.page(),
        };
        self.chrome.project_menu = false;
        window.focus(&self.settings.editor.focus, cx);
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.presentation.workspace = WorkspaceView::Page(self.presentation.workspace.page());
        self.settings.menu = None;
        if let Some(focus) = self.settings.editor.previous_focus.take() {
            window.focus(&focus, cx);
        } else {
            window.blur(cx);
        }
        cx.notify();
    }

    /// The gear's click: flips between the workspace page and the Settings
    /// page that replaces it.
    pub(crate) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.presentation.workspace.is_settings() {
            self.close_settings(window, cx);
        } else {
            self.open_settings(window, cx);
        }
    }

    pub(crate) fn keep_settings_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The modal is a sibling of the retained workspace. Moving focus also
        // removes graph Delete/Escape handlers from the keyboard dispatch path.
        if self.presentation.workspace.is_settings()
            && !self.settings.editor.focus.contains_focused(window, cx)
        {
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
            let mut fresh = self.current_settings_draft(cx);
            // A fresh draft restarts at revision 0; carrying the successor
            // keeps the replacement visible to the panel keys even when the
            // selection (and the captured values) compare equal — the draft
            // is a render input of the Devices and Inspector panels.
            fresh.revision = self.settings.editor.device.revision.wrapping_add(1);
            self.settings.editor.device = fresh;
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
        if self.chrome.density == density {
            return;
        }
        self.chrome.density = density;
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
