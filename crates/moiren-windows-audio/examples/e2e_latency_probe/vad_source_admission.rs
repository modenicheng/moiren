//! A Running graph does not imply that its VAD IO worker has queued READs.
//! Only this probe's still-Paused external source can use this startup check.

use anyhow::{Context, Result, ensure};
use moiren_windows_audio::vad::{
    Intent, PdoState, REQUEST_SLOTS, StreamState, VadExportDiagnosticProvenance,
    VadExportObservation,
};

pub(super) fn playback_reads_ready(
    before: &VadExportObservation,
    current: &VadExportObservation,
) -> Result<bool> {
    let baseline = before
        .snapshot
        .context("initial playback snapshot missing")?;
    let snapshot = current.snapshot.context("playback snapshot missing")?;
    ensure!(
        current.endpoint_id == before.endpoint_id
            && current.intent == Intent::Playback
            && before.intent == Intent::Playback
            && current.channels == before.channels
            && snapshot.stable_id() == baseline.stable_id()
            && snapshot.key() == baseline.key()
            && snapshot.stream_generation() == baseline.stream_generation()
            && snapshot.gate_epoch() == baseline.gate_epoch()
            && snapshot.packet_frames() == baseline.packet_frames()
            && snapshot.sample_format() == baseline.sample_format()
            && snapshot.channels() == baseline.channels()
            && snapshot.stable_id().to_virtual_endpoint_id() == current.endpoint_id
            && snapshot.intent() == Intent::Playback
            && snapshot.channels() == current.channels
            && snapshot.packet_frames() > 0
            && snapshot.sample_format().is_some(),
        "VAD playback identity/shape changed before source Start"
    );
    for view in [before, current] {
        let snapshot = view.snapshot.context("playback snapshot missing")?;
        ensure!(
            view.diagnostic_provenance == VadExportDiagnosticProvenance::Fresh
                && view.error.is_none()
                && !view.removing
                && view.transport_prepared
                && snapshot.state() == PdoState::Present
                && snapshot.audio_ready()
                && snapshot.policy_prepared()
                && snapshot.gate_open()
                && snapshot.stream_state() == StreamState::Paused,
            "VAD playback not freshly admitted and Paused before source Start"
        );
        ensure!(
            snapshot.dropped_packets() == 0 && snapshot.silence_packets() == 0,
            "VAD driver already reports loss before source Start"
        );
        let t = view.telemetry.context("playback READ telemetry missing")?;
        ensure!(
            [
                t.completed_packets,
                t.dropped_packets,
                t.driver_dropped_packets,
                t.quota_dropped_packets,
                t.carrier_dropped_packets,
                t.stale_packets,
                t.silence_packets,
                t.silence_frames,
                t.epoch_resets,
                t.recovery_events,
                t.sequence_gap_observations,
                t.missing_sequence_slots,
                t.completion_clock_regressions,
            ]
            .into_iter()
            .all(|count| count == 0),
            "VAD READ startup already completed, lost data or reset"
        );
        ensure!(
            t.submission_attempts <= REQUEST_SLOTS as u64
                && t.submitted_packets <= REQUEST_SLOTS as u64,
            "VAD READ startup exceeded its initial bounded prefix"
        );
    }
    let telemetry = current.telemetry.expect("checked telemetry");
    let initial = before.telemetry.expect("checked telemetry");
    ensure!(
        telemetry.submitted_packets >= initial.submitted_packets
            && telemetry.submission_attempts >= initial.submission_attempts,
        "VAD READ submission counters regressed before source Start"
    );
    Ok(telemetry.submitted_packets == REQUEST_SLOTS as u64
        && telemetry.submission_attempts == REQUEST_SLOTS as u64
        && telemetry.packet_frames == snapshot.packet_frames())
}
