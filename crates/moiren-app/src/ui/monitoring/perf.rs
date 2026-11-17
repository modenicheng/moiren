//! The performance panel's control-side data: a one-minute total-CPU history
//! sampled from the control pump and the per-thread table sampled off the UI
//! thread while the panel is open. Storage and cadence only — the system
//! reads and the delta math live in [`crate::system`] and this file's pure
//! helpers.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::system::{CpuTotals, ThreadSample};
use crate::ui::monitoring::runtime::RenderCounters;
use moiren_engine::threads::{ThreadClass, ThreadInfo};

/// One point per second for one minute.
pub(crate) const HISTORY_LEN: usize = 60;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub(crate) struct PerfModel {
    history: VecDeque<f32>,
    totals: Option<(CpuTotals, Instant)>,
    /// The per-thread panel model (see [`ThreadPerf`]).
    thread_perf: ThreadPerf,
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

    /// Lands one per-thread sample: OS thread accounting, the live registry
    /// rows, the control-side probe reads, and the master stream's cumulative
    /// RT counters from the observation ingest path. The first call only
    /// primes baselines; every later one turns deltas into the panel's rows.
    pub(crate) fn ingest_threads(
        &mut self,
        samples: Vec<ThreadSample>,
        registry: Vec<ThreadInfo>,
        probes: ProbeRead,
        rt: Option<(u64, RenderCounters)>,
        now: Instant,
        cores: f32,
    ) {
        self.thread_perf
            .ingest(samples, registry, probes, rt, now, cores);
    }

    /// The per-thread rows, grouped by the model for the panel.
    pub(crate) fn threads(&self) -> &ThreadPerf {
        &self.thread_perf
    }

    /// The panel closed: thread baselines go the same way as the process
    /// table, so reopening rebases instead of averaging over the gap.
    pub(crate) fn forget_threads(&mut self) {
        self.thread_perf = ThreadPerf::default();
    }

    /// The panel's reset action: every UI-side baseline and event count
    /// restarts at the next sample; the RT counters are never touched.
    pub(crate) fn reset_threads(&mut self) {
        self.thread_perf.reset();
    }
}

// --- Per-thread sampling -----------------------------------------------------

/// The control-side probe reads the sampler gathers in one tick: the
/// service loop's counters and the production frame counters. All plain
/// statics/fields — none of this rides `AppSnapshot`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProbeRead {
    pub service_wakes: u64,
    pub service_round_us: u64,
    pub shell_renders: usize,
    pub graph_frames: usize,
}

/// One per-thread row as the panel draws it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ThreadRow {
    pub tid: u32,
    pub name: String,
    pub class: ThreadClass,
    /// CPU share of one core (0..=100) over the last sampling interval;
    /// `None` until the row has a completed baseline pair.
    pub cpu_percent: Option<f32>,
    /// The row's rate figure: master RT rows carry blocks/s, the service row
    /// wake/s, the UI row fps. `None` = no figure for this row.
    pub rate: Option<f32>,
    /// The rate's caption: the service round's work duration, the graph
    /// page's meter-frame fps.
    pub detail: Option<String>,
    /// RT over-budget events in the 60 s window (master rows only).
    pub over_budget: u64,
}

