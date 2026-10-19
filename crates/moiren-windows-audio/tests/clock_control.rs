use moiren_windows_audio::clock_bridge::{
    BridgeSnapshot, CapturePacket, ClockBridgeConfig, capture_bridge,
};
use moiren_windows_audio::latency::{
    LatencyBudgetError, LatencyBudgetPolicy, NativeStreamTiming, PreparedInputQueue,
    RenderQueuePolicy, prepare_latency_budget,
};
use moiren_windows_audio::stats::TIMESTAMP_ERROR;

mod common;

#[derive(Debug, Clone, Copy)]
struct ClockCase {
    sample_rate: u32,
    capture_period: usize,
    render_period: usize,
    target: usize,
    segment: usize,
    drift_ppm: f64,
    render_drift_ppm: f64,
    invalid_clock: bool,
    late_frames: f64,
    render_first: bool,
    phase_frames: f64,
    prepared_input: Option<PreparedInputQueue>,
}

impl Default for ClockCase {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            capture_period: 480,
            render_period: 480,
            target: 1024,
            segment: 256,
            drift_ppm: 0.0,
            render_drift_ppm: 0.0,
            invalid_clock: false,
            late_frames: 0.0,
            render_first: false,
            phase_frames: 0.0,
            prepared_input: None,
        }
    }
}

#[derive(Debug)]
struct ClockResult {
    short_callbacks: usize,
    ending_inventory: f64,
    mean_correction_ppm: f64,
    last_rate_estimate_ppm: Option<i64>,
    bridge: BridgeSnapshot,
}

fn simulate(case: ClockCase, seconds: u32) -> ClockResult {
    let config = case.prepared_input.map_or(
        ClockBridgeConfig {
            input_sample_rate: case.sample_rate,
            target_fill_frames: case.target,
            ..ClockBridgeConfig::default()
        },
        |input| ClockBridgeConfig::with_prepared_input(case.sample_rate, 2, input),
    );
    let (mut ingress, mut source, observer) = capture_bridge(config).unwrap();
    let capture_interval =
        common::capture_interval(case.capture_period as f64, case.sample_rate, case.drift_ppm);
    let mut capture_at = capture_interval + case.phase_frames;
    let mut nominal_capture_at = capture_at;
    let render_interval = case.render_period as f64 / (1.0 + case.render_drift_ppm / 1_000_000.0);
    let mut render_at = render_interval;
    let mut capture_events = 0usize;
    let mut position = 0u64;
    let mut payload = vec![0u8; case.capture_period * 8];
    let mut output = vec![0.0f32; case.render_period * 2];
    let mut received_audio = false;
    let mut short_callbacks = 0usize;
    let mut last_sample = None;
    let mut warm_sample = None;
    let mut output_frames = 0u64;
    let end = f64::from(seconds) * 48_000.0;
    while capture_at.min(render_at) <= end {
        let capture_next =
            capture_at < render_at || (capture_at == render_at && !case.render_first);
        if capture_next {
            capture_events += 1;
            let stamp = common::capture_stamp(nominal_capture_at, capture_interval);
            common::fill_ramp(&mut payload, position);
            ingress
                .push_packet(
                    &payload,
                    CapturePacket {
                        frames: case.capture_period,
                        flags: if case.invalid_clock {
                            TIMESTAMP_ERROR
                        } else {
                            0
                        },
                        device_position_frames: position,
                        qpc_100ns: stamp,
                    },
                )
                .unwrap();
            position += case.capture_period as u64;
            nominal_capture_at += capture_interval;
            capture_at = nominal_capture_at
                + if (capture_events + 1).is_multiple_of(7) {
                    case.late_frames
                } else {
                    0.0
                };
        } else {
            let mut transferred = 0;
            for segment in output.chunks_mut(case.segment * 2) {
                let report = source.read_interleaved(segment).unwrap();
                transferred += report.transferred_frames;
                if report.transferred_frames > 0 {
                    last_sample = Some(segment[(report.transferred_frames - 1) * 2]);
                }
            }
            if received_audio && transferred < case.render_period {
                short_callbacks += 1;
            }
            received_audio |= transferred > 0;
            output_frames += transferred as u64;
            if render_at >= end / 2.0 && warm_sample.is_none() {
                warm_sample = Some((last_sample.unwrap(), output_frames));
            }
            render_at += render_interval;
        }
    }
    let (warm_sample, warm_frames) = warm_sample.unwrap();
    let advancing = f64::from(last_sample.unwrap()) - f64::from(warm_sample);
    ClockResult {
        short_callbacks,
        ending_inventory: position as f64 - f64::from(last_sample.unwrap()),
        mean_correction_ppm: (advancing / (output_frames - warm_frames) as f64 * 48_000.0
            / f64::from(case.sample_rate)
            - 1.0)
            * 1_000_000.0,
        bridge: observer.snapshot(),
        last_rate_estimate_ppm: source
            .oldest_frame_stamp()
            .and_then(|stamp| stamp.rate_correction_ppm),
    }
}

