//! Descriptive relative-delay fits on the control/offline side. Neither a
//! fitted slope nor clean packet metadata establishes hardware clock skew.
use super::WindowResult;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

const MAX_EXACT_INTEGER: u64 = (1_u64 << 53) - 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrendConfig {
    pub min_markers: usize,
    pub min_span_frames: u64,
}

impl TrendConfig {
    pub fn validate(self, max_markers: usize) -> Result<()> {
        ensure!(
            (3..=max_markers).contains(&self.min_markers),
            "trend min_markers must be at least 3 and within marker cap"
        );
        ensure!(
            (1..=MAX_EXACT_INTEGER).contains(&self.min_span_frames),
            "trend min_span_frames must be positive and within exact integer precision policy"
        );
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct TrendAnalysis {
    continuity: &'static str,
    scope: &'static str,
    segments: Vec<TrendSegment>,
}

#[derive(Debug, Serialize)]
struct TrendSegment {
    segment_id: usize,
    first_window_index: usize,
    last_window_index: usize,
    marker_count: usize,
    first_reference_frame: u64,
    last_reference_frame: u64,
    reference_span_frames: Option<u64>,
    fit: Option<DelayFit>,
    refusal: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct DelayFit {
    intercept_at_first_reference_frames: f64,
    slope_frames_per_reference_frame: f64,
    relative_delay_change_ppm: f64,
    residual_rms_frames: f64,
    residual_max_abs_frames: f64,
}

pub fn analyze(
    results: &[WindowResult],
    config: TrendConfig,
    breaks: Option<&[bool]>,
) -> TrendAnalysis {
    let mut segments = Vec::new();
    let mut start = None;
    let mut flush = |start: &mut Option<usize>, end| {
        if let Some(first) = start.take() {
            segments.push(segment(&results[first..end], config, segments.len()));
        }
    };
    for (index, result) in results.iter().enumerate() {
        if breaks.is_some_and(|barriers| barriers[index]) {
            flush(&mut start, index);
        }
        if result.error.is_some()
            || result.delay_frames.is_none()
            || result.reference.is_none()
            || result.processed.is_none()
        {
            flush(&mut start, index);
        } else if start.is_none() {
            start = Some(index);
        }
    }
    flush(&mut start, results.len());
    TrendAnalysis {
        continuity: if breaks.is_some() {
            "diagnostic_packet_metadata_not_native_epoch"
        } else {
            "unverified_no_packet_metadata"
        },
        scope: "relative_delay_change_not_clock_skew; raw_delays_and_distribution_not_detrended; residual_rms_sqrt_sse_over_n_not_physical_uncertainty; segments_are_analysis_only_not_native_epochs; no_full_waveform_continuity_or_qualification",
        segments,
    }
}

fn segment(results: &[WindowResult], config: TrendConfig, segment_id: usize) -> TrendSegment {
    let first = &results[0];
    let last = &results[results.len() - 1];
    let first_reference_frame = first
        .reference
        .as_ref()
        .expect("accepted marker")
        .absolute_frame;
    let last_reference_frame = last
        .reference
        .as_ref()
        .expect("accepted marker")
        .absolute_frame;
    let span = last_reference_frame.checked_sub(first_reference_frame);
    let fitted = fit(results, config);
    let (fit, refusal) = match fitted {
        Ok(fit) => (Some(fit), None),
        Err(reason) => (None, Some(reason)),
    };
    TrendSegment {
        segment_id,
        first_window_index: first.index,
        last_window_index: last.index,
        marker_count: results.len(),
        first_reference_frame,
        last_reference_frame,
        reference_span_frames: span,
        fit,
        refusal,
    }
}

fn fit(results: &[WindowResult], config: TrendConfig) -> Result<DelayFit, &'static str> {
    let origin = results[0]
        .reference
        .as_ref()
        .expect("accepted marker")
        .absolute_frame;
    let delay_origin = results[0].delay_frames.expect("accepted marker") as u64;
    let mut previous = None;
    let mut mean_x = 0.0;
    let mut mean_y = 0.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let mut span = 0;
    // Subtract integer origins before conversion, then center both axes.
    // Large absolute recording offsets must not erase nearby marker deltas.
    for (index, result) in results.iter().enumerate() {
        let reference = result
            .reference
            .as_ref()
            .expect("accepted marker")
            .absolute_frame;
        if previous.is_some_and(|previous| reference <= previous) {
            return Err("nonincreasing_reference_positions");
        }
        previous = Some(reference);
        span = reference
            .checked_sub(origin)
            .ok_or("reference_delta_overflow")?;
        let delay = result.delay_frames.expect("accepted marker") as u64;
        if span > MAX_EXACT_INTEGER || delay > MAX_EXACT_INTEGER {
            return Err("exact_integer_precision_policy_exceeded");
        }
        let x = span as f64;
        let y = signed_delta(delay, delay_origin);
        let dx = x - mean_x;
        let dy = y - mean_y;
        mean_x += dx / (index + 1) as f64;
        mean_y += dy / (index + 1) as f64;
        sxx += dx * (x - mean_x);
        sxy += dx * (y - mean_y);
    }
    if results.len() < config.min_markers {
        return Err("insufficient_marker_count");
    }
    if span < config.min_span_frames {
        return Err("insufficient_reference_span");
    }
    if sxx <= 0.0 {
        return Err("degenerate_reference_span");
    }
    let slope = sxy / sxx;
    let relative_intercept = mean_y - slope * mean_x;
    let mut sse = 0.0;
    let mut maximum = 0.0_f64;
    for result in results {
        let x = (result
            .reference
            .as_ref()
            .expect("accepted marker")
            .absolute_frame
            - origin) as f64;
        let y = signed_delta(
            result.delay_frames.expect("accepted marker") as u64,
            delay_origin,
        );
        let residual = y - (relative_intercept + slope * x);
        sse += residual * residual;
        maximum = maximum.max(residual.abs());
    }
    let fit = DelayFit {
        intercept_at_first_reference_frames: delay_origin as f64 + relative_intercept,
        slope_frames_per_reference_frame: slope,
        relative_delay_change_ppm: slope * 1_000_000.0,
        residual_rms_frames: (sse / results.len() as f64).sqrt(),
        residual_max_abs_frames: maximum,
    };
    if ![
        fit.intercept_at_first_reference_frames,
        slope,
        fit.relative_delay_change_ppm,
        fit.residual_rms_frames,
        maximum,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        return Err("nonfinite_fit");
    }
    Ok(fit)
}

fn signed_delta(value: u64, origin: u64) -> f64 {
    if value >= origin {
        (value - origin) as f64
    } else {
        -((origin - value) as f64)
    }
}
