//! One concrete VAD source/graph/independent-recorder run, always unqualified.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use std::time::Duration;
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    schema_version: u32,
    input_endpoint_id: String,
    ring_frames: usize,
    max_pcm_bytes: u64,
    max_metadata_bytes: u64,
    max_packets: u64,
    preroll_ms: u64,
    postroll_ms: u64,
}

impl Config {
    fn validate(&self, frames: u64, seconds: u64) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported collection schema");
        ensure!(
            !self.input_endpoint_id.trim().is_empty() && !self.input_endpoint_id.contains('\0'),
            "collection requires explicit independent input"
        );
        ensure!(
            self.ring_frames > 0
                && self
                    .ring_frames
                    .checked_mul(size_of::<moiren_windows_audio::capture::PcmEvidenceFrame>())
                    .is_some_and(|bytes| bytes <= 8 * 1024 * 1024),
            "collection ring exceeds 8 MiB"
        );
        ensure!(
            (1..=3 * 1024 * 1024 * 1024).contains(&self.max_pcm_bytes),
            "collection PCM cap must be 1..3GiB"
        );
        ensure!(
            (1..=256 * 1024 * 1024).contains(&self.max_metadata_bytes),
            "collection metadata cap must be 1..256MiB"
        );
        ensure!(
            (1..=1_000_000).contains(&self.max_packets),
            "collection packet cap must be 1..1000000"
        );
        ensure!(
            self.preroll_ms <= 5000 && self.postroll_ms <= 5000,
            "collection rolls must be 0..5000ms"
        );
        let source_ms = frames
            .checked_mul(1000)
            .context("source duration overflow")?
            .div_ceil(48_000);
        let budget = 5000u64
            .checked_add(self.preroll_ms)
            .and_then(|v| v.checked_add(5000))
            .and_then(|v| v.checked_add(source_ms))
            .and_then(|v| v.checked_add(2000))
            .and_then(|v| v.checked_add(self.postroll_ms))
            .and_then(|v| v.checked_add(20))
            .context("collection budget overflow")?;
        ensure!(
            frames > 0
                && (1..=600).contains(&seconds)
                && budget
                    <= seconds
                        .checked_mul(1000)
                        .context("collection duration overflow")?,
            "collection phase budget exceeds --seconds"
        );
        Ok(())
    }
}

pub(super) fn read_config(
    path: &Path,
    limit: u64,
    frames: u64,
    seconds: u64,
) -> Result<(Vec<u8>, Config)> {
    let limit = limit.min(64 * 1024);
    ensure!(limit > 0, "collection config byte cap must be positive");
    let file = File::open(path).context("open collection config")?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "collection config byte cap exceeded"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "collection config grew beyond byte cap"
    );
    let config: Config = serde_json::from_slice(&bytes)?;
    config.validate(frames, seconds)?;
    Ok((bytes, config))
}

pub(super) fn not_collected(
    config: &Config,
    source: super::source::SourceProvenance,
    failure: &str,
) -> serde_json::Value {
    let mut bounded = None;
    append_failure(&mut bounded, failure);
    serde_json::json!({
        "schema_version": 1, "qualified": false, "status": "unqualified",
        "config": config, "playback_source": source, "collection_complete": false,
        "source_submitted_complete": false, "cleanup_complete": false, "failure": bounded,
        "meaning": "submission and native Start are not physical marker detection or calibration",
    })
}

#[cfg(any(windows, test))]
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct PlaybackProgress {
    pub started: bool,
    pub source: super::source::SourceProgress,
    pub event_timeouts: u64,
    pub stopped: bool,
}

#[cfg(any(windows, test))]
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Wait,
    ActivateSource,
    Complete,
}

#[cfg(any(windows, test))]
struct Phases {
    config: Config,
    ceiling: Duration,
    source_duration: Duration,
    source_frames: u64,
    source_started: bool,
    recorder_ready: Option<Duration>,
    source_requested: Option<Duration>,
    completed: Option<Duration>,
    stamps: Vec<PhaseStamp>,
}

#[cfg(any(windows, test))]
#[derive(Debug, Serialize)]
struct PhaseStamp {
    phase: &'static str,
    elapsed_ms: u128,
}

#[cfg(any(windows, test))]
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Health {
    pub stamp: Option<[u64; 5]>,
    pub shape: Option<(String, String, u32)>,
    pub provider_ready: bool,
    pub provider_error: Option<String>,
    pub revision: Option<u64>,
    pub generations: [u64; 2],
    pub engine_running: bool,
    pub engine_fault: bool,
    pub native_generation: Option<u64>,
    pub native_running: bool,
    pub quality_known: bool,
    pub quality_faults: u64,
}

#[cfg(windows)]
#[derive(Debug, Serialize)]
pub(super) struct Admission {
    pub before_gate: [u64; 5],
    pub current: Health,
}

