use moiren_windows_audio::{
    clock_bridge::{CaptureIngress, CapturePacket, ClockBridgeConfig, ClockSource, capture_bridge},
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};

const ORIGIN: u64 = 1_000_000;

fn config(capacity: usize, rate: u32) -> ClockBridgeConfig {
    ClockBridgeConfig {
        input_sample_rate: rate,
        capacity_frames: capacity,
        target_fill_frames: 2,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::default()
    }
}

fn packet(frames: usize, position: u64, rate: u32) -> CapturePacket {
    CapturePacket {
        frames,
        flags: SILENT,
        device_position_frames: position,
        qpc_100ns: ORIGIN + (position * 10_000_000).div_ceil(u64::from(rate)),
    }
}

fn push(ingress: &mut CaptureIngress, frames: usize, position: u64, rate: u32) -> usize {
    ingress
        .push_packet(&[], packet(frames, position, rate))
        .unwrap()
}

fn consume(source: &mut ClockSource, frames: usize) {
    for _ in 0..frames {
        let report = source.read_interleaved(&mut [0.0; 2]).unwrap();
        assert_eq!(report.transferred_frames, 1);
        assert_eq!(report.xruns, 0);
    }
}

#[test]
fn age_query_preserves_packet_identity_and_does_not_consume_audio() {
    let (mut ingress, mut source, observer) = capture_bridge(config(32, 48_000)).unwrap();
    assert_eq!(source.oldest_frame_age(ORIGIN), None);
    push(&mut ingress, 8, 0, 48_000);
    push(&mut ingress, 8, 8, 48_000);
    assert_eq!(source.oldest_frame_age(ORIGIN + 20_000), None);
    consume(&mut source, 8);
    let now = packet(8, 8, 48_000).qpc_100ns + 10_000;
    let stamp = source.oldest_frame_stamp().unwrap();
    let age = source
        .oldest_frame_age(now)
        .expect("qualified pending frame");
    assert_eq!(age.stamp, stamp);
    assert_eq!(age.input_sample_rate, 48_000);
    assert_eq!(age.observed_qpc_100ns, now);
    assert_eq!(age.estimated_frame_qpc_100ns, stamp.packet_qpc_100ns);
    assert_eq!(age.estimated_age_100ns, 10_000);
    assert_eq!(source.oldest_frame_age(now), Some(age));
    assert_eq!(source.oldest_frame_stamp(), Some(stamp));
    assert_eq!(observer.snapshot().output_frames, 8);
    assert_eq!(observer.snapshot().resets, 0);
    consume(&mut source, 3);
    let age = source.oldest_frame_age(now).unwrap();
    assert_eq!(age.stamp.packet_offset_frames, 3);
    assert_eq!(age.estimated_frame_qpc_100ns, stamp.packet_qpc_100ns + 625);
    assert_eq!(age.estimated_age_100ns, 9_375);
}

#[test]
fn age_uses_native_offsets_at_fractional_rate_and_double_advance() {
    for rate in [44_100, 48_000] {
        let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
            max_correction_ppm: 2000.0,
            ..config(8192, rate)
        })
        .unwrap();
        push(&mut ingress, 4096, 0, rate);
        push(&mut ingress, 4096, 4096, rate);
        let now = ORIGIN + 2_000_000;
        let mut previous_position = None;
        let mut double_advances = 0;
        let mut queries = 0;
        for _ in 0..6500 {
            consume(&mut source, 1);
            let Some(stamp) = source.oldest_frame_stamp() else {
                continue;
            };
            let age = source.oldest_frame_age(now).unwrap();
            let position = stamp.packet_position_frames + stamp.packet_offset_frames;
            let offset_100ns =
                (stamp.packet_offset_frames * 10_000_000 + u64::from(rate) / 2) / u64::from(rate);
            assert_eq!(age.stamp, stamp);
            assert_eq!(
                age.estimated_frame_qpc_100ns,
                stamp.packet_qpc_100ns + offset_100ns
            );
            assert_eq!(age.estimated_age_100ns + age.estimated_frame_qpc_100ns, now);
            double_advances +=
                usize::from(previous_position.is_some_and(|previous| position == previous + 2));
            previous_position = Some(position);
            queries += 1;
        }
        assert!(queries > 2000);
        assert_eq!(observer.snapshot().resets, 0);
        if rate == 48_000 {
            assert!(double_advances > 0);
        }
    }
}

#[test]
fn age_uses_accepted_prefix_after_partial_and_full_overflow() {
    let (mut ingress, mut source, observer) = capture_bridge(config(16, 48_000)).unwrap();
    push(&mut ingress, 8, 0, 48_000);
    push(&mut ingress, 8, 8, 48_000);
    consume(&mut source, 8);
    assert_eq!(push(&mut ingress, 16, 16, 48_000), 9);
    assert_eq!(push(&mut ingress, 8, 32, 48_000), 0);
    consume(&mut source, 8);
    let now = ORIGIN + 20_000;
    let age = source.oldest_frame_age(now).unwrap();
    assert_eq!(age.stamp.packet_position_frames, 16);
    assert_eq!(age.stamp.packet_accepted_frames, 9);
    assert_eq!(age.stamp.packet_offset_frames, 0);
    assert_eq!(age.estimated_frame_qpc_100ns, 1_003_334);
    assert_eq!(age.estimated_age_100ns, 16_666);
    assert_eq!(
        observer.snapshot().last_qpc_100ns,
        packet(8, 32, 48_000).qpc_100ns
    );
}

