use moiren_windows_audio::{
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    virtual_owner::{ClockError, ClockSchedule, DEBT_CAP_PERIODS, VirtualBufferOptions},
};

#[path = "common/alloc_guard.rs"]
mod alloc_guard;

fn options(capacity: u32) -> VirtualBufferOptions {
    VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: capacity,
            target_frames: 320,
            jitter_frames: 32,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 128,
            max_gap_frames: 128,
            source: DeliveryTimingSource::Declared,
        }),
        ..Default::default()
    }
}

#[test]
fn virtual_budget_keeps_normal_residency_independent_of_carrier_capacity() {
    for capacity in [1024, 8192] {
        let queue = options(capacity).prepare(48_000, 128).unwrap().unwrap();
        assert_eq!(queue.capacity_frames, capacity);
        assert_eq!(queue.prime_frames, 320);
        assert_eq!(queue.modeled_minimum_frames, Some(320));
        options(capacity).admit_native_packet(128).unwrap();
    }
}

#[test]
fn idle_preparation_does_not_assume_maximum_packet_or_invent_native_timing() {
    let options = options(1024);
    assert!(options.prepare(48_000, 128).is_ok());
    assert!(options.admit_native_packet(129).is_err());
    assert!(options.admit_native_packet(0).is_err());
    assert!(
        VirtualBufferOptions {
            delivery: None,
            ..options
        }
        .prepare(48_000, 128)
        .is_err()
    );
    assert!(
        VirtualBufferOptions::default()
            .prepare(48_000, 128)
            .unwrap()
            .is_none()
    );
    // Compatible retains support for clients whose shape is not known yet.
    assert!(
        VirtualBufferOptions::default()
            .admit_native_packet(4800)
            .is_ok()
    );
}

#[test]
fn full_native_consumer_burst_cannot_be_replaced_by_a_smaller_graph_segment() {
    let options = options(8192);
    assert!(options.prepare(48_000, 128).is_ok());
    assert!(options.prepare(48_000, 256).is_err());
    assert!(options.prepare(44_100, 128).is_err());
    let wrong_rate = VirtualBufferOptions {
        delivery: Some(InputDeliveryTiming {
            sample_rate: 44_100,
            ..options.delivery.unwrap()
        }),
        ..options
    };
    assert!(wrong_rate.prepare(48_000, 128).is_err());
}

#[test]
fn explicit_recovery_cap_is_an_independent_serde_defaulted_option() {
    let mut value = serde_json::to_value(options(1024)).unwrap();
    value["recovery_cap_periods"] = serde_json::json!(1);
    let configured: VirtualBufferOptions = serde_json::from_value(value.clone())
        .expect("a bounded recovery cap must be independently configurable");
    assert_eq!(serde_json::to_value(configured).unwrap(), value);
    assert!(configured.prepare(48_000, 128).is_ok());
}

#[test]
fn zero_recovery_cap_cannot_disable_the_clock_budget() {
    let configured = VirtualBufferOptions {
        recovery_cap_periods: Some(0),
        ..options(1024)
    };
    assert!(configured.validate().is_err());
}

#[test]
fn legacy_serialized_options_and_clock_keep_the_eight_period_default() {
    let mut value = serde_json::to_value(options(1024)).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("recovery_cap_periods");
    let legacy: VirtualBufferOptions = serde_json::from_value(value).unwrap();
    assert_eq!(legacy.recovery_cap_periods, None);
    assert_eq!(
        legacy.admitted_recovery_cap_periods().unwrap(),
        DEBT_CAP_PERIODS
    );
    assert!(legacy.prepare_recovery(48_000, 256).is_err());
    let compatible = VirtualBufferOptions::default()
        .prepare_recovery(48_000, 256)
        .unwrap();
    assert_eq!(compatible.engine_burst_frames, 2048);
    assert_eq!(compatible.recovery_cap_periods, DEBT_CAP_PERIODS);
    assert_eq!(compatible.native_packet_frames, None);
    assert_eq!(compatible.queue, None);
    let mut clock = ClockSchedule::new(48_000, 256, 48_000, 0).unwrap();
    let tick = clock.due(10 * 256).unwrap().unwrap();
    assert_eq!((tick.backlog_periods, tick.clamped_periods), (8, 2));
}

