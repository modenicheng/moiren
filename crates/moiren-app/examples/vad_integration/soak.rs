//! Bounded control-side soak evidence. No wall clock or PCM stands in for Graph progress.
use super::validation::{Observation, TimelineProgress};
use anyhow::{Context, ensure};
use moiren_engine::meter::signal::AudioWindow;
use serde::Serialize;

const FRESH_MS: u64 = 2_000;
const SOAK_SAMPLE_RATE: u64 = 48_000;
const MAX_PACKET_FRAMES: u64 = 9_600;
const BOUNDARY_SCHEDULING_SLACK_MS: u64 = 50;
// Audio clocks and QPC should be much closer than this; 5% leaves room for
// coarse clock implementations while still rejecting grossly slowed streams.
const MAX_CLOCK_RATE_DEVIATION_PPM: u64 = 50_000;
// Ignore sub-100ms pairs so API/QPC sampling jitter does not dominate the rate fit.
const MIN_CLOCK_RATE_INTERVAL_100NS: u64 = 1_000_000;
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
    pub settling_duration_ms: u64,
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
    pub first_packet_ms: Option<u64>,
    pub last_packet_observed_ms: Option<u64>,
    pub captured_frames: u64,
    pub first_packet_frames: Option<u64>,
    pub last_packet_frames: u64,
    pub coverage_required_frames: Option<u64>,
    pub coverage_ratio: Option<f64>,
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
            settling_duration_ms: 0,
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
            first_packet_ms: None,
            last_packet_observed_ms: None,
            captured_frames: 0,
            first_packet_frames: None,
            last_packet_frames: 0,
            coverage_required_frames: None,
            coverage_ratio: None,
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
        let packet_frames = (samples.len() / 2) as u64;
        ensure!(
            packet_frames <= MAX_PACKET_FRAMES,
            "PCM soak packet exceeds native buffer boundary"
        );
        self.first_packet_frames.get_or_insert(packet_frames);
        self.first_packet_ms.get_or_insert(now_ms);
        self.last_packet_frames = packet_frames;
        self.captured_frames = self
            .captured_frames
            .checked_add(packet_frames)
            .context("PCM soak captured-frame counter overflow")?;
        self.last_packet_ms = Some(now_ms);
        self.last_packet_observed_ms = Some(now_ms);
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
        ensure!(
            now_ms >= self.settling_until_ms,
            "cannot change PCM source during settling"
        );
        self.current_window
            .require_signal()
            .context("preceding soak source window")?;
        self.qualified_windows += 1;
        self.source_changes += 1;
        self.current_window = Observation::default();
        self.expected = expected;
        self.settling_until_ms = now_ms + SETTLE_MS;
        self.settling_duration_ms = self
            .settling_duration_ms
            .checked_add(SETTLE_MS.min(self.requested_ms.saturating_sub(now_ms)))
            .context("PCM soak settling duration overflow")?;
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
        let maximum_initial_edge_ms = self
            .first_packet_frames
            .unwrap_or(MAX_PACKET_FRAMES)
            .saturating_mul(1_000)
            .div_ceil(SOAK_SAMPLE_RATE)
            .saturating_add(BOUNDARY_SCHEDULING_SLACK_MS);
        let maximum_final_edge_ms = self
            .last_packet_frames
            .saturating_mul(1_000)
            .div_ceil(SOAK_SAMPLE_RATE)
            .saturating_add(BOUNDARY_SCHEDULING_SLACK_MS);
        ensure!(
            self.first_packet_ms.unwrap_or(u64::MAX) <= maximum_initial_edge_ms,
            "PCM soak initial capture boundary was late: first packet at {}ms, limit {maximum_initial_edge_ms}ms",
            self.first_packet_ms.unwrap_or(u64::MAX)
        );
        let last_packet_ms = self.last_packet_observed_ms.unwrap_or(u64::MAX);
        ensure!(
            last_packet_ms.saturating_sub(self.requested_ms) <= maximum_final_edge_ms,
            "PCM soak final capture boundary was late: last packet at {last_packet_ms}ms, requested {}ms, limit {maximum_final_edge_ms}ms",
            self.requested_ms
        );
        ensure!(now_ms >= self.settling_until_ms, "PCM soak still settling");
        let requested_frames = u128::from(self.requested_ms)
            .checked_mul(u128::from(SOAK_SAMPLE_RATE))
            .context("PCM soak requested-frame count overflow")?
            / 1_000;
        let settling_frames = u128::from(self.settling_duration_ms)
            .checked_mul(u128::from(SOAK_SAMPLE_RATE))
            .context("PCM soak settling-frame count overflow")?
            / 1_000;
        let eligible_frames = requested_frames.saturating_sub(settling_frames);
        let required_frames = u64::try_from(eligible_frames)
            .context("PCM soak required-frame count exceeds evidence range")?;
        self.coverage_required_frames = Some(required_frames);
        self.coverage_ratio =
            Some(self.validated.total_frames as f64 / eligible_frames.max(1) as f64);
        ensure!(
            self.validated.total_frames >= required_frames,
            "validated PCM frame coverage too low: validated={}, required={} after {}ms settling (ratio {:.3})",
            self.validated.total_frames,
            required_frames,
            self.settling_duration_ms,
            self.coverage_ratio.unwrap_or_default()
        );
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