#[test]
fn age_unavailable_for_invalid_and_positionless_clocks() {
    for position_clock in [false, true] {
        let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
            detect_position_gaps: position_clock,
            ..config(64, 48_000)
        })
        .unwrap();
        push(&mut ingress, 8, 0, 48_000);
        push(&mut ingress, 8, 8, 48_000);
        let old = packet(8, 8, 48_000).qpc_100ns;
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    flags: SILENT | TIMESTAMP_ERROR,
                    ..packet(8, 16, 48_000)
                },
            )
            .unwrap();
        push(&mut ingress, 8, 24, 48_000);
        push(&mut ingress, 8, 32, 48_000);
        consume(&mut source, 8);
        if position_clock {
            assert_eq!(
                source
                    .oldest_frame_age(old + 10_000)
                    .unwrap()
                    .estimated_age_100ns,
                10_000
            );
        } else {
            assert_eq!(source.oldest_frame_age(old + 10_000), None);
        }
        consume(&mut source, 8);
        assert_eq!(source.oldest_frame_age(ORIGIN + 20_000), None);
        consume(&mut source, 8);
        assert_eq!(source.oldest_frame_age(ORIGIN + 20_000), None);
        consume(&mut source, 8);
        assert_eq!(
            source.oldest_frame_age(ORIGIN + 20_000).is_some(),
            position_clock
        );
        assert_eq!(observer.snapshot().resets, 0);
    }
}

#[test]
fn new_generation_rewarms_without_using_old_packet_time() {
    let (mut ingress, mut source, _) = capture_bridge(config(64, 48_000)).unwrap();
    push(&mut ingress, 8, 0, 48_000);
    ingress
        .push_packet(
            &[],
            CapturePacket {
                flags: SILENT | DISCONTINUITY,
                ..packet(8, 8, 48_000)
            },
        )
        .unwrap();
    push(&mut ingress, 8, 16, 48_000);
    let report = source.read_interleaved(&mut [0.0; 16]).unwrap();
    assert_eq!(report.xruns, 1);
    assert_eq!(source.oldest_frame_age(ORIGIN + 20_000), None);
    consume(&mut source, 7);
    let age = source.oldest_frame_age(ORIGIN + 20_000).unwrap();
    assert_eq!(age.stamp.generation, 2);
    assert_eq!(age.stamp.packet_position_frames, 16);
    assert_eq!(age.estimated_frame_qpc_100ns, 1_003_334);
}

#[test]
fn future_and_overflowing_frame_times_are_unavailable() {
    let (mut ingress, mut source, _) = capture_bridge(config(32, 48_000)).unwrap();
    push(&mut ingress, 8, 0, 48_000);
    push(&mut ingress, 8, 8, 48_000);
    consume(&mut source, 8);
    let anchor = packet(8, 8, 48_000).qpc_100ns;
    assert_eq!(source.oldest_frame_age(0), None);
    assert_eq!(source.oldest_frame_age(anchor - 1), None);
    assert_eq!(
        source.oldest_frame_age(anchor).unwrap().estimated_age_100ns,
        0
    );
    consume(&mut source, 3);
    assert_eq!(source.oldest_frame_age(anchor + 624), None);
    assert_eq!(
        source
            .oldest_frame_age(anchor + 625)
            .unwrap()
            .estimated_age_100ns,
        0
    );

    let (mut ingress, mut source, _) = capture_bridge(config(1024, 48_000)).unwrap();
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
    let age = source.oldest_frame_age(u64::MAX).unwrap();
    assert_eq!(age.estimated_frame_qpc_100ns, u64::MAX - 1_000);
    assert_eq!(age.estimated_age_100ns, 1_000);
    consume(&mut source, 3);
    assert_eq!(
        source
            .oldest_frame_age(u64::MAX)
            .unwrap()
            .estimated_age_100ns,
        375
    );
    consume(&mut source, 2);
    assert_eq!(source.oldest_frame_age(u64::MAX), None);
}

#[test]
fn age_uses_the_rate_estimate_attached_to_its_queued_frame() {
    for ppm in [-1_000i64, 1_000] {
        let (mut ingress, mut source, _) = capture_bridge(config(1024, 48_000)).unwrap();
        let mut position = 0u64;
        for _ in 0..450 {
            let elapsed = u128::from(position) * 10_000_000 * 1_000_000
                / (48_000 * (1_000_000 + ppm) as u128);
            ingress
                .push_packet(
                    &[],
                    CapturePacket {
                        frames: 256,
                        flags: SILENT,
                        device_position_frames: position,
                        qpc_100ns: ORIGIN + elapsed as u64,
                    },
                )
                .unwrap();
            position += 256;
            source.read_interleaved(&mut [0.0; 512]).unwrap();
        }
        let elapsed =
            u128::from(position) * 10_000_000 * 1_000_000 / (48_000 * (1_000_000 + ppm) as u128);
        let anchor = ORIGIN + elapsed as u64;
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 256,
                    flags: SILENT,
                    device_position_frames: position,
                    qpc_100ns: anchor,
                },
            )
            .unwrap();
        consume(&mut source, 48);
        let age = source.oldest_frame_age(anchor + 20_000).unwrap();
        assert_eq!(age.stamp.rate_correction_ppm, Some(ppm));
        assert_eq!(age.stamp.packet_offset_frames, 48);
        let projected = if ppm > 0 { 9_990 } else { 10_010 };
        assert_eq!(age.estimated_frame_qpc_100ns, anchor + projected);
        assert_eq!(age.estimated_age_100ns, 20_000 - projected);
    }
}
