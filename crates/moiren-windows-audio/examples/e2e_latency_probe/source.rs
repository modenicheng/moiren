//! Source-clock plans and acknowledged native submissions, never recorder windows.
//!
//! v1 repeats one marker. Opt-in indexed-v2 assigns each starts[index] the
//! corresponding segment of one continuous fixed-seed PRNG stream. Identity
//! requires the exact schedule and version, not the order of recorded peaks;
//! this generator neither aligns recorder clocks nor qualifies a waveform.

use anyhow::{Context, Result, ensure};
use moiren_windows_audio::latency::MarkerGenerator;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

pub const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
pub const WAVEFORM_VERSION: &str = "moiren-marker-xorshift32-v1";
pub const INDEXED_WAVEFORM_VERSION: &str = "moiren-marker-xorshift32-indexed-v2";
const MAX_CODEBOOK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceSchedule {
    pub schema_version: u32,
    pub schedule_id: String,
    pub waveform_version: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub marker_frames: usize,
    pub marker_amplitude: f32,
    pub total_source_frames: u64,
    pub starts: Vec<u64>,
}

#[derive(Clone, Debug)]
pub struct PreparedSource {
    schedule: SourceSchedule,
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    marker: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceProvenance {
    pub mode: &'static str,
    pub schedule: Option<SourceSchedule>,
    pub source_clock: &'static str,
    pub recorder_clock: &'static str,
    pub channel_policy: &'static str,
    pub completion_meaning: &'static str,
}

impl SourceProvenance {
    pub fn new(plan: Option<&PreparedSource>) -> Self {
        Self {
            mode: if plan.is_some() {
                "explicit_source_schedule"
            } else {
                "diagnostic_pulses_no_source_marker_provenance"
            },
            schedule: plan.map(|plan| plan.schedule.clone()),
            source_clock: if plan.is_some() {
                "Source frame zero: first successfully released post-Start payload; silent native preroll excluded"
            } else {
                "Diagnostic native submission frames only; no source marker coordinate provenance"
            },
            recorder_clock: "Independent recorder frames; no automatic source-to-recorder mapping",
            channel_policy: "Same source sample on both stereo channels; not independent measurement channels",
            completion_meaning: "Successfully submitted via ReleaseBuffer only; not played or captured",
        }
    }
}

impl SourceSchedule {
    pub fn prepare(self, seconds: u64) -> Result<PreparedSource> {
        ensure!(
            self.schema_version == 1,
            "unsupported source schedule schema"
        );
        ensure!(
            !self.schedule_id.is_empty()
                && self.schedule_id.len() <= 128
                && self
                    .schedule_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)),
            "source schedule identity must be 1..128 ASCII letters/digits or -_."
        );
        ensure!(
            matches!(
                self.waveform_version.as_str(),
                WAVEFORM_VERSION | INDEXED_WAVEFORM_VERSION
            ),
            "unsupported source waveform version"
        );
        ensure!(
            self.sample_rate == 48_000 && self.channels == 2,
            "source requires exact 48 kHz stereo"
        );
        ensure!(
            (32..=48_000).contains(&self.marker_frames),
            "source marker frames must be 32..48000"
        );
        ensure!(
            self.marker_amplitude.is_finite()
                && self.marker_amplitude > 0.0
                && self.marker_amplitude < 1.0,
            "source marker amplitude must be finite and strictly between zero and one"
        );
        ensure!(
            (1..=48_000 * 600).contains(&self.total_source_frames),
            "source total frame cap exceeded"
        );
        let available = seconds
            .checked_mul(48_000)
            .context("source duration overflow")?;
        ensure!(
            self.total_source_frames <= available,
            "source duration exceeds requested live --seconds"
        );
        ensure!(
            (1..=4096).contains(&self.starts.len()),
            "source requires 1..4096 markers"
        );
        let mut previous_end = 0;
        for &start in &self.starts {
            ensure!(
                start >= previous_end,
                "source starts must be strictly ordered and nonoverlapping"
            );
            let end = start
                .checked_add(self.marker_frames as u64)
                .context("source marker end overflow")?;
            ensure!(
                end <= self.total_source_frames,
                "source marker exceeds total source frames"
            );
            previous_end = end;
        }
        // v1 deliberately restarts one marker at every start. The explicit v2
        // codebook is one continuous PRNG stream: starts[index] owns segment
        // index, independently of silence, native chunking, or later detection.
        // Admission and generation happen before any native owner is opened.
        let marker_count = if self.waveform_version == INDEXED_WAVEFORM_VERSION {
            self.starts.len()
        } else {
            1
        };
        let marker_samples = self
            .marker_frames
            .checked_mul(marker_count)
            .context("source codebook sample count overflow")?;
        let marker_bytes = marker_samples
            .checked_mul(size_of::<f32>())
            .context("source codebook byte count overflow")?;
        ensure!(
            marker_bytes <= MAX_CODEBOOK_BYTES,
            "source codebook byte cap exceeded"
        );
        let mut marker = Vec::new();
        marker
            .try_reserve_exact(marker_samples)
            .context("reserve source codebook")?;
        marker.resize(marker_samples, 0.0);
        MarkerGenerator::new(marker_samples, self.marker_amplitude)?.fill(&mut marker);
        Ok(PreparedSource {
            schedule: self,
            marker,
        })
    }
}

