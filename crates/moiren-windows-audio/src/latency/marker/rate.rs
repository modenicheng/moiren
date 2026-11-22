//! Bounded offline template-rate search, never hardware clock calibration.
use super::{
    MIN_MARKER_FRAMES, MarkerAnalysisConfig, MarkerAnalysisError, MarkerCorrelation,
    marker_sections_match, mean, validate_limits,
};
use serde::{Deserialize, Serialize};

const PPM: i64 = 1_000_000;
const MAX_HYPOTHESES: usize = 129;

/// Positive ppm stretches a marker about its frame-zero onset. A finite grid
/// models linear interpolation only, not every SRC/filter or continuous drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerRateSearchConfig {
    pub min_rate_ppm: i32,
    pub max_rate_ppm: i32,
    pub rate_step_ppm: u32,
    pub max_hypotheses: usize,
    /// Sum of warped template lengths times (searched offsets + 1), including
    /// the second rate-score pass. Preparation, validation and section checks
    /// add linear overhead, never another onset search.
    pub max_total_correlation_samples: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedMarkerRateSearch {
    pub max_template_frames: usize,
    pub hypothesis_count: usize,
    pub correlation_samples: usize,
}

/// Integer common-recording onset plus an uncalibrated template hypothesis.
/// Rate ties/edge hits are retained, never presented as uniquely measured skew.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkerRateCorrelation {
    pub alignment: MarkerCorrelation,
    pub template_rate_ppm: i32,
    pub hypothesis_count: usize,
    pub rate_grid_step_ppm: u32,
    pub template_rate_peak_margin: f64,
    pub template_rate_ties: usize,
    pub grid_edge: bool,
    /// Search-grid resolution only, not a physical uncertainty bound.
    pub onset_resolution_frames: usize,
}

impl MarkerRateSearchConfig {
    /// Admit every hypothesis, full observation window and work bound before
    /// reading PCM or touching caller scratch. No allocation or hidden defaults.
    pub fn prepare(
        self,
        reference_frames: usize,
        config: MarkerAnalysisConfig,
    ) -> Result<PreparedMarkerRateSearch, MarkerAnalysisError> {
        if self.min_rate_ppm > self.max_rate_ppm
            || i64::from(self.min_rate_ppm) <= -PPM
            || self.rate_step_ppm == 0
            || !(1..=MAX_HYPOTHESES).contains(&self.max_hypotheses)
            || self.max_total_correlation_samples == 0
        {
            return Err(MarkerAnalysisError::InvalidConfig);
        }
        let span = (i64::from(self.max_rate_ppm) - i64::from(self.min_rate_ppm)) as u64;
        if !span.is_multiple_of(u64::from(self.rate_step_ppm)) {
            return Err(MarkerAnalysisError::InvalidConfig);
        }
        let count = span / u64::from(self.rate_step_ppm) + 1;
        if count > self.max_hypotheses as u64 {
            return Err(MarkerAnalysisError::LimitExceeded);
        }
        if reference_frames < MIN_MARKER_FRAMES {
            return Err(MarkerAnalysisError::InvalidMarker);
        }
        let mut prepared = PreparedMarkerRateSearch {
            max_template_frames: 0,
            hypothesis_count: count as usize,
            correlation_samples: 0,
        };
        let offsets = config
            .max_delay_frames
            .checked_add(2)
            .ok_or(MarkerAnalysisError::LimitExceeded)?;
        for index in 0..prepared.hypothesis_count {
            let length = template_length(reference_frames, self.rate(index))?;
            if length < MIN_MARKER_FRAMES {
                return Err(MarkerAnalysisError::InvalidMarker);
            }
            let window = length
                .checked_add(config.max_delay_frames)
                .ok_or(MarkerAnalysisError::LimitExceeded)?;
            validate_limits(length, window, config)?;
            prepared.max_template_frames = prepared.max_template_frames.max(length);
            prepared.correlation_samples = length
                .checked_mul(offsets)
                .and_then(|work| prepared.correlation_samples.checked_add(work))
                .ok_or(MarkerAnalysisError::LimitExceeded)?;
        }
        if reference_frames > config.max_marker_frames
            || prepared.correlation_samples > self.max_total_correlation_samples
        {
            return Err(MarkerAnalysisError::LimitExceeded);
        }
        Ok(prepared)
    }

