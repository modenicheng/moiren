use moiren_windows_audio::latency::{
    InputDeliveryRecorder,
    transport::{DeliveryTimingSource, InputDeliveryTiming},
};

#[test]
fn actual_arrival_gaps_and_packets_can_disqualify_a_declared_bound() {
    let mut recorder = InputDeliveryRecorder::new(Some(InputDeliveryTiming {
        sample_rate: 48_000,
        max_packet_frames: 128,
        max_gap_frames: 144,
        source: DeliveryTimingSource::Declared,
    }))
    .unwrap();
    recorder.packet(128, Some(1));
    recorder.packet(128, Some(30_001));
    recorder.packet(240, Some(60_002));
    recorder.callback_burst(368);
    let snapshot = recorder.snapshot();
    assert_eq!(snapshot.packets, 3);
    assert_eq!(snapshot.max_packet_frames, 240);
    assert_eq!(snapshot.max_callback_burst_frames, 368);
    assert_eq!(snapshot.gap_samples, 2);
    assert_eq!(snapshot.max_gap_100ns, 30_001);
    assert_eq!(snapshot.packet_bound_violations, 1);
    assert_eq!(snapshot.gap_bound_violations, 1);
}

#[test]
fn missing_and_regressed_clocks_never_fabricate_delivery_intervals() {
    let mut recorder = InputDeliveryRecorder::new(None).unwrap();
    recorder.packet(128, None);
    recorder.packet(128, Some(100));
    recorder.packet(128, Some(99));
    recorder.packet(0, Some(u64::MAX));
    recorder.packet(128, Some(0));
    recorder.packet(128, Some(u64::MAX));
    let snapshot = recorder.snapshot();
    assert_eq!(snapshot.packets, 5);
    assert_eq!(snapshot.gap_samples, 0);
    assert_eq!(snapshot.max_gap_100ns, 0);
    assert_eq!(snapshot.regressed_clocks, 1);
    assert_eq!(snapshot.unavailable_clocks, 2);
    assert_eq!(snapshot.gap_bound_violations, 0);
}

#[test]
fn invalid_delivery_limits_cannot_become_observed_evidence() {
    assert!(
        InputDeliveryRecorder::new(Some(InputDeliveryTiming {
            sample_rate: 0,
            max_packet_frames: 128,
            max_gap_frames: 128,
            source: DeliveryTimingSource::Declared,
        }))
        .is_err()
    );
}
