use moiren_windows_audio::vad::{Channels, Intent, PcmPacketStamp, StableEndpointId};
use serde::Serialize;

#[derive(Clone, Debug)]
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
    let stable_text = stable_id.to_string();
    let mut matches = candidates.iter().filter(|candidate| {
        let Some((hardware_id, instance)) = candidate.raw_instance.rsplit_once('\\') else {
            return false;
        };
        hardware_id.eq_ignore_ascii_case(r"MOIREN\VAD_RAW")
            && instance
                .rsplit('&')
                .next()
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&stable_text))
            && candidate.parent_instance.eq_ignore_ascii_case(root)
            && candidate.intent == intent
    });
    let first = matches.next();
    anyhow::ensure!(
        matches.next().is_none(),
        "multiple MMDevices mapped to owned raw endpoint {stable_id}"
    );
    Ok(first)
}

pub(super) fn pattern(index: usize, channels: Channels) -> [f32; 2] {
    [
        (index + 1) as f32 / 16.0,
        if channels == Channels::Stereo {
            -((index + 5) as f32) / 32.0
        } else {
            0.0
        },
    ]
}

#[derive(Default)]
pub(super) struct PacketProgress(Option<PcmPacketStamp>);

impl PacketProgress {
    pub fn observe(&mut self, stamp: PcmPacketStamp) -> anyhow::Result<()> {
        if let Some(previous) = self.0 {
            anyhow::ensure!(
                stamp.sequence > previous.sequence && stamp.qpc >= previous.qpc,
                "replayed or regressing PCM stamp: {stamp:?}, previous={previous:?}"
            );
        }
        self.0 = Some(stamp);
        Ok(())
    }
}

pub(super) fn timer_render_frames(capacity: u32, padding: u32) -> anyhow::Result<u32> {
    anyhow::ensure!(
        capacity > 0 && padding <= capacity,
        "invalid native padding"
    );
    Ok(capacity - padding)
}

#[derive(Clone, Default, Debug, Serialize)]
pub(super) struct PositionProgress {
    first: Option<PositionSample>,
    last: Option<PositionSample>,
    pub frequency: u64,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct PositionSample {
    position_units: u64,
    qpc_100ns: u64,
}

impl PositionProgress {
    pub fn begin_phase(&mut self) {
        // Keep the last sample for regression checks, but require fresh phase
        // evidence; closed-gate motion cannot qualify the later signal phase.
        self.first = None;
    }

    pub fn observe(&mut self, position: u64, qpc: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.frequency > 0 && qpc != 0,
            "invalid native clock sample"
        );
        if let Some(previous) = self.last {
            anyhow::ensure!(
                position >= previous.position_units && qpc >= previous.qpc_100ns,
                "native clock regressed: position={position}, qpc={qpc}, previous={:?}",
                self.last
            );
        }
        let sample = PositionSample {
            position_units: position,
            qpc_100ns: qpc,
        };
        self.first.get_or_insert(sample);
        self.last = Some(sample);
        Ok(())
    }

    pub fn require_advance(&self) -> anyhow::Result<()> {
        let (Some(first), Some(last)) = (self.first, self.last) else {
            anyhow::bail!("no native position evidence");
        };
        anyhow::ensure!(
            self.frequency > 0
                && last.position_units > first.position_units
                && last.qpc_100ns > first.qpc_100ns,
            "native clock did not advance: {self:?}"
        );
        Ok(())
    }
}

#[derive(Default, Debug, Serialize)]
pub(super) struct Observation {
    pub signal_frames: u64,
    pub silent_frames: u64,
}

impl Observation {
    pub fn observe(&mut self, frames: &[f32], expected: [f32; 2]) -> anyhow::Result<()> {
        anyhow::ensure!(frames.len().is_multiple_of(2), "incomplete two-slot frame");
        // Accommodate PCM16 rounding at each boundary, not gain/channel changes.
        const TOLERANCE: f32 = 2.0 / 32768.0;
        for frame in frames.as_chunks::<2>().0 {
            anyhow::ensure!(frame.iter().all(|v| v.is_finite()), "nonfinite PCM");
            if frame.iter().all(|v| v.abs() <= TOLERANCE) {
                self.silent_frames += 1;
            } else {
                anyhow::ensure!(
                    frame
                        .iter()
                        .zip(expected)
                        .all(|(actual, expected)| (actual - expected).abs() <= TOLERANCE),
                    "unexpected PCM frame {frame:?}, expected {expected:?} or startup silence"
                );
                self.signal_frames += 1;
            }
        }
        Ok(())
    }

