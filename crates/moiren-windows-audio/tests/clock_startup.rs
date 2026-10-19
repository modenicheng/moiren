use moiren_windows_audio::clock_bridge::{
    BridgeSnapshot, CapturePacket, ClockBridgeConfig, capture_bridge,
};
use moiren_windows_audio::latency::{
    LatencyBudgetError, LatencyBudgetPolicy, NativeStreamTiming, PreparedLatencyBudget,
    RenderQueuePolicy, prepare_latency_budget,
};

mod common;

use common::OUTPUT_RATE;

const CAPACITY: u32 = 1_056;
const LATE_FRAMES: u32 = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartOrder {
    Together,
    CaptureFirst,
    RenderFirst,
}

impl StartOrder {
    fn name(self) -> &'static str {
        match self {
            Self::Together => "together",
            Self::CaptureFirst => "capture_first",
            Self::RenderFirst => "render_first",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StartupCase {
    input_rate: u32,
    render_period: u32,
    drift_ppm: i32,
    phase_frames: u32,
    start_order: StartOrder,
    segment_frames: usize,
    prefill_capacity: bool,
}

impl StartupCase {
    fn capture_period(self) -> u32 {
        (u64::from(self.render_period) * u64::from(self.input_rate))
            .div_ceil(u64::from(OUTPUT_RATE)) as u32
    }

    fn target(self) -> u32 {
        if self.render_period == 480 {
            1_280
        } else {
            1_024
        }
    }

    fn render_target(self) -> u32 {
        self.render_period + LATE_FRAMES + 48
    }

    fn starts(self) -> (u64, u64) {
        let stagger = 12 * u64::from(self.render_period);
        match self.start_order {
            StartOrder::Together => (0, 0),
            StartOrder::CaptureFirst => (0, stagger),
            StartOrder::RenderFirst => (stagger, 0),
        }
    }

    fn budget(self, target: u32) -> Result<PreparedLatencyBudget, LatencyBudgetError> {
        prepare_latency_budget(
            NativeStreamTiming {
                sample_rate: self.input_rate,
                period_frames: Some(self.capture_period()),
                capacity_frames: CAPACITY,
            },
            NativeStreamTiming {
                sample_rate: OUTPUT_RATE,
                period_frames: Some(self.render_period),
                capacity_frames: CAPACITY,
            },
            LatencyBudgetPolicy::Custom {
                input_capacity_frames: 8_192,
                input_target_frames: target,
                input_jitter_frames: (u64::from(LATE_FRAMES) * u64::from(self.input_rate))
                    .div_ceil(u64::from(OUTPUT_RATE)) as u32,
                render: RenderQueuePolicy::Target {
                    frames: self.render_target(),
                    wake_margin_frames: LATE_FRAMES,
                    callback_budget_frames: 48,
                },
            },
        )
    }
}

#[derive(Debug)]
struct StartupResult {
    first_audio_at: u64,
    startup_partial_callbacks: u64,
    short_callbacks: u64,
    source_xruns: u64,
    render_missing_frames: u64,
    zero_demand_callbacks: u64,
    demanded_frames: u64,
    bridge: BridgeSnapshot,
}

fn burst_delay(event: u64) -> u64 {
    u64::from(common::burst_delay(event, LATE_FRAMES))
}

fn simulate(case: StartupCase, seconds: u32) -> StartupResult {
    let budget = case.budget(case.target()).unwrap();
    let (mut ingress, mut source, observer) = capture_bridge(
        ClockBridgeConfig::with_prepared_input(case.input_rate, 2, budget.input),
    )
    .unwrap();
    let (capture_start, render_start) = case.starts();
    let capture_interval = common::capture_interval(
        f64::from(case.capture_period()),
        case.input_rate,
        f64::from(case.drift_ppm),
    );
    let mut capture_event = 1u64;
    let mut nominal_capture_at =
        capture_start as f64 + capture_interval + f64::from(case.phase_frames);
    let mut capture_at = nominal_capture_at + burst_delay(capture_event) as f64;
    let mut render_event = 1u64;
    let mut render_at = render_start + u64::from(case.render_period) + burst_delay(render_event);
    let mut rendered_until = render_start;
    let mut padding = if case.prefill_capacity {
        CAPACITY
    } else {
        case.render_target()
    };
    let payload = vec![0.25f32.to_le_bytes(); case.capture_period() as usize * 2];
    let mut output = vec![0.0f32; case.render_target() as usize * 2];
    let mut position = 0u64;
    let mut first_audio_at = None;
    let mut result = StartupResult {
        first_audio_at: 0,
        startup_partial_callbacks: 0,
        short_callbacks: 0,
        source_xruns: 0,
        render_missing_frames: 0,
        zero_demand_callbacks: 0,
        demanded_frames: 0,
        bridge: observer.snapshot(),
    };
    let end = u64::from(seconds) * u64::from(OUTPUT_RATE);
    while capture_at.min(render_at as f64) <= end as f64 {
        if capture_at <= render_at as f64 {
            let qpc = common::capture_stamp(nominal_capture_at, capture_interval);
            ingress
                .push_packet(
                    payload.as_flattened(),
                    CapturePacket {
                        frames: case.capture_period() as usize,
                        flags: 0,
                        device_position_frames: position,
                        qpc_100ns: qpc,
                    },
                )
                .unwrap();
            position += u64::from(case.capture_period());
            capture_event += 1;
            nominal_capture_at += capture_interval;
            capture_at = nominal_capture_at + burst_delay(capture_event) as f64;
        } else {
            let consumed = render_at - rendered_until;
            result.render_missing_frames += consumed.saturating_sub(u64::from(padding));
            padding = padding.saturating_sub(consumed as u32);
            rendered_until = render_at;
            let demand = budget.render.demand(padding).unwrap();
            result.demanded_frames += u64::from(demand);
            let before = observer.snapshot();
            let mut transferred = 0usize;
            for segment in output[..demand as usize * 2].chunks_mut(case.segment_frames * 2) {
                let report = source.read_interleaved(segment).unwrap();
                transferred += report.transferred_frames;
                result.source_xruns += report.xruns;
                assert!(!report.discontinuity, "{case:?}: {report:?}");
                assert!(segment.iter().all(|sample| sample.is_finite()));
            }
            if demand == 0 {
                result.zero_demand_callbacks += 1;
                let after = observer.snapshot();
                assert_eq!(before.output_frames, after.output_frames);
                assert_eq!(before.priming_frames, after.priming_frames);
                assert_eq!(before.prime_discarded_frames, after.prime_discarded_frames);
            }
            if first_audio_at.is_some() && transferred < demand as usize {
                result.short_callbacks += 1;
            } else if first_audio_at.is_none() && transferred > 0 {
                first_audio_at = Some(render_at);
                result.startup_partial_callbacks += u64::from(transferred < demand as usize);
            }
            padding += demand;
            assert!(padding >= case.render_target());
            render_event += 1;
            render_at = render_start
                + render_event * u64::from(case.render_period)
                + burst_delay(render_event);
        }
    }
    result.first_audio_at = first_audio_at.expect("input must finish priming");
    result.bridge = observer.snapshot();
    result
}

fn assert_stable(case: StartupCase, result: &StartupResult) {
    assert_eq!(result.short_callbacks, 0, "{case:?}: {result:?}");
    assert_eq!(result.source_xruns, 0, "{case:?}: {result:?}");
    assert_eq!(result.render_missing_frames, 0, "{case:?}: {result:?}");
    assert_eq!(result.bridge.underrun_frames, 0, "{case:?}: {result:?}");
    assert_eq!(result.bridge.dropped_frames, 0, "{case:?}: {result:?}");
    assert_eq!(result.bridge.resets, 0, "{case:?}: {result:?}");
    assert_eq!(result.bridge.backlog_recoveries, 0, "{case:?}: {result:?}");
    assert_eq!(result.bridge.age_recoveries, 0, "{case:?}: {result:?}");
    assert_eq!(result.bridge.captured_frames, result.bridge.written_frames);
    assert_eq!(
        result.demanded_frames,
        result.bridge.output_frames + result.bridge.priming_frames
    );
    assert!(result.bridge.correction_ppm.abs() <= 2_000.0);
    let (capture_start, render_start) = case.starts();
    let both_started = capture_start.max(render_start);
    assert!(
        result.first_audio_at >= both_started,
        "{case:?}: {result:?}"
    );
    assert!(
        result.first_audio_at <= both_started + 12 * u64::from(case.render_period),
        "{case:?}: {result:?}"
    );
    if case.start_order == StartOrder::CaptureFirst {
        assert!(
            result.bridge.prime_discarded_frames > 0,
            "{case:?}: {result:?}"
        );
    }
}

#[test]
fn clock_startup_rejects_original_small_target_failures_at_actual_periods() {
    for input_rate in [44_100, 48_000] {
        for (render_period, targets) in [
            (480, &[128, 256, 480, 512, 1_024][..]),
            (128, &[128, 256][..]),
        ] {
            let case = StartupCase {
                input_rate,
                render_period,
                drift_ppm: -1_000,
                phase_frames: 0,
                start_order: StartOrder::Together,
                segment_frames: 256,
                prefill_capacity: false,
            };
            for &target in targets {
                assert_eq!(
                    case.budget(target),
                    Err(LatencyBudgetError::InsufficientInputBudget),
                    "{case:?}, target={target}"
                );
            }
            assert!(case.budget(case.target()).is_ok());
        }
    }
}

#[test]
fn clock_startup_joint_queue_matrix_handles_staggered_owners() {
    let seconds = std::env::var("MOIREN_STARTUP_SIM_SECONDS")
        .ok()
        .map(|value| {
            value
                .parse::<u32>()
                .expect("simulation duration is an integer")
        })
        .unwrap_or(120);
    assert!((120..=7_200).contains(&seconds));
    for input_rate in [44_100, 48_000] {
        for render_period in [336, 480] {
            let phases: &[u32] = if render_period == 336 {
                &[0, 84, 168, 335]
            } else {
                &[479]
            };
            for drift_ppm in [-1_000, 0, 1_000] {
                for &phase_frames in phases {
                    for start_order in [
                        StartOrder::Together,
                        StartOrder::CaptureFirst,
                        StartOrder::RenderFirst,
                    ] {
                        let case = StartupCase {
                            input_rate,
                            render_period,
                            drift_ppm,
                            phase_frames,
                            start_order,
                            segment_frames: 256,
                            prefill_capacity: false,
                        };
                        let result = simulate(case, seconds);
                        assert_stable(case, &result);
                        println!(
                            "startup_case,{input_rate},{},{render_period},{},{},1056,{drift_ppm},{phase_frames},{},256,{seconds},{},{},{},{},{},{},{},{},{},{},{},{}",
                            case.capture_period(),
                            case.target(),
                            case.render_target(),
                            start_order.name(),
                            result.first_audio_at,
                            result.startup_partial_callbacks,
                            result.short_callbacks,
                            result.source_xruns,
                            result.render_missing_frames,
                            result.bridge.priming_frames,
                            result.bridge.prime_discarded_frames,
                            result.bridge.underrun_frames,
                            result.bridge.dropped_frames,
                            result.bridge.resets,
                            result.bridge.low_water_guard_frames,
                            result.bridge.correction_ppm,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn clock_startup_capacity_prefill_and_segments_preserve_real_startup_accounting() {
    for input_rate in [44_100, 48_000] {
        for start_order in [StartOrder::CaptureFirst, StartOrder::RenderFirst] {
            for segment_frames in [1, 64, 256] {
                let case = StartupCase {
                    input_rate,
                    render_period: 336,
                    drift_ppm: -1_000,
                    phase_frames: 335,
                    start_order,
                    segment_frames,
                    prefill_capacity: true,
                };
                let result = simulate(case, 120);
                assert_stable(case, &result);
                assert!(result.zero_demand_callbacks > 0, "{case:?}: {result:?}");
                if start_order == StartOrder::RenderFirst {
                    assert!(result.bridge.priming_frames > 0, "{case:?}: {result:?}");
                }
            }
        }
    }
}
