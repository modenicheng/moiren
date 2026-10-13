use moiren_windows_audio::{
    clock_bridge::{
        CaptureIngress, CapturePacket, ClockBridgeConfig, QueuedFrameStamp, capture_bridge,
    },
    latency::PreparedInputQueue,
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};

const ORIGIN: u64 = 1_000_000;

fn config(capacity: usize) -> ClockBridgeConfig {
    ClockBridgeConfig {
        capacity_frames: capacity,
        target_fill_frames: 4,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::default()
    }
}

fn packet(frames: usize, position: u64) -> CapturePacket {
    CapturePacket {
        frames,
        flags: SILENT,
        device_position_frames: position,
        qpc_100ns: ORIGIN + (position * 10_000_000).div_ceil(48_000),
    }
}

fn push(ingress: &mut CaptureIngress, frames: usize, position: u64) -> usize {
    ingress.push_packet(&[], packet(frames, position)).unwrap()
}

fn stamp(generation: u64, position: u64, accepted: u64, offset: u64) -> QueuedFrameStamp {
    QueuedFrameStamp {
        generation,
        packet_position_frames: position,
        packet_qpc_100ns: packet(accepted as usize, position).qpc_100ns,
        packet_accepted_frames: accepted,
        packet_offset_frames: offset,
        rate_correction_ppm: None,
    }
}

#[test]
fn queued_stamp_tracks_pending_input_after_deferred_phase_advance() {
    let (mut ingress, mut source, _) = capture_bridge(config(32)).unwrap();
    assert_eq!(source.oldest_frame_stamp(), None);
    push(&mut ingress, 8, 0);
    push(&mut ingress, 8, 8);
    assert_eq!(source.oldest_frame_stamp(), None);
    let report = source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(report.transferred_frames, 8);
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 8, 8, 0)));
    source.read_interleaved(&mut [0.0; 6]).unwrap();
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 8, 8, 3)));
    source.read_interleaved(&mut []).unwrap();
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 8, 8, 3)));
}

#[test]
fn queued_stamp_preserves_accepted_prefix_when_newer_packets_are_dropped() {
    let (mut ingress, mut source, observer) = capture_bridge(config(16)).unwrap();
    push(&mut ingress, 8, 0);
    push(&mut ingress, 8, 8);
    source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(push(&mut ingress, 16, 16), 9);
    assert_eq!(push(&mut ingress, 8, 32), 0);
    assert_eq!(observer.snapshot().dropped_frames, 15);
    assert_eq!(observer.snapshot().last_qpc_100ns, packet(8, 32).qpc_100ns);
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 8, 8, 0)));
    source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 16, 9, 0)));
    source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 16, 9, 8)));
    assert_eq!(source.read_interleaved(&mut [0.0; 2]).unwrap().xruns, 1);
    assert_eq!(source.oldest_frame_stamp(), None);
}

#[test]
fn queued_stamp_follows_startup_trim_and_running_backlog_discard() {
    let (mut ingress, mut source, _) = capture_bridge(ClockBridgeConfig {
        trim_on_prime: true,
        ..config(32)
    })
    .unwrap();
    push(&mut ingress, 8, 0);
    push(&mut ingress, 8, 8);
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 8, 8, 5)));

    let input = PreparedInputQueue {
        capacity_frames: 256,
        prime_frames: 64,
        servo_reference_frames: 64,
        low_water_frames: 48,
        high_water_frames: 240,
        modeled_minimum_frames: None,
    };
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::with_prepared_input(48_000, 2, input)
    })
    .unwrap();
    push(&mut ingress, 64, 0);
    push(&mut ingress, 128, 64);
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    push(&mut ingress, 192, 192);
    let report = source.read_interleaved(&mut [0.0; 2]).unwrap();
    assert!(report.discontinuity);
    assert_eq!(report.xruns, 0);
    assert_eq!(observer.snapshot().backlog_recoveries, 1);
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 192, 192, 129)));
}

#[test]
fn queued_stamp_rewarms_in_new_generation_without_relabeling_old_frames() {
    let (mut ingress, mut source, _) = capture_bridge(config(32)).unwrap();
    push(&mut ingress, 8, 0);
    let mut changed = packet(8, 8);
    changed.flags |= DISCONTINUITY;
    ingress.push_packet(&[], changed).unwrap();
    push(&mut ingress, 8, 16);
    let report = source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(report.xruns, 1);
    assert_eq!(source.oldest_frame_stamp(), None);
    source.read_interleaved(&mut [0.0; 14]).unwrap();
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(2, 16, 8, 0)));
}

#[test]
fn invalid_packet_clocks_remove_timing_without_breaking_audio_or_position_tracking() {
    for (qpc, flags) in [
        (0, SILENT),
        (packet(8, 16).qpc_100ns, SILENT | TIMESTAMP_ERROR),
        (packet(8, 8).qpc_100ns, SILENT),
        (ORIGIN - 1, SILENT),
        (ORIGIN + 1_000_000, SILENT),
    ] {
        let (mut ingress, mut source, observer) = capture_bridge(config(64)).unwrap();
        push(&mut ingress, 8, 0);
        push(&mut ingress, 8, 8);
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    qpc_100ns: qpc,
                    flags,
                    ..packet(8, 16)
                },
            )
            .unwrap();
        push(&mut ingress, 8, 24);
        push(&mut ingress, 8, 32);
        let report = source.read_interleaved(&mut [0.0; 32]).unwrap();
        assert_eq!(report.transferred_frames, 16);
        assert_eq!(report.xruns, 0);
        assert_eq!(source.oldest_frame_stamp(), None);
        source.read_interleaved(&mut [0.0; 16]).unwrap();
        assert_eq!(source.oldest_frame_stamp(), None);
        source.read_interleaved(&mut [0.0; 16]).unwrap();
        assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 32, 8, 0)));
        assert_eq!(observer.snapshot().resets, 0);
    }
}