#[test]
fn clock_control_segment_sizes_preserve_inventory_and_correction() {
    let reference = simulate(
        ClockCase {
            segment: 480,
            ..ClockCase::default()
        },
        30,
    );
    assert_eq!(reference.short_callbacks, 0);
    for segment in [256, 64, 1] {
        let result = simulate(
            ClockCase {
                segment,
                ..ClockCase::default()
            },
            30,
        );
        assert_eq!(result.short_callbacks, 0, "segment={segment}: {result:?}");
        assert!(
            (result.ending_inventory - reference.ending_inventory).abs() <= 2.0,
            "segment={segment}: reference={reference:?}, result={result:?}"
        );
        assert!(
            (result.bridge.correction_ppm - reference.bridge.correction_ppm).abs() <= 2.0,
            "segment={segment}: reference={reference:?}, result={result:?}"
        );
    }
}

#[test]
fn clock_control_rejects_insufficient_period_gap_and_jitter_reserves() {
    for (period, target, render_target, jitter) in [(480, 512, 624, 96), (128, 256, 272, 128)] {
        let timing = NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(period),
            capacity_frames: 1056,
        };
        let result = prepare_latency_budget(
            timing,
            timing,
            LatencyBudgetPolicy::Custom {
                input_capacity_frames: 8192,
                input_target_frames: target,
                input_jitter_frames: jitter,
                render: RenderQueuePolicy::Target {
                    frames: render_target,
                    wake_margin_frames: 96,
                    callback_budget_frames: 48,
                },
            },
        );
        assert_eq!(result, Err(LatencyBudgetError::InsufficientInputBudget));
    }
}

#[test]
fn clock_control_prepared_low_inventory_handles_rate_phase_order_and_lateness() {
    let seconds = std::env::var("MOIREN_CONTROL_SIM_SECONDS")
        .ok()
        .map(|value| {
            value
                .parse::<u32>()
                .expect("simulation duration is an integer")
        })
        .unwrap_or(120);
    assert!((120..=7200).contains(&seconds));
    for sample_rate in [44_100, 48_000] {
        let capture_period = if sample_rate == 44_100 { 309 } else { 336 };
        let budget = prepare_latency_budget(
            NativeStreamTiming {
                sample_rate,
                period_frames: Some(capture_period),
                capacity_frames: 738,
            },
            NativeStreamTiming {
                sample_rate: 48_000,
                period_frames: Some(336),
                capacity_frames: 738,
            },
            LatencyBudgetPolicy::Custom {
                input_capacity_frames: 8192,
                input_target_frames: 1024,
                input_jitter_frames: (96u64 * u64::from(sample_rate)).div_ceil(48_000) as u32,
                render: RenderQueuePolicy::Target {
                    frames: 480,
                    wake_margin_frames: 96,
                    callback_budget_frames: 48,
                },
            },
        )
        .unwrap();
        for drift_ppm in [-1000.0, 0.0, 1000.0] {
            for phase_frames in [0.0, 84.0, 168.0, 252.0] {
                for render_first in [false, true] {
                    let case = ClockCase {
                        sample_rate,
                        capture_period: capture_period as usize,
                        render_period: 336,
                        target: 1024,
                        segment: 256,
                        drift_ppm,
                        render_drift_ppm: 0.0,
                        invalid_clock: false,
                        late_frames: 96.0,
                        render_first,
                        phase_frames,
                        prepared_input: Some(budget.input),
                    };
                    let result = simulate(case, seconds);
                    assert_eq!(result.short_callbacks, 0, "case={case:?}: {result:?}");
                    assert_eq!(
                        result.bridge.underrun_frames, 0,
                        "case={case:?}: {result:?}"
                    );
                    assert_eq!(result.bridge.dropped_frames, 0, "case={case:?}: {result:?}");
                    assert_eq!(
                        result.bridge.backlog_recoveries, 0,
                        "case={case:?}: {result:?}"
                    );
                    assert!(
                        (result.mean_correction_ppm - drift_ppm).abs() < 300.0,
                        "case={case:?}: {result:?}"
                    );
                    println!(
                        "clock_case,{sample_rate},{capture_period},336,1024,256,{drift_ppm:.0},96,{render_first},{phase_frames:.0},{seconds},{},{},{},{},{},{},{},{}",
                        result.short_callbacks,
                        result.bridge.underrun_frames,
                        result.bridge.dropped_frames,
                        result.bridge.backlog_recoveries,
                        result.bridge.low_water_guard_frames,
                        result.mean_correction_ppm,
                        result.bridge.correction_ppm,
                        result.ending_inventory
                    );
                }
            }
        }
    }
}

