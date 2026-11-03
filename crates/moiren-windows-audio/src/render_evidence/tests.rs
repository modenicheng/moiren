use super::*;
use crate::{codec::SampleCodec, test_allocation::assert_allocates_nothing};
use moiren_engine::audio_config::SampleFormat;

fn config(capacity_frames: usize, max_packet_frames: usize) -> RenderEvidenceConfig {
    RenderEvidenceConfig {
        capacity_frames,
        max_packet_frames,
    }
}

fn format() -> RenderEvidenceFormat {
    RenderEvidenceFormat {
        sample_rate: 48_000,
        sample_format: SampleFormat::Float32,
        channels: 2,
        container_bits: 32,
        valid_bits: 32,
    }
}

#[test]
fn snapshot_exposes_exact_layout_config_and_initial_empty_stream() {
    let (writer, mut reader) = render_evidence_stream(config(4, 2), format()).unwrap();
    let summary = writer.snapshot();
    assert_eq!(summary.config.capacity_frames, 4);
    assert_eq!(summary.config.max_packet_frames, 2);
    assert_eq!(summary.format.sample_rate, 48_000);
    assert_eq!(summary.format.sample_format, SampleFormat::Float32);
    assert_eq!(summary.format.channels, 2);
    assert_eq!(summary.format.container_bits, 32);
    assert_eq!(summary.format.valid_bits, 32);
    assert_eq!(summary.attempts, 0);
    assert_eq!(summary.accepted_frames, 0);
    assert_eq!(summary.rejected_frames, 0);
    assert_eq!(summary.stored_frames, 0);
    assert_eq!(summary.dropped_frames, 0);
    assert_eq!(summary.invalid_attempts, 0);
    assert!(!summary.counters_saturated);
    assert!(reader.pop().is_none());
    assert!(!reader.is_finished());
}

#[test]
fn layout_validation_matches_sample_codec_for_mono_and_stereo_only() {
    for sample_format in SampleFormat::ALL {
        for channels in [1, 2] {
            for container_bits in [0, 8, 16, 24, 32, 64] {
                for valid_bits in [0, 1, 16, 20, 24, 32, 64] {
                    let layout = RenderEvidenceFormat {
                        sample_format,
                        channels,
                        container_bits,
                        valid_bits,
                        ..format()
                    };
                    let expected = SampleCodec::with_layout(
                        sample_format,
                        channels,
                        container_bits,
                        valid_bits,
                    )
                    .is_ok();
                    assert_eq!(
                        render_evidence_stream(config(1, 1), layout).is_ok(),
                        expected,
                        "{layout:?}"
                    );
                }
            }
        }
    }
    for channels in [0, 3, 32, usize::MAX] {
        assert_allocates_nothing(|| {
            assert!(
                render_evidence_stream(
                    config(1, 1),
                    RenderEvidenceFormat {
                        channels,
                        ..format()
                    }
                )
                .is_err()
            );
        });
    }
}

#[test]
fn invalid_caps_rate_and_arithmetic_overflow_refuse_before_allocation() {
    for settings in [
        config(0, 1),
        config(1, 0),
        config(1, 2),
        config(65_537, 65_537),
        config(usize::MAX, 1),
    ] {
        assert_allocates_nothing(|| {
            assert!(render_evidence_stream(settings, format()).is_err());
        });
    }
    assert_allocates_nothing(|| {
        assert!(
            render_evidence_stream(
                config(1, 1),
                RenderEvidenceFormat {
                    sample_rate: 0,
                    ..format()
                }
            )
            .is_err()
        );
    });
    assert!(render_evidence_stream(config(65_536, 65_536), format()).is_ok());
}

#[test]
fn eight_mib_budget_includes_ring_frames_and_encoded_staging() {
    let budget = 8 * 1024 * 1024;
    let staging_bytes = 65_536 * 16;
    let capacity = (budget - staging_bytes) / size_of::<RenderEvidenceFrame>();
    let layout = RenderEvidenceFormat {
        sample_format: SampleFormat::Float64,
        container_bits: 64,
        valid_bits: 64,
        ..format()
    };
    assert!(render_evidence_stream(config(capacity, 65_536), layout).is_ok());
    assert_allocates_nothing(|| {
        assert!(render_evidence_stream(config(capacity + 1, 65_536), layout).is_err());
    });
}