#[test]
fn zero_position_virtual_sources_always_keep_fifo_mode() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        detect_position_gaps: false,
        ..config(64)
    })
    .unwrap();
    for position in [0, 8, 16, 24] {
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    device_position_frames: 0,
                    ..packet(8, position)
                },
            )
            .unwrap();
    }
    let report = source.read_interleaved(&mut [0.0; 48]).unwrap();
    assert_eq!(report.transferred_frames, 24);
    assert_eq!(report.xruns, 0);
    assert_eq!(source.oldest_frame_stamp(), None);
    assert_eq!(observer.snapshot().discontinuities, 0);
}

#[test]
fn empty_and_invalid_payload_packets_do_not_replace_the_clock_anchor() {
    let (mut ingress, mut source, _) = capture_bridge(config(32)).unwrap();
    push(&mut ingress, 8, 0);
    let mut empty = packet(0, 8);
    empty.qpc_100ns = 0;
    ingress.push_packet(&[], empty).unwrap();
    let mut invalid = packet(8, 8);
    invalid.flags = 0;
    assert!(ingress.push_packet(&[0; 7], invalid).is_err());
    push(&mut ingress, 8, 8);
    source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(source.oldest_frame_stamp(), Some(stamp(1, 8, 8, 0)));
}

#[test]
fn bridge_preparation_accounts_for_sample_and_stamp_storage() {
    assert!(
        capture_bridge(ClockBridgeConfig {
            byte_budget: 32 * 16,
            ..config(32)
        })
        .is_err()
    );
}

#[test]
fn queued_stamp_matches_fractional_phase_and_two_frame_advancement() {
    for input_rate in [44_100, 48_000] {
        let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
            input_sample_rate: input_rate,
            max_correction_ppm: 2000.0,
            ..config(8192)
        })
        .unwrap();
        for position in [0u64, 4096] {
            let payload: Vec<_> = (position..position + 4096)
                .flat_map(|frame| {
                    let value = (frame as f32).to_le_bytes();
                    value.into_iter().chain(value)
                })
                .collect();
            ingress
                .push_packet(
                    &payload,
                    CapturePacket {
                        frames: 4096,
                        flags: 0,
                        device_position_frames: position,
                        qpc_100ns: ORIGIN + (position * 10_000_000).div_ceil(u64::from(input_rate)),
                    },
                )
                .unwrap();
        }
        let mut previous_position = None;
        let mut double_advances = 0;
        for _ in 0..6500 {
            let mut output = [0.0; 2];
            assert_eq!(
                source
                    .read_interleaved(&mut output)
                    .unwrap()
                    .transferred_frames,
                1
            );
            let Some(stamp) = source.oldest_frame_stamp() else {
                continue;
            };
            let position = stamp.packet_position_frames + stamp.packet_offset_frames;
            let step = f64::from(input_rate) / 48_000.0
                * (1.0 + observer.snapshot().correction_ppm / 1_000_000.0);
            let next = f64::from(output[0]) + step;
            if next.fract() > 0.001 && next.fract() < 0.999 {
                assert_eq!(position, next.floor() as u64);
            }
            if previous_position.is_some_and(|previous| position == previous + 2) {
                double_advances += 1;
            }
            previous_position = Some(position);
            assert!(stamp.packet_offset_frames < stamp.packet_accepted_frames);
        }
        if input_rate == 48_000 {
            assert!(double_advances > 0);
        }
    }
}

#[test]
fn quantized_single_frame_packet_stamps_accept_positive_and_negative_drift() {
    for input_rate in [44_100, 48_000] {
        for drift_ppm in [-1000, 0, 1000] {
            let (mut ingress, mut source, _) = capture_bridge(ClockBridgeConfig {
                input_sample_rate: input_rate,
                ..config(64)
            })
            .unwrap();
            for position in 0..32u64 {
                let elapsed = (u128::from(position) * 10_000_000 * 1_000_000)
                    .div_ceil(u128::from(input_rate) * (1_000_000 + drift_ppm) as u128)
                    as u64;
                ingress
                    .push_packet(
                        &[],
                        CapturePacket {
                            frames: 1,
                            flags: SILENT,
                            device_position_frames: position,
                            qpc_100ns: ORIGIN + elapsed,
                        },
                    )
                    .unwrap();
            }
            source.read_interleaved(&mut [0.0; 16]).unwrap();
            let stamp = source.oldest_frame_stamp().unwrap();
            assert_eq!(stamp.packet_accepted_frames, 1);
            assert_eq!(stamp.packet_offset_frames, 0);
            assert_eq!(
                stamp.packet_position_frames,
                if input_rate == 44_100 { 7 } else { 8 }
            );
        }
    }
}

#[test]
fn overflowing_native_position_never_produces_a_wrapped_stamp() {
    let (mut ingress, mut source, _) = capture_bridge(config(32)).unwrap();
    for index in 0..3 {
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 8,
                    flags: SILENT,
                    device_position_frames: u64::MAX - 23 + index * 8,
                    qpc_100ns: ORIGIN + (index * 8 * 10_000_000).div_ceil(48_000),
                },
            )
            .unwrap();
    }
    source.read_interleaved(&mut [0.0; 16]).unwrap();
    let warmed = source.oldest_frame_stamp().unwrap();
    assert_eq!(warmed.packet_position_frames, u64::MAX - 15);
    assert_eq!(warmed.packet_offset_frames, 0);
    let report = source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(report.transferred_frames, 8);
    assert_eq!(report.xruns, 0);
    assert_eq!(source.oldest_frame_stamp(), None);
}
