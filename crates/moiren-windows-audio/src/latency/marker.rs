//! Portable mono marker generation and off-thread sample-alignment analysis.
//!
//! Prepare reference and bounded recording storage before streaming. Generation
//! only writes caller storage; correlation is deliberately offline, even though
//! it also needs no heap storage. A sample offset alone says nothing about the
//! measurement boundary, clock calibration, or physical end-to-end latency.
use thiserror::Error;

mod rate;
pub use rate::{
    MarkerRateCorrelation, MarkerRateSearchConfig, PreparedMarkerRateSearch,
    analyze_rate_aware_marker,
};

const MIN_MARKER_FRAMES: usize = 32;

/// Explicit memory/input and work admission limits; there is no unbounded mode.
///
/// Confidence, peak margin, and AC RMS thresholds must be finite in `(0, 1]`.
/// They are acceptance policy, not a calibrated probability of correctness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkerAnalysisConfig {
    pub max_marker_frames: usize,
    pub max_recording_frames: usize,
    /// Inclusive nonnegative sample offset from recording frame zero.
    pub max_delay_frames: usize,
    /// Maximum full-window search pairs:
    /// `reference.len() * (max_delay_frames + 1)`. Each pair costs constant
    /// work; validation/reference preparation and section verification add
    /// O(reference length + recording length). Rejected limits never scan PCM.
    pub max_correlation_samples: usize,
    /// Minimum absolute, mean-centered normalized correlation, also required
    /// in each quarter of the winning marker with consistent polarity.
    pub min_correlation: f64,
    /// Minimum difference between the best and second-best absolute scores.
    /// Every other searched offset competes, including adjacent frames and
    /// opposite polarity; a broad peak can therefore be rejected conservatively.
    pub min_peak_margin: f64,
    /// Minimum AC RMS for both the reference and a candidate recording window.
    pub min_rms: f64,
}

/// Alignment within the caller's search range, without a measurement label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkerCorrelation {
    pub delay_frames: usize,
    /// Absolute normalized correlation in `[0, 1]`, not a probability.
    pub confidence: f64,
    pub peak_margin: f64,
    pub inverted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MarkerAnalysisError {
    #[error("invalid marker analysis configuration")]
    InvalidConfig,
    #[error("marker needs at least 32 finite, unclipped frames and AC energy above the RMS floor")]
    InvalidMarker,
    #[error("caller analysis limit exceeded")]
    LimitExceeded,
    #[error("observation does not cover the entire requested delay range")]
    InsufficientObservation,
    #[error("reference or recording contains non-finite samples")]
    NonFiniteSamples,
    #[error("recording contains clipped samples")]
    ClippedRecording,
    #[error("recording contains no signal above the RMS floor")]
    SilentRecording,
    #[error("no marker matches the confidence threshold")]
    NoMatch,
    #[error("marker correlation peaks are ambiguous")]
    Ambiguous,
    #[error("sample rate must be nonzero")]
    InvalidSampleRate,
}

/// One deterministic, finite bipolar broadband marker, followed by silence.
///
/// Use a low amplitude such as `0.1`; this does not select an endpoint or make
/// claims about playback loudness. The fixed nonzero xorshift32 seed makes the
/// reference independent of callback chunking. This is a measurement signal,
/// not a cryptographic generator. New markers restart the same sequence, so
/// callers must space markers beyond the observation range to avoid ambiguity.
#[derive(Debug)]
pub struct MarkerGenerator {
    state: u32,
    remaining_frames: usize,
    amplitude: f32,
}

impl MarkerGenerator {
    /// Construct without allocation. Requires at least 32 frames and finite
    /// amplitude strictly between zero and full scale.
    pub fn new(frames: usize, amplitude: f32) -> Result<Self, MarkerAnalysisError> {
        if frames < MIN_MARKER_FRAMES
            || !amplitude.is_finite()
            || amplitude <= 0.0
            || amplitude >= 1.0
        {
            return Err(MarkerAnalysisError::InvalidMarker);
        }
        Ok(Self {
            state: 0x6d2b_79f5,
            remaining_frames: frames,
            amplitude,
        })
    }

