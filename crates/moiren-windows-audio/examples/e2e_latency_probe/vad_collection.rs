//! One concrete VAD source/graph/recorder run, always unqualified.

#[cfg(windows)]
use crate::controlled_load;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use std::time::Duration;
use std::{fs::File, io::Read, path::Path};

#[cfg(any(windows, test))]
use super::validation_policy::ValidationPolicy;

#[cfg(any(windows, test))]
#[path = "vad_collection/boundary_quality.rs"]
mod boundary_quality;

#[cfg(any(windows, test))]
#[path = "vad_collection/clock_quality.rs"]
pub(super) mod clock_quality;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    schema_version: u32,
    #[serde(default)]
    input_endpoint_id: String,
    ring_frames: usize,
    max_pcm_bytes: u64,
    max_metadata_bytes: u64,
    max_packets: u64,
    preroll_ms: u64,
    postroll_ms: u64,
}

impl Config {
    pub(super) fn validate_route(&self, route: super::vad_route::RouteKind) -> Result<()> {
        ensure!(
            matches!(self.schema_version, 1 | 2),
            "unsupported collection schema"
        );
        ensure!(
            !self.input_endpoint_id.contains('\0')
                && (self.input_endpoint_id.is_empty() || !self.input_endpoint_id.trim().is_empty()),
            "collection input identity malformed"
        );
        ensure!(
            !self.input_endpoint_id.is_empty()
                || (self.schema_version == 2 && route == super::vad_route::RouteKind::Microphone),
            "collection requires explicit input except schema2 owned microphone route"
        );
        Ok(())
    }

    #[cfg(any(windows, test))]
    pub(super) fn recorder_endpoint(&self, resolved_mic: Option<&str>) -> Result<String> {
        if let Some(resolved) = resolved_mic {
            self.validate_route(super::vad_route::RouteKind::Microphone)?;
            ensure!(
                !resolved.trim().is_empty() && !resolved.contains('\0'),
                "resolved microphone identity malformed"
            );
            ensure!(
                self.input_endpoint_id.is_empty() || self.input_endpoint_id == resolved,
                "explicit collection input must exactly match the owned microphone MMDevice"
            );
            return Ok(resolved.to_owned());
        }
        self.validate_route(super::vad_route::RouteKind::Physical)?;
        Ok(self.input_endpoint_id.clone())
    }

    fn validate(&self, frames: u64, seconds: u64) -> Result<()> {
        // Route validation runs before artifacts. Parsing alone permits the
        // schema2 omission, but never weakens schema1's explicit identity.
        self.validate_route(super::vad_route::RouteKind::Microphone)?;
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
fn collection_render_queue(
    options: &super::LiveOptions,
) -> Result<Option<moiren_windows_audio::latency::RenderQueuePolicy>> {
    (options.vad_route_kind() == super::vad_route::RouteKind::Physical)
        .then(|| super::render_queue(options))
        .transpose()
}

#[cfg(any(windows, test))]
fn pin_recorder_endpoint(selected: &str, prepared: &str) -> Result<()> {
    ensure!(
        !selected.trim().is_empty() && !selected.contains('\0') && prepared == selected,
        "prepared recorder input identity differs from exact selected MMDevice"
    );
    Ok(())
}

#[cfg(any(windows, test))]
fn pin_microphone_identity(
    selected: &serde_json::Value,
    prepared: &serde_json::Value,
) -> Result<()> {
    ensure!(
        selected == prepared,
        "owned microphone identity changed during recorder preparation"
    );
    Ok(())
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
    pub route: super::vad_route::RouteKind,
    pub microphone: Option<EndpointHealth>,
    pub stamp: Option<[u64; 5]>,
    pub shape: Option<(String, String, u32)>,
    pub provider_ready: bool,
    pub provider_error: Option<String>,
    pub revision: Option<u64>,
    pub generations: [u64; 2],
    pub engine_running: bool,
    pub engine_progress_overdue: bool,
    pub engine_fault: bool,
    pub native_generation: Option<u64>,
    pub native_running: bool,
    pub quality_known: bool,
    pub quality_faults: u64,
}

#[cfg(any(windows, test))]
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct EndpointHealth {
    pub stamp: Option<[u64; 5]>,
    pub shape: Option<(String, String, u32)>,
    pub stream_running: bool,
    pub provider_ready: bool,
    pub provider_error: Option<String>,
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
        self.require_operational()?;
        ensure!(
            !self.engine_progress_overdue,
            "Engine progress base deadline exceeded"
        );
        ensure!(
            self.quality_faults == 0
                && self
                    .microphone
                    .as_ref()
                    .is_none_or(|mic| mic.quality_faults == 0),
            "native/provider quality faults: {}",
            self.quality_faults
        );
        Ok(())
    }

    fn require_operational(&self) -> Result<()> {
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
        match self.route {
            super::vad_route::RouteKind::Physical => ensure!(
                self.microphone.is_none(),
                "physical route cannot carry microphone admission"
            ),
            super::vad_route::RouteKind::Microphone => {
                let mic = self
                    .microphone
                    .as_ref()
                    .context("microphone health unavailable")?;
                ensure!(
                    mic.stamp.is_some()
                        && mic.shape.is_some()
                        && mic.provider_ready
                        && mic.provider_error.is_none()
                        && mic.quality_known
                        && mic.stream_running,
                    "microphone provider absent, not Running, not admitted, unknown or reported error: {:?}",
                    mic.provider_error
                );
            }
        }
        Ok(())
    }
    fn validate_identity(&self, admitted: &Self) -> Result<()> {
        ensure!(
            self.route == admitted.route
                && self.stamp == admitted.stamp
                && self.shape == admitted.shape,
            "provider generation/gate/shape changed"
        );
        ensure!(
            match (&self.microphone, &admitted.microphone) {
                (None, None) => true,
                (Some(current), Some(before)) =>
                    current.stamp == before.stamp && current.shape == before.shape,
                _ => false,
            },
            "microphone provider generation/gate/shape changed"
        );
        ensure!(
            self.revision == admitted.revision
                && self.generations == admitted.generations
                && self.native_generation == admitted.native_generation,
            "graph revision/owner/resource generation changed"
        );
        Ok(())
    }
    pub(super) fn validate(&self, admitted: &Self) -> Result<()> {
        self.require_present()?;
        self.validate_identity(admitted)
    }
}

#[cfg(any(windows, test))]
#[derive(Default)]
struct Quality {
    failure: Option<String>,
    first_observations: Option<serde_json::Value>,
    playback_boundary: Option<boundary_quality::Tracker>,
}

#[cfg(any(windows, test))]
impl Quality {
    fn boundary_evidence(&self) -> serde_json::Value {
        serde_json::json!({"qualified": false, "quality_complete": false,
            "playback": self.playback_boundary})
    }

    fn record(
        &mut self,
        message: &str,
        report: Option<&super::NativeReport>,
        health: Option<&Health>,
    ) {
        // Polls often repeat cumulative counters. Keep diagnostics bounded and
        // retain the first raw observation before a later QUERY replaces it.
        if !self
            .failure
            .as_ref()
            .is_some_and(|text| text.contains(message))
        {
            append_failure(&mut self.failure, message);
        }
        if self.first_observations.is_none() {
            self.first_observations = Some(match report {
                Some(report) => serde_json::json!({
                    "provider": report.vad_provider, "microphone_provider": report.vad_microphone_provider,
                    "engine": report.engine, "health": health,
                }),
                None => serde_json::json!({"observation": message, "health": health}),
            });
        }
    }
}

#[cfg(any(windows, test))]
fn validate_health_with_policy(
    current: &Health,
    admitted: &Health,
    report: &super::NativeReport,
    first_failure: &mut Option<serde_json::Value>,
    policy: ValidationPolicy,
    quality: &mut Quality,
) -> Result<()> {
    if current.engine_progress_overdue {
        quality.record(
            "Engine progress base deadline exceeded",
            Some(report),
            Some(current),
        );
    }
    if quality.playback_boundary.is_some() {
        let result = (|| -> Result<()> {
            current.require_operational()?;
            current.validate_identity(admitted)?;
            let native = report
                .engine
                .as_ref()
                .and_then(|engine| engine.get("observations"))
                .context("classified playback native observations unavailable")?;
            ensure!(
                native
                    .get("owner_generation")
                    .and_then(serde_json::Value::as_u64)
                    == current.native_generation,
                "classified playback owner differs from admitted native owner"
            );
            let faults = quality
                .playback_boundary
                .as_mut()
                .unwrap()
                .observe(native, false)?;
            for message in faults {
                quality.record(&message, Some(report), Some(current));
                ensure!(policy.is_correctness(), "{message}");
            }
            Ok(())
        })();
        if let Err(error) = result {
            quality.record(
                &format!("playback boundary: {error:#}"),
                Some(report),
                Some(current),
            );
            latch_failure_observations(first_failure, report, Some(current));
            return Err(error);
        }
    }
    if !policy.is_correctness() {
        let result = validate_health(current, admitted, report, first_failure);
        if current.quality_faults != 0 {
            quality.record(
                &format!("native/provider quality faults: {}", current.quality_faults),
                Some(report),
                Some(current),
            );
        }
        return result;
    }
    let result = (|| {
        current.require_operational()?;
        current.validate_identity(admitted)?;
        // Health's aggregate mixes xruns with nonfinite samples and resets.
        // Check the raw typed-counter serialization before relaxing that gate.
        let provider = report
            .vad_provider
            .as_ref()
            .context("provider evidence unavailable")?;
        let epoch_resets = provider
            .pointer("/telemetry/epoch_resets")
            .and_then(serde_json::Value::as_u64)
            .context("provider epoch reset counter unavailable")?;
        ensure!(epoch_resets == 0, "provider epoch reset");
        if let Some(mic) = &current.microphone {
            let provider = report
                .vad_microphone_provider
                .as_ref()
                .context("microphone provider evidence unavailable")?;
            ensure!(
                provider
                    .pointer("/telemetry/epoch_resets")
                    .and_then(serde_json::Value::as_u64)
                    == Some(0),
                "microphone provider epoch reset or counter unavailable"
            );
            ensure!(
                provider
                    .pointer("/snapshot/packet_frames")
                    .and_then(serde_json::Value::as_u64)
                    == mic.shape.as_ref().map(|shape| u64::from(shape.2))
                    && mic.shape.as_ref().is_some_and(|shape| shape.2 > 0),
                "microphone packet shape does not match current health"
            );
        }
        let native = report
            .engine
            .as_ref()
            .and_then(|engine| engine.get("observations"))
            .context("native observations unavailable")?;
        let bridges = native
            .get("bridges")
            .and_then(serde_json::Value::as_array)
            .context("native bridge evidence unavailable")?;
        // Professional VAD uses virtual boundary probes, not ClockBridge.
        // An empty bridge array is valid only with real playback evidence;
        // never replace missing/malformed probes with fabricated zero faults.
        let streams = native
            .get("streams")
            .and_then(serde_json::Value::as_array)
            .context("native stream evidence unavailable")?;
        let probes = native
            .get("boundary_probes")
            .and_then(serde_json::Value::as_array)
            .context("native boundary probe evidence unavailable")?;
        let packet_frames = current
            .shape
            .as_ref()
            .context("provider shape unavailable")?
            .2;
        ensure!(
            packet_frames > 0
                && provider
                    .pointer("/snapshot/packet_frames")
                    .and_then(serde_json::Value::as_u64)
                    == Some(u64::from(packet_frames)),
            "provider packet shape does not match current health"
        );
        let playback: Vec<_> = streams
            .iter()
            .filter(|stream| {
                matches!(
                    stream.get("kind").and_then(serde_json::Value::as_str),
                    Some("virtual_playback" | "virtual_microphone")
                )
            })
            .collect();
        ensure!(
            !playback.is_empty(),
            "native VirtualPlayback stream evidence unavailable"
        );
        if current.route == super::vad_route::RouteKind::Microphone {
            ensure!(
                streams.len() == 2
                    && playback.len() == 2
                    && probes.len() == 2
                    && ["virtual_playback", "virtual_microphone"]
                        .iter()
                        .all(|kind| playback
                            .iter()
                            .filter(
                                |stream| stream.get("kind").and_then(serde_json::Value::as_str)
                                    == Some(*kind)
                            )
                            .count()
                            == 1),
                "microphone route requires exactly playback/microphone streams and probes"
            );
        } else {
            ensure!(
                playback.iter().all(|stream| stream
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("virtual_playback")),
                "physical route cannot carry a virtual microphone stream"
            );
        }
        let mut matched = std::collections::BTreeSet::new();
        let mut probe_faults = Vec::new();
        for stream in playback {
            let node = stream
                .get("node")
                .and_then(serde_json::Value::as_str)
                .filter(|node| !node.trim().is_empty())
                .context("native VirtualPlayback node identity unavailable")?;
            ensure!(
                matched.insert(node),
                "duplicate native VirtualPlayback node identity"
            );
            let matching: Vec<_> = probes
                .iter()
                .filter(|probe| probe.get("node").and_then(serde_json::Value::as_str) == Some(node))
                .collect();
            ensure!(
                matching.len() == 1,
                "native VirtualPlayback boundary probe missing or duplicated"
            );
            let probe = matching[0];
            ensure!(
                probe.get("kind") == stream.get("kind"),
                "native VirtualPlayback boundary probe kind mismatch"
            );
            let number = |key: &str| -> Result<u64> {
                probe
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .filter(|count| *count != u64::MAX)
                    .with_context(|| {
                        format!("native boundary probe {key} unavailable or malformed")
                    })
            };
            let capacity = number("capacity_frames")?;
            ensure!(
                capacity > 0
                    && capacity <= u64::from(u32::MAX)
                    && number("level_frames")? <= capacity,
                "native boundary probe capacity/occupancy invalid"
            );
            match probe.get("target_frames") {
                Some(serde_json::Value::Null) => {}
                Some(target) => ensure!(
                    target
                        .as_u64()
                        .is_some_and(|frames| frames > 0 && frames <= capacity),
                    "native boundary probe target malformed"
                ),
                None => anyhow::bail!("native boundary probe target availability missing"),
            }
            ensure!(
                probe
                    .get("applied_milli_ppm")
                    .and_then(serde_json::Value::as_i64)
                    .is_some_and(|ppm| i32::try_from(ppm).is_ok()),
                "native boundary probe correction unavailable or malformed"
            );
            for key in ["xruns", "soft_events"] {
                let count = number(key)?;
                let classified_playback = quality.playback_boundary.is_some()
                    && stream.get("kind").and_then(serde_json::Value::as_str)
                        == Some("virtual_playback");
                if count != 0 && !(key == "soft_events" && classified_playback) {
                    probe_faults.push(format!("native {} boundary {key}: {count}", stream["kind"]));
                }
            }
        }
        for bridge in bridges {
            for key in ["nonfinite_samples", "resets"] {
                let count = bridge
                    .pointer(&format!("/counters/{key}"))
                    .and_then(serde_json::Value::as_u64)
                    .with_context(|| format!("native bridge {key} unavailable"))?;
                ensure!(count == 0, "native bridge {key}: {count}");
            }
            if bridge
                .pointer("/counters/silent_frames")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|count| count != 0)
            {
                quality.record("native bridge silence", Some(report), Some(current));
            }
        }
        for message in probe_faults {
            quality.record(&message, Some(report), Some(current));
        }
        if current.quality_faults != 0 {
            quality.record(
                &format!("native/provider quality faults: {}", current.quality_faults),
                Some(report),
                Some(current),
            );
        }
        if let Some(mic) = &current.microphone {
            if mic.quality_faults != 0 {
                quality.record(
                    &format!("microphone provider quality faults: {}", mic.quality_faults),
                    Some(report),
                    Some(current),
                );
            }
            if report
                .vad_microphone_provider
                .as_ref()
                .is_some_and(|provider| {
                    ["/telemetry/silence_frames", "/snapshot/silence_packets"]
                        .iter()
                        .any(|path| {
                            provider
                                .pointer(path)
                                .and_then(serde_json::Value::as_u64)
                                .is_some_and(|count| count != 0)
                        })
                })
            {
                quality.record("microphone provider silence", Some(report), Some(current));
            }
        }
        if ["/telemetry/silence_frames", "/snapshot/silence_packets"]
            .iter()
            .any(|path| {
                provider
                    .pointer(path)
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|count| count != 0)
            })
        {
            quality.record("provider silence", Some(report), Some(current));
        }
        Ok(())
    })();
    if result.is_err() {
        latch_failure_observations(first_failure, report, Some(current));
    }
    result
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
            "provider": report.vad_provider, "microphone_provider": report.vad_microphone_provider,
            "engine": report.engine, "health": health,
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
#[cfg_attr(not(test), allow(dead_code))]
fn retire(mut step: impl FnMut(Retirement) -> Result<()>, failures: &mut Option<String>) {
    retire_with_route(super::vad_route::RouteKind::Physical, &mut step, failures);
}

