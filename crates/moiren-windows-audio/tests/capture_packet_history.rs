//! Raw native packet history is bounded evidence, never a presentation clock.
mod common;

use moiren_windows_audio::capture::{PcmArtifactConfig, PcmEvidencePacket, PcmRecorder};

#[test]
fn packet_history_keeps_raw_qpc_flags_and_partial_storage_without_rt_allocation() {
    let mut recorder =
        PcmRecorder::new(PcmArtifactConfig::new(3).with_packet_capacity(2), 48_000, 2).unwrap();
    let payload = [0_u8; 16];
    for (position, qpc) in [(0, 1234), (0, 0)] {
        common::alloc_guard::assert_allocates_nothing(|| {
            recorder
                .record_native_packet(
                    &payload,
                    PcmEvidencePacket {
                        frames: 2,
                        flags: 0,
                        device_position_frames: position,
                        qpc_100ns: qpc,
                    },
                )
                .unwrap();
        });
    }
    common::alloc_guard::assert_allocates_nothing(|| {
        recorder
            .record_native_packet(
                &[],
                PcmEvidencePacket {
                    frames: 2,
                    flags: 2,
                    device_position_frames: 0,
                    qpc_100ns: 4321,
                },
            )
            .unwrap();
    });
    let artifact = recorder.finish();
    let rows = artifact.packet_history();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].packet_sequence, 1);
    assert_eq!(rows[0].qpc_100ns, Some(1234));
    assert_eq!(rows[1].packet_sequence, 2);
    assert_eq!(rows[1].qpc_100ns, Some(0), "retain unavailable native zero");
    assert_eq!(rows[1].stored_frame_start, 2);
    assert_eq!(rows[1].stored_frames, 1);
    assert_eq!(rows[1].frames, 2);
    let history = artifact.metadata().packet_history.unwrap();
    assert_eq!(history.stored_packets, 2);
    assert_eq!(history.omitted_packets, 1);
    assert_eq!(history.omitted_frames, 2);
    assert!(!history.complete);
    assert_eq!(artifact.metadata().dropped_frames, 3);
    assert_eq!(artifact.metadata().silent_frames, 2);
}

#[test]
fn packet_history_is_opt_in_and_legacy_timestamps_stay_unknown() {
    let mut legacy = PcmRecorder::new(PcmArtifactConfig::new(4), 48_000, 2).unwrap();
    legacy.record_packet(&[0; 16], 2, 0, 0).unwrap();
    let artifact = legacy.finish();
    assert!(artifact.packet_history().is_empty());
    assert!(artifact.metadata().packet_history.is_none());
    assert_eq!(artifact.metadata().schema_version, 2);
    assert!(
        serde_json::to_value(artifact.metadata())
            .unwrap()
            .get("packet_history")
            .is_none()
    );

    let mut recorder =
        PcmRecorder::new(PcmArtifactConfig::new(4).with_packet_capacity(2), 48_000, 2).unwrap();
    recorder.record_packet(&[0; 16], 2, 0, 0).unwrap();
    let artifact = recorder.finish();
    assert_eq!(artifact.packet_history()[0].qpc_100ns, None);
    assert!(artifact.metadata().packet_history.unwrap().complete);
    assert_eq!(artifact.metadata().schema_version, 3);
}

#[test]
fn packet_history_preparation_refuses_excess_storage() {
    assert!(
        PcmRecorder::new(
            PcmArtifactConfig::new(4).with_packet_capacity(65_537),
            48_000,
            2,
        )
        .is_err()
    );
}

#[test]
fn packet_history_preserves_raw_flags_and_ignores_empty_or_invalid_packets() {
    let mut recorder =
        PcmRecorder::new(PcmArtifactConfig::new(4).with_packet_capacity(2), 48_000, 2).unwrap();
    let packet = PcmEvidencePacket {
        frames: 2,
        flags: 0,
        device_position_frames: u64::MAX,
        qpc_100ns: 77,
    };
    assert!(recorder.record_native_packet(&[0; 8], packet).is_err());
    recorder
        .record_native_packet(
            &[],
            PcmEvidencePacket {
                frames: 0,
                ..packet
            },
        )
        .unwrap();
    recorder
        .record_native_packet(&[], PcmEvidencePacket { flags: 7, ..packet })
        .unwrap();
    let artifact = recorder.finish();
    assert_eq!(artifact.packet_history().len(), 1);
    let row = artifact.packet_history()[0];
    assert_eq!(row.packet_sequence, 1);
    assert_eq!(row.flags, 7);
    assert_eq!(row.device_position_frames, u64::MAX);
    assert_eq!(
        row.qpc_100ns,
        Some(77),
        "timestamp-error flag does not rewrite raw QPC"
    );
    assert_eq!(artifact.metadata().timestamp_error_packets, 1);
    assert_eq!(artifact.metadata().discontinuity_events, 1);
    assert_eq!(
        artifact.metadata().packet_history.unwrap().omitted_packets,
        0
    );
}

#[test]
fn packet_history_retains_offers_after_pcm_fills_and_decomposes_without_loss() {
    let mut recorder =
        PcmRecorder::new(PcmArtifactConfig::new(1).with_packet_capacity(3), 48_000, 1).unwrap();
    for index in 0..3 {
        common::alloc_guard::assert_allocates_nothing(|| {
            recorder
                .record_native_packet(
                    &0.25_f32.to_le_bytes(),
                    PcmEvidencePacket {
                        frames: 1,
                        flags: 0,
                        device_position_frames: index,
                        qpc_100ns: 100 + index,
                    },
                )
                .unwrap();
        });
    }
    let (metadata, pcm, rows) = recorder.finish().into_parts_with_packet_history();
    assert_eq!(pcm, 0.25_f32.to_le_bytes());
    assert_eq!(metadata.dropped_frames, 2);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].stored_frames, 0);
    assert_eq!(rows[2].stored_frames, 0);
    assert_eq!(rows[2].stored_frame_start, 1);
    assert_eq!(rows[2].observed_frame_start, 2);
    assert_eq!(rows[2].qpc_100ns, Some(102));
    assert!(
        metadata.packet_history.unwrap().complete,
        "complete packet history does not assert complete PCM storage"
    );
}
