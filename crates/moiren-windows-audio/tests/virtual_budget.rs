use moiren_windows_audio::{
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    virtual_owner::VirtualBufferOptions,
};

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