/// One correlation window of looping music material: the alignment tracker
/// follows clock-rate drift while an inserted silence or repeated stretch
/// breaks the normalized correlation and fails the window.
pub(super) const MUSIC_WINDOW_MS: u64 = 1_000;
pub(super) const MUSIC_WINDOW_FRAMES: usize =
    SOAK_SAMPLE_RATE as usize * MUSIC_WINDOW_MS as usize / 1_000;
/// The pipeline (native buffers both directions, driver, carriers, FIR)
/// delays capture behind submission by an initially unknown bounded amount.
const MUSIC_ACQUIRE_LAG_FRAMES: i64 = 4 * 48_000;
/// After acquisition the alignment can only drift by clock-rate error.
const MUSIC_TRACK_LAG_FRAMES: i64 = 2_400;
const MUSIC_INITIAL_SETTLE_MS: u64 = 2_000;
/// Grid stride for alignment candidates in full-rate frames.
pub(super) const MUSIC_GRID_STRIDE: usize = 64;
/// Sub-window size for the exact refinement stage.
const MUSIC_REFINE_SUBWINDOW: usize = 8_192;
/// Loop-equivalent candidates score identically; keep this many distinct
/// loop residues so one sidelobe cannot eat the whole candidate set.
const MUSIC_COARSE_CANDIDATES: usize = 4;
const MUSIC_MIN_CORRELATION: f32 = 0.95;
const MUSIC_QUIET_ENERGY: f32 = 1e-10;
const MUSIC_QUALIFIED_RATIO: f64 = 0.99;
const MUSIC_MAX_FAILED_STREAK: u64 = 3;
const RECENT_WINDOWS: usize = 32;

#[derive(Clone, Debug, Serialize)]
pub(super) struct MusicWindow {
    pub start_ms: u64,
    pub lag_frames: i64,
    pub correlation: f32,
    pub both_quiet: bool,
    pub qualified: bool,
}

/// One completed capture window handed to the analysis thread. The job
/// carries the tracker state at collection time so results stay valid even
/// when earlier windows are still being analyzed.
pub(super) struct MusicJob {
    pub start_ms: u64,
    pub captured_base: u64,
    pub previous_lag: i64,
    pub acquired: bool,
    pub mono: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct MusicSoak {
    pub requested_ms: u64,
    pub elapsed_ms: u64,
    pub completed: bool,
    pub quality_failed: bool,
    pub settling_ms: u64,
    pub settling_frames: u64,
    pub material_frames: usize,
    pub material_loops: u64,
    pub packets: u64,
    pub captured_frames: u64,
    pub windows_total: u64,
    pub windows_qualified: u64,
    pub windows_both_quiet: u64,
    pub windows_failed: u64,
    pub longest_failed_streak: u64,
    pub minimum_correlation: f32,
    pub maximum_abs_lag_frames: i64,
    pub maximum_lag_step_frames: i64,
    pub lag_frames: i64,
    pub alignment_acquired: bool,
    pub first_packet_ms: Option<u64>,
    pub last_packet_ms: Option<u64>,
    pub maximum_packet_gap_ms: u64,
    pub first_packet_frames: Option<u64>,
    pub last_packet_frames: u64,
    pub coverage_required_frames: Option<u64>,
    pub coverage_ratio: Option<f64>,
    pub recent_windows: Vec<MusicWindow>,
    #[serde(skip)]
    settling_until_ms: u64,
    #[serde(skip)]
    window_started_ms: u64,
    #[serde(skip)]
    pending: Vec<f32>,
    #[serde(skip)]
    captured_base: u64,
    #[serde(skip)]
    failed_streak: u64,
}

impl MusicSoak {
    pub fn new(requested_ms: u64, material: &super::material::Material) -> Self {
        Self {
            requested_ms,
            elapsed_ms: 0,
            completed: false,
            quality_failed: false,
            settling_ms: 0,
            settling_frames: 0,
            material_frames: material.frames,
            material_loops: 0,
            packets: 0,
            captured_frames: 0,
            windows_total: 0,
            windows_qualified: 0,
            windows_both_quiet: 0,
            windows_failed: 0,
            longest_failed_streak: 0,
            minimum_correlation: 1.0,
            maximum_abs_lag_frames: 0,
            maximum_lag_step_frames: 0,
            lag_frames: 0,
            alignment_acquired: false,
            first_packet_ms: None,
            last_packet_ms: None,
            maximum_packet_gap_ms: 0,
            first_packet_frames: None,
            last_packet_frames: 0,
            coverage_required_frames: None,
            coverage_ratio: None,
            recent_windows: Vec::new(),
            settling_until_ms: requested_ms.min(MUSIC_INITIAL_SETTLE_MS),
            window_started_ms: 0,
            pending: Vec::with_capacity(MUSIC_WINDOW_FRAMES * 2 + 2 * MAX_PACKET_FRAMES as usize),
            captured_base: 0,
            failed_streak: 0,
        }
    }

