//! Bounded control-side soak evidence. No wall clock or PCM stands in for Graph progress.
use super::validation::{Observation, TimelineProgress};
use anyhow::{Context, ensure};
use moiren_engine::meter::signal::AudioWindow;
use serde::Serialize;

const FRESH_MS: u64 = 2_000;
pub(super) const PATTERN_MS: u64 = 30_000;
pub(super) const SETTLE_MS: u64 = 1_000;

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct NativeDiagnostics {
    pub capture: CapturePosition,
    pub render_clock: ClockProgress,
    pub capture_clock: ClockProgress,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct IdleSoak {
    pub requested_ms: u64,
    pub elapsed_ms: u64,
    pub completed: bool,
    pub quality_failed: bool,
    pub timeline: TimelineProgress,
    pub verified_windows: u64,
    pub maximum_observation_gap_ms: u64,
    pub maximum_advance_gap_ms: u64,
    last_poll_ms: u64,
    last_advance_ms: u64,
    window_started_ms: u64,
    window_baseline: u64,
}

impl IdleSoak {
    pub fn new(requested_ms: u64) -> Self {
        Self {
            requested_ms,
            elapsed_ms: 0,
            completed: false,
            quality_failed: false,
            timeline: TimelineProgress::default(),
            verified_windows: 0,
            maximum_observation_gap_ms: 0,
            maximum_advance_gap_ms: 0,
            last_poll_ms: 0,
            last_advance_ms: 0,
            window_started_ms: 0,
            window_baseline: 0,
        }
    }
    pub fn poll(&mut self, now_ms: u64, header: Option<AudioWindow>) -> anyhow::Result<()> {
        ensure!(
            !self.quality_failed,
            "idle soak has a failed observation window"
        );
        let result = self.poll_window(now_ms, header);
        self.quality_failed |= result.is_err();
        result
    }
    fn poll_window(&mut self, now_ms: u64, header: Option<AudioWindow>) -> anyhow::Result<()> {
        ensure!(now_ms >= self.elapsed_ms, "idle observation time regressed");
        self.elapsed_ms = now_ms;
        let gap = now_ms - self.last_poll_ms;
        self.maximum_observation_gap_ms = self.maximum_observation_gap_ms.max(gap);
        ensure!(
            gap <= FRESH_MS,
            "idle timeline was not observed for {gap}ms"
        );
        self.last_poll_ms = now_ms;
        if let Some(header) = header {
            if let Some(last) = self.timeline.last {
                ensure!(
                    header.timeline_epoch == last.timeline_epoch,
                    "idle Graph timeline epoch changed"
                );
            }
            let previous = self.timeline.last.map(|last| last.frame_end);
            self.timeline.observe(header)?;
            if previous.is_none_or(|end| header.frame_end > end) {
                let gap = now_ms - self.last_advance_ms;
                self.maximum_advance_gap_ms = self.maximum_advance_gap_ms.max(gap);
                ensure!(
                    gap <= FRESH_MS,
                    "idle Graph timeline stopped advancing for {gap}ms"
                );
                self.last_advance_ms = now_ms;
            }
        }
        let age = now_ms - self.last_advance_ms;
        self.maximum_advance_gap_ms = self.maximum_advance_gap_ms.max(age);
        ensure!(age <= FRESH_MS, "idle Graph timeline is stale by {age}ms");
        if now_ms - self.window_started_ms >= 1_000 {
            ensure!(
                self.timeline.forward_advances - self.window_baseline >= 2,
                "idle window lacks two actual frame_end advances"
            );
            self.verified_windows += 1;
            self.window_baseline = self.timeline.forward_advances;
            self.window_started_ms = now_ms;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> anyhow::Result<()> {
        ensure!(
            !self.quality_failed,
            "idle soak has a failed observation window"
        );
        ensure!(
            self.elapsed_ms >= self.requested_ms,
            "idle soak duration incomplete"
        );
        ensure!(
            self.end_observed(),
            "idle soak lacks a new frame_end observation at/after requested duration"
        );
        self.timeline.require_progress()?;
        ensure!(
            self.verified_windows > 0,
            "no idle timeline window qualified"
        );
        self.completed = true;
        Ok(())
    }

    pub fn end_observed(&self) -> bool {
        self.last_advance_ms >= self.requested_ms
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct PcmSoak {
    pub requested_ms: u64,
    pub elapsed_ms: u64,
    pub completed: bool,
    pub quality_failed: bool,
    pub source_changes: u64,
    pub qualified_windows: u64,
    pub expected: [f32; 2],
    pub settling_until_ms: u64,
    pub settling_frames: u64,
    pub settling_discontinuities: u64,
    pub settling_nonfinite_frames: u64,
    pub settling_matching_frames: u64,
    pub settling_silent_frames: u64,
    pub settling_other_finite_frames: u64,
    pub validated: Observation,
    pub current_window: Observation,
    pub last_packet_ms: Option<u64>,
    pub maximum_packet_gap_ms: u64,
}

impl PcmSoak {
    pub fn new(requested_ms: u64, expected: [f32; 2]) -> Self {
        Self {
            requested_ms,
            elapsed_ms: 0,
            completed: false,
            quality_failed: false,
            source_changes: 0,
            qualified_windows: 0,
            expected,
            settling_until_ms: 0,
            settling_frames: 0,
            settling_discontinuities: 0,
            settling_nonfinite_frames: 0,
            settling_matching_frames: 0,
            settling_silent_frames: 0,
            settling_other_finite_frames: 0,
            validated: Observation::default(),
            current_window: Observation::default(),
            last_packet_ms: None,
            maximum_packet_gap_ms: 0,
        }
    }
    pub fn require_fresh(&mut self, now_ms: u64) -> anyhow::Result<()> {
        self.quality_failed |= now_ms < self.elapsed_ms;
        ensure!(now_ms >= self.elapsed_ms, "PCM observation time regressed");
        self.elapsed_ms = now_ms;
        let gap = now_ms - self.last_packet_ms.unwrap_or(0);
        self.maximum_packet_gap_ms = self.maximum_packet_gap_ms.max(gap);
        self.quality_failed |= gap > FRESH_MS;
        ensure!(gap <= FRESH_MS, "PCM packet observation stale by {gap}ms");
        Ok(())
    }
    pub fn observe(
        &mut self,
        now_ms: u64,
        samples: &[f32],
        discontinuity: bool,
    ) -> anyhow::Result<()> {
        ensure!(!self.quality_failed, "PCM soak has a failed quality window");
        let result = self.observe_packet(now_ms, samples, discontinuity);
        self.quality_failed |= result.is_err();
        result
    }
    fn observe_packet(
        &mut self,
        now_ms: u64,
        samples: &[f32],
        discontinuity: bool,
    ) -> anyhow::Result<()> {
        // Check the previous packet's age before accepting a fresh packet so a
        // delayed native call cannot erase a long unobserved interval.
        self.require_fresh(now_ms)?;
        ensure!(
            samples.len().is_multiple_of(2),
            "incomplete soak stereo frame"
        );
        if samples.is_empty() {
            return Ok(());
        }
        self.last_packet_ms = Some(now_ms);
        if now_ms < self.settling_until_ms {
            self.settling_frames += samples.len() as u64 / 2;
            self.settling_discontinuities += u64::from(discontinuity);
            for frame in samples.as_chunks::<2>().0 {
                if frame.iter().any(|sample| !sample.is_finite()) {
                    self.settling_nonfinite_frames += 1;
                } else if frame
                    .iter()
                    .all(|sample| sample.abs() <= super::validation::TOLERANCE)
                {
                    self.settling_silent_frames += 1;
                } else if frame.iter().zip(self.expected).all(|(actual, expected)| {
                    (*actual - expected).abs() <= super::validation::TOLERANCE
                }) {
                    self.settling_matching_frames += 1;
                } else {
                    self.settling_other_finite_frames += 1;
                }
            }
            ensure!(
                samples.iter().all(|sample| sample.is_finite()),
                "nonfinite PCM during fixed soak settling"
            );
            return Ok(());
        }
        let before = self.current_window.clone();
        self.current_window
            .first_frame_ms
            .get_or_insert(now_ms.into());
        self.current_window.last_frame_ms = Some(now_ms.into());
        let result = self
            .current_window
            .observe_packet(samples, self.expected, discontinuity);
        let current = &self.current_window;
        let total = &mut self.validated;
        total.total_frames += current.total_frames - before.total_frames;
        total.matching_frames += current.matching_frames - before.matching_frames;
        total.silent_frames += current.silent_frames - before.silent_frames;
        total.unexpected_frames += current.unexpected_frames - before.unexpected_frames;
        total.discontinuities += current.discontinuities - before.discontinuities;
        total.consecutive_matching_frames = current.consecutive_matching_frames;
        total.longest_matching_run = total.longest_matching_run.max(current.longest_matching_run);
        total.first_frame_ms.get_or_insert(now_ms.into());
        total.last_frame_ms = Some(now_ms.into());
        result?;
        ensure!(!discontinuity, "PCM soak capture discontinuity/gap");
        ensure!(
            current.silent_frames == 0,
            "PCM soak unexpected silent/drop frame"
        );
        Ok(())
    }
    pub fn change_pattern(&mut self, now_ms: u64, expected: [f32; 2]) -> anyhow::Result<()> {
        ensure!(!self.quality_failed, "PCM soak has a failed quality window");
        self.require_fresh(now_ms)?;
        self.current_window
            .require_signal()
            .context("preceding soak source window")?;
        self.qualified_windows += 1;
        self.source_changes += 1;
        self.current_window = Observation::default();
        self.expected = expected;
        self.settling_until_ms = now_ms + SETTLE_MS;
        Ok(())
    }
    pub fn finish(&mut self, now_ms: u64) -> anyhow::Result<()> {
        ensure!(!self.quality_failed, "PCM soak has a failed quality window");
        self.require_fresh(now_ms)?;
        ensure!(now_ms >= self.requested_ms, "PCM soak duration incomplete");
        ensure!(
            self.end_observed(),
            "PCM soak lacks a capture packet at/after requested duration"
        );
        ensure!(now_ms >= self.settling_until_ms, "PCM soak still settling");
        self.current_window.require_signal()?;
        if !self.completed {
            self.qualified_windows += 1;
        }
        self.completed = true;
        Ok(())
    }

    pub fn end_observed(&self) -> bool {
        self.last_packet_ms
            .is_some_and(|last| last >= self.requested_ms)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct CapturePosition {
    pub packets: u64,
    pub frames: u64,
    pub discontinuities: u64,
    pub timestamp_errors: u64,
    pub inferred_gap_frames: u64,
    pub regressions: u64,
    pub first_device_frame: Option<u64>,
    pub last_device_frame: Option<u64>,
    pub last_qpc_100ns: Option<u64>,
    expected_next_frame: Option<u64>,
}

impl CapturePosition {
    pub fn observe(
        &mut self,
        frame: u64,
        qpc: u64,
        frames: u32,
        discontinuity: bool,
        timestamp_error: bool,
    ) -> bool {
        self.packets += 1;
        self.frames += u64::from(frames);
        self.discontinuities += u64::from(discontinuity);
        self.timestamp_errors += u64::from(timestamp_error);
        if timestamp_error {
            self.expected_next_frame = None;
            return true;
        }
        let gap = self
            .expected_next_frame
            .is_some_and(|expected| frame != expected);
        if let Some(expected) = self.expected_next_frame {
            if frame > expected {
                self.inferred_gap_frames += frame - expected;
            } else if frame < expected {
                self.regressions += 1;
            }
        }
        let qpc_regressed = self.last_qpc_100ns.is_some_and(|last| qpc < last);
        if qpc_regressed {
            self.regressions += 1;
        }
        self.first_device_frame.get_or_insert(frame);
        self.last_device_frame = Some(frame);
        self.last_qpc_100ns = Some(qpc);
        self.expected_next_frame = frame.checked_add(u64::from(frames));
        discontinuity || gap || qpc_regressed || self.expected_next_frame.is_none()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct ClockProgress {
    pub frequency: u64,
    pub samples: u64,
    pub inaccurate_samples: u64,
    pub regressions: u64,
    pub forward_advances: u64,
    pub first_position: Option<u64>,
    pub first_qpc_100ns: Option<u64>,
    pub last_observed_ms: u64,
    pub last_position: Option<u64>,
    pub last_qpc_100ns: Option<u64>,
    pub last_advance_ms: u64,
}

impl ClockProgress {
    pub fn observe(
        &mut self,
        position: u64,
        qpc: u64,
        accurate: bool,
        now_ms: u64,
    ) -> anyhow::Result<()> {
        self.samples += 1;
        self.last_observed_ms = now_ms;
        ensure!(self.frequency > 0, "invalid native audio clock frequency");
        if accurate {
            if let Some(last) = self.last_position {
                self.regressions +=
                    u64::from(position < last || qpc < self.last_qpc_100ns.unwrap_or(0));
                ensure!(
                    position >= last && qpc >= self.last_qpc_100ns.unwrap_or(0),
                    "native audio clock regressed"
                );
                if position > last {
                    ensure!(
                        now_ms.saturating_sub(self.last_advance_ms) <= 3_000,
                        "native audio clock advance was stale"
                    );
                    self.forward_advances += 1;
                    self.last_advance_ms = now_ms;
                }
            }
            self.first_position.get_or_insert(position);
            self.first_qpc_100ns.get_or_insert(qpc);
            self.last_position = Some(position);
            self.last_qpc_100ns = Some(qpc);
        } else {
            self.inaccurate_samples += 1;
        }
        ensure!(
            now_ms.saturating_sub(self.last_advance_ms) <= 3_000,
            "native audio clock stopped advancing"
        );
        Ok(())
    }
    pub fn require_fresh(&self, now_ms: u64) -> anyhow::Result<()> {
        ensure!(
            self.forward_advances >= 1 && self.frequency > 0,
            "native audio clock has no accurate forward samples"
        );
        ensure!(
            now_ms.saturating_sub(self.last_advance_ms) <= 3_000,
            "native audio clock evidence stale"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::{EXPECTED, SECOND_EXPECTED};

    fn header(end: u64) -> AudioWindow {
        AudioWindow {
            timeline_epoch: 1,
            frame_start: end - 480,
            frame_end: end,
            sample_rate: 48_000.0,
        }
    }

    #[test]
    fn idle_soak_requires_advances_in_every_window_and_rejects_late_fresh_header() {
        let mut idle = IdleSoak::new(3_000);
        for now in (0..=3_000).step_by(100) {
            idle.poll(now, Some(header(480 + now * 48))).unwrap();
        }
        idle.finish().unwrap();
        assert!(idle.completed);
        assert_eq!(idle.verified_windows, 3);
        let mut stalled = IdleSoak::new(8_000);
        stalled.poll(0, Some(header(480))).unwrap();
        assert!(stalled.poll(1_000, Some(header(480))).is_err());
        let mut unobserved = IdleSoak::new(8_000);
        unobserved.poll(0, Some(header(480))).unwrap();
        assert!(unobserved.poll(2_001, Some(header(96_480))).is_err());
    }

    #[test]
    fn idle_epoch_change_and_early_completion_are_not_continuous_graph_proof() {
        let mut idle = IdleSoak::new(3_000);
        idle.poll(0, Some(header(480))).unwrap();
        assert!(idle.finish().is_err());
        let mut changed = header(960);
        changed.timeline_epoch = 2;
        assert!(idle.poll(100, Some(changed)).is_err());
        assert!(idle.poll(3_000, Some(header(1440))).is_err());
        assert!(idle.finish().is_err());
    }

    #[test]
    fn eight_hour_idle_keeps_fixed_evidence_and_checks_every_window() {
        let requested = 28_800_000;
        let mut idle = IdleSoak::new(requested);
        for now in (0..=requested).step_by(250) {
            idle.poll(now, Some(header(480 + now * 48))).unwrap();
        }
        idle.finish().unwrap();
        assert_eq!(idle.verified_windows, 28_800);
        assert_eq!(idle.timeline.forward_advances, requested / 250);
        assert!(serde_json::to_vec(&idle).unwrap().len() < 2_048);
    }

    #[test]
    fn pcm_soak_checks_full_duration_strict_samples_and_data_freshness() {
        let mut pcm = PcmSoak::new(2_000, EXPECTED);
        for now in (0..=2_000).step_by(10) {
            pcm.observe(now, &EXPECTED.repeat(480), false).unwrap();
        }
        pcm.finish(2_000).unwrap();
        assert!(pcm.completed);
        assert_eq!(pcm.validated.matching_frames, 201 * 480);
        let mut silence = PcmSoak::new(2_000, EXPECTED);
        assert!(silence.observe(0, &[0.0, 0.0], false).is_err());
        assert_eq!(silence.validated.silent_frames, 1);
        assert!(
            PcmSoak::new(2_000, EXPECTED)
                .observe(0, &EXPECTED, true)
                .is_err()
        );
        assert!(PcmSoak::new(8_000, EXPECTED).require_fresh(2_001).is_err());
    }

    #[test]
    fn source_change_has_fixed_settling_then_rejects_old_signal() {
        let mut pcm = PcmSoak::new(60_000, EXPECTED);
        for now in (0..=1_000).step_by(10) {
            pcm.observe(now, &EXPECTED.repeat(480), false).unwrap();
        }
        pcm.change_pattern(1_000, SECOND_EXPECTED).unwrap();
        pcm.observe(1_999, &EXPECTED, false).unwrap();
        assert_eq!(pcm.settling_other_finite_frames, 1);
        assert!(pcm.observe(2_000, &EXPECTED, false).is_err());
        assert_eq!(pcm.source_changes, 1);
        assert_eq!(pcm.validated.unexpected_frames, 1);
        assert!(!pcm.completed);
        assert!(
            PcmSoak::new(60_000, EXPECTED)
                .change_pattern(1, SECOND_EXPECTED)
                .is_err()
        );
        let mut finite = PcmSoak::new(60_000, EXPECTED);
        finite.settling_until_ms = 1_000;
        assert!(finite.observe(0, &[f32::NAN, 0.0], false).is_err());
    }

    #[test]
    fn early_signal_success_or_later_failed_window_never_qualifies_whole_soak() {
        let mut pcm = PcmSoak::new(3_000, EXPECTED);
        for now in (0..=1_000).step_by(10) {
            pcm.observe(now, &EXPECTED.repeat(480), false).unwrap();
        }
        assert!(
            pcm.finish(1_000).is_err(),
            "early 48k frames cannot finish 3s"
        );
        assert!(pcm.observe(1_010, &[0.0, 0.0], false).is_err());
        assert!(
            pcm.finish(3_000).is_err(),
            "prior 48k cannot hide later failed window"
        );
        assert!(!pcm.completed);
        assert_eq!(pcm.validated.silent_frames, 1);
    }

    #[test]
    fn each_source_window_must_qualify_and_counts_remain_bounded_after_switch() {
        let mut pcm = PcmSoak::new(4_000, EXPECTED);
        let packet = EXPECTED.repeat(480);
        for now in (0..=1_000).step_by(10) {
            pcm.observe(now, &packet, false).unwrap();
        }
        pcm.change_pattern(1_000, SECOND_EXPECTED).unwrap();
        for now in (1_010..=4_000).step_by(10) {
            pcm.observe(now, &SECOND_EXPECTED.repeat(480), false)
                .unwrap();
        }
        pcm.finish(4_000).unwrap();
        assert!(pcm.completed);
        assert_eq!(pcm.qualified_windows, 2);
        assert!(pcm.validated.matching_frames > pcm.current_window.matching_frames);
        assert!(serde_json::to_vec(&pcm).unwrap().len() < 2_048);
        assert_eq!(PATTERN_MS, 30_000);
    }

    #[test]
    fn requested_wall_duration_without_new_end_evidence_cannot_finish() {
        let mut idle = IdleSoak::new(2_500);
        let mut pcm = PcmSoak::new(2_500, EXPECTED);
        for now in (0..=2_000).step_by(10) {
            idle.poll(now, Some(header(480 + now * 48))).unwrap();
            pcm.observe(now, &EXPECTED.repeat(480), false).unwrap();
        }
        idle.poll(2_500, None).unwrap();
        assert!(
            idle.finish().is_err(),
            "old signal plus elapsed wall time is insufficient"
        );
        assert!(
            pcm.finish(2_500).is_err(),
            "old 48k PCM plus elapsed wall time is insufficient"
        );
        idle.poll(2_510, Some(header(480 + 2_510 * 48))).unwrap();
        pcm.observe(2_510, &EXPECTED, false).unwrap();
        idle.finish().unwrap();
        pcm.finish(2_510).unwrap();
    }

    #[test]
    fn capture_positions_count_inferred_gaps_regressions_and_untrusted_timestamps() {
        let mut position = CapturePosition::default();
        assert!(!position.observe(0, 100, 480, false, false));
        assert!(!position.observe(480, 200, 480, false, false));
        assert!(position.observe(1_440, 300, 480, false, false));
        assert_eq!(position.inferred_gap_frames, 480);
        assert!(position.observe(0, 400, 480, false, false));
        assert_eq!(position.regressions, 1);
        assert!(position.observe(u64::MAX, 500, 480, false, true));
        assert_eq!(position.timestamp_errors, 1);
        assert_eq!(position.last_device_frame, Some(0));
    }

    #[test]
    fn inaccurate_stalled_or_regressing_native_clock_cannot_qualify() {
        let mut clock = ClockProgress {
            frequency: 48_000,
            ..Default::default()
        };
        clock.observe(0, 1, true, 0).unwrap();
        clock.observe(48_000, 10_000_001, true, 1_000).unwrap();
        clock.require_fresh(1_000).unwrap();
        assert!(clock.observe(47_999, 10_000_002, true, 1_001).is_err());
        assert!(clock.observe(96_000, 10_000_000, true, 2_000).is_err());
        assert!(clock.observe(96_000, 20_000_001, false, 4_001).is_err());
        assert_eq!(clock.inaccurate_samples, 1);
    }
}
