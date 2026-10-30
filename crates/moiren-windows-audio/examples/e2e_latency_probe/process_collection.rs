//! Owned Process collection; control-side orchestration, never qualification.

#[cfg(any(windows, test))]
use super::validation_policy::ValidationPolicy;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(any(windows, test))]
use std::time::Duration;
use std::{fs::File, io::Read, path::Path};

#[cfg(windows)]
#[path = "process_collection/graph.rs"]
pub(super) mod graph;

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
        ensure!(
            self.schema_version == 1,
            "unsupported Process collection schema"
        );
        ensure!(
            !self.input_endpoint_id.trim().is_empty() && !self.input_endpoint_id.contains('\0'),
            "explicit independent recorder input required"
        );
        ensure!(
            self.ring_frames > 0
                && self
                    .ring_frames
                    .checked_mul(size_of::<moiren_windows_audio::capture::PcmEvidenceFrame>())
                    .is_some_and(|bytes| bytes <= 8 * 1024 * 1024),
            "Process collection ring exceeds 8MiB"
        );
        ensure!(
            (1..=3 * 1024 * 1024 * 1024).contains(&self.max_pcm_bytes),
            "Process collection PCM cap must be 1..3GiB"
        );
        ensure!(
            (1..=256 * 1024 * 1024).contains(&self.max_metadata_bytes),
            "Process collection metadata cap must be 1..256MiB"
        );
        ensure!(
            (1..=1_000_000).contains(&self.max_packets),
            "Process collection packet cap invalid"
        );
        ensure!(
            self.preroll_ms <= 5000 && self.postroll_ms <= 5000,
            "Process collection rolls exceed 5 seconds"
        );
        let source_ms = frames
            .checked_mul(1000)
            .context("source duration overflow")?
            .div_ceil(48_000);
        let budget = 5000u64
            .checked_add(self.preroll_ms)
            .and_then(|n| n.checked_add(5000))
            .and_then(|n| n.checked_add(source_ms))
            .and_then(|n| n.checked_add(2000))
            .and_then(|n| n.checked_add(self.postroll_ms))
            .and_then(|n| n.checked_add(20))
            .context("collection budget overflow")?;
        ensure!(
            frames > 0 && (1..=600).contains(&seconds) && budget <= seconds * 1000,
            "Process collection phase budget exceeds --seconds"
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
    let file = File::open(path).context("open Process collection config")?;
    let metadata = file.metadata()?;
    ensure!(
        limit > 0 && metadata.is_file() && metadata.len() <= limit,
        "Process collection config byte cap exceeded"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Process collection config grew beyond byte cap"
    );
    let config: Config = serde_json::from_slice(&bytes)?;
    config.validate(frames, seconds)?;
    Ok((bytes, config))
}

pub(super) fn not_collected(
    config: &Config,
    provenance: super::source::SourceProvenance,
    reason: &str,
) -> Value {
    json!({"schema_version":1, "status":"unqualified", "qualified":false,
        "config":config, "playback_source":provenance, "collection_complete":false,
        "cleanup_complete":false, "failure":reason,
        "meaning":"raw owned source and independent capture, not analyzed or calibrated"})
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Default)]
struct Health {
    graph_started: bool,
    recorder_running: bool,
    recorder_deadline: bool,
    source_running: bool,
    source_submitted: bool,
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
    ceiling: Duration,
    start_requested: Duration,
    preroll: Duration,
    postroll: Duration,
    source_duration: Duration,
    ready: Option<Duration>,
    activated: Option<Duration>,
    running: Option<Duration>,
    submitted: Option<Duration>,
}

#[cfg(any(windows, test))]
impl Phases {
    #[cfg(test)]
    fn tick(&mut self, now: Duration, health: Health) -> Result<Action> {
        self.tick_with_policy(
            now,
            health,
            ValidationPolicy::default(),
            &mut QualityDiagnostics::default(),
        )
    }