/// The per-thread panel model: OS CPU baselines joined against the thread
/// registry, the RT counters' deltas, and the two sparklines' histories.
///
/// Baseline discipline mirrors `XrunStats`: a tid unseen in the latest
/// sample is retired; a tid reappearing with a different creation time is a
/// different thread (Windows reuses ids) and only rebases; the panel's reset
/// moves every UI-side baseline without touching any RT counter.
#[derive(Debug, Default)]
pub(crate) struct ThreadPerf {
    /// CPU baselines per tid: creation time, kernel+user 100 ns, sampled at.
    baselines: BTreeMap<u32, ThreadBaseline>,
    /// The live rows the panel renders, registry order preserved.
    rows: Vec<ThreadRow>,
    /// Unknown OS threads' CPU total and count (the Other group).
    other_percent: Option<f32>,
    other_threads: usize,
    /// Master RT accounting baseline: (generation, cumulative counters).
    rt_seen: Option<(u64, RenderCounters)>,
    /// Over-budget events (when, how many in that interval) in the window.
    over_budget: VecDeque<(Instant, u64)>,
    /// Whether the driver's callbacks advanced over the last RT pair: the
    /// attribution evidence behind the ASIO callback note.
    native_callbacks_advanced: bool,
    /// The master RT row's sparkline. Every row drawing its own would own
    /// the frame pipeline; the bottom total rides the retained system chart.
    master_history: VecDeque<f32>,
    /// Probe rates and figures of the last tick.
    service_wake_rate: Option<f32>,
    service_round_us: Option<u64>,
    ui_fps: Option<f32>,
    graph_fps: Option<f32>,
    /// Probe baselines: wake count and the (shell, graph) frame counters.
    wakes_seen: Option<u64>,
    frames_seen: Option<(usize, usize)>,
    /// The RT tick's master blocks rate.
    master_blocks_per_s: Option<f32>,
    /// Per-counter tick stamps: each rate gets its own denominator.
    rt_tick_at: Option<Instant>,
    wake_tick_at: Option<Instant>,
    shell_tick_at: Option<Instant>,
    graph_tick_at: Option<Instant>,
    os_tick_at: Option<Instant>,
    /// Whether one OS sample has landed (the panel's "Sampling…" note).
    sampled: bool,
    /// Whether OS thread accounting exists at all: false stays false on
    /// platforms without the sampler (the panel's degrade note).
    os_data: bool,
}

/// The RT rows that pace `Engine::render` for the master output: the
/// professional master, or the virtual owner for software-only graphs.
const MASTER_ROW_NAMES: [&str; 2] = ["moiren-professional-master", "moiren-virtual-owner"];
const SERVICE_ROW_NAME: &str = "app-service";
const UI_ROW_NAME: &str = "moiren-ui-main";
/// The ⚠ window: like the XRUN strip, it reads the last minute.
const OVER_BUDGET_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
struct ThreadBaseline {
    create_time_100ns: i64,
    cpu_100ns: u64,
}

impl ThreadPerf {
    fn ingest(
        &mut self,
        samples: Vec<ThreadSample>,
        registry: Vec<ThreadInfo>,
        probes: ProbeRead,
        rt: Option<(u64, RenderCounters)>,
        now: Instant,
        cores: f32,
    ) {
        self.note_rt(rt, now);
        self.note_probes(probes, now);
        let rows = self.compute_rows(&samples, registry, now, cores);
        self.rows = rows;
        self.sampled = true;
    }

    /// The RT counters' deltas: blocks/s for the master row, the callback
    /// delta for the ASIO attribution note, over-budget events into the 60 s
    /// window. A generation change (or the first read) only rebases —
    /// startup priming must never surface as a burst.
    fn note_rt(&mut self, rt: Option<(u64, RenderCounters)>, now: Instant) {
        let Some((generation, counters)) = rt else {
            // No observations: the session stopped. Stale figures go with it,
            // the window keeps its events until the reset or the expiry.
            self.rt_seen = None;
            self.master_blocks_per_s = None;
            self.native_callbacks_advanced = false;
            return;
        };
        let previous = self.rt_seen.replace((generation, counters));
        let Some((previous_generation, previous_counters)) = previous else {
            // First read: prime the rate stamp; the next pair owns an interval.
            self.rt_tick_at = Some(now);
            return;
        };
        if previous_generation != generation {
            // Session generation changed: counters restarted under new
            // ownership, the window's meaning ended — rebase, no burst.
            self.over_budget.clear();
            self.master_blocks_per_s = None;
            self.native_callbacks_advanced = false;
            self.rt_tick_at = Some(now);
            return;
        }
        self.native_callbacks_advanced =
            counters.native_callbacks > previous_counters.native_callbacks;
        let over_budget_delta = counters
            .over_budget
            .saturating_sub(previous_counters.over_budget);
        if over_budget_delta > 0 {
            self.over_budget.push_back((now, over_budget_delta));
        }
        self.master_blocks_per_s = thread_rate(
            previous_counters.blocks,
            counters.blocks,
            now,
            &mut self.rt_tick_at,
        );
    }

