use moiren_windows_audio::clock_bridge::{
    BridgeSnapshot, CapturePacket, ClockBridgeConfig, capture_bridge,
};
use moiren_windows_audio::latency::{
    LatencyBudgetError, LatencyBudgetPolicy, NativeStreamTiming, PreparedLatencyBudget,
    RenderQueuePolicy, prepare_latency_budget,
};

mod common;

use common::{OUTPUT_RATE, burst_delay};

const RENDER_PERIOD: u32 = 336;
const RENDER_TARGET: u32 = 480;
const PAUSE_START: u64 = 20 * OUTPUT_RATE as u64;
const PAUSE_FRAMES: u64 = 9_600;
const RECOVERY_DEADLINE: u64 = PAUSE_START + PAUSE_FRAMES + 10 * RENDER_PERIOD as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    Burst,
    CaptureStall,
    RenderPause,
}

impl Fault {
    fn name(self) -> &'static str {
        match self {
            Self::Burst => "burst",
            Self::CaptureStall => "capture_stall",
            Self::RenderPause => "render_pause",
        }
    }

    fn capture_delay(self) -> u32 {
        match self {
            Self::CaptureStall => 1024,
            Self::Burst | Self::RenderPause => 96,
        }
    }

    fn input_target(self) -> u32 {
        match self {
            Self::CaptureStall => 2048,
            Self::Burst | Self::RenderPause => 1024,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FaultCase {
    fault: Fault,
    input_rate: u32,
    drift_ppm: i32,
    phase_frames: u32,
    render_first: bool,
    segment_frames: usize,
}

impl FaultCase {
    fn capture_period(self) -> u32 {
        if self.input_rate == 44_100 { 309 } else { 336 }
    }

    fn budget(self, target: u32) -> Result<PreparedLatencyBudget, LatencyBudgetError> {
        prepare_latency_budget(
            NativeStreamTiming {
                sample_rate: self.input_rate,
                period_frames: Some(self.capture_period()),
                capacity_frames: 738,
            },
            NativeStreamTiming {
                sample_rate: OUTPUT_RATE,
                period_frames: Some(RENDER_PERIOD),
                capacity_frames: 738,
            },
            LatencyBudgetPolicy::Custom {
                input_capacity_frames: 8192,
                input_target_frames: target,
                input_jitter_frames: (u64::from(self.fault.capture_delay())
                    * u64::from(self.input_rate))
                .div_ceil(u64::from(OUTPUT_RATE)) as u32,
                render: RenderQueuePolicy::Target {
                    frames: RENDER_TARGET,
                    wake_margin_frames: 96,
                    callback_budget_frames: 48,
                },
            },
        )
    }
}

#[derive(Debug)]
struct FaultResult {
    short_callbacks: u64,
    stable_short_callbacks: u64,
    source_xruns: u64,
    stable_source_xruns: u64,
    render_missing_frames: u64,
    recovery_callbacks: u64,
    last_short_after_resume_frames: u64,
    resets_at_deadline: u64,
    underruns_at_deadline: u64,
    recoveries_at_deadline: u64,
    drops_at_deadline: u64,
    bridge: BridgeSnapshot,
}

fn simulate(case: FaultCase, seconds: u32) -> FaultResult {
    let budget = case.budget(case.fault.input_target()).unwrap();
    let (mut ingress, mut source, observer) = capture_bridge(
        ClockBridgeConfig::with_prepared_input(case.input_rate, 2, budget.input),
    )
    .unwrap();
    let capture_interval = common::capture_interval(
        f64::from(case.capture_period()),
        case.input_rate,
        f64::from(case.drift_ppm),
    );
    let mut capture_event = 1u64;
    let mut nominal_capture_at = capture_interval + f64::from(case.phase_frames);
    let mut capture_at =
        nominal_capture_at + f64::from(burst_delay(capture_event, case.fault.capture_delay()));
    let mut render_event = 1u64;
    let mut render_at = u64::from(RENDER_PERIOD + burst_delay(render_event, 96));
    let mut position = 0u64;
    let mut payload = vec![0u8; case.capture_period() as usize * 8];
    let mut output = vec![0.0f32; RENDER_TARGET as usize * 2];
    let mut padding = RENDER_TARGET;
    let mut rendered_until = 0u64;
    let mut received_audio = false;
    let mut result = FaultResult {
        short_callbacks: 0,
        stable_short_callbacks: 0,
        source_xruns: 0,
        stable_source_xruns: 0,
        render_missing_frames: 0,
        recovery_callbacks: 0,
        last_short_after_resume_frames: 0,
        resets_at_deadline: 0,
        underruns_at_deadline: 0,
        recoveries_at_deadline: 0,
        drops_at_deadline: 0,
        bridge: observer.snapshot(),
    };
    let mut deadline_snapshot = None;
    let mut pause_applied = false;
    let end = u64::from(seconds) * u64::from(OUTPUT_RATE);
    while capture_at.min(render_at as f64) <= end as f64 {
        let capture_next =
            capture_at < render_at as f64 || (capture_at == render_at as f64 && !case.render_first);
        if capture_next {
            common::fill_ramp(&mut payload, position);
            let stamp = common::capture_stamp(nominal_capture_at, capture_interval);
            ingress
                .push_packet(
                    &payload,
                    CapturePacket {
                        frames: case.capture_period() as usize,
                        flags: 0,
                        device_position_frames: position,
                        qpc_100ns: stamp,
                    },
                )
                .unwrap();
            position += u64::from(case.capture_period());
            capture_event += 1;
            nominal_capture_at += capture_interval;
            capture_at = capture_at.max(
                nominal_capture_at
                    + f64::from(burst_delay(capture_event, case.fault.capture_delay())),
            );
        } else {
            let consumed = render_at - rendered_until;
            result.render_missing_frames += consumed.saturating_sub(u64::from(padding));
            padding = padding.saturating_sub(consumed.min(u64::from(u32::MAX)) as u32);
            rendered_until = render_at;
            let demand = budget.render.demand(padding).unwrap();
            let fault_window = case.fault == Fault::RenderPause
                && (PAUSE_START..RECOVERY_DEADLINE).contains(&render_at);
            let mut transferred = 0usize;
            let mut xruns = 0u64;
            for segment in output[..demand as usize * 2].chunks_mut(case.segment_frames * 2) {
                let report = source.read_interleaved(segment).unwrap();
                transferred += report.transferred_frames;
                xruns += report.xruns;
                assert!(segment.iter().all(|sample| sample.is_finite()));
            }
            if received_audio && transferred < demand as usize {
                result.short_callbacks += 1;
                result.stable_short_callbacks += u64::from(!fault_window);
                if fault_window {
                    result.last_short_after_resume_frames =
                        render_at - (PAUSE_START + PAUSE_FRAMES);
                }
            }
            received_audio |= transferred > 0;
            result.source_xruns += xruns;
            result.stable_source_xruns += if fault_window { 0 } else { xruns };
            if fault_window {
                result.recovery_callbacks += 1;
            }
            padding += demand;
            assert_eq!(padding, RENDER_TARGET);
            if render_at >= RECOVERY_DEADLINE && deadline_snapshot.is_none() {
                deadline_snapshot = Some(observer.snapshot());
            }
            render_event += 1;
            render_at =
                render_event * u64::from(RENDER_PERIOD) + u64::from(burst_delay(render_event, 96));
            if case.fault == Fault::RenderPause && !pause_applied && render_at >= PAUSE_START {
                pause_applied = true;
                render_at = PAUSE_START + PAUSE_FRAMES;
                render_event = render_at / u64::from(RENDER_PERIOD);
            }
        }
    }
    let deadline = deadline_snapshot.unwrap();
    result.resets_at_deadline = deadline.resets;
    result.underruns_at_deadline = deadline.underrun_frames;
    result.recoveries_at_deadline = deadline.backlog_recoveries;
    result.drops_at_deadline = deadline.dropped_frames;
    result.bridge = observer.snapshot();
    result
}

#[test]
fn clock_faults_reject_capture_stall_without_declared_input_reserve() {
    for input_rate in [44_100, 48_000] {
        let case = FaultCase {
            fault: Fault::CaptureStall,
            input_rate,
            drift_ppm: 0,
            phase_frames: 0,
            render_first: false,
            segment_frames: 256,
        };
        assert_eq!(
            case.budget(1024),
            Err(LatencyBudgetError::InsufficientInputBudget)
        );
        assert!(case.budget(2048).is_ok());
    }
}

#[test]
fn clock_faults_joint_queue_handles_bursts_stalls_and_pause_recovery() {
    let seconds = std::env::var("MOIREN_FAULT_SIM_SECONDS")
        .ok()
        .map(|value| {
            value
                .parse::<u32>()
                .expect("simulation duration is an integer")
        })
        .unwrap_or(120);
    assert!((120..=7200).contains(&seconds));
    let mut cases = Vec::new();
    for fault in [Fault::Burst, Fault::CaptureStall, Fault::RenderPause] {
        for input_rate in [44_100, 48_000] {
            for drift_ppm in [-1000, 0, 1000] {
                for phase_frames in [0, 84, 168, 252] {
                    for render_first in [false, true] {
                        cases.push(FaultCase {
                            fault,
                            input_rate,
                            drift_ppm,
                            phase_frames,
                            render_first,
                            segment_frames: 256,
                        });
                    }
                }
            }
        }
    }
    common::for_each_parallel(&cases, |&case| {
        let result = simulate(case, seconds);
        assert_eq!(result.stable_short_callbacks, 0, "{case:?}: {result:?}");
        assert_eq!(result.stable_source_xruns, 0, "{case:?}: {result:?}");
        assert_eq!(
            result.bridge.resets, result.resets_at_deadline,
            "{case:?}: {result:?}"
        );
        assert_eq!(
            result.bridge.underrun_frames, result.underruns_at_deadline,
            "{case:?}: {result:?}"
        );
        assert_eq!(
            result.bridge.backlog_recoveries, result.recoveries_at_deadline,
            "{case:?}: {result:?}"
        );
        assert_eq!(
            result.bridge.dropped_frames, result.drops_at_deadline,
            "{case:?}: {result:?}"
        );
        if case.fault == Fault::RenderPause {
            assert!(result.render_missing_frames > 0, "{case:?}: {result:?}");
            assert!(result.bridge.dropped_frames > 0, "{case:?}: {result:?}");
            assert_eq!(result.bridge.backlog_recoveries, 1, "{case:?}: {result:?}");
            assert!(
                (u64::from(case.budget(1024).unwrap().input.high_water_frames) - 1024
                    ..=8194 - 1024)
                    .contains(&result.bridge.backlog_discarded_frames),
                "{case:?}: {result:?}"
            );
            assert!(result.recovery_callbacks <= 11, "{case:?}: {result:?}");
        } else {
            assert_eq!(result.short_callbacks, 0, "{case:?}: {result:?}");
            assert_eq!(result.source_xruns, 0, "{case:?}: {result:?}");
            assert_eq!(result.render_missing_frames, 0, "{case:?}: {result:?}");
            assert_eq!(result.bridge.dropped_frames, 0, "{case:?}: {result:?}");
            assert_eq!(result.bridge.backlog_recoveries, 0, "{case:?}: {result:?}");
        }
        println!(
            "fault_case,{},{},{},336,{},8192,480,{},96,8,17,256,{},{},{},{seconds},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            case.fault.name(),
            case.input_rate,
            case.capture_period(),
            case.fault.input_target(),
            case.fault.capture_delay(),
            case.drift_ppm,
            case.phase_frames,
            case.render_first,
            result.short_callbacks,
            result.stable_short_callbacks,
            result.source_xruns,
            result.stable_source_xruns,
            result.render_missing_frames,
            result.bridge.underrun_frames,
            result.bridge.dropped_frames,
            result.bridge.backlog_recoveries,
            result.bridge.backlog_discarded_frames,
            result.bridge.resets,
            result.recovery_callbacks,
            result.last_short_after_resume_frames,
            result.bridge.low_water_guard_frames,
        );
    });
}