    fn tick_with_policy(
        &mut self,
        now: Duration,
        health: Health,
        policy: ValidationPolicy,
        quality: &mut QualityDiagnostics,
    ) -> Result<Action> {
        policy.validate()?;
        let grace = policy.progress_grace();
        ensure!(
            now < self.ceiling,
            "incomplete Process collection: initial total deadline"
        );
        ensure!(
            !health.recorder_deadline,
            "incomplete Process collection: earlier recorder deadline"
        );
        let Some(activated) = self.activated else {
            let remaining = self.preroll.saturating_sub(
                self.ready
                    .map_or(Duration::ZERO, |ready| now.saturating_sub(ready)),
            ) + Duration::from_secs(7)
                + self.source_duration
                + self.postroll
                + grace;
            ensure!(
                now.saturating_add(remaining) < self.ceiling,
                "incomplete Process collection: remaining source phase budget"
            );
            if self.ready.is_none() {
                quality.progress_deadline(
                    now.saturating_sub(self.start_requested),
                    Duration::from_secs(5),
                    policy,
                    "Process graph/recorder Start deadline",
                )?;
            }
            if !health.graph_started || !health.recorder_running {
                return Ok(Action::Wait);
            }
            let ready = *self.ready.get_or_insert(now);
            if now.saturating_sub(ready) >= self.preroll {
                self.activated = Some(now);
                return Ok(Action::ActivateSource);
            }
            return Ok(Action::Wait);
        };
        ensure!(
            health.graph_started && health.recorder_running,
            "Process graph/recorder lost Running"
        );
        let elapsed = now.saturating_sub(activated);
        // Receipt presence is not proof it was observed before the deadline.
        // Enforce each first-observation bound before accepting a late receipt.
        if self.running.is_none() {
            quality.progress_deadline(
                elapsed,
                Duration::from_secs(5),
                policy,
                "source Start deadline",
            )?;
            if health.source_running {
                self.running = Some(now);
            }
        }
        if !health.source_running {
            ensure!(!health.source_submitted, "source submitted without Running");
            ensure!(
                self.running.is_none(),
                "source lost Running acknowledgement"
            );
        }
        if self.submitted.is_none() {
            quality.progress_deadline(
                elapsed,
                Duration::from_secs(7) + self.source_duration,
                policy,
                "source submission deadline",
            )?;
        }
        if health.source_submitted {
            let submitted = *self.submitted.get_or_insert(now);
            if now.saturating_sub(submitted) >= self.postroll {
                return Ok(Action::Complete);
            }
        }
        Ok(Action::Wait)
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retirement {
    StopGraph,
    StopRecorder,
    StopSource,
    StopLoad,
    JoinGraph,
    FinishSource,
    FinishLoad,
    FinishRecorder,
}

#[cfg(any(windows, test))]
fn retire(mut step: impl FnMut(Retirement) -> Result<()>, failures: &mut Option<String>) {
    for stage in [
        Retirement::StopGraph,
        Retirement::StopRecorder,
        Retirement::StopSource,
        Retirement::StopLoad,
        Retirement::JoinGraph,
        Retirement::FinishSource,
        Retirement::FinishLoad,
        Retirement::FinishRecorder,
    ] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(stage)));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => append_failure(failures, &format!("{stage:?}: {error:#}")),
            Err(_) => append_failure(failures, &format!("{stage:?}: cleanup panic")),
        }
    }
}

#[cfg(any(windows, test))]
fn append_failure(failure: &mut Option<String>, message: &str) {
    const CAP: usize = 16 * 1024;
    let text = failure.get_or_insert_with(String::new);
    let separator = if text.is_empty() { "" } else { "; " };
    let available = CAP.saturating_sub(text.len());
    if available <= separator.len() {
        return;
    }
    let mut end = message.len().min(available - separator.len());
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    if end > 0 {
        text.push_str(separator);
        text.push_str(&message[..end]);
    }
}

/// Diagnostic faults never replace operational failures or raw final reports.
/// Text is bounded independently of run duration and cumulative native counters.
#[cfg(any(windows, test))]
#[derive(Default, Serialize)]
pub(super) struct QualityDiagnostics {
    pub(super) failure: Option<String>,
}

#[cfg(any(windows, test))]
impl QualityDiagnostics {
    pub(super) fn fault(&mut self, policy: ValidationPolicy, message: &str) -> Result<()> {
        append_failure(&mut self.failure, message);
        ensure!(policy.is_correctness(), "{message}");
        Ok(())
    }

    fn progress_deadline(
        &mut self,
        elapsed: Duration,
        base: Duration,
        policy: ValidationPolicy,
        message: &str,
    ) -> Result<()> {
        ensure!(
            elapsed < base.saturating_add(policy.progress_grace()),
            "{message}"
        );
        if elapsed >= base {
            self.fault(policy, message)?;
        }
        Ok(())
    }
}

#[cfg(any(windows, test))]
#[cfg(test)]
fn validate_graph_observation(value: &Value, running_required: bool) -> Result<bool> {
    validate_graph_with_policy(
        value,
        running_required,
        ValidationPolicy::default(),
        &mut QualityDiagnostics::default(),
    )
}

#[cfg(any(windows, test))]
fn validate_graph_with_policy(
    value: &Value,
    running_required: bool,
    policy: ValidationPolicy,
    quality: &mut QualityDiagnostics,
) -> Result<bool> {
    let boolean = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_bool)
            .with_context(|| format!("missing graph {name}"))
    };
    ensure!(
        !boolean("capture_finished")? && !boolean("render_finished")?,
        "Process graph owner finished before stop"
    );
    let started = boolean("capture_started")? && boolean("render_started")?;
    ensure!(
        !running_required || started,
        "Process graph lost Start acknowledgement"
    );
    validate_bridge_with_policy(value, running_required, policy, quality)?;
    Ok(started)
}