    fn rate(self, index: usize) -> i32 {
        (i64::from(self.min_rate_ppm) + index as i64 * i64::from(self.rate_step_ppm)) as i32
    }
}

fn template_length(frames: usize, ppm: i32) -> Result<usize, MarkerAnalysisError> {
    let numerator = (frames as u128) * (PPM + i64::from(ppm)) as u128;
    usize::try_from(numerator.div_ceil(PPM as u128)).map_err(|_| MarkerAnalysisError::LimitExceeded)
}

fn warp(reference: &[f32], template: &mut [f32], ppm: i32) {
    let ratio = (PPM + i64::from(ppm)) as f64 / PPM as f64;
    for (index, sample) in template.iter_mut().enumerate() {
        let position = index as f64 / ratio;
        let low = position.floor() as usize;
        let fraction = (position - low as f64) as f32;
        // Zero extension is part of the full template, never capture padding.
        *sample = reference.get(low).copied().unwrap_or(0.0) * (1.0 - fraction)
            + reference.get(low + 1).copied().unwrap_or(0.0) * fraction;
    }
}

fn score(template: &[f32], window: &[f32], floor: f64) -> Option<(f64, bool)> {
    let template_mean = mean(template);
    let window_mean = mean(window);
    let mut template_energy = 0.0;
    let mut energy = 0.0;
    let mut dot = 0.0;
    for (marker, sample) in template.iter().zip(window) {
        let marker = f64::from(*marker) - template_mean;
        let sample = f64::from(*sample) - window_mean;
        template_energy += marker * marker;
        energy += sample * sample;
        dot += marker * sample;
    }
    if (template_energy / template.len() as f64).sqrt() < floor
        || (energy / template.len() as f64).sqrt() < floor
    {
        return None;
    }
    Some((
        (dot / (template_energy * energy).sqrt()).abs().min(1.0),
        dot < 0.0,
    ))
}

fn has_energy(samples: &[f32], floor: f64) -> bool {
    let average = mean(samples);
    let energy = samples
        .iter()
        .map(|sample| (f64::from(*sample) - average).powi(2))
        .sum::<f64>();
    (energy / samples.len() as f64).sqrt() >= floor
}