    /// Probe rates for this tick, each denominatored by its own interval.
    fn note_probes(&mut self, probes: ProbeRead, now: Instant) {
        self.service_round_us = (probes.service_round_us > 0).then_some(probes.service_round_us);
        let previous_wakes = self.wakes_seen.replace(probes.service_wakes);
        let previous_frames = self
            .frames_seen
            .replace((probes.shell_renders, probes.graph_frames));
        if previous_wakes.is_none() && previous_frames.is_none() {
            // First read: prime the stamps; the rates start on the next pair.
            self.wake_tick_at = Some(now);
            self.shell_tick_at = Some(now);
            self.graph_tick_at = Some(now);
            return;
        }
        if let Some(previous_wakes) = previous_wakes {
            self.service_wake_rate = thread_rate(
                previous_wakes,
                probes.service_wakes,
                now,
                &mut self.wake_tick_at,
            );
        }
        if let Some((previous_shell, previous_graph)) = previous_frames {
            self.ui_fps = frame_rate(
                previous_shell,
                probes.shell_renders,
                now,
                &mut self.shell_tick_at,
            );
            self.graph_fps = frame_rate(
                previous_graph,
                probes.graph_frames,
                now,
                &mut self.graph_tick_at,
            );
        }
    }

    /// Joins the OS samples against the registry: registered tids become
    /// rows (CPU `—` until a matched sample pair exists — on non-Windows
    /// there is no OS accounting at all, so rows degrade to names), unknown
    /// tids sum into the Other group, and a tid with no sample retires its
    /// baseline. No zombie rows: a thread that exits loses its registry row
    /// and its sample within one tick.
    fn compute_rows(
        &mut self,
        samples: &[ThreadSample],
        registry: Vec<ThreadInfo>,
        now: Instant,
        cores: f32,
    ) -> Vec<ThreadRow> {
        let elapsed = self
            .os_tick_at
            .replace(now)
            .map_or(Duration::ZERO, |previous| now.duration_since(previous));
        let mut known: BTreeMap<u32, &ThreadSample> = samples
            .iter()
            .filter(|sample| sample.tid > 0)
            .map(|sample| (sample.tid, sample))
            .collect();
        if !samples.is_empty() {
            self.os_data = true;
        }
        let mut rows = Vec::new();
        for info in &registry {
            if info.tid == 0 {
                // Registered but the thread has not entered yet: no row
                // until it does (avoid rendering a not-yet-born thread).
                continue;
            }
            let sample = known.remove(&info.tid);
            let cpu = sample.and_then(|sample| {
                let total = sample.kernel_100ns.saturating_add(sample.user_100ns);
                let matched = self
                    .baselines
                    .get(&info.tid)
                    .is_some_and(|previous| previous.create_time_100ns == sample.create_time_100ns);
                // Matched pair: rate + move the baseline; new or reused
                // tid (Windows recycles ids): prime/rebase, no rate.
                let percent = matched
                    .then(|| {
                        thread_cpu_percent(
                            self.baselines[&info.tid].cpu_100ns,
                            total,
                            elapsed,
                            cores,
                        )
                    })
                    .flatten();
                self.baselines.insert(
                    info.tid,
                    ThreadBaseline {
                        create_time_100ns: sample.create_time_100ns,
                        cpu_100ns: total,
                    },
                );
                percent
            });
            if sample.is_none() {
                // No OS sample for this tid: it exited (or the platform has
                // no accounting) — the baseline goes with it.
                self.baselines.remove(&info.tid);
            }
            rows.push(ThreadRow {
                tid: info.tid,
                name: info.name.clone(),
                class: info.class,
                cpu_percent: cpu,
                rate: self.row_rate(&info.name),
                detail: self.row_detail(&info.name),
                over_budget: self.over_budget_count(now),
            });
        }
        // Leftover OS threads nobody registered: the Other group's total.
        self.other_threads = known.len();
        let mut other = 0.;
        let mut any = false;
        for sample in known.values() {
            let total = sample.kernel_100ns.saturating_add(sample.user_100ns);
            let percent = match self.baselines.get(&sample.tid) {
                Some(previous) if previous.create_time_100ns == sample.create_time_100ns => {
                    thread_cpu_percent(previous.cpu_100ns, total, elapsed, cores)
                }
                _ => None,
            };
            self.baselines.insert(
                sample.tid,
                ThreadBaseline {
                    create_time_100ns: sample.create_time_100ns,
                    cpu_100ns: total,
                },
            );
            if let Some(percent) = percent {
                other += percent;
                any = true;
            }
        }
        self.other_percent = any.then_some(other);
        self.update_histories(&rows);
        rows
    }

    /// The rate figure one named row carries this tick.
    fn row_rate(&self, name: &str) -> Option<f32> {
        match name {
            SERVICE_ROW_NAME => self.service_wake_rate,
            UI_ROW_NAME => self.ui_fps,
            _ if MASTER_ROW_NAMES.contains(&name) => self.master_blocks_per_s,
            _ => None,
        }
    }

