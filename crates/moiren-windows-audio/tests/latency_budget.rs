use moiren_windows_audio::latency::{
    LatencyBudgetError, LatencyBudgetPolicy, NativeStreamTiming, RenderQueuePolicy,
    prepare_latency_budget, prepare_render_queue, target_demand,
};

fn timing(rate: u32, period: Option<u32>, capacity: u32) -> NativeStreamTiming {
    NativeStreamTiming {
        sample_rate: rate,
        period_frames: period,
        capacity_frames: capacity,
    }
}

fn target(frames: u32) -> RenderQueuePolicy {
    RenderQueuePolicy::Target {
        frames,
        wake_margin_frames: 96,
        callback_budget_frames: 48,
    }
}

fn custom(input_target_frames: u32) -> LatencyBudgetPolicy {
    LatencyBudgetPolicy::Custom {
        input_capacity_frames: 8192,
        input_target_frames,
        input_jitter_frames: 96,
        render: target(480),
    }
}

#[test]
fn target_refill_uses_queue_target_and_leaves_unused_native_capacity() {
    assert_eq!(target_demand(1056, 96, 576), Ok(480));
    assert_eq!(target_demand(1056, 600, 576), Ok(0));
    assert_eq!(target_demand(1056, 1056, 576), Ok(0));
    assert_eq!(target_demand(1056, 0, 576), Ok(576));
    assert_eq!(target_demand(738, 144, 480), Ok(336));
    assert_eq!(target_demand(738, 13, 480), Ok(467));
    assert_eq!(
        target_demand(1056, 1057, 576),
        Err(LatencyBudgetError::InvalidPadding)
    );
    for (capacity, target) in [(0, 1), (1056, 0), (1056, 1057)] {
        assert_eq!(
            target_demand(capacity, 0, target),
            Err(LatencyBudgetError::InvalidRenderTarget)
        );
    }
}

#[test]
fn compatible_budget_preserves_legacy_queues_even_when_actual_period_is_unknown() {
    let prepared = prepare_latency_budget(
        timing(44_100, None, 1056),
        timing(48_000, None, 1056),
        LatencyBudgetPolicy::Compatible,
    )
    .unwrap();
    assert_eq!(prepared.input.capacity_frames, 8192);
    assert_eq!(prepared.input.prime_frames, 2048);
    assert_eq!(prepared.input.servo_reference_frames, 2048);
    assert_eq!(prepared.input.modeled_minimum_frames, None);
    assert_eq!(prepared.render.target_frames, 1056);
    assert_eq!(prepared.render.deadline_budget_frames, None);
    assert_eq!(prepared.render.demand(96), Ok(960));
}

#[test]
fn admitted_custom_budget_keeps_capacity_independent_of_both_targets() {
    let prepared = prepare_latency_budget(
        timing(48_000, Some(336), 738),
        timing(48_000, Some(336), 738),
        custom(1024),
    )
    .unwrap();
    assert_eq!(prepared.input.capacity_frames, 8192);
    assert_eq!(prepared.input.prime_frames, 1024);
    assert_eq!(prepared.input.servo_reference_frames, 1024);
    assert_eq!(prepared.input.low_water_frames, 482);
    assert_eq!(prepared.input.modeled_minimum_frames, Some(914));
    assert_eq!(prepared.input.high_water_frames, 7856);
    assert_eq!(prepared.render.capacity_frames, 738);
    assert_eq!(prepared.render.target_frames, 480);
    assert_eq!(prepared.render.deadline_budget_frames, Some(480));
}

#[test]
fn different_rates_and_periods_use_rounded_native_input_frames() {
    let prepared = prepare_latency_budget(
        timing(44_100, Some(441), 970),
        timing(48_000, Some(336), 738),
        custom(1024),
    )
    .unwrap();
    assert_eq!(prepared.input.low_water_frames, 443);
    assert_eq!(prepared.input.modeled_minimum_frames, Some(980));
    let prepared = prepare_latency_budget(
        timing(44_100, Some(309), 680),
        timing(48_000, Some(336), 738),
        custom(1024),
    )
    .unwrap();
    assert_eq!(prepared.input.modeled_minimum_frames, Some(848));
}

