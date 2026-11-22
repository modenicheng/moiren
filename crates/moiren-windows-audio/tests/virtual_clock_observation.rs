use moiren_windows_audio::virtual_owner::ClockTick;

mod common;
use common as test_allocation;
#[cfg(windows)]
mod latency {
    pub use moiren_windows_audio::latency::QpcClock;
}
#[path = "../src/virtual_owner/callback_timing.rs"]
#[allow(dead_code)]
mod callback_timing;
// Exercise the production publisher under this binary's counting allocator.
#[path = "../src/virtual_owner/clock_observation.rs"]
#[allow(dead_code)]
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

#[test]
fn enabled_callback_publication_preserves_clock_accounting_without_heap_work() {
    let probe = VirtualClockProbe::new(48_000, 32, 1).with_callback_timing(true);
    let sample = callback_timing::VirtualCallbackSnapshot {
        attempts: 1,
        completed: 1,
        missing_clocks: 1,
        ..Default::default()
    };
    common::assert_allocates_nothing(|| {
        probe.callback_probe().unwrap().publish_for_test(sample);
        assert_eq!(probe.callback_snapshot(), Some(sample));
        assert!(probe.snapshot().is_none());
        probe.publish(VirtualClockCounters::default());
        assert_eq!(probe.callback_snapshot(), Some(sample));
    });
}