    /// The caption under a named row's rate.
    fn row_detail(&self, name: &str) -> Option<String> {
        match name {
            SERVICE_ROW_NAME => self
                .service_round_us
                .map(|micros| format!("{:.1} ms", micros as f32 / 1000.)),
            UI_ROW_NAME => self.graph_fps.map(|fps| format!("meter {fps:.0} fps")),
            _ => None,
        }
    }

    /// Over-budget events still inside the 60 s window, expired ones dropped.
    fn over_budget_count(&mut self, now: Instant) -> u64 {
        while self
            .over_budget
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > OVER_BUDGET_WINDOW)
        {
            self.over_budget.pop_front();
        }
        self.over_budget.iter().map(|(_, count)| count).sum()
    }

    /// The master RT row's sparkline, one point per sample. The Other group
    /// is a sum of strangers, not a thread to watch.
    fn update_histories(&mut self, rows: &[ThreadRow]) {
        let master = rows
            .iter()
            .find(|row| MASTER_ROW_NAMES.contains(&row.name.as_str()))
            .and_then(|row| row.cpu_percent);
        push_history(&mut self.master_history, master);
    }

    /// The panel's reset action: baselines and event counts restart now, the
    /// RT counters are never touched. The rendered rows drop their figures
    /// at once (the next sample re-primes them) instead of carrying stale
    /// numbers for up to a second.
    pub(crate) fn reset(&mut self) {
        for row in &mut self.rows {
            row.cpu_percent = None;
            row.rate = None;
            row.detail = None;
            row.over_budget = 0;
        }
        self.baselines.clear();
        self.rt_seen = None;
        self.over_budget.clear();
        self.wakes_seen = None;
        self.frames_seen = None;
        self.master_blocks_per_s = None;
        self.service_wake_rate = None;
        self.ui_fps = None;
        self.graph_fps = None;
        self.master_history.clear();
        self.other_percent = None;
        self.os_tick_at = None;
        self.rt_tick_at = None;
        self.wake_tick_at = None;
        self.shell_tick_at = None;
        self.graph_tick_at = None;
    }

    pub(crate) fn rows(&self) -> &[ThreadRow] {
        &self.rows
    }

    /// Unknown OS threads: CPU total and count (the Other group).
    pub(crate) fn other(&self) -> (Option<f32>, usize) {
        (self.other_percent, self.other_threads)
    }

    /// Whether the master stream's driver callbacks advanced (ASIO note).
    pub(crate) fn native_callbacks_advanced(&self) -> bool {
        self.native_callbacks_advanced
    }

    /// The master row's sparkline history, oldest first.
    pub(crate) fn master_history(&self) -> &VecDeque<f32> {
        &self.master_history
    }

    /// Whether one OS sample landed (the panel's "Sampling…" state).
    pub(crate) fn sampled(&self) -> bool {
        self.sampled
    }

    /// Whether OS thread accounting ever arrived; false on platforms
    /// without the sampler (the panel's degrade note).
    pub(crate) fn os_data(&self) -> bool {
        self.os_data
    }
}

/// One thread's CPU share of one core (0..=100) between two samples. `None`
/// while no time passed — a rate without a denominator is a lie, the same
/// discipline as `dsp_load`. The share divides by `cores` so a fully busy
/// core reads 100%, matching the process column's machine-relative scale.
fn thread_cpu_percent(
    previous_100ns: u64,
    current_100ns: u64,
    elapsed: Duration,
    cores: f32,
) -> Option<f32> {
    let seconds = elapsed.as_secs_f32();
    if seconds <= 0. || cores <= 0. {
        return None;
    }
    let delta = current_100ns.saturating_sub(previous_100ns);
    Some((delta as f32 / (seconds * 1e7 * cores) * 100.).clamp(0., 100.))
}

/// A counter's per-second rate between two reads, denominatored by the
/// interval since the previous read of the same counter. `None` until two
/// reads exist or while no time passed.
fn thread_rate(
    previous: u64,
    current: u64,
    now: Instant,
    tick: &mut Option<Instant>,
) -> Option<f32> {
    let elapsed = tick.replace(now).map_or(Duration::ZERO, |previous_at| {
        now.duration_since(previous_at)
    });
    if elapsed.is_zero() {
        return None;
    }
    Some(current.saturating_sub(previous) as f32 / elapsed.as_secs_f32())
}

