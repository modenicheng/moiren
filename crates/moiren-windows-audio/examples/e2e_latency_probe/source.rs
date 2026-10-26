//! Source-clock plans and acknowledged native submissions, never recorder windows.

use anyhow::{Context, Result, ensure};
use moiren_windows_audio::latency::MarkerGenerator;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

pub const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
pub const WAVEFORM_VERSION: &str = "moiren-marker-xorshift32-v1";

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
            self.waveform_version == WAVEFORM_VERSION,
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
        let mut marker = vec![0.0; self.marker_frames];
        MarkerGenerator::new(self.marker_frames, self.marker_amplitude)?.fill(&mut marker);
        Ok(PreparedSource {
            schedule: self,
            marker,
        })
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
                        plan.marker.get(offset as usize).copied().unwrap_or(0.0)
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