#[test]
fn recovery_budget_packet_and_engine_block_matrix_is_independent_of_capacity() {
    for packet in [48, 64, 128, 256, 480] {
        for block in [48, 64, 128, 256, 480] {
            for cap in [1, 2, DEBT_CAP_PERIODS] {
                let demand = block * cap as u32;
                let transfer = (u64::from(demand) * 1000).div_ceil(998) as u32;
                let target = transfer + 32 + packet + 32;
                let mut admitted = None;
                for capacity in [16_384, 32_768] {
                    let configured = VirtualBufferOptions {
                        input: TransportInputPolicy::Target {
                            capacity_frames: capacity,
                            target_frames: target,
                            jitter_frames: 32,
                        },
                        delivery: Some(InputDeliveryTiming {
                            sample_rate: 48_000,
                            max_packet_frames: packet,
                            max_gap_frames: packet,
                            source: DeliveryTimingSource::Declared,
                        }),
                        recovery_cap_periods: Some(cap),
                    };
                    let budget = configured.prepare_recovery(48_000, block).unwrap();
                    assert_eq!(budget.requested, configured);
                    assert_eq!(budget.engine_block_frames, block);
                    assert_eq!(budget.engine_burst_frames, demand);
                    assert_eq!(budget.native_transfer_frames, transfer);
                    assert_eq!(budget.fir_lookahead_frames, 32);
                    assert_eq!(budget.wake_allowance_frames, Some(32));
                    assert_eq!(budget.recovery_cap_periods, cap);
                    assert_eq!(
                        budget.native_packet_frames, None,
                        "declared is not negotiated"
                    );
                    let queue = budget.queue.unwrap();
                    assert_eq!(queue.capacity_frames, capacity);
                    assert_eq!(queue.prime_frames, target);
                    assert_eq!(queue.servo_reference_frames, target);
                    assert_eq!(queue.modeled_minimum_frames, Some(target));
                    assert_eq!(queue.low_water_frames, transfer + 32);
                    assert_eq!(
                        budget
                            .admit_native_packet(packet)
                            .unwrap()
                            .native_packet_frames,
                        Some(packet)
                    );
                    assert!(budget.admit_native_packet(packet + 1).is_err());
                    assert_eq!(budget.native_packet_frames, None);
                    let published = serde_json::to_value(budget).unwrap();
                    assert_eq!(published["requested"]["recovery_cap_periods"], cap);
                    assert_eq!(published["recovery_cap_periods"], cap);
                    assert_eq!(published["engine_burst_frames"], demand);
                    assert!(published["native_packet_frames"].is_null());
                    if let Some(previous) = admitted {
                        assert_eq!(
                            previous,
                            (
                                queue.prime_frames,
                                queue.low_water_frames,
                                budget.engine_burst_frames
                            )
                        );
                    }
                    admitted = Some((
                        queue.prime_frames,
                        queue.low_water_frames,
                        budget.engine_burst_frames,
                    ));

                    let smaller = VirtualBufferOptions {
                        input: TransportInputPolicy::Target {
                            capacity_frames: capacity,
                            target_frames: target - 1,
                            jitter_frames: 32,
                        },
                        ..configured
                    };
                    assert!(smaller.prepare_recovery(48_000, block).is_err());
                }
            }
        }
    }
}

#[test]
fn recovery_cap_rejects_unsupported_values_and_preserves_full_demand() {
    for cap in [0, DEBT_CAP_PERIODS + 1, u64::MAX] {
        let configured = VirtualBufferOptions {
            recovery_cap_periods: Some(cap),
            ..Default::default()
        };
        assert!(configured.validate().is_err());
        assert!(configured.prepare_recovery(48_000, 128).is_err());
        assert!(matches!(
            ClockSchedule::with_recovery_cap(48_000, 128, 48_000, 0, cap),
            Err(ClockError::InvalidConfiguration)
        ));
    }
    let configured = VirtualBufferOptions {
        recovery_cap_periods: Some(1),
        input: TransportInputPolicy::Target {
            capacity_frames: 1024,
            target_frames: 321,
            jitter_frames: 32,
        },
        ..options(1024)
    };
    assert!(configured.prepare_recovery(48_000, 128).is_ok());
    assert!(configured.prepare_recovery(48_000, 256).is_err());
    assert!(
        configured.prepare_budget(48_000, 128, 256).is_err(),
        "a physical wake is not truncated to the virtual recovery cap"
    );
    assert!(configured.prepare_recovery(44_100, 128).is_err());
    assert!(configured.prepare_recovery(0, 128).is_err());
    assert!(configured.prepare_recovery(48_000, 0).is_err());
    assert!(configured.prepare_budget(48_000, 256, 128).is_err());
    assert!(
        VirtualBufferOptions::default()
            .prepare_recovery(48_000, u32::MAX)
            .is_err()
    );
}