    pub fn require_fresh(&mut self, now_ms: u64) -> anyhow::Result<()> {
        self.quality_failed |= now_ms < self.elapsed_ms;
        ensure!(
            now_ms >= self.elapsed_ms,
            "music observation time regressed"
        );
        self.elapsed_ms = now_ms;
        let gap = now_ms - self.last_packet_ms.unwrap_or(0);
        self.maximum_packet_gap_ms = self.maximum_packet_gap_ms.max(gap);
        self.quality_failed |= gap > FRESH_MS;
        ensure!(gap <= FRESH_MS, "music packet observation stale by {gap}ms");
        Ok(())
    }

    pub fn observe_packet(
        &mut self,
        now_ms: u64,
        samples: &[f32],
        discontinuity: bool,
    ) -> anyhow::Result<()> {
        ensure!(
            !self.quality_failed,
            "music soak has a failed quality window"
        );
        let result = self.observe_packet_inner(now_ms, samples, discontinuity);
        self.quality_failed |= result.is_err();
        result
    }

    fn observe_packet_inner(
        &mut self,
        now_ms: u64,
        samples: &[f32],
        discontinuity: bool,
    ) -> anyhow::Result<()> {
        self.require_fresh(now_ms)?;
        ensure!(
            samples.len().is_multiple_of(2),
            "incomplete music stereo frame"
        );
        ensure!(
            samples.iter().all(|sample| sample.is_finite()),
            "nonfinite music capture sample"
        );
        ensure!(!discontinuity, "music soak capture discontinuity/gap");
        if samples.is_empty() {
            return Ok(());
        }
        let frames = (samples.len() / 2) as u64;
        ensure!(
            frames <= MAX_PACKET_FRAMES,
            "music soak packet exceeds native buffer boundary"
        );
        self.packets += 1;
        self.captured_frames += frames;
        self.first_packet_frames.get_or_insert(frames);
        self.first_packet_ms.get_or_insert(now_ms);
        self.last_packet_frames = frames;
        self.last_packet_ms = Some(now_ms);
        if now_ms < self.settling_until_ms {
            self.settling_frames += frames;
            return Ok(());
        }
        if self.windows_total == 0 && self.pending.is_empty() {
            self.settling_ms = self.settling_until_ms;
            self.window_started_ms = now_ms;
        }
        self.pending.extend_from_slice(samples);
        Ok(())
    }

    pub fn window_ready(&self) -> bool {
        self.pending.len() >= MUSIC_WINDOW_FRAMES * 2
    }

    /// Detach one completed window for analysis. The heavy alignment search
    /// never runs on the pumping thread: exclusive-mode buffers tolerate only
    /// a few milliseconds of loop stall.
    pub fn take_window(&mut self, now_ms: u64) -> Option<MusicJob> {
        if !self.window_ready() {
            return None;
        }
        let interleaved: Vec<f32> = self.pending.drain(..MUSIC_WINDOW_FRAMES * 2).collect();
        let job = MusicJob {
            start_ms: self.window_started_ms,
            captured_base: self.captured_base,
            previous_lag: self.lag_frames,
            acquired: self.alignment_acquired,
            mono: mono(&interleaved),
        };
        self.window_started_ms = now_ms;
        self.captured_base += MUSIC_WINDOW_FRAMES as u64;
        Some(job)
    }

    /// Fold one analyzed window into the bounded evidence. A missed
    /// correlation records a failed window; only the finish gates turn
    /// failures into a run failure so one reset reprime cannot discard
    /// hours of otherwise qualified endurance evidence.
    pub fn fold_window(&mut self, window: MusicWindow) {
        self.maximum_abs_lag_frames = self.maximum_abs_lag_frames.max(window.lag_frames.abs());
        if window.qualified {
            let step = if self.alignment_acquired {
                (window.lag_frames - self.lag_frames).abs()
            } else {
                0
            };
            self.maximum_lag_step_frames = self.maximum_lag_step_frames.max(step);
            self.lag_frames = window.lag_frames;
            self.alignment_acquired = true;
            self.minimum_correlation = self.minimum_correlation.min(window.correlation);
            self.windows_qualified += 1;
            self.failed_streak = 0;
        } else {
            self.windows_failed += 1;
            self.failed_streak += 1;
            self.longest_failed_streak = self.longest_failed_streak.max(self.failed_streak);
        }
        if window.both_quiet {
            self.windows_both_quiet += 1;
        }
        self.windows_total += 1;
        self.material_loops = self.captured_frames.div_ceil(self.material_frames as u64);
        self.recent_windows.push(window);
        if self.recent_windows.len() > RECENT_WINDOWS {
            self.recent_windows.remove(0);
        }
    }

