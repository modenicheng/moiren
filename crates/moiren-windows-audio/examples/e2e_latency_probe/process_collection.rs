//! Owned Process collection; control-side orchestration, never qualification.

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
    fn tick(&mut self, now: Duration, health: Health) -> Result<Action> {
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
                + self.postroll;
            ensure!(
                now.saturating_add(remaining) < self.ceiling,
                "incomplete Process collection: remaining source phase budget"
            );
            if self.ready.is_none() {
                ensure!(
                    now.saturating_sub(self.start_requested) < Duration::from_secs(5),
                    "Process graph/recorder Start deadline"
                );
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
            ensure!(elapsed < Duration::from_secs(5), "source Start deadline");
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
            ensure!(
                elapsed < Duration::from_secs(7) + self.source_duration,
                "source submission deadline"
            );
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

#[cfg(any(windows, test))]
fn validate_graph_observation(value: &Value, running_required: bool) -> Result<bool> {
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
    validate_bridge_observation(value, running_required)?;
    Ok(started)
}

#[cfg(any(windows, test))]
fn validate_bridge_observation(value: &Value, delivery_required: bool) -> Result<()> {
    let bridge = value.get("bridge").context("graph bridge missing")?;
    for key in [
        "dropped_frames",
        "nonfinite_samples",
        "discontinuities",
        "timestamp_errors",
        "backlog_discarded_frames",
        "age_discarded_frames",
        "underrun_frames",
        "resets",
    ] {
        ensure!(
            bridge
                .get(key)
                .and_then(Value::as_u64)
                .with_context(|| format!("missing bridge {key}"))?
                == 0,
            "Process bridge fault: {key}"
        );
    }
    let delivery = bridge.get("native_delivery").filter(|v| !v.is_null());
    if delivery_required {
        ensure!(delivery.is_some(), "Process delivery evidence unavailable");
    }
    if let Some(delivery) = delivery {
        for key in [
            "packet_bound_violations",
            "gap_bound_violations",
            "unavailable_clocks",
            "regressed_clocks",
        ] {
            ensure!(
                delivery
                    .get(key)
                    .and_then(Value::as_u64)
                    .with_context(|| format!("missing delivery {key}"))?
                    == 0,
                "Process delivery fault: {key}"
            );
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
    Ok(())
}

#[cfg(any(windows, test))]
fn validate_final_graph(capture: &Value, render: &Value, identity: &Value) -> Result<()> {
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
            ][..],
        ),
        (
            render,
            &["/stats/timeout_wakes", "/stats/bridge_shortfall_frames"][..],
        ),
    ] {
        for pointer in pointers {
            ensure!(
                report.pointer(pointer).and_then(Value::as_u64) == Some(0),
                "final graph missing/fault counter {pointer}"
            );
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

#[cfg(any(windows, test))]
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
    let mut guard = Guard::default();
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
                Ok(owner) => owner,
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
        guard.graph.as_ref().context("graph missing")?.activate()?;
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
            let observation = guard.graph.as_ref().context("graph missing")?.observation();
            guard.last_observation = Some(observation.clone());
            let graph_started = match validate_and_latch(
                &observation,
                phases.submitted.is_some(),
                &mut guard.first_failure,
            ) {
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
            let action = phases.tick(
                initial.elapsed(),
                Health {
                    graph_started,
                    recorder_running,
                    recorder_deadline: matches!(measurement, MeasurementProgress::DeadlineReached),
                    source_running: source.running,
                    source_submitted: source.submitted,
                },
            )?;
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
            validate_final_graph(&capture, &render, identity)
        })();
        if let Err(error) = validation {
            append_failure(&mut guard.failures, &format!("final graph: {error:#}"));
        }
    }
    let mut persistence = Ok(());
    if let Some(graph) = &guard.graph {
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
        if let Some(graph) = &self.graph {
            let value = graph.observation();
            if let Err(error) = validate_and_latch(&value, false, &mut self.first_failure) {
                append_failure(&mut self.failures, &format!("pre-stop graph: {error:#}"));
            }
            self.last_observation = Some(value);
        }
        let mut cleanup = None;
        retire(|stage| step(self, stage), &mut cleanup);
        if let Some(error) = cleanup {
            append_failure(&mut self.failures, &error);
        }
        if let Some(graph) = &self.graph {
            self.final_observation = Some(graph.observation());
        }
        if let Some(value) = &self.final_observation
            && let Err(error) = validate_bridge_observation(value, true)
        {
            // Owner finished/absent is expected after joins. Reconcile the
            // final counters and timeline independently, retaining earlier raw.
            if self.first_failure.is_none() {
                self.first_failure = Some(value.clone());
            }
            append_failure(&mut self.failures, &format!("final bridge: {error:#}"));
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
                    ensure!(
                        m.failure.is_none(),
                        "recorder final: {}",
                        m.failure.as_deref().unwrap_or_default()
                    );
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
