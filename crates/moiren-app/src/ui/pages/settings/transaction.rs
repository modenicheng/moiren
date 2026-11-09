use super::model::{DraftFeedback, Pending, Rejection, Stage, expected_active_profile};
use super::*;
use crate::ui::state::UiFailure;

impl StudioShell {
    pub(crate) fn save_audio_settings(
        &mut self,
        editor: Editor,
        restart: bool,
        cx: &mut Context<Self>,
    ) {
        if self.settings.editor.busy() {
            return;
        }
        let draft = self.settings.editor.draft_mut(editor);
        draft.changed();
        if self.preferences.path.is_none() {
            draft.feedback = Some(DraftFeedback::PreferencesDirUnavailable);
            cx.notify();
            return;
        }
        let snapshot = &self.state.read(cx).snapshot;
        let submitted = match draft.submission(&snapshot.audio_settings, snapshot.latency_profile) {
            Ok(submitted) => submitted,
            Err(feedback) => {
                draft.feedback = Some(feedback);
                cx.notify();
                return;
            }
        };
        if let Err(error) = submitted.audio.validate() {
            draft.feedback = Some(DraftFeedback::ValidateInvalid { error });
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
                draft.feedback = Some(DraftFeedback::SavingRequested);
            }
            Err(error) => {
                draft.feedback = Some(DraftFeedback::SaveDispatchFailed {
                    failure: UiFailure::Dispatch(error),
                });
            }
        }
        cx.notify();
    }

    pub(crate) fn finish_settings_save(&mut self, cx: &mut Context<Self>) {
        for editor in [Editor::Global, Editor::Device] {
            let draft = self.settings.editor.draft(editor);
            let Some(pending) = draft.pending.clone() else {
                continue;
            };
            let state = self.state.read(cx);
            let snapshot = &state.snapshot;
            if snapshot.consumed_request_id < pending.request {
                continue;
            }
            // The captured failure stays typed: `UiState.dispatch_error`
            // whole, the service's free-form prose as diagnostic data. The
            // footer translates whichever was stored at paint.
            let failure = state
                .dispatch_error
                .clone()
                .map(|failure| DraftFeedback::Failure { failure })
                .or_else(|| {
                    snapshot
                        .error
                        .clone()
                        .map(|cause| DraftFeedback::Diagnostic { cause })
                })
                .or_else(|| {
                    snapshot
                        .graph
                        .last_error
                        .clone()
                        .map(|cause| DraftFeedback::Diagnostic { cause })
                });
            let settings_match = snapshot.audio_settings == pending.submitted.audio;
            let profile_match = snapshot.latency_profile == pending.submitted.profile;
            let mut next = None;
            let mut complete = false;
            let mut feedback = None;
            match pending.stage {
                Stage::Audio | Stage::Profile if !settings_match => {
                    feedback = Some(failure.unwrap_or(DraftFeedback::Rejected {
                        what: Rejection::AudioSettings,
                    }));
                }
                Stage::Audio if !profile_match => {
                    next = Some((
                        AppRequest::SetLatencyProfile(pending.submitted.profile),
                        Stage::Profile,
                    ));
                }
                Stage::Profile if !profile_match => {
                    feedback = Some(failure.unwrap_or(DraftFeedback::Rejected {
                        what: Rejection::LatencyPolicy,
                    }));
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
                        feedback = Some(failure);
                    } else if matches!(snapshot.phase, SessionPhase::Idle | SessionPhase::Failed) {
                        feedback = Some(DraftFeedback::Rejected {
                            what: Rejection::NoRestart,
                        });
                    }
                }
            }
            if let Some((request, stage)) = next {
                let result = self
                    .state
                    .update(cx, |state, _| state.try_dispatch(request));
                let draft = self.settings.editor.draft_mut(editor);
                match result {
                    Ok(request) => {
                        draft.pending = Some(Pending {
                            request,
                            stage,
                            ..pending.clone()
                        });
                        draft.feedback = Some(if matches!(stage, Stage::Restart) {
                            DraftFeedback::RestartingAudio
                        } else {
                            DraftFeedback::SavingLatencyPolicy
                        });
                    }
                    Err(error) => {
                        feedback = Some(DraftFeedback::ApplyDispatchFailed {
                            failure: UiFailure::Dispatch(error),
                        });
                    }
                }
            }
            if let Some(feedback) = feedback {
                let draft = self.settings.editor.draft_mut(editor);
                draft.feedback = Some(feedback);
                draft.pending = None;
            } else if complete {
                let draft = self.settings.editor.draft_mut(editor);
                draft.audio = pending.submitted.audio;
                draft.profile = pending.submitted.profile;
                self.preferences.values.audio = draft.audio.clone();
                self.preferences.values.latency_profile = draft.profile;
                draft.committed();
                if matches!(pending.stage, Stage::Restart) {
                    draft.feedback = Some(DraftFeedback::Restarted);
                }
                self.save_preferences(cx);
                self.settings.editor.save_revision = Some((self.preferences.save_revision, editor));
            }
        }
    }
}