    /// Synchronous qualification used by tests; production routes the same
    /// analysis through the analyzer thread.
    #[cfg(test)]
    pub fn qualify_window(
        &mut self,
        material_mono: &[f32],
        material_decimated: &[f32],
        now_ms: u64,
    ) {
        if let Some(job) = self.take_window(now_ms) {
            let window = analyze_window(material_mono, material_decimated, job);
            self.fold_window(window);
        }
    }

    pub fn end_observed(&self) -> bool {
        self.last_packet_ms
            .is_some_and(|last| last >= self.requested_ms)
    }

    pub fn finish(&mut self, now_ms: u64) -> anyhow::Result<()> {
        ensure!(
            !self.quality_failed,
            "music soak has a failed quality window"
        );
        self.require_fresh(now_ms)?;
        ensure!(
            now_ms >= self.requested_ms,
            "music soak duration incomplete"
        );
        ensure!(
            self.end_observed(),
            "music soak lacks a capture packet at/after requested duration"
        );
        let maximum_initial_edge_ms = self
            .first_packet_frames
            .unwrap_or(MAX_PACKET_FRAMES)
            .saturating_mul(1_000)
            .div_ceil(SOAK_SAMPLE_RATE)
            .saturating_add(BOUNDARY_SCHEDULING_SLACK_MS);
        let maximum_final_edge_ms = self
            .last_packet_frames
            .saturating_mul(1_000)
            .div_ceil(SOAK_SAMPLE_RATE)
            .saturating_add(BOUNDARY_SCHEDULING_SLACK_MS);
        ensure!(
            self.first_packet_ms.unwrap_or(u64::MAX) <= maximum_initial_edge_ms,
            "music soak initial capture boundary was late: first packet at {}ms, limit {maximum_initial_edge_ms}ms",
            self.first_packet_ms.unwrap_or(u64::MAX)
        );
        let last_packet_ms = self.last_packet_ms.unwrap_or(u64::MAX);
        ensure!(
            last_packet_ms.saturating_sub(self.requested_ms) <= maximum_final_edge_ms,
            "music soak final capture boundary was late: last packet at {last_packet_ms}ms, requested {}ms, limit {maximum_final_edge_ms}ms",
            self.requested_ms
        );
        ensure!(
            self.windows_total > 0,
            "music soak qualified no correlation window"
        );
        let ratio = self.windows_qualified as f64 / self.windows_total as f64;
        ensure!(
            ratio >= MUSIC_QUALIFIED_RATIO,
            "music soak qualified-window ratio {ratio:.4} below {MUSIC_QUALIFIED_RATIO}: qualified={} of {}",
            self.windows_qualified,
            self.windows_total
        );
        ensure!(
            self.longest_failed_streak <= MUSIC_MAX_FAILED_STREAK,
            "music soak failed-window streak {} exceeds {MUSIC_MAX_FAILED_STREAK}",
            self.longest_failed_streak
        );
        let requested_frames = u128::from(self.requested_ms) * u128::from(SOAK_SAMPLE_RATE) / 1_000;
        let settling_frames = u128::from(self.settling_ms) * u128::from(SOAK_SAMPLE_RATE) / 1_000;
        let eligible = requested_frames.saturating_sub(settling_frames).max(1);
        let required = u64::try_from(eligible).context("music required frames overflow")?;
        self.coverage_required_frames = Some(required);
        self.coverage_ratio = Some(self.captured_frames as f64 / eligible as f64);
        ensure!(
            self.captured_frames >= required,
            "music soak frame coverage too low: captured={}, required={}",
            self.captured_frames,
            required
        );
        self.completed = true;
        Ok(())
    }
}

pub(super) fn mono(samples: &[f32]) -> Vec<f32> {
    samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| (frame[0] + frame[1]) * 0.5)
        .collect()
}

/// Raw dot score of a captured sub-window against the looping mono material
/// at a candidate lag. The running index avoids per-sample division.
fn dot_score(captured: &[f32], material_mono: &[f32], captured_base: u64, lag: i64) -> f32 {
    let frames = material_mono.len() as i64;
    let mut position = (captured_base as i64 + lag).rem_euclid(frames) as usize;
    let mut score = 0.0f32;
    for &sample in captured {
        score += sample * material_mono[position];
        position += 1;
        if position == frames as usize {
            position = 0;
        }
    }
    score
}

/// Box-average decimation: one output sample per `factor` input samples.
pub(super) fn decimate(samples: &[f32], factor: usize) -> Vec<f32> {
    let mut output = Vec::with_capacity(samples.len().div_ceil(factor));
    for bucket in samples.chunks(factor) {
        let sum: f32 = bucket.iter().sum();
        output.push(sum / bucket.len() as f32);
    }
    output
}