    /// Overwrite mono output, zeroing the tail after marker completion.
    /// Returns the number of marker frames written. Empty output does not
    /// advance the sequence. No allocation, deallocation, locks, or IO.
    pub fn fill(&mut self, output: &mut [f32]) -> usize {
        let written = output.len().min(self.remaining_frames);
        for sample in &mut output[..written] {
            self.state ^= self.state << 13;
            self.state ^= self.state >> 17;
            self.state ^= self.state << 5;
            *sample = if self.state & 1 == 0 {
                self.amplitude
            } else {
                -self.amplitude
            };
        }
        output[written..].fill(0.0);
        self.remaining_frames -= written;
        written
    }
}

/// Search complete mono windows on the control/analysis thread.
///
/// Requires `reference.len() + max_delay_frames` recording frames so a short
/// capture cannot silently shrink the search or produce a partial-window hit.
/// Both slices must fit caller caps. All supplied samples are validated as
/// finite and below full scale, including recording frames beyond the search.
/// Only offsets `0..=max_delay_frames` compete; align the reference origin with
/// the recording origin before calling. Different sample clocks/resampling
/// need separate preparation; this detector does not estimate rate drift.
///
/// Complexity is O(reference length * searched offsets + recording length),
/// with constant auxiliary storage. Confidence and a runner-up margin reject
/// missing/ambiguous signals instead of returning a fabricated zero offset.
/// Each quarter must also have energy and a matching shape, preventing a high
/// global score from hiding substantial marker truncation. Small erasures or
/// distortion within the selected thresholds cannot be distinguished from noise.
pub fn analyze_marker(
    reference: &[f32],
    recording: &[f32],
    config: MarkerAnalysisConfig,
) -> Result<MarkerCorrelation, MarkerAnalysisError> {
    validate_limits(reference.len(), recording.len(), config)?;
    if reference.len() < MIN_MARKER_FRAMES {
        return Err(MarkerAnalysisError::InvalidMarker);
    }
    if recording.len() < reference.len() + config.max_delay_frames {
        return Err(MarkerAnalysisError::InsufficientObservation);
    }
    if reference.iter().chain(recording).any(|x| !x.is_finite()) {
        return Err(MarkerAnalysisError::NonFiniteSamples);
    }
    if reference.iter().any(|x| x.abs() >= 1.0) {
        return Err(MarkerAnalysisError::InvalidMarker);
    }
    if recording.iter().any(|x| x.abs() >= 1.0) {
        return Err(MarkerAnalysisError::ClippedRecording);
    }

    let frames = reference.len() as f64;
    let reference_mean = mean(reference);
    let reference_energy: f64 = reference
        .iter()
        .map(|sample| (f64::from(*sample) - reference_mean).powi(2))
        .sum();
    // Do not square the caller's floor: a tiny valid f64 threshold can
    // underflow to zero and turn a zero-energy normalization into NaN.
    if (reference_energy / frames).sqrt() < config.min_rms {
        return Err(MarkerAnalysisError::InvalidMarker);
    }

    let mut best = MarkerCorrelation {
        delay_frames: 0,
        confidence: 0.0,
        peak_margin: 0.0,
        inverted: false,
    };
    let mut runner_up: f64 = 0.0;
    let mut has_signal = false;
    for delay in 0..=config.max_delay_frames {
        let window = &recording[delay..delay + reference.len()];
        let window_mean = mean(window);
        let mut energy = 0.0;
        let mut dot = 0.0;
        // Two passes avoid subtracting large nearly equal sums of squares
        // when a quiet signal sits on a DC offset. f64 keeps f32 PCM stable.
        for (sample, marker) in window.iter().zip(reference) {
            let centered = f64::from(*sample) - window_mean;
            energy += centered * centered;
            dot += centered * (f64::from(*marker) - reference_mean);
        }
        if (energy / frames).sqrt() < config.min_rms {
            continue;
        }
        has_signal = true;
        let confidence = (dot / (reference_energy * energy).sqrt()).abs().min(1.0);
        if confidence > best.confidence {
            runner_up = best.confidence;
            best.delay_frames = delay;
            best.confidence = confidence;
            best.inverted = dot < 0.0;
        } else {
            runner_up = runner_up.max(confidence);
        }
    }
    if !has_signal {
        return Err(MarkerAnalysisError::SilentRecording);
    }
    if best.confidence < config.min_correlation {
        return Err(MarkerAnalysisError::NoMatch);
    }
    best.peak_margin = best.confidence - runner_up;
    if best.peak_margin < config.min_peak_margin {
        return Err(MarkerAnalysisError::Ambiguous);
    }
    let window = &recording[best.delay_frames..best.delay_frames + reference.len()];
    if !marker_sections_match(reference, window, best.inverted, config) {
        return Err(MarkerAnalysisError::NoMatch);
    }
    Ok(best)
}

