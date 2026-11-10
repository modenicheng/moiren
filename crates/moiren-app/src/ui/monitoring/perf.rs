//! The performance panel's control-side data: a one-minute total-CPU history
//! sampled from the control pump, and the process table sampled off the UI
//! thread while the panel is open. Storage and cadence only — the system
//! reads and the ranking math live in [`crate::system`].

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::system::{CpuTotals, ProcessSample, ProcessUsage};

/// One point per second for one minute.
pub(crate) const HISTORY_LEN: usize = 60;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// Process rows listed after Moiren's own pinned row.
pub(crate) const PROCESS_ROWS: usize = 5;

#[derive(Debug, Default)]
pub(crate) struct PerfModel {
    history: VecDeque<f32>,
    totals: Option<(CpuTotals, Instant)>,
    processes: Vec<ProcessUsage>,
    process_table: Option<(Vec<ProcessSample>, Instant)>,
}

impl PerfModel {
    /// Advances the total-CPU history once a second has passed since the
    /// last sample. Returns whether a new point landed, so the pump only
    /// repaints when something visible changed.
    pub(crate) fn tick(&mut self, now: Instant) -> bool {
        if self
            .totals
            .is_some_and(|(_, at)| now.duration_since(at) < SAMPLE_INTERVAL)
        {
            return false;
        }
        let Some(totals) = crate::system::sample_cpu_totals() else {
            return false;
        };
        self.push_totals(totals, now)
    }

    /// The pure half of [`Self::tick`]: the first sample only primes the
    /// pair, every later one appends the busy share between the two.
    fn push_totals(&mut self, totals: CpuTotals, now: Instant) -> bool {
        let previous = self.totals.replace((totals, now));
        let Some(percent) =
            previous.and_then(|(previous, _)| crate::system::total_cpu_percent(&previous, &totals))
        else {
            return false;
        };
        self.history.push_back(percent);
        while self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        true
    }

    /// Total-CPU busy shares, oldest first.
    pub(crate) fn history(&self) -> &VecDeque<f32> {
        &self.history
    }

    pub(crate) fn current(&self) -> Option<f32> {
        self.history.back().copied()
    }

    /// Lands one process table sampled off the UI thread and re-ranks the
    /// rows against the previous table.
    pub(crate) fn ingest_processes(
        &mut self,
        table: Vec<ProcessSample>,
        now: Instant,
        cores: f32,
        own_pid: u32,
    ) {
        let (previous, elapsed) = match &self.process_table {
            Some((previous, at)) => (previous.as_slice(), now.duration_since(*at)),
            None => (&[][..], Duration::ZERO),
        };
        self.processes =
            crate::system::top_processes(previous, &table, elapsed, cores, own_pid, PROCESS_ROWS);
        self.process_table = Some((table, now));
    }

    pub(crate) fn processes(&self) -> &[ProcessUsage] {
        &self.processes
    }

    /// The panel closed: drop the table so reopening starts from a fresh
    /// pair instead of a share averaged over the closed interval.
    pub(crate) fn forget_processes(&mut self) {
        self.process_table = None;
        self.processes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_totals_prime_the_pair_and_later_ones_append() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        assert!(!model.push_totals(CpuTotals::synthetic(0, 0, 0), anchor));
        assert_eq!(model.current(), None, "one sample is not a rate");
        // One busy second: no idle growth inside the kernel time.
        let next = anchor + Duration::from_secs(1);
        assert!(model.push_totals(CpuTotals::synthetic(0, 5_000_000, 5_000_000), next));
        assert_eq!(model.current(), Some(100.));
        assert_eq!(model.history().len(), 1);
    }

    #[test]
    fn the_history_keeps_one_minute() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        for second in 0..=(HISTORY_LEN as u64 + 5) {
            // Half of each second idle: 50% busy, and kernel carries the idle.
            let elapsed = second * 10_000_000;
            model.push_totals(
                CpuTotals::synthetic(elapsed / 2, elapsed / 2, elapsed / 2),
                anchor + Duration::from_secs(second),
            );
        }
        assert_eq!(model.history().len(), HISTORY_LEN);
        assert!(
            model
                .history()
                .iter()
                .all(|percent| (percent - 50.).abs() < 1e-3),
            "every retained point is the steady half load"
        );
    }

    #[test]
    fn process_rows_rank_once_two_tables_exist_and_forget_clears_them() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let table = |cpu: u64| {
            vec![
                ProcessSample {
                    pid: 5,
                    name: "moiren-app".into(),
                    cpu_100ns: cpu,
                    mem_bytes: 9,
                },
                ProcessSample {
                    pid: 6,
                    name: "chrome".into(),
                    cpu_100ns: cpu * 3,
                    mem_bytes: 7,
                },
            ]
        };
        model.ingest_processes(table(0), anchor, 2., 5);
        assert_eq!(
            model.processes()[0].cpu_percent,
            0.,
            "one table has no rate"
        );
        assert_eq!(model.processes()[0].mem_bytes, 9, "memory is immediate");
        model.ingest_processes(table(2_000_000), anchor + Duration::from_secs(1), 2., 5);
        assert_eq!(model.processes()[0].pid, 5, "the own row stays pinned");
        assert!((model.processes()[0].cpu_percent - 10.).abs() < 1e-3);
        assert!((model.processes()[1].cpu_percent - 30.).abs() < 1e-3);
        model.forget_processes();
        assert!(model.processes().is_empty());
    }
}
