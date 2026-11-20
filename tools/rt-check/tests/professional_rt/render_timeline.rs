use super::{SampleFormat, measured, render_evidence::*};

#[test]
fn render_timeline_overflow_and_close_never_allocate_free_or_gate_pcm() {
    let (mut writer, mut reader) = render_evidence_stream(
        RenderEvidenceConfig {
            capacity_frames: 2,
            max_packet_frames: 2,
        },
        RenderEvidenceFormat {
            sample_rate: 48_000,
            sample_format: SampleFormat::Float32,
            channels: 2,
            container_bits: 32,
            valid_bits: 32,
        },
    )
    .unwrap();
    measured(|| {
        for _ in 0..=RENDER_EVENT_CAPACITY {
            writer.record_event(RenderEventSample {
                kind: RenderEventKind::Wait,
                qpc_before_100ns: Some(100),
                qpc_after_100ns: Some(101),
                ready: Some(false),
                ..Default::default()
            });
        }
        writer.stage_packet(&[7; 16], 2).unwrap();
        writer.finish_packet(0).unwrap();
        writer.close();
        for sequence in 1..=RENDER_EVENT_CAPACITY as u64 {
            assert_eq!(reader.pop_event().unwrap().sequence, sequence);
        }
        for _ in 0..2 {
            assert_eq!(reader.pop().unwrap().bytes[0], 7);
        }
        assert!(reader.events_finished());
        assert!(reader.is_finished());
    });
    let summary = writer.snapshot();
    assert_eq!(summary.timeline.dropped_events, 1);
    assert_eq!(summary.accepted_frames, 2);
    assert_eq!(summary.dropped_frames, 0);
    // The owned endpoints are retained until this unmeasured control scope.
}