#[cfg(any(windows, test))]
impl Health {
    pub(super) fn require_present(&self) -> Result<()> {
        ensure!(
            self.stamp.is_some()
                && self.shape.is_some()
                && self.provider_ready
                && self.provider_error.is_none(),
            "provider absent, not admitted, changed shape or reported error: {:?}",
            self.provider_error
        );
        ensure!(
            self.engine_running && !self.engine_fault && self.revision.is_some(),
            "graph not Running with confirmed revision, or faulted"
        );
        ensure!(
            self.native_running && self.native_generation.is_some() && self.quality_known,
            "required native quality observations unavailable"
        );
        ensure!(
            self.quality_faults == 0,
            "native/provider quality faults: {}",
            self.quality_faults
        );
        Ok(())
    }
    pub(super) fn validate(&self, admitted: &Self) -> Result<()> {
        self.require_present()?;
        ensure!(
            self.stamp == admitted.stamp && self.shape == admitted.shape,
            "provider generation/gate/shape changed"
        );
        ensure!(
            self.revision == admitted.revision
                && self.generations == admitted.generations
                && self.native_generation == admitted.native_generation,
            "graph revision/owner/resource generation changed"
        );
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn validate_health(
    current: &Health,
    admitted: &Health,
    report: &super::NativeReport,
    first_failure: &mut Option<serde_json::Value>,
) -> Result<()> {
    let result = current.validate(admitted);
    if result.is_err() {
        latch_failure_observations(first_failure, report, Some(current));
    }
    result
}

#[cfg(any(windows, test))]
fn latch_failure_observations(
    first_failure: &mut Option<serde_json::Value>,
    report: &super::NativeReport,
    health: Option<&Health>,
) {
    // Capture once at discovery: a later successful QUERY can replace cached
    // error provenance, and the bounded startup trace may already be full.
    if first_failure.is_none() {
        *first_failure = Some(serde_json::json!({
            "provider": report.vad_provider, "engine": report.engine, "health": health,
        }));
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retirement {
    Observe,
    ObserveFinal,
    StopSource,
    StopMeasurement,
    JoinSource,
    CloseGate,
    ShutdownHost,
    Invalidate,
    Retire,
    FinishMeasurement,
}

#[cfg(any(windows, test))]
fn retire(mut step: impl FnMut(Retirement) -> Result<()>, failures: &mut Option<String>) {
    for stage in [
        Retirement::Observe,
        Retirement::ObserveFinal,
        Retirement::StopSource,
        Retirement::StopMeasurement,
        Retirement::JoinSource,
        Retirement::CloseGate,
        Retirement::ShutdownHost,
        Retirement::Invalidate,
        Retirement::Retire,
        Retirement::FinishMeasurement,
    ] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(stage)))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("cleanup stage panicked")));
        if let Err(error) = result {
            append_failure(failures, &format!("{stage:?}: {error:#}"));
        }
    }
}

pub(super) fn append_failure(failures: &mut Option<String>, message: &str) {
    let text = failures.get_or_insert_with(String::new);
    let remaining = (16 * 1024usize).saturating_sub(text.len());
    let separator = if text.is_empty() { "" } else { "; " };
    if remaining <= separator.len() {
        return;
    }
    let mut end = (remaining - separator.len()).min(message.len());
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    if end > 0 {
        text.push_str(separator);
        text.push_str(&message[..end]);
    }
}

