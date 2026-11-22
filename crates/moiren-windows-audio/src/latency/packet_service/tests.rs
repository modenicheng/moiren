use super::*;

#[test]
fn service_clock_subtracts_raw_large_ticks_before_conservative_floor_and_ceil_conversion() {
    let recorder = PacketServiceRecorder::new(PacketStampOrigin::DriverTransfer, Some(3_000_001));
    let large = (1_u64 << 53) + 123;
    recorder.record(large, Some(large + 1));
    recorder.record(large, Some(large + 3));
    let s = recorder.snapshot();
    assert_eq!(s.origin, PacketStampOrigin::DriverTransfer);
    assert_eq!(s.qpc_frequency_hz, Some(3_000_001));
    assert_eq!(s.observations, 2);
    assert_eq!(s.min_delay_100ns, Some(3));
    assert_eq!(s.max_delay_100ns, Some(10));
    assert_eq!(s.unavailable_clocks, 0);
    assert_eq!(s.future_stamps, 0);
    assert_eq!(s.conversion_overflows, 0);
}

#[test]
fn missing_or_future_service_stamps_do_not_become_zero_delay_observations() {
    let r = PacketServiceRecorder::new(PacketStampOrigin::HostSubmission, Some(10_000_000));
    for (stamp, observed) in [(0, Some(100)), (100, None), (100, Some(0))] {
        r.record(stamp, observed);
    }
    r.record(101, Some(100));
    let s = r.snapshot();
    assert_eq!(s.observations, 0);
    assert_eq!(s.unavailable_clocks, 3);
    assert_eq!(s.future_stamps, 1);
    assert_eq!(s.min_delay_100ns, None);
    assert_eq!(s.max_delay_100ns, None);
    r.record(100, Some(100));
    assert_eq!(r.snapshot().min_delay_100ns, Some(0));
    assert_eq!(r.snapshot().max_delay_100ns, Some(0));
}

#[test]
fn service_frequency_unknown_or_zero_stays_unknown_and_arithmetic_overflow_is_separate() {
    for frequency in [None, Some(0)] {
        let r = PacketServiceRecorder::new(PacketStampOrigin::DriverTransfer, frequency);
        r.record(1, Some(2));
        let s = r.snapshot();
        assert_eq!(s.qpc_frequency_hz, None);
        assert_eq!(s.unavailable_clocks, 1);
        assert_eq!(s.observations, 0);
    }
    let r = PacketServiceRecorder::new(PacketStampOrigin::HostSubmission, Some(1));
    r.record(1, Some(u64::MAX));
    assert_eq!(r.snapshot().conversion_overflows, 1);
    assert_eq!(r.snapshot().observations, 0);
    assert_eq!(r.snapshot().min_delay_100ns, None);
}

#[test]
fn service_publication_is_bounded_allocation_free_and_saturating_without_retry() {
    use std::sync::atomic::Ordering::Relaxed;
    let r = PacketServiceRecorder::new(PacketStampOrigin::DriverTransfer, Some(10_000_000));
    let bytes = size_of_val(&r);
    for index in 1..=1000 {
        r.record(index, Some(index + 100));
    }
    assert_eq!(size_of_val(&r), bytes);
    assert_eq!(r.snapshot().observations, 1000);
    r.observations.store(u64::MAX, Relaxed);
    r.unavailable.store(u64::MAX, Relaxed);
    r.future.store(u64::MAX, Relaxed);
    r.overflows.store(u64::MAX, Relaxed);
    r.record(2, Some(3));
    r.record(0, None);
    r.record(3, Some(2));
    let s = r.snapshot();
    assert_eq!(s.observations, u64::MAX);
    assert_eq!(s.unavailable_clocks, u64::MAX);
    assert_eq!(s.future_stamps, u64::MAX);
    assert_eq!(s.conversion_overflows, u64::MAX);
}

#[test]
fn service_summary_json_has_exact_ticks_and_explicit_unknown_default_provenance() {
    let initial = PacketServiceSnapshot::default();
    assert_eq!(initial.origin, PacketStampOrigin::Unknown);
    assert_eq!(initial.qpc_frequency_hz, None);
    assert_eq!(initial.min_delay_100ns, None);
    let r = PacketServiceRecorder::new(PacketStampOrigin::HostSubmission, Some(10_000_000));
    let large = (1_u64 << 53) + 7;
    r.record(1, Some(large + 1));
    let json = serde_json::to_string(&r.snapshot()).unwrap();
    assert!(json.contains(&format!("\"max_delay_100ns\":{large}")));
    assert_eq!(
        serde_json::from_str::<PacketServiceSnapshot>(&json).unwrap(),
        r.snapshot()
    );
    assert_eq!(r.snapshot().origin, PacketStampOrigin::HostSubmission);
}
