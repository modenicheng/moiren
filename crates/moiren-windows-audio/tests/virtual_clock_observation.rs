use moiren_windows_audio::virtual_owner::ClockTick;

mod common;
// Exercise the production publisher under this binary's counting allocator.
#[path = "../src/virtual_owner/clock_observation.rs"]
mod clock_observation;
use clock_observation::{VirtualClockCounters, VirtualClockProbe};

#[test]
fn clock_accounting_publication_and_reading_never_allocate_or_deallocate() {
    let probe = VirtualClockProbe::new(48_000, 32, 1);
    #[cfg(windows)]
    assert!(probe.matches(48_000, 32, 1));
    let mut counters = VirtualClockCounters::default();
    common::assert_allocates_nothing(|| {
        for _ in 0..100 {
            counters.tick(
                ClockTick {
                    frames: 32,
                    backlog_periods: 1,
                    clamped_periods: 2,
                },
                true,
                false,
            );
            counters.rendered(32);
            probe.publish(counters);
            assert_eq!(probe.snapshot().unwrap().counters, counters);
        }
    });
    assert_eq!(probe.snapshot().unwrap().counters.clamped_frames, 6400);
}