/// Frames per second between two counter reads; `None` until two reads and
/// time exist.
fn frame_rate(
    previous: usize,
    current: usize,
    now: Instant,
    tick: &mut Option<Instant>,
) -> Option<f32> {
    thread_rate(previous as u64, current as u64, now, tick)
}

/// Appends one point, keeping one minute.
fn push_history(history: &mut VecDeque<f32>, point: Option<f32>) {
    let Some(point) = point else {
        return;
    };
    history.push_back(point);
    while history.len() > HISTORY_LEN {
        history.pop_front();
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

    // --- Per-thread model ---------------------------------------------------

    fn info(tid: u32, name: &str, class: ThreadClass) -> ThreadInfo {
        ThreadInfo {
            tid,
            name: name.to_string(),
            class,
        }
    }

    fn sample(tid: u32, kernel: u64, user: u64, create: i64) -> ThreadSample {
        ThreadSample {
            tid,
            kernel_100ns: kernel,
            user_100ns: user,
            create_time_100ns: create,
        }
    }

    fn rt(
        generation: u64,
        blocks: u64,
        over_budget: u64,
        callbacks: u64,
    ) -> Option<(u64, RenderCounters)> {
        Some((
            generation,
            RenderCounters {
                blocks,
                busy_100ns: 0,
                peak_block_100ns: 0,
                over_budget,
                native_callbacks: callbacks,
            },
        ))
    }

    /// One second on two cores is 2e7 units of capacity per core.
    const CORES: f32 = 2.;

    #[test]
    fn thread_cpu_percent_compares_the_delta_with_elapsed_capacity() {
        // 1e6 units over one second on two cores: 5% of one core.
        let percent = thread_cpu_percent(0, 1_000_000, Duration::from_secs(1), CORES).unwrap();
        assert!((percent - 5.).abs() < 1e-3, "{percent}");
        // No time passed: no rate, not 0%.
        assert_eq!(
            thread_cpu_percent(0, 1_000_000, Duration::ZERO, CORES),
            None
        );
        // A counter reset never reads negative.
        assert_eq!(
            thread_cpu_percent(9_000_000, 1_000_000, Duration::from_secs(1), CORES),
            Some(0.)
        );
    }

    #[test]
    fn the_first_sample_primes_rows_and_the_second_computes_shares() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![
            info(10, "moiren-professional-master", ThreadClass::Rt),
            info(11, "app-service", ThreadClass::Control),
        ];
        let probes = ProbeRead::default();
        model.ingest_threads(
            vec![sample(10, 0, 0, 1_000), sample(11, 0, 0, 2_000)],
            registry.clone(),
            probes,
            rt(7, 0, 0, 0),
            anchor,
            CORES,
        );
        let rows = model.threads().rows();
        assert_eq!(rows.len(), 2, "every registered thread is a row");
        assert_eq!(rows[0].cpu_percent, None, "priming shows no rate");
        assert_eq!(rows[0].name, "moiren-professional-master");
        assert_eq!(rows[0].class, ThreadClass::Rt);
        model.ingest_threads(
            vec![
                sample(10, 500_000, 500_000, 1_000),
                sample(11, 10_000, 10_000, 2_000),
            ],
            registry,
            probes,
            rt(7, 48_000, 0, 0),
            anchor + Duration::from_secs(1),
            CORES,
        );
        let rows = model.threads().rows();
        // 1e6 units over one second on two cores: 5%.
        assert!((rows[0].cpu_percent.unwrap() - 5.).abs() < 1e-3);
        // The master's rate figure is the block delta per second.
        assert!((rows[0].rate.unwrap() - 48_000.).abs() < 1e-2);
        // The service row carries no RT rate; zero wake delta reads 0/s.
        assert_eq!(rows[1].rate, Some(0.));
    }

    #[test]
    fn a_reused_tid_rebases_instead_of_producing_a_bogus_delta() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![info(10, "moiren-professional-master", ThreadClass::Rt)];
        model.ingest_threads(
            vec![sample(10, 0, 0, 1_000)],
            registry.clone(),
            ProbeRead::default(),
            None,
            anchor,
            CORES,
        );
        // Same tid, different creation time: a different thread. No rate.
        model.ingest_threads(
            vec![sample(10, 9_999_999, 0, 2_000)],
            registry.clone(),
            ProbeRead::default(),
            None,
            anchor + Duration::from_secs(1),
            CORES,
        );
        let rows = model.threads().rows();
        assert_eq!(rows[0].cpu_percent, None, "rebased on the create time");
        // The next pair measures from the new thread's own baseline.
        model.ingest_threads(
            vec![sample(10, 10_999_999, 0, 2_000)],
            registry,
            ProbeRead::default(),
            None,
            anchor + Duration::from_secs(2),
            CORES,
        );
        let rows = model.threads().rows();
        assert!((rows[0].cpu_percent.unwrap() - 5.).abs() < 1e-3);
    }

    #[test]
    fn exited_threads_lose_their_rows_without_zombie_lines() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![
            info(10, "moiren-professional-master", ThreadClass::Rt),
            info(11, "moiren-professional-device", ThreadClass::Rt),
        ];
        model.ingest_threads(
            vec![sample(10, 0, 0, 1), sample(11, 0, 0, 2)],
            registry.clone(),
            ProbeRead::default(),
            None,
            anchor,
            CORES,
        );
        assert_eq!(model.threads().rows().len(), 2);
        // The session stopped: both registry rows and both samples go. The
        // RT group collapses; no stale rows survive.
        model.ingest_threads(
            Vec::new(),
            Vec::new(),
            ProbeRead::default(),
            None,
            anchor + Duration::from_secs(1),
            CORES,
        );
        assert!(model.threads().rows().is_empty());
    }

    #[test]
    fn unnamed_os_threads_aggregate_into_the_other_group() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        model.ingest_threads(
            vec![
                sample(10, 0, 0, 1),
                sample(99, 0, 0, 2),
                sample(98, 0, 0, 3),
            ],
            vec![info(10, "moiren-professional-master", ThreadClass::Rt)],
            ProbeRead::default(),
            None,
            anchor,
            CORES,
        );
        let (percent, count) = model.threads().other();
        assert_eq!(count, 2, "two unregistered samples");
        assert_eq!(percent, None, "the first sample only primes");
        model.ingest_threads(
            vec![
                sample(10, 1_000_000, 0, 1),
                sample(99, 200_000, 0, 2),
                sample(98, 0, 0, 3),
            ],
            vec![info(10, "moiren-professional-master", ThreadClass::Rt)],
            ProbeRead::default(),
            None,
            anchor + Duration::from_secs(1),
            CORES,
        );
        let (percent, count) = model.threads().other();
        assert_eq!(count, 2);
        // Only one of the two strangers used CPU: 2e5 units over 1 s on two
        // cores is 1%.
        assert!((percent.unwrap() - 1.).abs() < 1e-3, "{percent:?}");
    }

    #[test]
    fn over_budget_events_count_in_a_window_and_reset_clears_them() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![info(10, "moiren-professional-master", ThreadClass::Rt)];
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry.clone(),
            ProbeRead::default(),
            rt(7, 0, 0, 0),
            anchor,
            CORES,
        );
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry.clone(),
            ProbeRead::default(),
            rt(7, 1_000, 3, 0),
            anchor + Duration::from_secs(1),
            CORES,
        );
        assert_eq!(model.threads().rows()[0].over_budget, 3);
        // The window drops entries older than a minute.
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry.clone(),
            ProbeRead::default(),
            rt(7, 2_000, 3, 0),
            anchor + Duration::from_secs(62),
            CORES,
        );
        assert_eq!(model.threads().rows()[0].over_budget, 0, "expired");
        // A new generation rebases: the counter restart does not become a
        // negative delta or a burst.
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry.clone(),
            ProbeRead::default(),
            rt(8, 0, 0, 0),
            anchor + Duration::from_secs(63),
            CORES,
        );
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry.clone(),
            ProbeRead::default(),
            rt(8, 1_000, 1, 0),
            anchor + Duration::from_secs(64),
            CORES,
        );
        assert_eq!(model.threads().rows()[0].over_budget, 1);
        // The panel's reset clears the window; the next pair re-primes.
        model.reset_threads();
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry,
            ProbeRead::default(),
            rt(8, 2_000, 1, 0),
            anchor + Duration::from_secs(65),
            CORES,
        );
        assert_eq!(model.threads().rows()[0].over_budget, 0);
    }

    #[test]
    fn the_asio_note_waits_for_native_callback_activity() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![info(10, "moiren-professional-master", ThreadClass::Rt)];
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry.clone(),
            ProbeRead::default(),
            rt(7, 0, 0, 100),
            anchor,
            CORES,
        );
        assert!(!model.threads().native_callbacks_advanced());
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry.clone(),
            ProbeRead::default(),
            rt(7, 1_000, 0, 148),
            anchor + Duration::from_secs(1),
            CORES,
        );
        assert!(
            model.threads().native_callbacks_advanced(),
            "callbacks advanced: the unnamed-thread CPU carries the note"
        );
        model.ingest_threads(
            vec![sample(10, 0, 0, 1)],
            registry,
            ProbeRead::default(),
            rt(7, 2_000, 0, 148),
            anchor + Duration::from_secs(2),
            CORES,
        );
        assert!(!model.threads().native_callbacks_advanced());
    }

    #[test]
    fn probe_rates_land_on_the_service_and_ui_rows() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![
            info(11, "app-service", ThreadClass::Control),
            info(12, "moiren-ui-main", ThreadClass::Ui),
        ];
        model.ingest_threads(
            Vec::new(),
            registry.clone(),
            ProbeRead {
                service_wakes: 0,
                service_round_us: 400,
                shell_renders: 0,
                graph_frames: 0,
            },
            None,
            anchor,
            CORES,
        );
        model.ingest_threads(
            Vec::new(),
            registry,
            ProbeRead {
                service_wakes: 12,
                service_round_us: 400,
                shell_renders: 60,
                graph_frames: 120,
            },
            None,
            anchor + Duration::from_secs(1),
            CORES,
        );
        let rows = model.threads().rows();
        assert!((rows[0].rate.unwrap() - 12.).abs() < 1e-2, "wake/s");
        assert!(
            rows[0]
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("0.4 ms")),
            "the round duration caption carries the probe: {:?}",
            rows[0].detail
        );
        assert!((rows[1].rate.unwrap() - 60.).abs() < 1e-2, "shell fps");
        assert!(
            rows[1]
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("meter")),
            "the graph meter fps rides as the caption: {:?}",
            rows[1].detail
        );
    }

    #[test]
    fn rows_without_os_accounting_degrade_to_names() {
        // Non-Windows: the sampler returns nothing, forever. Rows keep their
        // names with no CPU share instead of disappearing.
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![info(11, "app-service", ThreadClass::Control)];
        model.ingest_threads(
            Vec::new(),
            registry.clone(),
            ProbeRead::default(),
            None,
            anchor,
            CORES,
        );
        model.ingest_threads(
            Vec::new(),
            registry,
            ProbeRead::default(),
            None,
            anchor + Duration::from_secs(1),
            CORES,
        );
        let threads = model.threads();
        assert_eq!(threads.rows().len(), 1);
        assert_eq!(threads.rows()[0].cpu_percent, None);
        assert!(threads.sampled());
        assert!(!threads.os_data(), "no OS accounting ever arrived");
        // Unborn threads (tid 0) never render.
        let mut model = PerfModel::default();
        model.ingest_threads(
            Vec::new(),
            vec![info(0, "moiren-professional-master", ThreadClass::Rt)],
            ProbeRead::default(),
            None,
            anchor,
            CORES,
        );
        assert!(model.threads().rows().is_empty());
    }

    #[test]
    fn the_master_sparkline_accumulates_only_the_master_row() {
        let anchor = Instant::now();
        let mut model = PerfModel::default();
        let registry = vec![
            info(10, "moiren-professional-master", ThreadClass::Rt),
            info(11, "app-service", ThreadClass::Control),
        ];
        model.ingest_threads(
            vec![sample(10, 0, 0, 1), sample(11, 0, 0, 2)],
            registry.clone(),
            ProbeRead::default(),
            None,
            anchor,
            CORES,
        );
        model.ingest_threads(
            vec![sample(10, 1_000_000, 0, 1), sample(11, 2_000_000, 0, 2)],
            registry,
            ProbeRead::default(),
            None,
            anchor + Duration::from_secs(1),
            CORES,
        );
        let history = model.threads().master_history();
        assert_eq!(history.len(), 1);
        // 1e6 units over 1 s on two cores: 5%, not the service row's 10%.
        assert!((history[0] - 5.).abs() < 1e-3, "{}", history[0]);
    }
}