#[cfg(windows)]
pub(super) fn collect(
    options: &super::LiveOptions,
    paths: &super::ArtifactPaths,
    config: &Config,
    plan: super::source::PreparedSource,
    root: String,
    name: &str,
    vad: super::native::VadCleanup<'_>,
) -> Result<()> {
    use crate::measurement::session::{
        MeasurementConfig, MeasurementPaths, MeasurementProgress, MeasurementSession,
    };
    use std::{thread, time::Instant};
    let provenance = super::source::SourceProvenance::new(Some(&plan));
    let frames = provenance
        .schedule
        .as_ref()
        .context("collection source schedule missing")?
        .total_source_frames;
    let mut phases = Phases::new(config.clone(), frames, options.seconds);
    // Install the outer owner before recorder preparation or CREATE. Even a
    // half-prepared source has a stop-before-join callback into this same owner.
    let mut guard = CollectionGuard {
        vad,
        measurement: None,
        measurement_outcome: None,
        failures: None,
        cleanup_failures: None,
        first_failure_observations: None,
        retired: false,
    };
    let mut last_running = None;
    let collection = (|| -> Result<()> {
        guard.measurement = Some(MeasurementSession::prepare(
            MeasurementConfig {
                input_endpoint_id: config.input_endpoint_id.clone(),
                max_active_duration: Duration::from_secs(options.seconds),
                ring_frames: config.ring_frames,
                max_pcm_bytes: config.max_pcm_bytes,
                max_metadata_bytes: config.max_metadata_bytes,
                max_packets: config.max_packets,
            },
            MeasurementPaths {
                pcm: paths
                    .measurement_pcm
                    .as_deref()
                    .context("collection PCM path missing")?
                    .into(),
                packets: paths
                    .measurement_packets
                    .as_deref()
                    .context("collection packet path missing")?
                    .into(),
            },
        )?);
        {
            let CollectionGuard {
                vad,
                measurement,
                failures,
                ..
            } = &mut guard;
            let mut abort = || {
                if let Some(owner) = measurement.as_mut()
                    && let Err(error) = owner.request_stop()
                {
                    append_failure(
                        failures,
                        &format!("stop recorder before source startup join: {error:#}"),
                    );
                }
            };
            vad.prepare(options, name, root, Some(plan), Some(&mut abort))?;
        }
        guard
            .vad
            .admission
            .as_ref()
            .context("collection admission missing")?
            .current
            .require_present()?;
        // Anchor before the call; a delayed activation return never creates a
        // fresh interval. MeasurementSession's own earlier deadline also wins.
        let activated = Instant::now();
        phases.stamp("recorder_activation_requested", Duration::ZERO);
        guard
            .measurement
            .as_mut()
            .context("recorder missing")?
            .activate()?;
        loop {
            ensure!(
                activated.elapsed() < phases.ceiling,
                "incomplete collection: total activation-relative deadline"
            );
            let recorder = guard
                .measurement
                .as_ref()
                .context("recorder missing")?
                .check()?;
            ensure!(
                matches!(
                    recorder,
                    MeasurementProgress::Starting | MeasurementProgress::Running { .. }
                ),
                "incomplete collection: recorder deadline or non-collecting state"
            );
            let source = guard
                .vad
                .playback
                .as_ref()
                .context("source missing")?
                .check()?;
            if let Some(source) = &source {
                ensure!(
                    !source.stopped && source.event_timeouts == 0,
                    "source stopped or native event timeout"
                );
            }
            let current = guard.vad.current_health()?;
            validate_health(
                &current,
                &guard
                    .vad
                    .admission
                    .as_ref()
                    .context("collection admission missing")?
                    .current,
                guard.vad.report,
                &mut guard.first_failure_observations,
            )?;
            last_running = Some(current);
            match phases.tick(
                activated.elapsed(),
                matches!(recorder, MeasurementProgress::Running { .. }),
                recorder == MeasurementProgress::DeadlineReached,
                source.as_ref(),
            )? {
                Action::ActivateSource => guard
                    .vad
                    .playback
                    .as_mut()
                    .context("source missing")?
                    .activate()?,
                Action::Complete => break,
                Action::Wait => {}
            }
            thread::sleep(
                Duration::from_millis(10).min(phases.ceiling.saturating_sub(activated.elapsed())),
            );
        }
        Ok(())
    })();
    let phases_complete = collection.is_ok();
    if let Err(error) = collection {
        append_failure(&mut guard.failures, &format!("collection: {error:#}"));
    }
    if guard.failures.is_some() {
        latch_failure_observations(
            &mut guard.first_failure_observations,
            guard.vad.report,
            guard.vad.last_health.as_ref(),
        );
    }
    guard.finish();
    let source_submitted_complete = reconcile_source_outcome(
        guard.vad.playback_outcome.as_ref(),
        frames,
        &mut guard.failures,
    );
    if let Some(outcome) = &guard.measurement_outcome {
        if let Some(failure) = &outcome.failure {
            append_failure(&mut guard.failures, failure);
        }
        if outcome.native_capture.is_none()
            || outcome
                .sink
                .as_ref()
                .is_none_or(|sink| sink.written_frames == 0)
        {
            append_failure(
                &mut guard.failures,
                "independent native/sink evidence unavailable or empty",
            );
        }
    } else {
        append_failure(&mut guard.failures, "measurement outcome unavailable");
    }
    if (phases_complete || guard.vad.report.backend_diagnostics.is_some())
        && let Err(error) = graph_summary_health(guard.vad.report.backend_diagnostics.as_ref())
    {
        append_failure(
            &mut guard.failures,
            &format!("final graph native summary: {error:#}"),
        );
    }
    let complete = phases_complete && guard.failures.is_none();
    // All joins and typed outcome retention precede report serialization/IO.
    let measurement = guard.measurement_outcome.as_ref().map(|outcome| -> Result<_> {
        Ok(serde_json::json!({
            "info": measurement_info(&outcome.info),
            "native_capture": outcome.native_capture.as_ref().map(serde_json::to_value).transpose()?,
            "sink": outcome.sink.as_ref().map(serde_json::to_value).transpose()?,
            "activation_elapsed_seconds": outcome.activation_elapsed.map(|value| value.as_secs_f64()),
            "failure": outcome.failure,
        }))
    }).transpose()?;
    guard.vad.report.collection = Some(serde_json::json!({
        "schema_version": 1, "qualified": false, "status": "unqualified",
        "config": config, "playback_source": provenance, "source": guard.vad.playback_outcome,
        "graph": { "input_kind": "VirtualPlayback", "stable_endpoint_id": options.stable_endpoint_id,
            "output_endpoint_id": options.output_endpoint, "engine_block_frames": options.engine_block_frames,
            "render_queue": super::render_queue(options)?, "virtual_buffer": super::native::virtual_buffer(options)?,
            "telemetry": options.telemetry, "admission": guard.vad.admission,
            "last_running": last_running, "last_observation": guard.vad.last_health,
            "engine": guard.vad.report.engine, "provider": guard.vad.report.vad_provider,
            "native_summary": guard.vad.report.backend_diagnostics,
            "engine_cleanup": guard.vad.report.engine_cleanup, "vad_cleanup": guard.vad.report.vad_cleanup },
        "measurement": measurement, "phases": phases.stamps,
        "source_submitted_complete": source_submitted_complete,
        "source_completion_observed_in_phases": phases.completed.is_some(), "preroll_complete": phases.source_requested.is_some(),
        "postroll_complete": phases_complete, "collection_complete": complete,
        "cleanup_complete": guard.cleanup_failures.is_none(), "cleanup_failure": guard.cleanup_failures,
        "failure": guard.failures, "first_failure_observations": guard.first_failure_observations,
        "evidence_scope": "independent recorder clock and selected endpoint; wiring/reference/calibration unproven; source submission is not played/captured marker detection",
        "control_policy": "Start/grace/roll deadlines use monotonic control time; synchronous provider QUERY, native setup, joins and IO may block; no OS watchdog or physical settling claim",
    }));
    ensure!(
        complete,
        "incomplete unqualified VAD collection: {}",
        guard.failures.as_deref().unwrap_or("unknown")
    );
    Ok(())
}

#[cfg(windows)]
struct CollectionGuard<'a> {
    vad: super::native::VadCleanup<'a>,
    measurement: Option<crate::measurement::session::MeasurementSession>,
    measurement_outcome: Option<crate::measurement::session::MeasurementOutcome>,
    failures: Option<String>,
    cleanup_failures: Option<String>,
    first_failure_observations: Option<serde_json::Value>,
    retired: bool,
}