/// Dot score in the decimated domain. Both bases are whole grid blocks, so a
/// full-frame grid lag maps to an integer decimated offset.
fn decimated_dot(
    captured: &[f32],
    material: &[f32],
    captured_base_blocks: i64,
    lag_blocks: i64,
) -> f32 {
    let blocks = material.len() as i64;
    let mut position = (captured_base_blocks + lag_blocks).rem_euclid(blocks) as usize;
    let mut score = 0.0f32;
    for &sample in captured {
        score += sample * material[position];
        position += 1;
        if position == blocks as usize {
            position = 0;
        }
    }
    score
}

/// Alignment search for one capture window. The coarse stage scans grid
/// lags in the box-decimated domain, where the correlation peak is wide
/// enough that a grid point always lands inside it; exact refinement then
/// walks every full-rate frame around the best distinct candidates, and the
/// full-window normalized correlation gates the result. Loop-equivalent
/// candidates are deduplicated by residue, visiting outward from the search
/// center so the smallest magnitude wins exact ties.
pub(super) fn analyze_window(
    material_mono: &[f32],
    material_decimated: &[f32],
    job: MusicJob,
) -> MusicWindow {
    let frames = material_mono.len() as i64;
    let captured = &job.mono[..job.mono.len().min(MUSIC_WINDOW_FRAMES)];
    let mut window = MusicWindow {
        start_ms: job.start_ms,
        lag_frames: job.previous_lag,
        correlation: 0.0,
        both_quiet: false,
        qualified: false,
    };
    let stride = MUSIC_GRID_STRIDE as i64;
    let (low, high) = if job.acquired {
        (
            job.previous_lag - MUSIC_TRACK_LAG_FRAMES,
            job.previous_lag + MUSIC_TRACK_LAG_FRAMES,
        )
    } else {
        (-MUSIC_ACQUIRE_LAG_FRAMES, MUSIC_ACQUIRE_LAG_FRAMES)
    };
    let low = low - low.rem_euclid(stride);
    let high = high - high.rem_euclid(stride);
    let center = if job.acquired { job.previous_lag } else { 0 };
    let captured_decimated = decimate(captured, MUSIC_GRID_STRIDE);
    let base_blocks = (job.captured_base / MUSIC_GRID_STRIDE as u64) as i64;
    let mut candidates: Vec<(i64, f32)> = Vec::new();
    let mut radius = 0;
    while radius <= (high - center).abs().max((center - low).abs()) {
        for lag in [center + radius, center - radius] {
            if (low..=high).contains(&lag) {
                let score = decimated_dot(
                    &captured_decimated,
                    material_decimated,
                    base_blocks,
                    lag / stride,
                );
                candidates.push((lag, score));
            }
        }
        radius += stride;
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut distinct: Vec<i64> = Vec::with_capacity(MUSIC_COARSE_CANDIDATES);
    for (lag, _) in &candidates {
        let residue = lag.rem_euclid(frames);
        if distinct
            .iter()
            .all(|kept| kept.rem_euclid(frames) != residue)
        {
            distinct.push(*lag);
        }
        if distinct.len() >= MUSIC_COARSE_CANDIDATES {
            break;
        }
    }
    let refine = &captured[..captured.len().min(MUSIC_REFINE_SUBWINDOW)];
    let mut best_lag = distinct.first().copied().unwrap_or(job.previous_lag);
    let mut best_dot = f32::NEG_INFINITY;
    for &candidate in &distinct {
        for lag in (candidate - stride)..=(candidate + stride) {
            let score = dot_score(refine, material_mono, job.captured_base, lag);
            if score > best_dot {
                best_dot = score;
                best_lag = lag;
            }
        }
    }
    let mut dot = 0.0f32;
    let mut captured_energy = 0.0f32;
    let mut expected_energy = 0.0f32;
    let mut position = (job.captured_base as i64 + best_lag).rem_euclid(frames) as usize;
    for &sample in captured {
        let expected = material_mono[position];
        dot += sample * expected;
        captured_energy += sample * sample;
        expected_energy += expected * expected;
        position += 1;
        if position == frames as usize {
            position = 0;
        }
    }
    window.lag_frames = best_lag;
    window.both_quiet =
        captured_energy < MUSIC_QUIET_ENERGY && expected_energy < MUSIC_QUIET_ENERGY;
    window.correlation = if window.both_quiet {
        1.0
    } else if captured_energy < MUSIC_QUIET_ENERGY || expected_energy < MUSIC_QUIET_ENERGY {
        0.0
    } else {
        dot / (captured_energy * expected_energy).sqrt().max(f32::EPSILON)
    };
    window.qualified = window.both_quiet || window.correlation >= MUSIC_MIN_CORRELATION;
    window
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
    pub rate_tolerance_ppm: u64,
    pub rate_samples: u64,
    pub measured_rate_units_per_second: Option<f64>,
    pub measured_rate_deviation_ppm: Option<f64>,
    pub maximum_abs_rate_deviation_ppm: f64,
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
        self.rate_tolerance_ppm = MAX_CLOCK_RATE_DEVIATION_PPM;
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
            if let (Some(previous_position), Some(previous_qpc)) =
                (self.last_position, self.last_qpc_100ns)
                && let (Some(position_delta), Some(qpc_delta)) = (
                    position.checked_sub(previous_position),
                    qpc.checked_sub(previous_qpc),
                )
                && qpc_delta >= MIN_CLOCK_RATE_INTERVAL_100NS
            {
                let measured_rate = position_delta as f64 * 10_000_000.0 / qpc_delta as f64;
                let deviation_ppm = (measured_rate / self.frequency as f64 - 1.0) * 1_000_000.0;
                self.rate_samples += 1;
                self.measured_rate_units_per_second = Some(measured_rate);
                self.measured_rate_deviation_ppm = Some(deviation_ppm);
                self.maximum_abs_rate_deviation_ppm =
                    self.maximum_abs_rate_deviation_ppm.max(deviation_ppm.abs());
                ensure!(
                    deviation_ppm.abs() <= MAX_CLOCK_RATE_DEVIATION_PPM as f64,
                    "native audio clock rate differs from QPC by {deviation_ppm:.0}ppm (limit {MAX_CLOCK_RATE_DEVIATION_PPM}ppm)"
                );
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
            self.forward_advances >= 1 && self.rate_samples >= 1 && self.frequency > 0,
            "native audio clock lacks accurate advancing rate evidence"
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
        assert_eq!(pcm.coverage_required_frames, Some(96_000));
        assert!((pcm.coverage_ratio.unwrap() - 1.005).abs() < 0.0001);
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
        pcm.observe(2_510, &EXPECTED.repeat(480), false).unwrap();
        idle.finish().unwrap();
        assert!(pcm.finish(2_510).is_err());
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
        assert!((clock.measured_rate_units_per_second.unwrap() - 48_000.0).abs() < 0.1);
        assert!(clock.maximum_abs_rate_deviation_ppm < 1.0);
    }

    #[test]
    fn pcm_soak_rejects_tenfold_slow_capture_even_when_wall_time_elapses() {
        let duration_ms = 10_000;
        let mut pcm = PcmSoak::new(duration_ms, EXPECTED);
        for now in (0..=duration_ms).step_by(10) {
            // One tenth of nominal 48kHz coverage, while packet timestamps
            // still span the complete requested wall-clock interval.
            pcm.observe(now, &EXPECTED.repeat(48), false).unwrap();
        }
        let error = pcm.finish(duration_ms).unwrap_err().to_string();
        assert!(
            error.contains("validated PCM frame coverage too low"),
            "{error}"
        );
        assert_eq!(pcm.captured_frames, 48 * 1_001);
        assert_eq!(pcm.coverage_required_frames, Some(480_000));
        assert!((pcm.coverage_ratio.unwrap() - 0.1001).abs() < 0.0001);
        assert!(!pcm.completed);
    }

    #[test]
    fn pcm_coverage_excludes_explicit_settling_without_double_crediting_edges() {
        let mut pcm = PcmSoak::new(3_000, EXPECTED);
        for now in (0..=1_000).step_by(10) {
            pcm.observe(now, &EXPECTED.repeat(480), false).unwrap();
        }
        pcm.change_pattern(1_000, SECOND_EXPECTED).unwrap();
        for now in (1_010..=3_000).step_by(10) {
            pcm.observe(now, &SECOND_EXPECTED.repeat(480), false)
                .unwrap();
        }
        pcm.finish(3_000).unwrap();
        assert_eq!(pcm.settling_frames, 47_520);
        assert_eq!(pcm.settling_duration_ms, 1_000);
        assert_eq!(pcm.coverage_required_frames, Some(96_000));
        assert!(pcm.validated.total_frames >= pcm.coverage_required_frames.unwrap());
        assert_eq!(
            pcm.coverage_ratio,
            Some(pcm.validated.total_frames as f64 / 96_000.0)
        );
    }

    #[test]
    fn pcm_packet_edges_cannot_make_up_for_missing_in_window_coverage() {
        let mut pcm = PcmSoak::new(2_000, EXPECTED);
        let maximum_packet = EXPECTED.repeat(MAX_PACKET_FRAMES as usize);
        for index in 0..8 {
            let now = index * 2_000 / 7;
            pcm.observe(now, &maximum_packet, false).unwrap();
        }

        let error = pcm.finish(2_000).unwrap_err().to_string();
        assert!(
            error.contains("validated PCM frame coverage too low"),
            "{error}"
        );
        assert_eq!(pcm.validated.total_frames, 8 * MAX_PACKET_FRAMES);
        assert_eq!(pcm.coverage_required_frames, Some(96_000));
        assert!((pcm.coverage_ratio.unwrap() - 0.8).abs() < 0.0001);
        assert!(!pcm.completed);
    }

    #[test]
    fn pcm_coverage_does_not_let_packet_edges_hide_long_missing_intervals() {
        let mut late_start = PcmSoak::new(2_000, EXPECTED);
        late_start
            .observe(500, &EXPECTED.repeat(4_800), false)
            .unwrap();
        late_start
            .observe(2_000, &EXPECTED.repeat(4_800), false)
            .unwrap();
        assert!(late_start.finish(2_000).is_err());

        let mut late_end = PcmSoak::new(2_000, EXPECTED);
        late_end.observe(0, &EXPECTED.repeat(4_800), false).unwrap();
        late_end
            .observe(1_500, &EXPECTED.repeat(4_800), false)
            .unwrap();
        late_end
            .observe(2_500, &EXPECTED.repeat(4_800), false)
            .unwrap();
        assert!(late_end.finish(2_500).is_err());
    }

    #[test]
    fn clock_rate_is_measured_against_qpc_and_rejects_tenfold_slowdown() {
        let mut normal = ClockProgress {
            frequency: 48_000,
            ..Default::default()
        };
        normal.observe(0, 100_000_000, true, 0).unwrap();
        normal.observe(48_000, 110_000_000, true, 1_000).unwrap();
        normal.require_fresh(1_000).unwrap();
        assert_eq!(normal.rate_samples, 1);
        assert_eq!(normal.measured_rate_units_per_second, Some(48_000.0));
        assert_eq!(normal.measured_rate_deviation_ppm, Some(0.0));

        let mut slow = ClockProgress {
            frequency: 48_000,
            ..Default::default()
        };
        slow.observe(0, 100_000_000, true, 0).unwrap();
        let error = slow
            .observe(4_800, 110_000_000, true, 1_000)
            .unwrap_err()
            .to_string();
        assert!(error.contains("differs from QPC by -900000ppm"), "{error}");
        assert_eq!(slow.rate_samples, 1);
        assert_eq!(slow.measured_rate_units_per_second, Some(4_800.0));
        assert_eq!(slow.measured_rate_deviation_ppm, Some(-900_000.0));
        assert_eq!(slow.rate_tolerance_ppm, 50_000);
        assert!(slow.maximum_abs_rate_deviation_ppm >= 900_000.0);
    }

    #[test]
    fn clock_requires_a_long_enough_qpc_interval_for_rate_evidence() {
        let mut clock = ClockProgress {
            frequency: 48_000,
            ..Default::default()
        };
        clock.observe(0, 100_000_000, true, 0).unwrap();
        clock.observe(480, 100_100_000, true, 10).unwrap();
        assert_eq!(clock.forward_advances, 1);
        assert_eq!(clock.rate_samples, 0);
        assert!(clock.require_fresh(10).is_err());
    }
}

#[cfg(test)]
mod music_tests {
    use super::*;
    use crate::material::Material;

    /// Deterministic, band-limited "music": box-smoothed xorshift noise has a
    /// near-delta autocorrelation, unlike pure tones whose periodic sidelobes
    /// can beat the main peak on short sub-windows.
    fn test_material(frames: usize) -> Material {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut raw = Vec::with_capacity(frames * 2);
        for _ in 0..frames * 2 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            raw.push((state >> 33) as f32 / 2_147_483_648.0 * 0.6 - 0.3);
        }
        let mut samples = vec![0.0f32; frames * 2];
        for index in 8..raw.len() {
            samples[index] = raw[index - 8..=index].iter().sum::<f32>() / 9.0;
        }
        Material { samples, frames }
    }

    fn analysis(material: &Material) -> (Vec<f32>, Vec<f32>) {
        let material_mono = mono(&material.samples);
        let material_decimated = decimate(&material_mono, MUSIC_GRID_STRIDE);
        (material_mono, material_decimated)
    }

    /// Feed the soak looped material delayed by `lag` frames, like a pipeline.
    fn feed_windows(
        soak: &mut MusicSoak,
        material: &Material,
        windows: usize,
        lag: i64,
        now_ms: &mut u64,
        delay_drift: i64,
    ) {
        let (material_mono, material_decimated) = analysis(material);
        let mut lag = lag;
        for window in 0..windows {
            let mut samples = Vec::with_capacity(MUSIC_WINDOW_FRAMES * 2);
            for frame in 0..MUSIC_WINDOW_FRAMES {
                let position = (window as i64 * MUSIC_WINDOW_FRAMES as i64 + lag + frame as i64)
                    .rem_euclid(material.frames as i64) as usize;
                samples.push(material.samples[position * 2]);
                samples.push(material.samples[position * 2 + 1]);
            }
            lag += delay_drift;
            // Ten 4800-frame packets per window keep bookkeeping realistic.
            for packet in samples.as_chunks::<{ MUSIC_WINDOW_FRAMES * 2 / 10 }>().0 {
                *now_ms += 100;
                soak.observe_packet(*now_ms, packet, false).unwrap();
            }
            soak.qualify_window(&material_mono, &material_decimated, *now_ms);
        }
    }

    /// Move past the fixed two-second settling window with finite packets.
    /// The last packet lands strictly before the boundary so nothing leaks
    /// into the first correlation window.
    fn settle(soak: &mut MusicSoak, material: &Material, now_ms: &mut u64) {
        while *now_ms < 1_950 {
            *now_ms += 50;
            soak.observe_packet(*now_ms, &material.samples[..960], false)
                .unwrap();
        }
    }

    fn observe_silence(soak: &mut MusicSoak, now_ms: &mut u64) {
        let silence = vec![0.0f32; MUSIC_WINDOW_FRAMES * 2];
        for packet in silence.as_chunks::<9_600>().0 {
            *now_ms += 100;
            soak.observe_packet(*now_ms, packet, false).unwrap();
        }
    }

    #[test]
    fn music_soak_acquires_alignment_qualifies_windows_and_tracks_drift() {
        let material = test_material(48_000);
        let mut soak = MusicSoak::new(3_000, &material);
        let mut now_ms = 0;
        settle(&mut soak, &material, &mut now_ms);
        // The pipeline delays capture by 9_500 frames (inside +-4s acquire).
        feed_windows(&mut soak, &material, 2, 9_500, &mut now_ms, 0);
        assert!(soak.alignment_acquired);
        assert_eq!(soak.windows_total, 2);
        assert_eq!(soak.windows_qualified, 2);
        assert_eq!(soak.lag_frames, 9_500);
        // Clock drift moves the alignment 3 frames per second; the tracker
        // follows within its bounded window search.
        feed_windows(&mut soak, &material, 2, 9_506, &mut now_ms, 3);
        assert_eq!(soak.windows_qualified, 4);
        assert_eq!(soak.lag_frames, 9_509);
        assert_eq!(soak.maximum_lag_step_frames, 6);
        assert!(soak.recent_windows.iter().all(|window| window.qualified));
        assert!(serde_json::to_vec(&soak).unwrap().len() < 4_096);
    }

    #[test]
    fn music_soak_fails_inserted_silence_but_not_material_quiet_passages() {
        let material = test_material(48_000);
        let mut soak = MusicSoak::new(2_000, &material);
        let mut now_ms = 0;
        settle(&mut soak, &material, &mut now_ms);
        feed_windows(&mut soak, &material, 1, 0, &mut now_ms, 0);
        assert_eq!(soak.windows_qualified, 1);
        // One window of inserted silence is recorded as failed, not fatal.
        observe_silence(&mut soak, &mut now_ms);
        soak.qualify_window(&analysis(&material).0, &analysis(&material).1, now_ms);
        assert_eq!(soak.windows_failed, 1);
        assert_eq!(soak.longest_failed_streak, 1);
        assert!(!soak.quality_failed, "failed windows gate at finish only");
        // A genuinely quiet material stretch on both sides qualifies.
        let quiet = Material {
            samples: vec![0.0; 96_000],
            frames: 48_000,
        };
        let mut quiet_soak = MusicSoak::new(2_000, &quiet);
        let mut quiet_ms = 0;
        settle(&mut quiet_soak, &quiet, &mut quiet_ms);
        feed_windows(&mut quiet_soak, &quiet, 2, 0, &mut quiet_ms, 0);
        assert_eq!(quiet_soak.windows_both_quiet, 2);
        assert_eq!(quiet_soak.windows_qualified, 2);
    }

    #[test]
    fn music_finish_enforces_ratio_streak_and_coverage() {
        let material = test_material(48_000);
        // 400 windows (~405s) so four trailing failures keep the ratio at or
        // above 0.99 while the requested duration matches the fed span; only
        // the consecutive-failure streak may then reject the finish.
        let mut soak = MusicSoak::new(406_000, &material);
        let mut now_ms = 0;
        settle(&mut soak, &material, &mut now_ms);
        feed_windows(&mut soak, &material, 400, 0, &mut now_ms, 0);
        assert_eq!(soak.windows_qualified, 400);
        let (material_mono, material_decimated) = analysis(&material);
        for _ in 0..4 {
            observe_silence(&mut soak, &mut now_ms);
            soak.qualify_window(&material_mono, &material_decimated, now_ms);
        }
        assert_eq!(soak.longest_failed_streak, 4);
        now_ms += 100;
        soak.observe_packet(now_ms, &material.samples[..960], false)
            .unwrap();
        let error = soak.finish(now_ms).unwrap_err().to_string();
        assert!(error.contains("streak"), "{error}");
        assert!(!soak.completed);
    }

    #[test]
    fn music_observation_keeps_freshness_and_discontinuity_gates() {
        let material = test_material(48_000);
        let mut soak = MusicSoak::new(2_000, &material);
        let mut now_ms = 0;
        soak.observe_packet(now_ms, &material.samples[..960], false)
            .unwrap();
        now_ms += 2_001;
        assert!(
            soak.observe_packet(now_ms, &material.samples[..960], false)
                .is_err()
        );
        let mut discontinuous = MusicSoak::new(2_000, &material);
        assert!(
            discontinuous
                .observe_packet(0, &material.samples[..960], true)
                .is_err()
        );
        assert!(discontinuous.quality_failed);
    }
}
