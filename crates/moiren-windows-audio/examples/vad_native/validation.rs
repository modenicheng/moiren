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
}
