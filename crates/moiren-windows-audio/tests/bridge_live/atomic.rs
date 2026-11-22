pub use moiren_windows_audio::clock_bridge::{QueuedFrameAge, QueuedFrameStamp};
#[path = "../../src/clock_bridge/telemetry/live.rs"]
mod live;

#[test]
fn native_delivery_and_age_publish_snapshot_and_invalidation_are_allocation_free() {
    let delivery = live::DeliverySample::default();
    let age = live::AgeSample::default();
    let mut recorder = moiren_windows_audio::latency::InputDeliveryRecorder::new(None).unwrap();
    let sample = QueuedFrameAge {
        stamp: QueuedFrameStamp {
            generation: 5,
            packet_position_frames: 8,
            packet_qpc_100ns: 1_001_667,
            packet_accepted_frames: 8,
            packet_offset_frames: 0,
            rate_correction_ppm: Some(-1000),
        },
        observed_qpc_100ns: 1_020_000,
        input_sample_rate: 48_000,
        estimated_frame_qpc_100ns: 1_001_667,
        estimated_age_100ns: 18_333,
    };
    let (native, qualified, invalid) = crate::common::assert_allocates_nothing(|| {
        recorder.packet(8, Some(1_000_000));
        recorder.packet(8, Some(1_010_000));
        recorder.callback_burst(16);
        delivery.publish(recorder.snapshot());
        age.publish(Some(sample));
        let native = delivery.snapshot().unwrap();
        let qualified = age.snapshot().unwrap();
        age.publish(None);
        (native, qualified, age.snapshot())
    });
    assert_eq!(native.packets, 2);
    assert_eq!(native.max_callback_burst_frames, 16);
    assert_eq!(native.max_gap_100ns, 10_000);
    assert_eq!(qualified, sample);
    assert_eq!(invalid, None);
}
