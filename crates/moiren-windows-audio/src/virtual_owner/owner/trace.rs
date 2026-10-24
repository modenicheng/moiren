//! Bounded owner gap evidence. Recording never formats, allocates, or logs;
//! the control joiner emits the retained tail after the realtime thread exits.

const CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum GapStage {
    /// Periods that arrived late and are being repaid (backlog minus one).
    #[default]
    Wake,
    /// Periods discarded beyond the debt cap: permanently unrendered frames.
    Clamp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Gap {
    qpc: u64,
    frame: u64,
    skipped: u64,
    stage: GapStage,
}

pub(super) struct ClockTrace {
    origin: u64,
    frequency: u64,
    gaps: [Gap; CAPACITY],
    cursor: usize,
    retained: usize,
    total: u64,
}

impl Default for ClockTrace {
    fn default() -> Self {
        Self {
            origin: 0,
            frequency: 0,
            gaps: [Gap::default(); CAPACITY],
            cursor: 0,
            retained: 0,
            total: 0,
        }
    }
}

impl ClockTrace {
    pub(super) fn start(&mut self, origin: u64, frequency: u64) {
        self.origin = origin;
        self.frequency = frequency;
    }

    pub(super) fn record(&mut self, qpc: u64, frame: u64, skipped: u64, stage: GapStage) {
        debug_assert!(skipped > 0);
        self.gaps[self.cursor] = Gap {
            qpc,
            frame,
            skipped,
            stage,
        };
        self.cursor = (self.cursor + 1) % CAPACITY;
        self.retained = (self.retained + 1).min(CAPACITY);
        self.total = self.total.saturating_add(1);
    }

    fn events(&self) -> impl Iterator<Item = Gap> + '_ {
        let first = (self.cursor + CAPACITY - self.retained) % CAPACITY;
        (0..self.retained).map(move |offset| self.gaps[(first + offset) % CAPACITY])
    }

    #[cfg(test)]
    pub(super) fn observed(&self) -> impl Iterator<Item = (u64, u64, u64, GapStage)> + '_ {
        self.events()
            .map(|gap| (gap.qpc, gap.frame, gap.skipped, gap.stage))
    }

    pub(super) fn emit(&self) {
        if self.frequency == 0 {
            return;
        }
        log::debug!(
            target: "moiren::audio::vad",
            "event=virtual_clock_gap_summary origin_qpc={} frequency={} total={} retained={}",
            self.origin,
            self.frequency,
            self.total,
            self.retained,
        );
        for gap in self.events() {
            log::debug!(
                target: "moiren::audio::vad",
                "event=virtual_clock_gap qpc={} elapsed_us={} frame={} skipped={} stage={:?}",
                gap.qpc,
                u128::from(gap.qpc.saturating_sub(self.origin)) * 1_000_000
                    / u128::from(self.frequency),
                gap.frame,
                gap.skipped,
                gap.stage,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_trace_retains_only_the_latest_bounded_events_in_time_order() {
        let mut trace = ClockTrace::default();
        trace.start(100, 48_000);
        for index in 0..100_u64 {
            trace.record(100 + index, index * 256, index + 1, GapStage::Wake);
        }
        assert_eq!(trace.total, 100);
        assert_eq!(trace.retained, CAPACITY);
        let mut events = trace.events();
        for index in 36..100_u64 {
            assert_eq!(
                events.next(),
                Some(Gap {
                    qpc: 100 + index,
                    frame: index * 256,
                    skipped: index + 1,
                    stage: GapStage::Wake,
                })
            );
        }
        assert_eq!(events.next(), None);
    }

    #[test]
    fn gap_trace_distinguishes_wake_and_clamp_without_reordering() {
        let mut trace = ClockTrace::default();
        trace.start(10, 48_000);
        trace.record(20, 0, 2, GapStage::Wake);
        trace.record(30, 256, 1, GapStage::Clamp);
        assert_eq!(
            trace.events().map(|gap| gap.stage).collect::<Vec<_>>(),
            [GapStage::Wake, GapStage::Clamp]
        );
    }
}