fn marker_sections_match(
    reference: &[f32],
    window: &[f32],
    inverted: bool,
    config: MarkerAnalysisConfig,
) -> bool {
    let reference_mean = mean(reference);
    let window_mean = mean(window);
    let section_frames = reference.len() / 4;
    for section in 0..4 {
        let start = section * section_frames;
        let end = if section == 3 {
            reference.len()
        } else {
            start + section_frames
        };
        let mut reference_energy = 0.0;
        let mut energy = 0.0;
        let mut dot = 0.0;
        // Keep the whole-window DC estimate: even a locally constant section
        // of a valid bipolar marker still supplies useful polarity evidence.
        for (sample, marker) in window[start..end].iter().zip(&reference[start..end]) {
            let marker = f64::from(*marker) - reference_mean;
            let sample = f64::from(*sample) - window_mean;
            reference_energy += marker * marker;
            energy += sample * sample;
            dot += marker * sample;
        }
        let frames = (end - start) as f64;
        if (reference_energy / frames).sqrt() < config.min_rms
            || (energy / frames).sqrt() < config.min_rms
        {
            return false;
        }
        let signed_correlation = dot / (reference_energy * energy).sqrt();
        let aligned_correlation = if inverted {
            -signed_correlation
        } else {
            signed_correlation
        };
        if aligned_correlation < config.min_correlation {
            return false;
        }
    }
    true
}

fn mean(samples: &[f32]) -> f64 {
    samples.iter().map(|sample| f64::from(*sample)).sum::<f64>() / samples.len() as f64
}

fn validate_limits(
    reference_frames: usize,
    recording_frames: usize,
    config: MarkerAnalysisConfig,
) -> Result<(), MarkerAnalysisError> {
    if config.max_marker_frames == 0
        || config.max_recording_frames == 0
        || config.max_correlation_samples == 0
        || [
            config.min_correlation,
            config.min_peak_margin,
            config.min_rms,
        ]
        .iter()
        .any(|threshold| !threshold.is_finite() || *threshold <= 0.0 || *threshold > 1.0)
    {
        return Err(MarkerAnalysisError::InvalidConfig);
    }
    let delays = config
        .max_delay_frames
        .checked_add(1)
        .ok_or(MarkerAnalysisError::LimitExceeded)?;
    let work = reference_frames
        .checked_mul(delays)
        .ok_or(MarkerAnalysisError::LimitExceeded)?;
    let required_frames = reference_frames
        .checked_add(config.max_delay_frames)
        .ok_or(MarkerAnalysisError::LimitExceeded)?;
    if reference_frames > config.max_marker_frames
        || recording_frames > config.max_recording_frames
        || required_frames > config.max_recording_frames
        || work > config.max_correlation_samples
    {
        return Err(MarkerAnalysisError::LimitExceeded);
    }
    Ok(())
}

/// Convert a frame count using the explicit common sample rate. This changes
/// units only; it does not infer a measurement boundary or a clock correction.
pub fn frames_to_seconds(frames: usize, sample_rate: u32) -> Result<f64, MarkerAnalysisError> {
    if sample_rate == 0 {
        return Err(MarkerAnalysisError::InvalidSampleRate);
    }
    Ok(frames as f64 / f64::from(sample_rate))
}
