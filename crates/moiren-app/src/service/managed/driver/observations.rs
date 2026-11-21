//! Professional observation projection, execution-state projection, and the
//! starting-progress detail.
use super::super::*;

/// Project the backend's live prepare progress (issue #105) into the
/// Project the backend's live prepare progress (issue #105) into the
/// snapshot's Starting detail: shown only while the owner that published it
/// is still preparing, cleared on every other execution state so a stale
/// record never outlives its prepare.
#[cfg(windows)]
fn starting_progress(
    progress: Option<moiren_windows_audio::engine_backend::ProfessionalPrepareProgress>,
    owner_generation: u64,
    execution: ExecutionState,
) -> Option<super::StartingProgress> {
    let progress = progress?;
    (progress.owner_generation == owner_generation && execution == ExecutionState::Preparing)
        .then_some(super::StartingProgress {
            opened: progress.groups_opened,
            total: progress.groups_total,
            endpoint: progress.endpoint,
        })
}

impl ManagedDriver {
    /// Publish native and professional observations, filtered by the running
    /// owner generation. Refreshes are tick-batched at 1 Hz: the raw counters
    /// advance every audio block, while the UI renders only static timing
    /// rows. Owner/execution transitions bypass this cadence and invalidate
    /// stale native snapshots before they can be published again.
    #[cfg(windows)]
    pub(in crate::service::managed) fn poll_observations(
        &mut self,
        c: &mut ServiceContext,
        state: &host::HostSnapshot,
        generation: SessionGeneration,
    ) {
        let now = std::time::Instant::now();
        let identity = (generation, state.owner_generation, state.execution);
        // Recovery can replace a native owner inside one app generation.
        // Invalidate stale DTOs before the cadence guard, not one second later.
        let fresh = |observed_generation, running, owner_generation| {
            observed_generation == generation
                && running
                && owner_generation == state.owner_generation
                && state.execution == ExecutionState::Running
        };
        if c.native_observations.as_ref().is_some_and(|value| {
            !fresh(
                value.generation,
                value.observation.running,
                value.observation.owner_generation,
            )
        }) {
            c.native_observations = None;
        }
        if c.professional_observations.as_ref().is_some_and(|value| {
            !fresh(
                value.generation,
                value.observation.running,
                value.observation.owner_generation,
            )
        }) {
            c.professional_observations = None;
        }
        let due = self.observation_identity != Some(identity)
            || self
                .observation_refreshed_at
                .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(1));
        if !due {
            return;
        }
        self.observation_refreshed_at = Some(now);
        self.observation_identity = Some(identity);
        // Write-side stale guard: an observation counts only while its own
        // owner generation still runs.
        c.professional_observations = self
            .professional_observations
            .as_ref()
            .and_then(|handle| {
                handle
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                    .and_then(|value| value.snapshot())
            })
            .filter(|observation| {
                fresh(
                    generation,
                    observation.running,
                    observation.owner_generation,
                )
            })
            .map(|observation| ProfessionalSessionObservations {
                generation,
                observation,
            });
        c.native_observations = self
            .native_observations
            .as_ref()
            .and_then(|handle| {
                handle
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                    .and_then(|value| value.snapshot())
            })
            .filter(|observation| {
                fresh(
                    generation,
                    observation.running,
                    observation.owner_generation,
                )
            })
            .map(|observation| NativeSessionObservations {
                generation,
                observation,
            });
        // Live open progress rides the same 1 Hz cadence: it only matters
        // while the owner with the same generation is actually preparing,
        // and any other phase clears the detail.
        c.starting_progress = starting_progress(
            self.professional_observations.as_ref().and_then(|handle| {
                handle
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                    .and_then(|value| value.snapshot_progress())
            }),
            state.owner_generation,
            state.execution,
        );
        // Absent endpoints surface as node faults: the stream runs silent and
        // the badge explains the silence. Recording on the refresh cadence
        // keeps the fault visible across fault-group clears; record_fault
        // dedupes. Only a plainly missing device revives automatically; a
        // stuck open or a runtime-unplugged endpoint keeps its suppress
        // verdict for the whole backend session (the anti-reopen-storm
        // design, audit X12), so the narration must not promise an
        // automatic resume for every flavor - recovery is one re-plug
        // cycle (seen missing, then returns) or a fresh apply, which
        // builds a fresh backend with empty verdicts.
        if let Some(observations) = &c.professional_observations {
            for stream in observations
                .observation
                .streams
                .iter()
                .filter(|stream| stream.absent)
            {
                let endpoint = match &stream.endpoint {
                    moiren_windows_audio::engine_backend::StreamEndpoint::Physical(id) => {
                        id.clone()
                    }
                    _ => continue,
                };
                c.graph.record_fault(super::GraphFault {
                    subject: super::GraphSubject::Node(stream.node),
                    cause: format!(
                        "The device for this {} is absent ('{endpoint}'); \
                         the stream stays silent until the device is seen missing and \
                          returns, or the session is re-applied",
                        if stream.input { "input" } else { "output" },
                    ),
                    hint: Some(
                        "Re-plug the device (unplug and plug back in), or apply \
                          again to retry with a fresh audio session."
                            .into(),
                    ),
                });
            }
        }
    }
    #[cfg(not(windows))]
    pub(in crate::service::managed) fn poll_observations(
        &mut self,
        _c: &mut ServiceContext,
        _state: &host::HostSnapshot,
        _generation: SessionGeneration,
    ) {
    }
    /// Project execution state, faults, the active latency profile, and
    /// settled-revision confirmations into the service context.
    pub(super) fn poll_execution(
        &mut self,
        c: &mut ServiceContext,
        snapshot: &host::ProjectSnapshot,
    ) {
        let Some(owner) = self.owner.as_mut() else {
            return;
        };
        let running = snapshot.state.execution == ExecutionState::Running;
        c.active_execution = Some(snapshot.state.execution);
        let error = if snapshot.state.execution == ExecutionState::WaitingForResources {
            match &snapshot.state.update {
                UpdateState::Blocked { reason, .. } => Some(format!("{reason:?}")),
                _ => snapshot
                    .state
                    .fault
                    .as_ref()
                    .map(|error| format!("{error:?}")),
            }
        } else {
            None
        };
        if let Some(error) = error.as_deref()
            && c.core.error() != Some(error)
        {
            log::error!(
                target: "moiren::audio::service",
                "audio owner reported a backend fault: generation={} state={:?} fault={error}",
                owner.generation.0,
                snapshot.state.execution
            );
        }
        c.core.managed_execution(owner.generation, running, error);
        if running {
            // The latency profile applies to every configuration now: it maps
            // to a device buffer request when no explicit buffer is set.
            c.active_latency_profile = Some(owner.profile);
            c.active_audio_settings = Some(owner.audio_settings.clone());
            c.active_transport_latency = Some(owner.transport_latency);
            c.active_measured_process = owner
                .measured_process
                .as_ref()
                .map(|profile| profile.summary().clone());
            if owner.started.is_none() {
                log::info!(
                    target: "moiren::audio::service",
                    "audio owner started: generation={} backend={} processing_rate={} precision={:?} block_frames={}",
                    owner.generation.0,
                    owner.audio_settings.backend.title(),
                    owner.audio_settings.processing.sample_rate,
                    owner.audio_settings.processing.precision,
                    owner.audio_settings.processing.block_frames
                );
                owner.started = Some(Instant::now());
            }
            // An old confirmation still describes valid playback, but must not
            // finish a new Apply of that same revision. Its own ACK barrier below
            // clears pending; periodic projection only restores settled state.
            if snapshot.state.active_revision == Some(owner.active_revision.0)
                && (owner.active_apply_id == owner.latest_apply_id
                    || c.graph.pending_revision() != Some(owner.active_revision.1))
            {
                c.graph.confirmed(owner.generation, owner.active_revision.1);
            }
        } else if owner.started.is_some()
            && snapshot.state.execution != ExecutionState::WaitingForResources
        {
            // The engine ran and then went dormant on its own (a fault), and
            // it is not waiting for resources: the always-on session keeps
            // its intent, but this owner's settled projection is gone.
            c.active_latency_profile = None;
            c.active_audio_settings = None;
            c.active_transport_latency = None;
            c.active_measured_process = None;
            c.graph.stopped();
        }
        if let UpdateState::Blocked { reason, .. } = &snapshot.state.update {
            c.graph
                .preparation_failed(owner.generation, format!("{reason:?}"));
        }
        if snapshot.state.execution == ExecutionState::WaitingForResources {
            c.graph
                .preparation_failed(owner.generation, format!("{:?}", snapshot.state.fault));
        }
    }
}

