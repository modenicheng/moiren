//! Control snapshots clone prepared metadata; streaming only updates atomics.
use moiren_core::graph::NodeId;
use moiren_engine::node::DeviceOptions;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(any(windows, test))]
use crate::native::NativeStreamStats;
use crate::native::{NativeLatency, NativeStreamInfo};

#[derive(Debug, Clone)]
pub struct ProfessionalStreamObservation {
    pub node: NodeId,
    pub input: bool,
    pub master: bool,
    pub requested: DeviceOptions,
    pub actual: NativeStreamInfo,
    pub dropped_frames: u64,
    pub silent_frames: u64,
    /// Native callback count where supported; zero does not imply no progress.
    pub native_callbacks: u64,
    /// Actual signaled native waits, never synthesized into callbacks.
    pub native_wait_wakes: u64,
    /// Frames successfully written to the native stream, not graph renders.
    pub native_written_frames: u64,
    /// Actual native wait timeouts, independent of callback and frame counts.
    pub native_wait_timeouts: u64,
    pub native_dropped_frames: u64,
    pub native_silent_frames: u64,
    pub driver_latency: Option<NativeLatency>,
}

#[derive(Debug, Clone)]
pub struct ProfessionalObservationSnapshot {
    pub owner_generation: u64,
    /// Publication state only. False does not prove workers joined or cleanup
    /// succeeded; callers must establish teardown completion separately.
    pub running: bool,
    pub streams: Vec<ProfessionalStreamObservation>,
}

#[derive(Default)]
pub(crate) struct Counters {
    /// Single master writer publishes actual complete Engine consumption
    /// per native wake. Control retains the observer after worker retirement.
    pub consumer_events: std::sync::OnceLock<crate::latency::events::EventHistoryObserver>,
    pub dropped: AtomicU64,
    pub silent: AtomicU64,
    native_callbacks: AtomicU64,
    native_wait_wakes: AtomicU64,
    native_written_frames: AtomicU64,
    native_wait_timeouts: AtomicU64,
    native_dropped: AtomicU64,
    native_silent: AtomicU64,
    latency_sequence: AtomicU64,
    latency: AtomicU64,
    latency_valid: AtomicBool,
}

impl Counters {
    #[cfg(any(windows, test))]
    pub fn publish_native(&self, stats: NativeStreamStats, latency: Option<NativeLatency>) {
        self.native_callbacks
            .store(stats.callbacks, Ordering::Relaxed);
        self.native_wait_wakes
            .store(stats.wait_wakes, Ordering::Relaxed);
        self.native_written_frames
            .store(stats.written_frames, Ordering::Relaxed);
        self.native_wait_timeouts
            .store(stats.wait_timeouts, Ordering::Relaxed);
        self.native_dropped
            .store(stats.capture_dropped_frames, Ordering::Relaxed);
        self.native_silent
            .store(stats.render_silent_frames, Ordering::Relaxed);
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
pub struct ProfessionalObservations(Arc<Mutex<Option<Arc<Session>>>>);

/// Control-side read access to exactly one published owner generation.
/// Does not retain audio resources or republish a cleared session. Counters
/// remain live; read final values only after independently completed teardown.
#[derive(Clone)]
pub struct ProfessionalObservationReadHandle(Arc<Session>);

struct Session {
    generation: u64,
    running: AtomicBool,
    streams: Vec<(ProfessionalStreamObservation, Arc<Counters>)>,
}

impl ProfessionalObservationReadHandle {
    pub fn snapshot(&self) -> ProfessionalObservationSnapshot {
        let session = &self.0;
        ProfessionalObservationSnapshot {
            owner_generation: session.generation,
            running: session.running.load(Ordering::Acquire),
            streams: session
                .streams
                .iter()
                .map(|(stream, counters)| {
                    let mut stream = stream.clone();
                    stream.dropped_frames = counters.dropped.load(Ordering::Relaxed);
                    stream.silent_frames = counters.silent.load(Ordering::Relaxed);
                    stream.native_callbacks = counters.native_callbacks.load(Ordering::Relaxed);
                    stream.native_wait_wakes = counters.native_wait_wakes.load(Ordering::Relaxed);
                    stream.native_written_frames =
                        counters.native_written_frames.load(Ordering::Relaxed);
                    stream.native_wait_timeouts =
                        counters.native_wait_timeouts.load(Ordering::Relaxed);
                    stream.native_dropped_frames = counters.native_dropped.load(Ordering::Relaxed);
                    stream.native_silent_frames = counters.native_silent.load(Ordering::Relaxed);
                    stream.driver_latency = counters.driver_latency();
                    stream
                })
                .collect(),
        }
    }
}

impl ProfessionalObservations {
    /// Retain the current generation's metadata and shared counters off RT.
    /// Clearing or replacing the live session cannot redirect this handle.
    pub fn retain_session(&self) -> Option<ProfessionalObservationReadHandle> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|session| ProfessionalObservationReadHandle(session.clone()))
    }

    pub fn snapshot(&self) -> Option<ProfessionalObservationSnapshot> {
        self.retain_session().map(|handle| handle.snapshot())
    }

