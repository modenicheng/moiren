use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};

mod common;
// Compile the actual private publishers under the integration counting
// allocator; the library test binary already owns a different allocator.
#[path = "bridge_live/atomic.rs"]
mod clock_bridge;
mod latency {
    pub use moiren_windows_audio::latency::InputDeliverySnapshot;
}
use common::clock::{ORIGIN, consume, packet, push};

fn bridge(
    position_clock: bool,
) -> (
    moiren_windows_audio::clock_bridge::CaptureIngress,
    moiren_windows_audio::clock_bridge::ClockSource,
    moiren_windows_audio::clock_bridge::BridgeObserver,
) {
    capture_bridge(ClockBridgeConfig {
        capacity_frames: 64,
        target_fill_frames: 2,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        detect_position_gaps: position_clock,
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn live_age_samples_oldest_pending_frame_before_transfer_not_latest_packet() {
    let (mut ingress, mut source, observer) = bridge(true);
    push(&mut ingress, 8, 0, 48_000);
    push(&mut ingress, 8, 8, 48_000);
    push(&mut ingress, 8, 16, 48_000);
    consume(&mut source, 8);
    let report = source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
        .unwrap();
    assert_eq!(report.transferred_frames, 1);
    let json = serde_json::to_value(observer.snapshot()).unwrap();
    let age = &json["oldest_frame_age"];
    assert_eq!(age["stamp"]["packet_position_frames"], 8);
    assert_eq!(age["stamp"]["packet_offset_frames"], 0);
    assert_eq!(age["stamp"]["generation"], 1);
    assert_eq!(age["input_sample_rate"], 48_000);
    assert_eq!(age["observed_qpc_100ns"], 1_020_000);
    assert_eq!(age["estimated_frame_qpc_100ns"], 1_001_667);
    assert_eq!(age["estimated_age_100ns"], 18_333);
    assert_eq!(json["last_qpc_100ns"], 1_003_334);
    assert!(json["native_delivery"].is_null());
}

#[test]
fn live_age_clears_after_missing_future_invalid_timing_and_underrun() {
    for invalid in [0, 1, 2, 3] {
        let (mut ingress, mut source, observer) = bridge(true);
        push(&mut ingress, 8, 0, 48_000);
        push(&mut ingress, 8, 8, 48_000);
        consume(&mut source, 8);
        source
            .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
            .unwrap();
        assert!(serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_object());
        match invalid {
            0 => {
                source.read_interleaved(&mut [0.0; 2]).unwrap();
            }
            1 => {
                source
                    .read_interleaved_at_qpc(&mut [0.0; 2], Some(1))
                    .unwrap();
            }
            2 => {
                source
                    .read_interleaved_at_qpc(&mut [0.0; 128], Some(ORIGIN + 20_000))
                    .unwrap();
            }
            _ => {
                ingress
                    .push_packet(
                        &[],
                        CapturePacket {
                            flags: SILENT | DISCONTINUITY | TIMESTAMP_ERROR,
                            ..packet(8, 16, 48_000)
                        },
                    )
                    .unwrap();
                source
                    .read_interleaved_at_qpc(&mut [0.0; 32], Some(ORIGIN + 20_000))
                    .unwrap();
            }
        }
        assert!(
            serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_null(),
            "case {invalid}"
        );
    }
}

#[test]
fn positionless_and_unqualified_frames_never_publish_invented_age() {
    for position_clock in [false, true] {
        let (mut ingress, mut source, observer) = bridge(position_clock);
        push(&mut ingress, 8, 0, 48_000);
        source
            .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
            .unwrap();
        assert!(serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_null());
        if !position_clock {
            push(&mut ingress, 8, 8, 48_000);
            consume(&mut source, 7);
            source
                .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
                .unwrap();
            assert!(
                serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_null()
            );
        }
    }
}

#[test]
fn live_age_publication_and_reset_allocate_and_free_nothing() {
    let (mut ingress, mut source, observer) = bridge(true);
    push(&mut ingress, 8, 0, 48_000);
    push(&mut ingress, 8, 8, 48_000);
    consume(&mut source, 8);
    let snapshot = common::assert_allocates_nothing(|| {
        source
            .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
            .unwrap();
        observer.snapshot()
    });
    assert!(serde_json::to_value(snapshot).unwrap()["oldest_frame_age"].is_object());
    common::assert_allocates_nothing(|| {
        source
            .read_interleaved_at_qpc(&mut [0.0; 128], Some(ORIGIN + 20_000))
            .unwrap();
        observer.snapshot()
    });
}

#[test]
fn live_age_becomes_unknown_on_empty_output_and_invalid_packet_correlation() {
    let (mut ingress, mut source, observer) = bridge(true);
    push(&mut ingress, 8, 0, 48_000);
    push(&mut ingress, 8, 8, 48_000);
    ingress
        .push_packet(
            &[],
            CapturePacket {
                flags: SILENT | TIMESTAMP_ERROR,
                ..packet(8, 16, 48_000)
            },
        )
        .unwrap();
    consume(&mut source, 8);
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
        .unwrap();
    assert!(serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_object());
    source
        .read_interleaved_at_qpc(&mut [], Some(ORIGIN + 20_000))
        .unwrap();
    assert!(serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_null());
    consume(&mut source, 7);
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
        .unwrap();
    assert!(serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_null());
    assert_eq!(observer.snapshot().resets, 0);
}

#[test]
fn live_age_overflow_cannot_retain_an_earlier_successful_sample() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 1024,
        target_fill_frames: 2,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    for (position, qpc) in [(0, u64::MAX - 101_000), (480, u64::MAX - 1_000)] {
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 480,
                    flags: SILENT,
                    device_position_frames: position,
                    qpc_100ns: qpc,
                },
            )
            .unwrap();
    }
    consume(&mut source, 480);
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(u64::MAX))
        .unwrap();
    assert!(serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_object());
    consume(&mut source, 4);
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(u64::MAX))
        .unwrap();
    assert!(serde_json::to_value(observer.snapshot()).unwrap()["oldest_frame_age"].is_null());
    assert_eq!(observer.snapshot().resets, 0);
}

