use moiren_windows_audio::{
    clock_bridge::{CaptureIngress, CapturePacket, ClockBridgeConfig, ClockSource, capture_bridge},
    latency::PreparedInputQueue,
    stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR},
};

const ORIGIN: u64 = 1_000_000;

fn packet(position: u64, rate: u32, ppm: i64) -> CapturePacket {
    let numerator = u128::from(position) * 10_000_000 * 1_000_000;
    let denominator = u128::from(rate) * (1_000_000i64 + ppm) as u128;
    CapturePacket {
        frames: 256,
        flags: SILENT,
        device_position_frames: position,
        qpc_100ns: ORIGIN + (numerator / denominator) as u64,
    }
}

fn bridge(rate: u32, position_clock: bool) -> (CaptureIngress, ClockSource) {
    let (ingress, source, _) = capture_bridge(ClockBridgeConfig {
        input_sample_rate: rate,
        capacity_frames: 1_024,
        target_fill_frames: 512,
        trim_on_prime: false,
        detect_position_gaps: position_clock,
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    (ingress, source)
}

fn consume_available(source: &mut ClockSource) {
    let mut output = [0.0; 2];
    for _ in 0..400 {
        if source.read_interleaved(&mut output).unwrap().xruns > 0 {
            break;
        }
    }
}

fn advance(
    ingress: &mut CaptureIngress,
    source: &mut ClockSource,
    position: &mut u64,
    rate: u32,
    ppm: i64,
    packets: usize,
) -> Option<i64> {
    let mut last = None;
    for _ in 0..packets {
        let next = packet(*position, rate, ppm);
        assert_eq!(ingress.push_packet(&[], next).unwrap(), next.frames);
        *position += next.frames as u64;
        last = source
            .oldest_frame_stamp()
            .and_then(|stamp| stamp.rate_correction_ppm);
        consume_available(source);
    }
    last
}

#[test]
fn rate_estimate_waits_for_two_long_consistent_windows() {
    for rate in [44_100, 48_000] {
        for ppm in [-1_000, 0, 1_000] {
            let (mut ingress, mut source) = bridge(rate, true);
            let mut position = 0;
            let estimate = advance(&mut ingress, &mut source, &mut position, rate, ppm, 160);
            assert_eq!(estimate, None, "rate={rate}, ppm={ppm}");
            let estimate = advance(&mut ingress, &mut source, &mut position, rate, ppm, 280)
                .expect("two consistent windows publish the queued estimate");
            assert!(
                (estimate - ppm).abs() <= 1,
                "rate={rate}, ppm={ppm}, estimate={estimate}"
            );
        }
    }
}

#[test]
fn bad_packet_clock_revokes_future_estimates_and_requires_rewarming() {
    for flags in [TIMESTAMP_ERROR, DISCONTINUITY] {
        let (mut ingress, mut source) = bridge(48_000, true);
        let mut position = 0;
        assert_eq!(
            advance(&mut ingress, &mut source, &mut position, 48_000, 1_000, 440),
            Some(1_000)
        );
        let mut invalid = packet(position, 48_000, 1_000);
        invalid.flags |= flags;
        ingress.push_packet(&[], invalid).unwrap();
        position += 256;
        consume_available(&mut source);
        assert_eq!(
            advance(&mut ingress, &mut source, &mut position, 48_000, 1_000, 160),
            None
        );
        assert_eq!(
            advance(&mut ingress, &mut source, &mut position, 48_000, 1_000, 280),
            Some(1_000)
        );
    }
}

#[test]
fn implausible_absolute_rate_never_becomes_feed_forward() {
    for ppm in [-5_000, 5_000] {
        let (mut ingress, mut source) = bridge(48_000, true);
        let mut position = 0;
        assert_eq!(
            advance(&mut ingress, &mut source, &mut position, 48_000, ppm, 640),
            None
        );
    }
}

#[test]
fn disabled_position_clock_never_publishes_a_rate() {
    let (mut ingress, mut source) = bridge(48_000, false);
    let mut position = 0;
    assert_eq!(
        advance(&mut ingress, &mut source, &mut position, 48_000, 1_000, 440),
        None
    );
    assert_eq!(source.oldest_frame_stamp(), None);
}

#[test]
fn prepared_feed_forward_uses_queued_rate_and_clamps_total_correction() {
    for (ppm, limit, low_water, live, expected) in [
        (1_000, 2_000.0, 48, true, 1_000.0),
        (-1_000, 2_000.0, 48, true, -1_000.0),
        (1_000, 500.0, 48, true, 500.0),
        (1_000, 500.0, 256, true, -500.0),
        (1_000, 2_000.0, 48, false, 0.0),
    ] {
        let input = PreparedInputQueue {
            capacity_frames: 8_192,
            prime_frames: 256,
            servo_reference_frames: 256,
            low_water_frames: low_water,
            high_water_frames: 8_000,
            modeled_minimum_frames: None,
        };
        let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
            max_correction_ppm: limit,
            trim_on_prime: live,
            ..ClockBridgeConfig::with_prepared_input(48_000, 2, input)
        })
        .unwrap();
        let mut position = 0;
        assert_eq!(
            advance(&mut ingress, &mut source, &mut position, 48_000, ppm, 440),
            Some(ppm)
        );
        ingress
            .push_packet(&[], packet(position, 48_000, ppm))
            .unwrap();
        let report = source.read_interleaved(&mut [0.0; 2]).unwrap();
        assert_eq!(report.transferred_frames, 1);
        assert_eq!(report.xruns, 0);
        assert_eq!(
            source.oldest_frame_stamp().unwrap().rate_correction_ppm,
            Some(ppm)
        );
        assert!((observer.snapshot().correction_ppm - expected).abs() < 0.001);
    }
}