#[cfg(any(windows, test))]
fn retire_with_route(
    route: super::vad_route::RouteKind,
    mut step: impl FnMut(Retirement) -> Result<()>,
    failures: &mut Option<String>,
) {
    // Retain active evidence first, then revoke shared admission without a
    // native QUERY before either native Stop can cancel active PCM. Broadcast
    // both stops before joins or gate IO. A microphone recorder must also
    // release its client before removal of the export it was recording.
    let microphone = route == super::vad_route::RouteKind::Microphone;
    for stage in [
        Retirement::Observe,
        Retirement::ObserveFinal,
        Retirement::Invalidate,
        Retirement::StopSource,
        Retirement::StopMeasurement,
        Retirement::JoinSource,
        Retirement::CloseGate,
        Retirement::ShutdownHost,
        if microphone {
            Retirement::FinishMeasurement
        } else {
            Retirement::Retire
        },
        if microphone {
            Retirement::Retire
        } else {
            Retirement::FinishMeasurement
        },
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
    options.validation.validate()?;
    let provenance = super::source::SourceProvenance::new(Some(&plan));
    let frames = provenance
        .schedule
        .as_ref()
        .context("collection source schedule missing")?
        .total_source_frames;
    let mut phases = Phases::new(config.clone(), frames, options.seconds);
    let controlled_load_required_window = phases
        .source_duration
        .saturating_add(Duration::from_millis(config.postroll_ms));
    // Install the outer owner before recorder preparation or CREATE. Even a
    // half-prepared source has a stop-before-join callback into this same owner.
    let mut guard = CollectionGuard {
        vad,
        route: options.vad_route_kind(),
        recorder_admitted: false,
        measurement: None,
        measurement_outcome: None,
        measurement_preparation_capture: None,
        controlled_load_id: options.controlled_load_id.clone(),
        controlled_load: None,
        controlled_load_evidence: None,
        controlled_load_required_window,
        failures: None,
        cleanup_failures: None,
        first_failure_observations: None,
        policy: options.validation,
        quality: Quality {
            playback_boundary: Some(boundary_quality::Tracker::default()),
            ..Default::default()
        },
        retired: false,
    };
    let mut last_running = None;
    let collection = (|| -> Result<()> {
        config.validate_route(guard.route)?;
        let (recorder_endpoint, selected_microphone) = if let Some(target) =
            options.microphone_target()?
        {
            // The guard owns partial CREATEs before exact discovery. Never
            // replace the selected owned microphone with a default/alias.
            let discovery_deadline = Instant::now()
                .checked_add(Duration::from_secs(20))
                .context("microphone discovery deadline overflow")?;
            guard.vad.prepare_exports(options, name)?;
            let stable_id = moiren_windows_audio::vad::StableEndpointId::from_virtual_endpoint_id(
                target.endpoint_id,
            );
            let identity =
                super::vad::wait_microphone_identity(stable_id, &root, discovery_deadline)?;
            let selected = serde_json::to_value(&identity)?;
            let route = guard
                .vad
                .report
                .vad_route
                .as_mut()
                .context("microphone route metadata missing after export preparation")?;
            route["resolved_microphone"] = selected.clone();
            let endpoint = config.recorder_endpoint(Some(&identity.mmdevice_id))?;
            route["recorder_endpoint_id"] = endpoint.clone().into();
            (endpoint, Some((stable_id, selected)))
        } else {
            (config.recorder_endpoint(None)?, None)
        };
        guard.measurement = Some(
            match MeasurementSession::prepare(
                MeasurementConfig {
                    input_endpoint_id: recorder_endpoint.clone(),
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
            ) {
                Ok(measurement) => measurement.with_start_grace(guard.policy.progress_grace())?,
                Err(mut error) => {
                    guard.measurement_preparation_capture = error.native_capture.take();
                    return Err(error.error);
                }
            },
        );
        let prepared_endpoint = &guard
            .measurement
            .as_ref()
            .context("prepared recorder missing")?
            .info()
            .input_endpoint_id;
        if let Some(route) = guard.vad.report.vad_route.as_mut() {
            route["prepared_recorder_endpoint_id"] = prepared_endpoint.clone().into();
        }
        pin_recorder_endpoint(&recorder_endpoint, prepared_endpoint)?;
        if let Some((stable_id, selected)) = selected_microphone {
            // Native preparation is fallible and can race endpoint recreation.
            // Re-resolve the entire owned identity before opening graph/gates;
            // preserve the attempted check and both observations on refusal.
            let identity = super::vad::resolve_microphone_identity(stable_id, &root);
            let route =
                guard.vad.report.vad_route.as_mut().context(
                    "microphone route metadata missing during recorder identity recheck",
                )?;
            let identity = match identity {
                Ok(identity) => identity,
                Err(error) => {
                    route["recorder_identity_recheck"] = serde_json::json!({
                        "status": "failed", "error": format!("{error:#}"),
                    });
                    return Err(error)
                        .context("re-resolve owned microphone after recorder preparation");
                }
            };
            let prepared = serde_json::to_value(&identity)?;
            route["recorder_prepared_microphone"] = prepared.clone();
            let matched = pin_microphone_identity(&selected, &prepared);
            route["recorder_identity_recheck"] = match &matched {
                Ok(()) => serde_json::json!({"status": "matched"}),
                Err(error) => {
                    serde_json::json!({"status": "changed", "error": format!("{error:#}")})
                }
            };
            matched?;
        }
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
        if guard.route == super::vad_route::RouteKind::Physical {
            let admitted = &guard
                .vad
                .admission
                .as_ref()
                .context("collection admission missing")?
                .current;
            validate_health_with_policy(
                admitted,
                admitted,
                guard.vad.report,
                &mut guard.first_failure_observations,
                guard.policy,
                &mut guard.quality,
            )?;
            guard.recorder_admitted = true;
        }
        // Anchor before the call; a delayed activation return never creates a
        // fresh interval. MeasurementSession's own earlier deadline also wins.
        let activated = Instant::now();
        let recorder_start_deadline = phases.recorder_start_deadline(activated, guard.policy)?;
        phases.stamp("recorder_activation_requested", Duration::ZERO);
        guard
            .measurement
            .as_mut()
            .context("recorder missing")?
            .activate()?;
        let mut recorder_start_confirmed = false;
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
            if !recorder_start_confirmed && matches!(recorder, MeasurementProgress::Running { .. })
            {
                progress_deadline(
                    activated.elapsed(),
                    Duration::from_secs(5),
                    "recorder Start deadline",
                    guard.policy,
                    &mut guard.quality,
                )?;
                // Confirm initial native Running before preroll without adopting
                // changed stamps or granting a new startup interval.
                guard
                    .vad
                    .confirm_recorder_started(recorder_start_deadline)?;
                let current = guard.vad.current_health()?;
                validate_health_with_policy(
                    &current,
                    &guard
                        .vad
                        .admission
                        .as_ref()
                        .context("collection admission missing")?
                        .current,
                    guard.vad.report,
                    &mut guard.first_failure_observations,
                    guard.policy,
                    &mut guard.quality,
                )?;
                guard.recorder_admitted = true;
                recorder_start_confirmed = true;
                phases.admit_recorder(activated.elapsed())?;
                last_running = Some(current);
            }
            if let Some(load_id) = guard.controlled_load_id.as_deref()
                && guard.controlled_load.is_none()
                && matches!(recorder, MeasurementProgress::Running { .. })
            {
                let directory = Path::new(&paths.directory);
                guard.controlled_load = Some(controlled_load::Supervisor::start(
                    load_id,
                    options.seconds,
                    directory.join("controlled-load.stop"),
                    directory.join("controlled-load.result.json"),
                )?);
            }
            if let Some(load) = guard.controlled_load.as_mut() {
                load.check()?;
            }
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
                validate_source_progress(
                    source,
                    guard.policy,
                    &mut guard.quality,
                    Some(guard.vad.report),
                )?;
            }
            if guard.recorder_admitted {
                let current = guard.vad.current_health()?;
                validate_health_with_policy(
                    &current,
                    &guard
                        .vad
                        .admission
                        .as_ref()
                        .context("collection admission missing")?
                        .current,
                    guard.vad.report,
                    &mut guard.first_failure_observations,
                    guard.policy,
                    &mut guard.quality,
                )?;
                last_running = Some(current);
            }
            match phases.tick_with_policy(
                activated.elapsed(),
                matches!(recorder, MeasurementProgress::Running { .. }),
                recorder == MeasurementProgress::DeadlineReached,
                source.as_ref(),
                guard.policy,
                &mut guard.quality,
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
    let source_submitted_complete = reconcile_source_outcome_with_policy(
        guard.vad.playback_outcome.as_ref(),
        frames,
        &mut guard.failures,
        guard.policy,
        &mut guard.quality,
    );
    if let Some(outcome) = &guard.measurement_outcome {
        if let Err(error) =
            validate_measurement_outcome_with_policy(outcome, guard.policy, &mut guard.quality)
        {
            append_failure(
                &mut guard.failures,
                &format!("measurement outcome: {error:#}"),
            );
        }
        if outcome.native_capture.is_none()
            || outcome
                .sink
                .as_ref()
                .is_none_or(|sink| sink.written_frames == 0)
        {
            append_failure(
                &mut guard.failures,
                "selected recorder native/sink evidence unavailable or empty",
            );
        }
    } else {
        append_failure(&mut guard.failures, "measurement outcome unavailable");
    }
    if (phases_complete || guard.vad.report.backend_diagnostics.is_some())
        && let Err(error) = graph_summary_health_with_route(
            guard.vad.report.backend_diagnostics.as_ref(),
            guard.route,
            guard.policy,
            &mut guard.quality,
        )
    {
        append_failure(
            &mut guard.failures,
            &format!("final graph native summary: {error:#}"),
        );
    }
    let complete = phases_complete && guard.failures.is_none();
    // All joins and typed outcome retention precede report serialization/IO.
    let measurement = if let Some(outcome) = &guard.measurement_outcome {
        Some(serde_json::json!({
            "info": measurement_info(&outcome.info),
            "native_capture": outcome.native_capture.as_ref().map(serde_json::to_value).transpose()?,
            "sink": outcome.sink.as_ref().map(serde_json::to_value).transpose()?,
            "activation_elapsed_seconds": outcome.activation_elapsed.map(|value| value.as_secs_f64()),
            "failure": outcome.failure,
        }))
    } else {
        guard
            .measurement_preparation_capture
            .take()
            .map(|capture| {
                Ok::<_, anyhow::Error>(serde_json::json!({
                    "info": null,
                    "native_capture": serde_json::to_value(capture)?,
                    "sink": null,
                    "activation_elapsed_seconds": null,
                    "failure": guard.failures,
                }))
            })
            .transpose()?
    };
    guard.vad.report.collection = Some(serde_json::json!({
        "schema_version": 1, "qualified": false, "status": "unqualified",
        "validation_policy": guard.policy,
        "quality_passed": complete && guard.quality.failure.is_none(),
        "quality_complete": false,
        "boundary_quality": guard.quality.boundary_evidence(),
        "quality_failure": guard.quality.failure,
        "first_quality_observations": guard.quality.first_observations,
        "config": config, "playback_source": provenance, "source": guard.vad.playback_outcome,
        "graph": { "route": guard.route, "input_kind": "VirtualPlayback",
            "output_kind": if guard.route == super::vad_route::RouteKind::Microphone { "VirtualMicrophone" } else { "Physical" },
            "stable_endpoint_id": options.stable_endpoint_id,
            "playback_stable_endpoint_id": options.stable_endpoint_id,
            "microphone_stable_endpoint_id": options.vad_microphone_id.as_ref().map(|id| id.to_ascii_lowercase()),
            "output_endpoint_id": options.output_label(), "engine_block_frames": options.engine_block_frames,
            "pacing": if guard.route == super::vad_route::RouteKind::Microphone { "pure_virtual_realtime" } else { "physical_master" },
            "boundary": if guard.route == super::vad_route::RouteKind::Microphone { "internal" } else { "unproven_physical" },
            "qualified": false, "independent_reference": false,
            "route_evidence": guard.vad.report.vad_route,
            "render_queue": collection_render_queue(options)?,
            "physical_render_queue_applicable": guard.route == super::vad_route::RouteKind::Physical,
            "virtual_buffer": super::virtual_buffer(options)?,
            "telemetry": options.telemetry, "admission": guard.vad.admission,
            "last_running": last_running, "last_observation": guard.vad.last_health,
            "engine": guard.vad.report.engine, "provider": guard.vad.report.vad_provider,
            "microphone_provider": guard.vad.report.vad_microphone_provider,
            "native_summary": guard.vad.report.backend_diagnostics,
            "engine_cleanup": guard.vad.report.engine_cleanup, "vad_cleanup": guard.vad.report.vad_cleanup,
            "microphone_cleanup": guard.vad.report.vad_microphone_cleanup },
        "measurement": measurement, "phases": phases.stamps,
        "controlled_load_id": guard.controlled_load_id,
        "controlled_load": guard.controlled_load_evidence,
        "controlled_load_requested": guard.controlled_load_id.is_some(),
        "source_submitted_complete": source_submitted_complete,
        "source_completion_observed_in_phases": phases.completed.is_some(), "preroll_complete": phases.source_requested.is_some(),
        "postroll_complete": phases_complete, "collection_complete": complete,
        "cleanup_complete": guard.cleanup_failures.is_none(), "cleanup_failure": guard.cleanup_failures,
        "failure": guard.failures, "first_failure_observations": guard.first_failure_observations,
        "evidence_scope": if guard.route == super::vad_route::RouteKind::Microphone {
            "internal software playback/Bus/microphone boundary with owned native recorder; no independent reference or delay/calibration claim; duplicated stereo and source submission are not marker detection"
        } else {
            "selected independent recorder endpoint; wiring/reference/calibration unproven; source submission is not played/captured marker detection"
        },
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
    route: super::vad_route::RouteKind,
    recorder_admitted: bool,
    measurement: Option<crate::measurement::session::MeasurementSession>,
    measurement_outcome: Option<crate::measurement::session::MeasurementOutcome>,
    measurement_preparation_capture: Option<Box<moiren_windows_audio::capture::CaptureReport>>,
    controlled_load_id: Option<String>,
    controlled_load: Option<controlled_load::Supervisor>,
    controlled_load_evidence: Option<controlled_load::Evidence>,
    controlled_load_required_window: Duration,
    failures: Option<String>,
    cleanup_failures: Option<String>,
    first_failure_observations: Option<serde_json::Value>,
    policy: ValidationPolicy,
    quality: Quality,
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
        if let Some(load) = self.controlled_load.take() {
            match load.finish(self.controlled_load_required_window) {
                Ok(evidence) => {
                    if !evidence.valid {
                        append_failure(
                            &mut self.failures,
                            evidence
                                .failure
                                .as_deref()
                                .unwrap_or("controlled load evidence is invalid"),
                        );
                    }
                    self.controlled_load_evidence = Some(evidence);
                }
                Err(error) => append_failure(
                    &mut self.failures,
                    &format!("finish controlled load: {error:#}"),
                ),
            }
        } else if self.controlled_load_id.is_some() {
            append_failure(
                &mut self.failures,
                "controlled load was requested but never started",
            );
        }
        let mut failures = None;
        retire_with_route(
            self.route,
            |stage| match stage {
                Retirement::Observe => {
                    if self.recorder_admitted
                        && let Some(admitted) = &self.vad.admission
                    {
                        let admitted = admitted.current.clone();
                        let health = match self.vad.current_health() {
                            Ok(health) => health,
                            Err(error) => {
                                latch_failure_observations(
                                    &mut self.first_failure_observations,
                                    self.vad.report,
                                    self.vad.last_health.as_ref(),
                                );
                                return Err(error);
                            }
                        };
                        validate_health_with_policy(
                            &health,
                            &admitted,
                            self.vad.report,
                            &mut self.first_failure_observations,
                            self.policy,
                            &mut self.quality,
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

#[cfg(all(windows, test))]
fn reconcile_source_outcome(
    outcome: Option<&super::vad::PlaybackOutcome>,
    frames: u64,
    failures: &mut Option<String>,
) -> bool {
    reconcile_source_outcome_with_policy(
        outcome,
        frames,
        failures,
        ValidationPolicy::default(),
        &mut Quality::default(),
    )
}

#[cfg(any(windows, test))]
fn validate_source_progress(
    source: &PlaybackProgress,
    policy: ValidationPolicy,
    quality: &mut Quality,
    report: Option<&super::NativeReport>,
) -> Result<()> {
    ensure!(!source.stopped, "source stopped");
    if source.event_timeouts != 0 {
        quality.record(
            &format!("source native event timeouts: {}", source.event_timeouts),
            report,
            None,
        );
        ensure!(policy.is_correctness(), "source native event timeout");
    }
    Ok(())
}

#[cfg(windows)]
fn reconcile_source_outcome_with_policy(
    outcome: Option<&super::vad::PlaybackOutcome>,
    frames: u64,
    failures: &mut Option<String>,
    policy: ValidationPolicy,
    quality: &mut Quality,
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
            let message = format!(
                "joined source native event timeouts: {}",
                outcome.event_timeouts
            );
            let first = quality.first_observations.is_none();
            quality.record(&message, None, None);
            if first {
                quality.first_observations = Some(serde_json::json!({"source": outcome}));
            }
            if !policy.is_correctness() {
                append_failure(failures, &message);
            }
        }
        if !(outcome.started && outcome.stopped && outcome.worker_joined && submitted_complete) {
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
fn observe_measurement_quality(
    outcome: &crate::measurement::session::MeasurementOutcome,
    quality: &mut Quality,
) {
    let first = quality.first_observations.is_none();
    if let Some(native) = &outcome.native_capture {
        if native.timeout_wakes != 0 {
            quality.record(
                &format!(
                    "measurement native event timeouts: {}",
                    native.timeout_wakes
                ),
                None,
                None,
            );
        }
        if let Some(evidence) = native.evidence_metadata
            && (evidence.dropped_frames != 0
                || evidence.silent_frames != 0
                || evidence.overflow_packets != 0
                || evidence.gap_events != 0
                || evidence.gap_frames != 0
                || evidence.discontinuity_events != 0
                || evidence.timestamp_error_packets != 0)
        {
            quality.record(
                "measurement evidence loss, silence or discontinuity",
                None,
                None,
            );
        }
    }
    if outcome
        .sink
        .as_ref()
        .is_some_and(|sink| sink.sequence_gaps != 0)
    {
        quality.record("measurement sink sequence gaps", None, None);
    }
    if first && quality.first_observations.is_some() {
        quality.first_observations = Some(serde_json::json!({
            "measurement": {"info": measurement_info(&outcome.info),
                "native_capture": outcome.native_capture, "sink": outcome.sink,
                "failure": outcome.failure}
        }));
    }
}

#[cfg(any(windows, test))]
fn validate_measurement_outcome_with_policy(
    outcome: &crate::measurement::session::MeasurementOutcome,
    policy: ValidationPolicy,
    quality: &mut Quality,
) -> Result<()> {
    observe_measurement_quality(outcome, quality);
    if !policy.is_correctness() {
        ensure!(
            outcome.failure.is_none(),
            "{}",
            outcome.failure.as_deref().unwrap_or_default()
        );
        return Ok(());
    }
    let native = outcome
        .native_capture
        .as_ref()
        .context("measurement native evidence missing")?;
    let sink = outcome
        .sink
        .as_ref()
        .context("measurement sink evidence missing")?;
    let summary = native
        .evidence_metadata
        .context("measurement evidence summary missing")?;
    ensure!(
        outcome.activation_elapsed.is_some()
            && native.stream_started
            && native.stop_succeeded
            && native.failure.is_none()
            && sink.failure.is_none()
            && !sink.cancelled,
        "measurement activation/native Start/Stop/storage failure"
    );
    ensure!(
        sink.schema_version == 1
            && sink.sample_rate == outcome.info.sample_rate
            && sink.channels == outcome.info.channels
            && summary.sample_rate == outcome.info.sample_rate
            && summary.channels == outcome.info.channels,
        "measurement evidence schema/format mismatch"
    );
    ensure!(
        sink.nonfinite_samples == 0
            && sink.incomplete_packets == 0
            && !sink.counters_saturated
            && !summary.counters_saturated
            && !summary.sequence_exhausted
            && summary.position_overflow_packets == 0
            && summary.reader_abandoned_packets == 0,
        "measurement nonfinite, malformed, abandoned or exhausted evidence"
    );
    // The ring admits or drops whole packets before the sink sees them. Missing
    // sequence numbers are quality loss only when the native overflow counts
    // explain them; missing persisted PCM/metadata is still a hard failure.
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
        "measurement persisted frames/packets missing or inconsistent"
    );
    ensure!(
        (summary.dropped_frames == 0) == (summary.overflow_packets == 0),
        "measurement loss counters inconsistent"
    );
    let continuity_fault = summary.dropped_frames != 0
        || summary.overflow_packets != 0
        || summary.gap_events != 0
        || summary.discontinuity_events != 0
        || summary.timestamp_error_packets != 0;
    // Reconstruct the exact aggregate validate_outcome can generate from these
    // typed soft counters after every other cause above has been ruled out.
    // A prefix/substring filter could conceal a join, owner or storage failure.
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
    ensure!(
        outcome.failure.is_none() || outcome.failure == expected,
        "measurement operational failure: {}",
        outcome.failure.as_deref().unwrap_or_default()
    );
    Ok(())
}

#[cfg(any(windows, test))]
fn measurement_info(info: &crate::measurement::session::MeasurementInfo) -> serde_json::Value {
    serde_json::json!({"input_endpoint_id": info.input_endpoint_id, "sample_rate": info.sample_rate,
        "channels": info.channels, "ring_frames": info.ring_frames, "max_frames": info.max_frames})
}

#[cfg(test)]
fn graph_summary_health(summary: Option<&serde_json::Value>) -> Result<()> {
    graph_summary_health_with_policy(
        summary,
        ValidationPolicy::default(),
        &mut Quality::default(),
    )
}

#[cfg(test)]
fn graph_summary_health_with_policy(
    summary: Option<&serde_json::Value>,
    policy: ValidationPolicy,
    quality: &mut Quality,
) -> Result<()> {
    graph_summary_health_with_route(
        summary,
        super::vad_route::RouteKind::Physical,
        policy,
        quality,
    )
}

#[cfg(any(windows, test))]
fn graph_summary_health_with_route(
    summary: Option<&serde_json::Value>,
    route: super::vad_route::RouteKind,
    policy: ValidationPolicy,
    quality: &mut Quality,
) -> Result<()> {
    let summary = summary.context("graph native summary unknown")?;
    ensure!(
        summary
            .get("failure")
            .is_some_and(serde_json::Value::is_null),
        "graph native failure or unknown outcome"
    );
    if let Some(version) = summary.get("schema_version") {
        match version.as_u64() {
            Some(2) => {
                let result = final_graph_schema2_health(summary, route, policy, quality);
                if quality.playback_boundary.is_some()
                    && let Err(error) = &result
                {
                    let first = quality.first_observations.is_none();
                    quality.record(&format!("final graph evidence: {error:#}"), None, None);
                    if first {
                        quality.first_observations =
                            Some(serde_json::json!({"native_summary": summary}));
                    }
                }
                return result;
            }
            Some(1) => {}
            _ => anyhow::bail!("unsupported graph native summary schema"),
        }
    }
    ensure!(
        quality.playback_boundary.is_none(),
        "classified playback collection requires schema2 joined-owner evidence"
    );
    ensure!(
        route == super::vad_route::RouteKind::Physical,
        "microphone route requires schema2 final virtual boundary observations"
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
        let count = render
            .pointer(&format!("/stats/{key}"))
            .and_then(serde_json::Value::as_u64)
            .with_context(|| format!("graph native {key} unknown"))?;
        if count != 0 {
            let first = quality.first_observations.is_none();
            quality.record(&format!("graph native {key}: {count}"), None, None);
            if first {
                quality.first_observations = Some(serde_json::json!({"native_summary": summary}));
            }
            ensure!(policy.is_correctness(), "graph native {key} nonzero");
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn final_graph_schema2_health(
    summary: &serde_json::Value,
    route: super::vad_route::RouteKind,
    policy: ValidationPolicy,
    quality: &mut Quality,
) -> Result<()> {
    use serde_json::Value;
    let number = |value: &Value, key: &str| -> Result<u64> {
        value
            .get(key)
            .and_then(Value::as_u64)
            .filter(|count| *count != u64::MAX)
            .with_context(|| format!("final graph {key} missing, malformed or saturated"))
    };
    let flag = |value: &Value, key: &str| -> Result<bool> {
        value
            .get(key)
            .and_then(Value::as_bool)
            .with_context(|| format!("final graph {key} missing or malformed"))
    };
    let node = |value: &Value| -> Result<String> {
        value
            .get("node")
            .and_then(Value::as_str)
            .filter(|node| !node.trim().is_empty())
            .map(str::to_owned)
            .context("final graph node identity missing or malformed")
    };
    let array = |value: &Value, key: &str| -> Result<Vec<Value>> {
        value
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .with_context(|| format!("final graph {key} missing or malformed"))
    };
    ensure!(
        summary.get("observation_scope").and_then(Value::as_str) == Some("final_after_owner_join"),
        "graph summary is not a final joined-owner observation"
    );
    let generation = number(summary, "owner_generation")?;
    ensure!(generation > 0, "final graph owner generation unavailable");
    let professional = summary
        .get("professional")
        .context("final professional observation missing")?;
    let native = summary
        .get("native")
        .context("final native observation missing")?;
    for observation in [professional, native] {
        ensure!(
            number(observation, "owner_generation")? == generation
                && !flag(observation, "running")?,
            "final graph observation generation changed or owner still running"
        );
    }
    // The composing owner attests Closed/revision/failure against admission;
    // these retained DTOs prove the same generation and its post-join counters.
    let streams = array(native, "streams")?;
    let professional_streams = array(professional, "streams")?;
    ensure!(
        !streams.is_empty() && streams.len() == professional_streams.len(),
        "final native/professional stream identities incomplete"
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut master = None;
    let mut virtual_inputs = std::collections::BTreeSet::new();
    let mut virtual_outputs = std::collections::BTreeSet::new();
    let mut soft = Vec::new();
    match route {
        super::vad_route::RouteKind::Microphone => {
            let clock = native
                .get("virtual_clock")
                .context("final virtual clock unknown")?;
            let loss = clock_quality::parse_final(clock)?;
            if loss != 0 {
                soft.push(format!("final virtual clock clamped frames: {loss}"));
            }
        }
        super::vad_route::RouteKind::Physical => ensure!(
            native.get("virtual_clock").is_none_or(Value::is_null),
            "physical render master cannot claim a virtual clock"
        ),
    }
    for stream in &professional_streams {
        let id = node(stream)?;
        ensure!(
            seen.insert(id.clone()),
            "duplicate final professional node identity"
        );
        let matching: Vec<_> = streams
            .iter()
            .filter(|candidate| candidate.get("node") == stream.get("node"))
            .collect();
        ensure!(
            matching.len() == 1,
            "final native/professional node mismatch"
        );
        let observed = matching[0];
        let kind = observed
            .get("kind")
            .and_then(Value::as_str)
            .context("final native stream kind missing")?;
        let (input, is_master, virtual_stream) = match kind {
            "master_output" => (false, true, false),
            "follower_output" => (false, false, false),
            "physical_input" => (true, false, false),
            "process_input" => (true, false, true),
            "virtual_playback" => (true, false, true),
            "virtual_microphone" => (false, false, true),
            _ => anyhow::bail!("unknown final native stream kind"),
        };
        if route == super::vad_route::RouteKind::Microphone {
            ensure!(
                matches!(kind, "virtual_playback" | "virtual_microphone"),
                "microphone route cannot contain physical/master/process streams"
            );
        }
        ensure!(
            flag(stream, "input")? == input && flag(stream, "master")? == is_master,
            "final graph native/professional stream shape mismatch"
        );
        let actual = stream
            .get("actual")
            .context("final stream actual format missing")?;
        let rate = number(actual, "sample_rate")?;
        let channels = number(actual, "channels")?;
        let buffer = number(actual, "buffer_frames")?;
        ensure!(
            (8_000..=384_000).contains(&rate) && (1..=32).contains(&channels),
            "final stream negotiated sample rate/channels invalid"
        );
        let callbacks = number(stream, "native_callbacks")?;
        let written_frames = number(stream, "native_written_frames")?;
        number(stream, "native_wait_wakes")?;
        if virtual_stream {
            ensure!(
                buffer == 0 && observed.get("timing").is_some_and(Value::is_null),
                "virtual stream cannot claim a physical native buffer/timing"
            );
        } else {
            ensure!(
                (1..=65536).contains(&buffer),
                "final physical stream buffer invalid"
            );
            let timing = observed
                .get("timing")
                .context("final physical stream timing missing")?;
            ensure!(
                number(timing, "sample_rate")? == rate
                    && number(timing, "capacity_frames")? == buffer,
                "final physical timing disagrees with negotiated format"
            );
            match timing.get("period_frames") {
                Some(Value::Null) => {}
                Some(period) => ensure!(
                    period
                        .as_u64()
                        .is_some_and(|frames| frames > 0 && frames <= u64::from(u32::MAX)),
                    "final physical period malformed"
                ),
                None => anyhow::bail!("final physical period availability missing"),
            }
        }
        if is_master {
            // WASAPI publishes writes; ASIO publishes callbacks. Neither
            // backend's unimplemented counter is evidence of stalled output.
            ensure!(
                master.is_none() && (written_frames > 0 || callbacks > 0),
                "final physical master missing writes/callbacks or duplicated"
            );
            master = Some(id.clone());
        }
        if kind == "virtual_playback" {
            virtual_inputs.insert(id.clone());
        }
        if kind == "virtual_microphone" {
            virtual_outputs.insert(id.clone());
        }
        for key in [
            "dropped_frames",
            "silent_frames",
            "native_dropped_frames",
            "native_silent_frames",
            "native_wait_timeouts",
        ] {
            let count = number(stream, key)?;
            if count != 0 {
                soft.push(format!("final professional {id} {key}: {count}"));
            }
        }
    }
    match route {
        super::vad_route::RouteKind::Physical => {
            master.context("final graph physical master output missing")?;
            ensure!(
                virtual_outputs.is_empty(),
                "physical route cannot carry virtual microphone output"
            );
        }
        super::vad_route::RouteKind::Microphone => ensure!(
            streams.len() == 2
                && master.is_none()
                && virtual_inputs.len() == 1
                && virtual_outputs.len() == 1,
            "microphone route requires exactly one virtual playback and microphone"
        ),
    }
    ensure!(
        !virtual_inputs.is_empty(),
        "final graph VirtualPlayback input missing"
    );
    // VAD normally has no clock-bridge entries: its carrier evidence is the
    // boundary probe. Validate every bridge that is actually present instead
    // of inventing a render timeout or requiring Process-only bridge evidence.
    let mut bridge_nodes = std::collections::BTreeSet::new();
    for bridge in array(native, "bridges")? {
        let id = node(&bridge)?;
        ensure!(
            seen.contains(&id) && bridge_nodes.insert(id.clone()),
            "final bridge identity unknown or duplicated"
        );
        let counters = bridge
            .get("counters")
            .context("final native bridge counters missing")?;
        for key in [
            "captured_frames",
            "written_frames",
            "dropped_frames",
            "silent_frames",
            "nonfinite_samples",
            "discontinuities",
            "timestamp_errors",
            "last_device_position_frames",
            "last_qpc_100ns",
            "output_frames",
            "priming_frames",
            "prime_discarded_frames",
            "backlog_discarded_frames",
            "backlog_recoveries",
            "age_discarded_frames",
            "age_recoveries",
            "low_water_guard_frames",
            "underrun_frames",
            "last_underrun_at_output_frame",
            "resets",
            "fill_frames",
            "min_fill_frames",
            "max_fill_frames",
        ] {
            let count = number(counters, key)?;
            if matches!(key, "nonfinite_samples" | "resets") {
                ensure!(count == 0, "final native bridge {id} {key}: {count}");
            } else if count != 0
                && matches!(
                    key,
                    "dropped_frames"
                        | "silent_frames"
                        | "discontinuities"
                        | "timestamp_errors"
                        | "backlog_discarded_frames"
                        | "backlog_recoveries"
                        | "age_discarded_frames"
                        | "age_recoveries"
                        | "underrun_frames"
                )
            {
                soft.push(format!("final native bridge {id} {key}: {count}"));
            }
        }
        flag(counters, "last_underrun_producer_finished")?;
        ensure!(
            counters
                .get("correction_ppm")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite),
            "final native bridge correction nonfinite or missing"
        );
        let delivery = counters
            .get("native_delivery")
            .context("final bridge delivery availability missing")?;
        if !delivery.is_null() {
            for key in [
                "packets",
                "max_packet_frames",
                "max_callback_burst_frames",
                "gap_samples",
                "max_gap_100ns",
                "packet_bound_violations",
                "gap_bound_violations",
                "unavailable_clocks",
                "regressed_clocks",
            ] {
                let count = number(delivery, key)?;
                if count != 0
                    && matches!(
                        key,
                        "packet_bound_violations"
                            | "gap_bound_violations"
                            | "unavailable_clocks"
                            | "regressed_clocks"
                    )
                {
                    soft.push(format!("final native delivery {id} {key}: {count}"));
                }
            }
        }
    }
    let mut probes = std::collections::BTreeSet::new();
    for probe in array(native, "boundary_probes")? {
        let id = node(&probe)?;
        ensure!(
            seen.contains(&id) && probes.insert(id.clone()),
            "final boundary probe identity unknown or duplicated"
        );
        let observed = streams
            .iter()
            .find(|stream| stream.get("node") == probe.get("node"))
            .context("final boundary stream missing")?;
        ensure!(
            probe.get("kind") == observed.get("kind")
                && matches!(
                    probe.get("kind").and_then(Value::as_str),
                    Some("virtual_playback" | "virtual_microphone")
                ),
            "final boundary probe kind mismatch"
        );
        let capacity = number(&probe, "capacity_frames")?;
        ensure!(
            capacity > 0
                && capacity <= u64::from(u32::MAX)
                && number(&probe, "level_frames")? <= capacity,
            "final boundary capacity/occupancy invalid"
        );
        match probe.get("target_frames") {
            Some(Value::Null) => {}
            Some(target) => ensure!(
                target
                    .as_u64()
                    .is_some_and(|frames| frames > 0 && frames <= capacity),
                "final boundary target malformed"
            ),
            None => anyhow::bail!("final boundary target availability missing"),
        }
        ensure!(
            probe
                .get("applied_milli_ppm")
                .and_then(Value::as_i64)
                .is_some_and(|ppm| i32::try_from(ppm).is_ok()),
            "final boundary correction missing or malformed"
        );
        for key in ["xruns", "soft_events"] {
            let count = number(&probe, key)?;
            let classified_playback = quality.playback_boundary.is_some()
                && probe.get("kind").and_then(Value::as_str) == Some("virtual_playback");
            if count != 0 && !(key == "soft_events" && classified_playback) {
                soft.push(format!("final boundary {id} {key}: {count}"));
            }
        }
    }
    ensure!(
        virtual_inputs.is_subset(&probes) && virtual_outputs.is_subset(&probes),
        "final virtual playback/microphone boundary probe missing"
    );
    if route == super::vad_route::RouteKind::Microphone {
        ensure!(
            probes.len() == 2 && bridge_nodes.is_empty(),
            "pure virtual microphone route requires two boundary probes and no physical clock bridges"
        );
    }
    let mut renders = std::collections::BTreeSet::new();
    for render in array(native, "render_padding")? {
        let id = node(&render)?;
        let output = streams
            .iter()
            .find(|stream| stream.get("node") == render.get("node"));
        ensure!(
            output.is_some_and(|stream| matches!(
                stream.get("kind").and_then(Value::as_str),
                Some("master_output" | "follower_output")
            )) && renders.insert(id),
            "final render padding identity unknown or duplicated"
        );
        let sample = render
            .get("sample")
            .context("final render padding availability missing")?;
        if !sample.is_null() {
            let capacity = number(sample, "capacity_frames")?;
            ensure!(
                capacity > 0
                    && capacity <= u64::from(u32::MAX)
                    && number(sample, "padding_frames")? <= capacity
                    && number(sample, "sequence")? > 0,
                "final render padding sample invalid"
            );
            for key in ["sample_rate", "period_frames"] {
                match sample.get(key) {
                    Some(Value::Null) => {}
                    Some(value) => ensure!(
                        value
                            .as_u64()
                            .is_some_and(|number| number > 0 && number <= u64::from(u32::MAX)),
                        "final render padding {key} malformed"
                    ),
                    None => anyhow::bail!("final render padding {key} availability missing"),
                }
            }
        }
    }
    // Validate every structural/hard field before returning on a soft gate.
    // The raw report stays intact, including explicit unavailable padding.
    if let Some(tracker) = &mut quality.playback_boundary {
        soft.extend(tracker.observe(native, true)?);
    }
    for message in soft {
        let first = quality.first_observations.is_none();
        quality.record(&message, None, None);
        if first {
            quality.first_observations = Some(serde_json::json!({"native_summary": summary}));
        }
        ensure!(policy.is_correctness(), "{message}");
    }
    Ok(())
}

#[cfg(any(windows, test))]
impl Phases {
    fn recorder_start_deadline(
        &self,
        activated: std::time::Instant,
        policy: ValidationPolicy,
    ) -> Result<std::time::Instant> {
        let budget = Duration::from_secs(5)
            .saturating_add(policy.progress_grace())
            .min(self.ceiling);
        activated
            .checked_add(budget)
            .context("recorder Start deadline overflow")
    }

    fn admit_recorder(&mut self, now: Duration) -> Result<()> {
        ensure!(
            self.recorder_ready.is_none() && self.source_requested.is_none(),
            "initial recorder admission may occur only once before source activation"
        );
        ensure!(
            now < self.ceiling,
            "recorder admission exceeded collection deadline"
        );
        self.recorder_ready = Some(now);
        self.stamp("recorder_running", now);
        Ok(())
    }

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
    #[cfg(test)]
    fn tick(
        &mut self,
        now: Duration,
        recorder_running: bool,
        recorder_deadline: bool,
        source: Option<&PlaybackProgress>,
    ) -> Result<Action> {
        self.tick_with_policy(
            now,
            recorder_running,
            recorder_deadline,
            source,
            ValidationPolicy::default(),
            &mut Quality::default(),
        )
    }

    fn tick_with_policy(
        &mut self,
        now: Duration,
        recorder_running: bool,
        recorder_deadline: bool,
        source: Option<&PlaybackProgress>,
        policy: ValidationPolicy,
        quality: &mut Quality,
    ) -> Result<Action> {
        ensure!(
            !recorder_deadline && now < self.ceiling,
            "incomplete collection: recorder deadline"
        );
        if self.recorder_ready.is_none() {
            progress_deadline(
                now,
                Duration::from_secs(5),
                "recorder Start deadline",
                policy,
                quality,
            )?;
            if recorder_running {
                self.admit_recorder(now)?;
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
                progress_deadline(
                    age,
                    Duration::from_secs(5),
                    "source Start deadline",
                    policy,
                    quality,
                )?;
                if source.is_some_and(|source| source.started) {
                    self.source_started = true;
                    self.stamp("source_started_observed", now);
                }
            }
            if self.completed.is_none() {
                progress_deadline(
                    age,
                    Duration::from_secs(7).saturating_add(self.source_duration),
                    "source completion deadline",
                    policy,
                    quality,
                )?;
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

#[cfg(any(windows, test))]
fn progress_deadline(
    age: Duration,
    deadline: Duration,
    message: &str,
    policy: ValidationPolicy,
    quality: &mut Quality,
) -> Result<()> {
    if age >= deadline {
        quality.record(message, None, None);
    }
    ensure!(
        age < deadline.saturating_add(policy.progress_grace()),
        "{message}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn correctness() -> super::super::validation_policy::ValidationPolicy {
        super::super::validation_policy::ValidationPolicy {
            mode: super::super::validation_policy::ValidationMode::Correctness,
            jitter_tolerance_ms: 3000,
        }
    }

    pub(super) fn raw_health_report() -> super::super::NativeReport {
        let mut native = final_schema2()["native"].clone();
        native["running"] = true.into();
        native["bridges"] = serde_json::json!([{
            "node": "source", "counters": moiren_windows_audio::clock_bridge::BridgeSnapshot::default()
        }]);
        super::super::NativeReport {
            vad_provider: Some(serde_json::json!({
                "telemetry": {"epoch_resets": 0, "silence_frames": 0},
                "snapshot": {"packet_frames": 480, "silence_packets": 0}
            })),
            engine: Some(serde_json::json!({"observations": native})),
            ..Default::default()
        }
    }

    #[test]
    fn engine_health_correctness_progress_grace_keeps_quality_failure_after_recovery() {
        let admitted = healthy();
        let mut current = admitted.clone();
        current.engine_progress_overdue = true;
        let report = raw_health_report();
        let mut quality = Quality::default();
        let mut first_failure = None;
        validate_health_with_policy(
            &current,
            &admitted,
            &report,
            &mut first_failure,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(
            quality
                .failure
                .as_deref()
                .unwrap()
                .contains("Engine progress base deadline")
        );
        current.engine_progress_overdue = false;
        validate_health_with_policy(
            &current,
            &admitted,
            &report,
            &mut first_failure,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(quality.failure.is_some());
        current.engine_progress_overdue = true;
        assert!(
            validate_health_with_policy(
                &current,
                &admitted,
                &report,
                &mut first_failure,
                ValidationPolicy::default(),
                &mut quality,
            )
            .is_err()
        );
    }

    #[test]
    fn correctness_health_accepts_real_vad_boundary_evidence_without_clock_bridges() {
        let admitted = healthy();
        let mut report = raw_health_report();
        report.engine.as_mut().unwrap()["observations"]["bridges"] = serde_json::json!([]);
        let original = report.engine.clone();
        let mut quality = Quality::default();
        let mut first_failure = None;
        validate_health_with_policy(
            &admitted,
            &admitted,
            &report,
            &mut first_failure,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(quality.failure.is_none());
        assert!(first_failure.is_none());
        assert_eq!(report.engine, original);

        let mut current = admitted.clone();
        current.quality_faults = 3;
        report.engine.as_mut().unwrap()["observations"]["boundary_probes"][0]["xruns"] = 3.into();
        validate_health_with_policy(
            &current,
            &admitted,
            &report,
            &mut first_failure,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(quality.failure.is_some());
        assert!(first_failure.is_none());
        let first = quality.first_observations.as_ref().unwrap();
        assert_eq!(
            first["engine"]["observations"]["bridges"],
            serde_json::json!([])
        );
        assert_eq!(
            first["engine"]["observations"]["boundary_probes"][0]["xruns"],
            3
        );
        assert!(validate_health(&current, &admitted, &report, &mut first_failure).is_err());
    }

    #[test]
    fn correctness_health_empty_bridges_do_not_hide_missing_or_malformed_vad_probes() {
        let admitted = healthy();
        for variant in 0..16 {
            let mut report = raw_health_report();
            let native = &mut report.engine.as_mut().unwrap()["observations"];
            native["bridges"] = serde_json::json!([]);
            match variant {
                0 => {
                    native.as_object_mut().unwrap().remove("boundary_probes");
                }
                1 => native["boundary_probes"] = serde_json::json!([]),
                2 => native["boundary_probes"] = serde_json::json!("unavailable"),
                3 => native["boundary_probes"][0]["kind"] = "physical_input".into(),
                4 => native["boundary_probes"][0]["node"] = "different-source".into(),
                5 => native["boundary_probes"][0]["capacity_frames"] = 0.into(),
                6 => native["boundary_probes"][0]["level_frames"] = 1025.into(),
                7 => native["boundary_probes"][0]["target_frames"] = 1025.into(),
                8 => {
                    native["boundary_probes"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("xruns");
                }
                9 => native["boundary_probes"][0]["xruns"] = serde_json::Value::Null,
                10 => native["boundary_probes"][0]["soft_events"] = (-1).into(),
                11 => native["boundary_probes"][0]["applied_milli_ppm"] = "NaN".into(),
                12 => native["streams"][1]["kind"] = "physical_input".into(),
                13 => native["streams"] = serde_json::Value::Null,
                14 => {
                    let duplicate = native["boundary_probes"][0].clone();
                    native["boundary_probes"]
                        .as_array_mut()
                        .unwrap()
                        .push(duplicate);
                }
                _ => {
                    report.vad_provider.as_mut().unwrap()["snapshot"]["packet_frames"] = 479.into()
                }
            }
            let original = report.engine.clone();
            let mut first_failure = None;
            assert!(
                validate_health_with_policy(
                    &admitted,
                    &admitted,
                    &report,
                    &mut first_failure,
                    correctness(),
                    &mut Quality::default(),
                )
                .is_err(),
                "variant {variant}"
            );
            assert!(first_failure.is_some(), "variant {variant}");
            assert_eq!(report.engine, original);
        }
    }

    #[test]
    fn playback_sequence_holes_continue_correctness_but_fail_strict_without_signal_loss() {
        let admitted = healthy();
        let mut current = admitted.clone();
        let mut report = raw_health_report();
        let telemetry = &mut report.vad_provider.as_mut().unwrap()["telemetry"];
        telemetry["sequence_gap_observations"] = 2.into();
        telemetry["missing_sequence_slots"] = 5.into();
        current.quality_faults = super::super::native_quality(&report).delivery_bound_violations;
        assert_eq!(current.quality_faults, 5);
        let mut quality = Quality::default();
        let mut first_failure = None;
        validate_health_with_policy(
            &current,
            &admitted,
            &report,
            &mut first_failure,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(first_failure.is_none());
        assert!(quality.failure.is_some());
        let raw = quality.first_observations.unwrap();
        assert_eq!(raw["provider"]["telemetry"]["missing_sequence_slots"], 5);
        assert!(
            validate_health_with_policy(
                &current,
                &admitted,
                &report,
                &mut first_failure,
                ValidationPolicy::default(),
                &mut Quality::default(),
            )
            .is_err()
        );
        assert_eq!(super::super::native_quality(&report).dropped_frames, 0);
    }

    #[test]
    fn correctness_health_retains_soft_faults_without_relaxing_strict_validation() {
        let admitted = healthy();
        let mut current = admitted.clone();
        current.quality_faults = 17;
        let mut report = raw_health_report();
        report.vad_provider.as_mut().unwrap()["telemetry"]["dropped_packets"] = 17.into();
        let mut quality = Quality::default();
        let mut first_failure = None;
        validate_health_with_policy(
            &current,
            &admitted,
            &report,
            &mut first_failure,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(first_failure.is_none());
        assert!(quality.failure.is_some());
        let first = quality.first_observations.clone().unwrap();
        assert_eq!(first["health"]["quality_faults"], 17);
        assert_eq!(first["provider"]["telemetry"]["dropped_packets"], 17);
        current.quality_faults = 23;
        validate_health_with_policy(
            &current,
            &admitted,
            &report,
            &mut first_failure,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert_eq!(quality.first_observations.as_ref(), Some(&first));
        assert!(validate_health(&current, &admitted, &report, &mut first_failure).is_err());
        assert_eq!(
            report.vad_provider.as_ref().unwrap()["telemetry"]["dropped_packets"],
            17
        );
    }

    #[test]
    fn correctness_health_keeps_identity_lifecycle_and_nonfinite_failures_hard() {
        let admitted = healthy();
        for variant in 0..18 {
            let mut current = admitted.clone();
            let mut report = raw_health_report();
            match variant {
                0..=4 => current.stamp.as_mut().unwrap()[variant] += 1,
                5 => current.shape.as_mut().unwrap().2 += 1,
                6 => current.provider_ready = false,
                7 => current.provider_error = Some("QUERY failed".into()),
                8 => current.revision = Some(2),
                9 => current.generations[0] += 1,
                10 => current.engine_running = false,
                11 => current.engine_fault = true,
                12 => current.native_running = false,
                13 => current.quality_known = false,
                14 => {
                    report.engine.as_mut().unwrap()["observations"]["bridges"][0]["counters"]["nonfinite_samples"] =
                        1.into()
                }
                15 => {
                    report.engine.as_mut().unwrap()["observations"]["bridges"][0]["counters"]["resets"] =
                        1.into()
                }
                16 => report.vad_provider.as_mut().unwrap()["telemetry"]["epoch_resets"] = 1.into(),
                _ => report.engine = None,
            }
            let mut first_failure = None;
            assert!(
                validate_health_with_policy(
                    &current,
                    &admitted,
                    &report,
                    &mut first_failure,
                    correctness(),
                    &mut Quality::default(),
                )
                .is_err(),
                "variant {variant}"
            );
            assert!(first_failure.is_some(), "variant {variant}");
        }
    }

    #[test]
    fn correctness_graph_summary_soft_faults_do_not_hide_missing_or_failed_evidence() {
        let summary = serde_json::json!({"failure": null, "render": {
            "failure": null, "stream_started": true, "stop_succeeded": true,
            "stats": {"bridge_shortfall_frames": 42, "timeout_wakes": 3}
        }});
        let mut quality = Quality::default();
        graph_summary_health_with_policy(Some(&summary), correctness(), &mut quality).unwrap();
        assert!(quality.failure.is_some());
        assert!(graph_summary_health(Some(&summary)).is_err());
        for pointer in ["/failure", "/render/failure", "/render/stats/timeout_wakes"] {
            let mut malformed = summary.clone();
            *malformed.pointer_mut(pointer).unwrap() = serde_json::json!("unknown failure");
            assert!(
                graph_summary_health_with_policy(Some(&malformed), correctness(), &mut quality)
                    .is_err()
            );
        }
        assert!(graph_summary_health_with_policy(None, correctness(), &mut quality).is_err());
        assert_eq!(summary["render"]["stats"]["bridge_shortfall_frames"], 42);
    }

    pub(super) fn final_schema2() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 2, "observation_scope": "final_after_owner_join",
            "failure": null, "owner_generation": 7,
            "professional": {"owner_generation": 7, "running": false, "streams": [
                {"node": "output", "input": false, "master": true,
                    "actual": {"sample_rate": 48_000, "channels": 2, "buffer_frames": 480},
                    "native_callbacks": 8, "dropped_frames": 0, "silent_frames": 0,
                    "native_wait_wakes": 8, "native_wait_timeouts": 0, "native_written_frames": 0,
                    "native_dropped_frames": 0, "native_silent_frames": 0},
                {"node": "source", "input": true, "master": false,
                    "actual": {"sample_rate": 48_000, "channels": 2, "buffer_frames": 0},
                    "native_callbacks": 0, "dropped_frames": 0, "silent_frames": 0,
                    "native_wait_wakes": 0, "native_wait_timeouts": 0, "native_written_frames": 0,
                    "native_dropped_frames": 0, "native_silent_frames": 0}
            ]},
            "native": {"owner_generation": 7, "running": false, "streams": [
                {"node": "output", "kind": "master_output", "timing": {
                    "sample_rate": 48_000, "capacity_frames": 480, "period_frames": null},
                    "shared_period": null, "render_queue": null, "input_queue": null,
                    "virtual_budget": null},
                {"node": "source", "kind": "virtual_playback", "timing": null,
                    "shared_period": null, "render_queue": null, "input_queue": null,
                    "virtual_budget": null}
            ], "bridges": [], "boundary_probes": [
                {"node": "source", "kind": "virtual_playback", "capacity_frames": 1024,
                    "target_frames": null, "level_frames": 0, "applied_milli_ppm": 0,
                    "xruns": 0, "soft_events": 0}
            ], "render_padding": [{"node": "output", "sample": null}]}
        })
    }

    fn final_schema2_with_bridge() -> serde_json::Value {
        let mut summary = final_schema2();
        summary["native"]["bridges"] = serde_json::json!([{
            "node": "source", "counters": moiren_windows_audio::clock_bridge::BridgeSnapshot::default()
        }]);
        summary
    }

    #[test]
    fn collection_final_schema2_accepts_joined_physical_master_without_legacy_render_fields() {
        let summary = final_schema2();
        graph_summary_health(Some(&summary)).unwrap();
        let mut quality = Quality::default();
        graph_summary_health_with_policy(Some(&summary), correctness(), &mut quality).unwrap();
        assert!(quality.failure.is_none());
        assert!(summary.get("render").is_none());
        assert_eq!(summary["native"]["bridges"], serde_json::json!([]));
        assert_eq!(
            summary["professional"]["streams"][1]["actual"]["buffer_frames"],
            0
        );
    }

    #[test]
    fn collection_final_schema2_wasapi_writes_succeed_without_asio_callbacks() {
        let mut summary = final_schema2();
        summary["professional"]["streams"][0]["native_callbacks"] = 0.into();
        summary["professional"]["streams"][0]["native_written_frames"] = 3840.into();
        let original = summary.clone();
        graph_summary_health(Some(&summary)).unwrap();
        let mut quality = Quality::default();
        graph_summary_health_with_policy(Some(&summary), correctness(), &mut quality).unwrap();
        assert!(quality.failure.is_none());
        assert_eq!(summary, original);

        summary["professional"]["streams"][0]["native_wait_timeouts"] = 3.into();
        graph_summary_health_with_policy(Some(&summary), correctness(), &mut quality).unwrap();
        assert!(
            quality
                .failure
                .as_ref()
                .unwrap()
                .contains("native_wait_timeouts: 3")
        );
        let first = quality.first_observations.as_ref().unwrap();
        assert_eq!(
            first["native_summary"]["professional"]["streams"][0]["native_callbacks"],
            0
        );
        assert_eq!(
            first["native_summary"]["professional"]["streams"][0]["native_written_frames"],
            3840
        );
        assert_eq!(
            first["native_summary"]["professional"]["streams"][0]["native_wait_timeouts"],
            3
        );
        assert!(graph_summary_health(Some(&summary)).is_err());
    }

    #[test]
    fn collection_final_schema2_asio_callbacks_succeed_without_wasapi_writes() {
        let mut summary = final_schema2();
        summary["professional"]["streams"][0]["native_wait_wakes"] = 0.into();
        assert_eq!(
            summary["professional"]["streams"][0]["native_written_frames"],
            0
        );
        graph_summary_health(Some(&summary)).unwrap();
        let mut quality = Quality::default();
        graph_summary_health_with_policy(Some(&summary), correctness(), &mut quality).unwrap();
        assert!(quality.failure.is_none());
        assert_eq!(summary["professional"]["streams"][0]["native_callbacks"], 8);

        summary["professional"]["streams"][0]["native_callbacks"] = 0.into();
        summary["professional"]["streams"][0]["native_wait_wakes"] = 8.into();
        assert!(graph_summary_health(Some(&summary)).is_err());
        assert!(
            graph_summary_health_with_policy(Some(&summary), correctness(), &mut quality).is_err()
        );
    }

    #[test]
    fn collection_final_schema2_late_quality_counters_are_soft_only_in_correctness() {
        for pointer in [
            "/professional/streams/0/dropped_frames",
            "/professional/streams/0/silent_frames",
            "/professional/streams/0/native_dropped_frames",
            "/professional/streams/0/native_silent_frames",
            "/professional/streams/0/native_wait_timeouts",
            "/professional/streams/1/dropped_frames",
            "/native/boundary_probes/0/xruns",
            "/native/boundary_probes/0/soft_events",
            "/native/bridges/0/counters/dropped_frames",
            "/native/bridges/0/counters/silent_frames",
            "/native/bridges/0/counters/underrun_frames",
            "/native/bridges/0/counters/discontinuities",
            "/native/bridges/0/counters/timestamp_errors",
            "/native/bridges/0/counters/backlog_discarded_frames",
            "/native/bridges/0/counters/age_discarded_frames",
        ] {
            let mut summary = final_schema2_with_bridge();
            *summary.pointer_mut(pointer).unwrap() = 23.into();
            let original = summary.clone();
            let mut quality = Quality::default();
            graph_summary_health_with_policy(Some(&summary), correctness(), &mut quality).unwrap();
            assert!(
                quality.failure.as_ref().unwrap().contains("23"),
                "{pointer}"
            );
            assert_eq!(
                quality.first_observations.as_ref().unwrap()["native_summary"],
                original
            );
            assert!(graph_summary_health(Some(&summary)).is_err(), "{pointer}");
            assert_eq!(summary, original);
        }
    }

    #[test]
    fn collection_final_schema2_hard_shape_and_owner_faults_cannot_hide_behind_xruns() {
        let mut missing_failure = final_schema2();
        missing_failure.as_object_mut().unwrap().remove("failure");
        assert!(
            graph_summary_health_with_policy(
                Some(&missing_failure),
                correctness(),
                &mut Quality::default(),
            )
            .is_err()
        );
        for pointer in [
            "/schema_version",
            "/observation_scope",
            "/owner_generation",
            "/professional/owner_generation",
            "/professional/running",
            "/professional/streams",
            "/professional/streams/0/node",
            "/professional/streams/0/input",
            "/professional/streams/0/master",
            "/professional/streams/0/actual/sample_rate",
            "/professional/streams/0/actual/channels",
            "/professional/streams/0/actual/buffer_frames",
            "/professional/streams/0/native_callbacks",
            "/professional/streams/0/native_wait_wakes",
            "/professional/streams/0/native_wait_timeouts",
            "/professional/streams/0/native_written_frames",
            "/professional/streams/0/dropped_frames",
            "/native/owner_generation",
            "/native/running",
            "/native/streams",
            "/native/streams/0/kind",
            "/native/streams/0/timing",
            "/native/bridges",
            "/native/bridges/0/counters/nonfinite_samples",
            "/native/bridges/0/counters/resets",
            "/native/bridges/0/counters/correction_ppm",
            "/native/boundary_probes",
            "/native/boundary_probes/0/xruns",
            "/native/render_padding",
        ] {
            let mut summary = final_schema2_with_bridge();
            summary["professional"]["streams"][0]["silent_frames"] = 9.into();
            *summary.pointer_mut(pointer).unwrap() = serde_json::Value::Null;
            assert!(
                graph_summary_health_with_policy(
                    Some(&summary),
                    correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "{pointer}"
            );
        }
        for (pointer, value) in [
            ("/schema_version", serde_json::json!(3)),
            ("/failure", serde_json::json!("native Stop failed")),
            ("/observation_scope", serde_json::json!("before_owner_join")),
            ("/owner_generation", serde_json::json!(0)),
            ("/professional/owner_generation", serde_json::json!(8)),
            ("/native/owner_generation", serde_json::json!(8)),
            ("/professional/running", serde_json::json!(true)),
            ("/native/running", serde_json::json!(true)),
            (
                "/professional/streams/0/native_callbacks",
                serde_json::json!(0),
            ),
            (
                "/professional/streams/0/actual/buffer_frames",
                serde_json::json!(0),
            ),
            (
                "/professional/streams/0/actual/sample_rate",
                serde_json::json!(0),
            ),
            (
                "/professional/streams/0/actual/channels",
                serde_json::json!(33),
            ),
            (
                "/native/streams/0/timing/sample_rate",
                serde_json::json!(44_100),
            ),
            (
                "/native/bridges/0/counters/nonfinite_samples",
                serde_json::json!(1),
            ),
            ("/native/bridges/0/counters/resets", serde_json::json!(1)),
            ("/native/boundary_probes/0/xruns", serde_json::json!(-1)),
            (
                "/native/boundary_probes/0/xruns",
                serde_json::json!(u64::MAX),
            ),
            (
                "/professional/streams/0/dropped_frames",
                serde_json::json!(0.5),
            ),
            (
                "/native/boundary_probes/0/capacity_frames",
                serde_json::json!(0),
            ),
        ] {
            let mut summary = final_schema2_with_bridge();
            summary["native"]["boundary_probes"][0]["xruns"] = 9.into();
            *summary.pointer_mut(pointer).unwrap() = value;
            assert!(
                graph_summary_health_with_policy(
                    Some(&summary),
                    correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "{pointer}"
            );
        }
    }

    #[test]
    fn collection_final_schema2_requires_matching_master_and_virtual_probe_identity() {
        for variant in 0..7 {
            let mut summary = final_schema2();
            match variant {
                0 => summary["native"]["streams"][0]["node"] = "different-output".into(),
                1 => summary["native"]["streams"][1]["kind"] = "physical_input".into(),
                2 => summary["native"]["boundary_probes"] = serde_json::json!([]),
                3 => summary["native"]["boundary_probes"][0]["node"] = "different-source".into(),
                4 => summary["professional"]["streams"][1]["node"] = "output".into(),
                5 => summary["native"]["streams"][0]["kind"] = "follower_output".into(),
                _ => summary["native"]["render_padding"][0]["node"] = "different-output".into(),
            }
            assert!(
                graph_summary_health_with_policy(
                    Some(&summary),
                    correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "variant {variant}"
            );
        }
    }

    #[test]
    fn correctness_progress_grace_expires_without_extending_total_ceiling() {
        let mut quality = Quality::default();
        let mut phases = Phases::new(config(), 48_000, 20);
        assert_eq!(
            phases
                .tick_with_policy(
                    Duration::from_secs(5),
                    false,
                    false,
                    None,
                    correctness(),
                    &mut quality
                )
                .unwrap(),
            Action::Wait
        );
        assert!(quality.failure.is_some());
        assert!(
            phases
                .tick_with_policy(
                    Duration::from_secs(8),
                    false,
                    false,
                    None,
                    correctness(),
                    &mut quality
                )
                .is_err()
        );

        let mut phases = Phases::new(config(), 48_000, 20);
        phases
            .tick_with_policy(
                Duration::ZERO,
                true,
                false,
                None,
                correctness(),
                &mut quality,
            )
            .unwrap();
        assert_eq!(
            phases
                .tick_with_policy(
                    Duration::from_millis(100),
                    true,
                    false,
                    None,
                    correctness(),
                    &mut quality
                )
                .unwrap(),
            Action::ActivateSource
        );
        let source = PlaybackProgress {
            started: true,
            ..Default::default()
        };
        phases
            .tick_with_policy(
                Duration::from_millis(5100),
                true,
                false,
                Some(&source),
                correctness(),
                &mut quality,
            )
            .unwrap();
        phases
            .tick_with_policy(
                Duration::from_millis(8100),
                true,
                false,
                Some(&source),
                correctness(),
                &mut quality,
            )
            .unwrap();
        assert!(
            phases
                .tick_with_policy(
                    Duration::from_millis(11100),
                    true,
                    false,
                    Some(&source),
                    correctness(),
                    &mut quality
                )
                .is_err()
        );

        let mut phases = Phases::new(config(), 48_000, 6);
        assert!(
            phases
                .tick_with_policy(
                    Duration::from_secs(6),
                    false,
                    false,
                    None,
                    correctness(),
                    &mut quality
                )
                .is_err()
        );
        let mut phases = Phases::new(config(), 48_000, 20);
        assert!(
            phases
                .tick_with_policy(
                    Duration::ZERO,
                    true,
                    true,
                    None,
                    correctness(),
                    &mut quality
                )
                .is_err()
        );
    }

    #[test]
    fn correctness_quality_diagnostics_are_bounded_and_first_report_is_retained() {
        let report = raw_health_report();
        let mut quality = Quality::default();
        for index in 0..2000 {
            quality.record(
                &format!("observation {index}: {}", "x".repeat(100)),
                Some(&report),
                Some(&healthy()),
            );
        }
        assert!(quality.failure.as_ref().unwrap().len() <= 16 * 1024);
        assert!(
            quality
                .failure
                .as_ref()
                .unwrap()
                .starts_with("observation 0:")
        );
        assert_eq!(
            quality.first_observations.as_ref().unwrap()["health"]["stamp"],
            serde_json::json!([1, 2, 3, 4, 5])
        );
        assert_eq!(report.engine, raw_health_report().engine);
    }

    #[test]
    fn correctness_source_timeout_is_quality_but_stopped_owner_is_hard() {
        let mut source = PlaybackProgress {
            event_timeouts: 2,
            ..Default::default()
        };
        let mut quality = Quality::default();
        validate_source_progress(
            &source,
            correctness(),
            &mut quality,
            Some(&raw_health_report()),
        )
        .unwrap();
        assert!(quality.failure.as_deref().unwrap().contains("timeouts: 2"));
        assert!(
            validate_source_progress(&source, ValidationPolicy::default(), &mut quality, None)
                .is_err()
        );
        source.stopped = true;
        assert!(validate_source_progress(&source, correctness(), &mut quality, None).is_err());
        assert_eq!(source.event_timeouts, 2);
    }

    #[test]
    fn correctness_retirement_soft_health_does_not_become_cleanup_failure() {
        let admitted = healthy();
        let mut final_health = admitted.clone();
        final_health.quality_faults = 9;
        let report = raw_health_report();
        let mut first_failure = None;
        let mut quality = Quality::default();
        let mut cleanup_failure = None;
        let mut events = Vec::new();
        retire(
            |stage| {
                events.push(stage);
                if stage == Retirement::Observe {
                    validate_health_with_policy(
                        &final_health,
                        &admitted,
                        &report,
                        &mut first_failure,
                        correctness(),
                        &mut quality,
                    )?;
                }
                Ok(())
            },
            &mut cleanup_failure,
        );
        assert!(cleanup_failure.is_none());
        assert!(first_failure.is_none());
        assert!(quality.failure.is_some());
        assert_eq!(events.last(), Some(&Retirement::FinishMeasurement));

        retire(
            |stage| {
                if stage == Retirement::FinishMeasurement {
                    anyhow::bail!("storage flush failed");
                }
                Ok(())
            },
            &mut cleanup_failure,
        );
        assert!(cleanup_failure.unwrap().contains("storage flush failed"));
    }

    #[cfg(windows)]
    #[test]
    fn correctness_joined_source_late_timeout_retains_successful_operational_completion() {
        let mut source = super::super::vad::PlaybackOutcome {
            info: None,
            period_request: None,
            submitted_frames: 48_480,
            playback_source: super::super::source::SourceProvenance::new(None),
            source_progress: super::super::source::SourceProgress {
                native_preroll_frames: 480,
                native_submitted_frames: 48_480,
                origin_submission_frame: Some(480),
                acknowledged_source_frames: 48_000,
                schedule_submitted_complete: true,
            },
            event_timeouts: 3,
            elapsed_seconds: 1.3,
            started: true,
            stopped: true,
            worker_joined: true,
            native_stop_attempted: Some(true),
            native_stop_succeeded: Some(true),
            failure: None,
        };
        let mut failures = None;
        let mut quality = Quality::default();
        assert!(reconcile_source_outcome_with_policy(
            Some(&source),
            48_000,
            &mut failures,
            correctness(),
            &mut quality
        ));
        assert!(failures.is_none());
        assert!(quality.failure.as_ref().unwrap().contains("timeouts: 3"));
        assert_eq!(source.event_timeouts, 3);

        source.failure = Some("native Stop error".into());
        reconcile_source_outcome_with_policy(
            Some(&source),
            48_000,
            &mut failures,
            correctness(),
            &mut quality,
        );
        assert!(failures.unwrap().contains("native Stop error"));
        assert_eq!(source.failure.as_deref(), Some("native Stop error"));

        source.failure = None;
        source.source_progress.acknowledged_source_frames -= 1;
        let mut failures = None;
        assert!(!reconcile_source_outcome_with_policy(
            Some(&source),
            48_000,
            &mut failures,
            correctness(),
            &mut quality
        ));
        assert!(failures.is_some());
    }

    #[cfg(windows)]
    fn measurement_outcome() -> crate::measurement::session::MeasurementOutcome {
        use crate::measurement::session::{MeasurementInfo, MeasurementOutcome};
        use crate::measurement_sink::SinkReport;
        use moiren_windows_audio::{
            SessionDuration,
            capture::{CaptureReport, CaptureSource, PcmEvidenceSummary},
        };
        let mut native = CaptureReport::new(CaptureSource::Physical, SessionDuration::UntilStopped);
        native.stream_started = true;
        native.stop_succeeded = true;
        native.evidence_metadata = Some(PcmEvidenceSummary {
            sample_rate: 48_000,
            channels: 2,
            packets: 1,
            observed_frames: 2,
            stored_frames: 2,
            ..Default::default()
        });
        MeasurementOutcome {
            info: MeasurementInfo {
                input_endpoint_id: "independent".into(),
                sample_rate: 48_000,
                channels: 2,
                ring_frames: 8,
                max_frames: 48_000,
            },
            native_capture: Some(native),
            sink: Some(SinkReport {
                schema_version: 1,
                sample_rate: 48_000,
                channels: 2,
                written_frames: 2,
                written_pcm_bytes: 16,
                written_metadata_bytes: 180,
                complete_packets: 1,
                sequence_gaps: 0,
                missing_packets: 0,
                incomplete_packets: 0,
                nonfinite_samples: 0,
                counters_saturated: false,
                cancelled: false,
                failure: None,
            }),
            activation_elapsed: Some(Duration::from_secs(1)),
            failure: None,
        }
    }

    #[cfg(windows)]
    #[test]
    fn correctness_measurement_continuity_retains_raw_outcome_without_operational_failure() {
        let mut outcome = measurement_outcome();
        let summary = outcome
            .native_capture
            .as_mut()
            .unwrap()
            .evidence_metadata
            .as_mut()
            .unwrap();
        summary.dropped_frames = 7;
        summary.observed_frames = 9;
        summary.overflow_packets = 1;
        summary.packets = 2;
        outcome.failure = Some(
            concat!(
                "measurement evidence has loss or invalid continuity/timestamps; ",
                "measurement sink and final native evidence are not complete and consistent",
            )
            .into(),
        );
        let mut quality = Quality::default();
        validate_measurement_outcome_with_policy(&outcome, correctness(), &mut quality).unwrap();
        assert!(quality.failure.is_some());
        assert_eq!(
            quality.first_observations.as_ref().unwrap()["measurement"]["native_capture"]["evidence_metadata"]
                ["dropped_frames"],
            7
        );
        assert!(
            validate_measurement_outcome_with_policy(
                &outcome,
                ValidationPolicy::default(),
                &mut quality
            )
            .is_err()
        );
        assert_eq!(
            outcome.failure.as_deref(),
            Some(concat!(
                "measurement evidence has loss or invalid continuity/timestamps; ",
                "measurement sink and final native evidence are not complete and consistent",
            ))
        );
        assert_eq!(
            outcome
                .native_capture
                .as_ref()
                .unwrap()
                .evidence_metadata
                .unwrap()
                .dropped_frames,
            7
        );
    }

    #[cfg(windows)]
    #[test]
    fn correctness_measurement_soft_fault_cannot_conceal_storage_owner_or_missing_evidence() {
        for variant in 0..19 {
            let mut outcome = measurement_outcome();
            outcome
                .native_capture
                .as_mut()
                .unwrap()
                .evidence_metadata
                .as_mut()
                .unwrap()
                .gap_events = 1;
            outcome.failure =
                Some("measurement evidence has loss or invalid continuity/timestamps".into());
            match variant {
                0 => outcome.activation_elapsed = None,
                1 => outcome.native_capture.as_mut().unwrap().stop_succeeded = false,
                2 => {
                    outcome.native_capture.as_mut().unwrap().failure =
                        Some("native owner failed".into())
                }
                3 => outcome.sink.as_mut().unwrap().failure = Some("PCM flush failed".into()),
                4 => outcome.sink.as_mut().unwrap().nonfinite_samples = 1,
                5 => outcome.sink.as_mut().unwrap().cancelled = true,
                6 => outcome.sink.as_mut().unwrap().written_frames = 1,
                7 => outcome.sink.as_mut().unwrap().missing_packets = 1,
                8 => outcome.sink.as_mut().unwrap().incomplete_packets = 1,
                9 => outcome.sink.as_mut().unwrap().complete_packets = 0,
                10 => outcome.sink.as_mut().unwrap().sample_rate = 44_100,
                11 => outcome.native_capture.as_mut().unwrap().evidence_metadata = None,
                12 => outcome
                    .failure
                    .as_mut()
                    .unwrap()
                    .push_str("; native stop failed"),
                13 => outcome.failure = Some("unknown owner failure".into()),
                14 => {
                    outcome
                        .native_capture
                        .as_mut()
                        .unwrap()
                        .evidence_metadata
                        .as_mut()
                        .unwrap()
                        .position_overflow_packets = 1
                }
                15 => {
                    outcome
                        .native_capture
                        .as_mut()
                        .unwrap()
                        .evidence_metadata
                        .as_mut()
                        .unwrap()
                        .sequence_exhausted = true
                }
                16 => {
                    outcome
                        .native_capture
                        .as_mut()
                        .unwrap()
                        .evidence_metadata
                        .as_mut()
                        .unwrap()
                        .reader_abandoned_packets = 1
                }
                17 => outcome.native_capture = None,
                _ => outcome.sink = None,
            }
            assert!(
                validate_measurement_outcome_with_policy(
                    &outcome,
                    correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "variant {variant}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn correctness_whole_packet_ring_loss_is_soft_but_unexplained_persistence_loss_is_hard() {
        let mut outcome = measurement_outcome();
        let summary = outcome
            .native_capture
            .as_mut()
            .unwrap()
            .evidence_metadata
            .as_mut()
            .unwrap();
        summary.packets = 2;
        summary.observed_frames = 9;
        summary.dropped_frames = 7;
        summary.overflow_packets = 1;
        outcome.sink.as_mut().unwrap().missing_packets = 1;
        outcome.sink.as_mut().unwrap().sequence_gaps = 1;
        outcome.failure = Some(
            concat!(
                "measurement evidence has loss or invalid continuity/timestamps; ",
                "measurement sink and final native evidence are not complete and consistent",
            )
            .into(),
        );
        let mut quality = Quality::default();
        validate_measurement_outcome_with_policy(&outcome, correctness(), &mut quality).unwrap();
        assert!(quality.failure.is_some());
        let retained = quality.first_observations.clone();
        outcome.sink.as_mut().unwrap().written_frames = 1;
        assert!(
            validate_measurement_outcome_with_policy(&outcome, correctness(), &mut quality)
                .is_err()
        );
        outcome.sink.as_mut().unwrap().written_frames = 2;
        outcome.sink.as_mut().unwrap().complete_packets = 0;
        assert!(
            validate_measurement_outcome_with_policy(&outcome, correctness(), &mut quality)
                .is_err()
        );
        assert_eq!(quality.first_observations, retained);
    }

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
            period_request: None,
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
    fn microphone_route_collection_queue_is_inapplicable_without_parsing_physical_policy() {
        let mut options = super::super::LiveOptions::default();
        assert!(collection_render_queue(&options).unwrap().is_some());
        options.vad_microphone_id = Some("fedcba9876543210fedcba9876543210".into());
        assert!(collection_render_queue(&options).unwrap().is_none());
    }

    #[test]
    fn microphone_route_prepared_recorder_pins_exact_selected_id() {
        pin_recorder_endpoint("Exact-ID", "Exact-ID").unwrap();
        for actual in ["exact-id", "default", "Exact-ID ", "", "bad\0id"] {
            assert!(
                pin_recorder_endpoint("Exact-ID", actual).is_err(),
                "{actual:?}"
            );
        }
    }

    #[test]
    fn microphone_route_post_preparation_resolution_pins_full_identity() {
        let selected = serde_json::json!({
            "stable_endpoint_id": "stable", "mmdevice_id": "Exact-ID",
            "observed_name": "Owned microphone", "adapter_instance": "adapter",
            "root_instance": "root",
        });
        pin_microphone_identity(&selected, &selected).unwrap();
        for key in [
            "stable_endpoint_id",
            "mmdevice_id",
            "observed_name",
            "adapter_instance",
            "root_instance",
        ] {
            let mut changed = selected.clone();
            changed[key] = "changed".into();
            assert!(
                pin_microphone_identity(&selected, &changed).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn microphone_route_schema2_resolves_omitted_input_without_loosening_schema1() {
        let mut value = serde_json::to_value(config()).unwrap();
        value["schema_version"] = 2.into();
        value.as_object_mut().unwrap().remove("input_endpoint_id");
        let parsed = serde_json::from_value::<Config>(value);
        assert!(
            parsed.is_ok(),
            "schema2 omitted recorder ID must deserialize"
        );
        let mut parsed = parsed.unwrap();
        parsed.validate(48_000, 14).unwrap();
        parsed
            .validate_route(super::super::vad_route::RouteKind::Microphone)
            .unwrap();
        assert!(
            parsed
                .validate_route(super::super::vad_route::RouteKind::Physical)
                .is_err()
        );
        assert_eq!(
            parsed.recorder_endpoint(Some("exact-mmdevice")).unwrap(),
            "exact-mmdevice"
        );
        assert!(parsed.recorder_endpoint(None).is_err());
        parsed.schema_version = 1;
        assert!(parsed.validate(48_000, 14).is_err());
        assert!(
            parsed
                .validate_route(super::super::vad_route::RouteKind::Microphone)
                .is_err()
        );
    }

    #[test]
    fn microphone_route_recorder_override_requires_exact_discovered_identity() {
        let mut value = config();
        value.input_endpoint_id = "Exact-MMDevice".into();
        assert_eq!(
            value.recorder_endpoint(Some("Exact-MMDevice")).unwrap(),
            "Exact-MMDevice"
        );
        for resolved in ["exact-mmdevice", " Exact-MMDevice", "default", "", "\0"] {
            assert!(
                value.recorder_endpoint(Some(resolved)).is_err(),
                "{resolved:?}"
            );
        }
        value.schema_version = 2;
        value.input_endpoint_id.clear();
        for resolved in ["", " ", "bad\0id"] {
            assert!(value.recorder_endpoint(Some(resolved)).is_err());
        }
    }

    fn microphone_health() -> Health {
        Health {
            route: super::super::vad_route::RouteKind::Microphone,
            microphone: Some(EndpointHealth {
                stamp: Some([11, 12, 13, 14, 15]),
                shape: Some(("Stereo".into(), "Some(Float32)".into(), 480)),
                stream_running: true,
                provider_ready: true,
                quality_known: true,
                ..Default::default()
            }),
            ..healthy()
        }
    }

    #[test]
    fn microphone_route_health_requires_both_endpoints_and_stable_generations() {
        let admitted = microphone_health();
        admitted.validate(&admitted).unwrap();
        for variant in 0..10 {
            let mut current = admitted.clone();
            match variant {
                0 => current.microphone = None,
                1 => current.microphone.as_mut().unwrap().stamp = None,
                2 => current.microphone.as_mut().unwrap().stamp.as_mut().unwrap()[3] += 1,
                3 => current.microphone.as_mut().unwrap().shape = None,
                4 => {
                    current
                        .microphone
                        .as_mut()
                        .unwrap()
                        .shape
                        .as_mut()
                        .unwrap()
                        .2 += 1
                }
                5 => current.microphone.as_mut().unwrap().provider_ready = false,
                6 => {
                    current.microphone.as_mut().unwrap().provider_error =
                        Some("query failed".into())
                }
                7 => current.microphone.as_mut().unwrap().quality_known = false,
                8 => current.microphone.as_mut().unwrap().quality_faults = 1,
                _ => current.route = super::super::vad_route::RouteKind::Physical,
            }
            assert!(current.validate(&admitted).is_err(), "variant {variant}");
        }
        let mut physical = healthy();
        physical.microphone = admitted.microphone;
        assert!(physical.require_present().is_err());
    }

    #[test]
    fn microphone_route_same_stamp_pause_after_admission_is_hard_in_both_policies() {
        let admitted = microphone_health();
        let report = microphone_health_report();
        let mut paused = admitted.clone();
        paused.microphone.as_mut().unwrap().stream_running = false;
        assert_eq!(
            paused.microphone.as_ref().unwrap().stamp,
            admitted.microphone.as_ref().unwrap().stamp
        );
        assert_eq!(
            paused.microphone.as_ref().unwrap().shape,
            admitted.microphone.as_ref().unwrap().shape
        );
        let mut accepted_pause = Vec::new();
        for policy in [ValidationPolicy::default(), correctness()] {
            validate_health_with_policy(
                &admitted,
                &admitted,
                &report,
                &mut None,
                policy,
                &mut Quality::default(),
            )
            .unwrap();
            let mut first_failure = None;
            let result = validate_health_with_policy(
                &paused,
                &admitted,
                &report,
                &mut first_failure,
                policy,
                &mut Quality::default(),
            );
            if result.is_ok() {
                accepted_pause.push(policy);
            } else {
                assert_eq!(
                    first_failure.unwrap()["health"]["microphone"]["stream_running"],
                    false
                );
            }
        }
        assert!(
            accepted_pause.is_empty(),
            "same-stamp Pause accepted by {accepted_pause:?}"
        );
        healthy().require_present().unwrap();
    }

    fn microphone_final_summary() -> serde_json::Value {
        let mut summary = final_schema2();
        summary["native"]["virtual_clock"] = serde_json::json!({
            "schema_version": 1, "engine_sample_rate": 48000,
            "engine_block_frames": 32, "recovery_cap_periods": 8,
            "rendered_periods": 1000, "rendered_frames": 32000,
            "late_wake_bursts": 4, "clamped_periods": 0, "clamped_frames": 0,
            "timeline_resets": 0, "max_backlog_periods": 2, "counters_saturated": false,
        });
        summary["professional"]["streams"][0]["master"] = false.into();
        summary["professional"]["streams"][0]["actual"]["buffer_frames"] = 0.into();
        summary["native"]["streams"][0]["kind"] = "virtual_microphone".into();
        summary["native"]["streams"][0]["timing"] = serde_json::Value::Null;
        let mut probe = summary["native"]["boundary_probes"][0].clone();
        probe["node"] = "output".into();
        probe["kind"] = "virtual_microphone".into();
        summary["native"]["boundary_probes"]
            .as_array_mut()
            .unwrap()
            .push(probe);
        summary["native"]["render_padding"] = serde_json::json!([]);
        summary
    }

    #[test]
    fn microphone_final_clock_loss_is_retained_and_unknown_or_reset_cannot_pass() {
        let route = super::super::vad_route::RouteKind::Microphone;
        let mut lost = microphone_final_summary();
        lost["native"]["virtual_clock"]["clamped_periods"] = 9.into();
        lost["native"]["virtual_clock"]["clamped_frames"] = 288.into();
        lost["native"]["virtual_clock"]["max_backlog_periods"] = 17.into();
        let mut quality = Quality::default();
        graph_summary_health_with_route(Some(&lost), route, correctness(), &mut quality).unwrap();
        assert!(
            quality
                .failure
                .as_deref()
                .is_some_and(|failure| failure.contains("clock")),
            "final-only virtual clock loss must fail quality even after complete collection"
        );
        assert!(
            graph_summary_health_with_route(
                Some(&lost),
                route,
                ValidationPolicy::default(),
                &mut Quality::default()
            )
            .is_err()
        );
        for variant in 0..5 {
            let mut bad = microphone_final_summary();
            match variant {
                0 => bad["native"]["virtual_clock"] = serde_json::Value::Null,
                1 => bad["native"]["virtual_clock"]["counters_saturated"] = true.into(),
                2 => bad["native"]["virtual_clock"]["timeline_resets"] = 1.into(),
                3 => bad["native"]["virtual_clock"]["rendered_frames"] = 1.into(),
                _ => bad["native"]["virtual_clock"]
                    .as_object_mut()
                    .unwrap()
                    .remove("clamped_frames")
                    .map(|_| ())
                    .unwrap(),
            }
            assert!(
                graph_summary_health_with_route(
                    Some(&bad),
                    route,
                    correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "unknown, inconsistent or reset clock evidence variant {variant}"
            );
        }
    }

    #[test]
    fn microphone_route_final_requires_exact_virtual_pair_and_both_probes() {
        let summary = microphone_final_summary();
        graph_summary_health_with_route(
            Some(&summary),
            super::super::vad_route::RouteKind::Microphone,
            correctness(),
            &mut Quality::default(),
        )
        .unwrap();
        assert!(
            graph_summary_health(Some(&summary)).is_err(),
            "physical master must remain mandatory"
        );
        for variant in 0..6 {
            let mut bad = summary.clone();
            match variant {
                0 => {
                    bad["native"]["boundary_probes"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                1 => bad["native"]["streams"][0]["kind"] = "master_output".into(),
                2 => bad["professional"]["streams"][0]["master"] = true.into(),
                3 => bad["native"]["streams"][0]["kind"] = "virtual_playback".into(),
                4 => bad["native"]["boundary_probes"][1]["level_frames"] = 2048.into(),
                _ => bad["native"]["owner_generation"] = 8.into(),
            }
            assert!(
                graph_summary_health_with_route(
                    Some(&bad),
                    super::super::vad_route::RouteKind::Microphone,
                    correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "variant {variant}"
            );
        }
        assert!(
            graph_summary_health_with_route(
                Some(&final_schema2()),
                super::super::vad_route::RouteKind::Microphone,
                correctness(),
                &mut Quality::default()
            )
            .is_err()
        );
    }

    #[test]
    fn microphone_route_retirement_releases_capture_client_before_endpoint_removal() {
        let mut events = Vec::new();
        let mut failure = None;
        retire_with_route(
            super::super::vad_route::RouteKind::Microphone,
            |stage| {
                events.push(stage);
                anyhow::bail!("injected {stage:?}")
            },
            &mut failure,
        );
        let index = |stage| events.iter().position(|event| *event == stage).unwrap();
        assert!(index(Retirement::StopSource) < index(Retirement::JoinSource));
        assert!(index(Retirement::StopMeasurement) < index(Retirement::JoinSource));
        assert!(index(Retirement::ShutdownHost) < index(Retirement::FinishMeasurement));
        assert!(index(Retirement::Invalidate) < index(Retirement::StopSource));
        assert!(index(Retirement::Invalidate) < index(Retirement::StopMeasurement));
        assert!(index(Retirement::FinishMeasurement) < index(Retirement::Retire));
        assert_eq!(events.len(), 10);
        for stage in events {
            assert!(failure.as_ref().unwrap().contains(&format!("{stage:?}")));
        }
    }

    #[test]
    fn microphone_route_preroll_begins_after_one_time_recorder_admission() {
        let mut phases = Phases::new(config(), 48_000, 14);
        assert_eq!(
            phases
                .tick(Duration::from_millis(100), false, false, None)
                .unwrap(),
            Action::Wait
        );
        phases.admit_recorder(Duration::from_millis(300)).unwrap();
        assert_eq!(
            phases
                .tick(Duration::from_millis(399), true, false, None)
                .unwrap(),
            Action::Wait
        );
        assert!(phases.admit_recorder(Duration::from_millis(399)).is_err());
        assert_eq!(
            phases
                .tick(Duration::from_millis(400), true, false, None)
                .unwrap(),
            Action::ActivateSource
        );
        assert_eq!(phases.recorder_ready, Some(Duration::from_millis(300)));
        assert_eq!(phases.ceiling, Duration::from_secs(14));
    }

    #[test]
    fn microphone_route_recorder_start_deadline_uses_activation_and_smallest_budget() {
        let activated = std::time::Instant::now();
        for (ceiling, policy, budget_ms) in [
            (14, ValidationPolicy::default(), 5000),
            (14, correctness(), 8000),
            (6, correctness(), 6000),
            (3, ValidationPolicy::default(), 3000),
        ] {
            let phases = Phases::new(config(), 48_000, ceiling);
            assert_eq!(
                phases.recorder_start_deadline(activated, policy).unwrap(),
                activated + Duration::from_millis(budget_ms)
            );
        }
    }

    fn microphone_health_report() -> super::super::NativeReport {
        let mut report = raw_health_report();
        report.vad_microphone_provider = report.vad_provider.clone();
        report.engine.as_mut().unwrap()["observations"] =
            microphone_final_summary()["native"].clone();
        report.engine.as_mut().unwrap()["observations"]["running"] = true.into();
        report
    }

    #[test]
    fn microphone_route_correctness_never_relaxes_raw_resets_or_missing_probe() {
        let admitted = microphone_health();
        let report = microphone_health_report();
        validate_health_with_policy(
            &admitted,
            &admitted,
            &report,
            &mut None,
            correctness(),
            &mut Quality::default(),
        )
        .unwrap();
        for variant in 0..5 {
            let mut report = microphone_health_report();
            match variant {
                0 => {
                    report.vad_microphone_provider.as_mut().unwrap()["telemetry"]["epoch_resets"] =
                        1.into()
                }
                1 => report.vad_microphone_provider = None,
                2 => {
                    report.engine.as_mut().unwrap()["observations"]["boundary_probes"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                3 => {
                    report.vad_microphone_provider.as_mut().unwrap()["snapshot"]["packet_frames"] =
                        481.into()
                }
                _ => {
                    report.engine.as_mut().unwrap()["observations"]["boundary_probes"][1]["xruns"] =
                        u64::MAX.into()
                }
            }
            let mut first = None;
            assert!(
                validate_health_with_policy(
                    &admitted,
                    &admitted,
                    &report,
                    &mut first,
                    correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "variant {variant}"
            );
            assert!(first.is_some());
        }
        let mut soft = microphone_health_report();
        soft.engine.as_mut().unwrap()["observations"]["boundary_probes"][1]["xruns"] = 3.into();
        let mut quality = Quality::default();
        validate_health_with_policy(
            &admitted,
            &admitted,
            &soft,
            &mut None,
            correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(
            quality.failure.is_some(),
            "microphone xruns remain quality failures"
        );
        let first = quality.first_observations.unwrap();
        assert!(first.get("microphone_provider").is_some());
    }

    #[test]
    fn collection_config_checks_budget_caps_and_schema() {
        config().validate(48_000, 14).unwrap();
        for variant in 0..12 {
            let mut value = config();
            match variant {
                0 => value.schema_version = 3,
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
            &snapshot[..6],
            &[
                Retirement::Observe,
                Retirement::ObserveFinal,
                Retirement::Invalidate,
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

    pub(super) fn healthy() -> Health {
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
            &events[..6],
            &[
                Retirement::Observe,
                Retirement::ObserveFinal,
                Retirement::Invalidate,
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
                Retirement::Invalidate,
                Retirement::StopSource,
                Retirement::StopMeasurement,
                Retirement::JoinSource,
                Retirement::CloseGate,
                Retirement::ShutdownHost,
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

    #[test]
    fn retirement_intent_precedes_both_native_stops_for_both_routes() {
        use super::super::vad_route::RouteKind;
        for route in [RouteKind::Physical, RouteKind::Microphone] {
            let mut events = Vec::new();
            let mut invalidated = false;
            let mut stops = 0;
            let mut failures = None;
            retire_with_route(
                route,
                |stage| {
                    events.push(stage);
                    match stage {
                        Retirement::Invalidate => invalidated = true,
                        Retirement::StopSource | Retirement::StopMeasurement => {
                            ensure!(invalidated, "native stop before explicit retirement intent");
                            stops += 1;
                        }
                        Retirement::JoinSource => ensure!(stops == 2, "join before both stops"),
                        _ => {}
                    }
                    Ok(())
                },
                &mut failures,
            );
            assert!(failures.is_none(), "{route:?}: {failures:?}");
            assert_eq!(events.len(), 10);
            assert_eq!(events[2], Retirement::Invalidate);
            assert_eq!(
                events
                    .iter()
                    .filter(|stage| **stage == Retirement::Invalidate)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn retirement_intent_errors_and_panics_never_skip_broadcast_or_drain() {
        use super::super::vad_route::RouteKind;
        for route in [RouteKind::Physical, RouteKind::Microphone] {
            let mut expected = vec![
                Retirement::Observe,
                Retirement::ObserveFinal,
                Retirement::Invalidate,
                Retirement::StopSource,
                Retirement::StopMeasurement,
                Retirement::JoinSource,
                Retirement::CloseGate,
                Retirement::ShutdownHost,
            ];
            if route == RouteKind::Microphone {
                expected.extend([Retirement::FinishMeasurement, Retirement::Retire]);
            } else {
                expected.extend([Retirement::Retire, Retirement::FinishMeasurement]);
            }
            for failed_stage in &expected {
                for panics in [false, true] {
                    let mut events = Vec::new();
                    let mut failures = Some("original collection failure".into());
                    retire_with_route(
                        route,
                        |stage| {
                            events.push(stage);
                            if stage == *failed_stage {
                                assert!(!panics, "injected {stage:?} panic");
                                anyhow::bail!("injected {stage:?} failure");
                            }
                            Ok(())
                        },
                        &mut failures,
                    );
                    assert_eq!(
                        events, expected,
                        "{route:?}, {failed_stage:?}, panic={panics}"
                    );
                    let text = failures.unwrap();
                    assert!(text.contains("original collection failure"));
                    assert!(text.contains(&format!("{failed_stage:?}")));
                    assert!(text.contains(if panics { "panicked" } else { "failure" }));
                }
            }
        }
    }
}
