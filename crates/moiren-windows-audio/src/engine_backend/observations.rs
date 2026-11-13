//! Control snapshots clone prepared metadata; streaming only updates atomics.
use moiren_core::graph::NodeId;
use moiren_engine::node::{DeviceOptions, VirtualEndpointId};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(any(windows, test))]
use crate::native::NativeStreamStats;
use crate::native::{NativeLatency, NativeStreamInfo};

/// The device a live stream runs on, recorded when the stream is
/// registered. This is control-side metadata (it never enters the realtime
/// path) and carries the **service-side identity** — the string the graph
/// bound, which is the device catalog row id — not the canonical id a
/// native open resolves to (an ASIO `Group.id` is the upper-case bare
/// CLSID while the catalog row says `asio:{CLSID}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEndpoint {
    /// MMDevice endpoint id or the `asio:{CLSID}` driver id, exactly as the
    /// graph's binding names it. A `FollowDefault` selector records the
    /// resolved id instead, which is also a catalog row id.
    Physical(String),
    /// Process loopback target identity.
    Application { pid: u32, creation_time_100ns: u64 },
    /// A virtual playback/microphone boundary.
    Virtual(VirtualEndpointId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfessionalStreamObservation {
    pub node: NodeId,
    pub endpoint: StreamEndpoint,
    pub input: bool,
    pub master: bool,
    pub requested: DeviceOptions,
    pub actual: NativeStreamInfo,
    pub dropped_frames: u64,
    pub silent_frames: u64,
    /// Bridge shortfall events (capture ring overflow, follower silence
    /// fill), one per starved pump cycle; the frame fields above carry the
    /// frames lost.
    pub xruns: u64,
    pub native_callbacks: u64,
    pub native_dropped_frames: u64,
    pub native_silent_frames: u64,
    /// Native-callback shortfall events (ASIO driver level), one per
    /// starved callback per direction — a full-duplex callback starving on
    /// both ends counts twice.
    pub native_xruns: u64,
    /// Engine render accounting, advanced by the master output's pump only:
    /// blocks rendered, total time spent inside `Engine::render`, and the
    /// worst single block of the last completed one-second window, in 100 ns
    /// ticks. The DSP-load readout divides the deltas by rendered audio time.
    pub engine_blocks: u64,
    pub engine_busy_100ns: u64,
    pub engine_peak_block_100ns: u64,
    pub driver_latency: Option<NativeLatency>,
    /// The endpoint's device is missing: the stream is a silent null stream
    /// binding, not a native worker, and returns automatically once the
    /// device comes back.
    pub absent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfessionalObservationSnapshot {
    pub owner_generation: u64,
    pub running: bool,
    pub streams: Vec<ProfessionalStreamObservation>,
}

/// Live progress of one preparation's native opens (issue #105): how many
/// device groups settle out of how many, and one endpoint still opening.
/// Published between handshakes on the control side and cleared when the
/// preparation finishes, so the UI can show "Starting (2/3) · <device>"
/// instead of an unexplained Starting phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfessionalPrepareProgress {
    pub owner_generation: u64,
    pub groups_total: usize,
    pub groups_opened: usize,
    /// Service-side endpoint id of one group still opening, when any remain.
    pub endpoint: Option<String>,
}

#[derive(Default)]
pub(crate) struct Counters {
    pub dropped: AtomicU64,
    pub silent: AtomicU64,
    pub xruns: AtomicU64,
    /// Master pump only: engine blocks rendered, cumulative render time, and
    /// the worst single block of the last completed one-second window. The
    /// audio thread owns the windowing, so readers only load.
    pub blocks: AtomicU64,
    pub render_ticks: AtomicU64,
    pub render_peak_ticks: AtomicU64,
    native_callbacks: AtomicU64,
    native_dropped: AtomicU64,
    native_silent: AtomicU64,
    native_xruns: AtomicU64,
    latency_sequence: AtomicU64,
    latency: AtomicU64,
    latency_valid: AtomicBool,
}

impl Counters {
    #[cfg(any(windows, test))]
    pub fn publish_native(&self, stats: NativeStreamStats, latency: Option<NativeLatency>) {
        self.native_callbacks
            .store(stats.callbacks, Ordering::Relaxed);
        self.native_dropped
            .store(stats.capture_dropped_frames, Ordering::Relaxed);
        self.native_silent
            .store(stats.render_silent_frames, Ordering::Relaxed);
        self.native_xruns.store(stats.xruns, Ordering::Relaxed);
        // One native owner writes; readers use a bounded seqlock snapshot.
        self.latency_sequence.fetch_add(1, Ordering::SeqCst);
        if let Some(latency) = latency {
            self.latency.store(
                (u64::from(latency.input_frames) << 32) | u64::from(latency.output_frames),
                Ordering::SeqCst,
            );
        }
        self.latency_valid
            .store(latency.is_some(), Ordering::SeqCst);
        self.latency_sequence.fetch_add(1, Ordering::SeqCst);
    }

    fn driver_latency(&self) -> Option<NativeLatency> {
        let before = self.latency_sequence.load(Ordering::SeqCst);
        let valid = self.latency_valid.load(Ordering::SeqCst);
        let latency = self.latency.load(Ordering::SeqCst);
        (valid
            && before.is_multiple_of(2)
            && before == self.latency_sequence.load(Ordering::SeqCst))
        .then_some(NativeLatency {
            input_frames: (latency >> 32) as u32,
            output_frames: latency as u32,
        })
    }
}

#[derive(Clone, Default)]
pub struct ProfessionalObservations {
    session: Arc<Mutex<Option<Arc<Session>>>>,
    progress: Arc<Mutex<Option<ProfessionalPrepareProgress>>>,
}

struct Session {
    generation: u64,
    running: AtomicBool,
    streams: Vec<(ProfessionalStreamObservation, Arc<Counters>)>,
}

impl ProfessionalObservations {
    pub fn snapshot(&self) -> Option<ProfessionalObservationSnapshot> {
        let session = self
            .session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()?;
        Some(ProfessionalObservationSnapshot {
            owner_generation: session.generation,
            running: session.running.load(Ordering::Acquire),
            streams: session
                .streams
                .iter()
                .map(|(stream, counters)| {
                    let mut stream = stream.clone();
                    stream.dropped_frames = counters.dropped.load(Ordering::Relaxed);
                    stream.silent_frames = counters.silent.load(Ordering::Relaxed);
                    stream.xruns = counters.xruns.load(Ordering::Relaxed);
                    stream.native_callbacks = counters.native_callbacks.load(Ordering::Relaxed);
                    stream.native_dropped_frames = counters.native_dropped.load(Ordering::Relaxed);
                    stream.native_silent_frames = counters.native_silent.load(Ordering::Relaxed);
                    stream.native_xruns = counters.native_xruns.load(Ordering::Relaxed);
                    stream.engine_blocks = counters.blocks.load(Ordering::Relaxed);
                    stream.engine_busy_100ns = counters.render_ticks.load(Ordering::Relaxed);
                    stream.engine_peak_block_100ns =
                        counters.render_peak_ticks.load(Ordering::Relaxed);
                    stream.driver_latency = counters.driver_latency();
                    stream
                })
                .collect(),
        })
    }

    #[cfg(any(windows, test))]
    pub(super) fn publish(
        &self,
        generation: u64,
        streams: Vec<(ProfessionalStreamObservation, Arc<Counters>)>,
    ) {
        *self.session.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(Session {
            generation,
            running: AtomicBool::new(false),
            streams,
        }));
    }

    /// Live prepare progress, generation-fenced like the stream session. The
    /// preparation thread is the only writer; the app service polls it.
    #[cfg(any(windows, test))]
    pub(super) fn publish_progress(
        &self,
        owner_generation: u64,
        groups_total: usize,
        groups_opened: usize,
        endpoint: Option<String>,
    ) {
        *self.progress.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(ProfessionalPrepareProgress {
                owner_generation,
                groups_total,
                groups_opened,
                endpoint,
            });
    }

    #[cfg(any(windows, test))]
    pub(super) fn clear_progress(&self, owner_generation: u64) {
        let mut progress = self.progress.lock().unwrap_or_else(|p| p.into_inner());
        if progress
            .as_ref()
            .is_some_and(|p| p.owner_generation == owner_generation)
        {
            *progress = None;
        }
    }

    pub fn snapshot_progress(&self) -> Option<ProfessionalPrepareProgress> {
        self.progress
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    #[cfg(any(windows, test))]
    pub(super) fn running(&self, generation: u64) {
        if let Some(session) = self
            .session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|s| s.generation == generation)
        {
            session.running.store(true, Ordering::Release);
        }
    }

    #[cfg(any(windows, test))]
    pub(super) fn clear(&self, generation: u64) {
        let mut state = self.session.lock().unwrap_or_else(|p| p.into_inner());
        if state.as_ref().is_some_and(|s| s.generation == generation) {
            *state = None;
        }
        self.clear_progress(generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_owner_cannot_publish_running_or_clear_new_generation() {
        let observations = ProfessionalObservations::default();
        observations.publish(7, Vec::new());
        observations.running(7);
        assert!(observations.snapshot().unwrap().running);
        observations.publish(8, Vec::new());
        observations.running(7);
        observations.clear(7);
        assert_eq!(observations.snapshot().unwrap().owner_generation, 8);
        assert!(!observations.snapshot().unwrap().running);
        observations.clear(8);
        assert!(observations.snapshot().is_none());
    }

    #[test]
    fn prepare_progress_is_generation_fenced_and_cleared_with_the_session() {
        let observations = ProfessionalObservations::default();
        assert!(observations.snapshot_progress().is_none());
        observations.publish_progress(3, 4, 1, Some("{opening}".into()));
        assert_eq!(
            observations.snapshot_progress().unwrap(),
            ProfessionalPrepareProgress {
                owner_generation: 3,
                groups_total: 4,
                groups_opened: 1,
                endpoint: Some("{opening}".into()),
            }
        );
        // A newer generation's clear cannot retire the live record.
        observations.clear_progress(2);
        assert!(observations.snapshot_progress().is_some());
        observations.clear_progress(3);
        assert!(observations.snapshot_progress().is_none());
        // Session teardown clears the progress of its own generation too.
        observations.publish_progress(5, 1, 0, None);
        observations.clear(5);
        assert!(observations.snapshot_progress().is_none());
    }

    #[test]
    fn native_diagnostics_are_separate_from_bridge_and_generation_fenced() {
        let mut graph = moiren_core::graph::LogicalGraph::new();
        let node = graph
            .create_node(moiren_core::graph::NodeKind::Source, 2)
            .unwrap();
        let observations = ProfessionalObservations::default();
        let old = Arc::new(Counters::default());
        let fresh = Arc::new(Counters::default());
        let stream = ProfessionalStreamObservation {
            node,
            endpoint: super::StreamEndpoint::Physical("test-endpoint".into()),
            input: true,
            master: false,
            requested: DeviceOptions::default(),
            actual: NativeStreamInfo {
                sample_rate: 48000,
                format: moiren_engine::audio_config::SampleFormat::Float32,
                buffer_frames: 256,
                channels: 2,
            },
            dropped_frames: 0,
            silent_frames: 0,
            xruns: 0,
            native_callbacks: 0,
            native_dropped_frames: 0,
            native_silent_frames: 0,
            native_xruns: 0,
            engine_blocks: 0,
            engine_busy_100ns: 0,
            engine_peak_block_100ns: 0,
            driver_latency: None,
            absent: false,
        };
        observations.publish(1, vec![(stream.clone(), old.clone())]);
        observations.publish(2, vec![(stream, fresh.clone())]);
        old.publish_native(
            NativeStreamStats {
                callbacks: 999,
                ..Default::default()
            },
            None,
        );
        fresh.dropped.store(3, Ordering::Relaxed);
        fresh.xruns.store(2, Ordering::Relaxed);
        fresh.blocks.store(41_000, Ordering::Relaxed);
        fresh.render_ticks.store(9_000, Ordering::Relaxed);
        fresh.render_peak_ticks.store(150, Ordering::Relaxed);
        fresh.publish_native(
            NativeStreamStats {
                callbacks: 7,
                capture_dropped_frames: 11,
                render_silent_frames: 13,
                xruns: 5,
                ..Default::default()
            },
            Some(NativeLatency {
                input_frames: 32,
                output_frames: 64,
            }),
        );
        let snapshot = observations.snapshot().unwrap();
        let actual = &snapshot.streams[0];
        assert_eq!(actual.dropped_frames, 3);
        assert_eq!(actual.silent_frames, 0);
        assert_eq!(actual.xruns, 2);
        assert_eq!(actual.native_callbacks, 7);
        assert_eq!(actual.native_dropped_frames, 11);
        assert_eq!(actual.native_silent_frames, 13);
        assert_eq!(actual.native_xruns, 5);
        assert_eq!(actual.engine_blocks, 41_000);
        assert_eq!(actual.engine_busy_100ns, 9_000);
        assert_eq!(actual.engine_peak_block_100ns, 150);
        assert_eq!(actual.driver_latency.unwrap().output_frames, 64);
        fresh.publish_native(NativeStreamStats::default(), None);
        assert_eq!(
            observations.snapshot().unwrap().streams[0].driver_latency,
            None
        );
    }
}
