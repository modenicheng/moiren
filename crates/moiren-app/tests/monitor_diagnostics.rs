use moiren_app::monitor::{
    MonitorInitializationApi, MonitorInputBudget, MonitorObservation, MonitorPreparedDiagnostics,
    MonitorRenderBudget, MonitorStreamDiagnostics,
};
use std::time::Duration;

fn timing(sample_rate: u32, period: Option<u32>) -> MonitorStreamDiagnostics {
    MonitorStreamDiagnostics {
        sample_rate,
        actual_period_frames: period,
        capacity_frames: 1056,
        requested_period_frames: Some(240),
        initialization_api: Some(MonitorInitializationApi::Client3),
        period_fallback: None,
    }
}

#[test]
fn actual_period_uses_its_own_rate_and_never_substitutes_the_request() {
    let input = timing(44_100, Some(441));
    let output = timing(48_000, Some(288));
    assert_eq!(input.period_duration(), Some(Duration::from_millis(10)));
    assert_eq!(output.period_duration(), Some(Duration::from_millis(6)));
    assert_eq!(timing(48_000, None).period_duration(), None);
    assert_eq!(timing(48_000, Some(0)).period_duration(), None);
    assert_eq!(timing(0, Some(240)).period_duration(), None);
}

#[test]
fn queue_duration_handles_empty_fill_rounding_and_invalid_conversion() {
    let input = timing(44_100, None);
    assert_eq!(input.frames_duration(0), Some(Duration::ZERO));
    assert_eq!(input.frames_duration(1), Some(Duration::from_nanos(22_676)));
    assert_eq!(input.frames_duration(441), Some(Duration::from_millis(10)));
    assert_eq!(input.frames_duration(u64::MAX), None);
    assert_eq!(timing(0, None).frames_duration(0), None);
}

#[test]
fn input_fill_is_distinct_from_prime_target_and_render_capacity() {
    let observation = MonitorObservation {
        prepared: MonitorPreparedDiagnostics {
            input: timing(48_000, Some(240)),
            output: timing(48_000, Some(288)),
            input_queue: MonitorInputBudget {
                capacity_frames: 8192,
                prime_frames: 1024,
                servo_reference_frames: 1024,
                low_water_frames: 256,
                high_water_frames: 2048,
                modeled_minimum_frames: Some(818),
            },
            render_queue: MonitorRenderBudget {
                capacity_frames: 1056,
                target_frames: 480,
                deadline_budget_frames: Some(432),
            },
        },
        processed_frames: 960,
        input_fill_frames: 192,
        correction_ppm: -100.0,
    };
    assert_eq!(
        observation.input_fill_duration(),
        Some(Duration::from_millis(4))
    );
    assert_eq!(observation.prepared.input_queue.prime_frames, 1024);
    assert_eq!(observation.prepared.render_queue.target_frames, 480);
    assert_eq!(observation.prepared.render_queue.capacity_frames, 1056);
}