#[test]
fn unadmitted_input_targets_are_rejected_without_shrinking_the_budget() {
    for input_target in [128, 256, 512, 913] {
        assert_eq!(
            prepare_latency_budget(
                timing(48_000, Some(336), 738),
                timing(48_000, Some(336), 738),
                custom(input_target)
            ),
            Err(LatencyBudgetError::InsufficientInputBudget)
        );
    }
    assert_eq!(
        prepare_latency_budget(
            timing(48_000, Some(480), 1056),
            timing(48_000, Some(336), 738),
            custom(1024)
        ),
        Err(LatencyBudgetError::InsufficientInputBudget)
    );
    for input_target in [0, 1, 7856, 8192] {
        assert_eq!(
            prepare_latency_budget(
                timing(48_000, Some(336), 738),
                timing(48_000, Some(336), 738),
                custom(input_target)
            ),
            Err(LatencyBudgetError::InvalidInputTarget)
        );
    }
}

#[test]
fn render_targets_must_cover_actual_period_and_declared_deadline_costs() {
    assert_eq!(
        prepare_render_queue(timing(48_000, Some(336), 738), target(479)),
        Err(LatencyBudgetError::InsufficientRenderBudget)
    );
    assert_eq!(
        prepare_render_queue(timing(48_000, Some(480), 1056), target(480)),
        Err(LatencyBudgetError::InsufficientRenderBudget)
    );
    assert_eq!(
        prepare_render_queue(timing(48_000, Some(336), 400), target(480)),
        Err(LatencyBudgetError::InvalidRenderTarget)
    );
    assert_eq!(
        prepare_render_queue(timing(48_000, None, 738), target(480)),
        Err(LatencyBudgetError::MissingActualPeriod)
    );
    assert_eq!(
        target(0).validate(),
        Err(LatencyBudgetError::InvalidRenderTarget)
    );
    assert_eq!(
        RenderQueuePolicy::Target {
            frames: 480,
            wake_margin_frames: 96,
            callback_budget_frames: 0
        }
        .validate(),
        Err(LatencyBudgetError::InvalidRenderTarget)
    );
}

#[test]
fn custom_joint_budget_requires_both_current_device_periods() {
    assert_eq!(
        prepare_latency_budget(
            timing(48_000, None, 738),
            timing(48_000, Some(336), 738),
            custom(1024)
        ),
        Err(LatencyBudgetError::MissingActualPeriod)
    );
    assert_eq!(
        prepare_latency_budget(
            timing(48_000, Some(336), 738),
            timing(48_000, None, 738),
            custom(1024)
        ),
        Err(LatencyBudgetError::MissingActualPeriod)
    );
}

#[test]
fn invalid_timing_and_overflow_never_become_successful_clamped_budgets() {
    for invalid in [
        timing(0, Some(336), 738),
        timing(48_000, Some(0), 738),
        timing(48_000, Some(336), 0),
    ] {
        assert_eq!(
            prepare_render_queue(invalid, RenderQueuePolicy::FullCapacity),
            Err(LatencyBudgetError::InvalidTiming)
        );
    }
    assert_eq!(
        prepare_render_queue(
            timing(48_000, Some(336), u32::MAX),
            RenderQueuePolicy::Target {
                frames: u32::MAX,
                wake_margin_frames: u32::MAX,
                callback_budget_frames: 48
            }
        ),
        Err(LatencyBudgetError::ArithmeticOverflow)
    );
    assert_eq!(
        prepare_latency_budget(
            timing(u32::MAX, Some(1), u32::MAX),
            timing(1, Some(336), 738),
            custom(1024)
        ),
        Err(LatencyBudgetError::ArithmeticOverflow)
    );
}
