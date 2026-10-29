//! Live Process Loopback / VAD orchestration and evidence handoff.
//!
//! This module owns the probe's control-side lifecycle. PCM is copied into a
//! bounded owner-local recorder and written only after the native owner joins;
//! physical qualification still requires an independent measurement artifact
//! and calibration provenance.

#[cfg(windows)]
use super::controlled_load;
use super::pcm::{PcmAnalysis, PcmSchedule, analyze};
use anyhow::{Context, Result, bail, ensure};
use moiren_windows_audio::latency::qualification::{
    CalibrationProvenance, EvidenceBoundary, EvidenceConfiguration, EvidenceFormat,
    EvidenceRunIdentity, EvidenceSourceKind, QUALIFICATION_SCHEMA_VERSION, QualificationQuality,
    QualificationReport, SourceIdentityEvidence,
};
use moiren_windows_audio::{
    latency::{
        RenderQueuePolicy, TelemetryMode,
        transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    },
    process_loopback::ProcessBridgeOptions,
    virtual_owner::VirtualBufferOptions,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "source.rs"]
mod source;

#[path = "vad_route.rs"]
mod vad_route;

#[path = "render_request.rs"]
mod render_request;
use render_request::PhysicalRenderRequest;

#[cfg(any(windows, test))]
#[path = "process_trace.rs"]
mod process_trace;

#[path = "validation_policy.rs"]
mod validation_policy;
use validation_policy::{ValidationMode, ValidationPolicy};

#[cfg(any(windows, test))]
#[path = "process_admission.rs"]
mod process_admission;

#[path = "process_source.rs"]
#[cfg(any(windows, test))]
pub(crate) mod process_source;

#[path = "vad_collection.rs"]
mod vad_collection;

#[path = "process_collection.rs"]
mod process_collection;

#[cfg(any(windows, test))]
#[path = "render.rs"]
mod render;

#[cfg(any(windows, test))]
#[path = "control_observation.rs"]
mod control_observation;

#[cfg(windows)]
#[path = "vad.rs"]
mod vad;

#[cfg(any(windows, test))]
#[path = "vad_period.rs"]
mod vad_period;

#[cfg(any(windows, test))]
#[path = "soak.rs"]
mod soak;

#[cfg(any(windows, test))]
#[path = "soak_cleanup.rs"]
mod soak_cleanup;
#[cfg(any(windows, test))]
#[path = "soak_control.rs"]
pub(crate) mod soak_control;

const LIVE_SCHEMA_VERSION: u32 = 1;
const DEFAULT_MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_INPUT_CAPACITY_FRAMES: u32 = 8192;
const DEFAULT_INPUT_JITTER_FRAMES: u32 = 96;
const DEFAULT_MAX_PACKET_FRAMES: u32 = 480;
const DEFAULT_MAX_GAP_FRAMES: u32 = 480;
const DEFAULT_ENGINE_BLOCK_FRAMES: u32 = 256;
const DEFAULT_WAKE_MARGIN_FRAMES: u32 = 96;
const DEFAULT_CALLBACK_BUDGET_FRAMES: u32 = 48;

#[derive(Debug, Clone)]
struct LiveOptions {
    validation: ValidationPolicy,
    source_kind: EvidenceSourceKind,
    source_pid: u32,
    source_pid_explicit: bool,
    source_output: Option<String>,
    process_collection: Option<PathBuf>,
    stable_endpoint_id: Option<String>,
    source_name: Option<String>,
    vad_microphone_id: Option<String>,
    vad_microphone_name: Option<String>,
    output_endpoint: String,
    output_endpoint_explicit: bool,
    physical_render_options_explicit: bool,
    physical_render_request: PhysicalRenderRequest,
    input_budget_options_explicit: bool,
    artifact_dir: PathBuf,
    report_path: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    measurement_boundary: EvidenceBoundary,
    measurement_pcm: Option<PathBuf>,
    measurement_schedule: Option<PathBuf>,
    source_schedule: Option<PathBuf>,
    vad_collection: Option<PathBuf>,
    calibration: Option<PathBuf>,
    seconds: u64,
    max_input_bytes: u64,
    input_capacity_frames: u32,
    input_target_frames: Option<u32>,
    input_jitter_frames: u32,
    max_packet_frames: u32,
    max_gap_frames: u32,
    vad_recovery_cap_periods: Option<u64>,
    vad_minimum_period: bool,
    render_target_frames: Option<u32>,
    wake_margin_frames: u32,
    callback_budget_frames: u32,
    engine_block_frames: u32,
    telemetry: TelemetryMode,
    run_id: Option<String>,
    source_revision: Option<String>,
    binary_hash: Option<String>,
    config_hash: Option<String>,
    machine_id: Option<String>,
    windows_build: Option<String>,
    controlled_load_id: Option<String>,
    diagnostic_soak: bool,
    observation_interval_seconds: u64,
    dry_run: bool,
}

impl Default for LiveOptions {
    fn default() -> Self {
        Self {
            validation: ValidationPolicy::default(),
            source_kind: EvidenceSourceKind::ProcessLoopback,
            source_pid: 0,
            source_pid_explicit: false,
            source_output: None,
            process_collection: None,
            stable_endpoint_id: None,
            source_name: None,
            vad_microphone_id: None,
            vad_microphone_name: None,
            output_endpoint: String::new(),
            output_endpoint_explicit: false,
            physical_render_options_explicit: false,
            physical_render_request: PhysicalRenderRequest::default(),
            input_budget_options_explicit: false,
            artifact_dir: PathBuf::new(),
            report_path: None,
            manifest_path: None,
            measurement_boundary: EvidenceBoundary::Internal,
            measurement_pcm: None,
            measurement_schedule: None,
            source_schedule: None,
            vad_collection: None,
            calibration: None,
            seconds: 5,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            input_capacity_frames: DEFAULT_INPUT_CAPACITY_FRAMES,
            input_target_frames: None,
            input_jitter_frames: DEFAULT_INPUT_JITTER_FRAMES,
            max_packet_frames: DEFAULT_MAX_PACKET_FRAMES,
            max_gap_frames: DEFAULT_MAX_GAP_FRAMES,
            vad_recovery_cap_periods: None,
            vad_minimum_period: false,
            render_target_frames: None,
            wake_margin_frames: DEFAULT_WAKE_MARGIN_FRAMES,
            callback_budget_frames: DEFAULT_CALLBACK_BUDGET_FRAMES,
            engine_block_frames: DEFAULT_ENGINE_BLOCK_FRAMES,
            telemetry: TelemetryMode::Summary,
            run_id: None,
            source_revision: None,
            binary_hash: None,
            config_hash: None,
            machine_id: None,
            windows_build: None,
            controlled_load_id: None,
            diagnostic_soak: false,
            observation_interval_seconds: 5,
            dry_run: false,
        }
    }
}

impl LiveOptions {
    fn vad_route_kind(&self) -> vad_route::RouteKind {
        if self.vad_microphone_id.is_some() {
            vad_route::RouteKind::Microphone
        } else {
            vad_route::RouteKind::Physical
        }
    }

    fn microphone_target(&self) -> Result<Option<vad_route::MicrophoneTarget>> {
        match (&self.vad_microphone_id, &self.vad_microphone_name) {
            (None, None) => Ok(None),
            (Some(id), Some(name)) => Ok(Some(vad_route::microphone_target(
                id,
                name,
                self.stable_endpoint_id
                    .as_deref()
                    .context("playback identity missing")?,
            )?)),
            _ => bail!("--vad-microphone-id and --vad-microphone-name must be supplied together"),
        }
    }

    fn output_label(&self) -> String {
        self.vad_microphone_id.as_ref().map_or_else(
            || self.output_endpoint.clone(),
            |id| format!("virtual-microphone:{}", id.to_ascii_lowercase()),
        )
    }

    fn route_intent(&self) -> Option<Value> {
        self.vad_microphone_id.as_ref().map(|id| serde_json::json!({
            "route": "microphone", "input_kind": "virtual_playback", "bus": true,
            "output_kind": "virtual_microphone", "clock": "virtual_realtime",
            "playback_stable_endpoint_id": self.stable_endpoint_id,
            "microphone_stable_endpoint_id": id.to_ascii_lowercase(),
            "microphone_requested_name": self.vad_microphone_name,
            "source_native_request": {"mode": "exclusive", "sample_rate": 48000, "channels": 2},
            "recorder_native_request": {"mode": "shared", "sample_format": "float32", "sample_rate": 48000},
            "native_request_meaning": "client modes are distinct; negotiated recorder shape is retained in measurement.info, not inferred from requests",
            "boundary": "internal", "qualified": false,
            "independent_reference": false,
            "physical_render_queue_applicable": false,
            "meaning": "requested software route; source submission and duplicated stereo are not an independent delay reference",
        }))
    }
}

