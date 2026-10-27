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
    pub native_callbacks: u64,
    pub native_dropped_frames: u64,
    pub native_silent_frames: u64,
    pub driver_latency: Option<NativeLatency>,
}

#[derive(Debug, Clone)]
pub struct ProfessionalObservationSnapshot {
    pub owner_generation: u64,
    pub running: bool,
    pub streams: Vec<ProfessionalStreamObservation>,
}

#[derive(Default)]
pub(super) struct Counters {
    pub dropped: AtomicU64,
    pub silent: AtomicU64,
    native_callbacks: AtomicU64,
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

struct Session {
    generation: u64,
    running: AtomicBool,
    streams: Vec<(ProfessionalStreamObservation, Arc<Counters>)>,
}

impl ProfessionalObservations {
    pub fn snapshot(&self) -> Option<ProfessionalObservationSnapshot> {
        let session = self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
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
                    stream.native_callbacks = counters.native_callbacks.load(Ordering::Relaxed);
                    stream.native_dropped_frames = counters.native_dropped.load(Ordering::Relaxed);
                    stream.native_silent_frames = counters.native_silent.load(Ordering::Relaxed);
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
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(Session {
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
            *state = None;
        }
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
            native_dropped_frames: 0,
            native_silent_frames: 0,
            driver_latency: None,
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
        fresh.publish_native(
            NativeStreamStats {
                callbacks: 7,
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
        assert_eq!(actual.native_callbacks, 7);
        assert_eq!(actual.native_dropped_frames, 11);
        assert_eq!(actual.native_silent_frames, 13);
        assert_eq!(actual.driver_latency.unwrap().output_frames, 64);
        fresh.publish_native(NativeStreamStats::default(), None);
        assert_eq!(
            observations.snapshot().unwrap().streams[0].driver_latency,
            None
        );
    }
}
