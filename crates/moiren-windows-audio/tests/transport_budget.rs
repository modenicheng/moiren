use moiren_windows_audio::latency::{
    LatencyBudgetError,
    transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy, prepare_transport_input,
    },
};

fn delivery() -> InputDeliveryTiming {
    InputDeliveryTiming {
        sample_rate: 48_000,
        max_packet_frames: 128,
        max_gap_frames: 128,
        source: DeliveryTimingSource::Empirical,
    }
}

fn target(capacity: u32, frames: u32) -> TransportInputPolicy {
    TransportInputPolicy::Target {
        capacity_frames: capacity,
        target_frames: frames,
        jitter_frames: 32,
    }
}

#[test]
fn spare_capacity_does_not_increase_prime_or_servo_residency() {
    let small =
        prepare_transport_input(target(1024, 320), Some(delivery()), 48_000, 128, 16).unwrap();
    let large =
        prepare_transport_input(target(8192, 320), Some(delivery()), 48_000, 128, 16).unwrap();
    assert_eq!(small.prime_frames, 320);
    assert_eq!(small.servo_reference_frames, 320);
    assert_eq!(large.prime_frames, small.prime_frames);
    assert_eq!(large.servo_reference_frames, small.servo_reference_frames);
    assert_eq!(small.low_water_frames, 144);
    assert_eq!(small.modeled_minimum_frames, Some(304));
    assert_eq!(small.high_water_frames, 896);
    assert_eq!(large.high_water_frames, 8064);
}

#[test]
fn delivery_burst_phase_and_jitter_must_fit_target_and_capacity() {
    assert_eq!(
        prepare_transport_input(target(1024, 303), Some(delivery()), 48_000, 128, 16),
        Err(LatencyBudgetError::InsufficientInputBudget)
    );
    assert_eq!(
        prepare_transport_input(target(432, 304), Some(delivery()), 48_000, 128, 16),
        Err(LatencyBudgetError::InvalidInputTarget)
    );
    assert!(prepare_transport_input(target(433, 304), Some(delivery()), 48_000, 128, 16).is_ok());
}

#[test]
fn process_delivery_is_explicitly_empirical_and_never_a_hardware_period() {
    let timing = delivery();
    let serialized = serde_json::to_value(timing).unwrap();
    assert_eq!(serialized["source"], "empirical");
    assert!(serialized.get("period_frames").is_none());
    assert_eq!(
        prepare_transport_input(target(8192, 1024), None, 48_000, 128, 2),
        Err(LatencyBudgetError::MissingDeliveryBound)
    );
    let compatible =
        prepare_transport_input(TransportInputPolicy::Compatible, None, 48_000, 128, 2).unwrap();
    assert_eq!(compatible.capacity_frames, 8192);
    assert_eq!(compatible.prime_frames, 2048);
    assert_eq!(compatible.modeled_minimum_frames, None);
}

#[test]
fn demand_converts_upward_in_native_input_frames_at_fractional_rates() {
    for (input_rate, output_rate, demand, expected) in [
        (44_100, 48_000, 128, 118),
        (48_000, 44_100, 128, 140),
        (96_000, 48_000, 128, 256),
    ] {
        let timing = InputDeliveryTiming {
            sample_rate: input_rate,
            ..delivery()
        };
        let queue =
            prepare_transport_input(target(8192, 1024), Some(timing), output_rate, demand, 16)
                .unwrap();
        assert_eq!(queue.low_water_frames, expected + 16);
        assert_eq!(queue.modeled_minimum_frames, Some(expected + 16 + 128 + 32));
    }
}

#[test]
fn invalid_or_overflowing_bounds_are_refused_without_panicking() {
    for timing in [
        InputDeliveryTiming {
            sample_rate: 0,
            ..delivery()
        },
        InputDeliveryTiming {
            max_packet_frames: 0,
            ..delivery()
        },
        InputDeliveryTiming {
            max_gap_frames: 0,
            ..delivery()
        },
    ] {
        assert_eq!(
            prepare_transport_input(target(8192, 1024), Some(timing), 48_000, 128, 2),
            Err(LatencyBudgetError::InvalidDeliveryBound)
        );
    }
    assert!(prepare_transport_input(target(8192, 1024), Some(delivery()), 0, 128, 2).is_err());
    assert!(prepare_transport_input(target(8192, 1024), Some(delivery()), 48_000, 0, 2).is_err());
    assert_eq!(
        prepare_transport_input(
            target(u32::MAX, u32::MAX - 129),
            Some(InputDeliveryTiming {
                sample_rate: u32::MAX,
                ..delivery()
            }),
            1,
            u32::MAX,
            2,
        ),
        Err(LatencyBudgetError::ArithmeticOverflow)
    );
}
