//! Offline evidence contracts for one source/configuration on a target machine.
//!
//! This module does not collect PCM, inspect devices/artifacts, authenticate
//! hashes, or establish calibration. Producers must supply genuine observations
//! and retain failed windows. Qualification checks the supplied evidence against
//! independently supplied current expectations, never against queue estimates.
//! Evaluate VAD and Process Loopback separately under matching conditions; a
//! successful model evaluation does not complete the project's hardware gates.
use crate::latency::{MarkerCorrelation, frames_to_seconds};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const QUALIFICATION_SCHEMA_VERSION: u32 = 1;
const MAX_PROVENANCE_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSourceKind {
    /// Legacy VAD source label retained for reports written before the
    /// playback-specific source was introduced.
    Vad,
    /// Playback PCM submitted to a Moiren VAD endpoint.
    VadPlayback,
    ProcessLoopback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceBoundary {
    Internal,
    WasapiLoopback,
    /// Source sample generation to physical output, referenced by a calibrated
    /// electrical cable. Source-submit or render-submit timing is Internal.
    PhysicalCable,
    /// Source sample generation and physical output observed by a calibrated
    /// external dual-channel reference on a common sample clock.
    ExternalDualChannel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFormat {
    pub sample_rate: u32,
    pub channels: u16,
    /// Stable PCM representation, for example "f32" or "i16".
    pub sample_format: String,
}

/// Source identity details that are useful for comparing runs without making
/// a display name or a process id the authoritative identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentityEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_endpoint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mmdevice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceConfiguration {
    pub source_kind: EvidenceSourceKind,
    /// Stable endpoint or process-instance identity, not merely a display name.
    pub source_identity: String,
    /// Optional source-specific identity details. Older reports omit this
    /// field and remain valid under the versioned schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_details: Option<SourceIdentityEvidence>,
    pub output_endpoint_id: String,
    pub processing_format: EvidenceFormat,
    pub native_input_format: EvidenceFormat,
    pub native_output_format: EvidenceFormat,
    pub measurement_format: EvidenceFormat,
    /// Identity of the prepared Input -> Bus -> Output graph.
    pub graph_id: String,
    pub signal_id: String,
    pub controlled_load_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRunIdentity {
    pub run_id: String,
    pub source_revision: String,
    pub binary_hash: String,
    pub config_hash: String,
    pub machine_id: String,
    pub windows_build: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationProvenance {
    /// Calibrated electrical/external reference and retained calibration artifact.
    pub reference_id: String,
    pub evidence_id: String,
    pub output_endpoint_id: String,
    pub measurement_format: EvidenceFormat,
    /// QPC-only timing or unrelated sample clocks cannot satisfy this contract.
    pub common_sample_clock: bool,
    /// Nonnegative bound added to measured p95 for the 10 ms acceptance gate.
    pub uncertainty_ms: f64,
    /// Calibration is expired at this Unix-second deadline.
    pub valid_until_unix_seconds: u64,
    /// Hash of the retained calibration artifact, verified by the caller.
    pub calibration_hash: String,
}

/// Descriptive native-frame settings, deliberately unused by qualification.
/// None remains unknown; neither capacity nor target is an E2E measurement.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEvidence {
    pub requested_capacity_frames: Option<u32>,
    pub requested_target_frames: Option<u32>,
    pub effective_capacity_frames: Option<u32>,
    pub effective_target_frames: Option<u32>,
    pub actual_period_frames: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationQuality {
    pub dropped_frames: u64,
    pub silent_frames: u64,
    pub underrun_frames: u64,
    pub discontinuities: u64,
    pub epoch_resets: u64,
    pub marker_missing: u64,
    pub sample_gap_frames: u64,
    /// Runtime delivery-proof failures, including packet/gap bound violations.
    /// Regressed/unavailable clock observations must also be surfaced as faults
    /// by the producer; prepared assumptions alone are not empirical evidence.
    #[serde(default)]
    pub delivery_bound_violations: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MarkerDistribution {
    pub marker_count: u64,
    /// Rate used to convert detected sample delays into milliseconds.
    pub sample_rate: u32,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualificationReport {
    pub schema_version: u32,
    pub identity: EvidenceRunIdentity,
    /// Expected hash recorded at collection time. It must also match the
    /// independently supplied current hash when this report is evaluated.
    pub expected_config_hash: String,
    pub configuration: EvidenceConfiguration,
    pub boundary: EvidenceBoundary,
    pub calibration: Option<CalibrationProvenance>,
    pub input_queue: QueueEvidence,
    pub output_queue: QueueEvidence,
    /// Observed continuous window, never the requested run duration.
    pub observed_duration_seconds: f64,
    /// Actual observed frames in the measurement format's sample clock.
    pub observed_measurement_frames: u64,
    pub waveform_evidence_id: String,
    pub steady_state: bool,
    pub controlled_load: bool,
    /// All planned markers, including failed/missing detections.
    pub expected_marker_count: u64,
    pub distribution: Option<MarkerDistribution>,
    pub quality: QualificationQuality,
}

/// Current caller-verified identity/configuration, supplied independently of
/// archived reports. Comparing two stale hashes inside a report is insufficient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationExpectation {
    pub source_revision: String,
    pub binary_hash: String,
    pub current_config_hash: String,
    pub target_machine_id: String,
    pub windows_build: String,
    pub calibration_hash: String,
    /// Caller-supplied current evaluation time; no wall clock is read here.
    pub evaluated_at_unix_seconds: u64,
    pub configuration: EvidenceConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationLevel {
    /// Each submitted window must contain at least 30 continuous minutes.
    Experimental30Minutes,
    /// At least two unique runs, each at least two hours on the target machine.
    DefaultTwoHourRepeated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualificationError {
    #[error("no complete marker population")]
    MissingMarkers,
    #[error("sample rate must be nonzero")]
    InvalidSampleRate,
    #[error("caller evidence limit exceeded")]
    LimitExceeded,
    #[error("invalid marker detection")]
    InvalidDetection,
    #[error("unsupported qualification schema")]
    UnsupportedSchema,
    #[error("missing, invalid or oversized evidence metadata")]
    InvalidMetadata,
    #[error("report does not match the required configuration or target machine")]
    MismatchedConfiguration,
    #[error("source, binary or configuration identity is stale")]
    StaleIdentity,
    #[error("physical calibration provenance is missing or mismatched")]
    Uncalibrated,
    #[error("internal measurement cannot qualify physical end-to-end latency")]
    InternalOnly,
    #[error("observed continuous duration is insufficient")]
    InsufficientDuration,
    #[error("observed duration does not agree with measured PCM frames")]
    InvalidDuration,
    #[error("invalid latency distribution")]
    InvalidDistribution,
    #[error("quality counters contain a failure")]
    QualityFailure,
    #[error("measured p95 plus calibration uncertainty exceeds 10 ms")]
    LatencyTargetMissed,
    #[error("not enough independent runs for this qualification level")]
    InsufficientRepetitions,
    #[error("a repeated run ID is not independent evidence")]
    DuplicateRun,
}

/// Compute nearest-rank p50/p95/p99/max from accepted marker detections.
///
/// Rate and caller count cap are checked before scanning or allocating.
/// The complete supplied population is sorted off-thread using O(N) temporary
/// storage and O(N log N) time. Failed detections must be counted in the report's
/// expected-marker/quality fields, not silently omitted to improve percentiles.
pub fn marker_distribution(
    detections: &[MarkerCorrelation],
    sample_rate: u32,
    max_markers: usize,
) -> Result<MarkerDistribution, QualificationError> {
    if sample_rate == 0 {
        return Err(QualificationError::InvalidSampleRate);
    }
    if max_markers == 0 || detections.len() > max_markers {
        return Err(QualificationError::LimitExceeded);
    }
    if detections.is_empty() {
        return Err(QualificationError::MissingMarkers);
    }
    if detections.iter().any(|detection| {
        !detection.confidence.is_finite()
            || detection.confidence <= 0.0
            || detection.confidence > 1.0
            || !detection.peak_margin.is_finite()
            || detection.peak_margin <= 0.0
            || detection.peak_margin > detection.confidence
    }) {
        return Err(QualificationError::InvalidDetection);
    }
    let mut delays: Vec<_> = detections
        .iter()
        .map(|detection| detection.delay_frames)
        .collect();
    delays.sort_unstable();
    let percentile_ms = |percent: usize| {
        // Split the multiplication so even a large admitted count cannot
        // overflow while computing ceil(N * percent / 100).
        let rank = delays.len() / 100 * percent + (delays.len() % 100 * percent).div_ceil(100);
        // A nonzero rate was validated before allocating the delay storage.
        frames_to_seconds(delays[rank - 1], sample_rate).expect("validated sample rate") * 1000.0
    };
    Ok(MarkerDistribution {
        marker_count: u64::try_from(delays.len()).map_err(|_| QualificationError::LimitExceeded)?,
        sample_rate,
        p50_ms: percentile_ms(50),
        p95_ms: percentile_ms(95),
        p99_ms: percentile_ms(99),
        max_ms: percentile_ms(100),
    })
}

/// Evaluate every submitted run without filtering failed windows or mutating
/// evidence. No stored "qualified" flag is trusted or added to archived reports.
///
/// The caller caps run count before scans/allocation; provenance strings have
/// a 4096-byte limit each. Evaluation uses O(run count) auxiliary storage and
/// O(run count * log(run count)) time. Bound serialized input bytes separately
/// before deserializing untrusted reports; serde allocation precedes this evaluator.
/// p95 plus calibration uncertainty <= 10 ms is the gate; p50 <= 8 ms remains
/// an exploration target.
pub fn evaluate_qualification(
    reports: &[QualificationReport],
    expected: &QualificationExpectation,
    level: QualificationLevel,
    max_runs: usize,
) -> Result<(), QualificationError> {
    if max_runs == 0 || reports.len() > max_runs {
        return Err(QualificationError::LimitExceeded);
    }
    let (minimum_runs, minimum_seconds) = match level {
        QualificationLevel::Experimental30Minutes => (1, 1800.0),
        QualificationLevel::DefaultTwoHourRepeated => (2, 7200.0),
    };
    if reports.len() < minimum_runs {
        return Err(QualificationError::InsufficientRepetitions);
    }
    if ![
        expected.source_revision.as_str(),
        &expected.binary_hash,
        &expected.current_config_hash,
        &expected.target_machine_id,
        &expected.windows_build,
        &expected.calibration_hash,
    ]
    .into_iter()
    .all(valid_text)
        || !valid_configuration(&expected.configuration)
        || expected.configuration.processing_format.sample_rate != 48_000
        || expected.evaluated_at_unix_seconds == 0
    {
        return Err(QualificationError::InvalidMetadata);
    }
    let mut run_ids = BTreeSet::new();
    for report in reports {
        evaluate_run(report, expected, minimum_seconds)?;
        if !run_ids.insert(report.identity.run_id.as_str()) {
            return Err(QualificationError::DuplicateRun);
        }
    }
    Ok(())
}

fn evaluate_run(
    report: &QualificationReport,
    expected: &QualificationExpectation,
    minimum_seconds: f64,
) -> Result<(), QualificationError> {
    if report.schema_version != QUALIFICATION_SCHEMA_VERSION {
        return Err(QualificationError::UnsupportedSchema);
    }
    let identity = &report.identity;
    if ![
        identity.run_id.as_str(),
        &identity.source_revision,
        &identity.binary_hash,
        &identity.config_hash,
        &identity.machine_id,
        &identity.windows_build,
        &report.expected_config_hash,
        &report.waveform_evidence_id,
    ]
    .into_iter()
    .all(valid_text)
        || !valid_configuration(&report.configuration)
        || !report.steady_state
        || !report.controlled_load
    {
        return Err(QualificationError::InvalidMetadata);
    }
    if identity.source_revision != expected.source_revision
        || identity.binary_hash != expected.binary_hash
        || identity.config_hash != expected.current_config_hash
        || report.expected_config_hash != expected.current_config_hash
    {
        return Err(QualificationError::StaleIdentity);
    }
    if report.configuration != expected.configuration
        || identity.machine_id != expected.target_machine_id
        || identity.windows_build != expected.windows_build
    {
        return Err(QualificationError::MismatchedConfiguration);
    }
    if matches!(
        report.boundary,
        EvidenceBoundary::Internal | EvidenceBoundary::WasapiLoopback
    ) {
        return Err(QualificationError::InternalOnly);
    }
    let calibration = report
        .calibration
        .as_ref()
        .ok_or(QualificationError::Uncalibrated)?;
    if !valid_text(&calibration.reference_id)
        || !valid_text(&calibration.evidence_id)
        || !valid_text(&calibration.output_endpoint_id)
        || !valid_format(&calibration.measurement_format)
        || !calibration.common_sample_clock
        || !calibration.uncertainty_ms.is_finite()
        || calibration.uncertainty_ms < 0.0
        || calibration.valid_until_unix_seconds <= expected.evaluated_at_unix_seconds
        || !valid_text(&calibration.calibration_hash)
        || calibration.calibration_hash != expected.calibration_hash
        || calibration.output_endpoint_id != report.configuration.output_endpoint_id
        || calibration.measurement_format != report.configuration.measurement_format
    {
        return Err(QualificationError::Uncalibrated);
    }
    let observed_seconds = report.observed_duration_seconds;
    if !observed_seconds.is_finite() || observed_seconds <= 0.0 {
        return Err(QualificationError::InvalidDuration);
    }
    if observed_seconds < minimum_seconds {
        return Err(QualificationError::InsufficientDuration);
    }
    let rate = f64::from(report.configuration.measurement_format.sample_rate);
    let pcm_seconds = report.observed_measurement_frames as f64 / rate;
    // Allow one frame of rounding between a clock-based duration and PCM
    // count, while preventing requested duration from replacing observation.
    if (observed_seconds - pcm_seconds).abs() > 1.0 / rate {
        return Err(QualificationError::InvalidDuration);
    }
    if pcm_seconds < minimum_seconds {
        return Err(QualificationError::InsufficientDuration);
    }
    if report.quality != QualificationQuality::default() {
        return Err(QualificationError::QualityFailure);
    }
    let distribution = report
        .distribution
        .as_ref()
        .ok_or(QualificationError::MissingMarkers)?;
    if report.expected_marker_count == 0
        || distribution.marker_count == 0
        || distribution.marker_count != report.expected_marker_count
    {
        return Err(QualificationError::MissingMarkers);
    }
    let values = [
        distribution.p50_ms,
        distribution.p95_ms,
        distribution.p99_ms,
        distribution.max_ms,
    ];
    if distribution.sample_rate != report.configuration.measurement_format.sample_rate
        || values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || values.windows(2).any(|pair| pair[0] > pair[1])
    {
        return Err(QualificationError::InvalidDistribution);
    }
    if distribution.p95_ms + calibration.uncertainty_ms > 10.0 {
        return Err(QualificationError::LatencyTargetMissed);
    }
    Ok(())
}

fn valid_text(value: &str) -> bool {
    value.len() <= MAX_PROVENANCE_BYTES && !value.trim().is_empty()
}

fn valid_format(format: &EvidenceFormat) -> bool {
    format.sample_rate != 0 && format.channels != 0 && valid_text(&format.sample_format)
}

fn valid_configuration(configuration: &EvidenceConfiguration) -> bool {
    [
        configuration.source_identity.as_str(),
        &configuration.output_endpoint_id,
        &configuration.graph_id,
        &configuration.signal_id,
        &configuration.controlled_load_id,
    ]
    .into_iter()
    .all(valid_text)
        && [
            &configuration.processing_format,
            &configuration.native_input_format,
            &configuration.native_output_format,
            &configuration.measurement_format,
        ]
        .into_iter()
        .all(valid_format)
}