#[derive(Debug, Serialize)]
struct LiveRunManifest {
    validation: ValidationPolicy,
    schema_version: u32,
    mode: &'static str,
    run_id: String,
    status: &'static str,
    source_kind: EvidenceSourceKind,
    source_pid: u32,
    stable_endpoint_id: Option<String>,
    source_name: Option<String>,
    source_identity: Option<Value>,
    playback_source: Option<source::SourceProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vad_minimum_period: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vad_route: Option<Value>,
    collection: Option<Value>,
    output_endpoint_id: String,
    measurement_boundary: EvidenceBoundary,
    requested_seconds: u64,
    diagnostic_soak: bool,
    observation_interval_seconds: u64,
    process_bridge: ProcessBridgeOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    virtual_buffer: Option<VirtualBufferOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    render_queue: Option<RenderQueuePolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    physical_render_request: Option<PhysicalRenderRequest>,
    artifacts: ArtifactPaths,
    gaps: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LiveRunReport {
    validation: ValidationPolicy,
    schema_version: u32,
    mode: &'static str,
    run_id: String,
    status: &'static str,
    qualified: bool,
    source: SourceReport,
    output_endpoint_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    physical_render_request: Option<PhysicalRenderRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vad_route: Option<Value>,
    measurement_boundary: EvidenceBoundary,
    requested_seconds: u64,
    diagnostic_soak: bool,
    observation_interval_seconds: u64,
    observed_duration_seconds: f64,
    artifacts: ArtifactReport,
    native: NativeReport,
    collection: Option<Value>,
    qualification: Option<QualificationReport>,
    /// Retain opt-in rate hypotheses alongside any uncorrected distribution.
    /// Otherwise a downstream reader loses the analysis model and resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_search_measurement: Option<PcmAnalysis>,
    /// Trend-only imports retain the same full model without relabeling them
    /// as rate searches. Combined opt-ins stay in rate_search_measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    trend_measurement: Option<PcmAnalysis>,
    gaps: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SourceReport {
    source_kind: EvidenceSourceKind,
    pid: Option<u32>,
    stable_endpoint_id: Option<String>,
    requested_name: Option<String>,
    identity: Option<Value>,
    playback_source: Option<source::SourceProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vad_minimum_period: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
struct NativeReport {
    capture: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capture_failure: Option<Value>,
    #[cfg(any(windows, test))]
    #[serde(skip_serializing_if = "Option::is_none")]
    process_admission: Option<process_admission::ProcessAdmissionEvidence>,
    render: Option<Value>,
    render_failure: Option<Value>,
    bridge: Option<Value>,
    vad_playback: Option<Value>,
    vad_provider: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vad_microphone_provider: Option<Value>,
    vad_cleanup: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vad_microphone_cleanup: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vad_route: Option<Value>,
    engine: Option<Value>,
    engine_cleanup: Option<Value>,
    backend_diagnostics: Option<Value>,
    diagnostic_observations: u64,
    error: Option<String>,
    collection: Option<Value>,
    controlled_load: Option<Value>,
    process_trace: Option<Value>,
}

#[derive(Debug, Default, Serialize)]
struct ArtifactPaths {
    directory: String,
    report: String,
    manifest: String,
    measurement_pcm: Option<String>,
    measurement_schedule: Option<String>,
    source_schedule: Option<String>,
    collection_config: Option<String>,
    collection_report: Option<String>,
    measurement_packets: Option<String>,
    calibration: Option<String>,
    capture_pcm: Option<String>,
    capture_pcm_metadata: Option<String>,
    diagnostic_observations: Option<String>,
    process_trace: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct ArtifactReport {
    paths: ArtifactPaths,
    measurement_pcm_status: &'static str,
    capture_pcm_status: &'static str,
    calibration_status: &'static str,
}

#[derive(Debug)]
struct MeasurementEvidence {
    analysis: Option<PcmAnalysis>,
    calibration: Option<CalibrationProvenance>,
    gaps: Vec<String>,
}

pub fn run(arguments: Vec<String>) -> Result<i32> {
    if arguments.is_empty() || arguments == ["--help"] {
        println!(
            "live-process-loopback --source-pid PID --output ENDPOINT --artifact-dir DIR [options]\n\
             live-process-loopback --source-output ENDPOINT --source-schedule FILE --process-collection FILE [options]\n\
             live-vad --stable-endpoint-id ID --name NAME --output ENDPOINT --artifact-dir DIR [options]\n\
             live-vad --source-schedule FILE --vad-collection FILE opts into bounded source +\n\
             production graph + bounded raw waveform collection. --seconds is its total ceiling.\n\
             Replace physical --output with --vad-microphone-id ID --vad-microphone-name NAME\n\
             for the explicit playback -> Bus -> microphone software route. It requires owned\n\
             source/collection and Internal boundary; schema2 config may omit input_endpoint_id,\n\
             which resolves only the owned microphone. An explicit recorder ID must match exactly.\n\
             Software capture has no independent delay reference and never qualifies physical latency.\n\
             live-vad accepts --vad-recovery-cap-periods 1..8; omitted preserves 8.\n\
             This limits virtual-clock catch-up only, not physical whole-wake demand or measured latency; smaller caps may discard more overdue periods.\n\
             --vad-minimum-period requests the queried exclusive VAD minimum; omitted preserves the default request.\n\
             Period requests are unqualified; actual buffer and provider packet observations remain separate.\n\
             --render-mode shared|exclusive --render-period-frames 64|128|256|512|1024|2048|4096\n\
             selects physical output only in both probe paths; omission preserves existing defaults.\n\
             Explicit periods refuse unknown/mismatched observations or Shared fallback; Exclusive requires capacity rendering.\n\
             Explicit input capacity/jitter/packet/gap flags require --input-target-frames; omission preserves compatibility.\n\
             --validation-mode latency-sensitive|correctness [--jitter-tolerance-ms 0..3000]\n\
             Correctness retains jitter/underruns without early quality abort; never latency qualification.\n\
             Correctness exit 0 means collection and cleanup completed, even if quality_passed=false.\n\
             All runs remain unqualified. --dry-run validates/persists inputs without native IO."
        );
        return Ok(0);
    }
    let options = parse(arguments)?;
    validate(&options)?;
    // Decode and precompute before creating artifacts or opening native owners.
    let source_input = options
        .source_schedule
        .as_deref()
        .map(|path| source::read_plan(path, options.max_input_bytes, options.seconds))
        .transpose()?;
    let source_plan = source_input.as_ref().map(|(_, plan)| plan);
    let collection_input = options
        .vad_collection
        .as_deref()
        .map(|path| {
            let provenance = source::SourceProvenance::new(source_plan);
            let frames = provenance
                .schedule
                .as_ref()
                .context("collection requires source schedule")?
                .total_source_frames;
            vad_collection::read_config(path, options.max_input_bytes, frames, options.seconds)
        })
        .transpose()?;
    if let Some((_, config)) = &collection_input {
        config.validate_route(options.vad_route_kind())?;
    }
    let process_input = options
        .process_collection
        .as_deref()
        .map(|path| {
            let provenance = source::SourceProvenance::new(source_plan);
            let frames = provenance
                .schedule
                .as_ref()
                .context("Process collection source missing")?
                .total_source_frames;
            process_collection::read_config(path, options.max_input_bytes, frames, options.seconds)
        })
        .transpose()?;
    let paths = prepare_paths(&options)?;
    if let Some((bytes, _)) = &source_input {
        persist_source_plan(
            paths
                .source_schedule
                .as_deref()
                .context("source artifact path missing")?,
            bytes,
        )?;
    }
    if let Some((bytes, _)) = &collection_input {
        persist_source_plan(
            paths
                .collection_config
                .as_deref()
                .context("collection config artifact missing")?,
            bytes,
        )?;
    }
    if let Some((bytes, _)) = &process_input {
        persist_source_plan(
            paths
                .collection_config
                .as_deref()
                .context("Process collection config path missing")?,
            bytes,
        )?;
    }
    let run_id = options
        .run_id
        .clone()
        .unwrap_or_else(|| format!("live-{}-{}", std::process::id(), unix_seconds()));
    let mut gaps = Vec::new();
    if options.validation.is_correctness() {
        gaps.push("correctness mode continues through recorded timing/quality faults; excluded from latency qualification".into());
    }
    if options.diagnostic_soak {
        gaps.push("bounded diagnostic soak does not qualify continuous waveform, controlled load, markers or physical latency; PCM recording is disabled".into());
    }
    if options.dry_run {
        gaps.push("live source/native capture was intentionally skipped (--dry-run)".into());
    }
    if options.vad_route_kind() == vad_route::RouteKind::Microphone {
        gaps.push("VAD microphone collection is an internal software route; no independent reference or physical delay is established".into());
    }
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut source_identity = source_identity(&options, &mut gaps);
    let measurement = load_measurement(&options, &mut gaps)?;
    let process_bridge = bridge_options(&options)?;
    let render_queue = render_queue(&options)?;
    let mut native = NativeReport::default();
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut observed_duration_seconds = 0.0;

    #[cfg(windows)]
    if !options.dry_run && options.source_kind == EvidenceSourceKind::ProcessLoopback {
        let result = if let Some((_, config)) = &process_input {
            process_collection::collect(
                &options,
                &paths,
                config,
                source_plan.context("owned source missing")?,
                &process_bridge,
                render_queue,
                &mut native,
            )
        } else {
            native::run(
                &options,
                &process_bridge,
                render_queue,
                &paths,
                &mut native,
                &mut gaps,
            )
        };
        if options.process_collection.is_some() {
            source_identity = native
                .collection
                .as_ref()
                .and_then(|v| v.get("source_identity"))
                .filter(|v| !v.is_null())
                .cloned();
        }
        if let Err(error) = result {
            gaps.push(format!("live Process Loopback run failed: {error:#}"));
            native.error = Some(format!("{error:#}"));
        } else {
            observed_duration_seconds = observed_duration(&native);
        }
    }
    #[cfg(windows)]
    if !options.dry_run && options.source_kind == EvidenceSourceKind::VadPlayback {
        let result = native::run_vad(
            &options,
            &paths,
            source_plan.cloned(),
            collection_input.as_ref().map(|(_, config)| config),
            &mut native,
            &mut gaps,
        );
        if let Err(error) = result {
            gaps.push(format!("live VAD run failed: {error:#}"));
            native.error = Some(format!("{error:#}"));
        } else {
            observed_duration_seconds = observed_duration(&native);
            source_identity = native
                .vad_playback
                .as_ref()
                .and_then(|value| {
                    Some(serde_json::json!({
                        "stable_endpoint_id": value.get("stable_endpoint_id")?,
                        "mmdevice_id": value.get("mmdevice_id")?,
                        "adapter_instance": value.get("adapter_instance")?,
                        "requested_name": options.source_name,
                        "observed_name": value.get("observed_name"),
                    }))
                })
                .or(source_identity);
        }
    }
    #[cfg(not(windows))]
    if !options.dry_run {
        gaps.push("live Process Loopback capture/render requires Windows native APIs".into());
    }

    let collection = if let Some((_, config)) = &process_input {
        Some(native.collection.take().unwrap_or_else(|| {
            process_collection::not_collected(
                config,
                source::SourceProvenance::new(source_plan),
                if options.dry_run {
                    "dry-run: no native collection attempted"
                } else {
                    native
                        .error
                        .as_deref()
                        .unwrap_or("owned Process collection unavailable")
                },
            )
        }))
    } else {
        collection_input.as_ref().map(|(_, config)| {
            native.collection.take().unwrap_or_else(|| {
                vad_collection::not_collected(
                    config,
                    source::SourceProvenance::new(source_plan),
                    if options.dry_run {
                        "dry-run: no native collection attempted"
                    } else {
                        native
                            .error
                            .as_deref()
                            .unwrap_or("collection setup failed before native collection")
                    },
                )
            })
        })
    };
    let actual_pid = if options.process_collection.is_some() {
        source_identity
            .as_ref()
            .and_then(|v| v.get("pid"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .or_else(|| {
                collection
                    .as_ref()
                    .and_then(|v| v.pointer("/owned_source/child_pid"))
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
            })
    } else {
        (options.source_pid != 0).then_some(options.source_pid)
    };
    if source_identity.is_none() {
        gaps.push("live source identity was not observed".into());
    }
    if measurement.analysis.is_none() {
        gaps.push("no externally retained marker-bearing measurement PCM was analyzed".into());
    }
    if measurement.calibration.is_none() {
        gaps.push("independent calibration provenance is missing".into());
    }
    if matches!(
        options.measurement_boundary,
        EvidenceBoundary::Internal | EvidenceBoundary::WasapiLoopback
    ) {
        gaps.push(
            "internal/WASAPI loopback evidence cannot qualify physical end-to-end latency".into(),
        );
    }
    // This probe does not claim qualification itself. The offline evaluator
    // remains the gate after independent artifact/hash verification.
    if options.source_kind == EvidenceSourceKind::ProcessLoopback
        && !options.diagnostic_soak
        && paths
            .capture_pcm
            .as_deref()
            .is_none_or(|path| !Path::new(path).exists())
    {
        gaps.push("live capture PCM artifact was not produced".into());
    }

    let exit_code = collection_exit_code(&options, &native, collection.as_ref());
    let artifacts = artifact_report(&paths, &options, &measurement);
    let qualification = build_qualification_report(
        &options,
        &run_id,
        source_identity.as_ref(),
        observed_duration_seconds,
        &native,
        &measurement,
    );
    let (rate_search_measurement, trend_measurement) = match measurement.analysis {
        Some(analysis) if analysis.schedule.rate_search.is_some() => (Some(analysis), None),
        Some(analysis) if analysis.schedule.trend.is_some() => (None, Some(analysis)),
        _ => (None, None),
    };
    let report = LiveRunReport {
        validation: options.validation,
        schema_version: LIVE_SCHEMA_VERSION,
        mode: mode_name(options.source_kind),
        run_id: run_id.clone(),
        status: "unqualified",
        qualified: false,
        source: SourceReport {
            source_kind: options.source_kind,
            pid: actual_pid,
            stable_endpoint_id: options.stable_endpoint_id.clone(),
            requested_name: options.source_name.clone(),
            identity: source_identity.clone(),
            playback_source: (options.source_kind == EvidenceSourceKind::VadPlayback
                || options.process_collection.is_some())
            .then(|| source::SourceProvenance::new(source_plan)),
            vad_minimum_period: options.vad_minimum_period.then_some(true),
        },
        output_endpoint_id: options.output_label(),
        physical_render_request: (options.vad_route_kind() == vad_route::RouteKind::Physical)
            .then_some(options.physical_render_request),
        vad_route: options.route_intent(),
        measurement_boundary: options.measurement_boundary,
        requested_seconds: options.seconds,
        diagnostic_soak: options.diagnostic_soak,
        observation_interval_seconds: options.observation_interval_seconds,
        observed_duration_seconds,
        artifacts,
        native,
        collection: collection.clone(),
        qualification,
        rate_search_measurement,
        trend_measurement,
        gaps: gaps.clone(),
    };
    let manifest = LiveRunManifest {
        validation: options.validation,
        schema_version: LIVE_SCHEMA_VERSION,
        mode: mode_name(options.source_kind),
        run_id,
        status: "unqualified",
        source_kind: options.source_kind,
        source_pid: actual_pid.unwrap_or(0),
        stable_endpoint_id: options.stable_endpoint_id.clone(),
        source_name: options.source_name.clone(),
        source_identity,
        playback_source: (options.source_kind == EvidenceSourceKind::VadPlayback
            || options.process_collection.is_some())
        .then(|| source::SourceProvenance::new(source_plan)),
        vad_minimum_period: options.vad_minimum_period.then_some(true),
        vad_route: options.route_intent(),
        collection: collection.clone(),
        output_endpoint_id: options.output_label(),
        measurement_boundary: options.measurement_boundary,
        requested_seconds: options.seconds,
        diagnostic_soak: options.diagnostic_soak,
        observation_interval_seconds: options.observation_interval_seconds,
        process_bridge,
        virtual_buffer: (options.source_kind == EvidenceSourceKind::VadPlayback)
            .then(|| virtual_buffer(&options))
            .transpose()?,
        render_queue: (options.vad_route_kind() == vad_route::RouteKind::Physical)
            .then_some(render_queue),
        physical_render_request: (options.vad_route_kind() == vad_route::RouteKind::Physical)
            .then_some(options.physical_render_request),
        artifacts: artifact_paths(&paths, &options),
        gaps,
    };
    if let Some(value) = &collection {
        let mut failures = None;
        // Every artifact gets an attempt after all native/sink owners joined.
        for result in [
            write_json(
                paths
                    .collection_report
                    .as_deref()
                    .context("collection report path missing")?,
                value,
            ),
            write_json(&paths.report, &report),
            write_json(&paths.manifest, &manifest),
        ] {
            if let Err(error) = result {
                vad_collection::append_failure(
                    &mut failures,
                    &format!("write retained collection artifacts: {error:#}"),
                );
            }
        }
        ensure!(
            failures.is_none(),
            "{}",
            failures.as_deref().unwrap_or_default()
        );
    } else {
        write_json(&paths.report, &report)?;
        write_json(&paths.manifest, &manifest)?;
    }
    Ok(exit_code)
}

fn collection_exit_code(
    options: &LiveOptions,
    native: &NativeReport,
    collection: Option<&Value>,
) -> i32 {
    let completed = collection.is_some_and(|collection| {
        collection
            .get("collection_complete")
            .and_then(Value::as_bool)
            == Some(true)
            && collection.get("cleanup_complete").and_then(Value::as_bool) == Some(true)
            && collection.get("failure").is_some_and(Value::is_null)
    });
    i32::from(
        !options.validation.is_correctness()
            || options.dry_run
            || native.error.is_some()
            || !completed,
    )
}

fn parse(arguments: Vec<String>) -> Result<LiveOptions> {
    let mut options = LiveOptions::default();
    let mut source_kind_seen = false;
    let mut validation_seen = false;
    let mut tolerance_seen = false;
    let mut iter = arguments.into_iter();
    while let Some(flag) = iter.next() {
        let value = match flag.as_str() {
            "--vad-minimum-period" => {
                ensure!(
                    !options.vad_minimum_period,
                    "duplicate --vad-minimum-period"
                );
                options.vad_minimum_period = true;
                continue;
            }
            "--diagnostic-soak" => {
                ensure!(!options.diagnostic_soak, "duplicate --diagnostic-soak");
                options.diagnostic_soak = true;
                continue;
            }
            "--dry-run" => {
                ensure!(!options.dry_run, "duplicate --dry-run");
                options.dry_run = true;
                continue;
            }
            "--source-pid"
            | "--source-kind"
            | "--stable-endpoint-id"
            | "--name"
            | "--vad-microphone-id"
            | "--vad-microphone-name"
            | "--output"
            | "--artifact-dir"
            | "--report"
            | "--manifest"
            | "--measurement-boundary"
            | "--measurement-pcm"
            | "--schedule"
            | "--source-schedule"
            | "--source-output"
            | "--process-collection"
            | "--vad-collection"
            | "--calibration"
            | "--seconds"
            | "--observation-interval-seconds"
            | "--max-input-bytes"
            | "--input-capacity-frames"
            | "--input-target-frames"
            | "--input-jitter-frames"
            | "--max-packet-frames"
            | "--max-gap-frames"
            | "--vad-recovery-cap-periods"
            | "--render-target-frames"
            | "--render-mode"
            | "--render-period-frames"
            | "--wake-margin-frames"
            | "--callback-budget-frames"
            | "--engine-block-frames"
            | "--validation-mode"
            | "--jitter-tolerance-ms"
            | "--mode"
            | "--run-id"
            | "--source-revision"
            | "--binary-hash"
            | "--config-hash"
            | "--machine-id"
            | "--windows-build"
            | "--controlled-load-id" => iter
                .next()
                .with_context(|| format!("{flag} requires a value"))?,
            _ => bail!("unknown live option {flag}"),
        };
        match flag.as_str() {
            "--validation-mode" => {
                ensure!(!validation_seen, "duplicate --validation-mode");
                validation_seen = true;
                options.validation.mode = match value.as_str() {
                    "latency-sensitive" => ValidationMode::LatencySensitive,
                    "correctness" => ValidationMode::Correctness,
                    _ => bail!("validation mode must be latency-sensitive or correctness"),
                };
            }
            "--jitter-tolerance-ms" => {
                ensure!(!tolerance_seen, "duplicate --jitter-tolerance-ms");
                tolerance_seen = true;
                options.validation.jitter_tolerance_ms =
                    value.parse().context("invalid jitter tolerance")?;
            }
            "--source-kind" => {
                ensure!(!source_kind_seen, "duplicate --source-kind");
                source_kind_seen = true;
                options.source_kind = match value.as_str() {
                    "process-loopback" => EvidenceSourceKind::ProcessLoopback,
                    "vad-playback" => EvidenceSourceKind::VadPlayback,
                    "vad" => EvidenceSourceKind::Vad,
                    _ => bail!("--source-kind must be process-loopback or vad-playback"),
                }
            }
            "--source-pid" => {
                ensure!(!options.source_pid_explicit, "duplicate --source-pid");
                options.source_pid_explicit = true;
                options.source_pid = value.parse().context("invalid source PID")?;
            }
            "--source-output" => {
                ensure!(options.source_output.is_none(), "duplicate --source-output");
                options.source_output = Some(value);
            }
            "--process-collection" => {
                ensure!(
                    options.process_collection.is_none(),
                    "duplicate --process-collection"
                );
                ensure!(
                    !value.is_empty() && value.trim() == value && !value.contains('\0'),
                    "invalid --process-collection path"
                );
                options.process_collection = Some(value.into());
            }
            "--stable-endpoint-id" => options.stable_endpoint_id = Some(value),
            "--name" => options.source_name = Some(value),
            "--vad-microphone-id" => {
                ensure!(
                    options.vad_microphone_id.is_none(),
                    "duplicate --vad-microphone-id"
                );
                options.vad_microphone_id = Some(value);
            }
            "--vad-microphone-name" => {
                ensure!(
                    options.vad_microphone_name.is_none(),
                    "duplicate --vad-microphone-name"
                );
                options.vad_microphone_name = Some(value);
            }
            "--output" => {
                ensure!(!options.output_endpoint_explicit, "duplicate --output");
                options.output_endpoint_explicit = true;
                options.output_endpoint = value;
            }
            "--artifact-dir" => options.artifact_dir = PathBuf::from(value),
            "--report" => options.report_path = Some(PathBuf::from(value)),
            "--manifest" => options.manifest_path = Some(PathBuf::from(value)),
            "--measurement-boundary" => options.measurement_boundary = parse_boundary(&value)?,
            "--measurement-pcm" => options.measurement_pcm = Some(PathBuf::from(value)),
            "--schedule" => options.measurement_schedule = Some(PathBuf::from(value)),
            "--source-schedule" => {
                ensure!(
                    options.source_schedule.is_none(),
                    "duplicate --source-schedule"
                );
                ensure!(
                    !value.is_empty() && value.trim() == value && !value.contains('\0'),
                    "invalid --source-schedule path"
                );
                options.source_schedule = Some(PathBuf::from(value));
            }
            "--calibration" => options.calibration = Some(PathBuf::from(value)),
            "--vad-collection" => {
                ensure!(
                    options.vad_collection.is_none(),
                    "duplicate --vad-collection"
                );
                ensure!(
                    !value.is_empty() && value.trim() == value && !value.contains('\0'),
                    "invalid --vad-collection path"
                );
                options.vad_collection = Some(PathBuf::from(value));
            }
            "--seconds" => options.seconds = value.parse().context("invalid duration")?,
            "--observation-interval-seconds" => {
                options.observation_interval_seconds =
                    value.parse().context("invalid observation interval")?;
            }
            "--max-input-bytes" => {
                options.max_input_bytes = value.parse().context("invalid input byte cap")?
            }
            "--input-capacity-frames" => {
                options.input_budget_options_explicit = true;
                options.input_capacity_frames = value.parse().context("invalid input capacity")?
            }
            "--input-target-frames" => {
                options.input_target_frames = Some(value.parse().context("invalid input target")?)
            }
            "--input-jitter-frames" => {
                options.input_budget_options_explicit = true;
                options.input_jitter_frames = value.parse().context("invalid input jitter")?
            }
            "--max-packet-frames" => {
                options.input_budget_options_explicit = true;
                options.max_packet_frames = value.parse().context("invalid packet bound")?
            }
            "--max-gap-frames" => {
                options.input_budget_options_explicit = true;
                options.max_gap_frames = value.parse().context("invalid gap bound")?
            }
            "--vad-recovery-cap-periods" => {
                ensure!(
                    options.vad_recovery_cap_periods.is_none(),
                    "duplicate --vad-recovery-cap-periods"
                );
                options.vad_recovery_cap_periods =
                    Some(value.parse().context("invalid VAD recovery cap")?);
            }
            "--render-target-frames" => {
                options.physical_render_options_explicit = true;
                options.render_target_frames = if value == "capacity" {
                    None
                } else {
                    Some(value.parse().context("invalid render target")?)
                }
            }
            "--render-mode" => {
                ensure!(
                    options.physical_render_request.mode.is_none(),
                    "duplicate --render-mode"
                );
                options.physical_render_options_explicit = true;
                options.physical_render_request.mode = Some(match value.as_str() {
                    "shared" => moiren_engine::node::DeviceMode::Shared,
                    "exclusive" => moiren_engine::node::DeviceMode::Exclusive,
                    _ => bail!("--render-mode must be shared or exclusive"),
                });
            }
            "--render-period-frames" => {
                ensure!(
                    options.physical_render_request.period_frames.is_none(),
                    "duplicate --render-period-frames"
                );
                options.physical_render_options_explicit = true;
                options.physical_render_request.period_frames =
                    Some(value.parse().context("invalid physical render period")?);
            }
            "--wake-margin-frames" => {
                options.physical_render_options_explicit = true;
                options.wake_margin_frames = value.parse().context("invalid wake margin")?
            }
            "--callback-budget-frames" => {
                options.physical_render_options_explicit = true;
                options.callback_budget_frames = value.parse().context("invalid callback budget")?
            }
            "--engine-block-frames" => {
                options.engine_block_frames = value.parse().context("invalid engine block")?
            }
            "--mode" => {
                options.telemetry = match value.as_str() {
                    "disabled" => TelemetryMode::Disabled,
                    "summary" => TelemetryMode::Summary,
                    "trace" => TelemetryMode::Trace,
                    _ => bail!("--mode must be disabled, summary or trace"),
                }
            }
            "--run-id" => options.run_id = Some(value),
            "--source-revision" => options.source_revision = Some(value),
            "--binary-hash" => options.binary_hash = Some(value),
            "--config-hash" => options.config_hash = Some(value),
            "--machine-id" => options.machine_id = Some(value),
            "--windows-build" => options.windows_build = Some(value),
            "--controlled-load-id" => options.controlled_load_id = Some(value),
            _ => unreachable!("value-bearing flag was matched above"),
        }
    }
    Ok(options)
}

fn validate(options: &LiveOptions) -> Result<()> {
    options.validation.validate()?;
    options
        .physical_render_request
        .validate(render_queue(options)?)?;
    ensure!(
        !options.input_budget_options_explicit || options.input_target_frames.is_some(),
        "explicit input capacity/jitter/packet/gap options require --input-target-frames"
    );
    if options.microphone_target()?.is_some() {
        ensure!(
            options.source_kind == EvidenceSourceKind::VadPlayback
                && options.vad_collection.is_some()
                && options.source_schedule.is_some(),
            "VAD microphone output requires owned live-vad --source-schedule and --vad-collection"
        );
        ensure!(
            !options.output_endpoint_explicit
                && options.output_endpoint.is_empty()
                && !options.physical_render_options_explicit,
            "VAD microphone output cannot mix physical --output or render queue options"
        );
        ensure!(
            options.measurement_boundary == EvidenceBoundary::Internal
                && options.calibration.is_none()
                && options.measurement_pcm.is_none()
                && options.measurement_schedule.is_none()
                && !options.diagnostic_soak,
            "VAD microphone collection is internal software evidence, not physical calibration or imported delay"
        );
    }
    ensure!(
        !options.vad_minimum_period || options.source_kind == EvidenceSourceKind::VadPlayback,
        "--vad-minimum-period is only valid for live-vad"
    );
    if let Some(cap) = options.vad_recovery_cap_periods {
        ensure!(
            options.source_kind == EvidenceSourceKind::VadPlayback,
            "--vad-recovery-cap-periods is only valid for live-vad"
        );
        ensure!(
            (1..=8).contains(&cap),
            "--vad-recovery-cap-periods must be 1..=8"
        );
    }
    ensure!(
        !options.validation.is_correctness()
            || options.vad_collection.is_some()
            || options.process_collection.is_some(),
        "correctness mode requires a supervised VAD or owned Process collection"
    );
    let owned = options.process_collection.is_some() || options.source_output.is_some();
    if owned {
        ensure!(
            options.source_kind == EvidenceSourceKind::ProcessLoopback
                && options.process_collection.is_some()
                && options.source_output.is_some()
                && options.source_schedule.is_some(),
            "owned Process requires --source-output, --source-schedule and --process-collection"
        );
        ensure!(
            !options.source_pid_explicit && options.source_pid == 0,
            "--source-pid is incompatible with owned Process collection"
        );
        ensure!(
            options.vad_collection.is_none()
                && options.stable_endpoint_id.is_none()
                && options.source_name.is_none()
                && !options.diagnostic_soak
                && options.measurement_pcm.is_none()
                && options.measurement_schedule.is_none()
                && options.calibration.is_none(),
            "owned Process cannot mix VAD, diagnostic soak, imported measurement or calibration"
        );
        let endpoint = options
            .source_output
            .as_deref()
            .context("source output missing")?;
        ensure!(
            !endpoint.trim().is_empty()
                && !endpoint.contains('\0')
                && endpoint.encode_utf16().count() <= 4096,
            "invalid explicit source output endpoint"
        );
        ensure!(
            options.telemetry != TelemetryMode::Disabled,
            "owned Process requires native summary observations"
        );
    }
    if options.vad_collection.is_some() {
        ensure!(
            options.source_kind == EvidenceSourceKind::VadPlayback
                && options.source_schedule.is_some(),
            "--vad-collection requires live-vad and --source-schedule"
        );
        ensure!(
            !options.diagnostic_soak
                && options.measurement_pcm.is_none()
                && options.measurement_schedule.is_none()
                && options.calibration.is_none(),
            "--vad-collection cannot mix diagnostic-soak, analysis or calibration"
        );
        ensure!(
            options.telemetry != TelemetryMode::Disabled,
            "collection requires native summary observations"
        );
    }
    if options.controlled_load_id.is_some() {
        ensure!(
            options.seconds >= 1,
            "controlled load requires a positive collection duration"
        );
        ensure!(
            !options.diagnostic_soak,
            "--controlled-load-id cannot mix diagnostic-soak"
        );
        ensure!(
            options.source_kind == EvidenceSourceKind::ProcessLoopback
                || options.vad_collection.is_some(),
            "--controlled-load-id requires Process Loopback or --vad-collection"
        );
    }
    ensure!(
        options.source_schedule.is_none()
            || options.source_kind == EvidenceSourceKind::VadPlayback
            || owned,
        "--source-schedule is only valid for live-vad"
    );
    match options.source_kind {
        EvidenceSourceKind::ProcessLoopback | EvidenceSourceKind::Vad => {
            ensure!(
                owned || options.source_pid != 0,
                "--source-pid must be nonzero"
            );
            ensure!(
                options.stable_endpoint_id.is_none() && options.source_name.is_none(),
                "VAD identity options are only valid for live-vad"
            );
        }
        EvidenceSourceKind::VadPlayback => {
            ensure!(
                options.source_pid == 0,
                "--source-pid is invalid for live-vad"
            );
            let stable_id = options
                .stable_endpoint_id
                .as_deref()
                .context("--stable-endpoint-id is required for live-vad")?;
            ensure!(
                !stable_id.trim().is_empty() && !stable_id.contains('\0'),
                "--stable-endpoint-id is invalid"
            );
            let name = options
                .source_name
                .as_deref()
                .context("--name is required for live-vad")?;
            ensure!(
                !name.trim().is_empty() && !name.contains('\0'),
                "--name is invalid"
            );
        }
    }
    ensure!(
        options.vad_route_kind() == vad_route::RouteKind::Microphone
            || (!options.output_endpoint.trim().is_empty()
                && !options.output_endpoint.contains('\0')),
        "--output must be a nonempty endpoint ID without NUL"
    );
    ensure!(
        !options.artifact_dir.as_os_str().is_empty(),
        "--artifact-dir is required"
    );
    ensure!(
        (1..=if options.diagnostic_soak { 7200 } else { 600 }).contains(&options.seconds),
        "--seconds must be 1..600, or 1..7200 with --diagnostic-soak"
    );
    ensure!(
        (5..=60).contains(&options.observation_interval_seconds),
        "--observation-interval-seconds must be 5..60"
    );
    ensure!(
        options.max_input_bytes > 0 && options.max_input_bytes < usize::MAX as u64,
        "--max-input-bytes is invalid"
    );
    ensure!(
        options.engine_block_frames > 0 && options.engine_block_frames <= 4096,
        "--engine-block-frames must be 1..4096"
    );
    ensure!(
        options.input_capacity_frames > 0,
        "input capacity must be nonzero"
    );
    ensure!(
        options.input_jitter_frames > 0,
        "input jitter must be nonzero"
    );
    ensure!(
        options.max_packet_frames > 0,
        "packet bound must be nonzero"
    );
    ensure!(options.max_gap_frames > 0, "gap bound must be nonzero");
    if let Some(target) = options.input_target_frames {
        ensure!(
            target >= 2 && target < options.input_capacity_frames,
            "input target must be 2..capacity-1"
        );
    }
    if let Some(target) = options.render_target_frames {
        ensure!(target > 0, "render target must be nonzero");
    }
    ensure!(
        options.measurement_pcm.is_some() == options.measurement_schedule.is_some(),
        "--measurement-pcm and --schedule must be supplied together"
    );
    for (name, value) in [
        ("--run-id", options.run_id.as_deref()),
        ("--source-revision", options.source_revision.as_deref()),
        ("--binary-hash", options.binary_hash.as_deref()),
        ("--config-hash", options.config_hash.as_deref()),
        ("--machine-id", options.machine_id.as_deref()),
        ("--windows-build", options.windows_build.as_deref()),
        (
            "--controlled-load-id",
            options.controlled_load_id.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            ensure!(
                !value.trim().is_empty() && !value.contains('\0'),
                "{name} is invalid"
            );
        }
    }
    Ok(())
}

fn parse_boundary(value: &str) -> Result<EvidenceBoundary> {
    match value {
        "internal" => Ok(EvidenceBoundary::Internal),
        "wasapi-loopback" => Ok(EvidenceBoundary::WasapiLoopback),
        "physical-cable" => Ok(EvidenceBoundary::PhysicalCable),
        "external-dual-channel" => Ok(EvidenceBoundary::ExternalDualChannel),
        _ => bail!(
            "measurement boundary must be internal, wasapi-loopback, physical-cable or external-dual-channel"
        ),
    }
}

fn prepare_paths(options: &LiveOptions) -> Result<ArtifactPaths> {
    ensure!(
        !options.artifact_dir.exists(),
        "artifact directory already exists; refusing to overwrite evidence"
    );
    let report = options
        .report_path
        .clone()
        .unwrap_or_else(|| options.artifact_dir.join("report.json"));
    let manifest = options
        .manifest_path
        .clone()
        .unwrap_or_else(|| options.artifact_dir.join("manifest.json"));
    let capture_pcm = options.artifact_dir.join("capture.f32le");
    let capture_pcm_metadata = options.artifact_dir.join("capture.metadata.json");
    let report_key = output_path_key(&report)?;
    let manifest_key = output_path_key(&manifest)?;
    let process_trace = options.artifact_dir.join("process-trace.json");
    let process_trace_key = output_path_key(&process_trace)?;
    if options.source_kind == EvidenceSourceKind::ProcessLoopback {
        ensure!(
            report_key != process_trace_key && manifest_key != process_trace_key,
            "report/manifest aliases reserved process-trace.json"
        );
    }
    let source_path = options.artifact_dir.join("source-schedule.json");
    let source_key = output_path_key(&source_path)?;
    let reserved: Vec<PathBuf> = [
        "collection-config.json",
        "measurement.f32le",
        "measurement.packets.jsonl",
        "collection.json",
        "capture.f32le",
        "capture.metadata.json",
        "controlled-load.stop",
        "controlled-load.result.json",
        "process-source",
    ]
    .iter()
    .map(|name| output_path_key(&options.artifact_dir.join(name)))
    .collect::<Result<_>>()?;
    if options.vad_collection.is_some() || options.process_collection.is_some() {
        ensure!(
            !reserved.contains(&report_key) && !reserved.contains(&manifest_key),
            "report/manifest aliases reserved collection output"
        );
    }
    ensure!(
        report_key != source_key && manifest_key != source_key,
        "report and manifest must not alias reserved source-schedule.json"
    );
    for input in [
        &options.source_schedule,
        &options.measurement_pcm,
        &options.measurement_schedule,
        &options.calibration,
        &options.vad_collection,
        &options.process_collection,
    ]
    .into_iter()
    .flatten()
    {
        let key = output_path_key(input)?;
        ensure!(
            options.source_kind != EvidenceSourceKind::ProcessLoopback || key != process_trace_key,
            "input aliases reserved process-trace.json"
        );
        ensure!(
            key != source_key && key != report_key && key != manifest_key,
            "input must not alias reserved source plan, report or manifest output"
        );
        ensure!(
            (options.vad_collection.is_none() && options.process_collection.is_none())
                || !reserved.contains(&key),
            "input aliases reserved collection output"
        );
    }
    ensure!(
        report_key != manifest_key,
        "--report and --manifest must differ"
    );
    if options.diagnostic_soak {
        let journal_key = output_path_key(&options.artifact_dir.join("observations.jsonl"))?;
        ensure!(
            report_key != journal_key && manifest_key != journal_key,
            "--report and --manifest must not alias the reserved diagnostic observations.jsonl"
        );
    }
    ensure!(
        !report.exists() && !manifest.exists(),
        "report and manifest must be new files"
    );
    ensure!(
        !capture_pcm.exists() && !capture_pcm_metadata.exists(),
        "capture artifact files must be new files"
    );
    if options.process_collection.is_some() {
        let child = output_path_key(&options.artifact_dir.join("process-source"))?;
        ensure!(
            !report_key.starts_with(&child) && !manifest_key.starts_with(&child),
            "report/manifest aliases child private directory"
        );
        let mut parent = options.artifact_dir.parent();
        while let Some(path) = parent {
            if let Ok(metadata) = fs::symlink_metadata(path) {
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "collection parent is a symlink"
                );
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    ensure!(
                        metadata.file_attributes() & 0x400 == 0,
                        "collection parent is a reparse point"
                    );
                }
            }
            parent = path.parent();
        }
    }
    fs::create_dir_all(&options.artifact_dir).with_context(|| {
        format!(
            "create artifact directory {}",
            options.artifact_dir.display()
        )
    })?;
    Ok(ArtifactPaths {
        directory: options.artifact_dir.to_string_lossy().into_owned(),
        report: report.to_string_lossy().into_owned(),
        manifest: manifest.to_string_lossy().into_owned(),
        measurement_pcm: options
            .measurement_pcm
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| {
                (options.vad_collection.is_some() || options.process_collection.is_some()).then(
                    || {
                        options
                            .artifact_dir
                            .join("measurement.f32le")
                            .to_string_lossy()
                            .into_owned()
                    },
                )
            }),
        measurement_schedule: options
            .measurement_schedule
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        source_schedule: options
            .source_schedule
            .as_ref()
            .map(|_| source_path.to_string_lossy().into_owned()),
        collection_config: (options.vad_collection.is_some()
            || options.process_collection.is_some())
        .then(|| {
            options
                .artifact_dir
                .join("collection-config.json")
                .to_string_lossy()
                .into_owned()
        }),
        collection_report: (options.vad_collection.is_some()
            || options.process_collection.is_some())
        .then(|| {
            options
                .artifact_dir
                .join("collection.json")
                .to_string_lossy()
                .into_owned()
        }),
        measurement_packets: (options.vad_collection.is_some()
            || options.process_collection.is_some())
        .then(|| {
            options
                .artifact_dir
                .join("measurement.packets.jsonl")
                .to_string_lossy()
                .into_owned()
        }),
        calibration: options
            .calibration
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        capture_pcm: (!options.diagnostic_soak).then(|| capture_pcm.to_string_lossy().into_owned()),
        capture_pcm_metadata: (!options.diagnostic_soak)
            .then(|| capture_pcm_metadata.to_string_lossy().into_owned()),
        diagnostic_observations: options.diagnostic_soak.then(|| {
            options
                .artifact_dir
                .join("observations.jsonl")
                .to_string_lossy()
                .into_owned()
        }),
        process_trace: (options.source_kind == EvidenceSourceKind::ProcessLoopback)
            .then(|| process_trace.to_string_lossy().into_owned()),
    })
}

fn output_path_key(path: &Path) -> Result<PathBuf> {
    use std::path::Component;
    let absolute = std::path::absolute(path).context("resolve artifact output path")?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    // New outputs cannot be canonicalized. Compare lexical absolute paths,
    // including Windows' ordinary case-insensitive spellings, before mkdir.
    #[cfg(windows)]
    let normalized = PathBuf::from(normalized.to_string_lossy().to_lowercase());
    Ok(normalized)
}

fn source_identity(options: &LiveOptions, gaps: &mut Vec<String>) -> Option<Value> {
    if options.process_collection.is_some() {
        return None;
    }
    if options.source_kind == EvidenceSourceKind::VadPlayback {
        return Some(serde_json::json!({
            "stable_endpoint_id": options.stable_endpoint_id,
            "requested_name": options.source_name,
        }));
    }
    #[cfg(windows)]
    {
        match moiren_windows_audio::process_loopback::inspect_process(options.source_pid) {
            Ok(identity) => Some(serde_json::to_value(identity).expect("identity serializes")),
            Err(error) => {
                gaps.push(format!("inspect source process: {error}"));
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = options;
        gaps.push("source process inspection requires Windows".into());
        None
    }
}

fn bridge_options(options: &LiveOptions) -> Result<ProcessBridgeOptions> {
    let input =
        options
            .input_target_frames
            .map_or(TransportInputPolicy::Compatible, |target_frames| {
                TransportInputPolicy::Target {
                    capacity_frames: options.input_capacity_frames,
                    target_frames,
                    jitter_frames: options.input_jitter_frames,
                }
            });
    let delivery = options.input_target_frames.map(|_| InputDeliveryTiming {
        sample_rate: 48_000,
        max_packet_frames: options.max_packet_frames,
        max_gap_frames: options.max_gap_frames,
        source: DeliveryTimingSource::Declared,
    });
    Ok(ProcessBridgeOptions {
        input,
        delivery,
        engine_sample_rate: 48_000,
        max_demand_frames: options.engine_block_frames,
        age_limit_100ns: None,
        max_ingress_elapsed_100ns: None,
    })
}

fn virtual_buffer(options: &LiveOptions) -> Result<VirtualBufferOptions> {
    let bridge = bridge_options(options)?;
    Ok(VirtualBufferOptions {
        input: bridge.input,
        delivery: bridge.delivery,
        recovery_cap_periods: options.vad_recovery_cap_periods,
    })
}

fn render_queue(options: &LiveOptions) -> Result<RenderQueuePolicy> {
    let queue = options
        .render_target_frames
        .map_or(RenderQueuePolicy::FullCapacity, |frames| {
            RenderQueuePolicy::Target {
                frames,
                wake_margin_frames: options.wake_margin_frames,
                callback_budget_frames: options.callback_budget_frames,
            }
        });
    queue.validate().map_err(|error| anyhow::anyhow!(error))?;
    Ok(queue)
}

fn load_measurement(options: &LiveOptions, gaps: &mut Vec<String>) -> Result<MeasurementEvidence> {
    let mut evidence = MeasurementEvidence {
        analysis: None,
        calibration: None,
        gaps: Vec::new(),
    };
    if let (Some(pcm_path), Some(schedule_path)) =
        (&options.measurement_pcm, &options.measurement_schedule)
    {
        let schedule: PcmSchedule = match read_json(schedule_path, options.max_input_bytes) {
            Ok(schedule) => schedule,
            Err(error) => {
                evidence
                    .gaps
                    .push(format!("read measurement schedule: {error:#}"));
                return Ok(evidence);
            }
        };
        let mut pcm = match File::open(pcm_path) {
            Ok(file) => file,
            Err(error) => {
                evidence.gaps.push(format!("open measurement PCM: {error}"));
                return Ok(evidence);
            }
        };
        let bytes = pcm.metadata()?.len();
        match analyze(&mut pcm, bytes, schedule) {
            Ok(analysis) => evidence.analysis = Some(analysis),
            Err(error) => evidence
                .gaps
                .push(format!("analyze measurement PCM: {error:#}")),
        }
    }
    if let Some(path) = &options.calibration {
        match read_json(path, options.max_input_bytes) {
            Ok(calibration) => evidence.calibration = Some(calibration),
            Err(error) => evidence
                .gaps
                .push(format!("read calibration provenance: {error:#}")),
        }
    }
    gaps.extend(evidence.gaps.iter().cloned());
    Ok(evidence)
}

fn artifact_paths(paths: &ArtifactPaths, _options: &LiveOptions) -> ArtifactPaths {
    paths.clone_for_manifest()
}

impl ArtifactPaths {
    fn clone_for_manifest(&self) -> Self {
        Self {
            directory: self.directory.clone(),
            report: self.report.clone(),
            manifest: self.manifest.clone(),
            measurement_pcm: self.measurement_pcm.clone(),
            measurement_schedule: self.measurement_schedule.clone(),
            source_schedule: self.source_schedule.clone(),
            collection_config: self.collection_config.clone(),
            collection_report: self.collection_report.clone(),
            measurement_packets: self.measurement_packets.clone(),
            calibration: self.calibration.clone(),
            capture_pcm: self.capture_pcm.clone(),
            capture_pcm_metadata: self.capture_pcm_metadata.clone(),
            diagnostic_observations: self.diagnostic_observations.clone(),
            process_trace: self.process_trace.clone(),
        }
    }
}

fn artifact_report(
    paths: &ArtifactPaths,
    options: &LiveOptions,
    measurement: &MeasurementEvidence,
) -> ArtifactReport {
    ArtifactReport {
        paths: paths.clone_for_manifest(),
        measurement_pcm_status: if measurement.analysis.is_some() {
            "externally_supplied_and_analyzed"
        } else if (options.vad_collection.is_some() || options.process_collection.is_some())
            && options.dry_run
        {
            "not_collected_dry_run"
        } else if options.vad_collection.is_some() || options.process_collection.is_some() {
            if paths
                .measurement_pcm
                .as_deref()
                .is_some_and(|path| Path::new(path).is_file())
            {
                "collected_not_analyzed"
            } else {
                "not_collected"
            }
        } else if options.measurement_pcm.is_some() {
            "supplied_but_unusable"
        } else {
            "missing"
        },
        capture_pcm_status: if options.diagnostic_soak {
            "disabled_diagnostic_soak"
        } else if options.source_kind == EvidenceSourceKind::VadPlayback {
            "not_applicable"
        } else if paths
            .capture_pcm
            .as_deref()
            .is_some_and(|path| Path::new(path).exists())
        {
            "recorded_and_retained"
        } else {
            "missing"
        },
        calibration_status: if measurement.calibration.is_some() {
            "supplied"
        } else {
            "missing"
        },
    }
}

#[cfg(windows)]
fn diagnostic_journal(
    options: &LiveOptions,
    paths: &ArtifactPaths,
) -> Result<Option<soak::Journal>> {
    paths
        .diagnostic_observations
        .as_deref()
        .map(|path| {
            soak::Journal::create(
                Path::new(path),
                options
                    .seconds
                    .div_ceil(options.observation_interval_seconds)
                    + 2,
            )
        })
        .transpose()
}

#[cfg(windows)]
fn persist_capture_artifact(
    paths: &ArtifactPaths,
    report: &moiren_windows_audio::capture::CaptureReport,
) -> Result<()> {
    let Some(artifact) = report.artifact.as_ref() else {
        return Ok(());
    };
    let pcm_path = paths
        .capture_pcm
        .as_deref()
        .context("capture PCM artifact path is missing")?;
    let metadata_path = paths
        .capture_pcm_metadata
        .as_deref()
        .context("capture PCM metadata path is missing")?;

    // The native owner has joined, so the bounded recorder is no longer used
    // by the audio callback and can be copied to disk on the control side.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(pcm_path)
        .with_context(|| format!("create new capture PCM artifact {pcm_path}"))?;
    file.write_all(artifact.pcm_bytes())
        .with_context(|| format!("write capture PCM artifact {pcm_path}"))?;
    file.flush()
        .with_context(|| format!("flush capture PCM artifact {pcm_path}"))?;
    write_json(metadata_path, &artifact.metadata())
        .with_context(|| format!("write capture PCM metadata {metadata_path}"))?;
    Ok(())
}

#[cfg(windows)]
fn observed_duration(native: &NativeReport) -> f64 {
    [
        native
            .capture
            .as_ref()
            .and_then(|value| value.get("elapsed_seconds"))
            .and_then(Value::as_f64),
        native
            .render
            .as_ref()
            .and_then(|value| value.get("elapsed_seconds"))
            .and_then(Value::as_f64),
        native
            .vad_playback
            .as_ref()
            .and_then(|value| value.get("elapsed_seconds"))
            .and_then(Value::as_f64),
    ]
    .into_iter()
    .flatten()
    .reduce(f64::min)
    .unwrap_or(0.0)
}

fn build_qualification_report(
    options: &LiveOptions,
    run_id: &str,
    source_identity: Option<&Value>,
    observed_duration_seconds: f64,
    native: &NativeReport,
    measurement: &MeasurementEvidence,
) -> Option<QualificationReport> {
    if options.validation.is_correctness()
        || options.vad_collection.is_some()
        || options.process_collection.is_some()
    {
        // Negotiated recorder format and partial counts live in collection.
        // There is no analyzed waveform/calibration to feed the legacy builder.
        // Correctness runs must never produce promotable latency evidence.
        return None;
    }
    let (source_identity, source_details) = match options.source_kind {
        EvidenceSourceKind::VadPlayback => {
            let stable_id = options
                .stable_endpoint_id
                .clone()
                .unwrap_or_else(|| "unprovided".into());
            (
                format!("stable-endpoint:{stable_id}"),
                Some(SourceIdentityEvidence {
                    stable_endpoint_id: options.stable_endpoint_id.clone(),
                    mmdevice_id: source_identity
                        .and_then(|value| value.get("mmdevice_id").and_then(Value::as_str))
                        .map(str::to_owned),
                    display_name: source_identity
                        .and_then(|value| value.get("observed_name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    process_id: None,
                }),
            )
        }
        EvidenceSourceKind::Vad => ("legacy-vad".into(), None),
        EvidenceSourceKind::ProcessLoopback => {
            let pid = source_identity
                .and_then(|value| value.get("pid").and_then(Value::as_u64))
                .and_then(|pid| u32::try_from(pid).ok())
                .unwrap_or(options.source_pid);
            (
                format!("pid:{pid}"),
                Some(SourceIdentityEvidence {
                    stable_endpoint_id: None,
                    mmdevice_id: None,
                    display_name: None,
                    process_id: Some(pid),
                }),
            )
        }
    };
    let format = EvidenceFormat {
        sample_rate: 48_000,
        channels: 2,
        sample_format: "f32".into(),
    };
    let string_or_unknown =
        |value: &Option<String>| value.clone().unwrap_or_else(|| "unprovided".into());
    let configuration = EvidenceConfiguration {
        source_kind: options.source_kind,
        source_identity,
        source_details,
        output_endpoint_id: options.output_label(),
        processing_format: format.clone(),
        native_input_format: format.clone(),
        native_output_format: format.clone(),
        measurement_format: format,
        graph_id: "input-bus-output".into(),
        signal_id: "marker-v1".into(),
        controlled_load_id: string_or_unknown(&options.controlled_load_id),
    };
    let identity = EvidenceRunIdentity {
        run_id: run_id.into(),
        source_revision: string_or_unknown(&options.source_revision),
        binary_hash: string_or_unknown(&options.binary_hash),
        config_hash: string_or_unknown(&options.config_hash),
        machine_id: string_or_unknown(&options.machine_id),
        windows_build: string_or_unknown(&options.windows_build),
    };
    let distribution = measurement
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.distribution);
    let expected_marker_count = measurement
        .analysis
        .as_ref()
        .map_or(0, |analysis| analysis.schedule.windows.len() as u64);
    let marker_missing = measurement
        .analysis
        .as_ref()
        .map_or(expected_marker_count, |analysis| {
            analysis.failed_markers as u64
        });
    let observed_measurement_frames = measurement
        .analysis
        .as_ref()
        .map_or(0, |analysis| analysis.capture_frames);
    let quality = QualificationQuality {
        marker_missing,
        ..native_quality(native)
    };
    Some(QualificationReport {
        schema_version: QUALIFICATION_SCHEMA_VERSION,
        identity,
        expected_config_hash: options
            .config_hash
            .clone()
            .unwrap_or_else(|| "unprovided".into()),
        configuration,
        boundary: options.measurement_boundary,
        calibration: measurement.calibration.clone(),
        input_queue: queue_evidence(native.capture.as_ref(), "input_queue"),
        output_queue: queue_evidence(native.render.as_ref(), "queue"),
        observed_duration_seconds,
        observed_measurement_frames,
        waveform_evidence_id: options.measurement_pcm.as_ref().map_or_else(
            || "not-recorded".into(),
            |path| path.to_string_lossy().into_owned(),
        ),
        steady_state: false,
        controlled_load: false,
        expected_marker_count,
        distribution,
        quality,
    })
}

fn mode_name(source_kind: EvidenceSourceKind) -> &'static str {
    match source_kind {
        EvidenceSourceKind::ProcessLoopback => "live_process_loopback",
        EvidenceSourceKind::Vad | EvidenceSourceKind::VadPlayback => "live_vad",
    }
}

fn queue_evidence(
    native: Option<&Value>,
    key: &str,
) -> moiren_windows_audio::latency::qualification::QueueEvidence {
    let Some(queue) = native.and_then(|value| value.get(key)) else {
        return Default::default();
    };
    let get = |name| {
        queue
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    moiren_windows_audio::latency::qualification::QueueEvidence {
        requested_capacity_frames: get("capacity_frames"),
        requested_target_frames: get("target_frames"),
        effective_capacity_frames: get("capacity_frames"),
        effective_target_frames: get("target_frames"),
        actual_period_frames: native
            .and_then(|value| value.get("shared_period"))
            .and_then(|value| value.get("actual_period_frames"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    }
}

fn native_delivery_violations(native: &NativeReport) -> u64 {
    let process_violations = native
        .capture
        .as_ref()
        .and_then(|value| value.get("delivery"))
        .map_or(0, |delivery| {
            [
                "packet_bound_violations",
                "gap_bound_violations",
                "unavailable_clocks",
                "regressed_clocks",
            ]
            .into_iter()
            .filter_map(|key| delivery.get(key).and_then(Value::as_u64))
            .fold(0_u64, u64::saturating_add)
        });
    let vad_violations = native
        .vad_provider
        .as_ref()
        .and_then(|value| value.get("telemetry"))
        .map_or(0, |telemetry| {
            [
                "completion_clock_regressions",
                "stale_packets",
                "recovery_events",
                "dropped_packets",
                // Unobserved native slots are delivery quality, not a
                // measured non-silent PCM/frame loss. Do not also sum gaps.
                "missing_sequence_slots",
            ]
            .into_iter()
            .filter_map(|key| telemetry.get(key).and_then(Value::as_u64))
            .fold(0_u64, u64::saturating_add)
        });
    process_violations.saturating_add(vad_violations)
}

fn native_quality(native: &NativeReport) -> QualificationQuality {
    let bridge_counter = |name| {
        native
            .bridge
            .as_ref()
            .and_then(|bridge| bridge.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    // Native and backend diagnostics can retain the same render report. Use
    // one authoritative copy, including the VAD backend-only report path.
    let render = native.render.as_ref().or_else(|| {
        native
            .backend_diagnostics
            .as_ref()
            .and_then(|diagnostics| diagnostics.get("render"))
    });
    let render_shortfall = render
        .and_then(|report| report.pointer("/stats/bridge_shortfall_frames"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut quality = QualificationQuality {
        delivery_bound_violations: native_delivery_violations(native),
        dropped_frames: [
            "dropped_frames",
            "backlog_discarded_frames",
            "age_discarded_frames",
        ]
        .into_iter()
        .map(bridge_counter)
        .fold(0_u64, u64::saturating_add),
        underrun_frames: bridge_counter("underrun_frames").saturating_add(render_shortfall),
        epoch_resets: bridge_counter("resets"),
        // Input SILENT flags describe source content, and prime trimming is
        // intentional startup alignment. Missing output is counted by the
        // bridge's underrun frames; empty padding alone is only diagnostic.
        // Boundary probes count xrun events, not the number of missing frames.
        // Keep those events visible even when the driver delivered every packet.
        discontinuities: native
            .engine
            .as_ref()
            .and_then(|value| value.pointer("/observations/boundary_probes"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|probe| probe.get("xruns").and_then(Value::as_u64))
            .fold(bridge_counter("discontinuities"), u64::saturating_add),
        ..QualificationQuality::default()
    };
    let Some(telemetry) = native
        .vad_provider
        .as_ref()
        .and_then(|value| value.get("telemetry"))
    else {
        return quality;
    };
    let packet_frames = telemetry
        .get("packet_frames")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    quality.dropped_frames = quality.dropped_frames.saturating_add(
        telemetry
            .get("dropped_packets")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .saturating_mul(packet_frames),
    );
    quality.silent_frames = telemetry
        .get("silence_frames")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    quality.epoch_resets = quality.epoch_resets.saturating_add(
        telemetry
            .get("epoch_resets")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    );
    quality
}

fn read_json<T: DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= limit,
        "JSON input byte cap exceeded: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "JSON input grew beyond byte cap: {}",
        path.display()
    );
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

fn write_json(path: &str, value: &impl Serialize) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create new evidence file {path}"))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    use std::io::Write;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn persist_source_plan(path: &str, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create new source plan {path}"))?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(windows)]
#[path = "live_native.rs"]
mod native;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn physical_render_args(directory: &Path, source: &str) -> Vec<String> {
        let mut args = source_test_args(directory);
        if source == "process-loopback" {
            let index = args
                .iter()
                .position(|flag| flag == "--source-kind")
                .unwrap();
            args[index + 1] = source.into();
            args.extend(["--source-pid".into(), "42".into()]);
            let index = args
                .iter()
                .position(|flag| flag == "--stable-endpoint-id")
                .unwrap();
            args.drain(index..index + 4);
        }
        args
    }

    #[test]
    fn physical_render_parser_accepts_common_mode_period_for_both_sources() {
        let directory = temp_dir("physical-render-parser");
        for source in ["process-loopback", "vad-playback"] {
            for mode in ["shared", "exclusive"] {
                for period in ["64", "128", "256", "512", "1024", "2048", "4096"] {
                    let mut args = physical_render_args(&directory, source);
                    args.extend(
                        ["--render-mode", mode, "--render-period-frames", period]
                            .map(str::to_owned),
                    );
                    let options = parse(args).expect("common physical render request must parse");
                    validate(&options).unwrap();
                }
            }
        }
    }

    #[test]
    fn physical_render_rejects_invalid_duplicate_and_conflicting_requests_before_artifacts() {
        let directory = temp_dir("physical-render-invalid");
        for extra in [
            vec!["--render-mode", "Shared"],
            vec!["--render-mode", ""],
            vec!["--render-mode", "shared", "--render-mode", "exclusive"],
            vec![
                "--render-period-frames",
                "128",
                "--render-period-frames",
                "256",
            ],
            vec!["--render-period-frames", "0"],
            vec!["--render-period-frames", "48"],
            vec!["--render-period-frames", "480"],
            vec!["--render-period-frames", "8192"],
            vec!["--render-period-frames", "-64"],
            vec!["--render-period-frames", "4294967296"],
            vec![
                "--render-mode",
                "exclusive",
                "--render-target-frames",
                "256",
            ],
        ] {
            let mut args = physical_render_args(&directory, "process-loopback");
            args.extend(extra.into_iter().map(str::to_owned));
            assert!(run(args).is_err());
            assert!(
                !directory.exists(),
                "invalid native request created artifacts"
            );
        }
    }

    #[test]
    fn physical_render_controls_are_not_accepted_by_the_microphone_route() {
        let directory = temp_dir("physical-render-microphone");
        for extra in [
            vec!["--render-mode", "shared"],
            vec!["--render-period-frames", "128"],
        ] {
            let mut args = microphone_route_args(&directory);
            args.extend(extra.into_iter().map(str::to_owned));
            assert!(run(args).is_err());
            assert!(!directory.exists());
        }
    }

    #[test]
    fn physical_render_request_is_retained_separately_from_native_observations() {
        for (case, extra, mode, period) in [
            ("default", vec![], Value::Null, Value::Null),
            (
                "mode-only",
                vec!["--render-mode", "exclusive"],
                Value::from("exclusive"),
                Value::Null,
            ),
            (
                "period-only",
                vec!["--render-period-frames", "128"],
                Value::Null,
                Value::from(128),
            ),
            (
                "explicit",
                vec![
                    "--render-mode",
                    "exclusive",
                    "--render-period-frames",
                    "128",
                ],
                Value::from("exclusive"),
                Value::from(128),
            ),
        ] {
            let directory = temp_dir(&format!("physical-render-report-{case}"));
            let mut args = physical_render_args(&directory, "process-loopback");
            args.extend(extra.into_iter().map(str::to_owned));
            assert_eq!(run(args).unwrap(), 1);
            for name in ["report.json", "manifest.json"] {
                let value: Value =
                    serde_json::from_slice(&fs::read(directory.join(name)).unwrap()).unwrap();
                assert!(value["physical_render_request"].is_object());
                assert_eq!(value["physical_render_request"]["mode"], mode);
                assert_eq!(value["physical_render_request"]["period_frames"], period);
                assert!(value["native"]["render"].is_null());
            }
            let _ = fs::remove_dir_all(directory);
        }
    }

    fn microphone_route_args(directory: &Path) -> Vec<String> {
        [
            "--source-kind",
            "vad-playback",
            "--stable-endpoint-id",
            "0123456789abcdef0123456789abcdef",
            "--name",
            "Playback",
            "--vad-microphone-id",
            "fedcba9876543210fedcba9876543210",
            "--vad-microphone-name",
            "Microphone",
            "--source-schedule",
            "source.json",
            "--vad-collection",
            "collection.json",
            "--seconds",
            "20",
            "--dry-run",
            "--artifact-dir",
            directory.to_str().unwrap(),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn microphone_route_parser_accepts_exact_owned_internal_route_without_physical_output() {
        let directory = temp_dir("microphone-route-parser");
        let options = parse(microphone_route_args(&directory)).unwrap();
        validate(&options).expect("owned microphone route must not require physical output");
        assert_eq!(options.vad_route_kind(), vad_route::RouteKind::Microphone);
        assert_eq!(
            options.microphone_target().unwrap().unwrap().name,
            "Microphone"
        );
        assert_eq!(
            options.output_label(),
            "virtual-microphone:fedcba9876543210fedcba9876543210"
        );
        assert!(!directory.exists());
    }

    #[test]
    fn microphone_route_rejects_ignored_physical_intent_and_incomplete_identity() {
        let directory = temp_dir("microphone-route-invalid");
        for extra in [
            vec!["--output", "physical"],
            vec!["--render-target-frames", "capacity"],
            vec!["--wake-margin-frames", "96"],
            vec!["--callback-budget-frames", "48"],
            vec!["--measurement-boundary", "physical-cable"],
            vec!["--calibration", "cal.json"],
            vec!["--diagnostic-soak"],
            vec!["--vad-microphone-id", "fedcba9876543210fedcba9876543210"],
            vec!["--vad-microphone-name", "Other"],
        ] {
            let mut arguments = microphone_route_args(&directory);
            arguments.extend(extra.into_iter().map(str::to_owned));
            assert!(
                parse(arguments)
                    .and_then(|options| validate(&options))
                    .is_err()
            );
        }
        for missing in [
            "--vad-microphone-name",
            "--vad-microphone-id",
            "--source-schedule",
            "--vad-collection",
        ] {
            let mut arguments = microphone_route_args(&directory);
            let index = arguments.iter().position(|value| value == missing).unwrap();
            arguments.drain(index..index + 2);
            assert!(
                parse(arguments)
                    .and_then(|options| validate(&options))
                    .is_err(),
                "{missing}"
            );
        }
        assert!(!directory.exists());
    }

    #[test]
    fn microphone_route_rejects_empty_and_erased_explicit_physical_output() {
        let directory = temp_dir("microphone-route-empty-output");
        for extra in [
            vec!["--output", ""],
            vec!["--output", "physical", "--output", ""],
        ] {
            let mut arguments = microphone_route_args(&directory);
            arguments.extend(extra.into_iter().map(str::to_owned));
            assert!(
                parse(arguments)
                    .and_then(|options| validate(&options))
                    .is_err()
            );
        }
        assert!(!directory.exists());
    }

    #[test]
    fn input_budget_flags_require_explicit_target_in_both_routes() {
        let directory = temp_dir("input-budget-orphan");
        for pair in [
            ["--input-capacity-frames", "8192"],
            ["--input-jitter-frames", "96"],
            ["--max-packet-frames", "480"],
            ["--max-gap-frames", "480"],
        ] {
            for microphone in [false, true] {
                let mut arguments = if microphone {
                    microphone_route_args(&directory)
                } else {
                    [
                        "--source-pid",
                        "42",
                        "--output",
                        "physical",
                        "--artifact-dir",
                        directory.to_str().unwrap(),
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
                };
                arguments.extend(pair.into_iter().map(str::to_owned));
                assert!(
                    parse(arguments.clone())
                        .and_then(|options| validate(&options))
                        .is_err(),
                    "{pair:?}"
                );
                arguments.extend(
                    ["--input-target-frames", "2048"]
                        .into_iter()
                        .map(str::to_owned),
                );
                let options = parse(arguments).unwrap();
                validate(&options).unwrap();
                assert!(matches!(
                    bridge_options(&options).unwrap().input,
                    TransportInputPolicy::Target { .. }
                ));
            }
        }
        assert!(!directory.exists());
    }

    #[test]
    fn microphone_route_dry_run_retains_two_id_intent_without_native_or_delay_claim() {
        let directory = temp_dir("microphone-route-dry");
        let source_path = directory.with_extension("source.json");
        let collection_path = directory.with_extension("collection.json");
        fs::write(&source_path, source_plan_bytes()).unwrap();
        let config = br#"{"schema_version":2,"ring_frames":1024,"max_pcm_bytes":1048576,"max_metadata_bytes":100000,"max_packets":1000,"preroll_ms":100,"postroll_ms":200}"#;
        fs::write(&collection_path, config).unwrap();
        let mut args = microphone_route_args(&directory);
        for (flag, path) in [
            ("--source-schedule", &source_path),
            ("--vad-collection", &collection_path),
        ] {
            let index = args.iter().position(|value| value == flag).unwrap();
            args[index + 1] = path.to_string_lossy().into_owned();
        }
        let outcome = run(args);
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(outcome.unwrap(), 1);
        for name in ["manifest.json", "report.json"] {
            let value: Value =
                serde_json::from_slice(&fs::read(directory.join(name)).unwrap()).unwrap();
            let route = &value["vad_route"];
            assert_eq!(route["route"], "microphone");
            assert_eq!(
                route["microphone_stable_endpoint_id"],
                "fedcba9876543210fedcba9876543210"
            );
            assert_eq!(
                route["playback_stable_endpoint_id"],
                "0123456789abcdef0123456789abcdef"
            );
            assert_eq!(route["clock"], "virtual_realtime");
            assert_eq!(route["independent_reference"], false);
            assert_eq!(route["physical_render_queue_applicable"], false);
            assert_eq!(route["source_native_request"]["mode"], "exclusive");
            assert_eq!(route["recorder_native_request"]["mode"], "shared");
            assert_eq!(
                value["output_endpoint_id"],
                "virtual-microphone:fedcba9876543210fedcba9876543210"
            );
            assert_eq!(value["measurement_boundary"], "internal");
            assert_eq!(value["collection"]["collection_complete"], false);
        }
        let report: Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["qualified"], false);
        assert!(report["native"]["engine"].is_null());
        assert!(report["qualification"].is_null());
        let manifest: Value =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        assert!(manifest.get("render_queue").is_none());
        assert!(manifest.get("physical_render_request").is_none());
        assert!(report.get("physical_render_request").is_none());
        assert!(!directory.join("measurement.f32le").exists());
        assert_eq!(
            fs::read(directory.join("collection-config.json")).unwrap(),
            config
        );
        fs::remove_file(source_path).unwrap();
        fs::remove_file(collection_path).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn owned_process_parser_is_explicit_and_preserves_external_pid_mode() {
        let args = vec![
            "--source-output",
            "source",
            "--source-schedule",
            "plan.json",
            "--process-collection",
            "config.json",
            "--output",
            "render",
            "--artifact-dir",
            "new-run",
            "--seconds",
            "20",
        ];
        let owned = parse(args.iter().map(|s| s.to_string()).collect()).unwrap();
        validate(&owned).unwrap();
        for extra in [
            vec!["--source-pid", "0"],
            vec!["--source-output", "again"],
            vec!["--process-collection", "again"],
            vec!["--diagnostic-soak"],
            vec!["--mode", "disabled"],
        ] {
            let mut mixed = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            mixed.extend(extra.into_iter().map(str::to_owned));
            assert!(parse(mixed).and_then(|o| validate(&o)).is_err());
        }
        let external = parse(
            [
                "--source-pid",
                "42",
                "--output",
                "render",
                "--artifact-dir",
                "external",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        )
        .unwrap();
        validate(&external).unwrap();
        assert_eq!(external.source_pid, 42);
    }

    #[test]
    fn owned_process_dry_run_retains_inputs_without_child_or_waveform_claim() {
        let directory = temp_dir("owned-process-dry");
        let source = directory.with_extension("source.json");
        let config = directory.with_extension("config.json");
        fs::write(&source, source_plan_bytes()).unwrap();
        let bytes=br#"{"schema_version":1,"input_endpoint_id":"independent-recorder","ring_frames":1024,"max_pcm_bytes":1048576,"max_metadata_bytes":100000,"max_packets":1000,"preroll_ms":100,"postroll_ms":100}"#;
        fs::write(&config, bytes).unwrap();
        let args = vec![
            "--dry-run".into(),
            "--source-output".into(),
            "explicit-source".into(),
            "--source-schedule".into(),
            source.to_string_lossy().into_owned(),
            "--process-collection".into(),
            config.to_string_lossy().into_owned(),
            "--output".into(),
            "explicit-graph-output".into(),
            "--artifact-dir".into(),
            directory.to_string_lossy().into_owned(),
            "--seconds".into(),
            "14".into(),
        ];
        assert_eq!(run(args).unwrap(), 1);
        let report: Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["qualified"], false);
        assert!(report["qualification"].is_null());
        assert_eq!(
            report["artifacts"]["measurement_pcm_status"],
            "not_collected_dry_run"
        );
        assert_eq!(
            report["collection"]["config"]["input_endpoint_id"],
            "independent-recorder"
        );
        assert!(report["source"]["pid"].is_null());
        assert_eq!(
            fs::read(directory.join("collection-config.json")).unwrap(),
            bytes
        );
        assert!(!directory.join("process-source").exists());
        assert!(!directory.join("measurement.f32le").exists());
        fs::remove_dir_all(directory).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_file(config).unwrap();
    }

    #[test]
    fn collection_artifacts_never_use_source_format_or_claim_analyzed_or_missing_pcm() {
        let directory = temp_dir("collection-format-status");
        fs::create_dir_all(&directory).unwrap();
        let pcm = directory.join("measurement.f32le");
        fs::write(&pcm, [0u8; 4]).unwrap();
        let options = LiveOptions {
            vad_collection: Some("config.json".into()),
            ..Default::default()
        };
        let paths = ArtifactPaths {
            measurement_pcm: Some(pcm.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let evidence = MeasurementEvidence {
            analysis: None,
            calibration: None,
            gaps: Vec::new(),
        };
        let artifacts = artifact_report(&paths, &options, &evidence);
        assert_eq!(artifacts.measurement_pcm_status, "collected_not_analyzed");
        assert!(
            build_qualification_report(
                &options,
                "test",
                None,
                0.0,
                &NativeReport::default(),
                &evidence
            )
            .is_none(),
            "collection must not synthesize a nominal 48k/stereo qualification configuration"
        );
        let dry = LiveOptions {
            dry_run: true,
            ..options
        };
        assert_eq!(
            artifact_report(&paths, &dry, &evidence).measurement_pcm_status,
            "not_collected_dry_run"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("moiren-live-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn source_test_args(directory: &Path) -> Vec<String> {
        vec![
            "--dry-run".into(),
            "--source-kind".into(),
            "vad-playback".into(),
            "--stable-endpoint-id".into(),
            "07070707070707070707070707070707".into(),
            "--name".into(),
            "Source test".into(),
            "--output".into(),
            "out".into(),
            "--artifact-dir".into(),
            directory.to_string_lossy().into_owned(),
        ]
    }

    #[test]
    fn vad_minimum_period_parser_accepts_only_one_vad_switch() {
        let directory = temp_dir("vad-min-period-parser");
        let mut args = source_test_args(&directory);
        args.push("--vad-minimum-period".into());
        let options = parse(args.clone()).expect("explicit minimum period must parse");
        validate(&options).unwrap();
        args.push("--vad-minimum-period".into());
        assert!(parse(args).is_err());

        for source in ["process-loopback", "vad"] {
            let args = [
                "--source-kind",
                source,
                "--source-pid",
                "42",
                "--output",
                "out",
                "--artifact-dir",
                "unused",
                "--vad-minimum-period",
            ];
            assert!(
                parse(args.map(str::to_owned).to_vec())
                    .and_then(|options| validate(&options))
                    .is_err(),
                "{source}"
            );
        }
        let mut value = source_test_args(&directory);
        value.extend(["--vad-minimum-period".into(), "true".into()]);
        assert!(parse(value).is_err(), "the option is a switch");
        assert!(!directory.exists());
    }

    #[test]
    fn vad_minimum_period_dry_reports_retain_intent_without_native_evidence() {
        for minimum in [false, true] {
            let directory = temp_dir(&format!("vad-min-period-dry-{minimum}"));
            let mut args = source_test_args(&directory);
            if minimum {
                args.push("--vad-minimum-period".into());
            }
            assert_eq!(run(args).unwrap(), 1);
            let manifest: Value =
                serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap())
                    .unwrap();
            let report: Value =
                serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
            if minimum {
                assert_eq!(manifest["vad_minimum_period"], true);
                assert_eq!(report["source"]["vad_minimum_period"], true);
            } else {
                assert!(manifest.get("vad_minimum_period").is_none());
                assert!(report["source"].get("vad_minimum_period").is_none());
            }
            assert_eq!(manifest["status"], "unqualified");
            assert_eq!(report["qualified"], false);
            assert!(report["native"]["vad_playback"].is_null());
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn vad_recovery_cap_parser_admits_only_explicit_vad_periods() {
        let directory = temp_dir("vad-cap-parser");
        for cap in 1..=8 {
            let mut args = source_test_args(&directory);
            args.extend(["--vad-recovery-cap-periods".into(), cap.to_string()]);
            let options = parse(args).unwrap();
            validate(&options).unwrap();
            let buffer = virtual_buffer(&options).unwrap();
            assert_eq!(buffer.recovery_cap_periods, Some(cap));
            let prepared = buffer.prepare_recovery(48_000, 32).unwrap();
            assert_eq!(prepared.engine_burst_frames, 32 * cap as u32);
            assert_eq!(prepared.recovery_cap_periods, cap);
        }
        for cap in ["0", "9", "18446744073709551615", "-1", "1.5", "word"] {
            let mut args = source_test_args(&directory);
            args.extend(["--vad-recovery-cap-periods".into(), cap.into()]);
            assert!(
                parse(args).and_then(|options| validate(&options)).is_err(),
                "{cap}"
            );
        }
        let mut duplicate = source_test_args(&directory);
        duplicate.extend(
            [
                "--vad-recovery-cap-periods",
                "1",
                "--vad-recovery-cap-periods",
                "2",
            ]
            .map(str::to_owned),
        );
        assert!(parse(duplicate).is_err());
        let process = [
            "--source-pid",
            "42",
            "--output",
            "out",
            "--artifact-dir",
            "unused",
            "--vad-recovery-cap-periods",
            "1",
        ];
        assert!(
            parse(process.map(str::to_owned).to_vec())
                .and_then(|options| validate(&options))
                .is_err()
        );
        assert!(!directory.exists());
    }

    #[test]
    fn vad_recovery_cap_probe_preserves_whole_wake_and_native_packet_admission() {
        let directory = temp_dir("vad-cap-budget");
        let mut args = source_test_args(&directory);
        args.extend(
            [
                "--vad-recovery-cap-periods",
                "1",
                "--engine-block-frames",
                "32",
                "--input-capacity-frames",
                "1024",
                "--input-target-frames",
                "128",
                "--input-jitter-frames",
                "1",
                "--max-packet-frames",
                "32",
                "--max-gap-frames",
                "32",
            ]
            .map(str::to_owned),
        );
        let mut options = parse(args).unwrap();
        validate(&options).unwrap();
        let buffer = virtual_buffer(&options).unwrap();
        let prepared = buffer
            .prepare_recovery(48_000, options.engine_block_frames)
            .unwrap();
        assert_eq!(prepared.engine_burst_frames, 32);
        assert_eq!(prepared.queue.unwrap().prime_frames, 128);
        assert!(buffer.prepare_budget(48_000, 32, 256).is_err());
        assert!(buffer.admit_native_packet(64).is_err());
        options.vad_recovery_cap_periods = None;
        let buffer = virtual_buffer(&options).unwrap();
        assert_eq!(buffer.admitted_recovery_cap_periods().unwrap(), 8);
        assert!(
            buffer
                .prepare_recovery(48_000, options.engine_block_frames)
                .is_err()
        );
        assert!(!directory.exists());
    }

    #[test]
    fn vad_recovery_cap_dry_manifest_retains_exact_intent_without_native_claim() {
        for cap in [None, Some(1), Some(8)] {
            let directory = temp_dir(&format!("vad-cap-manifest-{cap:?}"));
            let mut args = source_test_args(&directory);
            if let Some(cap) = cap {
                args.extend(["--vad-recovery-cap-periods".into(), cap.to_string()]);
            }
            assert_eq!(run(args).unwrap(), 1);
            let manifest: Value =
                serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap())
                    .unwrap();
            assert!(manifest["virtual_buffer"].is_object());
            assert_eq!(
                manifest["virtual_buffer"]["recovery_cap_periods"],
                serde_json::json!(cap)
            );
            assert_eq!(
                manifest["virtual_buffer"]["input"],
                serde_json::json!({"policy": "compatible"})
            );
            assert_eq!(manifest["status"], "unqualified");
            let report: Value =
                serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
            assert_eq!(report["qualified"], false);
            assert!(report["native"]["engine"].is_null());
            fs::remove_dir_all(directory).unwrap();
        }
    }

    fn source_plan_bytes() -> Vec<u8> {
        br#"{
  "schema_version": 1, "schedule_id": "independent-test",
  "waveform_version": "moiren-marker-xorshift32-v1",
  "sample_rate": 48000, "channels": 2, "marker_frames": 32,
  "marker_amplitude": 0.1, "total_source_frames": 48000, "starts": [0, 40000]
}
"#
        .to_vec()
    }

    #[test]
    fn collection_dry_run_admits_exact_bytes_without_native_or_waveform_claim() {
        let directory = temp_dir("collection-dry");
        let source = directory.with_extension("source.json");
        let config = directory.with_extension("config.json");
        let bytes = br#"{"schema_version":1,"input_endpoint_id":"independent-input","ring_frames":1024,"max_pcm_bytes":1048576,"max_metadata_bytes":100000,"max_packets":1000,"preroll_ms":100,"postroll_ms":200}"#;
        fs::write(&source, source_plan_bytes()).unwrap();
        fs::write(&config, bytes).unwrap();
        let mut args = source_test_args(&directory);
        args.extend([
            "--seconds".into(),
            "14".into(),
            "--source-schedule".into(),
            source.to_string_lossy().into_owned(),
            "--vad-collection".into(),
            config.to_string_lossy().into_owned(),
        ]);
        assert_eq!(run(args.clone()).unwrap(), 1);
        assert_eq!(
            fs::read(directory.join("collection-config.json")).unwrap(),
            bytes
        );
        let outcome: Value =
            serde_json::from_slice(&fs::read(directory.join("collection.json")).unwrap()).unwrap();
        assert_eq!(outcome["qualified"], false);
        assert_eq!(outcome["collection_complete"], false);
        assert_eq!(outcome["status"], "unqualified");
        assert!(outcome["failure"].as_str().unwrap().contains("dry-run"));
        assert!(!directory.join("measurement.f32le").exists());
        for file in ["manifest.json", "report.json"] {
            let report: Value =
                serde_json::from_slice(&fs::read(directory.join(file)).unwrap()).unwrap();
            assert!(report["artifacts"].to_string().contains("collection.json"));
        }
        assert!(run(args).is_err());
        fs::remove_dir_all(&directory).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_file(config).unwrap();
    }

    #[test]
    fn collection_rejects_mixed_modes_and_reserved_path_aliases_before_mkdir() {
        let directory = temp_dir("collection-invalid");
        let mut base = source_test_args(&directory);
        base.extend([
            "--source-schedule".into(),
            "source.json".into(),
            "--vad-collection".into(),
            "config.json".into(),
        ]);
        assert!(parse(base.clone()).and_then(|o| validate(&o)).is_ok());
        for extra in [
            vec!["--vad-collection", "other.json"],
            vec!["--diagnostic-soak"],
            vec!["--measurement-pcm", "pcm", "--schedule", "analysis"],
            vec!["--calibration", "calibration"],
        ] {
            let mut args = base.clone();
            args.extend(extra.into_iter().map(str::to_owned));
            assert!(parse(args).and_then(|o| validate(&o)).is_err());
        }
        let mut options = parse(base).unwrap();
        options.source_schedule = None;
        assert!(validate(&options).is_err());
        options.source_schedule = Some("source.json".into());
        for file in [
            "collection-config.json",
            "measurement.f32le",
            "measurement.packets.jsonl",
            "collection.json",
        ] {
            options.report_path = Some(directory.join(file));
            assert!(prepare_paths(&options).is_err(), "reserved {file}");
            assert!(!directory.exists());
        }
    }

    #[test]
    fn source_dry_run_retains_exact_plan_without_any_measurement_pcm() {
        let directory = temp_dir("source-explicit-dry-run");
        let input = directory.with_extension("input.json");
        let bytes = source_plan_bytes();
        fs::write(&input, &bytes).unwrap();
        let mut args = source_test_args(&directory);
        args.extend([
            "--source-schedule".into(),
            input.to_string_lossy().into_owned(),
        ]);
        let code = run(args).unwrap();
        assert_eq!(code, 1);
        assert_eq!(
            fs::read(directory.join("source-schedule.json")).unwrap(),
            bytes
        );
        for (file, path) in [
            ("report.json", vec!["source", "playback_source"]),
            ("manifest.json", vec!["playback_source"]),
        ] {
            let value: Value =
                serde_json::from_slice(&fs::read(directory.join(file)).unwrap()).unwrap();
            let provenance = path.iter().fold(&value, |value, key| &value[*key]);
            assert_eq!(provenance["mode"], "explicit_source_schedule");
            assert_eq!(provenance["schedule"]["schedule_id"], "independent-test");
            assert_eq!(
                provenance["schedule"]["starts"],
                serde_json::json!([0, 40000])
            );
            assert!(
                provenance["recorder_clock"]
                    .as_str()
                    .unwrap()
                    .contains("no automatic")
            );
            assert_eq!(value["status"], "unqualified");
            assert!(value["native"]["vad_playback"].is_null());
        }
        assert!(!directory.join("measurement.f32le").exists());
        let report: Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["qualified"], false);
        assert!(
            report["artifacts"]["paths"]["source_schedule"]
                .as_str()
                .unwrap()
                .ends_with("source-schedule.json")
        );
        assert_eq!(report["artifacts"]["measurement_pcm_status"], "missing");
        assert!(
            run(source_test_args(&directory)).is_err(),
            "existing run directory must not be overwritten"
        );
        fs::remove_file(input).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_parser_is_vad_only_rejects_duplicates_and_invalid_paths() {
        let directory = temp_dir("source-parser");
        let mut valid = source_test_args(&directory);
        valid.extend(["--source-schedule".into(), "source.json".into()]);
        assert!(
            parse(valid.clone())
                .and_then(|options| validate(&options))
                .is_ok()
        );
        let mut duplicate = valid.clone();
        duplicate.extend(["--source-schedule".into(), "other.json".into()]);
        assert!(parse(duplicate).is_err());
        let mut override_mode = valid.clone();
        override_mode.extend(["--source-kind".into(), "vad-playback".into()]);
        assert!(
            parse(override_mode).is_err(),
            "source kind must not override CLI command mode"
        );
        for path in ["", " ", " source.json", "source.json ", "bad\0path"] {
            let mut args = source_test_args(&directory);
            args.extend(["--source-schedule".into(), path.into()]);
            assert!(parse(args).and_then(|options| validate(&options)).is_err());
        }
        for kind in ["process-loopback", "vad"] {
            assert!(
                parse(vec![
                    "--source-kind".into(),
                    kind.into(),
                    "--source-pid".into(),
                    "1".into(),
                    "--output".into(),
                    "out".into(),
                    "--artifact-dir".into(),
                    directory.to_string_lossy().into_owned(),
                    "--source-schedule".into(),
                    "source.json".into()
                ])
                .and_then(|options| validate(&options))
                .is_err()
            );
        }
    }

    #[test]
    fn source_input_aliases_and_invalid_duration_fail_before_artifact_creation() {
        for input_flag in [
            "--source-schedule",
            "--measurement-pcm",
            "--schedule",
            "--calibration",
        ] {
            let directory = temp_dir(&format!("source-input-alias-{input_flag}"));
            let mut options = LiveOptions {
                artifact_dir: directory.clone(),
                ..Default::default()
            };
            let reserved = directory.join("source-schedule.json");
            match input_flag {
                "--source-schedule" => {
                    let mut args = source_test_args(&directory);
                    args.extend([input_flag.into(), reserved.to_string_lossy().into_owned()]);
                    options = parse(args).unwrap();
                }
                "--measurement-pcm" => options.measurement_pcm = Some(reserved),
                "--schedule" => options.measurement_schedule = Some(reserved),
                _ => options.calibration = Some(reserved),
            }
            let result = prepare_paths(&options);
            let created = directory.exists();
            if created {
                fs::remove_dir_all(&directory).unwrap();
            }
            assert!(result.is_err());
            assert!(!created);
        }
        let directory = temp_dir("source-too-long");
        let input = directory.with_extension("input.json");
        let mut plan: Value = serde_json::from_slice(&source_plan_bytes()).unwrap();
        plan["total_source_frames"] = serde_json::json!(48001);
        fs::write(&input, serde_json::to_vec(&plan).unwrap()).unwrap();
        let mut args = source_test_args(&directory);
        args.extend([
            "--seconds".into(),
            "1".into(),
            "--source-schedule".into(),
            input.to_string_lossy().into_owned(),
        ]);
        assert!(run(args).is_err());
        assert!(!directory.exists());
        fs::remove_file(input).unwrap();
    }

    #[test]
    fn source_artifact_create_new_preserves_existing_bytes() {
        let directory = temp_dir("source-no-overwrite");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("source-schedule.json");
        let text = path.to_str().unwrap();
        persist_source_plan(text, b"original immutable plan").unwrap();
        assert!(persist_source_plan(text, b"replacement").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original immutable plan");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_command_modes_cannot_be_overridden_to_bypass_process_rejection() {
        let directory = temp_dir("source-command-mode");
        let mut args = vec!["live-process-loopback".into()];
        args.extend(source_test_args(&directory));
        args.extend(["--source-schedule".into(), "source.json".into()]);
        let error = crate::cli::run(args).unwrap_err();
        assert!(error.to_string().contains("duplicate --source-kind"));
        assert!(!directory.exists());
    }

    #[test]
    fn source_old_measurement_analysis_never_selects_or_changes_playback_plan() {
        for explicit in [false, true] {
            let directory = temp_dir(&format!("source-old-analysis-{explicit}"));
            let pcm = directory.with_extension("f32le");
            let analysis = directory.with_extension("analysis.json");
            let source = directory.with_extension("source.json");
            fs::write(&pcm, vec![0_u8; 256 * 8]).unwrap();
            let analysis_plan = serde_json::json!({
                "schema_version": 1, "sample_rate": 48000,
                "reference_channel": 0, "processed_channel": 1,
                "marker_frames": 32, "marker_amplitude": 0.2,
                "reference_search_frames": 1, "max_delay_frames": 1,
                "max_marker_frames": 32, "max_window_frames": 34,
                "max_markers": 1, "max_pcm_frames": 256,
                "max_correlation_samples_per_marker": 128,
                "max_total_correlation_samples": 128,
                "max_total_window_frames": 34,
                "min_correlation": 0.85, "min_peak_margin": 0.15, "min_rms": 0.0001,
                "windows": [{"start_frame": 137}]
            });
            fs::write(&analysis, serde_json::to_vec(&analysis_plan).unwrap()).unwrap();
            fs::write(&source, source_plan_bytes()).unwrap();
            let mut args = source_test_args(&directory);
            args.extend([
                "--measurement-pcm".into(),
                pcm.to_string_lossy().into_owned(),
                "--schedule".into(),
                analysis.to_string_lossy().into_owned(),
            ]);
            if explicit {
                args.extend([
                    "--source-schedule".into(),
                    source.to_string_lossy().into_owned(),
                ]);
            }
            assert_eq!(run(args).unwrap(), 1);
            let report: Value =
                serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
            assert_eq!(
                report["artifacts"]["measurement_pcm_status"],
                "externally_supplied_and_analyzed"
            );
            let provenance = &report["source"]["playback_source"];
            if explicit {
                assert_eq!(provenance["mode"], "explicit_source_schedule");
                assert_eq!(
                    provenance["schedule"]["starts"],
                    serde_json::json!([0, 40000])
                );
                assert_eq!(
                    provenance["schedule"]["marker_amplitude"].as_f64().unwrap() as f32,
                    0.1
                );
            } else {
                assert_eq!(
                    provenance["mode"],
                    "diagnostic_pulses_no_source_marker_provenance"
                );
                assert!(provenance["schedule"].is_null());
                assert!(!directory.join("source-schedule.json").exists());
            }
            fs::remove_file(pcm).unwrap();
            fs::remove_file(analysis).unwrap();
            fs::remove_file(source).unwrap();
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn source_reserved_path_rejects_output_aliases_before_creation() {
        for manifest in [false, true] {
            let directory = temp_dir(&format!("source-reserved-{manifest}"));
            let reserved = directory.join("child/../source-schedule.json");
            let mut options = LiveOptions {
                artifact_dir: directory.clone(),
                ..Default::default()
            };
            if manifest {
                options.manifest_path = Some(reserved);
            } else {
                options.report_path = Some(reserved);
            }
            let result = prepare_paths(&options);
            let created = directory.exists();
            if created {
                fs::remove_dir_all(&directory).unwrap();
            }
            assert!(result.is_err(), "source artifact path must be reserved");
            assert!(!created, "reject aliases before creating outputs");
        }
    }

    #[test]
    fn source_without_plan_dry_run_labels_pulses_without_marker_provenance() {
        let directory = temp_dir("source-diagnostic-provenance");
        let result = run(vec![
            "--dry-run".into(),
            "--source-kind".into(),
            "vad-playback".into(),
            "--stable-endpoint-id".into(),
            "07070707070707070707070707070707".into(),
            "--name".into(),
            "Source test".into(),
            "--output".into(),
            "out".into(),
            "--artifact-dir".into(),
            directory.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert_eq!(result, 1);
        let report: Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        assert_eq!(
            report["source"]["playback_source"]["mode"],
            "diagnostic_pulses_no_source_marker_provenance"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn soak_reserved_journal_path_rejects_report_and_manifest_aliases_before_creation() {
        for source_kind in [
            EvidenceSourceKind::ProcessLoopback,
            EvidenceSourceKind::VadPlayback,
        ] {
            for manifest in [false, true] {
                for spelling in [
                    "observations.jsonl",
                    "./observations.jsonl",
                    "child/../observations.jsonl",
                ]
                .into_iter()
                .chain(if cfg!(windows) {
                    Some("OBSERVATIONS.JSONL")
                } else {
                    None
                }) {
                    let directory = temp_dir(
                        &format!("soak-reserved-{source_kind:?}-{manifest}-{spelling:?}")
                            .replace(['/', '\\', ':', '"'], "-"),
                    );
                    let reserved = directory.join(spelling);
                    let mut options = LiveOptions {
                        source_kind,
                        artifact_dir: directory.clone(),
                        diagnostic_soak: true,
                        ..Default::default()
                    };
                    if manifest {
                        options.manifest_path = Some(reserved);
                    } else {
                        options.report_path = Some(reserved);
                    }
                    let result = prepare_paths(&options);
                    let created = directory.exists();
                    if created {
                        fs::remove_dir_all(&directory).unwrap();
                    }
                    assert!(result.is_err(), "reserved journal path was admitted");
                    assert!(!created, "collision must fail before creating artifacts");
                }
            }
        }
    }

    #[test]
    fn soak_reserved_journal_path_does_not_restrict_ordinary_live_outputs() {
        let directory = temp_dir("normal-reserved-name");
        let options = LiveOptions {
            artifact_dir: directory.clone(),
            report_path: Some(directory.join("observations.jsonl")),
            ..Default::default()
        };
        let paths = prepare_paths(&options).unwrap();
        assert!(paths.diagnostic_observations.is_none());
        assert!(paths.capture_pcm.is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn diagnostic_soak_is_explicit_bounded_and_never_claims_pcm_or_qualification() {
        let directory = temp_dir("diagnostic-soak");
        let code = run(vec![
            "--dry-run".into(),
            "--diagnostic-soak".into(),
            "--seconds".into(),
            "7200".into(),
            "--source-pid".into(),
            "1".into(),
            "--output".into(),
            "out".into(),
            "--artifact-dir".into(),
            directory.to_string_lossy().into_owned(),
        ])
        .unwrap();
        assert_eq!(code, 1);
        let report: Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(report["qualified"], false);
        assert_eq!(manifest["diagnostic_soak"], true);
        assert_eq!(manifest["observation_interval_seconds"], 5);
        assert!(report["artifacts"]["paths"]["capture_pcm"].is_null());
        assert_eq!(
            report["artifacts"]["capture_pcm_status"],
            "disabled_diagnostic_soak"
        );
        assert!(report["gaps"].as_array().unwrap().iter().any(|gap| {
            gap.as_str()
                .unwrap()
                .contains("does not qualify continuous waveform")
        }));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn diagnostic_soak_rejects_oversized_duration_and_unbounded_observation_frequency() {
        let base = vec![
            "--source-pid".into(),
            "1".into(),
            "--output".into(),
            "out".into(),
            "--artifact-dir".into(),
            "unused".into(),
        ];
        for extra in [
            vec!["--seconds", "601"],
            vec!["--diagnostic-soak", "--seconds", "7201"],
            vec!["--diagnostic-soak", "--observation-interval-seconds", "0"],
            vec!["--diagnostic-soak", "--observation-interval-seconds", "4"],
            vec!["--diagnostic-soak", "--observation-interval-seconds", "61"],
        ] {
            let mut args = base.clone();
            args.extend(extra.into_iter().map(str::to_owned));
            assert!(parse(args).and_then(|options| validate(&options)).is_err());
        }
        for seconds in [1, 600, 1800, 7200] {
            let mut args = base.clone();
            args.extend([
                "--diagnostic-soak".into(),
                "--seconds".into(),
                seconds.to_string(),
            ]);
            let options = parse(args).unwrap();
            validate(&options).unwrap();
            assert!(options.diagnostic_soak);
            assert_eq!(options.seconds, seconds);
        }
    }

    #[test]
    fn parser_requires_explicit_source_output_and_artifact_directory() {
        assert!(run(vec!["--dry-run".into()]).is_err());
        assert!(
            run(vec![
                "--source-pid".into(),
                "1".into(),
                "--output".into(),
                "out".into()
            ])
            .is_err()
        );
    }

    #[test]
    fn dry_run_writes_versioned_unqualified_manifest_and_report() {
        let directory = temp_dir("report");
        let code = run(vec![
            "--dry-run".into(),
            "--source-pid".into(),
            "1".into(),
            "--output".into(),
            "render-endpoint".into(),
            "--artifact-dir".into(),
            directory.to_string_lossy().into_owned(),
            "--run-id".into(),
            "test-run".into(),
            "--measurement-boundary".into(),
            "internal".into(),
        ])
        .unwrap();
        assert_eq!(code, 1);
        let report: Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["mode"], "live_process_loopback");
        assert_eq!(report["status"], "unqualified");
        assert_eq!(report["qualified"], false);
        assert_eq!(report["measurement_boundary"], "internal");
        assert!(
            report["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| { gap.as_str().unwrap().contains("capture PCM artifact") })
        );
        assert_eq!(manifest["source_pid"], 1);
        assert_eq!(manifest["output_endpoint_id"], "render-endpoint");
        assert_eq!(
            manifest["artifacts"]["report"],
            directory.join("report.json").to_string_lossy().as_ref()
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn measurement_options_are_paired_and_boundary_is_explicit() {
        let options = LiveOptions {
            source_pid: 1,
            output_endpoint: "out".into(),
            artifact_dir: temp_dir("validate"),
            measurement_pcm: Some("capture.raw".into()),
            ..Default::default()
        };
        assert!(validate(&options).is_err());
        assert_eq!(
            parse_boundary("physical-cable").unwrap(),
            EvidenceBoundary::PhysicalCable
        );
        assert!(parse_boundary("physical").is_err());
    }

    #[test]
    fn process_loopback_accepts_supervised_controlled_load() {
        let options = LiveOptions {
            source_pid: 1,
            output_endpoint: "out".into(),
            artifact_dir: temp_dir("process-controlled-load"),
            controlled_load_id: Some("process-load".into()),
            ..Default::default()
        };
        validate(&options).unwrap();
    }

    #[test]
    fn process_loopback_controlled_load_is_retained_in_native_report() {
        let native = NativeReport {
            controlled_load: Some(serde_json::json!({
                "load_id": "process-load",
                "window_complete": false,
                "valid": false,
                "failure": "controlled load did not cover the measured collection window"
            })),
            ..Default::default()
        };
        let value = serde_json::to_value(native).unwrap();
        assert_eq!(value["controlled_load"]["load_id"], "process-load");
        assert_eq!(value["controlled_load"]["valid"], false);
    }

    #[test]
    fn target_bridge_is_marked_declared_and_preserves_requested_frames() {
        let options = LiveOptions {
            input_target_frames: Some(1024),
            ..Default::default()
        };
        let bridge = bridge_options(&options).unwrap();
        assert_eq!(
            bridge.input,
            TransportInputPolicy::Target {
                capacity_frames: 8192,
                target_frames: 1024,
                jitter_frames: 96,
            }
        );
        assert_eq!(
            bridge.delivery.unwrap().source,
            DeliveryTimingSource::Declared
        );
    }

    #[test]
    fn vad_boundary_xruns_fail_quality_even_without_driver_drops() {
        let native = NativeReport {
            vad_provider: Some(serde_json::json!({
                "telemetry": {
                    "packet_frames": 480,
                    "dropped_packets": 0,
                    "silence_frames": 0,
                    "epoch_resets": 0
                }
            })),
            engine: Some(serde_json::json!({
                "observations": {
                    "boundary_probes": [
                        {"kind": "virtual_playback", "xruns": 11},
                        {"kind": "virtual_microphone", "xruns": 2}
                    ]
                }
            })),
            ..NativeReport::default()
        };
        let quality = native_quality(&native);
        assert_eq!(quality.discontinuities, 13);
        // The probe reports events, so it cannot invent an underrun frame count.
        assert_eq!(quality.underrun_frames, 0);
        assert_eq!(quality.dropped_frames, 0);
    }

    #[test]
    fn boundary_xrun_quality_saturates_instead_of_wrapping() {
        let native = NativeReport {
            engine: Some(serde_json::json!({
                "observations": {
                    "boundary_probes": [
                        {"xruns": u64::MAX},
                        {"xruns": 1}
                    ]
                }
            })),
            ..NativeReport::default()
        };
        assert_eq!(native_quality(&native).discontinuities, u64::MAX);
    }

    #[test]
    fn process_quality_preserves_bridge_loss_shortfall_and_resets() {
        let bridge = moiren_windows_audio::clock_bridge::BridgeSnapshot {
            dropped_frames: 7,
            silent_frames: 11,
            underrun_frames: 13,
            discontinuities: 17,
            resets: 19,
            backlog_discarded_frames: 23,
            age_discarded_frames: 29,
            ..Default::default()
        };
        let native = NativeReport {
            bridge: Some(serde_json::to_value(bridge).unwrap()),
            render: Some(serde_json::json!({
                "stats": {"bridge_shortfall_frames": 31}
            })),
            ..NativeReport::default()
        };
        let quality = native_quality(&native);
        assert_eq!(quality.dropped_frames, 59);
        assert_eq!(quality.underrun_frames, 44);
        assert_eq!(quality.discontinuities, 17);
        assert_eq!(quality.epoch_resets, 19);
        // SILENT input flags describe the application's content, whereas the
        // missing output frames are already counted as bridge underruns.
        assert_eq!(quality.silent_frames, 0);
    }

    #[test]
    fn native_shortfall_uses_one_render_report_and_keeps_other_faults() {
        let native = NativeReport {
            bridge: Some(serde_json::json!({
                "dropped_frames": 7,
                "resets": 2
            })),
            render: Some(serde_json::json!({
                "stats": {"bridge_shortfall_frames": 31}
            })),
            backend_diagnostics: Some(serde_json::json!({
                "render": {"stats": {"bridge_shortfall_frames": 31}}
            })),
            vad_provider: Some(serde_json::json!({
                "telemetry": {
                    "packet_frames": 64,
                    "dropped_packets": 2,
                    "silence_frames": 5,
                    "epoch_resets": 3
                }
            })),
            ..NativeReport::default()
        };
        let quality = native_quality(&native);
        assert_eq!(quality.underrun_frames, 31, "do not count a report twice");
        assert_eq!(quality.dropped_frames, 135);
        assert_eq!(quality.silent_frames, 5);
        assert_eq!(quality.epoch_resets, 5);
        let backend_only = NativeReport {
            render: None,
            ..native
        };
        assert_eq!(native_quality(&backend_only).underrun_frames, 31);
    }

    #[test]
    fn playback_sequence_holes_are_delivery_quality_not_invented_sample_loss() {
        let native = NativeReport {
            vad_provider: Some(serde_json::json!({
                "telemetry": {
                    "sequence_gap_observations": 2,
                    "missing_sequence_slots": 5
                }
            })),
            ..NativeReport::default()
        };
        let quality = native_quality(&native);
        assert_eq!(quality.delivery_bound_violations, 5);
        assert_eq!(quality.dropped_frames, 0);
        assert_eq!(quality.silent_frames, 0);
        assert_eq!(quality.epoch_resets, 0);
    }

    #[test]
    fn native_quality_sums_saturate_for_delivery_and_frame_faults() {
        let native = NativeReport {
            capture: Some(serde_json::json!({
                "delivery": {
                    "packet_bound_violations": u64::MAX,
                    "gap_bound_violations": 1
                }
            })),
            bridge: Some(serde_json::json!({
                "dropped_frames": u64::MAX,
                "backlog_discarded_frames": 1,
                "age_discarded_frames": 1,
                "underrun_frames": u64::MAX
            })),
            render: Some(serde_json::json!({
                "stats": {"bridge_shortfall_frames": 1}
            })),
            vad_provider: Some(serde_json::json!({
                "telemetry": {"recovery_events": u64::MAX, "stale_packets": 1}
            })),
            ..NativeReport::default()
        };
        let quality = native_quality(&native);
        assert_eq!(quality.delivery_bound_violations, u64::MAX);
        assert_eq!(quality.dropped_frames, u64::MAX);
        assert_eq!(quality.underrun_frames, u64::MAX);
    }

    #[test]
    fn source_silence_priming_and_empty_padding_are_not_invented_audio_faults() {
        let native = NativeReport {
            bridge: Some(serde_json::json!({
                "silent_frames": 480,
                "priming_frames": 1024,
                "prime_discarded_frames": 128
            })),
            render: Some(serde_json::json!({
                "stats": {"empty_padding_wakes": 7}
            })),
            ..NativeReport::default()
        };
        assert_eq!(native_quality(&native), QualificationQuality::default());
    }
}
