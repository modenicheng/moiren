use super::*;
use crate::{
    SessionDuration, StopSignal,
    capture::{PcmEvidenceConfig, PcmEvidencePacket, pcm_evidence_stream},
    stats::SILENT,
};

#[test]
fn capture_evidence_survives_native_error_and_unwind_until_control_join() {
    for panics in [false, true] {
        let stop = StopSignal::new().unwrap();
        let (mut writer, mut reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 2,
                detect_position_gaps: true,
            },
            48_000,
            2,
        )
        .unwrap();
        writer
            .record_packet(
                &[],
                PcmEvidencePacket {
                    frames: 2,
                    flags: SILENT,
                    device_position_frames: 10,
                    qpc_100ns: 42,
                },
            )
            .unwrap();
        let worker = std::thread::spawn(move || {
            let (sender, _startup) = std::sync::mpsc::sync_channel(1);
            let preparation = CapturePreparation::Immediate(sender);
            let mut report =
                CaptureReport::new(CaptureSource::Physical, SessionDuration::UntilStopped);
            report.evidence_recorder = Some(writer);
            run_owner_with(&stop.event, &preparation, report, |_| {
                if panics {
                    panic!("native packet callback panic");
                }
                Err(CaptureError::InvalidPacket)
            })
        });
        let report = worker.join().unwrap();
        let expected = if panics {
            CaptureError::WorkerPanicked
        } else {
            CaptureError::InvalidPacket
        };
        assert_eq!(
            report.failure.as_deref(),
            Some(expected.to_string().as_str())
        );
        assert_eq!(report.status, CaptureStatus::Failed);
        assert!(
            report.evidence_recorder.is_some(),
            "native unwind dropped the writer"
        );
        assert_eq!(report.evidence_metadata.unwrap().stored_frames, 2);
        assert!(!reader.is_finished());
        assert_eq!(reader.pop().unwrap().device_position_frames, 10);
        assert_eq!(reader.pop().unwrap().packet_frame_offset, 1);
        assert!(reader.is_finished());
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("evidence_recorder").is_none());
        assert_eq!(json["schema_version"], 10);
        assert_eq!(json["evidence_metadata"]["stored_frames"], 2);
    }
}

#[test]
fn native_evidence_close_never_frees_abandoned_reader_storage() {
    let stop = StopSignal::new().unwrap();
    let (sender, _startup) = std::sync::mpsc::sync_channel(1);
    let preparation = CapturePreparation::Immediate(sender);
    let (writer, reader) = pcm_evidence_stream(
        PcmEvidenceConfig {
            capacity_frames: 2,
            detect_position_gaps: false,
        },
        48_000,
        1,
    )
    .unwrap();
    drop(reader);
    let mut report = CaptureReport::new(CaptureSource::Physical, SessionDuration::UntilStopped);
    report.evidence_recorder = Some(writer);
    let worker = std::thread::spawn(move || {
        crate::render::wasapi::count_test_allocations(|| {
            preparation.finish(Ok(CaptureStatus::Stopped), &stop.event, report)
        })
    });
    let (report, counts) = worker.join().unwrap();
    assert_eq!(counts, (0, 0), "native close allocated or freed evidence");
    assert!(report.evidence_recorder.is_some());
    let (_, (_, frees)) = crate::render::wasapi::count_test_allocations(|| drop(report));
    assert!(frees > 0, "control-side report did not reclaim storage");
}