#[cfg(windows)]
impl CollectionGuard<'_> {
    fn finish(&mut self) {
        use super::soak_cleanup::Cleanup;
        if self.retired {
            return;
        }
        self.retired = true;
        let mut failures = None;
        retire(
            |stage| match stage {
                Retirement::Observe => {
                    if let Some(admitted) = &self.vad.admission {
                        let admitted = admitted.current.clone();
                        let health = self.vad.current_health()?;
                        validate_health(
                            &health,
                            &admitted,
                            self.vad.report,
                            &mut self.first_failure_observations,
                        )
                    } else {
                        Ok(())
                    }
                }
                Retirement::ObserveFinal => {
                    let result = self.vad.observe_final();
                    if result.is_err() {
                        latch_failure_observations(
                            &mut self.first_failure_observations,
                            self.vad.report,
                            self.vad.last_health.as_ref(),
                        );
                    }
                    result
                }
                Retirement::StopSource => {
                    if let Some(source) = &self.vad.playback {
                        source.request_stop();
                    }
                    Ok(())
                }
                Retirement::StopMeasurement => {
                    if let Some(owner) = &mut self.measurement {
                        owner.request_stop()?;
                    }
                    Ok(())
                }
                Retirement::JoinSource => self.vad.stop_source(),
                Retirement::CloseGate => self.vad.close_gate(),
                Retirement::ShutdownHost => self.vad.shutdown_host(),
                Retirement::Invalidate => self.vad.invalidate_admission(),
                Retirement::Retire => self.vad.retire_endpoint(),
                Retirement::FinishMeasurement => {
                    if let Some(mut owner) = self.measurement.take() {
                        self.measurement_outcome = Some(owner.finish()?);
                    }
                    Ok(())
                }
            },
            &mut failures,
        );
        if let Some(failure) = &failures {
            append_failure(&mut self.failures, failure);
        }
        self.cleanup_failures = failures;
    }
}

#[cfg(windows)]
impl Drop for CollectionGuard<'_> {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(windows)]
fn reconcile_source_outcome(
    outcome: Option<&super::vad::PlaybackOutcome>,
    frames: u64,
    failures: &mut Option<String>,
) -> bool {
    let submitted_complete = outcome.is_some_and(|outcome| {
        outcome.source_progress.origin_submission_frame.is_some()
            && outcome.source_progress.schedule_submitted_complete
            && outcome.source_progress.acknowledged_source_frames == frames
    });
    if let Some(outcome) = outcome {
        if let Some(failure) = &outcome.failure {
            append_failure(failures, failure);
        }
        if outcome.event_timeouts != 0 {
            append_failure(
                failures,
                &format!(
                    "joined source native event timeouts: {}",
                    outcome.event_timeouts
                ),
            );
        }
        if !(outcome.started && outcome.stopped && submitted_complete) {
            append_failure(
                failures,
                "source did not retain full acknowledged schedule and clean stop",
            );
        }
    } else {
        append_failure(failures, "source outcome unavailable");
    }
    submitted_complete
}

#[cfg(any(windows, test))]
fn measurement_info(info: &crate::measurement::session::MeasurementInfo) -> serde_json::Value {
    serde_json::json!({"input_endpoint_id": info.input_endpoint_id, "sample_rate": info.sample_rate,
        "channels": info.channels, "ring_frames": info.ring_frames, "max_frames": info.max_frames})
}

#[cfg(any(windows, test))]
fn graph_summary_health(summary: Option<&serde_json::Value>) -> Result<()> {
    let summary = summary.context("graph native summary unknown")?;
    ensure!(
        summary
            .get("failure")
            .is_some_and(serde_json::Value::is_null),
        "graph native failure or unknown outcome"
    );
    let render = summary
        .get("render")
        .filter(|render| !render.is_null())
        .context("graph render report unavailable")?;
    ensure!(
        render
            .get("failure")
            .is_some_and(serde_json::Value::is_null)
            && render
                .get("stream_started")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            && render
                .get("stop_succeeded")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
        "graph native render start/stop/failure"
    );
    for key in ["bridge_shortfall_frames", "timeout_wakes"] {
        ensure!(
            render
                .pointer(&format!("/stats/{key}"))
                .and_then(serde_json::Value::as_u64)
                == Some(0),
            "graph native {key} nonzero or unknown"
        );
    }
    Ok(())
}