#[test]
fn clock_control_low_water_prevents_positive_stale_fill_correction() {
    let input = PreparedInputQueue {
        capacity_frames: 256,
        prime_frames: 64,
        servo_reference_frames: 64,
        low_water_frames: 48,
        high_water_frames: 240,
        modeled_minimum_frames: None,
    };
    let (mut ingress, mut source, observer) =
        capture_bridge(ClockBridgeConfig::with_prepared_input(48_000, 2, input)).unwrap();
    let payload = vec![0u8; 128 * 8];
    ingress
        .push_packet(
            &payload[..64 * 8],
            CapturePacket {
                frames: 64,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    ingress
        .push_packet(
            &payload,
            CapturePacket {
                frames: 128,
                flags: 0,
                device_position_frames: 64,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    let report = source.read_interleaved(&mut [0.0; 320]).unwrap();
    assert_eq!(report.transferred_frames, 160);
    assert_eq!(report.xruns, 0);
    assert!(
        observer.snapshot().correction_ppm <= 0.0,
        "{:?}",
        observer.snapshot()
    );
    assert_eq!(observer.snapshot().underrun_frames, 0);
    assert_eq!(observer.snapshot().correction_ppm, -2000.0);
    assert_eq!(observer.snapshot().low_water_guard_frames, 1);
    let json = serde_json::to_value(observer.snapshot()).unwrap();
    assert_eq!(json["low_water_guard_frames"], 1);
}

#[test]
fn clock_control_guard_spans_segments_and_releases_after_refill() {
    for segment in [160, 32, 1] {
        let input = PreparedInputQueue {
            capacity_frames: 256,
            prime_frames: 64,
            servo_reference_frames: 64,
            low_water_frames: 48,
            high_water_frames: 240,
            modeled_minimum_frames: None,
        };
        let (mut ingress, mut source, observer) =
            capture_bridge(ClockBridgeConfig::with_prepared_input(48_000, 2, input)).unwrap();
        let payload = vec![0u8; 128 * 8];
        for (frames, position) in [(64, 0), (128, 64)] {
            ingress
                .push_packet(
                    &payload[..frames * 8],
                    CapturePacket {
                        frames,
                        flags: 0,
                        device_position_frames: position,
                        qpc_100ns: 1,
                    },
                )
                .unwrap();
            if position == 0 {
                source.read_interleaved(&mut [0.0; 2]).unwrap();
            }
        }
        let mut output = [0.0; 320];
        for samples in output.chunks_mut(segment * 2) {
            let report = source.read_interleaved(samples).unwrap();
            assert_eq!(report.transferred_frames, samples.len() / 2);
            assert_eq!(report.xruns, 0);
        }
        assert_eq!(observer.snapshot().low_water_guard_frames, 1);
        assert_eq!(observer.snapshot().correction_ppm, -2000.0);
        ingress
            .push_packet(
                &payload,
                CapturePacket {
                    frames: 128,
                    flags: 0,
                    device_position_frames: 192,
                    qpc_100ns: 1,
                },
            )
            .unwrap();
        let report = source.read_interleaved(&mut [0.0; 128]).unwrap();
        assert_eq!(report.transferred_frames, 64);
        assert_eq!(report.xruns, 0);
        assert_eq!(observer.snapshot().low_water_guard_frames, 32);
        assert!(observer.snapshot().correction_ppm > -2000.0);
        assert_eq!(observer.snapshot().resets, 0);
    }
}

#[test]
fn clock_control_guard_preserves_real_starvation_reports() {
    let input = PreparedInputQueue {
        capacity_frames: 256,
        prime_frames: 64,
        servo_reference_frames: 64,
        low_water_frames: 48,
        high_water_frames: 240,
        modeled_minimum_frames: None,
    };
    let (mut ingress, mut source, observer) =
        capture_bridge(ClockBridgeConfig::with_prepared_input(48_000, 2, input)).unwrap();
    let payload = vec![0u8; 128 * 8];
    ingress
        .push_packet(
            &payload[..64 * 8],
            CapturePacket {
                frames: 64,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    ingress
        .push_packet(
            &payload,
            CapturePacket {
                frames: 128,
                flags: 0,
                device_position_frames: 64,
                qpc_100ns: 1,
            },
        )
        .unwrap();
    source.read_interleaved(&mut [0.0; 320]).unwrap();
    let report = source.read_interleaved(&mut [99.0; 128]).unwrap();
    assert!(report.transferred_frames < 64);
    assert!(report.discontinuity);
    assert_eq!(report.xruns, 1);
    assert_eq!(
        observer.snapshot().underrun_frames,
        (64 - report.transferred_frames) as u64
    );
    assert_eq!(observer.snapshot().correction_ppm, 0.0);
    assert_eq!(observer.snapshot().resets, 1);
}

#[test]
fn clock_control_handles_independent_render_drift_and_timestamp_fallback() {
    let seconds = std::env::var("MOIREN_CONTROL_SIM_SECONDS")
        .ok()
        .map(|value| {
            value
                .parse::<u32>()
                .expect("simulation duration is an integer")
        })
        .unwrap_or(120);
    assert!((120..=7200).contains(&seconds));
    for sample_rate in [44_100, 48_000] {
        let capture_period = if sample_rate == 44_100 { 309 } else { 336 };
        let budget = prepare_latency_budget(
            NativeStreamTiming {
                sample_rate,
                period_frames: Some(capture_period),
                capacity_frames: 738,
            },
            NativeStreamTiming {
                sample_rate: 48_000,
                period_frames: Some(336),
                capacity_frames: 738,
            },
            LatencyBudgetPolicy::Custom {
                input_capacity_frames: 8_192,
                input_target_frames: 1_024,
                input_jitter_frames: (96u64 * u64::from(sample_rate)).div_ceil(48_000) as u32,
                render: RenderQueuePolicy::Target {
                    frames: 480,
                    wake_margin_frames: 96,
                    callback_budget_frames: 48,
                },
            },
        )
        .unwrap();
        for drift_ppm in [-1_000.0, 1_000.0] {
            for render_drift_ppm in [-500.0, 500.0] {
                for invalid_clock in [false, true] {
                    let case = ClockCase {
                        sample_rate,
                        capture_period: capture_period as usize,
                        render_period: 336,
                        target: 1_024,
                        segment: 256,
                        drift_ppm,
                        render_drift_ppm,
                        invalid_clock,
                        late_frames: 96.0,
                        render_first: true,
                        phase_frames: 168.0,
                        prepared_input: Some(budget.input),
                    };
                    let result = simulate(case, seconds);
                    let expected_relative = ((1.0 + drift_ppm / 1_000_000.0)
                        / (1.0 + render_drift_ppm / 1_000_000.0)
                        - 1.0)
                        * 1_000_000.0;
                    assert_eq!(result.short_callbacks, 0, "case={case:?}: {result:?}");
                    assert_eq!(
                        result.bridge.underrun_frames, 0,
                        "case={case:?}: {result:?}"
                    );
                    assert_eq!(result.bridge.dropped_frames, 0, "case={case:?}: {result:?}");
                    assert_eq!(
                        result.bridge.backlog_recoveries, 0,
                        "case={case:?}: {result:?}"
                    );
                    assert!(
                        (result.mean_correction_ppm - expected_relative).abs() < 300.0,
                        "case={case:?}: {result:?}"
                    );
                    assert!(result.bridge.correction_ppm.abs() <= 2_000.0);
                    assert_eq!(
                        result.last_rate_estimate_ppm,
                        if invalid_clock {
                            None
                        } else {
                            Some(drift_ppm as i64)
                        }
                    );
                    println!(
                        "rate_case,{sample_rate},{drift_ppm},{render_drift_ppm},{invalid_clock},{seconds},{},{},{},{},{:.6},{:.6},{}",
                        result.short_callbacks,
                        result.bridge.underrun_frames,
                        result.bridge.dropped_frames,
                        result.bridge.backlog_recoveries,
                        result.mean_correction_ppm,
                        result.bridge.correction_ppm,
                        result
                            .last_rate_estimate_ppm
                            .map_or_else(|| "none".to_owned(), |value| value.to_string()),
                    );
                }
            }
        }
    }
}
