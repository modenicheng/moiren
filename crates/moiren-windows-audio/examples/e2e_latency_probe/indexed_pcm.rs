//! Offline indexed-v2 identities, never a source/recorder clock alignment.
//!
//! Windows are explicit recorder coordinates, with no source labels. Every
//! window competes against every schedule code. No peak-order renumbering,
//! origin fit, rate-hypothesis search or physical acceptance occurs here.

use super::live::source::{INDEXED_WAVEFORM_VERSION, SourceSchedule};
use super::pcm::PacketTimeline;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};

const MAX_PCM_FRAMES: u64 = 48_000 * 600;
const MAX_WINDOW_FRAMES: usize = 96_000;
const MAX_CORRELATION_SAMPLES: usize = 256_000_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndexedConfig {
    pub schema_version: u32,
    pub recorder_origin: String,
    pub sample_rate: u32,
    pub channel: usize,
    pub max_pcm_frames: u64,
    pub max_correlation_samples: usize,
    pub min_correlation: f64,
    pub min_peak_margin: f64,
    pub min_rms: f64,
    pub windows: Vec<RecorderWindow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecorderWindow {
    pub start_frame: u64,
    pub frames: usize,
}

#[derive(Debug, Serialize)]
struct Candidate {
    identity_index: usize,
    recording_frame: u64,
    confidence: f64,
    onset_peak_margin: f64,
    inverted: bool,
    sections_match: bool,
    valid_onsets: usize,
}

#[derive(Debug, Serialize)]
struct WindowResult {
    window_index: usize,
    status: &'static str,
    identity_index: Option<usize>,
    recording_frame: Option<u64>,
    identity_peak_margin: Option<f64>,
    // Even a strongest candidate is not an identity when ambiguity remains.
    best_candidate: Option<Candidate>,
    issues: Vec<String>,
}

impl IndexedConfig {
    fn validate(&self, bytes: u64, source: &SourceSchedule) -> Result<usize> {
        ensure!(
            self.schema_version == 1,
            "unsupported indexed analysis schema"
        );
        ensure!(
            self.recorder_origin == "stored_pcm_frame_zero",
            "unsupported recorder origin; only stored_pcm_frame_zero, never source clock origin"
        );
        ensure!(
            source.waveform_version == INDEXED_WAVEFORM_VERSION,
            "indexed analysis requires exact indexed-v2 source waveform version"
        );
        ensure!(
            self.sample_rate == 48_000 && self.channel < 2,
            "indexed analysis requires exact 48 kHz stereo and channel 0 or 1"
        );
        ensure!(
            bytes > 0 && bytes.is_multiple_of(8),
            "PCM must contain nonempty complete stereo f32le frames"
        );
        ensure!(
            (1..=MAX_PCM_FRAMES).contains(&self.max_pcm_frames) && bytes / 8 <= self.max_pcm_frames,
            "indexed PCM frame cap exceeded"
        );
        ensure!(
            (1..=4096).contains(&self.windows.len()),
            "indexed analysis requires 1..4096 recorder windows"
        );
        ensure!(
            (1..=MAX_CORRELATION_SAMPLES).contains(&self.max_correlation_samples),
            "invalid indexed correlation work cap"
        );
        ensure!(
            self.min_correlation.is_finite()
                && (0.7..=1.0).contains(&self.min_correlation)
                && [self.min_peak_margin, self.min_rms]
                    .iter()
                    .all(|n| n.is_finite() && *n > 0.0 && *n <= 1.0),
            "invalid indexed thresholds (correlation floor 0.7)"
        );
        let mut work = 0usize;
        let mut previous_end = 0;
        for window in &self.windows {
            ensure!(
                (source.marker_frames..=MAX_WINDOW_FRAMES).contains(&window.frames),
                "recorder window must contain a full marker within window cap"
            );
            let end = window
                .start_frame
                .checked_add(window.frames as u64)
                .context("indexed recorder window overflow")?;
            ensure!(
                window.start_frame >= previous_end && end <= bytes / 8,
                "recorder windows must be ordered, nonoverlapping and fully retained"
            );
            previous_end = end;
            let pairs = (window.frames - source.marker_frames + 1)
                .checked_mul(source.marker_frames)
                .and_then(|n| n.checked_mul(source.starts.len()))
                .context("indexed correlation work overflow")?;
            work = work
                .checked_add(pairs)
                .context("indexed total work overflow")?;
        }
        ensure!(
            work <= self.max_correlation_samples,
            "indexed correlation work cap exceeded"
        );
        Ok(work)
    }
}

fn validate_pcm(reader: &mut (impl Read + Seek), bytes: u64) -> Result<()> {
    reader.seek(SeekFrom::Start(0))?;
    let mut buffer = [0u8; 16_384];
    let mut remaining = bytes;
    while remaining != 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        reader.read_exact(&mut buffer[..count])?;
        for sample in buffer[..count].as_chunks::<4>().0 {
            let value = f32::from_le_bytes(*sample);
            ensure!(value.is_finite(), "nonfinite PCM sample");
            ensure!(value.abs() < 1.0, "clipped PCM sample");
        }
        remaining -= count as u64;
    }
    let mut tail = [0u8; 1];
    ensure!(
        reader.read(&mut tail)? == 0,
        "PCM grew beyond admitted byte length"
    );
    Ok(())
}