#[cfg(any(windows, test))]
impl Phases {
    fn new(config: Config, frames: u64, seconds: u64) -> Self {
        Self {
            config,
            ceiling: Duration::from_secs(seconds),
            source_duration: Duration::from_millis((frames * 1000).div_ceil(48_000)),
            source_frames: frames,
            source_started: false,
            recorder_ready: None,
            source_requested: None,
            completed: None,
            stamps: Vec::with_capacity(8),
        }
    }
    fn stamp(&mut self, phase: &'static str, now: Duration) {
        if self.stamps.len() < 32 {
            self.stamps.push(PhaseStamp {
                phase,
                elapsed_ms: now.as_millis(),
            });
        }
    }
    fn tick(
        &mut self,
        now: Duration,
        recorder_running: bool,
        recorder_deadline: bool,
        source: Option<&PlaybackProgress>,
    ) -> Result<Action> {
        ensure!(
            !recorder_deadline && now < self.ceiling,
            "incomplete collection: recorder deadline"
        );
        if self.recorder_ready.is_none() {
            ensure!(now < Duration::from_secs(5), "recorder Start deadline");
            if recorder_running {
                self.recorder_ready = Some(now);
                self.stamp("recorder_running", now);
            }
        }
        let Some(ready) = self.recorder_ready else {
            return Ok(Action::Wait);
        };
        ensure!(recorder_running, "recorder lost Running state");
        if self.source_requested.is_none()
            && now.saturating_sub(ready) >= Duration::from_millis(self.config.preroll_ms)
        {
            self.source_requested = Some(now);
            self.stamp("source_activation_requested", now);
            return Ok(Action::ActivateSource);
        }
        if let Some(requested) = self.source_requested {
            let age = now.saturating_sub(requested);
            if !self.source_started {
                ensure!(age < Duration::from_secs(5), "source Start deadline");
                if source.is_some_and(|source| source.started) {
                    self.source_started = true;
                    self.stamp("source_started_observed", now);
                }
            }
            if self.completed.is_none() {
                ensure!(
                    age < Duration::from_secs(7) + self.source_duration,
                    "source completion deadline"
                );
                if source.is_some_and(|s| {
                    s.started
                        && s.source.origin_submission_frame.is_some()
                        && s.source.schedule_submitted_complete
                }) {
                    ensure!(
                        source.is_some_and(
                            |s| s.source.acknowledged_source_frames == self.source_frames
                        ),
                        "source completion acknowledgement count mismatch"
                    );
                    self.completed = Some(now);
                    self.stamp("source_submitted_complete", now);
                }
            }
        }
        if self.completed.is_some_and(|completed| {
            now.saturating_sub(completed) >= Duration::from_millis(self.config.postroll_ms)
        }) {
            self.stamp("postroll_complete", now);
            return Ok(Action::Complete);
        }
        Ok(Action::Wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn collection_joined_source_late_timeout_fails_after_healthy_postroll() {
        let mut phases = Phases::new(config(), 48_000, 14);
        let source = PlaybackProgress {
            started: true,
            source: super::super::source::SourceProgress {
                native_preroll_frames: 480,
                native_submitted_frames: 48_480,
                origin_submission_frame: Some(480),
                acknowledged_source_frames: 48_000,
                schedule_submitted_complete: true,
            },
            ..Default::default()
        };
        phases.tick(Duration::ZERO, true, false, None).unwrap();
        assert_eq!(
            phases
                .tick(Duration::from_millis(100), true, false, None)
                .unwrap(),
            Action::ActivateSource
        );
        phases
            .tick(Duration::from_millis(1100), true, false, Some(&source))
            .unwrap();
        assert_eq!(
            phases
                .tick(Duration::from_millis(1300), true, false, Some(&source))
                .unwrap(),
            Action::Complete
        );
        assert_eq!(source.event_timeouts, 0);
        let mut outcome = super::super::vad::PlaybackOutcome {
            info: None,
            submitted_frames: 48_480,
            playback_source: super::super::source::SourceProvenance::new(None),
            source_progress: source.source,
            event_timeouts: 0,
            elapsed_seconds: 1.3,
            started: true,
            stopped: true,
            worker_joined: true,
            native_stop_attempted: Some(true),
            native_stop_succeeded: Some(true),
            failure: None,
        };
        let mut failures = None;
        assert!(reconcile_source_outcome(
            Some(&outcome),
            48_000,
            &mut failures
        ));
        assert!(
            failures.is_none(),
            "healthy joined source must remain valid"
        );

        // The timeout appears after the last healthy poll and before native Stop.
        outcome.event_timeouts = 1;
        assert!(reconcile_source_outcome(
            Some(&outcome),
            48_000,
            &mut failures
        ));
        assert!(
            failures
                .as_deref()
                .is_some_and(|text| text.contains("timeout")),
            "late joined source timeout must fail otherwise-complete collection"
        );
        assert_eq!(outcome.event_timeouts, 1);

        outcome.failure = Some("primary PCM error; native Stop error".into());
        let mut failures = Some("earlier collection error".into());
        reconcile_source_outcome(Some(&outcome), 48_000, &mut failures);
        let text = failures.unwrap();
        for retained in [
            "earlier collection error",
            "primary PCM error",
            "native Stop error",
            "timeout",
        ] {
            assert!(text.contains(retained), "lost failure: {retained}: {text}");
        }
        assert_eq!(outcome.event_timeouts, 1);
        assert_eq!(
            outcome.failure.as_deref(),
            Some("primary PCM error; native Stop error")
        );
    }

    #[test]
    fn collection_measurement_metadata_retains_actual_mono_44100_format() {
        let value = measurement_info(&crate::measurement::session::MeasurementInfo {
            input_endpoint_id: "independent-mono".into(),
            sample_rate: 44_100,
            channels: 1,
            ring_frames: 1024,
            max_frames: 123,
        });
        assert_eq!(value["sample_rate"], 44_100);
        assert_eq!(value["channels"], 1);
        assert_eq!(value["max_frames"], 123);
        assert_eq!(value["input_endpoint_id"], "independent-mono");
    }

    fn config() -> Config {
        Config {
            schema_version: 1,
            input_endpoint_id: "independent".into(),
            ring_frames: 1024,
            max_pcm_bytes: 1024 * 1024,
            max_metadata_bytes: 100_000,
            max_packets: 1000,
            preroll_ms: 100,
            postroll_ms: 200,
        }
    }

    #[test]
    fn collection_config_checks_budget_caps_and_schema() {
        config().validate(48_000, 14).unwrap();
        for variant in 0..12 {
            let mut value = config();
            match variant {
                0 => value.schema_version = 2,
                1 => value.input_endpoint_id.clear(),
                2 => value.input_endpoint_id.push('\0'),
                3 => value.ring_frames = usize::MAX,
                4 => value.ring_frames = 0,
                5 => value.max_pcm_bytes = 3 * 1024 * 1024 * 1024 + 1,
                6 => value.max_pcm_bytes = 0,
                7 => value.max_metadata_bytes = 256 * 1024 * 1024 + 1,
                8 => value.max_metadata_bytes = 0,
                9 => value.max_packets = 1_000_001,
                10 => value.preroll_ms = 5001,
                _ => value.postroll_ms = 5001,
            }
            assert!(
                value.validate(48_000, 14).is_err(),
                "invalid config {variant}"
            );
        }
        assert!(config().validate(48_000, 13).is_err());
        assert!(config().validate(u64::MAX, 600).is_err());
        assert!(config().validate(48_000, 601).is_err());
        let mut json = serde_json::to_value(config()).unwrap();
        json["unknown"] = true.into();
        assert!(serde_json::from_value::<Config>(json).is_err());
    }

    #[test]
    fn collection_requires_recorder_preroll_source_ack_and_postroll() {
        let mut phases = Phases::new(config(), 48_000, 14);
        let mut source = PlaybackProgress::default();
        assert_eq!(
            phases
                .tick(Duration::ZERO, false, false, Some(&source))
                .unwrap(),
            Action::Wait
        );
        assert_eq!(
            phases
                .tick(Duration::from_millis(10), true, false, Some(&source))
                .unwrap(),
            Action::Wait
        );
        assert_eq!(
            phases
                .tick(Duration::from_millis(110), true, false, Some(&source))
                .unwrap(),
            Action::ActivateSource
        );
        assert_eq!(
            phases
                .tick(Duration::from_millis(120), true, false, Some(&source))
                .unwrap(),
            Action::Wait
        );
        source.started = true;
        source.source.origin_submission_frame = Some(480);
        source.source.acknowledged_source_frames = 48_000;
        source.source.schedule_submitted_complete = true;
        assert_eq!(
            phases
                .tick(Duration::from_millis(1110), true, false, Some(&source))
                .unwrap(),
            Action::Wait
        );
        assert_eq!(
            phases
                .tick(Duration::from_millis(1309), true, false, Some(&source))
                .unwrap(),
            Action::Wait
        );
        assert_eq!(
            phases
                .tick(Duration::from_millis(1310), true, false, Some(&source))
                .unwrap(),
            Action::Complete
        );
    }

    #[test]
    fn collection_phase_deadlines_and_unknown_snapshots_never_extend_budget() {
        for now in [Duration::from_secs(5), Duration::from_secs(14)] {
            let mut phases = Phases::new(config(), 48_000, 14);
            assert!(phases.tick(now, false, false, None).is_err());
        }
        let mut phases = Phases::new(config(), 48_000, 14);
        assert!(phases.tick(Duration::ZERO, true, true, None).is_err());
        let mut phases = Phases::new(config(), 48_000, 14);
        phases.tick(Duration::ZERO, true, false, None).unwrap();
        assert_eq!(
            phases
                .tick(Duration::from_millis(100), true, false, None)
                .unwrap(),
            Action::ActivateSource
        );
        assert!(
            phases
                .tick(Duration::from_millis(5100), true, false, None)
                .is_err()
        );
        let mut phases = Phases::new(config(), 48_000, 14);
        phases.tick(Duration::ZERO, true, false, None).unwrap();
        phases
            .tick(Duration::from_millis(100), true, false, None)
            .unwrap();
        let source = PlaybackProgress {
            started: true,
            ..Default::default()
        };
        assert!(
            phases
                .tick(Duration::from_millis(8100), true, false, Some(&source))
                .is_err()
        );
    }

    #[test]
    fn collection_delayed_start_and_short_ack_cannot_start_postroll() {
        let mut phases = Phases::new(config(), 48_000, 14);
        phases.tick(Duration::ZERO, true, false, None).unwrap();
        phases
            .tick(Duration::from_millis(100), true, false, None)
            .unwrap();
        let source = PlaybackProgress {
            started: true,
            ..Default::default()
        };
        assert!(
            phases
                .tick(Duration::from_millis(5100), true, false, Some(&source))
                .is_err()
        );
        let mut phases = Phases::new(config(), 48_000, 14);
        phases.tick(Duration::ZERO, true, false, None).unwrap();
        phases
            .tick(Duration::from_millis(100), true, false, None)
            .unwrap();
        let mut source = source;
        source.source.origin_submission_frame = Some(480);
        source.source.schedule_submitted_complete = true;
        source.source.acknowledged_source_frames = 47_999;
        assert!(
            phases
                .tick(Duration::from_millis(1000), true, false, Some(&source))
                .is_err()
        );
        assert!(phases.completed.is_none());
    }

    #[test]
    fn collection_postroll_keeps_start_fact_but_earlier_recorder_deadline_wins() {
        let mut phases = Phases::new(config(), 48_000, 14);
        phases.tick(Duration::ZERO, true, false, None).unwrap();
        phases
            .tick(Duration::from_millis(100), true, false, None)
            .unwrap();
        let mut source = PlaybackProgress {
            started: true,
            ..Default::default()
        };
        phases
            .tick(Duration::from_secs(1), true, false, Some(&source))
            .unwrap();
        source.source.origin_submission_frame = Some(480);
        source.source.schedule_submitted_complete = true;
        source.source.acknowledged_source_frames = 48_000;
        phases
            .tick(Duration::from_secs(6), true, false, Some(&source))
            .unwrap();
        assert_eq!(
            phases
                .tick(Duration::from_millis(6100), true, false, None)
                .unwrap(),
            Action::Wait
        );
        assert!(
            phases
                .tick(Duration::from_millis(6200), true, true, None)
                .is_err()
        );
        assert!(
            phases
                .stamps
                .iter()
                .all(|stamp| stamp.phase != "postroll_complete")
        );
    }

    #[test]
    fn collection_config_read_is_bounded_exact_and_requires_regular_file() {
        let path = std::env::temp_dir().join(format!(
            "moiren-collection-config-{}.json",
            std::process::id()
        ));
        let bytes = serde_json::to_vec(&config()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let (admitted, _) = read_config(&path, bytes.len() as u64, 48_000, 14).unwrap();
        assert_eq!(admitted, bytes);
        assert!(read_config(&path, bytes.len() as u64 - 1, 48_000, 14).is_err());
        std::fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();
        assert!(read_config(&path, u64::MAX, 48_000, 14).is_err());
        std::fs::remove_file(path).unwrap();
        assert!(read_config(&std::env::temp_dir(), 64 * 1024, 48_000, 14).is_err());
    }

    #[test]
    fn collection_cleanup_unwind_broadcast_precedes_a_blocked_sink() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        // This private fake executes the same retirement sequence as the real
        // owning guard, including Drop during a partial-setup unwind.
        struct FakeGuard(
            Arc<Mutex<Vec<Retirement>>>,
            Option<mpsc::Receiver<()>>,
            mpsc::Sender<()>,
        );
        impl Drop for FakeGuard {
            fn drop(&mut self) {
                retire(
                    |stage| {
                        self.0.lock().unwrap().push(stage);
                        if stage == Retirement::Observe {
                            panic!("injected final QUERY panic");
                        }
                        if stage == Retirement::FinishMeasurement {
                            self.2.send(()).unwrap();
                            self.1.take().unwrap().recv().unwrap();
                        }
                        Ok(())
                    },
                    &mut None,
                );
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let (release, wait) = mpsc::channel();
        let (at_sink_tx, at_sink_rx) = mpsc::channel();
        let observed = events.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = finished.clone();
        let worker = std::thread::spawn(move || {
            let _ = std::panic::catch_unwind(|| {
                let _guard = FakeGuard(observed, Some(wait), at_sink_tx);
                panic!("injected partial setup unwind");
            });
            worker_finished.store(true, Ordering::Release);
        });
        let reached_sink = at_sink_rx.recv_timeout(Duration::from_secs(2));
        let snapshot = events.lock().unwrap().clone();
        let still_joining = !finished.load(Ordering::Acquire);
        // Always release/join the private fake before asserting, including on
        // regression failure; the test itself never leaves a detached worker.
        let _ = release.send(());
        worker.join().unwrap();
        assert!(reached_sink.is_ok() && still_joining);
        assert_eq!(
            &snapshot[..5],
            &[
                Retirement::Observe,
                Retirement::ObserveFinal,
                Retirement::StopSource,
                Retirement::StopMeasurement,
                Retirement::JoinSource
            ]
        );
        assert!(
            snapshot
                .iter()
                .position(|stage| *stage == Retirement::Retire)
                .unwrap()
                < snapshot
                    .iter()
                    .position(|stage| *stage == Retirement::FinishMeasurement)
                    .unwrap()
        );
    }

    #[test]
    fn collection_final_graph_summary_rejects_unknown_and_trailing_native_failure() {
        let valid = serde_json::json!({"failure": null, "render": {
            "failure": null, "stream_started": true, "stop_succeeded": true,
            "stats": {"bridge_shortfall_frames": 0, "timeout_wakes": 0}
        }});
        graph_summary_health(Some(&valid)).unwrap();
        assert!(graph_summary_health(None).is_err());
        for pointer in [
            "/failure",
            "/render/failure",
            "/render/stats/bridge_shortfall_frames",
            "/render/stats/timeout_wakes",
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = serde_json::json!(1);
            assert!(graph_summary_health(Some(&invalid)).is_err());
        }
        let mut missing = valid;
        missing["render"]["stats"]
            .as_object_mut()
            .unwrap()
            .remove("timeout_wakes");
        assert!(graph_summary_health(Some(&missing)).is_err());
    }

    fn healthy() -> Health {
        Health {
            stamp: Some([1, 2, 3, 4, 5]),
            shape: Some(("Stereo".into(), "Some(Float32)".into(), 480)),
            provider_ready: true,
            revision: Some(1),
            generations: [7, 8],
            engine_running: true,
            native_generation: Some(7),
            native_running: true,
            quality_known: true,
            ..Default::default()
        }
    }

    #[test]
    fn collection_raw_failure_retirement_preserves_first_query_and_distinct_final() {
        let admitted = healthy();
        let mut first_failure = None;
        let mut report = super::super::NativeReport {
            vad_provider: Some(serde_json::json!({
                "diagnostic_provenance": "FreshQuery", "error": null,
                "snapshot": {"stream_generation": 4, "gate_open": true, "dropped_packets": 0}
            })),
            engine: Some(serde_json::json!({
                "active_revision": 1, "owner_generation": 7,
                "observations": {"owner_generation": 7, "bridges": []},
                "startup_trace": {
                    "sample_limit": 256,
                    "samples": vec![serde_json::json!({"provider_error": null}); 256]
                }
            })),
            ..Default::default()
        };
        let mut phases = Phases::new(config(), 48_000, 14);
        let source = PlaybackProgress {
            started: true,
            source: super::super::source::SourceProgress {
                origin_submission_frame: Some(480),
                acknowledged_source_frames: 48_000,
                schedule_submitted_complete: true,
                ..Default::default()
            },
            ..Default::default()
        };
        for (ms, expected) in [
            (0, Action::Wait),
            (100, Action::ActivateSource),
            (1100, Action::Wait),
            (1300, Action::Complete),
        ] {
            validate_health(&admitted, &admitted, &report, &mut first_failure).unwrap();
            assert_eq!(
                phases
                    .tick(Duration::from_millis(ms), true, false, Some(&source))
                    .unwrap(),
                expected
            );
        }
        assert!(first_failure.is_none());
        let mut before_next_query = None;
        let mut cleanup_failures = None;
        let mut events = Vec::new();
        retire(
            |stage| {
                events.push(stage);
                match stage {
                    Retirement::Observe => {
                        report.vad_provider = Some(serde_json::json!({
                            "diagnostic_provenance": "CachedAfterQueryFailure",
                            "error": "first retirement QUERY failure",
                            "snapshot": {"stream_generation": 4, "gate_open": true, "dropped_packets": 17}
                        }));
                        report.engine.as_mut().unwrap()["observations"] = serde_json::json!({
                            "owner_generation": 7,
                            "bridges": [{"counters": {"underrun_frames": 23}}]
                        });
                        let mut health = healthy();
                        health.provider_error = Some("first retirement QUERY failure".into());
                        health.quality_faults = 23;
                        validate_health(&health, &admitted, &report, &mut first_failure)
                    }
                    Retirement::ObserveFinal => {
                        before_next_query = first_failure.clone();
                        report.vad_provider = Some(serde_json::json!({
                            "diagnostic_provenance": "FreshQuery", "error": null,
                            "snapshot": {"stream_generation": 9, "gate_open": false, "dropped_packets": 31}
                        }));
                        report.engine.as_mut().unwrap()["observations"] = serde_json::json!({
                            "owner_generation": 99, "bridges": []
                        });
                        Ok(())
                    }
                    _ => Ok(()),
                }
            },
            &mut cleanup_failures,
        );
        let first = first_failure.expect("first retirement failure must retain raw evidence");
        assert_eq!(before_next_query.as_ref(), Some(&first));
        assert_eq!(
            first["provider"]["diagnostic_provenance"],
            "CachedAfterQueryFailure"
        );
        assert_eq!(first["provider"]["snapshot"]["dropped_packets"], 17);
        assert_eq!(
            first["engine"]["observations"]["bridges"][0]["counters"]["underrun_frames"],
            23
        );
        assert_eq!(
            first["health"]["provider_error"],
            "first retirement QUERY failure"
        );
        assert_eq!(
            first["engine"]["startup_trace"]["samples"]
                .as_array()
                .unwrap()
                .len(),
            256
        );
        let final_provider = report.vad_provider.as_ref().unwrap();
        assert_eq!(final_provider["diagnostic_provenance"], "FreshQuery");
        assert_eq!(final_provider["snapshot"]["dropped_packets"], 31);
        assert_eq!(final_provider["snapshot"]["gate_open"], false);
        assert_eq!(
            report.engine.as_ref().unwrap()["observations"]["owner_generation"],
            99
        );
        assert!(
            cleanup_failures
                .unwrap()
                .contains("first retirement QUERY failure")
        );
        assert_eq!(
            &events[..5],
            &[
                Retirement::Observe,
                Retirement::ObserveFinal,
                Retirement::StopSource,
                Retirement::StopMeasurement,
                Retirement::JoinSource,
            ]
        );
    }

    #[test]
    fn collection_raw_failure_validation_latches_active_failure_only_once() {
        let admitted = healthy();
        let mut current = healthy();
        current.provider_error = Some("first active failure".into());
        let mut report = super::super::NativeReport {
            vad_provider: Some(
                serde_json::json!({"error": "first active failure", "telemetry": {"dropped_packets": 17}}),
            ),
            engine: Some(
                serde_json::json!({"owner_generation": 7, "observations": {"raw_counter": 23}}),
            ),
            ..Default::default()
        };
        let mut first_failure = None;
        assert!(validate_health(&current, &admitted, &report, &mut first_failure).is_err());
        report.vad_provider = Some(serde_json::json!({"error": "later failure"}));
        report.engine = Some(serde_json::json!({"owner_generation": 99}));
        current.provider_error = Some("later failure".into());
        assert!(validate_health(&current, &admitted, &report, &mut first_failure).is_err());
        let first = first_failure.expect("validation itself must latch the first raw failure");
        assert_eq!(first["provider"]["error"], "first active failure");
        assert_eq!(first["provider"]["telemetry"]["dropped_packets"], 17);
        assert_eq!(first["engine"]["observations"]["raw_counter"], 23);
        assert_eq!(first["health"]["provider_error"], "first active failure");
    }

    #[test]
    fn collection_health_is_required_without_journal_and_generation_changes_fail_closed() {
        let admitted = healthy();
        admitted.validate(&admitted).unwrap();
        for variant in 0..12 {
            let mut current = healthy();
            match variant {
                0 => current.stamp = None,
                1 => current.stamp.as_mut().unwrap()[3] += 1,
                2 => current.shape.as_mut().unwrap().2 += 1,
                3 => current.provider_ready = false,
                4 => current.provider_error = Some("provider query failed".into()),
                5 => current.revision = Some(2),
                6 => current.generations[1] += 1,
                7 => current.engine_running = false,
                8 => current.engine_fault = true,
                9 => current.native_generation = None,
                10 => current.quality_known = false,
                _ => current.quality_faults = 1,
            }
            assert!(current.validate(&admitted).is_err(), "variant {variant}");
        }
    }

    #[test]
    fn collection_retirement_broadcasts_before_joins_and_accumulates_every_failure() {
        let mut events = Vec::new();
        let mut failures = None;
        retire(
            |stage| {
                events.push(stage);
                anyhow::bail!("injected {stage:?}")
            },
            &mut failures,
        );
        assert_eq!(
            events,
            [
                Retirement::Observe,
                Retirement::ObserveFinal,
                Retirement::StopSource,
                Retirement::StopMeasurement,
                Retirement::JoinSource,
                Retirement::CloseGate,
                Retirement::ShutdownHost,
                Retirement::Invalidate,
                Retirement::Retire,
                Retirement::FinishMeasurement
            ]
        );
        let text = failures.unwrap();
        for stage in events {
            assert!(text.contains(&format!("{stage:?}")));
        }
        let mut failure = Some("x".repeat(16 * 1024 - 1));
        append_failure(&mut failure, "tail");
        assert_eq!(failure.as_ref().unwrap().len(), 16 * 1024 - 1);
        let mut failure = Some("x".repeat(16 * 1024 - 5));
        append_failure(&mut failure, "\u{1f642}tail");
        assert!(failure.unwrap().len() <= 16 * 1024);
    }
}
