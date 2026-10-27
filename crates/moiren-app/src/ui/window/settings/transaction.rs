use rust_i18n::t;

use super::model::{Pending, Stage, expected_active_profile};
use super::*;

impl StudioShell {
    pub(in super::super) fn save_audio_settings(
        &mut self,
        editor: Editor,
        restart: bool,
        cx: &mut Context<Self>,
    ) {
        if self.settings.busy() {
            return;
        }
        let draft = self.settings.draft_mut(editor);
        draft.changed();
        if self.preferences_path.is_none() {
            draft.error = Some(
                t!("User configuration directory is unavailable. Your draft is retained.").into(),
            );
            cx.notify();
            return;
        }
        let snapshot = &self.state.read(cx).snapshot;
        let submitted = match draft.submission(
            &snapshot.audio_settings,
            snapshot.latency_profile,
            self.density,
        ) {
            Ok(submitted) => submitted,
            Err(error) => {
                draft.error = Some(error);
                cx.notify();
                return;
            }
        };
        if let Err(error) = submitted.audio.validate() {
            draft.error = Some(error.to_string());
            cx.notify();
            return;
        }
        // Recheck the phase at activation: a stale running button must never
        // turn Save into an unexpected start after audio has stopped.
        let snapshot = &self.state.read(cx).snapshot;
        let restart = restart
            && snapshot.phase == SessionPhase::Running
            && (snapshot.active_audio_settings.as_ref() != Some(&submitted.audio)
                || snapshot.active_latency_profile != expected_active_profile(submitted.profile));
        match self.state.update(cx, |state, _| {
            state.try_dispatch(AppRequest::SetAudioSettings(submitted.audio.clone()))
        }) {
            Ok(request) => {
                draft.pending = Some(Pending {
                    request,
                    stage: Stage::Audio,
                    restart,
                    submitted,
                });
                draft.message = Some(t!("Saving requested settings...").into());
            }
            Err(error) => {
                draft.error = Some(
                    t!(
                        "Could not save audio settings: %{error}",
                        error = format!("{error:?}")
                    )
                    .to_string(),
                )
            }
        }
        cx.notify();
    }

    pub(in super::super) fn finish_settings_save(&mut self, cx: &mut Context<Self>) {
        for editor in [Editor::Global, Editor::Device] {
            let draft = self.settings.draft(editor);
            let Some(pending) = draft.pending.clone() else {
                continue;
            };
            let state = self.state.read(cx);
            let snapshot = &state.snapshot;
            if snapshot.consumed_request_id < pending.request {
                continue;
            }
            let failure = state
                .dispatch_error
                .clone()
                .or_else(|| snapshot.error.clone())
                .or_else(|| snapshot.graph.last_error.clone());
            let settings_match = snapshot.audio_settings == pending.submitted.audio;
            let profile_match = snapshot.latency_profile == pending.submitted.profile;
            let mut next = None;
            let mut complete = false;
            let mut error = None;
            match pending.stage {
                Stage::Audio | Stage::Profile if !settings_match => {
                    error =
                        Some(failure.unwrap_or_else(|| t!("Audio settings were rejected.").into()));
                }
                Stage::Audio if !profile_match => {
                    next = Some((
                        AppRequest::SetLatencyProfile(pending.submitted.profile),
                        Stage::Profile,
                    ));
                }
                Stage::Profile if !profile_match => {
                    error =
                        Some(failure.unwrap_or_else(|| t!("Latency policy was rejected.").into()));
                }
                Stage::Audio | Stage::Profile => {
                    if pending.restart && snapshot.phase == SessionPhase::Running {
                        next = Some((AppRequest::ApplyGraph, Stage::Restart));
                    } else {
                        complete = true;
                    }
                }
                Stage::Restart => {
                    if snapshot.phase == SessionPhase::Running
                        && snapshot.active_audio_settings.as_ref() == Some(&pending.submitted.audio)
                        && snapshot.active_latency_profile
                            == expected_active_profile(pending.submitted.profile)
                        && snapshot.graph.pending_revision.is_none()
                        && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                        && failure.is_none()
                    {
                        complete = true;
                    } else if let Some(failure) = failure {
                        error = Some(failure);
                    } else if matches!(snapshot.phase, SessionPhase::Idle | SessionPhase::Failed) {
                        error = Some(t!("Audio did not restart. Your draft is retained.").into());
                    }
                }
            }
            if let Some((request, stage)) = next {
                let result = self
                    .state
                    .update(cx, |state, _| state.try_dispatch(request));
                let draft = self.settings.draft_mut(editor);
                match result {
                    Ok(request) => {
                        draft.pending = Some(Pending {
                            request,
                            stage,
                            ..pending.clone()
                        });
                        draft.message = Some(if matches!(stage, Stage::Restart) {
                            t!("Restarting audio; waiting for the active session...").into()
                        } else {
                            t!("Saving latency policy...").into()
                        });
                    }
                    Err(failure) => {
                        error = Some(
                            t!(
                                "Could not apply audio settings: %{error}",
                                error = format!("{failure:?}")
                            )
                            .to_string(),
                        );
                    }
                }
            }
            if let Some(error) = error {
                let draft = self.settings.draft_mut(editor);
                draft.error = Some(error);
                draft.message = None;
                draft.pending = None;
            } else if complete {
                let draft = self.settings.draft_mut(editor);
                draft.audio = pending.submitted.audio;
                draft.profile = pending.submitted.profile;
                draft.density = pending.submitted.density;
                self.preferences.audio = draft.audio.clone();
                self.preferences.latency_profile = draft.profile;
                self.preferences.density = if draft.density == Density::Compact {
                    crate::project::UiDensity::Compact
                } else {
                    crate::project::UiDensity::Comfortable
                };
                self.density = draft.density;
                draft.committed();
                if matches!(pending.stage, Stage::Restart) {
                    draft.message = Some(t!("Audio restarted with the saved settings.").into());
                }
                self.save_preferences(cx);
                self.settings.save_revision = Some((self.preference_save_revision, editor));
            }
        }
    }
}
