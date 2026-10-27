//! Control-side snapshot windows, never the time of the retained RT observation.

use moiren_windows_audio::latency::qpc_ticks_to_100ns;
use serde::Serialize;

#[cfg(windows)]
pub(super) const SCOPE: &str = "control-side reads before provider QUERY and after all snapshots, \
    in WASAPI capture QPC 100ns units; not RT event times or proof that retained boundary probes \
    were refreshed (including during priming); absent endpoints remain unknown";

#[derive(Debug, Default, Serialize)]
pub(super) struct ControlObservationWindow {
    before_qpc_100ns: Option<u64>,
    after_qpc_100ns: Option<u64>,
}

pub(super) struct ControlObservationClock {
    frequency: Option<u64>,
    previous_ticks: Option<u64>,
}

impl ControlObservationClock {
    #[cfg(windows)]
    pub(super) fn prepare() -> Self {
        use windows::Win32::System::Performance::QueryPerformanceFrequency;
        let mut frequency = 0;
        // SAFETY: writable stack output; unavailable QPF leaves diagnostics unknown.
        let frequency = unsafe { QueryPerformanceFrequency(&mut frequency) }
            .ok()
            .map(|()| frequency);
        Self::from_frequency(frequency)
    }

    fn from_frequency(frequency: Option<i64>) -> Self {
        Self {
            frequency: frequency
                .and_then(|value| u64::try_from(value).ok())
                .filter(|value| *value > 0),
            previous_ticks: None,
        }
    }

    #[cfg(windows)]
    pub(super) fn observe<T>(
        &mut self,
        snapshot: impl FnOnce() -> T,
    ) -> (T, ControlObservationWindow) {
        use windows::Win32::System::Performance::QueryPerformanceCounter;
        self.observe_with(
            &mut || {
                let mut ticks = 0;
                // SAFETY: writable stack output, only on the probe's control thread.
                unsafe { QueryPerformanceCounter(&mut ticks) }
                    .ok()
                    .map(|()| ticks)
            },
            snapshot,
        )
    }

    fn observe_with<T>(
        &mut self,
        read: &mut impl FnMut() -> Option<i64>,
        snapshot: impl FnOnce() -> T,
    ) -> (T, ControlObservationWindow) {
        let before_qpc_100ns = self.read_with(read);
        let value = snapshot();
        let after_qpc_100ns = self.read_with(read);
        (
            value,
            ControlObservationWindow {
                before_qpc_100ns,
                after_qpc_100ns,
            },
        )
    }

    fn read_with(&mut self, read: &mut impl FnMut() -> Option<i64>) -> Option<u64> {
        let frequency = self.frequency?;
        let ticks = u64::try_from(read()?).ok()?;
        // Compare raw ticks so 100ns rounding cannot hide a regression. Once
        // regressed, this run cannot assert the clock domain is still coherent.
        if self.previous_ticks.is_some_and(|previous| ticks < previous) {
            self.frequency = None;
            return None;
        }
        self.previous_ticks = Some(ticks);
        qpc_ticks_to_100ns(ticks, frequency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn control_window_brackets_query_and_all_snapshots_in_capture_qpc_units() {
        let ticks = Cell::new(300);
        let mut clock = ControlObservationClock::from_frequency(Some(3_000_000));
        let (snapshots, window) = clock.observe_with(&mut || Some(ticks.get()), || {
            // Deterministic elapsed time models a blocking QUERY and two later
            // snapshots; the ending read must happen after all three.
            ticks.set(900);
            let provider = "query";
            ticks.set(1200);
            let engine = 480;
            ticks.set(1500);
            (provider, engine, "native")
        });
        assert_eq!(snapshots, ("query", 480, "native"));
        assert_eq!(window.before_qpc_100ns, Some(1000));
        assert_eq!(window.after_qpc_100ns, Some(5000));
        ticks.set(3000); // Later control work/serialization is outside the window.
        let retained = serde_json::to_value(window).unwrap();
        assert_eq!(retained["before_qpc_100ns"], 1000);
        assert_eq!(retained["after_qpc_100ns"], 5000);
    }

    #[test]
    fn unavailable_or_regressed_clock_does_not_invent_observation_time() {
        for frequency in [None, Some(0), Some(-1)] {
            let mut clock = ControlObservationClock::from_frequency(frequency);
            let (_, window) = clock.observe_with(&mut || Some(100), || ());
            assert!(window.before_qpc_100ns.is_none());
            assert!(window.after_qpc_100ns.is_none());
        }
        let mut clock = ControlObservationClock::from_frequency(Some(10_000_000));
        let mut ticks = [Some(100), Some(99)].into_iter();
        let (_, regressed) = clock.observe_with(&mut || ticks.next().unwrap(), || ());
        assert_eq!(regressed.before_qpc_100ns, Some(100));
        assert!(regressed.after_qpc_100ns.is_none());
        let (_, unavailable) =
            clock.observe_with(&mut || panic!("a regressed clock stays unknown"), || ());
        assert!(unavailable.before_qpc_100ns.is_none());
        assert!(unavailable.after_qpc_100ns.is_none());
    }

    #[test]
    fn missing_negative_or_overflowed_read_is_null_without_reusing_an_old_timestamp() {
        let mut clock = ControlObservationClock::from_frequency(Some(10_000_000));
        let mut ticks = [Some(100), Some(101), None, Some(-1), Some(200), None].into_iter();
        let mut read = || ticks.next().unwrap();
        let (_, first) = clock.observe_with(&mut read, || ());
        assert_eq!(first.after_qpc_100ns, Some(101));
        let (_, missing) = clock.observe_with(&mut read, || ());
        let json = serde_json::to_value(missing).unwrap();
        assert!(json["before_qpc_100ns"].is_null());
        assert!(json["after_qpc_100ns"].is_null());
        let (_, incomplete) = clock.observe_with(&mut read, || ());
        assert_eq!(incomplete.before_qpc_100ns, Some(200));
        assert!(incomplete.after_qpc_100ns.is_none());

        let mut clock = ControlObservationClock::from_frequency(Some(1));
        let (_, overflow) = clock.observe_with(&mut || Some(i64::MAX), || ());
        assert!(overflow.before_qpc_100ns.is_none());
        assert!(overflow.after_qpc_100ns.is_none());
    }

    #[test]
    fn raw_tick_regression_cannot_be_hidden_by_100ns_rounding() {
        let mut clock = ControlObservationClock::from_frequency(Some(40_000_000));
        let mut ticks = [Some(1001), Some(1000)].into_iter();
        let (_, window) = clock.observe_with(&mut || ticks.next().unwrap(), || ());
        assert_eq!(window.before_qpc_100ns, Some(250));
        assert!(window.after_qpc_100ns.is_none());
    }
}