#[test]
fn staging_copies_before_release_and_publishes_only_on_finish() {
    let (mut writer, mut reader) = render_evidence_stream(config(2, 2), format()).unwrap();
    let original = [
        0x34, 0x12, 0xc0, 0x7f, 0x00, 0x00, 0x80, 0xff, 0xff, 0xff, 0x7f, 0x7f, 0x00, 0x00, 0x00,
        0x80,
    ];
    let mut lease = original;
    writer.stage_packet(&lease, 2).unwrap();
    lease.fill(0x55);
    assert!(reader.pop().is_none());
    assert_eq!(writer.snapshot().attempts, 0);
    assert_eq!(writer.finish_packet(0).unwrap(), 2);
    for offset in 0..2 {
        let frame = reader.pop().unwrap();
        assert_eq!(&frame.bytes[..8], &original[offset * 8..offset * 8 + 8]);
        assert_eq!(&frame.bytes[8..], &[0; 8]);
        assert_eq!(frame.packet_sequence, 1);
        assert_eq!(frame.packet_frame_offset, offset as u32);
        assert_eq!(frame.packet_frames, 2);
        assert_eq!(frame.release_hresult, 0);
    }
    assert_eq!(lease, [0x55; 16]);
    assert!(reader.pop().is_none());
}

#[test]
fn exact_container_bytes_are_preserved_including_padding_and_float_bits() {
    for (sample_format, container_bits, valid_bits, width) in [
        (SampleFormat::Pcm16, 16, 16, 2),
        (SampleFormat::Pcm24, 24, 24, 3),
        (SampleFormat::Pcm24, 32, 24, 4),
        (SampleFormat::Pcm32, 32, 1, 4),
        (SampleFormat::Pcm32, 32, 20, 4),
        (SampleFormat::Pcm32, 32, 32, 4),
        (SampleFormat::Float32, 32, 32, 4),
        (SampleFormat::Float64, 64, 64, 8),
    ] {
        for channels in [1, 2] {
            let layout = RenderEvidenceFormat {
                sample_format,
                channels,
                container_bits,
                valid_bits,
                ..format()
            };
            let (mut writer, mut reader) = render_evidence_stream(config(1, 1), layout).unwrap();
            let bytes = [
                0xff, 0x81, 0xa5, 0x7f, 0x00, 0xff, 0xf0, 0x7f, 0x01, 0x80, 0xfe, 0x00, 0x33, 0xcc,
                0x55, 0xaa,
            ];
            let used = width * channels;
            writer.stage_packet(&bytes[..used], 1).unwrap();
            assert_eq!(writer.finish_packet(0).unwrap(), 1);
            let frame = reader.pop().unwrap();
            assert_eq!(&frame.bytes[..used], &bytes[..used]);
            assert!(frame.bytes[used..].iter().all(|byte| *byte == 0));
        }
    }
}

#[test]
fn release_hresult_sign_records_both_accepted_and_rejected_attempts() {
    let (mut writer, mut reader) = render_evidence_stream(config(1, 1), format()).unwrap();
    for (index, hr) in [0, 1, i32::MAX, -1, i32::MIN].into_iter().enumerate() {
        writer.stage_packet(&[0xa5; 8], 1).unwrap();
        assert_eq!(writer.finish_packet(hr).unwrap(), 1);
        let frame = reader.pop().unwrap();
        assert_eq!(frame.packet_sequence, index as u64 + 1);
        assert_eq!(frame.release_hresult, hr);
        assert_eq!(&frame.bytes[..8], &[0xa5; 8]);
    }
    let summary = writer.snapshot();
    assert_eq!(summary.attempts, 5);
    assert_eq!(summary.accepted_frames, 3);
    assert_eq!(summary.rejected_frames, 2);
    assert_eq!(summary.stored_frames, 5);
    assert_eq!(summary.dropped_frames, 0);
}

