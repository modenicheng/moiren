use moiren_windows_audio::{
    capture::{PcmEvidenceConfig, PcmEvidencePacket, pcm_evidence_stream},
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};

mod common;

fn config(capacity_frames: usize) -> PcmEvidenceConfig {
    PcmEvidenceConfig {
        capacity_frames,
        detect_position_gaps: true,
    }
}

fn packet(frames: u32, position: u64) -> PcmEvidencePacket {
    PcmEvidencePacket {
        frames,
        flags: 0,
        device_position_frames: position,
        qpc_100ns: 42,
    }
}

#[test]
fn evidence_tap_preserves_float_bits_packet_identity_and_fifo_across_wrap() {
    let (mut owner, mut reader) = pcm_evidence_stream(config(3), 48_000, 2).unwrap();
    let samples = [
        0.25f32,
        -0.5,
        f32::from_bits(0x7fc0_1234),
        f32::NEG_INFINITY,
    ];
    let bytes: Vec<_> = samples.into_iter().flat_map(f32::to_le_bytes).collect();
    assert_eq!(owner.record_packet(&bytes, packet(2, 100)).unwrap(), 2);
    let first = reader.pop().unwrap();
    assert_eq!(first.samples, [0.25, -0.5]);
    assert_eq!(first.packet_sequence, 1);
    assert_eq!(first.packet_frame_offset, 0);
    assert_eq!(first.packet_frames, 2);
    assert_eq!(first.device_position_frames, 100);
    assert_eq!(first.qpc_100ns, 42);
    assert_eq!(owner.record_packet(&bytes, packet(2, 102)).unwrap(), 2);
    let tail = reader.pop().unwrap();
    assert_eq!(tail.samples[0].to_bits(), 0x7fc0_1234);
    assert_eq!(tail.samples[1].to_bits(), f32::NEG_INFINITY.to_bits());
    assert_eq!(tail.packet_frame_offset, 1);
    assert_eq!(reader.pop().unwrap().packet_sequence, 2);
    assert_eq!(reader.pop().unwrap().packet_frame_offset, 1);
    assert!(reader.pop().is_none());
    assert_eq!(owner.snapshot().stored_frames, 4);
    assert_eq!(owner.snapshot().gap_frames, 0);
}

#[test]
fn evidence_tap_drops_whole_packets_and_retains_sequence_and_quality() {
    let (mut owner, mut reader) = pcm_evidence_stream(config(2), 48_000, 1).unwrap();
    let bytes = [1.0f32.to_le_bytes(), 2.0f32.to_le_bytes()].concat();
    owner.record_packet(&bytes[..4], packet(1, 0)).unwrap();
    assert_eq!(owner.record_packet(&bytes, packet(2, 1)).unwrap(), 0);
    let first = reader.pop().unwrap();
    assert_eq!(first.packet_sequence, 1);
    assert!(
        reader.pop().is_none(),
        "full-ring packets must not publish a prefix"
    );
    let mut silent = packet(2, 5);
    silent.flags = SILENT | DISCONTINUITY;
    assert_eq!(owner.record_packet(&[], silent).unwrap(), 2);
    let next = reader.pop().unwrap();
    assert_eq!(next.samples, [0.0, 0.0]);
    assert_eq!(next.packet_sequence, 3);
    assert_eq!(next.flags, SILENT | DISCONTINUITY);
    let summary = owner.snapshot();
    assert_eq!(summary.observed_frames, 5);
    assert_eq!(summary.stored_frames, 3);
    assert_eq!(summary.dropped_frames, 2);
    assert_eq!(summary.overflow_packets, 1);
    assert_eq!(summary.silent_frames, 2);
    assert_eq!(summary.gap_events, 1);
    assert_eq!(summary.gap_frames, 2);
    assert_eq!(summary.discontinuity_events, 1);
}