#[cfg(any(windows, test))]
#[cfg(test)]
fn validate_bridge_observation(value: &Value, delivery_required: bool) -> Result<()> {
    validate_bridge_with_policy(
        value,
        delivery_required,
        ValidationPolicy::default(),
        &mut QualityDiagnostics::default(),
    )
}

#[cfg(any(windows, test))]
fn validate_bridge_with_policy(
    value: &Value,
    delivery_required: bool,
    policy: ValidationPolicy,
    quality: &mut QualityDiagnostics,
) -> Result<()> {
    let bridge = value.get("bridge").context("graph bridge missing")?;
    let bridge_keys = [
        "dropped_frames",
        "nonfinite_samples",
        "discontinuities",
        "timestamp_errors",
        "backlog_discarded_frames",
        "age_discarded_frames",
        "underrun_frames",
        "resets",
    ];
    // Validate the entire schema and hard faults before any soft quality return.
    for key in bridge_keys {
        let count = bridge
            .get(key)
            .and_then(Value::as_u64)
            .with_context(|| format!("missing bridge {key}"))?;
        if key == "nonfinite_samples" {
            ensure!(count == 0, "Process bridge fault: {key}");
        }
    }
    let delivery = bridge.get("native_delivery").filter(|v| !v.is_null());
    if delivery_required {
        ensure!(delivery.is_some(), "Process delivery evidence unavailable");
    }
    let delivery_keys = [
        "packet_bound_violations",
        "gap_bound_violations",
        "unavailable_clocks",
        "regressed_clocks",
    ];
    if let Some(delivery) = delivery {
        for key in delivery_keys {
            delivery
                .get(key)
                .and_then(Value::as_u64)
                .with_context(|| format!("missing delivery {key}"))?;
        }
    }
    let timeline = value
        .get("timeline")
        .filter(|v| !v.is_null())
        .context("graph timeline unavailable")?;
    ensure!(
        timeline.get("epoch").and_then(Value::as_u64) == Some(1)
            && timeline.get("revision").and_then(Value::as_u64) == Some(1),
        "Process graph epoch/revision changed or unavailable"
    );
    for key in bridge_keys {
        if bridge[key].as_u64() != Some(0) {
            quality.fault(policy, &format!("Process bridge fault: {key}"))?;
        }
    }
    if let Some(delivery) = delivery {
        for key in delivery_keys {
            if delivery[key].as_u64() != Some(0) {
                quality.fault(policy, &format!("Process delivery fault: {key}"))?;
            }
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
#[cfg(test)]
fn validate_final_graph(capture: &Value, render: &Value, identity: &Value) -> Result<()> {
    validate_final_graph_with_policy(
        capture,
        render,
        identity,
        ValidationPolicy::default(),
        &mut QualityDiagnostics::default(),
    )
}

#[cfg(any(windows, test))]
fn validate_final_graph_with_policy(
    capture: &Value,
    render: &Value,
    identity: &Value,
    policy: ValidationPolicy,
    quality: &mut QualityDiagnostics,
) -> Result<()> {
    for (name, report) in [("capture", capture), ("render", render)] {
        ensure!(
            report.get("failure").is_some_and(Value::is_null),
            "final {name} failure: {}",
            report.get("failure").unwrap_or(&Value::Null)
        );
        ensure!(
            report.get("stream_started").and_then(Value::as_bool) == Some(true)
                && report.get("stop_succeeded").and_then(Value::as_bool) == Some(true),
            "final {name} Start/Stop not acknowledged"
        );
        ensure!(
            report.get("status").and_then(Value::as_str) == Some("stopped"),
            "final {name} did not stop on request"
        );
        ensure!(
            report.get("sample_rate").and_then(Value::as_u64) == Some(48_000)
                && report.get("channels").and_then(Value::as_u64) == Some(2),
            "final {name} format is not 48k stereo"
        );
    }
    for key in ["pid", "creation_time_100ns"] {
        let expected = identity
            .get(key)
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .with_context(|| format!("pinned {key} missing"))?;
        ensure!(
            capture
                .pointer(&format!("/process/{key}"))
                .and_then(Value::as_u64)
                == Some(expected),
            "final capture identity mismatch: {key}"
        );
    }
    // Process positions are not a sample clock. Old metadata cannot distinguish
    // an unchecked zero counter from established device-position continuity.
    ensure!(
        capture
            .pointer("/artifact_metadata/schema_version")
            .and_then(Value::as_u64)
            == Some(2)
            && capture
                .pointer("/artifact_metadata/detect_position_gaps")
                .and_then(Value::as_bool)
                == Some(false),
        "final Process artifact position semantics unavailable or invalid"
    );
    for (report, pointers) in [
        (
            capture,
            &[
                "/timeout_wakes",
                "/delivery/packet_bound_violations",
                "/delivery/gap_bound_violations",
                "/delivery/unavailable_clocks",
                "/delivery/regressed_clocks",
                "/artifact_metadata/dropped_frames",
                "/artifact_metadata/overflow_events",
                "/artifact_metadata/gap_events",
                "/artifact_metadata/discontinuity_events",
                "/artifact_metadata/timestamp_error_packets",
            ][..],
        ),
        (
            render,
            &["/stats/timeout_wakes", "/stats/bridge_shortfall_frames"][..],
        ),
    ] {
        for pointer in pointers {
            let count = report
                .pointer(pointer)
                .and_then(Value::as_u64)
                .with_context(|| format!("final graph missing/fault counter {pointer}"))?;
            if count != 0 {
                quality.fault(
                    policy,
                    &format!("final graph missing/fault counter {pointer}"),
                )?;
            }
        }
    }
    ensure!(
        capture
            .get("packets")
            .and_then(Value::as_u64)
            .is_some_and(|n| n > 0)
            && render
                .pointer("/stats/processed_frames")
                .and_then(Value::as_u64)
                .is_some_and(|n| n > 0),
        "final graph has no captured/rendered frames"
    );
    Ok(())
}

#[cfg(windows)]
pub(super) fn validate_measurement_outcome(
    outcome: &crate::measurement::session::MeasurementOutcome,
    policy: ValidationPolicy,
    quality: &mut QualityDiagnostics,
) -> Result<()> {
    // Strict mode retains the session's complete fail-fast reconciliation.
    if !policy.is_correctness() {
        ensure!(
            outcome.failure.is_none(),
            "recorder final: {}",
            outcome.failure.as_deref().unwrap_or_default()
        );
        return Ok(());
    }
    let native = outcome
        .native_capture
        .as_ref()
        .context("recorder final native report missing")?;
    let sink = outcome
        .sink
        .as_ref()
        .context("recorder final sink report missing")?;
    let summary = native
        .evidence_metadata
        .context("recorder final evidence summary missing")?;
    ensure!(
        outcome.activation_elapsed.is_some()
            && native.stream_started
            && native.stop_succeeded
            && native.failure.is_none()
            && sink.failure.is_none()
            && !sink.cancelled,
        "recorder final activation/Start/Stop/persistence failed"
    );
    ensure!(
        sink.schema_version == 1
            && summary.sample_rate == outcome.info.sample_rate
            && summary.channels == outcome.info.channels
            && sink.sample_rate == outcome.info.sample_rate
            && sink.channels == outcome.info.channels,
        "recorder final schema/format mismatch"
    );
    ensure!(
        sink.nonfinite_samples == 0
            && sink.incomplete_packets == 0
            && summary.reader_abandoned_packets == 0,
        "recorder final nonfinite, malformed or abandoned evidence"
    );
    ensure!(
        !summary.counters_saturated
            && !summary.sequence_exhausted
            && summary.position_overflow_packets == 0
            && !sink.counters_saturated,
        "recorder final evidence counter overflow/exhaustion"
    );
    // Native packet sequences include whole packets rejected by the ring.
    // Account for those drops, but never soften missing persisted PCM/metadata.
    ensure!(
        summary.stored_frames > 0
            && sink.written_frames == summary.stored_frames
            && summary.stored_frames.checked_add(summary.dropped_frames)
                == Some(summary.observed_frames)
            && summary.packets.checked_sub(summary.overflow_packets) == Some(sink.complete_packets)
            && sink.complete_packets > 0
            && sink.missing_packets <= summary.overflow_packets
            && sink.sequence_gaps <= sink.missing_packets
            && (sink.missing_packets == 0) == (sink.sequence_gaps == 0)
            && summary.silent_frames <= summary.observed_frames,
        "recorder final persisted frames/packets missing or inconsistent"
    );
    ensure!(
        (summary.dropped_frames == 0) == (summary.overflow_packets == 0),
        "recorder final loss counters inconsistent"
    );
    let continuity_fault = summary.dropped_frames != 0
        || summary.overflow_packets != 0
        || summary.gap_events != 0
        || summary.discontinuity_events != 0
        || summary.timestamp_error_packets != 0;
    // Reconstruct the session's exact aggregate only after structured checks
    // explain all missing packets as ring loss. A prefix filter could hide IO
    // or lifecycle errors alongside either quality diagnostic.
    let mut expected = None;
    if continuity_fault {
        append_failure(
            &mut expected,
            "measurement evidence has loss or invalid continuity/timestamps",
        );
    }
    if sink.missing_packets != 0 || sink.complete_packets != summary.packets {
        append_failure(
            &mut expected,
            "measurement sink and final native evidence are not complete and consistent",
        );
    }
    if let Some(failure) = &outcome.failure {
        ensure!(
            Some(failure) == expected.as_ref(),
            "recorder final: {failure}"
        );
        quality.fault(policy, failure)?;
    }
    for (name, count) in [
        ("timeout_wakes", native.timeout_wakes),
        ("dropped_frames", summary.dropped_frames),
        ("overflow_packets", summary.overflow_packets),
        ("gap_events", summary.gap_events),
        ("gap_frames", summary.gap_frames),
        ("discontinuity_events", summary.discontinuity_events),
        ("timestamp_error_packets", summary.timestamp_error_packets),
        ("sequence_gaps", sink.sequence_gaps),
        ("missing_packets", sink.missing_packets),
    ] {
        if count != 0 {
            quality.fault(policy, &format!("recorder final quality: {name}={count}"))?;
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
#[cfg(test)]
fn validate_and_latch(value: &Value, running: bool, first: &mut Option<Value>) -> Result<bool> {
    validate_graph_observation(value, running).inspect_err(|_| {
        if first.is_none() {
            *first = Some(value.clone());
        }
    })
}

#[cfg(any(windows, test))]
fn after_recorder<T>(deadline: bool, poll_source: impl FnOnce() -> Result<T>) -> Result<T> {
    ensure!(
        !deadline,
        "incomplete Process collection: earlier recorder deadline"
    );
    poll_source()
}

#[cfg(any(windows, test))]
fn guarded_run<T>(state: &mut T, run: impl FnOnce(&mut T) -> Result<()>) -> Result<()> {
    // Keep the owner OUTSIDE the unwind scope so typed partial outcomes remain
    // available for retirement and artifact projection even after a panic.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(state))) {
        Ok(result) => result,
        Err(_) => anyhow::bail!("Process collection control panic"),
    }
}

#[cfg(windows)]
pub(super) fn collect(
    options: &super::LiveOptions,
    paths: &super::ArtifactPaths,
    config: &Config,
    plan: &super::source::PreparedSource,
    bridge: &moiren_windows_audio::process_loopback::ProcessBridgeOptions,
    queue: moiren_windows_audio::latency::RenderQueuePolicy,
    native: &mut super::NativeReport,
) -> Result<()> {
    use super::process_source::{Config as SourceConfig, ProcessSourceSession};
    use crate::measurement::session::{MeasurementConfig, MeasurementPaths, MeasurementSession};
    use std::time::Instant;

    let initial = Instant::now();
    options.validation.validate()?;
    let provenance = super::source::SourceProvenance::new(Some(plan));
    let frames = provenance
        .schedule
        .as_ref()
        .context("source schedule missing")?
        .total_source_frames;
    let mut phases = Phases {
        ceiling: Duration::from_secs(options.seconds),
        start_requested: Duration::ZERO,
        preroll: Duration::from_millis(config.preroll_ms),
        postroll: Duration::from_millis(config.postroll_ms),
        source_duration: Duration::from_millis(
            frames
                .checked_mul(1000)
                .context("source duration overflow")?
                .div_ceil(48_000),
        ),
        ready: None,
        activated: None,
        running: None,
        submitted: None,
    };
    let mut guard = Guard::with_policy(options.validation);
    let mut complete = false;
    let mut phase_stamps = Vec::with_capacity(8);
    let mut active_frame = None;
    let mut advanced = false;
    let result = guarded_run(&mut guard, |guard| -> Result<()> {
        guard.measurement = Some(
            match MeasurementSession::prepare(
                MeasurementConfig {
                    input_endpoint_id: config.input_endpoint_id.clone(),
                    max_active_duration: phases.ceiling,
                    ring_frames: config.ring_frames,
                    max_pcm_bytes: config.max_pcm_bytes,
                    max_metadata_bytes: config.max_metadata_bytes,
                    max_packets: config.max_packets,
                },
                MeasurementPaths {
                    pcm: paths
                        .measurement_pcm
                        .as_deref()
                        .context("measurement PCM path missing")?
                        .into(),
                    packets: paths
                        .measurement_packets
                        .as_deref()
                        .context("measurement packets path missing")?
                        .into(),
                },
            ) {
                Ok(owner) => owner.with_start_grace(options.validation.progress_grace())?,
                Err(mut failure) => {
                    guard.preparation_capture = failure.native_capture.take();
                    return Err(failure.error);
                }
            },
        );
        ensure!(
            initial.elapsed() < phases.ceiling,
            "initial setup deadline after recorder"
        );
        let mut partial = None;
        let source = ProcessSourceSession::prepare_with_abort(
            SourceConfig {
                source_endpoint: options
                    .source_output
                    .clone()
                    .context("source output missing")?,
                source_schedule: paths
                    .source_schedule
                    .as_deref()
                    .context("retained source schedule missing")?
                    .into(),
                seconds: options.seconds,
                validation: options.validation,
            },
            &Path::new(&paths.directory).join("process-source"),
            || {
                guard.stop_peers();
            },
            &mut partial,
        );
        guard.source_outcome = partial;
        guard.source = Some(source?);
        ensure!(
            initial.elapsed() < phases.ceiling,
            "initial setup deadline after source"
        );
        let identity = guard
            .source
            .as_ref()
            .context("source missing")?
            .identity()
            .clone();
        guard.identity = Some(serde_json::to_value(&identity)?);
        guard.graph = Some(graph::Graph::new()?);
        {
            let Guard {
                graph,
                measurement,
                source,
                failures,
                ..
            } = &mut *guard;
            graph.as_mut().context("graph missing")?.prepare(
                options,
                bridge,
                queue,
                identity,
                || stop_peers(measurement, source, failures),
            )?;
        }
        ensure!(
            initial.elapsed() < phases.ceiling,
            "initial setup deadline after graph"
        );
        phase_stamps.push(json!({"phase":"activation_requested","elapsed_seconds":initial.elapsed().as_secs_f64()}));
        phases.start_requested = initial.elapsed();
        guard.graph.as_mut().context("graph missing")?.activate()?;
        guard
            .measurement
            .as_mut()
            .context("recorder missing")?
            .activate()?;
        loop {
            let now = initial.elapsed();
            ensure!(
                now < phases.ceiling,
                "incomplete Process collection: initial total deadline"
            );
            let observation = guard.graph.as_mut().context("graph missing")?.observation();
            guard.last_observation = Some(observation.clone());
            let graph_started = match guard.observe_graph(&observation, phases.submitted.is_some())
            {
                Ok(started) => started,
                Err(error) => {
                    guard.latch(&observation);
                    return Err(error);
                }
            };
            let measurement = guard
                .measurement
                .as_ref()
                .context("recorder missing")?
                .check()?;
            use crate::measurement::session::MeasurementProgress;
            let recorder_running = matches!(measurement, MeasurementProgress::Running { .. });
            let source = after_recorder(
                matches!(measurement, MeasurementProgress::DeadlineReached),
                || guard.source.as_mut().context("source missing")?.check(),
            )?;
            guard.observe_source(&source)?;
            if let Some(load) = guard.load.as_mut() {
                load.check()?;
            }
            if graph_started
                && recorder_running
                && guard.load.is_none()
                && let Some(id) = options.controlled_load_id.as_deref()
            {
                guard.load = Some(crate::controlled_load::Supervisor::start(
                    id,
                    options.seconds,
                    Path::new(&paths.directory).join("controlled-load.stop"),
                    Path::new(&paths.directory).join("controlled-load.result.json"),
                )?);
            }
            let action = phases.tick_with_policy(
                initial.elapsed(),
                Health {
                    graph_started,
                    recorder_running,
                    recorder_deadline: matches!(measurement, MeasurementProgress::DeadlineReached),
                    source_running: source.running,
                    source_submitted: source.submitted,
                },
                guard.policy,
                &mut guard.quality,
            )?;
            if guard.quality.failure.is_some() {
                guard.latch(&observation);
            }
            if phases.activated.is_some()
                && let Some(frame) = observation
                    .pointer("/timeline/frame")
                    .and_then(Value::as_u64)
            {
                let first = *active_frame.get_or_insert(frame);
                advanced |= frame > first;
            }
            match action {
                Action::ActivateSource => {
                    ensure!(
                        initial.elapsed() < phases.ceiling,
                        "deadline before source activation"
                    );
                    guard
                        .source
                        .as_mut()
                        .context("source missing")?
                        .activate()?;
                    phase_stamps.push(json!({"phase":"source_activation_requested","elapsed_seconds":phases.activated.map(|v|v.as_secs_f64())}));
                }
                Action::Complete => {
                    ensure!(
                        advanced,
                        "Process graph timeline never advanced during source collection"
                    );
                    complete = true;
                    guard.load_window = initial
                        .elapsed()
                        .saturating_sub(phases.activated.context("source activation missing")?);
                    phase_stamps.push(json!({"phase":"postroll_complete","elapsed_seconds":initial.elapsed().as_secs_f64()}));
                    break;
                }
                Action::Wait => {}
            }
            let remaining = phases.ceiling.saturating_sub(initial.elapsed());
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
        Ok(())
    });
    if let Err(error) = result {
        append_failure(&mut guard.failures, &format!("collection: {error:#}"));
        if let Some(value) = guard.last_observation.clone() {
            guard.latch(&value);
        }
    }
    guard.finish();
    if let (Some(graph), Some(identity)) = (&guard.graph, &guard.identity) {
        let validation = (|| -> Result<()> {
            let capture = serde_json::to_value(
                graph
                    .capture_report
                    .as_ref()
                    .context("final graph capture report missing")?,
            )?;
            let render = serde_json::to_value(
                graph
                    .render_report
                    .as_ref()
                    .context("final graph render report missing")?,
            )?;
            let result = validate_final_graph_with_policy(
                &capture,
                &render,
                identity,
                guard.policy,
                &mut guard.quality,
            );
            if guard.quality.failure.is_some() && guard.first_failure.is_none() {
                guard.first_failure = Some(
                    json!({"stage":"final_graph", "capture":capture, "render":render, "identity":identity}),
                );
            }
            result
        })();
        if let Err(error) = validation {
            append_failure(&mut guard.failures, &format!("final graph: {error:#}"));
        }
    }
    let mut persistence = Ok(());
    if let Some(graph) = &guard.graph {
        if let Err(error) = graph.export_trace(paths, native) {
            append_failure(
                &mut guard.failures,
                &format!("persist Process trace: {error:#}"),
            );
        }
        native.capture = graph
            .capture_report
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        native.render = graph
            .render_report
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        native.bridge = guard
            .final_observation
            .as_ref()
            .and_then(|o| o.get("bridge").cloned());
        persistence = graph.persist(paths);
    }
    guard.record_graph_persistence(persistence);
    let outcome = guard.measurement_outcome.as_ref();
    native.controlled_load = guard
        .load_evidence
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;
    let source = guard.source_outcome.as_ref();
    let measurement=outcome.map(|o|json!({"info":{"input_endpoint_id":o.info.input_endpoint_id,"sample_rate":o.info.sample_rate,"channels":o.info.channels,
        "ring_frames":o.info.ring_frames,"max_frames":o.info.max_frames},"native_capture":o.native_capture,"sink":o.sink,"failure":o.failure}));
    native.collection = Some(
        json!({"schema_version":1,"status":"unqualified","qualified":false,
        "collection_complete":guard.collection_complete(complete),"cleanup_complete":guard.cleanup_complete(),
        "validation":guard.policy,"quality_passed":guard.quality_passed(complete),"quality_diagnostics":guard.quality,
        "config":config,"playback_source":provenance,"owned_source":source,"source_identity":guard.identity,
        "source_output_endpoint_id":options.source_output,"graph_output_endpoint_id":options.output_endpoint,
        "measurement":measurement,"measurement_preparation_capture":guard.preparation_capture,
        "phase_stamps":phase_stamps,"first_failure_observations":guard.first_failure,"final_observations":guard.final_observation,
        "graph_abort_evidence":guard.graph.as_ref().map(|g|&g.aborts),"controlled_load":guard.load_evidence,
        "failure":guard.failures,"meaning":"submission and independent PCM collection are not physical calibration or analyzed delay"}),
    );
    ensure!(
        guard.failures.is_none(),
        "{}",
        guard.failures.as_deref().unwrap_or_default()
    );
    Ok(())
}

#[cfg(windows)]
#[derive(Default)]
struct Guard {
    policy: ValidationPolicy,
    quality: QualityDiagnostics,
    measurement: Option<crate::measurement::session::MeasurementSession>,
    measurement_outcome: Option<crate::measurement::session::MeasurementOutcome>,
    preparation_capture: Option<Box<moiren_windows_audio::capture::CaptureReport>>,
    source: Option<super::process_source::ProcessSourceSession>,
    source_outcome: Option<super::process_source::Outcome>,
    graph: Option<graph::Graph>,
    identity: Option<Value>,
    load: Option<crate::controlled_load::Supervisor>,
    load_evidence: Option<crate::controlled_load::Evidence>,
    load_window: Duration,
    failures: Option<String>,
    first_failure: Option<Value>,
    last_observation: Option<Value>,
    final_observation: Option<Value>,
    retired: bool,
}

#[cfg(windows)]
fn stop_peers(
    measurement: &mut Option<crate::measurement::session::MeasurementSession>,
    source: &mut Option<super::process_source::ProcessSourceSession>,
    failures: &mut Option<String>,
) -> Option<String> {
    for peer in [0, 1] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            if peer == 0 {
                if let Some(owner) = measurement.as_mut() {
                    owner.request_stop()?;
                }
            } else if let Some(owner) = source.as_mut() {
                owner.request_stop()?;
            }
            Ok(())
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => append_failure(failures, &format!("abort peer {peer}: {error:#}")),
            Err(_) => append_failure(failures, &format!("abort peer {peer}: panic")),
        }
    }
    failures.clone()
}

#[cfg(windows)]
impl Guard {
    fn with_policy(policy: ValidationPolicy) -> Self {
        let mut guard = Self::default();
        guard.policy = policy;
        guard
    }
    fn observe_graph(&mut self, value: &Value, running: bool) -> Result<bool> {
        let result = validate_graph_with_policy(value, running, self.policy, &mut self.quality);
        if result.is_err() || self.quality.failure.is_some() {
            self.latch(value);
        }
        result
    }
    fn observe_source(&mut self, progress: &super::process_source::Progress) -> Result<()> {
        for receipt in [&progress.running_receipt, &progress.submitted_receipt]
            .into_iter()
            .flatten()
        {
            self.observe_source_receipt(receipt)?;
        }
        Ok(())
    }
    fn observe_source_receipt(&mut self, receipt: &super::process_source::Receipt) -> Result<()> {
        let timeouts = receipt.playback["event_timeouts"]
            .as_u64()
            .context("source event timeout counter missing")?;
        if timeouts != 0 {
            if self.first_failure.is_none() {
                self.first_failure = Some(json!({"stage":"source_quality", "receipt":receipt}));
            }
            self.quality.fault(
                self.policy,
                &format!("source quality: event_timeouts={timeouts}"),
            )?;
        }
        Ok(())
    }
    fn quality_passed(&self, phases_complete: bool) -> bool {
        self.collection_complete(phases_complete) && self.quality.failure.is_none()
    }
    fn record_graph_persistence(&mut self, result: Result<()>) {
        if let Err(error) = result {
            append_failure(
                &mut self.failures,
                &format!("persist graph artifacts: {error:#}"),
            );
        }
    }
    fn collection_complete(&self, phases_complete: bool) -> bool {
        phases_complete && self.failures.is_none()
    }
    fn stop_peers(&mut self) {
        stop_peers(&mut self.measurement, &mut self.source, &mut self.failures);
    }
    fn latch(&mut self, value: &Value) {
        if self.first_failure.is_none() {
            self.first_failure = Some(value.clone());
        }
    }
    fn cleanup_complete(&self) -> bool {
        self.retired
            && self.source_outcome.as_ref().is_some_and(|s| s.joined)
            && self.measurement_outcome.is_some()
            && self
                .graph
                .as_ref()
                .is_some_and(|g| g.capture_report.is_some() && g.render_report.is_some())
    }
    fn finish(&mut self) {
        self.finish_with(Self::retire_step);
    }
    fn finish_with(&mut self, mut step: impl FnMut(&mut Self, Retirement) -> Result<()>) {
        if self.retired {
            return;
        }
        if let Some(graph) = &mut self.graph {
            let value = graph.observation();
            if let Err(error) = self.observe_graph(&value, false) {
                append_failure(&mut self.failures, &format!("pre-stop graph: {error:#}"));
            }
            self.last_observation = Some(value);
        }
        let mut cleanup = None;
        retire(|stage| step(self, stage), &mut cleanup);
        if let Some(error) = cleanup {
            append_failure(&mut self.failures, &error);
        }
        if let Some(graph) = &mut self.graph {
            self.final_observation = Some(graph.observation());
        }
        if let Some(value) = &self.final_observation
            && let Err(error) =
                validate_bridge_with_policy(value, true, self.policy, &mut self.quality)
        {
            // Owner finished/absent is expected after joins. Reconcile the
            // final counters and timeline independently, retaining earlier raw.
            if self.first_failure.is_none() {
                self.first_failure = Some(value.clone());
            }
            append_failure(&mut self.failures, &format!("final bridge: {error:#}"));
        }
        if self.quality.failure.is_some() && self.first_failure.is_none() {
            self.first_failure = self.final_observation.clone();
        }
        self.retired = true;
    }
    fn retire_step(&mut self, stage: Retirement) -> Result<()> {
        match stage {
            Retirement::StopGraph => {
                if let Some(g) = &self.graph {
                    g.request_stop()?;
                }
            }
            Retirement::StopRecorder => {
                if let Some(m) = &mut self.measurement {
                    m.request_stop()?;
                }
            }
            Retirement::StopSource => {
                if let Some(s) = &mut self.source {
                    s.request_stop()?;
                }
            }
            Retirement::StopLoad => {
                if let Some(l) = &mut self.load {
                    l.request_stop()?;
                }
            }
            Retirement::JoinGraph => {
                if let Some(g) = &mut self.graph {
                    g.finish()?;
                }
            }
            Retirement::FinishSource => {
                if let Some(s) = &mut self.source {
                    self.source_outcome = Some(s.finish());
                }
                if let Some(s) = &self.source_outcome {
                    ensure!(
                        s.valid,
                        "source final: {}",
                        s.failure.as_deref().unwrap_or("invalid final outcome")
                    );
                }
                if let Some(receipt) = self
                    .source_outcome
                    .as_ref()
                    .and_then(|source| source.final_receipt.clone())
                {
                    self.observe_source_receipt(&receipt)?;
                }
            }
            Retirement::FinishLoad => {
                if let Some(l) = self.load.take() {
                    self.load_evidence = Some(l.finish(self.load_window)?);
                }
                if let Some(l) = &self.load_evidence {
                    ensure!(
                        l.valid,
                        "load final: {}",
                        l.failure.as_deref().unwrap_or("invalid load evidence")
                    );
                }
            }
            Retirement::FinishRecorder => {
                if let Some(m) = &mut self.measurement {
                    self.measurement_outcome = Some(m.finish()?);
                }
                if let Some(m) = &self.measurement_outcome {
                    validate_measurement_outcome(m, self.policy, &mut self.quality)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for Guard {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
#[path = "process_collection/tests.rs"]
mod tests;
