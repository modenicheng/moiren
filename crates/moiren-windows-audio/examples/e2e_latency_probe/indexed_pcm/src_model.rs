//! Synthetic-only offline indexed-code model; not wired into a probe CLI.
//!
//! The affine model is not an exact reconstruction of unknown cascaded SRCs.
//! Source/recorder clock origin and packet/runtime quality are not evaluated.

use crate::resample::StereoResampler;
use serde::{Deserialize, Serialize};

pub const CORRELATION_FLOOR: f64 = 0.7;
pub const PEAK_MARGIN: f64 = 0.05;
pub const RMS_FLOOR: f64 = 0.0001;
const WORK_CAP: usize = 256_000_000;
const GUARD: usize = 48;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub schema_version: u32,
    pub phase_quarters: Vec<u8>,
    /// Positive stretches the template, unlike positive SRC consumption ppm.
    pub scale_ppm: Vec<i32>,
    pub filter_passes: Vec<u8>,
    pub max_correlation_samples: usize,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            phase_quarters: vec![0, 1, 2, 3],
            scale_ppm: vec![-4000, -2000, 0, 2000, 4000],
            filter_passes: vec![1, 2],
            max_correlation_samples: WORK_CAP,
        }
    }
}

pub struct RecorderWindow<'a> {
    pub start_frame: u64,
    pub samples: &'a [f32],
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("invalid frozen synthetic model configuration or input shape")]
    InvalidConfig,
    #[error("bounded model work exceeded")]
    WorkLimit,
    #[error("non-finite or clipped raw observation/codebook")]
    InvalidSamples,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub enum Status {
    Unique,
    Ambiguous,
    Unidentified,
}

#[derive(Debug, Serialize)]
pub struct WindowResult {
    pub status: Status,
    pub source_index: Option<usize>,
    /// Model body boundary in stored recorder frames, not physical onset or
    /// calibrated timing uncertainty. Includes accepted phase/rate hypotheses.
    pub recording_frame_interval: Option<[u64; 2]>,
    pub best_score: f64,
}

#[derive(Debug, Serialize)]
pub struct ModelReport {
    pub model: &'static str,
    pub source_origin: &'static str,
    pub packet_quality: &'static str,
    pub correlation_samples: usize,
    pub results: Vec<WindowResult>,
}

fn template_len(frames: usize, ppm: i32) -> usize {
    ((frames + 2 * GUARD) as f64 * (1.0 + f64::from(ppm) * 1e-6)).ceil() as usize + 2
}

