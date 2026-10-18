//! Control snapshots clone prepared metadata; streaming only updates atomics.
use moiren_core::graph::NodeId;
use moiren_engine::node::DeviceOptions;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use super::native::NativeStreamInfo;

#[derive(Debug, Clone)]
pub struct ProfessionalStreamObservation {
    pub node: NodeId,
    pub input: bool,
    pub master: bool,
    pub requested: DeviceOptions,
    pub actual: NativeStreamInfo,
    pub dropped_frames: u64,
    pub silent_frames: u64,
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
}
