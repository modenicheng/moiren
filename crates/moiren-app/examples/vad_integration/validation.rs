//! Pure evidence checks: wall time and silence never stand in for Engine/PCM progress.
use anyhow::{bail, ensure};
use moiren_engine::meter::signal::AudioWindow;
use moiren_windows_audio::vad::{Intent, StableEndpointId};
use serde::Serialize;

pub(super) const SIGNAL: [f32; 2] = [0.25, -0.5];
pub(super) const EXPECTED: [f32; 2] = [0.125, -0.25];
pub(super) const SECOND_SIGNAL: [f32; 2] = [-0.375, 0.125];
pub(super) const SECOND_EXPECTED: [f32; 2] = [-0.1875, 0.0625];
pub(super) const REQUIRED_FRAMES: u64 = 48_000;
pub(super) const TOLERANCE: f32 = 0.0001;
const READMISSION_LIMIT_MS: u128 = 5_000;
const GATE_EVENT_CAPACITY: usize = 32;

#[derive(Clone, Debug, Serialize)]
pub(super) struct GateEvent {
    pub observed_ms: u128,
    pub event: &'static str,
    pub native_stamp: [u64; 5],
    pub gate_open: bool,
    pub closed_duration_ms: Option<u128>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct Readmission {
    pub observed_ms: u128,
    pub deadline_ms: u128,
    pub closed_since_ms: Option<u128>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct GateProgress {
    pub endpoint_id: String,
    pub native_identity: [u64; 3],
    pub settled_stream_generation: u64,
    pub last_stamp: [u64; 5],
    pub last_gate_open: bool,
    pub last_observed_ms: u128,
    pub pending: Option<Readmission>,
    pub closures: u64,
    pub reopens: u64,
    pub total_closed_ms: u128,
    pub longest_closed_ms: u128,
    pub events: Vec<GateEvent>,
    pub omitted_events: u64,
}

impl GateProgress {
    pub fn new(endpoint_id: String, stamp: Option<[u64; 5]>, now_ms: u128) -> anyhow::Result<Self> {
        let stamp =
            stamp.ok_or_else(|| anyhow::anyhow!("{endpoint_id}: missing initial native stamp"))?;
        let mut progress = Self {
            endpoint_id,
            native_identity: [stamp[0], stamp[1], stamp[2]],
            settled_stream_generation: stamp[3],
            last_stamp: stamp,
            last_gate_open: true,
            last_observed_ms: now_ms,
            pending: None,
            closures: 0,
            reopens: 0,
            total_closed_ms: 0,
            longest_closed_ms: 0,
            events: Vec::new(),
            omitted_events: 0,
        };
        progress.event("initial_open_baseline", stamp, true, now_ms, None);
        Ok(progress)
    }

    pub fn observe(
        &mut self,
        stamp: Option<[u64; 5]>,
        open: bool,
        now_ms: u128,
        phase_deadline_ms: u128,
    ) -> anyhow::Result<()> {
        let stamp =
            stamp.ok_or_else(|| anyhow::anyhow!("{}: missing native stamp", self.endpoint_id))?;
        ensure!(
            [stamp[0], stamp[1], stamp[2]] == self.native_identity,
            "{}: native identity changed: previous={:?}, current={stamp:?}",
            self.endpoint_id,
            self.last_stamp
        );
        ensure!(
            stamp[3] >= self.last_stamp[3] && stamp[4] >= self.last_stamp[4],
            "{}: native stamp regressed: previous={:?}, current={stamp:?}",
            self.endpoint_id,
            self.last_stamp
        );
        ensure!(
            now_ms >= self.last_observed_ms && now_ms < phase_deadline_ms,
            "{}: observation outside phase deadline",
            self.endpoint_id
        );
        if let Some(pending) = &mut self.pending {
            pending.deadline_ms = pending.deadline_ms.min(phase_deadline_ms);
        }
        if let Some(pending) = &self.pending
            && now_ms >= pending.deadline_ms
        {
            ensure!(
                pending.closed_since_ms.is_none() && self.last_gate_open,
                "{}: native gate re-admission timed out: pending={pending:?}, stamp={stamp:?}",
                self.endpoint_id
            );
            // An advance observed while open is not proof of re-admission.
            // Retain its correlation window, then expire it without inventing
            // a close/reopen event that the sampled App API did not observe.
            // A newly observed higher generation may already be closed; it gets
            // its own window after the old, never-closed generation is settled.
            self.settled_stream_generation = self.last_stamp[3];
            self.pending = None;
            self.event(
                "advance_without_observed_closure",
                self.last_stamp,
                true,
                now_ms,
                None,
            );
        }
        if stamp[3] > self.settled_stream_generation && self.pending.is_none() {
            self.pending = Some(Readmission {
                observed_ms: now_ms,
                deadline_ms: (now_ms + READMISSION_LIMIT_MS).min(phase_deadline_ms),
                closed_since_ms: None,
            });
        }
        if stamp[3] > self.last_stamp[3] {
            self.event("stream_generation_advanced", stamp, open, now_ms, None);
        }
        if !open {
            ensure!(
                stamp[3] > self.settled_stream_generation && self.pending.is_some(),
                "{}: unexplained same-stream gate closure: settled_stream={}, stamp={stamp:?}",
                self.endpoint_id,
                self.settled_stream_generation
            );
            if self.last_gate_open {
                self.pending
                    .as_mut()
                    .expect("checked transition")
                    .closed_since_ms = Some(now_ms);
                self.closures += 1;
                self.event(
                    "gate_closed_after_stream_advance",
                    stamp,
                    false,
                    now_ms,
                    None,
                );
            }
        } else if !self.last_gate_open {
            let pending = self.pending.take().expect("closed gate has a transition");
            let duration = now_ms - pending.closed_since_ms.expect("observed closure");
            self.reopens += 1;
            self.total_closed_ms += duration;
            self.longest_closed_ms = self.longest_closed_ms.max(duration);
            self.settled_stream_generation = stamp[3];
            self.event("gate_reopened", stamp, true, now_ms, Some(duration));
        }
        self.last_stamp = stamp;
        self.last_gate_open = open;
        self.last_observed_ms = now_ms;
        Ok(())
    }

    pub fn require_deadline(&self, now_ms: u128) -> anyhow::Result<()> {
        if let Some(pending) = &self.pending
            && pending.closed_since_ms.is_some()
        {
            ensure!(
                now_ms < pending.deadline_ms,
                "{}: native gate re-admission timed out: {pending:?}",
                self.endpoint_id
            );
        }
        Ok(())
    }

    fn event(
        &mut self,
        event: &'static str,
        native_stamp: [u64; 5],
        gate_open: bool,
        observed_ms: u128,
        closed_duration_ms: Option<u128>,
    ) {
        if self.events.len() == GATE_EVENT_CAPACITY {
            self.events.remove(0);
            self.omitted_events += 1;
        }
        self.events.push(GateEvent {
            observed_ms,
            event,
            native_stamp,
            gate_open,
            closed_duration_ms,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub id: String,
    pub raw_instance: String,
    pub parent_instance: String,
    pub intent: Intent,
}

pub(super) fn select<'a>(
    candidates: &'a [Candidate],
    stable_id: StableEndpointId,
    intent: Intent,
    root: &str,
) -> anyhow::Result<Option<&'a Candidate>> {
    let stable = stable_id.to_string();
    let mut matches = candidates.iter().filter(|candidate| {
        let Some((hardware, instance)) = candidate.raw_instance.rsplit_once('\\') else {
            return false;
        };
        hardware.eq_ignore_ascii_case(r"MOIREN\VAD_RAW")
            && instance
                .rsplit('&')
                .next()
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&stable))
            && candidate.parent_instance.eq_ignore_ascii_case(root)
            && candidate.intent == intent
    });
    let first = matches.next();
    ensure!(
        matches.next().is_none(),
        "multiple MMDevices mapped to {stable_id}"
    );
    Ok(first)
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct TimelineProgress {
    pub first: Option<WindowEvidence>,
    pub last: Option<WindowEvidence>,
    pub forward_advances: u64,
    pub epoch_changes: u64,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct WindowEvidence {
    pub timeline_epoch: u64,
    pub frame_start: u64,
    pub frame_end: u64,
    pub sample_rate: f64,
}

impl TimelineProgress {
    pub fn observe(&mut self, header: AudioWindow) -> anyhow::Result<()> {
        ensure!(
            header.timeline_epoch != 0
                && header.frame_end > header.frame_start
                && header.sample_rate == 48_000.0,
            "invalid Engine signal header: {header:?}"
        );
        let next = WindowEvidence {
            timeline_epoch: header.timeline_epoch,
            frame_start: header.frame_start,
            frame_end: header.frame_end,
            sample_rate: header.sample_rate,
        };
        if let Some(last) = self.last {
            if last.timeline_epoch == next.timeline_epoch {
                ensure!(
                    next.frame_end >= last.frame_end,
                    "signal frame_end regressed"
                );
                if next.frame_end > last.frame_end {
                    self.forward_advances += 1;
                }
            } else {
                self.epoch_changes += 1;
                self.forward_advances = 0;
                self.first = Some(next);
            }
        }
        self.first.get_or_insert(next);
        self.last = Some(next);
        Ok(())
    }

    pub fn require_progress(&self) -> anyhow::Result<()> {
        ensure!(
            self.forward_advances >= 2,
            "need two actual signal frame_end advances: {self:?}"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Observation {
    pub total_frames: u64,
    pub matching_frames: u64,
    pub silent_frames: u64,
    pub unexpected_frames: u64,
    pub discontinuities: u64,
    pub consecutive_matching_frames: u64,
    pub longest_matching_run: u64,
    pub first_frame_ms: Option<u128>,
    pub last_frame_ms: Option<u128>,
}

impl Observation {
    pub fn observe_packet(
        &mut self,
        samples: &[f32],
        expected: [f32; 2],
        discontinuity: bool,
    ) -> anyhow::Result<()> {
        if discontinuity {
            self.discontinuities += 1;
            // The packet starts a fresh contiguous run even if every sample
            // matches. Preserve totals and the previous longest run as evidence.
            self.consecutive_matching_frames = 0;
        }
        self.observe(samples, expected)
    }

    pub fn observe(&mut self, samples: &[f32], expected: [f32; 2]) -> anyhow::Result<()> {
        ensure!(
            samples.len().is_multiple_of(2),
            "incomplete stereo capture frame"
        );
        for frame in samples.as_chunks::<2>().0 {
            self.total_frames += 1;
            if !frame.iter().all(|sample| sample.is_finite()) {
                self.unexpected_frames += 1;
                self.consecutive_matching_frames = 0;
                bail!("nonfinite capture PCM");
            }
            if frame.iter().all(|sample| sample.abs() <= TOLERANCE) {
                self.silent_frames += 1;
                self.consecutive_matching_frames = 0;
            } else {
                if !frame
                    .iter()
                    .zip(expected)
                    .all(|(actual, wanted)| (*actual - wanted).abs() <= TOLERANCE)
                {
                    self.unexpected_frames += 1;
                    self.consecutive_matching_frames = 0;
                    bail!("unexpected capture frame {frame:?}; expected {expected:?}");
                }
                self.matching_frames += 1;
                self.consecutive_matching_frames += 1;
                self.longest_matching_run = self
                    .longest_matching_run
                    .max(self.consecutive_matching_frames);
            }
        }
        Ok(())
    }

    pub fn require_signal(&self) -> anyhow::Result<()> {
        ensure!(
            self.longest_matching_run >= REQUIRED_FRAMES,
            "need {REQUIRED_FRAMES} consecutive matching signal frames: {self:?}"
        );
        Ok(())
    }

    pub fn require_silence(&self) -> anyhow::Result<()> {
        ensure!(
            self.matching_frames == 0 && self.silent_frames >= REQUIRED_FRAMES,
            "need {REQUIRED_FRAMES} actual silent capture frames: {self:?}"
        );
        Ok(())
    }
}

pub(super) fn start_clients(
    started: &mut u32,
    capture: impl FnOnce() -> anyhow::Result<()>,
    render: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    capture()?;
    *started += 1;
    render()?;
    *started += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(epoch: u64, end: u64) -> AudioWindow {
        AudioWindow {
            timeline_epoch: epoch,
            frame_start: end - 480,
            frame_end: end,
            sample_rate: 48_000.0,
        }
    }

    #[test]
    fn timeline_requires_two_distinct_frame_end_advances_not_repeated_latest() {
        let mut progress = TimelineProgress::default();
        progress.observe(header(1, 480)).unwrap();
        for _ in 0..10 {
            progress.observe(header(1, 480)).unwrap();
        }
        assert!(progress.require_progress().is_err());
        progress.observe(header(1, 960)).unwrap();
        assert!(progress.require_progress().is_err());
        progress.observe(header(1, 1440)).unwrap();
        progress.require_progress().unwrap();
        assert!(progress.observe(header(1, 960)).is_err());
    }

    #[test]
    fn epoch_changes_are_explicit_not_counted_as_frame_advances() {
        let mut progress = TimelineProgress::default();
        progress.observe(header(1, 960)).unwrap();
        progress.observe(header(2, 480)).unwrap();
        assert_eq!(progress.forward_advances, 0);
        assert_eq!(progress.epoch_changes, 1);
        assert!(progress.require_progress().is_err());
    }

    #[test]
    fn signal_requires_real_consecutive_nonzero_frames_and_rejects_bypass() {
        let mut progress = Observation::default();
        progress.observe(&[0.0; 960], EXPECTED).unwrap();
        assert!(progress.require_signal().is_err());
        progress
            .observe(&EXPECTED.repeat(REQUIRED_FRAMES as usize - 1), EXPECTED)
            .unwrap();
        assert!(progress.require_signal().is_err());
        progress.observe(&EXPECTED, EXPECTED).unwrap();
        progress.require_signal().unwrap();
        assert!(Observation::default().observe(&SIGNAL, EXPECTED).is_err());
        assert!(
            Observation::default()
                .observe(&[EXPECTED[1], EXPECTED[0]], EXPECTED)
                .is_err()
        );
        assert!(
            Observation::default()
                .observe(&[f32::NAN, 0.0], EXPECTED)
                .is_err()
        );
        assert!(Observation::default().observe(&[0.125], EXPECTED).is_err());
        assert!(
            Observation::default()
                .observe(&EXPECTED, SECOND_EXPECTED)
                .is_err()
        );
        assert_eq!(SECOND_SIGNAL.map(|v| v * 0.5), SECOND_EXPECTED);
    }

    #[test]
    fn stop_silence_needs_samples_not_absence_or_previous_signal() {
        let mut progress = Observation::default();
        assert!(progress.require_silence().is_err());
        progress
            .observe(&[0.0; 2].repeat(REQUIRED_FRAMES as usize), [0.0; 2])
            .unwrap();
        progress.require_silence().unwrap();
        assert!(progress.observe(&EXPECTED, [0.0; 2]).is_err());
    }

    #[test]
    fn identity_requires_exact_parent_stable_suffix_flow_and_uniqueness() {
        let stable = StableEndpointId::from_virtual_endpoint_id(
            moiren_engine::node::VirtualEndpointId::from_bytes(*b"MoirenAppVadPlay"),
        );
        let candidate = Candidate {
            id: "opaque-mmdevice-id".into(),
            raw_instance: format!(r"MOIREN\VAD_RAW\5&123&0&{stable}"),
            parent_instance: r"ROOT\MEDIA\0000".into(),
            intent: Intent::Playback,
        };
        let candidates = [candidate.clone()];
        assert_eq!(
            select(&candidates, stable, Intent::Playback, r"root\media\0000")
                .unwrap()
                .unwrap()
                .id,
            candidate.id
        );
        assert!(
            select(&candidates, stable, Intent::Microphone, r"ROOT\MEDIA\0000")
                .unwrap()
                .is_none()
        );
        assert!(
            select(&candidates, stable, Intent::Playback, r"ROOT\MEDIA\0001")
                .unwrap()
                .is_none()
        );
        assert!(
            select(
                &[candidate.clone(), candidate],
                stable,
                Intent::Playback,
                r"ROOT\MEDIA\0000"
            )
            .is_err()
        );
    }

    #[test]
    fn failed_signal_keeps_frame_counts_and_dropout_breaks_consecutive_run() {
        let mut observation = Observation::default();
        observation.observe(&EXPECTED.repeat(32), EXPECTED).unwrap();
        observation.observe(&[0.0, 0.0], EXPECTED).unwrap();
        observation.observe(&EXPECTED.repeat(16), EXPECTED).unwrap();
        assert_eq!(observation.longest_matching_run, 32);
        assert_eq!(observation.consecutive_matching_frames, 16);
        assert!(observation.observe(&SIGNAL, EXPECTED).is_err());
        let json = serde_json::to_value(observation).unwrap();
        assert_eq!(json["total_frames"], 50);
        assert_eq!(json["matching_frames"], 48);
        assert_eq!(json["silent_frames"], 1);
        assert_eq!(json["unexpected_frames"], 1);
    }

    #[test]
    fn capture_discontinuity_cannot_join_two_half_second_runs() {
        let half_second = EXPECTED.repeat(24_000);
        let mut observation = Observation::default();
        observation
            .observe_packet(&half_second, EXPECTED, false)
            .unwrap();
        observation
            .observe_packet(&half_second, EXPECTED, true)
            .unwrap();
        assert!(observation.require_signal().is_err());
        assert_eq!(observation.matching_frames, 48_000);
        assert_eq!(observation.longest_matching_run, 24_000);
        assert_eq!(observation.discontinuities, 1);
        observation
            .observe_packet(&half_second, EXPECTED, false)
            .unwrap();
        observation.require_signal().unwrap();
    }

    #[test]
    fn partial_native_start_keeps_capture_success_when_render_fails() {
        let mut started = 0;
        let calls = std::cell::RefCell::new(Vec::new());
        let result = start_clients(
            &mut started,
            || {
                calls.borrow_mut().push("capture");
                Ok(())
            },
            || {
                calls.borrow_mut().push("render");
                bail!("render Start failure")
            },
        );
        assert_eq!(*calls.borrow(), ["capture", "render"]);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("render Start failure")
        );
        assert_eq!(started, 1);
    }

    #[test]
    fn capture_start_failure_never_starts_render_or_reports_a_success() {
        let mut started = 0;
        let result = start_clients(
            &mut started,
            || bail!("capture Start failure"),
            || panic!("render must not start after capture failure"),
        );
        assert!(result.is_err());
        assert_eq!(started, 0);
        start_clients(&mut started, || Ok(()), || Ok(())).unwrap();
        assert_eq!(started, 2);
    }

    fn gate() -> GateProgress {
        GateProgress::new("playback".into(), Some([31, 42, 31, 0, 1]), 0).unwrap()
    }

    #[test]
    fn open_new_generation_retains_permission_for_a_later_observed_close() {
        let mut gate = gate();
        gate.observe(Some([31, 42, 31, 4, 9]), true, 100, 12_000)
            .unwrap();
        assert_eq!(gate.settled_stream_generation, 0);
        assert_eq!(gate.pending.as_ref().unwrap().deadline_ms, 5_100);
        gate.observe(Some([31, 42, 31, 4, 10]), false, 200, 12_000)
            .unwrap();
        gate.observe(Some([31, 42, 31, 4, 10]), true, 350, 12_000)
            .unwrap();
        assert_eq!(gate.closures, 1);
        assert_eq!(gate.reopens, 1);
        assert_eq!(gate.total_closed_ms, 150);
        assert_eq!(gate.settled_stream_generation, 4);
        assert!(gate.pending.is_none());
        assert!(
            gate.observe(Some([31, 42, 31, 4, 11]), false, 400, 12_000)
                .is_err()
        );
    }

    #[test]
    fn same_stream_gate_epoch_change_or_unknown_stamp_never_authorizes_closure() {
        assert!(
            gate()
                .observe(Some([31, 42, 31, 0, 9]), false, 100, 12_000)
                .is_err()
        );
        assert!(gate().observe(None, false, 100, 12_000).is_err());
        assert!(gate().observe(None, true, 100, 12_000).is_err());
        assert!(GateProgress::new("missing".into(), None, 0).is_err());
    }

    #[test]
    fn native_identity_and_regressing_stream_or_gate_stamps_fail_closed() {
        for stamp in [[32, 42, 31, 4, 9], [31, 43, 31, 4, 9], [31, 42, 32, 4, 9]] {
            assert!(gate().observe(Some(stamp), true, 100, 12_000).is_err());
        }
        let mut gate = gate();
        gate.observe(Some([31, 42, 31, 4, 9]), true, 100, 12_000)
            .unwrap();
        assert!(
            gate.observe(Some([31, 42, 31, 3, 10]), true, 200, 12_000)
                .is_err()
        );
        assert!(
            gate.observe(Some([31, 42, 31, 4, 8]), true, 200, 12_000)
                .is_err()
        );
    }

    #[test]
    fn further_generations_and_phase_changes_cannot_extend_readmission_deadline() {
        let mut gate = gate();
        gate.observe(Some([31, 42, 31, 4, 10]), false, 100, 4_000)
            .unwrap();
        gate.observe(Some([31, 42, 31, 5, 12]), false, 1_000, 12_000)
            .unwrap();
        assert_eq!(gate.pending.as_ref().unwrap().deadline_ms, 4_000);
        assert!(
            gate.observe(Some([31, 42, 31, 5, 12]), true, 4_000, 12_000)
                .is_err()
        );
    }

    #[test]
    fn unobserved_closure_permission_expires_without_claiming_readmission() {
        let mut gate = gate();
        gate.observe(Some([31, 42, 31, 4, 9]), true, 100, 12_000)
            .unwrap();
        gate.observe(Some([31, 42, 31, 4, 9]), true, 5_100, 12_000)
            .unwrap();
        assert!(gate.pending.is_none());
        assert_eq!(gate.reopens, 0);
        assert_eq!(gate.settled_stream_generation, 4);
        assert!(
            gate.observe(Some([31, 42, 31, 4, 10]), false, 5_200, 12_000)
                .is_err()
        );
    }

    #[test]
    fn newer_closed_generation_after_an_unclosed_window_expires_gets_its_own_deadline() {
        let mut gate = gate();
        gate.observe(Some([31, 42, 31, 1, 3]), true, 100, 12_000)
            .unwrap();
        gate.observe(Some([31, 42, 31, 2, 6]), false, 5_200, 12_000)
            .unwrap();
        assert_eq!(gate.settled_stream_generation, 1);
        assert_eq!(gate.pending.as_ref().unwrap().deadline_ms, 10_200);
        assert_eq!(gate.pending.as_ref().unwrap().closed_since_ms, Some(5_200));
        gate.observe(Some([31, 42, 31, 2, 6]), true, 5_300, 12_000)
            .unwrap();
        assert_eq!(gate.reopens, 1);
        assert_eq!(gate.total_closed_ms, 100);
    }

    #[test]
    fn closed_readmission_deadline_is_checked_without_another_snapshot_or_pcm() {
        let mut gate = gate();
        gate.observe(Some([31, 42, 31, 1, 4]), false, 100, 12_000)
            .unwrap();
        gate.require_deadline(5_099).unwrap();
        assert!(gate.require_deadline(5_100).is_err());
        assert!(
            gate.observe(Some([31, 42, 31, 2, 6]), false, 5_200, 12_000)
                .is_err()
        );
    }
}