#[test]
fn evidence_tap_invalid_packet_is_atomic_and_invalid_time_rewarms_position() {
    let (mut owner, mut reader) = pcm_evidence_stream(config(4), 48_000, 2).unwrap();
    assert!(owner.record_packet(&[0; 7], packet(1, 0)).is_err());
    assert!(reader.pop().is_none());
    assert_eq!(owner.snapshot().packets, 0);
    assert_eq!(owner.record_packet(&[], packet(0, 999)).unwrap(), 0);
    let mut silent = packet(1, 10);
    silent.flags = SILENT;
    owner.record_packet(&[], silent).unwrap();
    silent.device_position_frames = 1000;
    silent.flags |= TIMESTAMP_ERROR;
    owner.record_packet(&[], silent).unwrap();
    silent.device_position_frames = 2000;
    silent.flags = SILENT;
    owner.record_packet(&[], silent).unwrap();
    assert_eq!(owner.snapshot().gap_events, 0);
    assert_eq!(owner.snapshot().timestamp_error_packets, 1);
    assert_eq!(reader.pop().unwrap().packet_sequence, 1);
    assert_eq!(reader.pop().unwrap().flags, SILENT | TIMESTAMP_ERROR);
}

#[test]
fn evidence_tap_explicit_position_policy_does_not_invent_process_gaps() {
    let mut settings = config(4);
    settings.detect_position_gaps = false;
    let (mut owner, _reader) = pcm_evidence_stream(settings, 48_000, 1).unwrap();
    let mut silent = packet(1, 0);
    silent.flags = SILENT;
    owner.record_packet(&[], silent).unwrap();
    owner.record_packet(&[], silent).unwrap();
    assert_eq!(owner.snapshot().gap_events, 0);
}

#[test]
fn evidence_tap_is_bounded_and_allocation_free_when_reader_is_full_or_gone() {
    let (mut owner, mut reader) = pcm_evidence_stream(config(2), 48_000, 2).unwrap();
    let mut silent = packet(2, 0);
    silent.flags = SILENT;
    common::assert_allocates_nothing(|| {
        assert_eq!(owner.record_packet(&[], silent).unwrap(), 2);
        assert_eq!(owner.record_packet(&[], silent).unwrap(), 0);
        assert!(reader.pop().is_some());
        assert!(reader.pop().is_some());
    });
    drop(reader);
    common::assert_allocates_nothing(|| {
        assert_eq!(owner.record_packet(&[], silent).unwrap(), 0);
    });
    assert_eq!(owner.snapshot().reader_abandoned_packets, 1);
    assert_eq!(owner.snapshot().dropped_frames, 4);
}

#[test]
fn evidence_tap_validates_format_and_memory_budget_before_allocating() {
    for (capacity, rate, channels) in [
        (0, 48_000, 2),
        (1, 0, 2),
        (1, 48_000, 0),
        (1, 48_000, 3),
        (usize::MAX, 48_000, 2),
        (8 * 1024 * 1024, 48_000, 2),
    ] {
        common::assert_allocates_nothing(|| {
            assert!(pcm_evidence_stream(config(capacity), rate, channels).is_err());
        });
    }
}

#[test]
fn evidence_tap_close_is_allocation_free_and_drains_before_end_of_stream() {
    let (mut owner, mut reader) = pcm_evidence_stream(config(1), 48_000, 1).unwrap();
    let mut silent = packet(1, 0);
    silent.flags = SILENT;
    owner.record_packet(&[], silent).unwrap();
    common::assert_allocates_nothing(|| owner.close());
    assert!(
        !reader.is_finished(),
        "closed still retains pending evidence"
    );
    assert!(owner.record_packet(&[], silent).is_err());
    assert!(reader.pop().is_some());
    assert!(reader.is_finished());
    assert_eq!(owner.snapshot().packets, 1);
}

#[test]
fn evidence_tap_streams_longer_than_storage_without_growing_or_losing_fifo() {
    let (mut owner, mut reader) = pcm_evidence_stream(config(16), 48_000, 1).unwrap();
    common::assert_allocates_nothing(|| {
        for index in 0..10_000u64 {
            let bytes = (index as f32).to_le_bytes();
            owner.record_packet(&bytes, packet(1, index)).unwrap();
            let frame = reader.pop().unwrap();
            assert_eq!(frame.samples, [index as f32, 0.0]);
            assert_eq!(frame.packet_sequence, index + 1);
            assert_eq!(frame.device_position_frames, index);
        }
    });
    assert_eq!(owner.snapshot().stored_frames, 10_000);
    assert_eq!(owner.snapshot().dropped_frames, 0);
}
