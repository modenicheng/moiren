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