impl PreparedSource {
    /// Read the exact precomputed indexed code; never advance the source cursor.
    pub fn indexed_marker(&self, index: usize) -> Option<&[f32]> {
        if self.schedule.waveform_version != INDEXED_WAVEFORM_VERSION
            || index >= self.schedule.starts.len()
        {
            return None;
        }
        let start = index * self.schedule.marker_frames;
        Some(&self.marker[start..start + self.schedule.marker_frames])
    }
}

pub fn read_plan(path: &Path, limit: u64, seconds: u64) -> Result<(Vec<u8>, PreparedSource)> {
    let limit = limit.min(MAX_SOURCE_BYTES);
    ensure!(limit > 0, "source byte cap must be positive");
    let file =
        File::open(path).with_context(|| format!("open source schedule {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "source schedule byte cap exceeded or not a regular file"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "source schedule grew beyond byte cap"
    );
    let schedule: SourceSchedule = serde_json::from_slice(&bytes)?;
    Ok((bytes, schedule.prepare(seconds)?))
}

#[cfg(any(windows, test))]
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct SourceProgress {
    pub native_preroll_frames: u64,
    pub native_submitted_frames: u64,
    pub origin_submission_frame: Option<u64>,
    pub acknowledged_source_frames: u64,
    pub schedule_submitted_complete: bool,
    /// Generated frames accepted by ReleaseBuffer, not native delivery or playback.
    pub submitted_nonzero_frames: u64,
    pub first_nonzero_submission_frame: Option<u64>,
}

#[cfg(any(windows, test))]
pub struct SourceCursor {
    plan: Option<PreparedSource>,
    pub progress: SourceProgress,
}

#[cfg(any(windows, test))]
impl SourceCursor {
    pub fn new(plan: Option<PreparedSource>) -> Self {
        Self {
            plan,
            progress: SourceProgress::default(),
        }
    }

    pub fn provenance(&self) -> SourceProvenance {
        SourceProvenance::new(self.plan.as_ref())
    }

    pub fn has_schedule(&self) -> bool {
        self.plan.is_some()
    }

    pub fn submit(
        &mut self,
        samples: &mut [f32],
        preroll: bool,
        release: impl FnOnce(&[f32]) -> Result<()>,
    ) -> Result<()> {
        ensure!(
            !samples.is_empty() && samples.len().is_multiple_of(2),
            "source payload requires complete nonempty stereo frames"
        );
        ensure!(
            !preroll || self.progress.origin_submission_frame.is_none(),
            "native preroll cannot follow source payload"
        );
        let frames =
            u64::try_from(samples.len() / 2).context("native source frame count overflow")?;
        let mut next = self.progress.clone();
        next.native_submitted_frames = next
            .native_submitted_frames
            .checked_add(frames)
            .context("native submitted frame overflow")?;
        if preroll {
            next.native_preroll_frames = next
                .native_preroll_frames
                .checked_add(frames)
                .context("native preroll frame overflow")?;
        } else if let Some(plan) = &self.plan {
            let remaining = plan
                .schedule
                .total_source_frames
                .checked_sub(next.acknowledged_source_frames)
                .context("source cursor exceeds schedule")?;
            next.acknowledged_source_frames = next
                .acknowledged_source_frames
                .checked_add(frames.min(remaining))
                .context("acknowledged source frame overflow")?;
            next.origin_submission_frame
                .get_or_insert(self.progress.native_submitted_frames);
            next.schedule_submitted_complete =
                next.acknowledged_source_frames == plan.schedule.total_source_frames;
        }
        for (index, pair) in samples.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let frame = if self.plan.is_some() {
                self.progress
                    .acknowledged_source_frames
                    .checked_add(index as u64)
            } else {
                self.progress
                    .native_submitted_frames
                    .checked_add(index as u64)
            }
            .context("source sample position overflow")?;
            let value = match &self.plan {
                Some(_) if preroll => 0.0,
                Some(plan) if frame < plan.schedule.total_source_frames => {
                    let index = plan
                        .schedule
                        .starts
                        .partition_point(|start| *start <= frame);
                    if index == 0 {
                        0.0
                    } else {
                        let offset = frame - plan.schedule.starts[index - 1];
                        if offset < plan.schedule.marker_frames as u64 {
                            let code = if plan.schedule.waveform_version == INDEXED_WAVEFORM_VERSION
                            {
                                index - 1
                            } else {
                                0
                            };
                            // Slice the selected code first: silence after it
                            // must never read into the next codebook segment.
                            let start = code * plan.schedule.marker_frames;
                            plan.marker[start..start + plan.schedule.marker_frames][offset as usize]
                        } else {
                            0.0
                        }
                    }
                }
                Some(_) => 0.0,
                None => {
                    if frame % 48_000 < 256 {
                        0.1
                    } else {
                        0.0
                    }
                }
            };
            if value != 0.0 {
                next.submitted_nonzero_frames = next
                    .submitted_nonzero_frames
                    .checked_add(1)
                    .context("nonzero source frame overflow")?;
                if next.first_nonzero_submission_frame.is_none() {
                    next.first_nonzero_submission_frame = Some(
                        self.progress
                            .native_submitted_frames
                            .checked_add(index as u64)
                            .context("first nonzero source position overflow")?,
                    );
                }
            }
            pair.fill(value);
        }
        // ReleaseBuffer is the acknowledgement boundary. A failed write keeps
        // origin and both clocks unchanged so no submitted/played claim leaks.
        release(samples)?;
        self.progress = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn schedule() -> SourceSchedule {
        SourceSchedule {
            schema_version: 1,
            schedule_id: "source-only-test".into(),
            waveform_version: WAVEFORM_VERSION.into(),
            sample_rate: 48_000,
            channels: 2,
            marker_frames: 32,
            marker_amplitude: 0.1,
            total_source_frames: 117,
            starts: vec![0, 65],
        }
    }

    fn indexed_schedule() -> SourceSchedule {
        SourceSchedule {
            waveform_version: "moiren-marker-xorshift32-indexed-v2".into(),
            total_source_frames: 181,
            starts: vec![0, 65, 130],
            ..schedule()
        }
    }

    fn rendered(plan: SourceSchedule, chunks: &[usize]) -> Vec<f32> {
        let frames = plan.total_source_frames as usize + 19;
        let mut cursor = SourceCursor::new(Some(plan.prepare(600).unwrap()));
        let mut output = Vec::new();
        for &chunk in chunks.iter().cycle() {
            let count = chunk.min(frames - output.len() / 2);
            let mut packet = vec![99.0; count * 2];
            cursor.submit(&mut packet, false, |_| Ok(())).unwrap();
            output.extend(packet);
            if output.len() / 2 == frames {
                break;
            }
        }
        assert!(cursor.progress.schedule_submitted_complete);
        output
    }

    #[test]
    fn source_v1_golden_bytes_repeat_the_original_fixed_seed_marker() {
        // Fixed expected bits, independent of the generator under test.
        let negative_mask = 0x3f94_64abu32;
        let output = rendered(schedule(), &[7, 31, 1, 480]);
        for (frame, pair) in output.as_chunks::<2>().0.iter().enumerate() {
            let offset = [0, 65]
                .into_iter()
                .find_map(|start| frame.checked_sub(start).filter(|offset| *offset < 32));
            let expected = offset.map_or(0.0f32, |offset| {
                if negative_mask & (1 << offset) == 0 {
                    0.1
                } else {
                    -0.1
                }
            });
            assert_eq!(pair.map(f32::to_bits), [expected.to_bits(); 2]);
        }
    }

    #[test]
    fn source_v2_codes_keep_plan_indices_across_silence_and_chunk_boundaries() {
        let plan = indexed_schedule();
        let output = rendered(plan.clone(), &[4096]);
        assert_eq!(output, rendered(plan.clone(), &[1, 31, 33, 480, 7]));
        let masks = [0x3f94_64abu32, 0x4ca4_5baa, 0x9187_8970];
        for (frame, pair) in output.as_chunks::<2>().0.iter().enumerate() {
            let expected = plan
                .starts
                .iter()
                .enumerate()
                .find_map(|(index, &start)| {
                    frame
                        .checked_sub(start as usize)
                        .filter(|offset| *offset < 32)
                        .map(|offset| {
                            if masks[index] & (1 << offset) == 0 {
                                0.1f32
                            } else {
                                -0.1
                            }
                        })
                })
                .unwrap_or(0.0);
            assert_eq!(pair.map(f32::to_bits), [expected.to_bits(); 2]);
        }
        let mut moved = plan;
        moved.starts = vec![13, 101, 167];
        moved.total_source_frames = 211;
        let shifted = rendered(moved.clone(), &[17, 29]);
        for (index, &start) in moved.starts.iter().enumerate() {
            let original_start = [0, 65, 130][index];
            assert_eq!(
                &shifted[start as usize * 2..(start as usize + 32) * 2],
                &output[original_start * 2..(original_start + 32) * 2]
            );
        }
        // If the first observation is absent, marker 1 is still code 1, not
        // code 0. This is a source identity check, not a recovery algorithm.
        assert_ne!(&output[65 * 2..97 * 2], &output[..32 * 2]);
        assert_ne!(&output[130 * 2..162 * 2], &output[65 * 2..97 * 2]);
    }

    #[test]
    fn source_v2_precomputes_29_distinct_segments_of_one_continuous_generator() {
        let plan = SourceSchedule {
            marker_frames: 1024,
            total_source_frames: 29 * 1041,
            starts: (0..29).map(|index| index * 1041).collect(),
            ..indexed_schedule()
        };
        let prepared = plan.prepare(1).unwrap();
        let mut expected = vec![0.0; 29 * 1024];
        MarkerGenerator::new(expected.len(), 0.1)
            .unwrap()
            .fill(&mut expected);
        assert_eq!(prepared.marker, expected);
        let codes = prepared.marker.as_chunks::<1024>().0;
        for (index, code) in codes.iter().enumerate() {
            assert!(codes[..index].iter().all(|prior| prior != code));
        }
    }

    #[test]
    fn source_v2_codebook_cap_is_independent_of_json_duration_and_v1_limits() {
        let mut plan = indexed_schedule();
        plan.marker_frames = 256;
        plan.starts = (0..4096).map(|index| index * 256).collect();
        plan.total_source_frames = 4096 * 256;
        let exact = plan.clone().prepare(600).unwrap();
        assert_eq!(exact.marker.len() * size_of::<f32>(), 4 * 1024 * 1024);
        plan.marker_frames = 257;
        plan.starts = (0..4096).map(|index| index * 257).collect();
        plan.total_source_frames = 4096 * 257;
        assert!(
            plan.clone()
                .prepare(600)
                .unwrap_err()
                .to_string()
                .contains("codebook")
        );
        plan.waveform_version = WAVEFORM_VERSION.into();
        assert_eq!(plan.prepare(600).unwrap().marker.len(), 257);
    }

    #[test]
    fn source_v2_failed_release_retries_identical_code_without_advancing_identity() {
        let mut cursor = SourceCursor::new(Some(indexed_schedule().prepare(1).unwrap()));
        cursor
            .submit(&mut [99.0; 16], true, |samples| {
                assert!(samples.iter().all(|sample| *sample == 0.0));
                Ok(())
            })
            .unwrap();
        for frames in [65, 32, 52, 32] {
            let before = cursor.progress.clone();
            let mut attempted = Vec::new();
            let mut buffer = vec![99.0; frames * 2];
            assert!(
                cursor
                    .submit(&mut buffer, false, |samples| {
                        attempted.extend_from_slice(samples);
                        anyhow::bail!("injected ReleaseBuffer failure")
                    })
                    .is_err()
            );
            assert_eq!(cursor.progress, before);
            cursor
                .submit(&mut buffer, false, |samples| {
                    assert_eq!(samples, attempted);
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(cursor.progress.origin_submission_frame, Some(8));
        assert_eq!(cursor.progress.acknowledged_source_frames, 181);
        assert_eq!(cursor.progress.submitted_nonzero_frames, 96);
        assert!(cursor.progress.schedule_submitted_complete);
    }

    #[test]
    fn source_plan_rejects_invalid_bounds_overflow_order_and_unknown_fields() {
        let valid = schedule();
        assert!(valid.clone().prepare(1).is_ok());
        for (field, value) in [
            ("schema_version", serde_json::json!(2)),
            ("schedule_id", serde_json::json!(" ")),
            ("schedule_id", serde_json::json!("a".repeat(129))),
            ("waveform_version", serde_json::json!("unknown")),
            ("sample_rate", serde_json::json!(44_100)),
            ("channels", serde_json::json!(1)),
            ("marker_frames", serde_json::json!(31)),
            ("marker_frames", serde_json::json!(48_001)),
            ("marker_amplitude", serde_json::json!(0.0)),
            ("marker_amplitude", serde_json::json!(1.0)),
            ("total_source_frames", serde_json::json!(0)),
            ("total_source_frames", serde_json::json!(48_000 * 600 + 1)),
            ("total_source_frames", serde_json::json!(48_001)),
            ("starts", serde_json::json!([])),
            ("starts", serde_json::json!([0, 0])),
            ("starts", serde_json::json!([65, 0])),
            ("starts", serde_json::json!([0, 31])),
            ("starts", serde_json::json!([u64::MAX])),
            ("starts", serde_json::json!([100])),
            ("starts", serde_json::json!(vec![0; 4097])),
        ] {
            let mut value_plan = serde_json::to_value(&valid).unwrap();
            value_plan[field] = value;
            let plan: SourceSchedule = serde_json::from_value(value_plan).unwrap();
            assert!(plan.prepare(1).is_err(), "accepted invalid {field}");
        }
        for amplitude in [f32::NAN, f32::INFINITY, -0.1] {
            let mut plan = valid.clone();
            plan.marker_amplitude = amplitude;
            assert!(plan.prepare(1).is_err());
        }
        let mut unknown = serde_json::to_value(valid).unwrap();
        unknown["recorder_start_frame"] = serde_json::json!(0);
        assert!(serde_json::from_value::<SourceSchedule>(unknown).is_err());
    }

    #[test]
    fn source_v2_uses_existing_schema_bounds_and_rejects_version_fallback() {
        let valid = indexed_schedule();
        for (field, value) in [
            ("schema_version", serde_json::json!(2)),
            (
                "waveform_version",
                serde_json::json!("moiren-marker-xorshift32-indexed-v3"),
            ),
            (
                "waveform_version",
                serde_json::json!("moiren-marker-xorshift32-indexed-v2 "),
            ),
            ("sample_rate", serde_json::json!(44_100)),
            ("channels", serde_json::json!(1)),
            ("marker_frames", serde_json::json!(31)),
            ("marker_frames", serde_json::json!(48_001)),
            ("starts", serde_json::json!([0, 31])),
            ("starts", serde_json::json!([u64::MAX])),
        ] {
            let mut value_plan = serde_json::to_value(&valid).unwrap();
            value_plan[field] = value;
            let plan: SourceSchedule = serde_json::from_value(value_plan).unwrap();
            assert!(plan.prepare(1).is_err(), "accepted invalid v2 {field}");
        }
        let mut missing = serde_json::to_value(valid).unwrap();
        missing.as_object_mut().unwrap().remove("waveform_version");
        assert!(serde_json::from_value::<SourceSchedule>(missing).is_err());
    }

    #[test]
    fn source_submission_has_silent_preroll_and_commits_only_successful_release() {
        let mut cursor = SourceCursor::new(Some(schedule().prepare(1).unwrap()));
        let mut buffer = [1.0; 16];
        cursor
            .submit(&mut buffer, true, |samples| {
                assert!(samples.iter().all(|sample| *sample == 0.0));
                Ok(())
            })
            .unwrap();
        assert_eq!(cursor.progress.native_preroll_frames, 8);
        assert_eq!(cursor.progress.native_submitted_frames, 8);
        assert_eq!(cursor.progress.origin_submission_frame, None);
        assert_eq!(cursor.progress.acknowledged_source_frames, 0);
        let before = cursor.progress.clone();
        assert!(
            cursor
                .submit(&mut buffer, false, |_| anyhow::bail!(
                    "ReleaseBuffer failed"
                ))
                .is_err()
        );
        assert_eq!(cursor.progress, before);
        cursor.submit(&mut buffer, false, |_| Ok(())).unwrap();
        assert_eq!(cursor.progress.origin_submission_frame, Some(8));
        assert_eq!(cursor.progress.acknowledged_source_frames, 8);
        assert_eq!(cursor.progress.native_submitted_frames, 16);
    }

    #[test]
    fn source_generation_is_independent_chunk_continuous_and_zero_after_schedule() {
        let mut cursor = SourceCursor::new(Some(schedule().prepare(1).unwrap()));
        let mut expected_marker = [0.0; 32];
        MarkerGenerator::new(32, 0.1)
            .unwrap()
            .fill(&mut expected_marker);
        let mut position = 0;
        for frames in [1, 7, 31, 3, 24, 1, 55, 13] {
            let mut buffer = vec![99.0; frames * 2];
            cursor
                .submit(&mut buffer, false, |samples| {
                    for frame in samples.as_chunks::<2>().0 {
                        let expected = [0, 65]
                            .into_iter()
                            .find_map(|start| {
                                (position >= start)
                                    .then(|| expected_marker.get(position - start).copied())
                                    .flatten()
                            })
                            .unwrap_or(0.0);
                        assert_eq!(*frame, [expected, expected]);
                        position += 1;
                    }
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(position, 135);
        assert_eq!(cursor.progress.native_submitted_frames, 135);
        assert_eq!(cursor.progress.acknowledged_source_frames, 117);
        assert_eq!(cursor.progress.origin_submission_frame, Some(0));
        assert!(cursor.progress.schedule_submitted_complete);
        let mut postroll = [1.0; 20];
        cursor.submit(&mut postroll, false, |_| Ok(())).unwrap();
        assert!(postroll.iter().all(|sample| *sample == 0.0));
        assert_eq!(cursor.progress.acknowledged_source_frames, 117);
        assert_eq!(cursor.progress.native_submitted_frames, 145);
    }

    #[test]
    fn source_nonzero_submission_receipt_tracks_native_chunks_and_failed_release() {
        for packet_frames in [144, 480, 1056] {
            let plan = SourceSchedule {
                schema_version: 1,
                schedule_id: "native-payload-evidence".into(),
                waveform_version: WAVEFORM_VERSION.into(),
                sample_rate: 48_000,
                channels: 2,
                marker_frames: 1024,
                marker_amplitude: 0.025,
                total_source_frames: 96_000,
                starts: vec![24_000, 72_000],
            };
            let mut cursor = SourceCursor::new(Some(plan.prepare(2).unwrap()));
            let mut buffer = vec![0.0; packet_frames * 2];
            cursor.submit(&mut buffer, true, |_| Ok(())).unwrap();
            let mut observed_nonzero = 0u64;
            while !cursor.progress.schedule_submitted_complete {
                let before = cursor.progress.clone();
                assert!(
                    cursor
                        .submit(&mut buffer, false, |_| {
                            anyhow::bail!("injected ReleaseBuffer failure")
                        })
                        .is_err()
                );
                assert_eq!(cursor.progress, before);
                cursor
                    .submit(&mut buffer, false, |samples| {
                        observed_nonzero += samples
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .filter(|frame| frame.iter().any(|sample| *sample != 0.0))
                            .count() as u64;
                        Ok(())
                    })
                    .unwrap();
            }
            let receipt = serde_json::to_value(&cursor.progress).unwrap();
            assert_eq!(observed_nonzero, 2048);
            assert_eq!(receipt["submitted_nonzero_frames"], observed_nonzero);
            assert_eq!(
                receipt["first_nonzero_submission_frame"],
                packet_frames + 24_000
            );
            cursor.submit(&mut buffer, false, |_| Ok(())).unwrap();
            assert_eq!(
                serde_json::to_value(&cursor.progress).unwrap()["submitted_nonzero_frames"],
                2048
            );
        }
    }

    #[test]
    fn source_accounting_overflow_rejects_before_release_without_saturation() {
        let mut cursor = SourceCursor::new(Some(schedule().prepare(1).unwrap()));
        cursor.progress.native_submitted_frames = u64::MAX;
        let before = cursor.progress.clone();
        assert!(
            cursor
                .submit(&mut [0.0; 2], false, |_| panic!(
                    "overflow must preflight release"
                ))
                .is_err()
        );
        assert_eq!(cursor.progress, before);
    }

    #[test]
    fn source_input_hard_and_caller_byte_caps_apply_before_parsing() {
        let path =
            std::env::temp_dir().join(format!("moiren-source-caps-{}.json", std::process::id()));
        let bytes = serde_json::to_vec(&schedule()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(read_plan(&path, bytes.len() as u64 - 1, 1).is_err());
        assert_eq!(read_plan(&path, bytes.len() as u64, 1).unwrap().0, bytes);
        let mut exact_cap = bytes;
        exact_cap.resize(MAX_SOURCE_BYTES as usize, b' ');
        std::fs::write(&path, &exact_cap).unwrap();
        assert_eq!(read_plan(&path, u64::MAX, 1).unwrap().0, exact_cap);
        std::fs::write(&path, vec![b' '; MAX_SOURCE_BYTES as usize + 1]).unwrap();
        let error = read_plan(&path, u64::MAX, 1).unwrap_err();
        assert!(error.to_string().contains("byte cap"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn source_max_duration_and_marker_count_precompute_only_one_marker() {
        let mut plan = schedule();
        plan.total_source_frames = 48_000 * 600;
        plan.starts = (0..4096).map(|index| index * 32).collect();
        let prepared = plan.prepare(600).unwrap();
        assert_eq!(prepared.marker.len(), 32);
        assert_eq!(prepared.schedule.starts.len(), 4096);
        let mut cursor = SourceCursor::new(Some(prepared));
        let mut output = [0.0; 4];
        cursor.submit(&mut output, false, |_| Ok(())).unwrap();
        assert_eq!(cursor.progress.acknowledged_source_frames, 2);
        assert!(!cursor.progress.schedule_submitted_complete);
    }

    #[test]
    fn source_diagnostic_pulses_are_unchanged_and_never_acquire_marker_provenance() {
        let mut cursor = SourceCursor::new(None);
        assert!(!cursor.has_schedule());
        let mut preroll = [0.0; 1024];
        cursor.submit(&mut preroll, true, |_| Ok(())).unwrap();
        for (index, frame) in preroll.as_chunks::<2>().0.iter().enumerate() {
            let expected = if index < 256 { 0.1 } else { 0.0 };
            assert_eq!(*frame, [expected; 2]);
        }
        let mut next = [0.0; 96];
        cursor.submit(&mut next, false, |_| Ok(())).unwrap();
        assert!(next.iter().all(|value| *value == 0.0));
        assert_eq!(cursor.progress.native_preroll_frames, 512);
        assert_eq!(cursor.progress.native_submitted_frames, 560);
        assert_eq!(cursor.progress.origin_submission_frame, None);
        assert_eq!(cursor.progress.acknowledged_source_frames, 0);
        assert!(!cursor.progress.schedule_submitted_complete);
        assert!(cursor.provenance().schedule.is_none());
    }

    #[test]
    fn source_failed_preroll_and_later_release_keep_exact_committed_progress() {
        let mut cursor = SourceCursor::new(Some(schedule().prepare(1).unwrap()));
        assert!(cursor.has_schedule());
        let mut buffer = [0.0; 20];
        let initial = cursor.progress.clone();
        assert!(
            cursor
                .submit(&mut buffer, true, |_| anyhow::bail!(
                    "preroll ReleaseBuffer failed"
                ))
                .is_err()
        );
        assert_eq!(cursor.progress, initial);
        cursor.submit(&mut buffer, true, |_| Ok(())).unwrap();
        cursor.submit(&mut buffer, false, |_| Ok(())).unwrap();
        let committed = cursor.progress.clone();
        assert!(
            cursor
                .submit(&mut buffer, false, |_| anyhow::bail!(
                    "later ReleaseBuffer failed"
                ))
                .is_err()
        );
        assert_eq!(cursor.progress, committed);
        assert_eq!(cursor.progress.origin_submission_frame, Some(10));
        assert_eq!(cursor.progress.acknowledged_source_frames, 10);
        assert!(
            cursor
                .submit(&mut buffer, true, |_| panic!(
                    "late preroll must be rejected"
                ))
                .is_err()
        );
        assert!(
            cursor
                .submit(&mut [], false, |_| panic!("empty payload must be rejected"))
                .is_err()
        );
        assert!(
            cursor
                .submit(&mut [0.0], false, |_| panic!(
                    "partial frame must be rejected"
                ))
                .is_err()
        );
    }
}