#[test]
fn malformed_and_nested_stages_do_not_overwrite_the_pending_packet() {
    let (mut writer, mut reader) = render_evidence_stream(config(2, 2), format()).unwrap();
    for (bytes, frames) in [
        (&[0; 7][..], 1),
        (&[0; 9][..], 1),
        (&[][..], 0),
        (&[0; 24][..], 3),
        (&[][..], usize::MAX),
    ] {
        assert!(writer.stage_packet(bytes, frames).is_err());
    }
    assert!(reader.pop().is_none());
    assert_eq!(writer.snapshot().attempts, 0);
    writer.stage_packet(&[0x33; 8], 1).unwrap();
    assert!(writer.stage_packet(&[0x44; 16], 2).is_err());
    assert_eq!(writer.finish_packet(0).unwrap(), 1);
    assert_eq!(&reader.pop().unwrap().bytes[..8], &[0x33; 8]);
    assert_eq!(writer.snapshot().invalid_attempts, 6);
    assert_eq!(writer.snapshot().attempts, 1);
}

#[test]
fn finish_without_stage_and_double_finish_do_not_republish() {
    let (mut writer, mut reader) = render_evidence_stream(config(2, 1), format()).unwrap();
    assert!(writer.finish_packet(0).is_err());
    writer.stage_packet(&[0; 8], 1).unwrap();
    assert_eq!(writer.finish_packet(-1).unwrap(), 1);
    assert!(writer.finish_packet(0).is_err());
    assert_eq!(reader.pop().unwrap().release_hresult, -1);
    assert!(reader.pop().is_none());
    assert_eq!(writer.snapshot().attempts, 1);
    assert_eq!(writer.snapshot().rejected_frames, 1);
    assert_eq!(writer.snapshot().accepted_frames, 0);
    assert_eq!(writer.snapshot().invalid_attempts, 2);
}

#[test]
fn full_ring_drops_whole_packet_but_preserves_outcome_and_sequence_gap() {
    let (mut writer, mut reader) = render_evidence_stream(config(3, 2), format()).unwrap();
    writer.stage_packet(&[1; 16], 2).unwrap();
    assert_eq!(writer.finish_packet(0).unwrap(), 2);
    writer.stage_packet(&[2; 16], 2).unwrap();
    assert_eq!(writer.finish_packet(-7).unwrap(), 0);
    assert_eq!(reader.pop().unwrap().packet_sequence, 1);
    assert_eq!(reader.pop().unwrap().packet_sequence, 1);
    assert!(reader.pop().is_none(), "overflow must not publish a prefix");
    writer.stage_packet(&[3; 8], 1).unwrap();
    writer.finish_packet(0).unwrap();
    assert_eq!(reader.pop().unwrap().packet_sequence, 3);
    let summary = writer.snapshot();
    assert_eq!(summary.attempts, 3);
    assert_eq!(summary.accepted_frames, 3);
    assert_eq!(summary.rejected_frames, 2);
    assert_eq!(summary.stored_frames, 3);
    assert_eq!(summary.dropped_frames, 2);
    assert_eq!(summary.overflow_packets, 1);
    assert_eq!(summary.reader_abandoned_packets, 0);
}

#[test]
fn fifo_across_wrap_does_not_reuse_previous_packet_tail() {
    let (mut writer, mut reader) = render_evidence_stream(config(3, 2), format()).unwrap();
    writer.stage_packet(&[1; 16], 2).unwrap();
    writer.finish_packet(0).unwrap();
    assert_eq!(reader.pop().unwrap().packet_frame_offset, 0);
    writer.stage_packet(&[2; 16], 2).unwrap();
    writer.finish_packet(5).unwrap();
    let tail = reader.pop().unwrap();
    assert_eq!(tail.packet_sequence, 1);
    assert_eq!(tail.packet_frame_offset, 1);
    for offset in 0..2 {
        let frame = reader.pop().unwrap();
        assert_eq!(frame.packet_sequence, 2);
        assert_eq!(frame.packet_frame_offset, offset);
        assert_eq!(frame.release_hresult, 5);
        assert_eq!(&frame.bytes[..8], &[2; 8]);
        assert_eq!(&frame.bytes[8..], &[0; 8]);
    }
    assert!(reader.pop().is_none());
}