fn moments(
    reference: &[f32],
    recording: &[f32],
    reference_mean: f64,
    recording_mean: f64,
) -> (f64, f64, f64) {
    let (mut dot, mut left, mut right) = (0.0, 0.0, 0.0);
    for (&a, &b) in reference.iter().zip(recording) {
        let a = f64::from(a) - reference_mean;
        let b = f64::from(b) - recording_mean;
        dot += a * b;
        left += a * a;
        right += b * b;
    }
    (dot, left, right)
}

fn sections_match(
    reference: &[f32],
    samples: &[f32],
    reference_mean: f64,
    recording_mean: f64,
    inverted: bool,
    config: &IndexedConfig,
) -> bool {
    let section_frames = reference.len() / 4;
    (0..4).all(|section| {
        let start = section * section_frames;
        let end = if section == 3 {
            reference.len()
        } else {
            start + section_frames
        };
        let (dot, left, right) = moments(
            &reference[start..end],
            &samples[start..end],
            reference_mean,
            recording_mean,
        );
        let polarity = if inverted { -1.0 } else { 1.0 };
        (left / (end - start) as f64).sqrt() >= config.min_rms
            && (right / (end - start) as f64).sqrt() >= config.min_rms
            && polarity * dot / (left * right).sqrt() >= config.min_correlation
    })
}

fn search_code(reference: &[f32], recording: &[f32], config: &IndexedConfig) -> Option<Candidate> {
    let mean =
        |samples: &[f32]| samples.iter().map(|v| f64::from(*v)).sum::<f64>() / samples.len() as f64;
    let reference_mean = mean(reference);
    let mut best: Option<Candidate> = None;
    let mut runner_up: f64 = 0.0;
    let mut valid_onsets = 0;
    for (offset, samples) in recording.windows(reference.len()).enumerate() {
        let recording_mean = mean(samples);
        let (dot, left, right) = moments(reference, samples, reference_mean, recording_mean);
        if (left / reference.len() as f64).sqrt() < config.min_rms
            || (right / reference.len() as f64).sqrt() < config.min_rms
        {
            continue;
        }
        let score = (dot / (left * right).sqrt()).abs().min(1.0);
        let matches_sections = score >= config.min_correlation
            && sections_match(
                reference,
                samples,
                reference_mean,
                recording_mean,
                dot < 0.0,
                config,
            );
        valid_onsets += usize::from(matches_sections);
        if best.as_ref().is_none_or(|old| score > old.confidence) {
            runner_up = best
                .as_ref()
                .map_or(runner_up, |old| old.confidence.max(runner_up));
            best = Some(Candidate {
                identity_index: 0,
                recording_frame: offset as u64,
                confidence: score,
                onset_peak_margin: 0.0,
                inverted: dot < 0.0,
                sections_match: matches_sections,
                valid_onsets: 0,
            });
        } else {
            runner_up = runner_up.max(score);
        }
    }
    let mut best = best?;
    best.onset_peak_margin = best.confidence - runner_up;
    best.valid_onsets = valid_onsets;
    Some(best)
}