    #[cfg(any(windows, test))]
    pub(super) fn publish(
        &self,
        generation: u64,
        streams: Vec<(ProfessionalStreamObservation, Arc<Counters>)>,
    ) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(previous) = state.as_ref() {
            previous.running.store(false, Ordering::Release);
        }
        *state = Some(Arc::new(Session {
            generation,
            running: AtomicBool::new(false),
            streams,
        }));
    }

    #[cfg(any(windows, test))]
    pub(super) fn running(&self, generation: u64) {
        if let Some(session) = self
            .0
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
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.as_ref().is_some_and(|s| s.generation == generation) {
            // Cancellation also clears before joins. Retained counters still
            // receive final owner writes; false is not a completion witness.
            if let Some(session) = state.as_ref() {
                session.running.store(false, Ordering::Release);
            }
            *state = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_session_survives_clear_without_resurrection_or_replacement_mixing() {
        let observations = ProfessionalObservations::default();
        assert!(observations.retain_session().is_none());
        observations.publish(1, Vec::new());
        observations.running(1);
        let old = observations.retain_session().unwrap();
        assert!(old.snapshot().running);
        observations.publish(2, Vec::new());
        let current = observations.retain_session().unwrap();
        observations.running(2);
        observations.clear(1);
        observations.running(1);
        assert_eq!(old.snapshot().owner_generation, 1);
        assert!(!old.snapshot().running);
        assert_eq!(current.snapshot().owner_generation, 2);
        assert!(current.snapshot().running);
        observations.clear(2);
        observations.running(2);
        assert!(observations.snapshot().is_none());
        assert!(observations.retain_session().is_none());
        assert!(!current.snapshot().running);
        let retained = current.clone();
        drop(observations);
        drop(current);
        assert_eq!(retained.snapshot().owner_generation, 2);
        assert!(!retained.snapshot().running);
    }

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
            native_callbacks: 0,
            native_wait_wakes: 0,
            native_written_frames: 0,
            native_wait_timeouts: 0,
            native_dropped_frames: 0,
            native_silent_frames: 0,
            driver_latency: None,
        };
        observations.publish(1, vec![(stream.clone(), old.clone())]);
        let old_read = observations.retain_session().unwrap();
        observations.publish(2, vec![(stream, fresh.clone())]);
        let retained = observations.retain_session().unwrap();
        old.publish_native(
            NativeStreamStats {
                callbacks: 999,
                wait_wakes: 997,
                written_frames: 99_999,
                wait_timeouts: 991,
                ..Default::default()
            },
            None,
        );
        fresh.dropped.store(3, Ordering::Relaxed);
        fresh.publish_native(
            NativeStreamStats {
                callbacks: 0,
                wait_wakes: 5,
                written_frames: 480,
                wait_timeouts: 2,
                capture_dropped_frames: 11,
                render_silent_frames: 13,
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
        assert_eq!(actual.native_callbacks, 0);
        assert_eq!(actual.native_wait_wakes, 5);
        assert_eq!(actual.native_written_frames, 480);
        assert_eq!(actual.native_wait_timeouts, 2);
        assert_eq!(actual.native_dropped_frames, 11);
        assert_eq!(actual.native_silent_frames, 13);
        assert_eq!(actual.driver_latency.unwrap().output_frames, 64);
        assert_eq!(old_read.snapshot().streams[0].native_callbacks, 999);
        let old_stream = &old_read.snapshot().streams[0];
        assert_eq!(old_stream.native_wait_wakes, 997);
        assert_eq!(old_stream.native_written_frames, 99_999);
        assert_eq!(old_stream.native_wait_timeouts, 991);
        observations.running(2);
        observations.clear(2);
        assert!(observations.snapshot().is_none());
        // Cancellation may clear before joins; the handle must still observe
        // the final owner writes rather than freezing the pre-stop counters.
        fresh.silent.store(17, Ordering::Relaxed);
        // WASAPI reports wait/write progress without ASIO-style callbacks.
        // Preserve those independent measurements even after live clear.
        fresh.publish_native(
            NativeStreamStats {
                callbacks: 0,
                wait_wakes: 19,
                written_frames: 960,
                wait_timeouts: 3,
                capture_dropped_frames: 23,
                render_silent_frames: 29,
                ..Default::default()
            },
            Some(NativeLatency {
                input_frames: 31,
                output_frames: 37,
            }),
        );
        let final_snapshot = retained.snapshot();
        assert_eq!(final_snapshot.owner_generation, 2);
        assert!(!final_snapshot.running);
        let final_stream = &final_snapshot.streams[0];
        assert_eq!(final_stream.node, node);
        assert_eq!(final_stream.actual.sample_rate, 48_000);
        assert_eq!(final_stream.actual.buffer_frames, 256);
        assert_eq!(final_stream.dropped_frames, 3);
        assert_eq!(final_stream.silent_frames, 17);
        assert_eq!(final_stream.native_callbacks, 0);
        assert_eq!(final_stream.native_wait_wakes, 19);
        assert_eq!(final_stream.native_written_frames, 960);
        assert_eq!(final_stream.native_wait_timeouts, 3);
        assert_eq!(final_stream.native_dropped_frames, 23);
        assert_eq!(final_stream.native_silent_frames, 29);
        assert_eq!(final_stream.driver_latency.unwrap().output_frames, 37);
        fresh.publish_native(NativeStreamStats::default(), None);
        let cleared = retained.snapshot();
        assert_eq!(cleared.streams[0].driver_latency, None);
        assert_eq!(cleared.streams[0].native_callbacks, 0);
        assert_eq!(cleared.streams[0].native_wait_wakes, 0);
        assert_eq!(cleared.streams[0].native_written_frames, 0);
        assert_eq!(cleared.streams[0].native_wait_timeouts, 0);
    }
}