#[cfg(all(test, windows))]
mod starting_progress_tests {
    use super::starting_progress;
    use moiren_engine::host::ExecutionState;
    use moiren_windows_audio::engine_backend::ProfessionalPrepareProgress;

    fn progress(generation: u64, opened: usize) -> Option<ProfessionalPrepareProgress> {
        Some(ProfessionalPrepareProgress {
            owner_generation: generation,
            groups_total: 3,
            groups_opened: opened,
            endpoint: (opened < 3).then(|| "{opening}".to_owned()),
        })
    }

    #[test]
    fn progress_projects_only_while_its_owner_prepares() {
        use super::super::StartingProgress;
        assert_eq!(
            starting_progress(progress(7, 1), 7, ExecutionState::Preparing),
            Some(StartingProgress {
                opened: 1,
                total: 3,
                endpoint: Some("{opening}".into()),
            })
        );
        // Any other execution state clears the detail.
        for execution in [
            ExecutionState::Dormant,
            ExecutionState::Running,
            ExecutionState::WaitingForResources,
        ] {
            assert_eq!(
                starting_progress(progress(7, 1), 7, execution),
                None,
                "{execution:?} must not show starting progress"
            );
        }
        // A stale record from another owner generation never projects.
        assert_eq!(
            starting_progress(progress(6, 2), 7, ExecutionState::Preparing),
            None
        );
        assert_eq!(starting_progress(None, 7, ExecutionState::Preparing), None);
    }
}