#[test]
fn graph_follower_audio_never_implies_native_delivery_or_frame_age() {
    use moiren_engine::{
        boundary::RtAudioSink,
        buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
        processor::ProcessContext,
    };
    let (mut ingress, mut source, observer) = bridge(true);
    let mut arena = BufferArena::<f32>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 8,
        }],
        1024,
    )
    .unwrap();
    let input = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    for timeline_start in [0, 8] {
        let report = arena
            .with_io(&input, 0..8, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                ingress.write(
                    &ProcessContext {
                        timeline_epoch: 1,
                        timeline_start,
                        frames: 8,
                        processing_sr: 48_000.0,
                    },
                    inputs.get(0).unwrap(),
                )
            })
            .unwrap();
        assert_eq!(report.transferred_frames, 8);
    }
    source
        .read_interleaved_at_qpc(&mut [0.0; 16], Some(ORIGIN + 20_000))
        .unwrap();
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(ORIGIN + 20_000))
        .unwrap();
    let snapshot = observer.snapshot();
    assert_eq!(snapshot.output_frames, 9);
    assert_eq!(snapshot.native_delivery, None);
    assert_eq!(snapshot.oldest_frame_age, None);
    assert_eq!(snapshot.timestamp_errors, 0);
}

#[test]
fn live_age_keeps_the_rate_qualification_of_the_queued_frame() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 1024,
        target_fill_frames: 2,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    for index in 0..=450u64 {
        let position = index * 256;
        let qpc = ORIGIN
            + (u128::from(position) * 10_000_000 * 1_000_000 / (48_000 * 1_001_000u128)) as u64;
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 256,
                    flags: SILENT,
                    device_position_frames: position,
                    qpc_100ns: qpc,
                },
            )
            .unwrap();
        if index < 450 {
            source.read_interleaved(&mut [0.0; 512]).unwrap();
        }
    }
    consume(&mut source, 48);
    let anchor =
        ORIGIN + (450u128 * 256 * 10_000_000 * 1_000_000 / (48_000 * 1_001_000u128)) as u64;
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(anchor + 20_000))
        .unwrap();
    let age = observer.snapshot().oldest_frame_age.unwrap();
    assert_eq!(age.stamp.rate_correction_ppm, Some(1000));
    assert_eq!(age.stamp.packet_offset_frames, 48);
    assert_eq!(age.estimated_frame_qpc_100ns, anchor + 9990);
    assert_eq!(age.estimated_age_100ns, 10_010);
}