pub fn analyze(
    pcm: &mut (impl Read + Seek),
    bytes: u64,
    source: SourceSchedule,
    config: IndexedConfig,
    packets: impl Read,
    max_metadata_bytes: u64,
    max_packets: usize,
) -> Result<Value> {
    // Source admission/codebook and the total Cartesian search budget are
    // validated before reading PCM or invoking even one correlation search.
    let prepared = source.clone().prepare(600)?;
    let work = config.validate(bytes, &source)?;
    let mut issues = Vec::new();
    let packet_evidence = match PacketTimeline::read(
        packets,
        max_metadata_bytes,
        max_packets,
        bytes / 8,
        source.sample_rate,
    ) {
        Ok(timeline) => {
            if timeline.diagnostic_invalid() {
                issues.push(
                    "packet_evidence_invalid_loss_reset_silence_timestamp_or_coverage".to_string(),
                );
            }
            serde_json::to_value(timeline.diagnostic_evidence())?
        }
        Err(error) => {
            issues.push("invalid_packet_metadata".to_string());
            json!({"error": format!("{error:#}"), "native_epoch": "not_recorded"})
        }
    };
    let pcm_valid = match validate_pcm(pcm, bytes) {
        Ok(()) => true,
        Err(error) => {
            issues.push(format!("invalid_pcm: {error:#}"));
            false
        }
    };
    let mut results = Vec::new();
    if pcm_valid {
        for (window_index, window) in config.windows.iter().enumerate() {
            let mut raw = vec![0; window.frames * 8];
            pcm.seek(SeekFrom::Start(window.start_frame * 8))?;
            pcm.read_exact(&mut raw)?;
            let recording: Vec<_> = raw
                .as_chunks::<8>()
                .0
                .iter()
                .map(|frame| {
                    let offset = config.channel * 4;
                    f32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap())
                })
                .collect();
            let mut candidates = Vec::new();
            for identity_index in 0..source.starts.len() {
                let reference = prepared
                    .indexed_marker(identity_index)
                    .context("missing indexed code")?;
                if let Some(mut hit) = search_code(reference, &recording, &config) {
                    hit.identity_index = identity_index;
                    hit.recording_frame += window.start_frame;
                    candidates.push(hit);
                }
            }
            candidates.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
            let valid_onsets: usize = candidates.iter().map(|hit| hit.valid_onsets).sum();
            let margin = candidates.first().map(|best| {
                best.confidence - candidates.get(1).map_or(0.0, |next| next.confidence)
            });
            let best = candidates.into_iter().next();
            // Subthreshold/section-rejected competitors still count against
            // identity margin. Threshold filtering must never inflate it.
            let matching = best
                .as_ref()
                .is_some_and(|hit| hit.confidence >= config.min_correlation && hit.sections_match);
            let ambiguous = valid_onsets > 1
                || matching
                    && (margin.is_some_and(|m| m < config.min_peak_margin)
                        || best
                            .as_ref()
                            .is_some_and(|hit| hit.onset_peak_margin < config.min_peak_margin));
            let resolved = matching && !ambiguous;
            results.push(WindowResult {
                window_index,
                status: if ambiguous {
                    "ambiguous"
                } else if resolved {
                    "unique"
                } else {
                    "missing"
                },
                identity_index: resolved.then(|| best.as_ref().unwrap().identity_index),
                recording_frame: resolved.then(|| best.as_ref().unwrap().recording_frame),
                identity_peak_margin: margin,
                best_candidate: best,
                issues: if ambiguous {
                    vec!["competing_codewords_or_onsets".into()]
                } else {
                    vec![]
                },
            });
        }
    }
    let mut counts = vec![0; source.starts.len()];
    for result in &results {
        if let Some(index) = result.identity_index {
            counts[index] += 1;
        }
    }
    let missing: Vec<_> = counts
        .iter()
        .enumerate()
        .filter_map(|(i, &n)| (n == 0).then_some(i))
        .collect();
    let duplicate: Vec<_> = counts
        .iter()
        .enumerate()
        .filter_map(|(i, &n)| (n > 1).then_some(i))
        .collect();
    for result in &mut results {
        if result.identity_index.is_some_and(|i| counts[i] > 1) {
            result.status = "duplicate";
            result
                .issues
                .push("codeword_occurs_in_multiple_windows".into());
        }
    }
    let mut spacing = Vec::new();
    for index in 1..source.starts.len() {
        if counts[index - 1] != 1 || counts[index] != 1 {
            continue;
        }
        let frame = |identity| {
            results
                .iter()
                .find(|r| r.identity_index == Some(identity))
                .unwrap()
                .recording_frame
                .unwrap()
        };
        let observed = frame(index) as i64 - frame(index - 1) as i64;
        let expected = source.starts[index] - source.starts[index - 1];
        if observed <= 0 {
            issues.push("recorded_identity_order_differs_from_source_schedule".into());
        }
        spacing.push(json!({
            "from_identity_index": index - 1, "to_identity_index": index,
            "source_delta_frames": expected, "recording_delta_frames": observed,
            "spacing_residual_frames": observed - expected as i64,
            "relative_spacing_ppm": (observed as f64 / expected as f64 - 1.0) * 1_000_000.0
        }));
    }
    let complete = issues.is_empty()
        && missing.is_empty()
        && duplicate.is_empty()
        && results.iter().all(|r| r.status == "unique");
    Ok(json!({
        "schema_version": 1, "mode": "offline_indexed_v2_identity_diagnostic", "qualified": false,
        "identity_set_complete": complete, "source_to_recorder_origin": "unknown_not_estimated",
        "coordinate_basis": "stored_pcm_frame_zero", "native_epoch": "not_recorded",
        "template_rate_model": "exact_nominal_48000_no_resampling",
        "scope": "Explicit recorder windows only, not whole-recording absence or waveform continuity; relative spacing is not physical delay or calibrated clock drift.",
        "source_schedule": source, "config": config, "capture_frames": bytes / 8,
        "admitted_correlation_samples": work, "results": results, "spacing": spacing,
        "missing_identities": missing, "duplicate_identities": duplicate,
        "packet_evidence": packet_evidence, "issues": issues
    }))
}

// The first SRC-model slice is a synthetic test artifact, not a CLI mode.
#[cfg(test)]
#[path = "indexed_pcm/src_model.rs"]
mod src_model;

#[cfg(test)]
#[path = "indexed_pcm/tests.rs"]
mod tests;