#[test]
fn native_packet_capacity_and_fir_reserves_are_not_spent_on_recovery() {
    // Transfer=129, FIR=32 and a FIR-length residual even with tiny packets.
    let tiny = VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 1024,
            target_frames: 192,
            jitter_frames: 0,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 1,
            max_gap_frames: 1,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(1),
    };
    assert!(tiny.prepare_recovery(48_000, 128).is_err());
    let configured = VirtualBufferOptions {
        recovery_cap_periods: Some(1),
        input: TransportInputPolicy::Target {
            capacity_frames: 609,
            target_frames: 321,
            jitter_frames: 32,
        },
        ..options(609)
    };
    assert!(
        configured.prepare_recovery(48_000, 128).is_err(),
        "capacity must also reserve target + packet + drift/FIR transfer (610)"
    );
    let configured = VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 610,
            target_frames: 321,
            jitter_frames: 32,
        },
        ..configured
    };
    assert!(configured.prepare_recovery(48_000, 128).is_ok());
}

#[test]
fn configured_clock_counts_every_clamped_period_without_allocating() {
    for block in [48, 64, 128, 256, 480] {
        for cap in 1..=DEBT_CAP_PERIODS {
            let mut clock =
                ClockSchedule::with_recovery_cap(48_000, block, 48_000, 0, cap).unwrap();
            let (rendered, clamped) = alloc_guard::assert_allocates_nothing(|| {
                let mut rendered = 0;
                let mut clamped = 0;
                let mut expected_backlog = cap;
                while let Some(tick) = clock.due(20 * block as u64).unwrap() {
                    assert_eq!(tick.frames, block);
                    assert_eq!(tick.backlog_periods, expected_backlog);
                    expected_backlog -= 1;
                    rendered += 1;
                    clamped += tick.clamped_periods;
                }
                (rendered, clamped)
            });
            assert_eq!(rendered, cap);
            assert_eq!(rendered + clamped, 20);
            assert_eq!(
                clock.remaining_ticks(20 * block as u64).unwrap(),
                block as u64
            );
        }
    }
}

#[test]
fn configured_fractional_clock_repays_supported_jitter_without_rate_bias() {
    for cap in [2, 3, DEBT_CAP_PERIODS] {
        let mut clock = ClockSchedule::with_recovery_cap(10_000_000, 128, 44_100, 7, cap).unwrap();
        let mut rendered = 0;
        for period in (cap..=3000).step_by(cap as usize) {
            let now = 7 + (u128::from(period) * 128 * 10_000_000).div_ceil(44_100) as u64;
            while let Some(tick) = clock.due(now).unwrap() {
                assert_eq!(tick.clamped_periods, 0);
                rendered += 1;
            }
        }
        assert_eq!(rendered, 3000 / cap * cap);
    }
}

fn reprime_options(capacity: u32, target: u32, packet: u32, jitter: u32) -> VirtualBufferOptions {
    VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: capacity,
            target_frames: target,
            jitter_frames: jitter,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: packet,
            max_gap_frames: packet,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(1),
    }
}

#[test]
fn microphone_reprime_small_packet_covers_graph_wake_and_keeps_target() {
    let budget = reprime_options(1024, 369, 48, 32)
        .prepare_recovery(48_000, 256)
        .unwrap();
    let before = budget;
    let recovery = budget.microphone_reprime(48, 1024).unwrap();
    assert_eq!(recovery.packet_frames, 48);
    assert_eq!(recovery.required_frames, 369);
    // ceil(401 frames / 0.998 at 48 kHz) + one Engine wake + jitter.
    assert_eq!(recovery.max_wait_100ns, 83_710 + 53_334 + 6_667);
    assert!(
        recovery.max_wait_100ns > 25_000,
        "old 2.5-packet wait is too short"
    );
    assert_eq!(
        budget, before,
        "recovery must not rewrite steady-state admission"
    );
    assert_eq!(budget.queue.unwrap().prime_frames, 369);
    assert_eq!(budget.native_packet_frames, None, "no invented negotiation");
    assert_eq!(
        serde_json::to_value(recovery).unwrap(),
        serde_json::json!({
            "packet_frames": 48,
            "required_frames": 369,
            "max_wait_100ns": 143_711,
            "capacity_limited": false,
        })
    );
}