/// Offline-only search using caller scratch: max warped template frames and
/// `max_delay_frames + 1` onset scores. All supplied samples are checked. Every
/// distinct onset competes across all rates, including adjacent/weak peaks.
/// A rate grid cannot compensate time-varying drift within a marker, infer
/// hardware clocks, or justify subtracting drift from an end-to-end delay.
pub fn analyze_rate_aware_marker(
    reference: &[f32],
    recording: &[f32],
    template_scratch: &mut [f32],
    onset_scores: &mut [f64],
    config: MarkerAnalysisConfig,
    rates: MarkerRateSearchConfig,
) -> Result<MarkerRateCorrelation, MarkerAnalysisError> {
    let prepared = rates.prepare(reference.len(), config)?;
    let offsets = config.max_delay_frames + 1;
    if recording.len() > config.max_recording_frames
        || template_scratch.len() < prepared.max_template_frames
        || onset_scores.len() < offsets
    {
        return Err(MarkerAnalysisError::LimitExceeded);
    }
    if recording.len() < prepared.max_template_frames + config.max_delay_frames {
        return Err(MarkerAnalysisError::InsufficientObservation);
    }
    if reference
        .iter()
        .chain(recording)
        .any(|sample| !sample.is_finite())
    {
        return Err(MarkerAnalysisError::NonFiniteSamples);
    }
    if reference.iter().any(|sample| sample.abs() >= 1.0) {
        return Err(MarkerAnalysisError::InvalidMarker);
    }
    if recording.iter().any(|sample| sample.abs() >= 1.0) {
        return Err(MarkerAnalysisError::ClippedRecording);
    }
    let reference_mean = mean(reference);
    let energy = reference
        .iter()
        .map(|sample| (f64::from(*sample) - reference_mean).powi(2))
        .sum::<f64>();
    if (energy / reference.len() as f64).sqrt() < config.min_rms {
        return Err(MarkerAnalysisError::InvalidMarker);
    }
    onset_scores[..offsets].fill(0.0);
    let mut best = MarkerCorrelation {
        delay_frames: 0,
        confidence: 0.0,
        peak_margin: 0.0,
        inverted: false,
    };
    let mut best_rate = rates.min_rate_ppm;
    let mut has_signal = false;
    let mut valid_template = false;
    for index in 0..prepared.hypothesis_count {
        let ppm = rates.rate(index);
        let length = template_length(reference.len(), ppm)?;
        let template = &mut template_scratch[..length];
        warp(reference, template, ppm);
        if !has_energy(template, config.min_rms) {
            continue;
        }
        valid_template = true;
        for (delay, maximum) in onset_scores[..offsets].iter_mut().enumerate() {
            if let Some((confidence, inverted)) =
                score(template, &recording[delay..delay + length], config.min_rms)
            {
                has_signal = true;
                *maximum = maximum.max(confidence);
                if confidence > best.confidence {
                    best = MarkerCorrelation {
                        delay_frames: delay,
                        confidence,
                        peak_margin: 0.0,
                        inverted,
                    };
                    best_rate = ppm;
                }
            }
        }
    }
    if !valid_template {
        return Err(MarkerAnalysisError::InvalidMarker);
    }
    if !has_signal {
        return Err(MarkerAnalysisError::SilentRecording);
    }
    if best.confidence < config.min_correlation {
        return Err(MarkerAnalysisError::NoMatch);
    }
    let runner = onset_scores[..offsets]
        .iter()
        .enumerate()
        .filter(|(delay, _)| *delay != best.delay_frames)
        .map(|(_, score)| *score)
        .fold(0.0_f64, f64::max);
    best.peak_margin = best.confidence - runner;
    if best.peak_margin < config.min_peak_margin {
        return Err(MarkerAnalysisError::Ambiguous);
    }
    let mut rate_runner = 0.0_f64;
    let mut ties = 0;
    for index in 0..prepared.hypothesis_count {
        let ppm = rates.rate(index);
        let length = template_length(reference.len(), ppm)?;
        let template = &mut template_scratch[..length];
        warp(reference, template, ppm);
        if let Some((confidence, _)) = score(
            template,
            &recording[best.delay_frames..best.delay_frames + length],
            config.min_rms,
        ) {
            ties += usize::from((confidence - best.confidence).abs() <= 1e-12);
            if ppm != best_rate {
                rate_runner = rate_runner.max(confidence);
            }
        }
    }
    let length = template_length(reference.len(), best_rate)?;
    let template = &mut template_scratch[..length];
    warp(reference, template, best_rate);
    if !marker_sections_match(
        template,
        &recording[best.delay_frames..best.delay_frames + length],
        best.inverted,
        config,
    ) {
        return Err(MarkerAnalysisError::NoMatch);
    }
    Ok(MarkerRateCorrelation {
        alignment: best,
        template_rate_ppm: best_rate,
        hypothesis_count: prepared.hypothesis_count,
        rate_grid_step_ppm: rates.rate_step_ppm,
        template_rate_peak_margin: best.confidence - rate_runner,
        template_rate_ties: ties,
        grid_edge: best_rate == rates.min_rate_ppm || best_rate == rates.max_rate_ppm,
        onset_resolution_frames: 1,
    })
}
