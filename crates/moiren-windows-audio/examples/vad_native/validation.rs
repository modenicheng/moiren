use moiren_windows_audio::vad::{
    Channels, EndpointSnapshot, Intent, PcmPacketStamp, PdoState, SampleFormat, StableEndpointId,
    StreamState,
};
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

/// Detached diagnostic evidence only. Explicit scalar fields keep the report
/// portable without parsing Debug output or lending the snapshot new authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct EndpointDiagnostic {
    pub stable_id: String,
    pub pdo_token: u64,
    pub session_epoch: u64,
    pub endpoint_generation: u64,
    pub stream_generation: u64,
    pub gate_epoch: u64,
    pub gate_open: bool,
    pub state: &'static str,
    pub stream_state: &'static str,
    pub intent: &'static str,
    pub sample_format: Option<&'static str>,
    pub packet_frames: u32,
    pub channels: u32,
    pub dropped_packets: u64,
    pub silence_packets: u64,
    pub last_status: u32,
    pub policy_prepared: bool,
    pub audio_ready: bool,
}

impl From<EndpointSnapshot> for EndpointDiagnostic {
    fn from(snapshot: EndpointSnapshot) -> Self {
        Self {
            stable_id: snapshot.stable_id().to_string(),
            pdo_token: snapshot.pdo_token().get(),
            session_epoch: snapshot.session_epoch(),
            endpoint_generation: snapshot.endpoint_generation(),
            stream_generation: snapshot.stream_generation(),
            gate_epoch: snapshot.gate_epoch(),
            gate_open: snapshot.gate_open(),
            state: match snapshot.state() {
                PdoState::Present => "Present",
                PdoState::Removing => "Removing",
                PdoState::Removed => "Removed",
                PdoState::Creating => "Creating",
            },
            stream_state: match snapshot.stream_state() {
                StreamState::Closed => "Closed",
                StreamState::Paused => "Paused",
                StreamState::Running => "Running",
            },
            intent: match snapshot.intent() {
                Intent::Playback => "Playback",
                Intent::Microphone => "Microphone",
            },
            sample_format: snapshot.sample_format().map(|format| match format {
                SampleFormat::Pcm16 => "Pcm16",
                SampleFormat::Float32 => "Float32",
            }),
            packet_frames: snapshot.packet_frames(),
            channels: snapshot.channels() as u32,
            dropped_packets: snapshot.dropped_packets(),
            silence_packets: snapshot.silence_packets(),
            last_status: snapshot.last_status(),
            policy_prepared: snapshot.policy_prepared(),
            audio_ready: snapshot.audio_ready(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct FailureObservation {
    pub scope: &'static str,
    pub current_snapshot: Option<EndpointDiagnostic>,
    pub query_error: Option<String>,
    pub progress_validation_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct MicrophoneCompletionIdentity {
    // Names the actual PcmCompletion variant, not an NTSTATUS or the reason
    // for a driver's freshness rejection.
    status: &'static str,
    // One-based capture completion collection order within this case, not a
    // native packet index. Unknown after the diagnostic counter overflows.
    ordinal: Option<u64>,
    // Capture completion stamps are the original user-side WRITE submission
    // identity, not driver consumption time. Both QPC fields are raw ticks,
    // unlike PositionSample's qpc_100ns; unavailable collection QPC stays null.
    stamp_sequence: u64,
    stamp_qpc: u64,
    collected_qpc: Option<u64>,
}

#[derive(Clone, Copy, Default, Debug, Serialize)]
pub(super) struct MicrophoneCompletionDiagnostics {
    completion_count: u64,
    ordinal_overflow: bool,
    first_completion: Option<MicrophoneCompletionIdentity>,
    first_dropped_completion: Option<MicrophoneCompletionIdentity>,
}

impl MicrophoneCompletionDiagnostics {
    #[cfg(windows)]
    pub fn observe(
        &mut self,
        completion: &moiren_windows_audio::vad::PcmCompletion<'_>,
        collected_qpc: impl FnOnce() -> Option<u64>,
    ) {
        use moiren_windows_audio::vad::PcmCompletion;

        let (status, stamp, dropped) = match completion {
            PcmCompletion::Capture { stamp, .. } => ("capture", stamp, false),
            PcmCompletion::CaptureDropped { stamp } => ("capture_dropped", stamp, true),
            PcmCompletion::Render { .. } => return,
        };
        match self.completion_count.checked_add(1) {
            Some(count) => self.completion_count = count,
            None => self.ordinal_overflow = true,
        }
        if self.first_completion.is_some() && (!dropped || self.first_dropped_completion.is_some())
        {
            return;
        }
        // Exactly two fixed records, with at most one QPC sample per retained
        // completion. A first completion that drops fills both with one sample.
        // Counter/sample failure is diagnostic only: never change PCM handling.
        let identity = MicrophoneCompletionIdentity {
            status,
            ordinal: (!self.ordinal_overflow).then_some(self.completion_count),
            stamp_sequence: stamp.sequence,
            stamp_qpc: stamp.qpc,
            collected_qpc: collected_qpc(),
        };
        self.first_completion.get_or_insert(identity);
        if dropped {
            self.first_dropped_completion.get_or_insert(identity);
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct FailureCounters {
    pub mode: crate::NativeMode,
    pub signal_frames: u64,
    pub silent_frames: u64,
    pub microphone_dropped_packets: u64,
    pub microphone_completion_diagnostics: MicrophoneCompletionDiagnostics,
    pub native_buffer_frames: u32,
    pub driver_packet_frames: u32,
    pub stream_generation: u64,
    pub native_discontinuities: u64,
    pub native_position: Option<PositionProgress>,
    pub admitted_opened: Option<EndpointDiagnostic>,
    pub admitted_pcm_shape: Option<EndpointDiagnostic>,
    pub failure_observation: Option<FailureObservation>,
}

pub(super) fn observe_signal_failure<T>(
    outcome: anyhow::Result<T>,
    admitted: EndpointSnapshot,
    observation: &mut Option<FailureObservation>,
    query: impl FnOnce(moiren_windows_audio::vad::EndpointKey) -> anyhow::Result<EndpointSnapshot>,
) -> anyhow::Result<T> {
    if outcome.is_err() {
        let mut evidence = FailureObservation {
            scope: "Post-failure pre-cleanup QUERY; later observation, not atomic failure instant or admission authority",
            current_snapshot: None,
            query_error: None,
            progress_validation_error: None,
        };
        // One best-effort observation, never a retry or a new admission stamp.
        // Retain decoded evidence even if lifecycle validation rejects it.
        match query(admitted.key()) {
            Ok(current) => {
                evidence.current_snapshot = Some(current.into());
                evidence.progress_validation_error = current
                    .require_progress_after(admitted)
                    .err()
                    .map(|error| error.to_string());
            }
            Err(error) => evidence.query_error = Some(format!("{error:#}")),
        }
        *observation = Some(evidence);
    }
    outcome
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

    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    // Synthetic protocol fixtures only: these epochs are not target observations.
    fn endpoint_fixture(adjust: impl FnOnce(&mut [u8; 128])) -> EndpointSnapshot {
        let mut caps = [0; 64];
        for (offset, value) in [
            (0, 64),
            (4, 2),
            (8, 31),
            (12, 64),
            (16, 1),
            (20, 48_000),
            (24, 3),
            (28, 6),
            (32, 6),
            (36, 4),
            (48, 4800),
        ] {
            put32(&mut caps, offset, value);
        }
        put64(&mut caps, 40, 7);
        let mut bytes = [0; 128];
        for (offset, value) in [
            (0, 128),
            (4, 2),
            (16, 1),
            (20, 2),
            (24, 1),
            (28, 1),
            (32, 1),
            (36, 1),
            (88, 1),
            (92, 480),
            (96, 0xc000_0059),
            (100, 2),
        ] {
            put32(&mut bytes, offset, value);
        }
        for (offset, value) in [
            (8, 0x42),
            (40, 7),
            (48, 0x42),
            (56, 11),
            (64, 13),
            (104, (1 << 53) + 1),
            (112, 23),
        ] {
            put64(&mut bytes, offset, value);
        }
        bytes[72..88].copy_from_slice(&id().to_le_bytes());
        adjust(&mut bytes);
        EndpointSnapshot::decode(
            &bytes,
            moiren_windows_audio::vad::Capabilities::decode(&caps).unwrap(),
        )
        .unwrap()
    }

    fn failure_counters() -> FailureCounters {
        FailureCounters {
            mode: crate::NativeMode::Event,
            signal_frames: 36_960,
            silent_frames: 9_600,
            microphone_dropped_packets: 2,
            microphone_completion_diagnostics: Default::default(),
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 11,
            native_discontinuities: 0,
            native_position: None,
            admitted_opened: Some(endpoint_fixture(|_| {}).into()),
            admitted_pcm_shape: Some(endpoint_fixture(|_| {}).into()),
            failure_observation: None,
        }
    }

    #[cfg(windows)]
    #[test]
    fn first_dropped_capture_identity_survives_phase_and_cleanup_failures() {
        use moiren_windows_audio::vad::PcmCompletion;

        let mut counters = failure_counters();
        counters.microphone_completion_diagnostics.observe(
            &PcmCompletion::CaptureDropped {
                stamp: PcmPacketStamp {
                    sequence: 42,
                    qpc: 91_000,
                },
            },
            || None, // QPC sampling failure must not change the PCM outcome.
        );
        counters.microphone_completion_diagnostics.observe(
            &PcmCompletion::Capture {
                stamp: PcmPacketStamp {
                    sequence: 43,
                    qpc: 92_000,
                },
                bytes: 960,
            },
            || panic!("first drop already filled both records"),
        );
        let original = anyhow::Error::new(std::io::Error::from_raw_os_error(5))
            .context("original phase failure");
        let identity = original.downcast_ref::<std::io::Error>().unwrap() as *const _;
        let outcome = revoke_and_drain::<()>(
            Err(original),
            || anyhow::bail!("revoke failed"),
            || anyhow::bail!("drain failed"),
        );
        let mut line = String::new();
        let error =
            diagnose_failure(outcome, &counters, |message| line = message.into()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap() as *const _,
            identity
        );
        let json: serde_json::Value = serde_json::from_str(&line).unwrap();
        let diagnostics = &json["counters"]["microphone_completion_diagnostics"];
        let expected = serde_json::json!({
            "status": "capture_dropped", "ordinal": 1, "stamp_sequence": 42,
            "stamp_qpc": 91_000, "collected_qpc": null,
        });
        assert_eq!(diagnostics["first_completion"], expected);
        assert_eq!(diagnostics["first_dropped_completion"], expected);
        assert_eq!(diagnostics["completion_count"], 2);
        assert_eq!(diagnostics["ordinal_overflow"], false);
    }

    #[cfg(windows)]
    #[test]
    fn capture_completion_ordinal_overflow_is_explicit_and_never_wraps() {
        use moiren_windows_audio::vad::PcmCompletion;

        let mut diagnostics = MicrophoneCompletionDiagnostics::default();
        diagnostics.observe(
            &PcmCompletion::Capture {
                stamp: PcmPacketStamp {
                    sequence: 7,
                    qpc: 100,
                },
                bytes: 960,
            },
            || Some(110),
        );
        diagnostics.completion_count = u64::MAX - 1;
        diagnostics.observe(
            &PcmCompletion::Capture {
                stamp: PcmPacketStamp {
                    sequence: 8,
                    qpc: 200,
                },
                bytes: 960,
            },
            || panic!("the first record is already retained"),
        );
        assert_eq!(diagnostics.completion_count, u64::MAX);
        assert!(!diagnostics.ordinal_overflow);
        diagnostics.observe(
            &PcmCompletion::CaptureDropped {
                stamp: PcmPacketStamp {
                    sequence: 9,
                    qpc: 300,
                },
            },
            || Some(310),
        );
        diagnostics.observe(
            &PcmCompletion::CaptureDropped {
                stamp: PcmPacketStamp {
                    sequence: 10,
                    qpc: 400,
                },
            },
            || panic!("overflow cannot replace either retained record"),
        );
        let json = serde_json::to_value(diagnostics).unwrap();
        assert_eq!(json["completion_count"], u64::MAX);
        assert_eq!(json["ordinal_overflow"], true);
        assert_eq!(json["first_completion"]["ordinal"], 1);
        assert_eq!(json["first_dropped_completion"]["stamp_sequence"], 9);
        assert!(json["first_dropped_completion"]["ordinal"].is_null());
    }

    #[test]
    fn endpoint_diagnostic_serializes_every_admitted_identity_shape_and_readiness_field() {
        let diagnostic = EndpointDiagnostic::from(endpoint_fixture(|_| {}));
        assert_eq!(
            serde_json::to_value(diagnostic).unwrap(),
            serde_json::json!({
                "stable_id": "123456789abcdef00000000000000042",
                "pdo_token": 66,
                "session_epoch": 7,
                "endpoint_generation": 66,
                "stream_generation": 11,
                "gate_epoch": 13,
                "gate_open": true,
                "state": "Present",
                "stream_state": "Running",
                "intent": "Microphone",
                "sample_format": "Pcm16",
                "packet_frames": 480,
                "channels": 1,
                "dropped_packets": 9_007_199_254_740_993_u64,
                "silence_packets": 23,
                "last_status": 0xc000_0059_u32,
                "policy_prepared": true,
                "audio_ready": true,
            })
        );
    }

    #[test]
    fn endpoint_diagnostic_keeps_closed_unready_and_unknown_format_explicit() {
        let snapshot = endpoint_fixture(|bytes| {
            for (offset, value) in [
                (16, 2),
                (20, 1),
                (24, 2),
                (28, 0),
                (32, 0),
                (36, 0),
                (88, 0),
                (92, 0),
                (100, 0),
            ] {
                put32(bytes, offset, value);
            }
        });
        let value = serde_json::to_value(EndpointDiagnostic::from(snapshot)).unwrap();
        assert_eq!(value["state"], "Removing");
        assert_eq!(value["stream_state"], "Closed");
        assert_eq!(value["intent"], "Playback");
        assert_eq!(value["channels"], 2);
        assert_eq!(value["gate_open"], false);
        assert_eq!(value["audio_ready"], false);
        assert_eq!(value["policy_prepared"], false);
        assert!(value["sample_format"].is_null());
        assert_eq!(value["packet_frames"], 0);
    }

    #[test]
    fn successful_signal_phase_does_not_query_or_add_failure_observation() {
        let mut counters = failure_counters();
        assert_eq!(
            observe_signal_failure(
                Ok(42),
                endpoint_fixture(|_| {}),
                &mut counters.failure_observation,
                |_| panic!("successful signal phase must not issue a diagnostic QUERY"),
            )
            .unwrap(),
            42
        );
        assert!(counters.failure_observation.is_none());
    }

    #[test]
    fn failed_signal_queries_once_before_revoke_and_drain_then_emits_original_error() {
        let admitted = endpoint_fixture(|_| {});
        let mut counters = failure_counters();
        let actions = std::cell::RefCell::new(Vec::new());
        let gate = std::cell::Cell::new(13);
        let original = anyhow::Error::new(std::io::Error::from_raw_os_error(1306))
            .context("GetOverlappedResult(VAD PCM)");
        let original_chain = format!("{original:#}");
        let identity = original.downcast_ref::<std::io::Error>().unwrap() as *const _;
        let outcome = observe_signal_failure::<()>(
            Err(original),
            admitted,
            &mut counters.failure_observation,
            |key| {
                actions.borrow_mut().push("query");
                assert_eq!(key, admitted.key());
                Ok(endpoint_fixture(|bytes| put64(bytes, 64, gate.get())))
            },
        );
        let outcome = revoke_and_drain(
            outcome,
            || {
                actions.borrow_mut().push("revoke");
                gate.set(14);
                anyhow::bail!("secondary revoke failure")
            },
            || {
                actions.borrow_mut().push("drain");
                anyhow::bail!("secondary drain failure")
            },
        );
        let mut line = String::new();
        let error = diagnose_failure(outcome, &counters, |message| {
            actions.borrow_mut().push("emit");
            line = message.to_owned();
        })
        .unwrap_err();
        assert_eq!(*actions.borrow(), ["query", "revoke", "drain", "emit"]);
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap() as *const _,
            identity
        );
        assert_eq!(error.to_string(), "GetOverlappedResult(VAD PCM)");
        let diagnostic: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(diagnostic["error"], original_chain);
        let evidence = &diagnostic["counters"];
        assert_eq!(evidence["admitted_opened"]["gate_epoch"], 13);
        assert_eq!(evidence["admitted_pcm_shape"]["gate_epoch"], 13);
        let observation = &evidence["failure_observation"];
        assert_eq!(
            observation["scope"],
            "Post-failure pre-cleanup QUERY; later observation, not atomic failure instant or admission authority"
        );
        assert_eq!(observation["current_snapshot"]["gate_epoch"], 13);
        assert!(observation["query_error"].is_null());
        assert!(observation["progress_validation_error"].is_null());
        assert_eq!(gate.get(), 14);
    }

    #[test]
    fn failed_signal_retains_decoded_query_snapshot_even_when_progress_validation_fails() {
        let admitted = endpoint_fixture(|_| {});
        for (current, invalid) in [
            (admitted, false),
            (endpoint_fixture(|bytes| put64(bytes, 64, 14)), false),
            (
                endpoint_fixture(|bytes| {
                    put32(bytes, 28, 0);
                    put32(bytes, 36, 0);
                    put32(bytes, 100, 1);
                }),
                false,
            ),
            (
                endpoint_fixture(|bytes| {
                    put64(bytes, 56, 12);
                    put32(bytes, 88, 2);
                    put32(bytes, 92, 960);
                }),
                false,
            ),
            (endpoint_fixture(|bytes| put64(bytes, 64, 12)), true),
            (endpoint_fixture(|bytes| put64(bytes, 72, 0x43)), true),
        ] {
            let mut counters = failure_counters();
            let calls = std::cell::Cell::new(0);
            let error = observe_signal_failure::<()>(
                Err(anyhow::anyhow!("original PCM phase failure")),
                admitted,
                &mut counters.failure_observation,
                |key| {
                    calls.set(calls.get() + 1);
                    assert_eq!(key, admitted.key());
                    Ok(current)
                },
            )
            .unwrap_err();
            assert_eq!(calls.get(), 1);
            assert_eq!(error.to_string(), "original PCM phase failure");
            let observation = counters.failure_observation.as_ref().unwrap();
            assert_eq!(observation.current_snapshot, Some(current.into()));
            assert!(observation.query_error.is_none());
            assert_eq!(observation.progress_validation_error.is_some(), invalid);
            if invalid {
                assert!(
                    observation
                        .progress_validation_error
                        .as_ref()
                        .unwrap()
                        .contains("endpoint identity or lifecycle regressed")
                );
            }
            let value = serde_json::to_value(&counters).unwrap();
            assert_eq!(
                value["failure_observation"]["current_snapshot"],
                serde_json::to_value(EndpointDiagnostic::from(current)).unwrap()
            );
            assert_eq!(value["admitted_opened"]["gate_epoch"], 13);
            assert_eq!(value["admitted_pcm_shape"]["packet_frames"], 480);
        }
    }

    #[test]
    fn failed_diagnostic_query_keeps_full_query_chain_and_original_phase_error() {
        let admitted = endpoint_fixture(|_| {});
        let mut counters = failure_counters();
        let actions = std::cell::RefCell::new(Vec::new());
        let original = anyhow::Error::new(std::io::Error::from_raw_os_error(1306))
            .context("original PCM failure context");
        let original_chain = format!("{original:#}");
        let identity = original.downcast_ref::<std::io::Error>().unwrap() as *const _;
        let outcome = observe_signal_failure::<()>(
            Err(original),
            admitted,
            &mut counters.failure_observation,
            |key| {
                actions.borrow_mut().push("query");
                assert_eq!(key, admitted.key());
                Err(anyhow::anyhow!("diagnostic QUERY transport refused")
                    .context("best-effort current endpoint QUERY"))
            },
        );
        let outcome = revoke_and_drain(
            outcome,
            || {
                actions.borrow_mut().push("revoke");
                Ok(())
            },
            || {
                actions.borrow_mut().push("drain");
                Ok(())
            },
        );
        let mut line = String::new();
        let error = diagnose_failure(outcome, &counters, |message| {
            actions.borrow_mut().push("emit");
            line = message.to_owned();
        })
        .unwrap_err();
        assert_eq!(*actions.borrow(), ["query", "revoke", "drain", "emit"]);
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap() as *const _,
            identity
        );
        assert_eq!(error.to_string(), "original PCM failure context");
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let observation = &value["counters"]["failure_observation"];
        assert!(observation["current_snapshot"].is_null());
        assert!(observation["progress_validation_error"].is_null());
        assert_eq!(
            observation["query_error"],
            "best-effort current endpoint QUERY: diagnostic QUERY transport refused"
        );
        assert_eq!(value["error"], original_chain);
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
    fn failure_counters_distinguish_unobserved_endpoint_evidence_from_missing_fields() {
        let counters = FailureCounters {
            mode: crate::NativeMode::Event,
            signal_frames: 0,
            silent_frames: 0,
            microphone_dropped_packets: 0,
            microphone_completion_diagnostics: Default::default(),
            native_buffer_frames: 0,
            driver_packet_frames: 0,
            stream_generation: 0,
            native_discontinuities: 0,
            native_position: None,
            admitted_opened: None,
            admitted_pcm_shape: None,
            failure_observation: None,
        };
        let diagnostic = serde_json::to_value(counters).unwrap();
        for field in [
            "admitted_opened",
            "admitted_pcm_shape",
            "failure_observation",
        ] {
            assert_eq!(
                diagnostic.get(field),
                Some(&serde_json::Value::Null),
                "{field} must explicitly report unavailable evidence before gate admission"
            );
        }
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
            microphone_completion_diagnostics: Default::default(),
            native_buffer_frames: 2048,
            driver_packet_frames: 480,
            stream_generation: 7,
            native_discontinuities: 2,
            native_position: Some(position),
            admitted_opened: None,
            admitted_pcm_shape: None,
            failure_observation: None,
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
            microphone_completion_diagnostics: Default::default(),
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 9,
            native_discontinuities: 0,
            native_position: None,
            admitted_opened: None,
            admitted_pcm_shape: None,
            failure_observation: None,
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
            microphone_completion_diagnostics: Default::default(),
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 9,
            native_discontinuities: 0,
            native_position: None,
            admitted_opened: None,
            admitted_pcm_shape: None,
            failure_observation: None,
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
            microphone_completion_diagnostics: Default::default(),
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 9,
            native_discontinuities: 0,
            native_position: None,
            admitted_opened: None,
            admitted_pcm_shape: None,
            failure_observation: None,
        };
        let drained = std::cell::Cell::new(false);
        let mut failure_observation = None;
        let outcome = observe_signal_failure(
            Ok(42),
            endpoint_fixture(|_| {}),
            &mut failure_observation,
            |_| panic!("successful signal phase must not QUERY for a later cleanup failure"),
        );
        let outcome = revoke_and_drain(
            outcome,
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
        assert!(failure_observation.is_none());
        assert_eq!(error.to_string(), "revoke failed");
        let diagnostic: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(diagnostic["error"], "revoke failed");
        assert_eq!(diagnostic["counters"]["signal_frames"], 47_520);
    }
}
