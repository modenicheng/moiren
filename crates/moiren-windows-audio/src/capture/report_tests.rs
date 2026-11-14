use super::*;

#[test]
fn capture_artifact_process_zero_positions_do_not_manufacture_packet_gaps() {
    use crate::capture::{CaptureSource, PcmArtifactConfig};
    for source in [CaptureSource::ProcessLoopback, CaptureSource::Physical] {
        let mut report = CaptureReport::new(source, crate::SessionDuration::UntilStopped);
        report
            .prepare_artifact(Some(PcmArtifactConfig::new(8)), 48_000, 2)
            .unwrap();
        let recorder = report.artifact_recorder.as_mut().unwrap();
        for _ in 0..2 {
            recorder.record_packet(&[0; 16], 2, 0, 0).unwrap();
        }
        report.finish_artifact();
        let metadata = report.artifact_metadata.unwrap();
        assert_eq!(metadata.stored_frames, 4);
        assert_eq!(metadata.schema_version, 2);
        assert_eq!(
            metadata.detect_position_gaps,
            source == CaptureSource::Physical
        );
        assert_eq!(
            metadata.gap_events,
            u64::from(source == CaptureSource::Physical),
            "Process has no trustworthy device-position continuity; physical still does"
        );
    }
}

#[test]
fn capture_artifact_process_retains_native_flags_without_position_correlation() {
    use crate::capture::{CaptureSource, PcmArtifactConfig};
    let mut report = CaptureReport::new(
        CaptureSource::ProcessLoopback,
        crate::SessionDuration::UntilStopped,
    );
    report
        .prepare_artifact(Some(PcmArtifactConfig::new(2)), 48_000, 2)
        .unwrap();
    let recorder = report.artifact_recorder.as_mut().unwrap();
    recorder
        .record_packet(
            &[],
            2,
            crate::stats::SILENT | crate::stats::DISCONTINUITY | crate::stats::TIMESTAMP_ERROR,
            0,
        )
        .unwrap();
    recorder
        .record_packet(&[], 2, crate::stats::SILENT, 0)
        .unwrap();
    report.finish_artifact();
    let json = serde_json::to_value(report).unwrap();
    let metadata = &json["artifact_metadata"];
    assert_eq!(metadata["timestamp_error_packets"], 1);
    assert_eq!(metadata["discontinuity_events"], 1);
    assert_eq!(metadata["silent_frames"], 4);
    assert_eq!(metadata["dropped_frames"], 2);
    assert_eq!(metadata["overflow_events"], 1);
    assert_eq!(metadata["gap_events"], 0);
    assert_eq!(metadata["detect_position_gaps"], false);
    assert_eq!(metadata["schema_version"], 2);
}

#[test]
fn capture_artifact_converted_physical_positions_do_not_use_client_frame_units() {
    use crate::capture::{CaptureSource, PcmArtifactConfig};
    let mut report = CaptureReport::new(
        CaptureSource::Physical,
        crate::SessionDuration::UntilStopped,
    );
    report.shared_period.mix_format = Some(crate::latency::shared_period::SharedFormatInfo {
        sample_rate: 96_000,
        channels: 2,
        container_bits: 32,
        valid_bits: None,
        channel_mask: None,
        floating_point: true,
    });
    report
        .prepare_artifact(Some(PcmArtifactConfig::new(8)), 48_000, 2)
        .unwrap();
    let recorder = report.artifact_recorder.as_mut().unwrap();
    recorder.record_packet(&[0; 16], 2, 0, 0).unwrap();
    recorder.record_packet(&[0; 16], 2, 0, 4).unwrap();
    report.finish_artifact();
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(json["artifact_metadata"]["detect_position_gaps"], false);
    assert_eq!(json["artifact_metadata"]["gap_events"], 0);
}