fn admission(
    codes: &[Vec<f32>],
    windows: &[RecorderWindow<'_>],
    c: &ModelConfig,
) -> Result<usize, ModelError> {
    let ordered = |values: &[i32]| !values.is_empty() && values.windows(2).all(|p| p[0] < p[1]);
    let phases: Vec<_> = c.phase_quarters.iter().map(|v| i32::from(*v)).collect();
    let passes: Vec<_> = c.filter_passes.iter().map(|v| i32::from(*v)).collect();
    let n = codes.first().map_or(0, Vec::len);
    if c.schema_version != 1
        || !(128..=4096).contains(&n)
        || !(1..=64).contains(&codes.len())
        || codes.iter().any(|code| code.len() != n)
        || !(1..=64).contains(&windows.len())
        || !ordered(&phases)
        || phases.iter().any(|v| !(0..=3).contains(v))
        || !ordered(&c.scale_ppm)
        || c.scale_ppm
            .iter()
            .any(|v| ![-4000, -2000, 0, 2000, 4000].contains(v))
        || !ordered(&passes)
        || passes.iter().any(|v| !(1..=2).contains(v))
        || c.max_correlation_samples == 0
        || c.max_correlation_samples > WORK_CAP
    {
        return Err(ModelError::InvalidConfig);
    }
    let mut work = 0usize;
    let mut previous_end = 0;
    for w in windows {
        let end = w
            .start_frame
            .checked_add(w.samples.len() as u64)
            .ok_or(ModelError::InvalidConfig)?;
        if w.start_frame < previous_end || w.samples.len() > 8192 {
            return Err(ModelError::InvalidConfig);
        }
        previous_end = end;
        for ppm in &c.scale_ppm {
            let len = template_len(n, *ppm);
            if w.samples.len() < len {
                return Err(ModelError::InvalidConfig);
            }
            // The four sections are disjoint slices inside the full warped
            // template; charge a second full length, including stretch/rounding.
            let pairs = (w.samples.len() - len + 1)
                .checked_mul(2 * len)
                .and_then(|x| x.checked_mul(codes.len()))
                .and_then(|x| x.checked_mul(phases.len()))
                .and_then(|x| x.checked_mul(passes.len()))
                .ok_or(ModelError::WorkLimit)?;
            work = work.checked_add(pairs).ok_or(ModelError::WorkLimit)?;
        }
    }
    if work > c.max_correlation_samples {
        return Err(ModelError::WorkLimit);
    }
    if codes
        .iter()
        .flatten()
        .chain(windows.iter().flat_map(|w| w.samples.iter()))
        .any(|x| !x.is_finite() || x.abs() >= 1.0)
    {
        return Err(ModelError::InvalidSamples);
    }
    Ok(work)
}

// Production FIR at nominal rate supplies the fixed filter response. Applying
// affine interpolation afterwards is deliberately an approximation, not a
// simulation of unknown first/second-stage phases or servo histories.
fn filtered(code: &[f32], passes: u8) -> Vec<f32> {
    let mut signal = vec![0.0; GUARD];
    signal.extend_from_slice(code);
    signal.resize(code.len() + 2 * GUARD, 0.0);
    for _ in 0..passes {
        let mut src = StereoResampler::new(48_000, 48_000, 8192).unwrap();
        for sample in &signal {
            assert!(src.push_frame([f64::from(*sample); 2]));
        }
        for _ in 0..32 {
            assert!(src.push_frame([0.0; 2]));
        }
        for sample in &mut signal {
            *sample = src.next_frame().unwrap()[0] as f32;
        }
    }
    signal
}

fn warped(base: &[f32], ppm: i32, phase: u8, frames: usize) -> Vec<f32> {
    let ratio = 1.0 + f64::from(ppm) * 1e-6;
    (0..frames)
        .map(|i| {
            let p = (i as f64 + f64::from(phase) / 4.0) / ratio;
            let j = p.floor() as usize;
            let f = (p - j as f64) as f32;
            base.get(j).copied().unwrap_or(0.0) * (1.0 - f)
                + base.get(j + 1).copied().unwrap_or(0.0) * f
        })
        .collect()
}

fn score(a: &[f32], b: &[f32]) -> f64 {
    let ma = a.iter().map(|v| f64::from(*v)).sum::<f64>() / a.len() as f64;
    let mb = b.iter().map(|v| f64::from(*v)).sum::<f64>() / b.len() as f64;
    let (mut aa, mut bb, mut ab) = (0.0, 0.0, 0.0);
    for (a, b) in a.iter().zip(b) {
        let (a, b) = (f64::from(*a) - ma, f64::from(*b) - mb);
        aa += a * a;
        bb += b * b;
        ab += a * b;
    }
    if (aa / a.len() as f64).sqrt() < RMS_FLOOR || (bb / b.len() as f64).sqrt() < RMS_FLOOR {
        0.0
    } else {
        (ab / (aa * bb).sqrt()).clamp(-1.0, 1.0)
    }
}

struct Hit {
    index: usize,
    score: f64,
    body: f64,
    sections: bool,
}

/// Exact source index is the supplied codebook index, never the window order.
pub fn analyze(
    codes: &[Vec<f32>],
    windows: &[RecorderWindow<'_>],
    config: &ModelConfig,
) -> Result<ModelReport, ModelError> {
    let work = admission(codes, windows, config)?;
    let mut results = Vec::new();
    for window in windows {
        let mut hits = Vec::new();
        for (index, code) in codes.iter().enumerate() {
            for passes in &config.filter_passes {
                let base = filtered(code, *passes);
                for ppm in &config.scale_ppm {
                    let ratio = 1.0 + f64::from(*ppm) * 1e-6;
                    for phase in &config.phase_quarters {
                        let template = warped(&base, *ppm, *phase, template_len(code.len(), *ppm));
                        let body = GUARD as f64 * ratio - f64::from(*phase) / 4.0;
                        for (offset, samples) in window.samples.windows(template.len()).enumerate()
                        {
                            let full = score(&template, samples);
                            let polarity = full.signum();
                            let sections = (0..4).all(|q| {
                                let lo = ((GUARD + q * code.len() / 4) as f64 * ratio
                                    - f64::from(*phase) / 4.0)
                                    .ceil() as usize;
                                let hi = ((GUARD + (q + 1) * code.len() / 4) as f64 * ratio
                                    - f64::from(*phase) / 4.0)
                                    .ceil() as usize;
                                polarity * score(&template[lo..hi], &samples[lo..hi])
                                    >= CORRELATION_FLOOR
                            });
                            hits.push(Hit {
                                index,
                                score: full.abs(),
                                body: offset as f64 + body,
                                sections,
                            });
                        }
                    }
                }
            }
        }
        let best = hits
            .iter()
            .max_by(|a, b| a.score.total_cmp(&b.score))
            .unwrap();
        let valid: Vec<_> = hits
            .iter()
            .filter(|h| h.sections && h.score >= CORRELATION_FLOOR)
            .collect();
        let other = hits
            .iter()
            .filter(|h| h.index != best.index || (h.body - best.body).abs() > 2.0)
            .map(|h| h.score)
            .fold(0.0, f64::max);
        let lo = valid.iter().map(|h| h.body).fold(f64::INFINITY, f64::min);
        let hi = valid
            .iter()
            .map(|h| h.body)
            .fold(f64::NEG_INFINITY, f64::max);
        let matching = best.sections && best.score >= CORRELATION_FLOOR;
        let ambiguous = matching
            && (best.score - other < PEAK_MARGIN
                || valid.iter().any(|h| h.index != best.index)
                || hi.ceil() - lo.floor() > 2.0);
        let unique = matching && !ambiguous;
        results.push(WindowResult {
            status: if ambiguous {
                Status::Ambiguous
            } else if unique {
                Status::Unique
            } else {
                Status::Unidentified
            },
            source_index: unique.then_some(best.index),
            recording_frame_interval: unique.then(|| {
                [
                    window.start_frame + lo.floor() as u64,
                    window.start_frame + hi.ceil() as u64,
                ]
            }),
            best_score: best.score,
        });
    }
    let mut counts = vec![0; codes.len()];
    for result in &results {
        if let Some(index) = result.source_index {
            counts[index] += 1;
        }
    }
    for result in &mut results {
        if result.source_index.is_some_and(|index| counts[index] > 1) {
            result.status = Status::Ambiguous;
            result.source_index = None;
            result.recording_frame_interval = None;
        }
    }
    Ok(ModelReport {
        model: "affine_fir_synthetic_v1_not_exact_cascade",
        source_origin: "unknown_not_estimated",
        packet_quality: "not_evaluated_no_complete_set_claim",
        correlation_samples: work,
        results,
    })
}

#[path = "src_model/tests.rs"]
mod tests;
