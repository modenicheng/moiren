//! The status bar's runtime statistics model: an XRUN event history and the
//! DSP-load readout, both derived on the control side from the backend's
//! authoritative counters. The counters reset only with a session generation
//! and carry no reset path — "reset" here is a control-side baseline move
//! over the same totals, so the service protocol stays untouched and the
//! strip/summary read one consistent model. Everything below is
//! platform-free: the non-Windows build ingests an empty view and the tests
//! run everywhere.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use moiren_core::graph::NodeId;

/// The runtime-relevant slice of one stream's observation, extracted at the
/// snapshot ingest site so this model stays synthetic-sample tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamCounters {
    pub node: NodeId,
    /// Bridge shortfall events (capture overflow, follower silence fill).
    pub xruns: u64,
    /// Native-callback shortfall events (ASIO driver level).
    pub native_xruns: u64,
    /// Engine blocks the master output's pump rendered; zero elsewhere.
    pub engine_blocks: u64,
}

/// The master output's engine render accounting from one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RenderCounters {
    pub blocks: u64,
    pub busy_100ns: u64,
    /// Worst single block of the last completed one-second window.
    pub peak_block_100ns: u64,
}

/// One observation snapshot, reduced to what the runtime statistics consume.
/// The streams are owned because the extractor collects them per snapshot;
/// the ingest runs at the observations' 1 Hz tick, so the cost is nil.
pub(crate) struct ObservationView {
    pub generation: u64,
    pub streams: Vec<StreamCounters>,
    pub render: Option<RenderCounters>,
    /// The engine's block geometry, the denominator of every load figure.
    pub block_frames: u32,
    pub sample_rate: u32,
}

/// The DSP-load readout: how much of the rendered audio time the engine
/// spent inside `Engine::render`, as percentages. `average` covers the
/// interval between the last two observations; `peak` is the worst single
/// block of the last completed one-second window — the number that explains
/// a single-block shortfall.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DspLoad {
    pub average: f32,
    pub peak: f32,
}

/// Bounded history: the strip draws one minute, the summary needs the last
/// event and the clean streaks; 256 shortfalls cover any session that is
/// still diagnosable, and the headline total stays unbounded.
const EVENT_CAP: usize = 256;

/// XRUN events seen since the baseline. `events` carries
/// `(when, how many in that observation interval)` in ingest order.
#[derive(Debug, Default)]
pub(crate) struct XrunStats {
    generation: Option<u64>,
    /// The observations vanished (the session stopped): the next ingest
    /// rebases instead of trusting stale per-stream totals.
    dormant: bool,
    seen: BTreeMap<NodeId, (u64, u64, u64)>,
    events: VecDeque<(Instant, u64)>,
    total: u64,
    /// Master blocks rendered since the baseline: the percentage's
    /// denominator, accumulated by the same saturating deltas.
    blocks: u64,
    last_event: Option<Instant>,
    /// Longest closed clean streak; the live streak is measured against
    /// `mark` at read time.
    longest_clean: Duration,
    mark: Option<Instant>,
    started_at: Option<Instant>,
}

impl XrunStats {
    /// Ingests one observation snapshot. The first observation of a
    /// generation (or after a gap) only primes the baseline — startup
    /// priming must never surface as a burst of events — and every later
    /// observation turns counter deltas into events.
    pub(crate) fn ingest(&mut self, generation: u64, streams: &[StreamCounters], now: Instant) {
        if self.generation != Some(generation) || self.dormant {
            *self = Self::baseline(generation, streams, now);
            return;
        }
        for stream in streams {
            let previous = self.seen.insert(
                stream.node,
                (stream.xruns, stream.native_xruns, stream.engine_blocks),
            );
            let Some((xruns, native_xruns, blocks)) = previous else {
                continue;
            };
            let shortfall = stream
                .xruns
                .saturating_sub(xruns)
                .saturating_add(stream.native_xruns.saturating_sub(native_xruns));
            if shortfall > 0 {
                self.record(shortfall, now);
            }
            self.blocks += stream.engine_blocks.saturating_sub(blocks);
        }
    }

    /// The observations disappeared: counters will be rebased on reappearance.
    pub(crate) fn note_gap(&mut self) {
        if self.generation.is_some() {
            self.dormant = true;
        }
    }

    /// The panel's reset action: forget history and restart the clocks now.
    /// The baseline moves to the live counters at the next observation.
    pub(crate) fn reset(&mut self, now: Instant) {
        let generation = self.generation;
        *self = Self {
            generation,
            mark: Some(now),
            started_at: Some(now),
            ..Self::default()
        };
    }

