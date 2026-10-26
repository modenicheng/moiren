//! Live Process Loopback orchestration and evidence handoff.
//!
//! This module owns the probe's control-side lifecycle. PCM is copied into a
//! bounded owner-local recorder and written only after the native owner joins;
//! physical qualification still requires an independent measurement artifact
//! and calibration provenance.

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
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
#[path = "vad.rs"]
mod vad;

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
    source_kind: EvidenceSourceKind,
    source_pid: u32,
    stable_endpoint_id: Option<String>,
    source_name: Option<String>,
    output_endpoint: String,
    artifact_dir: PathBuf,
    report_path: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    measurement_boundary: EvidenceBoundary,
    measurement_pcm: Option<PathBuf>,
    measurement_schedule: Option<PathBuf>,
    calibration: Option<PathBuf>,
    seconds: u64,
    max_input_bytes: u64,
    input_capacity_frames: u32,
    input_target_frames: Option<u32>,
    input_jitter_frames: u32,
    max_packet_frames: u32,
    max_gap_frames: u32,
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
            source_kind: EvidenceSourceKind::ProcessLoopback,
            source_pid: 0,
            stable_endpoint_id: None,
            source_name: None,
            output_endpoint: String::new(),
            artifact_dir: PathBuf::new(),
            report_path: None,
            manifest_path: None,
            measurement_boundary: EvidenceBoundary::Internal,
            measurement_pcm: None,
            measurement_schedule: None,
            calibration: None,
            seconds: 5,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            input_capacity_frames: DEFAULT_INPUT_CAPACITY_FRAMES,
            input_target_frames: None,
            input_jitter_frames: DEFAULT_INPUT_JITTER_FRAMES,
            max_packet_frames: DEFAULT_MAX_PACKET_FRAMES,
            max_gap_frames: DEFAULT_MAX_GAP_FRAMES,
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