#[test]
fn abandoned_reader_drops_evidence_without_losing_actual_release_accounting() {
    let (mut writer, reader) = render_evidence_stream(config(2, 2), format()).unwrap();
    writer.stage_packet(&[0; 16], 2).unwrap();
    drop(reader);
    assert_allocates_nothing(|| {
        assert_eq!(writer.finish_packet(-42).unwrap(), 0);
        writer.stage_packet(&[0; 8], 1).unwrap();
        assert_eq!(writer.finish_packet(0).unwrap(), 0);
    });
    let summary = writer.snapshot();
    assert_eq!(summary.attempts, 2);
    assert_eq!(summary.accepted_frames, 1);
    assert_eq!(summary.rejected_frames, 2);
    assert_eq!(summary.dropped_frames, 3);
    assert_eq!(summary.stored_frames, 0);
    assert_eq!(summary.reader_abandoned_packets, 2);
    assert_eq!(summary.overflow_packets, 0);
}

#[test]
fn close_abandons_unreleased_stage_without_inventing_a_release_outcome() {
    let (mut writer, mut reader) = render_evidence_stream(config(2, 2), format()).unwrap();
    writer.stage_packet(&[0; 16], 2).unwrap();
    assert_allocates_nothing(|| {
        writer.close();
        writer.close();
    });
    let summary = writer.snapshot();
    assert_eq!(summary.attempts, 0);
    assert_eq!(summary.accepted_frames, 0);
    assert_eq!(summary.rejected_frames, 0);
    assert_eq!(summary.stored_frames, 0);
    assert_eq!(summary.pending_abandoned_packets, 1);
    assert_eq!(summary.pending_abandoned_frames, 2);
    assert_eq!(summary.invalid_attempts, 1);
    assert!(reader.pop().is_none());
    assert!(reader.is_finished());
    assert!(writer.finish_packet(0).is_err());
    assert!(writer.stage_packet(&[0; 8], 1).is_err());
    assert_eq!(writer.snapshot().attempts, 0);
}

#[test]
fn close_drains_published_frames_before_reporting_finished() {
    let (mut writer, mut reader) = render_evidence_stream(config(2, 2), format()).unwrap();
    writer.stage_packet(&[0; 16], 2).unwrap();
    writer.finish_packet(0).unwrap();
    writer.close();
    assert!(!reader.is_finished());
    assert!(reader.pop().is_some());
    assert!(!reader.is_finished());
    assert!(reader.pop().is_some());
    assert!(reader.is_finished());
    assert!(reader.pop().is_none());
    assert_eq!(writer.snapshot().attempts, 1);
    assert_eq!(writer.snapshot().pending_abandoned_packets, 0);
}

#[test]
fn control_side_writer_drop_closes_reader_without_publishing_pending_bytes() {
    let (mut writer, mut reader) = render_evidence_stream(config(1, 1), format()).unwrap();
    writer.stage_packet(&[0; 8], 1).unwrap();
    drop(writer);
    assert!(reader.is_finished());
    assert!(reader.pop().is_none());
}

#[test]
fn hot_paths_allocate_and_free_nothing_including_errors_overflow_and_close() {
    let (mut writer, mut reader) = render_evidence_stream(config(2, 2), format()).unwrap();
    assert_allocates_nothing(|| {
        assert!(writer.finish_packet(0).is_err());
        assert!(writer.stage_packet(&[0; 7], 1).is_err());
        writer.stage_packet(&[0; 16], 2).unwrap();
        assert!(writer.stage_packet(&[1; 8], 1).is_err());
        assert_eq!(writer.finish_packet(0).unwrap(), 2);
        writer.stage_packet(&[2; 8], 1).unwrap();
        assert_eq!(writer.finish_packet(-1).unwrap(), 0);
        assert!(reader.pop().is_some());
        assert!(reader.pop().is_some());
        for _ in 0..10_000 {
            writer.stage_packet(&[3; 8], 1).unwrap();
            assert_eq!(writer.finish_packet(0).unwrap(), 1);
            assert_eq!(reader.pop().unwrap().bytes[0], 3);
        }
        let _ = writer.snapshot();
        writer.close();
        assert!(reader.is_finished());
        assert!(writer.finish_packet(0).is_err());
        assert!(writer.stage_packet(&[0; 8], 1).is_err());
    });
    assert_eq!(writer.snapshot().stored_frames, 10_002);
    assert_eq!(writer.snapshot().dropped_frames, 1);
}