    fn baseline(generation: u64, streams: &[StreamCounters], now: Instant) -> Self {
        let mut stats = Self {
            generation: Some(generation),
            mark: Some(now),
            started_at: Some(now),
            ..Self::default()
        };
        for stream in streams {
            stats.seen.insert(
                stream.node,
                (stream.xruns, stream.native_xruns, stream.engine_blocks),
            );
        }
        stats
    }

    fn record(&mut self, delta: u64, now: Instant) {
        self.total += delta;
        self.events.push_back((now, delta));
        while self.events.len() > EVENT_CAP {
            self.events.pop_front();
        }
        let clean = self
            .mark
            .map_or(Duration::ZERO, |mark| now.duration_since(mark));
        if clean > self.longest_clean {
            self.longest_clean = clean;
        }
        self.mark = Some(now);
        self.last_event = Some(now);
    }

    /// Shortfall events since the baseline (the bar's headline number).
    pub(crate) fn total(&self) -> u64 {
        self.total
    }

    /// Shortfall events per rendered block, in percent.
    pub(crate) fn percent(&self) -> Option<f32> {
        (self.blocks > 0)
            .then(|| self.total as f64 / self.blocks as f64 * 100.)
            .map(|percent| percent as f32)
    }

    /// Master blocks rendered since the baseline (the panel's block count).
    pub(crate) fn blocks(&self) -> u64 {
        self.blocks
    }

    pub(crate) fn events(&self) -> &VecDeque<(Instant, u64)> {
        &self.events
    }

    pub(crate) fn last_event(&self) -> Option<Instant> {
        self.last_event
    }

    /// The longest stretch without a shortfall, counting the live streak.
    pub(crate) fn longest_clean(&self, now: Instant) -> Duration {
        let live = self
            .mark
            .map_or(Duration::ZERO, |mark| now.duration_since(mark));
        self.longest_clean.max(live)
    }

    /// When this statistics window began (session start or the last reset).
    pub(crate) fn started_at(&self) -> Option<Instant> {
        self.started_at
    }
}

/// Everything the status bar derives from consecutive observation snapshots:
/// the XRUN history and the DSP-load readout.
#[derive(Debug, Default)]
pub(crate) struct RuntimeStats {
    pub(crate) xruns: XrunStats,
    dsp: Option<DspLoad>,
    render_seen: Option<(u64, RenderCounters)>,
}

impl RuntimeStats {
    /// Ingests one observation snapshot. `None` means no observations exist
    /// right now — a stopped session, or a platform without the native
    /// backend — and takes the same gap path: stale readouts go with them
    /// and the next `Some` rebases. Keeping the gap inside the model keeps
    /// every item here compiled on every platform the crate builds on.
    pub(crate) fn ingest(&mut self, view: Option<ObservationView>, now: Instant) {
        let Some(view) = view else {
            self.note_gap();
            return;
        };
        self.xruns.ingest(view.generation, &view.streams, now);
        self.dsp = match (self.render_seen, view.render) {
            (Some((generation, previous)), Some(current)) if generation == view.generation => {
                dsp_load(previous, current, view.block_frames, view.sample_rate).or(self.dsp)
            }
            _ => None,
        };
        self.render_seen = view.render.map(|render| (view.generation, render));
    }

    /// The observations disappeared: everything stale goes with them.
    pub(crate) fn note_gap(&mut self) {
        self.xruns.note_gap();
        self.dsp = None;
        self.render_seen = None;
    }

    pub(crate) fn dsp(&self) -> Option<DspLoad> {
        self.dsp
    }
}

/// Load between two observations of the same session: one block renders
/// `block_frames / sample_rate` seconds of audio, so the average is render
/// time over rendered audio time and the peak compares the worst block with
/// one block's audio time. `None` while no block was rendered in between —
/// a readout without a denominator is a lie — and while zero time was
/// measured over rendered blocks: real rendering takes nonzero time, so a
/// zero delta means the render clock was missing, and a confident 0.0%
/// would be a lie of the same kind.
pub(crate) fn dsp_load(
    previous: RenderCounters,
    current: RenderCounters,
    block_frames: u32,
    sample_rate: u32,
) -> Option<DspLoad> {
    let blocks = current
        .blocks
        .checked_sub(previous.blocks)
        .filter(|blocks| *blocks > 0)?;
    if block_frames == 0 || sample_rate == 0 {
        return None;
    }
    let busy = current.busy_100ns.saturating_sub(previous.busy_100ns);
    if busy == 0 {
        return None;
    }
    let block_100ns = f64::from(block_frames) * 1e7 / f64::from(sample_rate);
    Some(DspLoad {
        average: (busy as f64 / (blocks as f64 * block_100ns) * 100.) as f32,
        peak: (current.peak_block_100ns as f64 / block_100ns * 100.) as f32,
    })
}

