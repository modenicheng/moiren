#[path = "../src/latency/packet_service.rs"]
#[allow(dead_code)]
mod packet_service;
use packet_service::{PacketServiceRecorder, PacketStampOrigin};

#[path = "common/alloc_guard.rs"]
mod alloc_guard;

#[test]
fn packet_service_updates_and_live_snapshots_allocate_and_free_nothing() {
    let r = PacketServiceRecorder::new(PacketStampOrigin::DriverTransfer, Some(3_000_001));
    alloc_guard::assert_allocates_nothing(|| {
        for n in 1..=1000 {
            r.record(n, Some(n + 5));
            r.record(0, None);
            r.record(n + 1, Some(n));
            let _ = r.snapshot();
        }
    });
    let s = r.snapshot();
    assert_eq!(s.observations, 1000);
    assert_eq!(s.unavailable_clocks, 1000);
    assert_eq!(s.future_stamps, 1000);
}