    pub fn require_signal(&self, minimum: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            minimum > 0 && self.signal_frames >= minimum,
            "insufficient matching PCM: {} signal frames, {} silent frames, need {minimum}",
            self.signal_frames,
            self.silent_frames
        );
        Ok(())
    }

    pub fn require_silence(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.signal_frames == 0 && self.silent_frames > 0,
            "closed gate did not produce observed silence: {self:?}"
        );
        Ok(())
    }
}

pub(super) fn revoke_and_drain<T>(
    outcome: anyhow::Result<T>,
    revoke: impl FnOnce() -> anyhow::Result<()>,
    drain: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<T> {
    // A failed phase or revocation must not short-circuit the cancellation
    // drain. Preserve the phase error without ever converting cleanup failure
    // into a successful native case.
    let revoked = revoke();
    let drained = drain();
    let value = outcome?;
    revoked?;
    drained?;
    Ok(value)
}

pub(super) fn cleanup_all<T: Copy>(
    items: &[T],
    mut cleanup: impl FnMut(T) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    for &item in items {
        if let Err(error) = cleanup(item) {
            errors.push(format!("{error:#}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} cleanup operation(s) failed: {}",
            errors.len(),
            errors.join("; ")
        )
    }
}

#[derive(Debug, Serialize)]
pub(super) struct FailureCounters {
    pub mode: crate::NativeMode,
    pub signal_frames: u64,
    pub silent_frames: u64,
    pub microphone_dropped_packets: u64,
    pub native_buffer_frames: u32,
    pub driver_packet_frames: u32,
    pub stream_generation: u64,
    pub native_discontinuities: u64,
    pub native_position: Option<PositionProgress>,
}

pub(super) fn diagnose_failure<T>(
    outcome: anyhow::Result<T>,
    counters: &FailureCounters,
    emit: impl FnOnce(&str),
) -> anyhow::Result<T> {
    if let Err(error) = &outcome {
        #[derive(Serialize)]
        struct Diagnostic<'a> {
            diagnostic: &'static str,
            error: String,
            counters: &'a FailureCounters,
        }
        let diagnostic = Diagnostic {
            diagnostic: "vad-native-case-failure",
            error: format!("{error:#}"),
            counters,
        };
        // Diagnostics must not replace the phase or cleanup error, even if
        // formatting fails. Return the same error object and its full chain.
        let line = serde_json::to_string(&diagnostic).unwrap_or_else(|format_error| {
            format!("vad-native-case-failure: {error:#}; counters={counters:?}; diagnostic formatting: {format_error}")
        });
        emit(&line);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = r"ROOT\MOIRENVAD\0000";

    fn candidate() -> Candidate {
        Candidate {
            id: "opaque MMDevice identity".into(),
            raw_instance: r"MOIREN\VAD_RAW\7&ABCD&0&123456789ABCDEF00000000000000042".into(),
            parent_instance: ROOT.into(),
            intent: Intent::Playback,
        }
    }

    fn id() -> StableEndpointId {
        StableEndpointId::new(0x42, 0x1234_5678_9abc_def0)
    }

    #[test]
    fn exact_owned_adapter_and_direction_select_an_opaque_mmdevice_id() {
        let rows = [candidate()];
        assert_eq!(
            select(&rows, id(), Intent::Playback, &ROOT.to_ascii_lowercase())
                .unwrap()
                .unwrap()
                .id,
            rows[0].id
        );
        assert!(
            select(&rows, id(), Intent::Microphone, ROOT)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn another_root_or_similar_instance_cannot_authorize_audio() {
        for instance in [
            r"OTHER\VAD_RAW\7&ABCD&0&123456789ABCDEF00000000000000042",
            r"MOIREN\VAD_RAW\7&ABCD&0&123456789ABCDEF00000000000000043",
            r"MOIREN\VAD_RAW\7123456789ABCDEF00000000000000042",
            r"MOIREN\VAD_RAW\7&ABCD&0&123456789ABCDEF00000000000000042tail",
            r"MOIREN\VAD_RAW\123456789ABCDEF00000000000000042\extra",
        ] {
            let rows = [Candidate {
                raw_instance: instance.into(),
                ..candidate()
            }];
            assert!(
                select(&rows, id(), Intent::Playback, ROOT)
                    .unwrap()
                    .is_none(),
                "{instance}"
            );
        }
        let rows = [Candidate {
            parent_instance: r"ROOT\MOIRENVAD\0001".into(),
            ..candidate()
        }];
        assert!(
            select(&rows, id(), Intent::Playback, ROOT)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn ambiguous_adapter_mapping_is_an_error() {
        let rows = [
            candidate(),
            Candidate {
                id: "different endpoint on same adapter".into(),
                ..candidate()
            },
        ];
        assert!(select(&rows, id(), Intent::Playback, ROOT).is_err());
    }

    #[test]
    fn patterns_distinguish_channels_and_siblings() {
        for index in 0..8 {
            assert_ne!(
                pattern(index, Channels::Stereo)[0],
                pattern(index, Channels::Stereo)[1]
            );
            assert_eq!(pattern(index, Channels::Mono)[1], 0.0);
            for other in 0..index {
                assert_ne!(
                    pattern(index, Channels::Stereo),
                    pattern(other, Channels::Stereo)
                );
            }
        }
    }

    #[test]
    fn replayed_or_regressing_packets_cannot_count_as_fresh_pcm() {
        let stamp = |sequence, qpc| PcmPacketStamp { sequence, qpc };
        let mut progress = PacketProgress::default();
        progress.observe(stamp(0, 100)).unwrap();
        assert!(progress.observe(stamp(0, 100)).is_err());
        progress.observe(stamp(3, 101)).unwrap(); // Gaps from bounded drops are valid.
        assert!(progress.observe(stamp(2, 102)).is_err());
        assert!(progress.observe(stamp(4, 99)).is_err());
        progress.observe(stamp(4, 101)).unwrap();
    }

    #[test]
    fn startup_silence_does_not_count_as_pcm_evidence() {
        let mut seen = Observation::default();
        seen.observe(&[0.0; 4], pattern(0, Channels::Mono)).unwrap();
        assert_eq!(seen.silent_frames, 2);
        assert!(seen.require_signal(1).is_err());
        seen.require_silence().unwrap();
        assert!(Observation::default().require_silence().is_err());
    }

    #[test]
    fn both_channels_and_complete_frames_are_required() {
        let expected = pattern(1, Channels::Stereo);
        let mut seen = Observation::default();
        seen.observe(&expected.repeat(3), expected).unwrap();
        assert_eq!(seen.signal_frames, 3);
        seen.require_signal(3).unwrap();
        assert!(seen.require_signal(4).is_err());
        assert!(seen.require_silence().is_err());
        for corrupt in [
            vec![expected[1], expected[0]],
            vec![expected[0], 0.0],
            vec![f32::NAN, expected[1]],
            vec![f32::INFINITY, expected[1]],
            vec![expected[0]],
        ] {
            assert!(Observation::default().observe(&corrupt, expected).is_err());
        }
    }

    #[test]
    fn pcm16_rounding_is_allowed_but_wrong_audio_is_rejected() {
        let expected = pattern(0, Channels::Stereo);
        let quantum = 1.0 / 32768.0;
        let mut seen = Observation::default();
        seen.observe(&[expected[0] + quantum, expected[1] - quantum], expected)
            .unwrap();
        seen.require_signal(1).unwrap();
        assert!(seen.observe(&[0.5, 0.5], expected).is_err());
    }

    #[test]
    fn failed_pcm_phase_still_revokes_before_draining() {
        let actions = std::cell::RefCell::new(Vec::new());
        let result = revoke_and_drain::<()>(
            Err(anyhow::anyhow!("PCM failed")),
            || {
                actions.borrow_mut().push("revoke");
                Ok(())
            },
            || {
                actions.borrow_mut().push("drain");
                Ok(())
            },
        );
        assert_eq!(*actions.borrow(), ["revoke", "drain"]);
        assert!(result.unwrap_err().to_string().contains("PCM failed"));
    }

    #[test]
    fn failed_revoke_still_drains_and_cannot_report_success() {
        let drained = std::cell::Cell::new(false);
        let result = revoke_and_drain(
            Ok(42),
            || Err(anyhow::anyhow!("revoke failed")),
            || {
                drained.set(true);
                Ok(())
            },
        );
        assert!(drained.get());
        assert!(result.unwrap_err().to_string().contains("revoke failed"));
        assert!(
            revoke_and_drain(Ok(42), || Ok(()), || Err(anyhow::anyhow!("drain failed"))).is_err()
        );
        assert_eq!(revoke_and_drain(Ok(42), || Ok(()), || Ok(())).unwrap(), 42);
    }

    #[test]
    fn cleanup_runs_every_item_and_reports_all_failures() {
        let actions = std::cell::RefCell::new(Vec::new());
        let result = cleanup_all(&[1_u8, 2, 3], |item| {
            actions.borrow_mut().push(item);
            match item {
                1 => Err(anyhow::anyhow!("first cleanup failed")),
                2 => Ok(()),
                3 => Err(anyhow::anyhow!("last cleanup failed")),
                _ => unreachable!(),
            }
        });
        assert_eq!(*actions.borrow(), [1, 2, 3]);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("2 cleanup operation(s) failed"));
        assert!(error.contains("first cleanup failed"));
        assert!(error.contains("last cleanup failed"));
    }

    #[test]
    fn timer_render_uses_only_unpadded_frames_without_catch_up() {
        assert_eq!(timer_render_frames(480, 480).unwrap(), 0);
        assert_eq!(timer_render_frames(480, 360).unwrap(), 120);
        assert_eq!(timer_render_frames(480, 0).unwrap(), 480);
        assert!(timer_render_frames(480, 481).is_err());
        assert!(timer_render_frames(0, 0).is_err());
    }

    #[test]
    fn timer_position_requires_real_progress_and_rejects_regressions() {
        let mut progress = PositionProgress {
            frequency: 48_000,
            ..Default::default()
        };
        assert!(progress.require_advance().is_err());
        progress.observe(0, 100).unwrap();
        progress.observe(0, 100).unwrap();
        assert!(progress.require_advance().is_err());
        progress.observe(48, 10_100).unwrap();
        progress.require_advance().unwrap();
        assert!(progress.observe(47, 10_101).is_err());
        assert!(progress.observe(49, 10_099).is_err());
        progress.frequency = 0;
        assert!(progress.require_advance().is_err());
    }

    #[test]
    fn timer_signal_phase_cannot_reuse_closed_gate_clock_evidence() {
        let mut progress = PositionProgress {
            frequency: 48_000,
            ..Default::default()
        };
        progress.observe(0, 100).unwrap();
        progress.observe(48, 10_100).unwrap();
        progress.require_advance().unwrap();
        progress.begin_phase();
        assert_eq!(progress.frequency, 48_000);
        assert!(progress.require_advance().is_err());
        progress.observe(48, 10_101).unwrap();
        assert!(progress.require_advance().is_err());
        progress.observe(96, 20_100).unwrap();
        progress.require_advance().unwrap();
        let report = serde_json::to_value(&progress).unwrap();
        assert_eq!(report["first"]["position_units"], 48);
        assert_eq!(report["last"]["qpc_100ns"], 20_100);
    }

    #[test]
    fn failed_outcome_diagnoses_counters_after_cleanup_and_preserves_original_error() {
        let mut position = PositionProgress {
            frequency: 48_000,
            ..Default::default()
        };
        position.observe(48, 10_100).unwrap();
        position.observe(96, 20_100).unwrap();
        let counters = FailureCounters {
            mode: crate::NativeMode::Timer,
            signal_frames: 23_136,
            silent_frames: 25_344,
            microphone_dropped_packets: 27,
            native_buffer_frames: 2048,
            driver_packet_frames: 480,
            stream_generation: 7,
            native_discontinuities: 2,
            native_position: Some(position),
        };
        let actions = std::cell::RefCell::new(Vec::new());
        let error = anyhow::Error::new(std::io::Error::from_raw_os_error(5))
            .context("native PCM transfer failed");
        let identity = error.downcast_ref::<std::io::Error>().unwrap() as *const _;
        let outcome = revoke_and_drain::<()>(
            Err(error),
            || {
                actions.borrow_mut().push("revoke");
                anyhow::bail!("revoke also failed")
            },
            || {
                actions.borrow_mut().push("drain");
                anyhow::bail!("drain also failed")
            },
        );
        let mut diagnostic = String::new();
        let error = diagnose_failure(outcome, &counters, |line| {
            actions.borrow_mut().push("diagnose");
            diagnostic = line.to_owned();
        })
        .unwrap_err();
        assert_eq!(*actions.borrow(), ["revoke", "drain", "diagnose"]);
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap() as *const _,
            identity
        );
        assert_eq!(error.to_string(), "native PCM transfer failed");
        let diagnostic: serde_json::Value = serde_json::from_str(&diagnostic).unwrap();
        assert!(
            diagnostic["error"]
                .as_str()
                .unwrap()
                .contains("native PCM transfer failed")
        );
        let values = &diagnostic["counters"];
        assert_eq!(values["mode"], "exclusive-timer");
        assert_eq!(values["signal_frames"], 23_136);
        assert_eq!(values["silent_frames"], 25_344);
        assert_eq!(values["microphone_dropped_packets"], 27);
        assert_eq!(values["native_buffer_frames"], 2048);
        assert_eq!(values["driver_packet_frames"], 480);
        assert_eq!(values["stream_generation"], 7);
        assert_eq!(values["native_discontinuities"], 2);
        assert_eq!(values["native_position"]["last"]["position_units"], 96);
        assert_eq!(values["native_position"]["last"]["qpc_100ns"], 20_100);
    }

    #[test]
    fn failure_diagnostic_does_not_emit_or_change_a_successful_outcome() {
        let counters = FailureCounters {
            mode: crate::NativeMode::Event,
            signal_frames: 47_520,
            silent_frames: 0,
            microphone_dropped_packets: 0,
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 9,
            native_discontinuities: 0,
            native_position: None,
        };
        assert_eq!(
            diagnose_failure(Ok(42), &counters, |_| panic!("success is not failure")).unwrap(),
            42
        );
    }

    #[test]
    fn diagnostic_keeps_the_existing_signal_threshold_and_unavailable_position() {
        let seen = Observation {
            signal_frames: 23_136,
            silent_frames: 25_344,
        };
        let outcome = seen.require_signal(48_000 / 2);
        assert!(outcome.is_err());
        let counters = FailureCounters {
            mode: crate::NativeMode::Event,
            signal_frames: seen.signal_frames,
            silent_frames: seen.silent_frames,
            microphone_dropped_packets: 31,
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 9,
            native_discontinuities: 0,
            native_position: None,
        };
        let mut line = String::new();
        let error =
            diagnose_failure(outcome, &counters, |message| line = message.to_owned()).unwrap_err();
        assert!(error.to_string().contains("need 24000"));
        let diagnostic: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(diagnostic["counters"]["signal_frames"], 23_136);
        assert_eq!(diagnostic["counters"]["silent_frames"], 25_344);
        assert_eq!(diagnostic["counters"]["mode"], "exclusive-event");
        assert!(diagnostic["counters"]["native_position"].is_null());
    }

    #[test]
    fn cleanup_failure_after_success_still_emits_counters_and_fails() {
        let counters = FailureCounters {
            mode: crate::NativeMode::Event,
            signal_frames: 47_520,
            silent_frames: 0,
            microphone_dropped_packets: 0,
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 9,
            native_discontinuities: 0,
            native_position: None,
        };
        let drained = std::cell::Cell::new(false);
        let outcome = revoke_and_drain(
            Ok(42),
            || anyhow::bail!("revoke failed"),
            || {
                drained.set(true);
                Ok(())
            },
        );
        let mut line = String::new();
        let error =
            diagnose_failure(outcome, &counters, |message| line = message.to_owned()).unwrap_err();
        assert!(drained.get());
        assert_eq!(error.to_string(), "revoke failed");
        let diagnostic: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(diagnostic["error"], "revoke failed");
        assert_eq!(diagnostic["counters"]["signal_frames"], 47_520);
    }
}