#[cfg(test)]
mod tests {
    use moiren_core::graph::{LogicalGraph, NodeKind};

    use super::*;

    /// Real node identities: `NodeId` has no public constructor, so the
    /// synthetic streams bind to nodes of a throwaway graph.
    fn nodes() -> Vec<NodeId> {
        let mut graph = LogicalGraph::new();
        (0..3)
            .map(|_| graph.create_node(NodeKind::Sink, 2).unwrap())
            .collect()
    }

    fn stream(node: NodeId, xruns: u64, native_xruns: u64) -> StreamCounters {
        StreamCounters {
            node,
            xruns,
            native_xruns,
            engine_blocks: 0,
        }
    }

    #[test]
    fn the_first_observation_primes_the_baseline_without_events() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = XrunStats::default();
        stats.ingest(7, &[stream(nodes[0], 4, 9)], at(0));
        assert_eq!(stats.total(), 0, "startup priming is the baseline");
        assert!(stats.events().is_empty());
        assert_eq!(stats.blocks(), 0);
        assert_eq!(stats.longest_clean(at(5)), Duration::from_secs(5));
    }

    #[test]
    fn counter_deltas_become_events_and_feed_the_summary() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = XrunStats::default();
        stats.ingest(7, &[stream(nodes[0], 4, 9)], at(0));
        // A stream seen for the first time only primes its baseline; the
        // known stream's delta turns into events.
        stats.ingest(
            7,
            &[
                StreamCounters {
                    node: nodes[0],
                    xruns: 6,
                    native_xruns: 9,
                    engine_blocks: 0,
                },
                StreamCounters {
                    node: nodes[1],
                    xruns: 0,
                    native_xruns: 2,
                    engine_blocks: 0,
                },
            ],
            at(10),
        );
        assert_eq!(stats.total(), 2, "one bridge shortfall, two events' worth");
        assert_eq!(stats.events().len(), 1);
        let (when, count) = stats.events()[0];
        assert_eq!(count, 2);
        assert_eq!(stats.last_event(), Some(when));
        // The streak from the baseline to the event is the longest so far.
        assert_eq!(stats.longest_clean(at(11)), Duration::from_secs(10));
        // The previously unseen stream counts from its own baseline onward.
        stats.ingest(
            7,
            &[StreamCounters {
                node: nodes[1],
                xruns: 0,
                native_xruns: 3,
                engine_blocks: 0,
            }],
            at(15),
        );
        assert_eq!(stats.total(), 3);
    }

    #[test]
    fn a_reset_counter_pair_never_becomes_a_negative_event() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = XrunStats::default();
        stats.ingest(7, &[stream(nodes[0], 4, 9)], at(0));
        // A fresh backend reports zeros for a node it had counted before;
        // saturating deltas swallow the drop instead of inventing events.
        stats.ingest(7, &[stream(nodes[0], 0, 0)], at(5));
        assert_eq!(stats.total(), 0);
        assert!(stats.events().is_empty());
    }

    #[test]
    fn a_new_generation_clears_history_and_rebases() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = XrunStats::default();
        stats.ingest(7, &[stream(nodes[0], 4, 9)], at(0));
        stats.ingest(7, &[stream(nodes[0], 9, 9)], at(5));
        stats.ingest(8, &[stream(nodes[0], 2, 0)], at(20));
        assert_eq!(stats.total(), 0, "the old session's events do not carry");
        assert!(stats.events().is_empty());
        stats.ingest(8, &[stream(nodes[0], 5, 0)], at(25));
        assert_eq!(stats.total(), 3, "deltas measure inside one generation");
    }

    #[test]
    fn observations_returning_after_a_gap_rebase_without_a_burst() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = XrunStats::default();
        stats.ingest(7, &[stream(nodes[0], 4, 9)], at(0));
        stats.ingest(7, &[stream(nodes[0], 5, 9)], at(5));
        stats.note_gap();
        stats.ingest(7, &[stream(nodes[0], 0, 0)], at(30));
        assert_eq!(stats.total(), 0, "rebased on reappearance");
        stats.ingest(7, &[stream(nodes[0], 2, 0)], at(35));
        assert_eq!(stats.total(), 2);
    }

    #[test]
    fn reset_forgets_history_and_rebases_on_the_live_counters() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = XrunStats::default();
        stats.ingest(7, &[stream(nodes[0], 4, 9)], at(0));
        stats.ingest(7, &[stream(nodes[0], 9, 12)], at(5));
        assert_eq!(stats.total(), 8);
        stats.reset(at(6));
        assert_eq!(stats.total(), 0);
        assert_eq!(stats.started_at(), Some(at(6)), "the streak clock restarts");
        // The baseline moves to the live totals: only new shortfalls count.
        stats.ingest(7, &[stream(nodes[0], 9, 12)], at(7));
        assert_eq!(stats.total(), 0);
        stats.ingest(7, &[stream(nodes[0], 10, 12)], at(8));
        assert_eq!(stats.total(), 1);
    }

    #[test]
    fn the_event_cap_drops_the_oldest_entries() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = XrunStats::default();
        stats.ingest(7, &[stream(nodes[0], 0, 0)], at(0));
        for second in 1..=(EVENT_CAP as u64 + 10) {
            stats.ingest(
                7,
                &[StreamCounters {
                    node: nodes[0],
                    xruns: second,
                    native_xruns: 0,
                    engine_blocks: 0,
                }],
                at(second),
            );
        }
        assert_eq!(stats.events().len(), EVENT_CAP);
        assert_eq!(
            stats.total(),
            EVENT_CAP as u64 + 10,
            "the total is unbounded"
        );
        assert_eq!(stats.events().front().unwrap().0, at(11), "oldest dropped");
    }

    #[test]
    fn rendered_blocks_accumulate_from_the_baseline_into_the_percentage() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let master = |xruns: u64, blocks: u64| StreamCounters {
            node: nodes[0],
            xruns,
            native_xruns: 0,
            engine_blocks: blocks,
        };
        let mut stats = XrunStats::default();
        stats.ingest(7, &[master(1, 1_000)], at(0));
        stats.ingest(7, &[master(3, 11_000)], at(1));
        assert_eq!(
            stats.blocks(),
            10_000,
            "only blocks since the baseline count"
        );
        assert_eq!(stats.percent(), Some(0.02), "2 events over 10k blocks");
        stats.reset(at(2));
        stats.ingest(7, &[master(3, 20_000)], at(3));
        assert_eq!(
            stats.blocks(),
            0,
            "the first observation after a reset only rebases"
        );
        stats.ingest(7, &[master(4, 29_000)], at(4));
        assert_eq!(stats.blocks(), 9_000, "the baseline moved with the reset");
    }

    #[test]
    fn dsp_load_compares_render_time_with_rendered_audio_time() {
        // 48 kHz, 256-frame blocks: one block is 5333.33.. 100 ns units.
        let previous = RenderCounters {
            blocks: 1_000,
            busy_100ns: 10_000,
            peak_block_100ns: 20,
        };
        // 100 blocks in one second, 5.33 s of audio, 26666 busy units.
        let current = RenderCounters {
            blocks: 1_100,
            busy_100ns: 36_666,
            peak_block_100ns: 100,
        };
        let load = dsp_load(previous, current, 256, 48_000).unwrap();
        assert!(
            (load.average - 0.5).abs() < 0.05,
            "about half a percent busy: {}",
            load.average
        );
        assert!(
            (load.peak - 0.1875).abs() < 0.01,
            "the worst block took 0.1875% of its own audio time: {}",
            load.peak
        );
    }

    #[test]
    fn dsp_load_is_none_without_rendered_blocks_or_geometry() {
        let counters = RenderCounters {
            blocks: 5,
            busy_100ns: 1,
            peak_block_100ns: 1,
        };
        assert_eq!(
            dsp_load(counters, counters, 256, 48_000),
            None,
            "no new blocks"
        );
        assert_eq!(
            dsp_load(
                counters,
                RenderCounters {
                    blocks: 6,
                    ..counters
                },
                0,
                48_000
            ),
            None,
            "no block geometry"
        );
        assert_eq!(
            dsp_load(
                counters,
                RenderCounters {
                    blocks: 6,
                    ..counters
                },
                256,
                0
            ),
            None,
            "no sample rate"
        );
        // Blocks rendered with zero measured time mean the render clock was
        // missing (real rendering takes nonzero time): no readout, not 0%.
        assert_eq!(
            dsp_load(
                counters,
                RenderCounters {
                    blocks: 6,
                    busy_100ns: 0,
                    peak_block_100ns: 0,
                },
                256,
                48_000
            ),
            None,
            "zero busy over rendered blocks is unmeasured, not idle"
        );
    }

    #[test]
    fn runtime_stats_ingest_wires_xruns_and_dsp_together() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let master = |xruns: u64, blocks: u64| StreamCounters {
            node: nodes[0],
            xruns,
            native_xruns: 0,
            engine_blocks: blocks,
        };
        let render = |blocks: u64, busy: u64| RenderCounters {
            blocks,
            busy_100ns: busy,
            peak_block_100ns: 0,
        };
        let mut stats = RuntimeStats::default();
        stats.ingest(
            Some(ObservationView {
                generation: 7,
                streams: vec![master(0, 1_000)],
                render: Some(render(1_000, 10_000)),
                block_frames: 256,
                sample_rate: 48_000,
            }),
            at(0),
        );
        assert_eq!(stats.dsp(), None, "one observation is not a rate");
        stats.ingest(
            Some(ObservationView {
                generation: 7,
                streams: vec![master(1, 11_000)],
                render: Some(render(11_000, 20_000)),
                block_frames: 256,
                sample_rate: 48_000,
            }),
            at(1),
        );
        assert_eq!(stats.xruns.total(), 1);
        assert!(stats.dsp().is_some());
        // The observations vanish: the DSP readout must not survive the gap,
        // while the count stays on the bar until the next session's first
        // observation rebases it.
        stats.note_gap();
        assert_eq!(stats.dsp(), None);
        assert_eq!(stats.xruns.total(), 1, "the count survives the gap");
        stats.ingest(
            Some(ObservationView {
                generation: 7,
                streams: vec![master(0, 11_000)],
                render: Some(render(11_000, 20_000)),
                block_frames: 256,
                sample_rate: 48_000,
            }),
            at(2),
        );
        assert_eq!(
            stats.xruns.total(),
            0,
            "the reappearance rebased the history"
        );
        stats.ingest(
            Some(ObservationView {
                generation: 7,
                streams: vec![master(2, 21_000)],
                render: Some(render(21_000, 30_000)),
                block_frames: 256,
                sample_rate: 48_000,
            }),
            at(3),
        );
        assert_eq!(stats.xruns.total(), 2);
        assert!(
            stats.dsp().is_some(),
            "the load readout resumed with the stream"
        );
    }

    #[test]
    fn a_none_view_takes_the_gap_path_without_touching_history() {
        let nodes = nodes();
        let anchor = Instant::now();
        let at = |seconds: u64| anchor + Duration::from_secs(seconds);
        let mut stats = RuntimeStats::default();
        stats.ingest(
            Some(ObservationView {
                generation: 7,
                streams: vec![StreamCounters {
                    node: nodes[0],
                    xruns: 0,
                    native_xruns: 0,
                    engine_blocks: 1_000,
                }],
                render: Some(RenderCounters {
                    blocks: 1_000,
                    busy_100ns: 10_000,
                    peak_block_100ns: 0,
                }),
                block_frames: 256,
                sample_rate: 48_000,
            }),
            at(0),
        );
        stats.ingest(None, at(5));
        assert_eq!(stats.xruns.total(), 0, "nothing existed to count");
        assert_eq!(
            stats.dsp(),
            None,
            "the stale load readout went with the gap"
        );
        // The reappearance itself only rebases — see the next ingest.
        stats.ingest(
            Some(ObservationView {
                generation: 7,
                streams: vec![StreamCounters {
                    node: nodes[0],
                    xruns: 2,
                    native_xruns: 0,
                    engine_blocks: 5_000,
                }],
                render: None,
                block_frames: 256,
                sample_rate: 48_000,
            }),
            at(6),
        );
        assert_eq!(stats.xruns.total(), 0, "the reappearance only rebases");
        // The reappearance itself only rebases — the documented
        // reappear-after-gap semantics — and deltas count from there.
        stats.ingest(
            Some(ObservationView {
                generation: 7,
                streams: vec![StreamCounters {
                    node: nodes[0],
                    xruns: 3,
                    native_xruns: 0,
                    engine_blocks: 6_000,
                }],
                render: None,
                block_frames: 256,
                sample_rate: 48_000,
            }),
            at(7),
        );
        assert_eq!(
            stats.xruns.total(),
            1,
            "deltas count again after the rebase"
        );
        assert_eq!(stats.dsp(), None, "no master render, no load readout");
    }
}