#[test]
fn newly_rejected_packet_cannot_relabel_old_queued_rate() {
    let (mut ingress, mut source) = bridge(48_000, true);
    let mut position = 0;
    assert_eq!(
        advance(&mut ingress, &mut source, &mut position, 48_000, 1_000, 440),
        Some(1_000)
    );
    ingress
        .push_packet(&[], packet(position, 48_000, 1_000))
        .unwrap();
    let queued = source.oldest_frame_stamp().unwrap();
    assert_eq!(queued.rate_correction_ppm, Some(1_000));
    let mut invalid = packet(position + 256, 48_000, 1_000);
    invalid.flags |= TIMESTAMP_ERROR;
    invalid.frames = 1_024;
    assert!(ingress.push_packet(&[], invalid).unwrap() < invalid.frames);
    assert_eq!(source.oldest_frame_stamp(), Some(queued));
    invalid.device_position_frames += invalid.frames as u64;
    assert_eq!(ingress.push_packet(&[], invalid).unwrap(), 0);
    assert_eq!(source.oldest_frame_stamp(), Some(queued));
}

#[test]
fn quantized_jitter_and_large_absolute_qpc_preserve_long_rate_estimate() {
    let (mut ingress, mut source) = bridge(44_100, true);
    let mut estimate = None;
    for index in 0..440 {
        let mut next = packet(index * 256, 44_100, -1_000);
        next.qpc_100ns += u64::MAX / 2 + if index % 2 == 0 { 20 } else { 0 };
        ingress.push_packet(&[], next).unwrap();
        estimate = source
            .oldest_frame_stamp()
            .and_then(|stamp| stamp.rate_correction_ppm);
        consume_available(&mut source);
    }
    assert!(estimate.is_some_and(|ppm| (ppm + 1_000).abs() <= 3));
}

#[test]
fn disagreeing_windows_revoke_rate_until_two_new_windows_agree() {
    let (mut ingress, mut source) = bridge(48_000, true);
    let transition = 440 * 256;
    for index in 0..1_040 {
        let position = index * 256;
        let mut next = packet(position, 48_000, 0);
        if position >= transition {
            next.qpc_100ns = ORIGIN
                + transition * 10_000_000 / 48_000
                + (((position - transition) as u128 * 10_000_000 * 1_000_000)
                    / (48_000 * 1_001_000u128)) as u64;
        }
        ingress.push_packet(&[], next).unwrap();
        let estimate = source
            .oldest_frame_stamp()
            .and_then(|stamp| stamp.rate_correction_ppm);
        match index {
            400 => assert_eq!(estimate, Some(0)),
            600 => assert_eq!(estimate, None),
            1_000 => assert_eq!(estimate, Some(1_000)),
            _ => {}
        }
        consume_available(&mut source);
    }
}