#[derive(Debug, Serialize)]
struct LiveRunManifest {
    schema_version: u32,
    mode: &'static str,
    run_id: String,
    status: &'static str,
    source_kind: EvidenceSourceKind,
    source_pid: u32,
    stable_endpoint_id: Option<String>,
    source_name: Option<String>,
    source_identity: Option<Value>,
    output_endpoint_id: String,
    measurement_boundary: EvidenceBoundary,
    requested_seconds: u64,
    diagnostic_soak: bool,
    observation_interval_seconds: u64,
    process_bridge: ProcessBridgeOptions,
    render_queue: RenderQueuePolicy,
    artifacts: ArtifactPaths,
    gaps: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LiveRunReport {
    schema_version: u32,
    mode: &'static str,
    run_id: String,
    status: &'static str,
    qualified: bool,
    source: SourceReport,
    output_endpoint_id: String,
    measurement_boundary: EvidenceBoundary,
    requested_seconds: u64,
    diagnostic_soak: bool,
    observation_interval_seconds: u64,
    observed_duration_seconds: f64,
    artifacts: ArtifactReport,
    native: NativeReport,
    qualification: Option<QualificationReport>,
    gaps: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SourceReport {
    source_kind: EvidenceSourceKind,
    pid: Option<u32>,
    stable_endpoint_id: Option<String>,
    requested_name: Option<String>,
    identity: Option<Value>,
}

#[derive(Debug, Default, Serialize)]
struct NativeReport {
    capture: Option<Value>,
    render: Option<Value>,
    bridge: Option<Value>,
    vad_playback: Option<Value>,
    vad_provider: Option<Value>,
    vad_cleanup: Option<Value>,
    engine: Option<Value>,
    engine_cleanup: Option<Value>,
    backend_diagnostics: Option<Value>,
    diagnostic_observations: u64,
    error: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct ArtifactPaths {
    directory: String,
    report: String,
    manifest: String,
    measurement_pcm: Option<String>,
    measurement_schedule: Option<String>,
    calibration: Option<String>,
    capture_pcm: Option<String>,
    capture_pcm_metadata: Option<String>,
    diagnostic_observations: Option<String>,
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
             Runs production Process Loopback capture -> Bus -> WASAPI render ownership.\n\
             The command writes a versioned manifest/report and returns 1 when evidence is\n+             missing or cannot qualify physical end-to-end latency. Use --dry-run for\n+             parser/artifact validation without opening Windows audio devices."
        );
        return Ok(0);
    }
    let options = parse(arguments)?;
    validate(&options)?;
    let paths = prepare_paths(&options)?;
    let run_id = options
        .run_id
        .clone()
        .unwrap_or_else(|| format!("live-{}-{}", std::process::id(), unix_seconds()));
    let mut gaps = Vec::new();
    if options.diagnostic_soak {
        gaps.push("bounded diagnostic soak does not qualify continuous waveform, controlled load, markers or physical latency; PCM recording is disabled".into());
    }
    if options.dry_run {
        gaps.push("live source/native capture was intentionally skipped (--dry-run)".into());
    }
    let mut source_identity = source_identity(&options, &mut gaps);
    let measurement = load_measurement(&options, &mut gaps)?;
    let process_bridge = bridge_options(&options)?;
    let render_queue = render_queue(&options)?;
    let mut native = NativeReport::default();
    let mut observed_duration_seconds = 0.0;

    #[cfg(windows)]
    if !options.dry_run && options.source_kind == EvidenceSourceKind::ProcessLoopback {
        let result = native::run(
            &options,
            &process_bridge,
            render_queue,
            &paths,
            &mut native,
            &mut gaps,
        );
        if let Err(error) = result {
            gaps.push(format!("live Process Loopback run failed: {error:#}"));
            native.error = Some(format!("{error:#}"));
        } else {
            observed_duration_seconds = observed_duration(&native);
        }
    }
    #[cfg(windows)]
    if !options.dry_run && options.source_kind == EvidenceSourceKind::VadPlayback {
        let result = native::run_vad(&options, &paths, &measurement, &mut native, &mut gaps);
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

    let artifacts = artifact_report(&paths, &options, &measurement);
    let qualification = build_qualification_report(
        &options,
        &run_id,
        source_identity.as_ref(),
        observed_duration_seconds,
        &native,
        &measurement,
    );
    let report = LiveRunReport {
        schema_version: LIVE_SCHEMA_VERSION,
        mode: mode_name(options.source_kind),
        run_id: run_id.clone(),
        status: "unqualified",
        qualified: false,
        source: SourceReport {
            source_kind: options.source_kind,
            pid: (options.source_pid != 0).then_some(options.source_pid),
            stable_endpoint_id: options.stable_endpoint_id.clone(),
            requested_name: options.source_name.clone(),
            identity: source_identity.clone(),
        },
        output_endpoint_id: options.output_endpoint.clone(),
        measurement_boundary: options.measurement_boundary,
        requested_seconds: options.seconds,
        diagnostic_soak: options.diagnostic_soak,
        observation_interval_seconds: options.observation_interval_seconds,
        observed_duration_seconds,
        artifacts,
        native,
        qualification,
        gaps: gaps.clone(),
    };
    let manifest = LiveRunManifest {
        schema_version: LIVE_SCHEMA_VERSION,
        mode: mode_name(options.source_kind),
        run_id,
        status: "unqualified",
        source_kind: options.source_kind,
        source_pid: options.source_pid,
        stable_endpoint_id: options.stable_endpoint_id.clone(),
        source_name: options.source_name.clone(),
        source_identity,
        output_endpoint_id: options.output_endpoint.clone(),
        measurement_boundary: options.measurement_boundary,
        requested_seconds: options.seconds,
        diagnostic_soak: options.diagnostic_soak,
        observation_interval_seconds: options.observation_interval_seconds,
        process_bridge,
        render_queue,
        artifacts: artifact_paths(&paths, &options),
        gaps,
    };
    write_json(&paths.report, &report)?;
    write_json(&paths.manifest, &manifest)?;
    Ok(1)
}

fn parse(arguments: Vec<String>) -> Result<LiveOptions> {
    let mut options = LiveOptions::default();
    let mut iter = arguments.into_iter();
    while let Some(flag) = iter.next() {
        let value = match flag.as_str() {
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
            | "--output"
            | "--artifact-dir"
            | "--report"
            | "--manifest"
            | "--measurement-boundary"
            | "--measurement-pcm"
            | "--schedule"
            | "--calibration"
            | "--seconds"
            | "--observation-interval-seconds"
            | "--max-input-bytes"
            | "--input-capacity-frames"
            | "--input-target-frames"
            | "--input-jitter-frames"
            | "--max-packet-frames"
            | "--max-gap-frames"
            | "--render-target-frames"
            | "--wake-margin-frames"
            | "--callback-budget-frames"
            | "--engine-block-frames"
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
            "--source-kind" => {
                options.source_kind = match value.as_str() {
                    "process-loopback" => EvidenceSourceKind::ProcessLoopback,
                    "vad-playback" => EvidenceSourceKind::VadPlayback,
                    "vad" => EvidenceSourceKind::Vad,
                    _ => bail!("--source-kind must be process-loopback or vad-playback"),
                }
            }
            "--source-pid" => options.source_pid = value.parse().context("invalid source PID")?,
            "--stable-endpoint-id" => options.stable_endpoint_id = Some(value),
            "--name" => options.source_name = Some(value),
            "--output" => options.output_endpoint = value,
            "--artifact-dir" => options.artifact_dir = PathBuf::from(value),
            "--report" => options.report_path = Some(PathBuf::from(value)),
            "--manifest" => options.manifest_path = Some(PathBuf::from(value)),
            "--measurement-boundary" => options.measurement_boundary = parse_boundary(&value)?,
            "--measurement-pcm" => options.measurement_pcm = Some(PathBuf::from(value)),
            "--schedule" => options.measurement_schedule = Some(PathBuf::from(value)),
            "--calibration" => options.calibration = Some(PathBuf::from(value)),
            "--seconds" => options.seconds = value.parse().context("invalid duration")?,
            "--observation-interval-seconds" => {
                options.observation_interval_seconds =
                    value.parse().context("invalid observation interval")?;
            }
            "--max-input-bytes" => {
                options.max_input_bytes = value.parse().context("invalid input byte cap")?
            }
            "--input-capacity-frames" => {
                options.input_capacity_frames = value.parse().context("invalid input capacity")?
            }
            "--input-target-frames" => {
                options.input_target_frames = Some(value.parse().context("invalid input target")?)
            }
            "--input-jitter-frames" => {
                options.input_jitter_frames = value.parse().context("invalid input jitter")?
            }
            "--max-packet-frames" => {
                options.max_packet_frames = value.parse().context("invalid packet bound")?
            }
            "--max-gap-frames" => {
                options.max_gap_frames = value.parse().context("invalid gap bound")?
            }
            "--render-target-frames" => {
                options.render_target_frames = if value == "capacity" {
                    None
                } else {
                    Some(value.parse().context("invalid render target")?)
                }
            }
            "--wake-margin-frames" => {
                options.wake_margin_frames = value.parse().context("invalid wake margin")?
            }
            "--callback-budget-frames" => {
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
    match options.source_kind {
        EvidenceSourceKind::ProcessLoopback | EvidenceSourceKind::Vad => {
            ensure!(options.source_pid != 0, "--source-pid must be nonzero");
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
        !options.output_endpoint.trim().is_empty() && !options.output_endpoint.contains('\0'),
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
            .map(|path| path.to_string_lossy().into_owned()),
        measurement_schedule: options
            .measurement_schedule
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
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
            calibration: self.calibration.clone(),
            capture_pcm: self.capture_pcm.clone(),
            capture_pcm_metadata: self.capture_pcm_metadata.clone(),
            diagnostic_observations: self.diagnostic_observations.clone(),
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
        output_endpoint_id: options.output_endpoint.clone(),
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

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(windows)]
mod native {
    use super::*;
    use anyhow::{Context, bail, ensure};
    use moiren_core::graph::{LogicalGraph, NodeKind, SendParams, edit::connect_to_new_bus_input};
    use moiren_engine::{
        boundary::{ClockRole, audio_bridge},
        compiler::{CompileConfig, NodeBindings, compile},
        host::{EngineHost, EngineProject, ExecutionState, HostConfig, NodeConfig},
        node::{
            DeviceOptions, EndpointSelector, InputConfig, InputSource, OutputConfig, OutputTarget,
            VirtualEndpointId,
        },
        runtime::EngineConfig,
    };
    use moiren_windows_audio::{
        ActivationGate, SessionDuration, StopSignal,
        engine_backend::{WindowsBackend, WindowsBackendConfig},
        latency::RenderQueuePolicy,
        process_loopback::{
            ProcessLoopbackOptions, inspect_process, prepare_process_capture_with_gate,
        },
        render::{DemandRenderer, RenderOptions, prepare_render_native_with_gate},
        vad::{Channels, Intent, PdoState, WindowsVadProvider},
        virtual_owner::VirtualBufferOptions,
    };
    use std::{
        str::FromStr,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    const STARTUP_SAMPLE_LIMIT: usize = 256;
    const STARTUP_SAMPLE_INTERVAL: Duration = Duration::from_millis(10);

    pub fn run(
        options: &LiveOptions,
        bridge: &ProcessBridgeOptions,
        queue: RenderQueuePolicy,
        paths: &ArtifactPaths,
        native_report: &mut NativeReport,
        gaps: &mut Vec<String>,
    ) -> Result<()> {
        let identity =
            inspect_process(options.source_pid).context("inspect selected source process")?;
        let duration = if options.diagnostic_soak {
            SessionDuration::UntilStopped
        } else {
            SessionDuration::For(Duration::from_secs(options.seconds))
        };
        let mut journal = diagnostic_journal(options, paths)?;
        let stop = StopSignal::new().context("create shared stop signal")?;
        let gate = ActivationGate::new(stop.clone()).context("create activation gate")?;
        let capture = prepare_process_capture_with_gate(
            ProcessLoopbackOptions {
                artifact: if options.diagnostic_soak {
                    None
                } else {
                    Some(moiren_windows_audio::capture::PcmArtifactConfig::new(
                        usize::try_from(options.seconds)
                            .ok()
                            .and_then(|seconds| seconds.checked_mul(48_000))
                            .ok_or_else(|| anyhow::anyhow!("capture artifact size overflow"))?,
                    ))
                },
                bridge: *bridge,
                latency_telemetry: options.telemetry,
                target: identity,
                duration,
            },
            stop.clone(),
            gate.clone(),
        )
        .context("prepare Process Loopback capture")?;
        let render = prepare_render_native_with_gate(
            RenderOptions {
                queue,
                endpoint_id: options.output_endpoint.clone(),
                latency_telemetry: options.telemetry,
                shared_period: Default::default(),
                duration,
            },
            stop.clone(),
            gate.clone(),
        )
        .context("prepare WASAPI render")?;
        let render_info = render.info();
        let padding = render.padding_observer();
        let bridge_observer = capture.observer.clone();
        let prepared = capture;
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2)?;
        let bus = graph.create_node(NodeKind::Bus, 2)?;
        let sink = graph.create_node(NodeKind::Sink, 2)?;
        let source_port = graph.get_node(source)?.outputs()[0].id();
        let bus_port = graph.get_node(bus)?.outputs()[0].id();
        let sink_port = graph.get_node(sink)?.inputs()[0].id();
        connect_to_new_bus_input(&mut graph, source_port, bus, SendParams::default())?;
        graph.connect(bus_port, sink_port, SendParams::default())?;
        let (writer, reader) = audio_bridge(2, 8192, 8 * 1024 * 1024)?;
        let mut bindings = NodeBindings::new();
        bindings.bind_source(source, prepared.source)?;
        bindings.bind_sink(sink, writer)?;
        let compiled = compile(
            &graph,
            bindings,
            CompileConfig {
                engine: EngineConfig {
                    processing_sr: 48_000.0,
                    max_block_frames: options.engine_block_frames as usize,
                    max_events_per_block: 64,
                },
                audio_byte_budget: 8 * 1024 * 1024,
                plan_revision: 1,
                timeline_epoch: 1,
                control_capacity: 64,
                control_horizon_frames: 48_000,
            },
        )
        .context("compile Input -> Bus -> Output graph")?;
        let renderer =
            DemandRenderer::new(compiled.engine, reader).context("prepare Engine renderer")?;
        let timeline = renderer.timeline_observer();
        let prepared_render = render
            .attach(renderer, queue)
            .context("attach Engine renderer")?;
        gate.activate().context("activate capture/render gate")?;
        let capture_session = prepared.session;
        let render_session = prepared_render.into_session();
        let collection = if let Some(journal) = journal.as_mut() {
            let duration = Duration::from_secs(options.seconds);
            let interval = Duration::from_secs(options.observation_interval_seconds);
            let mut next_observation = Duration::ZERO;
            soak_control::run_until(
                duration,
                |elapsed| {
                    journal.check()?;
                    if elapsed >= next_observation {
                        journal.record(&serde_json::json!({
                            "schema_version": 1, "source_kind": options.source_kind,
                            "elapsed_seconds": elapsed.as_secs_f64(),
                            "capture_started": capture_session.has_started(),
                            "render_started": render_session.has_started(),
                            "capture_finished": capture_session.is_finished(),
                            "render_finished": render_session.is_finished(),
                            "bridge": bridge_observer.snapshot(),
                            "render_padding": soak::process_padding_observation(&padding),
                            "timeline": timeline.try_snapshot().map(|value| serde_json::json!({
                                "frame": value.frame, "epoch": value.epoch, "revision": value.revision,
                            })),
                        }))?;
                        next_observation = elapsed.saturating_add(interval);
                    }
                    ensure!(
                        !capture_session.is_finished() && !render_session.is_finished(),
                        "native owner stopped before diagnostic deadline"
                    );
                    Ok(())
                },
                || stop.request_stop().context("stop diagnostic native owners"),
            )
        } else {
            Ok(())
        };
        let capture_result = capture_session.join();
        let render_result = render_session.join();
        match capture_result {
            Ok(report) => {
                if let Err(error) = persist_capture_artifact(paths, &report) {
                    gaps.push(format!("persist capture PCM artifact: {error:#}"));
                }
                native_report.capture = Some(serde_json::to_value(report)?);
            }
            Err(error) => gaps.push(format!("join Process Loopback capture: {error}")),
        }
        match render_result {
            Ok(report) => native_report.render = Some(serde_json::to_value(report)?),
            Err(error) => gaps.push(format!("join WASAPI render: {error}")),
        }
        // Keep the observer alive across both joins so this is the final
        // runtime state, rather than the all-zero pre-activation snapshot.
        native_report.bridge = Some(serde_json::to_value(bridge_observer.snapshot())?);
        // Keep the metadata read above in the report path even if the source
        // process exits during the finite run.
        if render_info.timing.capacity_frames == 0 {
            gaps.push("render reported zero native capacity".into());
        }
        let owners = if native_report.capture.is_none() || native_report.render.is_none() {
            Err(anyhow::anyhow!(
                "one or more native owners did not return a report"
            ))
        } else {
            Ok(())
        };
        // Preserve final owner/bridge reports before any wait on the sink.
        // A stalled OS write may delay exit, never native cancellation.
        let result = soak_cleanup::combine(collection, owners);
        soak_cleanup::combine(result, finish_journal(&mut journal, native_report))
    }

    fn finish_journal(
        journal: &mut Option<soak::Journal>,
        report: &mut NativeReport,
    ) -> Result<()> {
        let Some(journal) = journal else {
            return Ok(());
        };
        let result = journal.finish();
        report.diagnostic_observations = journal.samples();
        result
    }

    /// Run the production VAD playback -> Engine -> physical render path.
    ///
    /// The native VAD client is deliberately kept separate from EngineHost:
    /// it is the external application source, while the provider owns the
    /// driver's PCM worker and the backend owns the graph/render owner.
    pub fn run_vad(
        options: &LiveOptions,
        paths: &ArtifactPaths,
        measurement: &MeasurementEvidence,
        native_report: &mut NativeReport,
        _gaps: &mut Vec<String>,
    ) -> Result<()> {
        let mut journal = diagnostic_journal(options, paths)?;
        let stable_text = options
            .stable_endpoint_id
            .as_deref()
            .context("--stable-endpoint-id is required for live-vad")?;
        let endpoint_id = VirtualEndpointId::from_str(stable_text)
            .map_err(|_| anyhow::anyhow!("--stable-endpoint-id must be exactly 32 hex digits"))?;
        let name = options
            .source_name
            .as_deref()
            .context("--name is required for live-vad")?;
        let (_, root) = vad::discover_root().context("discover VAD control root")?;
        let provider = WindowsVadProvider::discover().context("discover VAD provider")?;
        // From CREATE onward every failure takes the same best-effort cleanup
        // path. The journal was declared first, so even unwinding stops native
        // owners before joining a potentially stalled filesystem worker.
        let mut cleanup = VadCleanup {
            provider: &provider,
            endpoint_id,
            playback: None,
            host: None,
            observer: None,
            observations: None,
            backend_diagnostics: None,
            playback_info: None,
            startup_samples: Vec::with_capacity(STARTUP_SAMPLE_LIMIT),
            report: native_report,
        };
        let collection = (|| -> Result<()> {
            provider
                .create_export(endpoint_id, Intent::Playback, Channels::Stereo, name, false)
                .context("CREATE VAD playback export")?;
            wait_vad_ready(&provider, endpoint_id)?;
            let marker_plan = measurement
                .analysis
                .as_ref()
                .map(|analysis| vad::MarkerPlan::from_schedule(&analysis.schedule))
                .transpose()?;
            cleanup.playback = Some(
                vad::PlaybackSession::prepare(stable_from_virtual(endpoint_id), root, marker_plan)
                    .context("prepare VAD native playback")?,
            );
            let output = EndpointSelector::PinnedEndpoint {
                endpoint_id: options.output_endpoint.clone(),
                stable_id: None,
            };
            let mut project = minimal_vad_project(endpoint_id, output)?;
            project.processing.engine.max_block_frames = options.engine_block_frames as usize;
            let backend_config = WindowsBackendConfig {
                render_queue: render_queue(options)?,
                virtual_buffer: virtual_buffer(options)?,
                telemetry: options.telemetry,
                ..WindowsBackendConfig::default()
            };
            let backend = WindowsBackend::new(backend_config)
                .context("prepare Windows backend")?
                .with_virtual_provider(provider.clone());
            cleanup.backend_diagnostics = Some(backend.diagnostics_handle());
            cleanup.observations = Some(backend.observations());
            let host = EngineHost::open(HostConfig::default(), project, backend)
                .context("open VAD EngineHost")?;
            cleanup.observer = Some(host.observer());
            cleanup.host = Some(host);
            let observer = cleanup.observer.as_ref().expect("prepared host observer");
            let observations = cleanup
                .observations
                .as_ref()
                .expect("prepared native observations");
            wait_engine_running(observer)?;
            let active = observer.snapshot();
            let revision = active
                .active_revision
                .context("VAD EngineHost has no active revision")?;
            let current = provider
                .export_observations()
                .into_iter()
                .find(|view| view.endpoint_id == endpoint_id)
                .and_then(|view| view.snapshot)
                .context("VAD export disappeared before gate admission")?;
            ensure!(
                current.state() == PdoState::Present,
                "VAD export is not Present"
            );
            ensure!(
                current.policy_prepared() && current.audio_ready(),
                "VAD export is not audio-ready"
            );
            let ticket = provider.set_active_revision(Some(revision));
            provider
                .set_export_gate(endpoint_id, snapshot_stamp(current), true, revision, ticket)
                .context("open VAD export gate")?;
            let playback = cleanup.playback.as_ref().expect("prepared VAD playback");
            playback.activate();
            cleanup.playback_info = Some(serde_json::to_value(&playback.info)?);
            // The control side owns the interval and samples while the source still
            // runs. Native client retirement advances stream generations and must
            // not overwrite these running observations with cleanup counters.
            // Retain a bounded startup trace to distinguish initial admission from
            // later starvation. Sampling and serialization stay on this control
            // thread; the final runtime counters still cover the entire interval.
            let duration = Duration::from_secs(options.seconds);
            let observation_interval = Duration::from_secs(options.observation_interval_seconds);
            let mut next_observation = Duration::ZERO;
            soak_control::run_until(
                duration,
                |elapsed| {
                    if let Some(journal) = journal.as_ref() {
                        journal.check()?;
                    }
                    if elapsed >= next_observation
                        && let Some(journal) = journal.as_mut()
                    {
                        let runtime = provider
                            .export_observations()
                            .into_iter()
                            .find(|view| view.endpoint_id == endpoint_id);
                        let engine = observer.snapshot();
                        journal.record(&serde_json::json!({
                            "schema_version": 1, "source_kind": options.source_kind,
                            "elapsed_seconds": elapsed.as_secs_f64(),
                            "provider": runtime.as_ref().map(vad_observation_json),
                            "engine": engine_snapshot_value(&engine),
                            "native_observations": observations.snapshot(),
                        }))?;
                        ensure!(
                            runtime.as_ref().is_some_and(|view| view.error.is_none()),
                            "VAD provider failed during diagnostic collection"
                        );
                        ensure!(
                            engine.execution == ExecutionState::Running
                                && engine.active_revision == Some(revision),
                            "Engine stopped or revision changed during diagnostic collection"
                        );
                        next_observation = elapsed.saturating_add(observation_interval);
                    }
                    if cleanup.startup_samples.len() < STARTUP_SAMPLE_LIMIT {
                        let runtime = provider
                            .export_observations()
                            .into_iter()
                            .find(|view| view.endpoint_id == endpoint_id);
                        let native = observations.snapshot();
                        cleanup.startup_samples.push(serde_json::json!({
                            "elapsed_seconds": elapsed.as_secs_f64(),
                            "provider": runtime.as_ref().map(vad_observation_json),
                            "boundary_probes": native.map(|snapshot| snapshot.boundary_probes),
                        }));
                    }
                    Ok(())
                },
                || Ok(()),
            )
        })();
        let result = soak_cleanup::run(collection, &mut cleanup);
        drop(cleanup);
        soak_cleanup::combine(result, finish_journal(&mut journal, native_report))
    }

    struct VadCleanup<'a> {
        provider: &'a WindowsVadProvider,
        endpoint_id: VirtualEndpointId,
        playback: Option<vad::PlaybackSession>,
        host: Option<EngineHost<f32>>,
        observer: Option<moiren_engine::host::EngineObserver>,
        observations: Option<moiren_windows_audio::engine_backend::NativeObservations>,
        backend_diagnostics: Option<Arc<Mutex<Option<String>>>>,
        playback_info: Option<Value>,
        startup_samples: Vec<Value>,
        report: &'a mut NativeReport,
    }

    impl soak_cleanup::Cleanup for VadCleanup<'_> {
        fn observe_final(&mut self) -> Result<()> {
            if let Some(observer) = &self.observer {
                let mut engine = engine_snapshot_value(&observer.snapshot());
                engine["observations"] = serde_json::json!(
                    self.observations
                        .as_ref()
                        .and_then(|handle| handle.snapshot())
                );
                engine["playback_info"] = serde_json::json!(self.playback_info);
                engine["startup_trace"] = serde_json::json!({
                    "sample_limit": STARTUP_SAMPLE_LIMIT,
                    "sample_interval_ms": STARTUP_SAMPLE_INTERVAL.as_millis(),
                    "scope": "bounded startup diagnostics; final counters cover the full run",
                    "samples": self.startup_samples,
                });
                self.report.engine = Some(engine);
            }
            let observation = self
                .provider
                .export_observations()
                .into_iter()
                .find(|view| view.endpoint_id == self.endpoint_id)
                .context("VAD export disappeared during measurement")?;
            self.report.vad_provider = Some(vad_observation_json(&observation));
            if let Some(error) = observation.error {
                bail!("VAD final observation: {error}");
            }
            Ok(())
        }

        fn stop_source(&mut self) -> Result<()> {
            if let Some(playback) = self.playback.take() {
                let report = playback.stop().context("stop VAD native playback")?;
                self.report.vad_playback = Some(serde_json::to_value(report)?);
            }
            Ok(())
        }

        fn close_gate(&mut self) -> Result<()> {
            let view = self
                .provider
                .export_observations()
                .into_iter()
                .find(|view| view.endpoint_id == self.endpoint_id)
                .context("VAD export disappeared before gate close")?;
            let observed = view.error.map_or(Ok(()), |error| {
                Err(anyhow::anyhow!("VAD gate-close observation: {error}"))
            });
            let closed = (|| {
                let current = view
                    .snapshot
                    .context("VAD gate-close snapshot is unknown")?;
                if current.gate_open() {
                    self.provider
                        .set_export_gate(self.endpoint_id, snapshot_stamp(current), false, 0, 0)
                        .context("close VAD export gate")?;
                }
                Ok(())
            })();
            soak_cleanup::combine(observed, closed)
        }

        fn shutdown_host(&mut self) -> Result<()> {
            drop(self.host.take());
            self.report.backend_diagnostics = self
                .backend_diagnostics
                .as_ref()
                .and_then(diagnostics_value);
            if let Some(observer) = &self.observer {
                let snapshot = observer.snapshot();
                self.report.engine_cleanup = Some(engine_snapshot_value(&snapshot));
                if let Some(error) = snapshot.cleanup_error {
                    bail!("VAD EngineHost cleanup: {error}");
                }
                ensure!(
                    snapshot.host == moiren_engine::host::HostState::Closed,
                    "VAD EngineHost did not confirm Closed after shutdown"
                );
            }
            Ok(())
        }

        fn invalidate_admission(&mut self) -> Result<()> {
            self.provider.set_active_revision(None);
            Ok(())
        }

        fn retire_endpoint(&mut self) -> Result<()> {
            let observation = retire_vad(
                self.provider,
                self.endpoint_id,
                &mut self.report.vad_cleanup,
            )?;
            if let Some(error) = observation.error {
                bail!("VAD provider reported cleanup error: {error}");
            }
            Ok(())
        }
    }

    fn stable_from_virtual(id: VirtualEndpointId) -> moiren_windows_audio::vad::StableEndpointId {
        moiren_windows_audio::vad::StableEndpointId::from_virtual_endpoint_id(id)
    }

    fn vad_observation_json(
        observation: &moiren_windows_audio::vad::VadExportObservation,
    ) -> Value {
        serde_json::json!({
            "endpoint_id": observation.endpoint_id,
            "spec_name": observation.spec_name,
            "transport_prepared": observation.transport_prepared,
            "removing": observation.removing,
            "diagnostic_provenance": format!("{:?}", observation.diagnostic_provenance),
            "telemetry": observation.telemetry,
            "error": observation.error,
            "snapshot": observation.snapshot.map(|snapshot| serde_json::json!({
                "state": format!("{:?}", snapshot.state()),
                "gate_open": snapshot.gate_open(),
                "policy_prepared": snapshot.policy_prepared(),
                "audio_ready": snapshot.audio_ready(),
                "packet_frames": snapshot.packet_frames(),
                "stream_generation": snapshot.stream_generation(),
                "gate_epoch": snapshot.gate_epoch(),
                "dropped_packets": snapshot.dropped_packets(),
                "silence_packets": snapshot.silence_packets(),
            })),
        })
    }

    fn snapshot_stamp(snapshot: moiren_windows_audio::vad::EndpointSnapshot) -> [u64; 5] {
        [
            snapshot.pdo_token().get(),
            snapshot.session_epoch(),
            snapshot.endpoint_generation(),
            snapshot.stream_generation(),
            snapshot.gate_epoch(),
        ]
    }

    fn wait_vad_ready(
        provider: &WindowsVadProvider,
        endpoint_id: VirtualEndpointId,
    ) -> Result<moiren_windows_audio::vad::EndpointSnapshot> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(view) = provider
                .export_observations()
                .into_iter()
                .find(|view| view.endpoint_id == endpoint_id)
            {
                if let Some(snapshot) = view.snapshot
                    && snapshot.state() == PdoState::Present
                    && snapshot.policy_prepared()
                    && snapshot.audio_ready()
                    && view.error.is_none()
                {
                    return Ok(snapshot);
                }
                if let Some(error) = view.error {
                    bail!("VAD CREATE failed: {error}");
                }
            }
            ensure!(
                Instant::now() < deadline,
                "VAD export did not become audio-ready"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_engine_running(observer: &moiren_engine::host::EngineObserver) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let snapshot = observer.snapshot();
            if snapshot.execution == ExecutionState::Running
                && snapshot.active_revision == Some(1)
                && snapshot.timeline.is_some_and(|timeline| timeline.frame > 0)
            {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "VAD EngineHost did not reach Running; last snapshot: {:?}",
                observer.snapshot()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn diagnostics_value(handle: &Arc<Mutex<Option<String>>>) -> Option<Value> {
        let raw = handle
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()?;
        serde_json::from_str(&raw).ok().or_else(|| {
            Some(serde_json::json!({
                "raw": raw,
            }))
        })
    }

    fn engine_snapshot_value(snapshot: &moiren_engine::host::HostSnapshot) -> Value {
        serde_json::json!({
            "host": format!("{:?}", snapshot.host),
            "execution": format!("{:?}", snapshot.execution),
            "desired_revision": snapshot.desired_revision,
            "active_revision": snapshot.active_revision,
            "plan_revision": snapshot.plan_revision,
            "owner_generation": snapshot.owner_generation,
            "resource_generation": snapshot.resource_generation,
            "timeline": snapshot.timeline.map(|timeline| serde_json::json!({
                "epoch": timeline.epoch,
                "revision": timeline.revision,
                "frame": timeline.frame,
            })),
            "update": format!("{:?}", snapshot.update),
            "fault": format!("{:?}", snapshot.fault),
            "cleanup_error": format!("{:?}", snapshot.cleanup_error),
        })
    }

    fn virtual_buffer(options: &LiveOptions) -> Result<VirtualBufferOptions> {
        let bridge = bridge_options(options)?;
        Ok(VirtualBufferOptions {
            input: bridge.input,
            delivery: bridge.delivery,
        })
    }

    fn minimal_vad_project(
        endpoint_id: VirtualEndpointId,
        output: EndpointSelector,
    ) -> Result<EngineProject> {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2)?;
        let bus = graph.create_node(NodeKind::Bus, 2)?;
        let sink = graph.create_node(NodeKind::Sink, 2)?;
        let source_port = graph.get_node(source)?.outputs()[0].id();
        let bus_port = graph.get_node(bus)?.outputs()[0].id();
        let sink_port = graph.get_node(sink)?.inputs()[0].id();
        connect_to_new_bus_input(&mut graph, source_port, bus, SendParams::default())?;
        graph.connect(bus_port, sink_port, SendParams::default())?;
        let mut project = EngineProject {
            graph,
            ..EngineProject::default()
        };
        project.nodes.insert(
            source,
            NodeConfig::Input(InputConfig {
                source: InputSource::VirtualPlayback { endpoint_id },
                channels: 2,
                device: DeviceOptions::default(),
            }),
        );
        project.nodes.insert(bus, NodeConfig::Bus);
        project.nodes.insert(
            sink,
            NodeConfig::Output(OutputConfig {
                target: OutputTarget::Physical(output),
                channels: 2,
                device: DeviceOptions {
                    clock_role: ClockRole::Master,
                    ..DeviceOptions::default()
                },
            }),
        );
        project
            .validate()
            .context("validate VAD Input -> Bus -> Output project")?;
        Ok(project)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn minimal_vad_project_declares_physical_output_master() {
            let project = minimal_vad_project(
                VirtualEndpointId::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]),
                EndpointSelector::PinnedEndpoint {
                    endpoint_id: "render".into(),
                    stable_id: None,
                },
            )
            .unwrap();
            let output = project
                .nodes
                .values()
                .find_map(|node| match node {
                    NodeConfig::Output(output) => Some(output),
                    _ => None,
                })
                .expect("minimal VAD project output");
            assert_eq!(output.device.clock_role, ClockRole::Master);
        }
    }

    fn retire_vad(
        provider: &WindowsVadProvider,
        endpoint_id: VirtualEndpointId,
        last_observation: &mut Option<Value>,
    ) -> Result<moiren_windows_audio::vad::VadExportObservation> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let view = provider
                .export_observations()
                .into_iter()
                .find(|view| view.endpoint_id == endpoint_id)
                .context("VAD export missing during cleanup")?;
            *last_observation = Some(vad_observation_json(&view));
            let Some(snapshot) = view.snapshot else {
                ensure!(
                    Instant::now() < deadline,
                    "VAD cleanup snapshot became uncertain"
                );
                thread::sleep(Duration::from_millis(50));
                continue;
            };
            if snapshot.state() == PdoState::Removed {
                return Ok(view);
            }
            if snapshot.gate_open() {
                provider.set_export_gate(endpoint_id, snapshot_stamp(snapshot), false, 0, 0)?;
            } else if snapshot.state() != PdoState::Removing {
                provider.retire_export(endpoint_id, snapshot_stamp(snapshot))?;
            }
            ensure!(
                Instant::now() < deadline,
                "VAD cleanup did not reach Removed"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("moiren-live-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
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
        let mut options = LiveOptions::default();
        options.source_pid = 1;
        options.output_endpoint = "out".into();
        options.artifact_dir = temp_dir("validate");
        options.measurement_pcm = Some("capture.raw".into());
        assert!(validate(&options).is_err());
        assert_eq!(
            parse_boundary("physical-cable").unwrap(),
            EvidenceBoundary::PhysicalCable
        );
        assert!(parse_boundary("physical").is_err());
    }

    #[test]
    fn target_bridge_is_marked_declared_and_preserves_requested_frames() {
        let mut options = LiveOptions::default();
        options.input_target_frames = Some(1024);
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
