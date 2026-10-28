use super::*;
use crate::service::{AppHandle, AppSnapshot};
#[cfg(windows)]
use crate::service::{MeasuredProcessRequest, RequestId};

pub(super) struct Measurement {
    seconds: Option<u16>,
    #[cfg(windows)]
    target: Option<(u32, u32, u32)>,
    stage: Stage,
}

enum Stage {
    Baseline,
    #[cfg(windows)]
    Collecting {
        since: Instant,
        generation: u64,
        revision: u64,
        native: u64,
    },
    #[cfg(windows)]
    Applying {
        since: Instant,
        request: MeasuredProcessRequest,
        id: RequestId,
    },
    Ready,
}

impl Measurement {
    pub(super) fn new(options: &MonitorOptions) -> Self {
        Self {
            seconds: options.measure_seconds,
            #[cfg(windows)]
            target: options.measured_target().expect("validated measured tuple"),
            stage: if options.measure_seconds.is_some() {
                Stage::Baseline
            } else {
                Stage::Ready
            },
        }
    }

    pub(super) fn ready(&self) -> bool {
        matches!(self.stage, Stage::Ready)
    }

    pub(super) fn startup_expired(&self, began: Instant, timeout_ms: u64) -> bool {
        if self.seconds.is_none() {
            return began.elapsed() >= Duration::from_millis(timeout_ms);
        }
        let elapsed = match self.stage {
            #[cfg(windows)]
            Stage::Collecting { since, .. } => {
                // A missing publication cannot keep collection alive forever.
                // Collection time and the fixed observation grace share this
                // single anchor; successful polls never renew it.
                return since.elapsed()
                    >= Duration::from_secs(u64::from(self.seconds.unwrap()))
                        + Duration::from_millis(timeout_ms);
            }
            Stage::Ready => return false,
            #[cfg(windows)]
            Stage::Applying { since, .. } => since.elapsed(),
            Stage::Baseline => began.elapsed(),
        };
        elapsed >= Duration::from_millis(timeout_ms)
    }

    pub(super) fn observe(
        &mut self,
        snapshot: &AppSnapshot,
        handle: &AppHandle,
        report: &mut MonitorReport,
        began: Instant,
        timeout_ms: u64,
    ) -> Result<()> {
        ensure!(
            !self.startup_expired(began, timeout_ms),
            "monitor startup deadline elapsed before matching active confirmation"
        );
        if self.seconds.is_none() {
            return Ok(());
        }
        #[cfg(windows)]
        {
            let matching = snapshot.phase == SessionPhase::Running
                && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                && snapshot.graph.pending_revision.is_none()
                && snapshot.active_audio_settings.as_ref() == Some(&report.requested_audio)
                && snapshot.active_transport_latency == Some(report.requested_transport);
            if matches!(self.stage, Stage::Baseline) && matching {
                let Some(native) = snapshot.native_observations.as_ref().filter(|native| {
                    native.generation == snapshot.generation && native.observation.running
                }) else {
                    // Wait within the one startup deadline; a Running phase
                    // alone is not a complete native observation publication.
                    return Ok(());
                };
                let generation = snapshot.generation.0;
                let revision = snapshot.graph.candidate_revision;
                let native = native.observation.owner_generation;
                report.measurement_generation = Some(generation);
                report.measurement_native_generation = Some(native);
                self.stage = Stage::Collecting {
                    since: Instant::now(),
                    generation,
                    revision,
                    native,
                };
            }
            match self.stage {
                Stage::Collecting {
                    since,
                    generation,
                    revision,
                    native,
                } => {
                    ensure!(
                        matching
                            && snapshot.generation.0 == generation
                            && snapshot.graph.active_revision == Some(revision),
                        "measured baseline owner/revision changed"
                    );
                    report.observed_measurement_ms =
                        since.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                    let observations = snapshot
                        .native_observations
                        .as_ref()
                        .filter(|value| {
                            value.generation.0 == generation
                                && value.observation.running
                                && value.observation.owner_generation == native
                        })
                        .context(
                            "measured baseline native owner changed or observations disappeared",
                        )?;
                    if since.elapsed() >= Duration::from_secs(u64::from(self.seconds.unwrap())) {
                        let (node, state) = snapshot
                            .graph
                            .node_of_kind(NodeKind::Source)
                            .context("measured source missing")?;
                        ensure!(
                            state.source
                                == Some(InputSelection::Process {
                                    pid: report.process_pid,
                                    creation_time_100ns: report.process_creation_time_100ns,
                                }),
                            "measured source identity changed"
                        );
                        let observed = observations
                            .observation
                            .process_delivery
                            .iter()
                            .find(|value| value.node == node.id())
                            .context("measured source delivery observations missing")?;
                        ensure!(
                            observed.process.pid == report.process_pid
                                && observed.process.creation_time_100ns
                                    == report.process_creation_time_100ns,
                            "measured native Process identity changed"
                        );
                        let (capacity_frames, target_frames, jitter_frames) = self.target.unwrap();
                        let request = MeasuredProcessRequest {
                            node: node.id(),
                            session_generation: snapshot.generation,
                            native_owner_generation: native,
                            graph_revision: revision,
                            capacity_frames,
                            target_frames,
                            jitter_frames,
                        };
                        // Fail explicitly at the measured boundary rather than silently
                        // trying a compatibility/default tuple after a bad profile.
                        crate::service::measured_process::MeasuredProcessProfile::from_observation(
                            1, request, observed,
                        )
                        .map_err(anyhow::Error::msg)?;
                        let id = handle
                            .try_request_tracked(
                                crate::service::AppRequest::ApplyMeasuredProcessBridge(request),
                            )
                            .map_err(|error| {
                                anyhow::anyhow!("measured Apply request refused: {error:?}")
                            })?;
                        self.stage = Stage::Applying {
                            since: Instant::now(),
                            request,
                            id,
                        };
                    }
                }
                Stage::Applying { request, id, .. } => {
                    if snapshot.consumed_request_id < id {
                        return Ok(());
                    }
                    if matching
                        && snapshot.generation != request.session_generation
                        && live_native_generation(snapshot).is_some()
                        && snapshot
                            .active_measured_process
                            .as_ref()
                            .is_some_and(|profile| profile.request == request)
                        && snapshot.measured_process == snapshot.active_measured_process
                    {
                        self.stage = Stage::Ready;
                    }
                }
                Stage::Baseline | Stage::Ready => {}
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (snapshot, handle, report);
            anyhow::bail!("measured Process monitor is only supported on Windows")
        }
    }
}

#[cfg(all(test, windows))]
mod tests;