#[test]
fn counter_overflow_saturates_and_is_visible_without_wrapping() {
    let (mut writer, mut reader) = render_evidence_stream(config(1, 1), format()).unwrap();
    writer.summary.accepted_frames = u64::MAX;
    writer.stage_packet(&[0; 8], 1).unwrap();
    writer.finish_packet(0).unwrap();
    assert_eq!(writer.snapshot().accepted_frames, u64::MAX);
    assert!(writer.snapshot().counters_saturated);
    assert_eq!(reader.pop().unwrap().packet_sequence, 1);
    writer.summary.invalid_attempts = u64::MAX;
    assert!(writer.finish_packet(0).is_err());
    assert_eq!(writer.snapshot().invalid_attempts, u64::MAX);
    assert!(writer.snapshot().counters_saturated);
}

#[test]
fn exhausted_sequence_refuses_stage_and_finish_without_reusing_identity() {
    let (mut writer, mut reader) = render_evidence_stream(config(1, 1), format()).unwrap();
    writer.summary.attempts = u64::MAX;
    assert_allocates_nothing(|| {
        assert_eq!(
            writer.stage_packet(&[0; 8], 1),
            Err(RenderEvidenceError::SequenceExhausted)
        );
        assert!(writer.finish_packet(0).is_err());
    });
    let summary = writer.snapshot();
    assert_eq!(summary.attempts, u64::MAX);
    assert_eq!(summary.accepted_frames, 0);
    assert_eq!(summary.rejected_frames, 0);
    assert_eq!(summary.stored_frames, 0);
    assert!(summary.sequence_exhausted);
    assert!(summary.counters_saturated);
    assert!(reader.pop().is_none());
    writer.close();
    assert!(writer.snapshot().sequence_exhausted);
    assert!(writer.snapshot().counters_saturated);
    assert!(reader.is_finished());
}

#[test]
fn last_sequence_is_published_once_then_exhaustion_remains_sticky() {
    let (mut writer, mut reader) = render_evidence_stream(config(1, 1), format()).unwrap();
    writer.summary.attempts = u64::MAX - 1;
    writer.stage_packet(&[7; 8], 1).unwrap();
    assert_eq!(writer.finish_packet(-1).unwrap(), 1);
    assert_eq!(reader.pop().unwrap().packet_sequence, u64::MAX);
    for _ in 0..2 {
        assert_eq!(
            writer.stage_packet(&[8; 8], 1),
            Err(RenderEvidenceError::SequenceExhausted)
        );
        assert!(writer.finish_packet(0).is_err());
        assert_eq!(writer.snapshot().attempts, u64::MAX);
        assert!(writer.snapshot().counters_saturated);
        assert!(writer.snapshot().sequence_exhausted);
        assert!(reader.pop().is_none());
    }
    assert_eq!(writer.snapshot().rejected_frames, 1);
    assert_eq!(writer.snapshot().accepted_frames, 0);
    assert_eq!(writer.snapshot().stored_frames, 1);
}

#[test]
fn frames_are_copy_and_serialize_exact_bytes_and_signed_release_result() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<RenderEvidenceFrame>();
    let (mut writer, mut reader) = render_evidence_stream(config(1, 1), format()).unwrap();
    writer.stage_packet(&[0xff; 8], 1).unwrap();
    writer.finish_packet(-123).unwrap();
    let frame = reader.pop().unwrap();
    let json = serde_json::to_value(frame).unwrap();
    assert_eq!(json["packet_sequence"], 1);
    assert_eq!(json["packet_frame_offset"], 0);
    assert_eq!(json["packet_frames"], 1);
    assert_eq!(json["release_hresult"], -123);
    assert_eq!(json["bytes"][0], 255);
    assert_eq!(json["bytes"][8], 0);
    let summary = serde_json::to_value(writer.snapshot()).unwrap();
    assert_eq!(summary["format"]["sample_format"], "float32");
    assert_eq!(summary["config"]["max_packet_frames"], 1);
}