#[test]
fn microphone_reprime_packet_block_rate_matrix_has_bounded_independent_reserve() {
    let mut cases = 0;
    for rate in [44_100_u32, 48_000, 96_000] {
        for block in [48_u32, 64, 128, 256, 480] {
            for packet in [48_u32, 64, 128, 256, 480] {
                for cap in [1_u64, 2, DEBT_CAP_PERIODS] {
                    let burst = block * cap as u32;
                    let transfer =
                        (u64::from(burst) * 48_000 * 1000).div_ceil(u64::from(rate) * 998) as u32;
                    let target = transfer + 32 + packet + 32;
                    let mut previous = None;
                    for capacity in [16_384, 32_768] {
                        let configured = VirtualBufferOptions {
                            recovery_cap_periods: Some(cap),
                            ..reprime_options(capacity, target, packet, 32)
                        };
                        let budget = configured.prepare_recovery(rate, block).unwrap();
                        let recovery = budget.microphone_reprime(packet, capacity).unwrap();
                        let required = target.max(packet * 2);
                        let fill =
                            ((u64::from(required) + 32) * 1000 * 10_000_000).div_ceil(48_000 * 998);
                        let wake = (u64::from(burst) * 10_000_000).div_ceil(u64::from(rate));
                        let jitter = (32_u64 * 10_000_000).div_ceil(48_000);
                        let legacy = (u64::from(packet) * 5 * 10_000_000).div_ceil(48_000 * 2);
                        assert_eq!(recovery.required_frames, required);
                        assert_eq!(recovery.max_wait_100ns, legacy.max(fill + wake + jitter));
                        assert!(recovery.required_frames <= capacity);
                        assert!(!recovery.capacity_limited);
                        assert_eq!(budget.queue.unwrap().prime_frames, target);
                        if let Some(previous) = previous {
                            assert_eq!(
                                recovery, previous,
                                "spare capacity must not retune recovery"
                            );
                        }
                        previous = Some(recovery);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 450);
}

#[test]
fn microphone_reprime_larger_target_and_jitter_have_separate_effects() {
    let baseline = reprime_options(2048, 369, 48, 32)
        .prepare_recovery(48_000, 256)
        .unwrap()
        .microphone_reprime(48, 2048)
        .unwrap();
    let larger_target = reprime_options(2048, 512, 48, 32)
        .prepare_recovery(48_000, 256)
        .unwrap()
        .microphone_reprime(48, 2048)
        .unwrap();
    assert_eq!(larger_target.required_frames, 512);
    assert!(larger_target.max_wait_100ns > baseline.max_wait_100ns);
    let jitter = reprime_options(2048, 512, 48, 64)
        .prepare_recovery(48_000, 256)
        .unwrap()
        .microphone_reprime(48, 2048)
        .unwrap();
    assert_eq!(jitter.required_frames, larger_target.required_frames);
    assert_eq!(jitter.max_wait_100ns - larger_target.max_wait_100ns, 6_667);
}

#[test]
fn microphone_reprime_compatible_threshold_is_reachable_and_keeps_legacy_wait_floor() {
    let budget = VirtualBufferOptions::default()
        .prepare_recovery(48_000, 64)
        .unwrap();
    for (packet, capacity, required) in [(480, 480, 480), (480, 720, 720), (480, 2048, 1026)] {
        let recovery = budget.microphone_reprime(packet, capacity).unwrap();
        assert_eq!(recovery.required_frames, required);
        assert_eq!(recovery.capacity_limited, capacity < 1026);
        assert!(recovery.required_frames <= capacity);
        assert!(recovery.max_wait_100ns >= 250_000);
    }
    let quick = VirtualBufferOptions {
        recovery_cap_periods: Some(1),
        ..Default::default()
    }
    .prepare_recovery(96_000, 1)
    .unwrap()
    .microphone_reprime(480, 480)
    .unwrap();
    assert_eq!(quick.max_wait_100ns, 250_000, "legacy floor still applies");
    assert_eq!(budget.queue, None);
}

#[test]
fn microphone_reprime_compatible_readiness_covers_a_complete_engine_publication_gap() {
    let budget = VirtualBufferOptions {
        recovery_cap_periods: Some(1),
        ..Default::default()
    }
    .prepare_recovery(48_000, 256)
    .unwrap();
    let recovery = budget.microphone_reprime(48, 1024).unwrap();
    assert_eq!(budget.native_transfer_frames, 257);
    assert_eq!(recovery.required_frames, 257 + 48 + 32);
    assert!(!recovery.capacity_limited);
    assert!(
        recovery.required_frames > 2 * 48,
        "two packets do not span an Engine wake"
    );
    let constrained = budget.microphone_reprime(48, 256).unwrap();
    assert_eq!(constrained.required_frames, 256);
    assert!(
        constrained.capacity_limited,
        "legacy capacity limitation is not hidden"
    );
    let exact = budget
        .microphone_reprime(48, recovery.required_frames)
        .unwrap();
    assert_eq!(exact.required_frames, recovery.required_frames);
    assert!(
        !exact.capacity_limited,
        "equal capacity reaches the ideal threshold"
    );
}

#[test]
fn microphone_reprime_target_never_silently_caps_an_impossible_recovery_threshold() {
    use moiren_windows_audio::latency::LatencyBudgetError;
    let mut budget = reprime_options(1024, 369, 48, 32)
        .prepare_recovery(48_000, 256)
        .unwrap();
    // Public preparation DTOs must not silently cap inconsistent later demand.
    budget.native_transfer_frames = 1024;
    assert_eq!(
        budget.microphone_reprime(48, 1024),
        Err(LatencyBudgetError::InvalidDeliveryBound)
    );
}

#[test]
fn microphone_reprime_rejects_underreported_public_preparation_evidence() {
    let budget = VirtualBufferOptions {
        recovery_cap_periods: Some(1),
        ..Default::default()
    }
    .prepare_recovery(48_000, 256)
    .unwrap();
    for field in 0..4 {
        let mut altered = budget;
        match field {
            0 => altered.native_transfer_frames = 0,
            1 => altered.fir_lookahead_frames = 0,
            2 => altered.recovery_cap_periods = 0,
            _ => altered.wake_allowance_frames = Some(0),
        }
        assert!(
            altered.microphone_reprime(48, 1024).is_err(),
            "field={field}"
        );
    }
    let explicit = reprime_options(1024, 369, 48, 32)
        .prepare_recovery(48_000, 256)
        .unwrap();
    let mut altered = explicit;
    altered.queue.as_mut().unwrap().low_water_frames = 0;
    assert!(altered.microphone_reprime(48, 1024).is_err());
    assert_eq!(
        explicit
            .admit_native_packet(48)
            .unwrap()
            .microphone_reprime(48, 1024),
        explicit.microphone_reprime(48, 1024)
    );
    assert!(
        explicit
            .admit_native_packet(48)
            .unwrap()
            .microphone_reprime(32, 1024)
            .is_err()
    );
}

#[test]
fn microphone_reprime_rejects_wrong_packet_capacity_and_malformed_budget() {
    use moiren_windows_audio::latency::LatencyBudgetError;
    let budget = reprime_options(1024, 369, 48, 32)
        .prepare_recovery(48_000, 256)
        .unwrap();
    assert_eq!(
        budget.microphone_reprime(0, 1024),
        Err(LatencyBudgetError::InvalidDeliveryBound)
    );
    assert!(budget.microphone_reprime(49, 1024).is_err());
    for capacity in [0, 47, 512, 1025] {
        assert!(budget.microphone_reprime(48, capacity).is_err());
    }
    let mut wrong_rate = budget;
    wrong_rate.engine_sample_rate = 0;
    assert!(wrong_rate.microphone_reprime(48, 1024).is_err());
    let mut missing_queue = budget;
    missing_queue.queue = None;
    assert!(missing_queue.microphone_reprime(48, 1024).is_err());
    let compatible = VirtualBufferOptions::default()
        .prepare_recovery(48_000, 64)
        .unwrap();
    assert!(compatible.microphone_reprime(480, 479).is_err());
}

#[test]
fn microphone_reprime_large_compatible_packet_does_not_wrap_doubling_or_fir_sum() {
    let budget = VirtualBufferOptions::default()
        .prepare_recovery(48_000, 64)
        .unwrap();
    let recovery = budget.microphone_reprime(u32::MAX, u32::MAX).unwrap();
    assert_eq!(recovery.required_frames, u32::MAX);
    let legacy = (u128::from(u32::MAX) * 5 * 10_000_000).div_ceil(48_000 * 2);
    assert_eq!(u128::from(recovery.max_wait_100ns), legacy);
}

#[test]
fn microphone_reprime_preparation_allocates_and_frees_nothing() {
    let budget = reprime_options(1024, 369, 48, 32)
        .prepare_recovery(48_000, 256)
        .unwrap();
    let recovery = alloc_guard::assert_allocates_nothing(|| budget.microphone_reprime(48, 1024));
    assert_eq!(recovery.unwrap().required_frames, 369);
}
