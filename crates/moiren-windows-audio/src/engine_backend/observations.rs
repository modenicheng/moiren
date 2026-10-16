//! Preparation records are immutable per owner; RT bridge counters are bounded
//! atomic observations. Snapshot cloning and publication only run on control.
use crate::{
    clock_bridge::{BridgeObserver, BridgeSnapshot},
    latency::{NativeStreamTiming, PreparedInputQueue, PreparedRenderQueue},
    shared_stream::SharedPeriodReport,
};
use moiren_core::graph::NodeId;
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeStreamKind {
    MasterOutput,
    FollowerOutput,
    PhysicalInput,
    ProcessInput,
}
#[derive(Debug, Clone, Serialize)]
pub struct NativeStreamObservation {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub kind: NativeStreamKind,
    /// Process loopback has a virtual stream, not a measured device period.
    pub timing: Option<NativeStreamTiming>,
    pub shared_period: Option<SharedPeriodReport>,
    pub render_queue: Option<PreparedRenderQueue>,
    pub input_queue: Option<PreparedInputQueue>,
}
#[derive(Debug, Clone, Serialize)]
pub struct NativeBridgeObservation {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub counters: BridgeSnapshot,
}

fn serialize_node<S: serde::Serializer>(node: &NodeId, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&format_args!("{node:?}"))
}
#[derive(Debug, Clone, Serialize)]
pub struct NativeObservationSnapshot {
    pub owner_generation: u64,
    pub running: bool,
    pub streams: Vec<NativeStreamObservation>,
    pub bridges: Vec<NativeBridgeObservation>,
}
#[derive(Clone, Default)]
pub struct NativeObservations(Arc<Mutex<Option<Arc<Session>>>>);
#[cfg(any(windows, test))]
pub(super) struct ObservationLease {
    observations: NativeObservations,
    generation: u64,
}
#[cfg(any(windows, test))]
impl Drop for ObservationLease {
    fn drop(&mut self) {
        self.observations.clear(self.generation);
    }
}
pub(super) struct Session {
    generation: u64,
    running: AtomicBool,
    streams: Vec<NativeStreamObservation>,
    bridges: Vec<(NodeId, BridgeObserver)>,
}
impl NativeObservations {
    #[cfg(any(windows, test))]
    pub(super) fn lease(&self, generation: u64) -> ObservationLease {
        ObservationLease {
            observations: self.clone(),
            generation,
        }
    }
    pub fn snapshot(&self) -> Option<NativeObservationSnapshot> {
        let session = self
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()?;
        Some(NativeObservationSnapshot {
            owner_generation: session.generation,
            running: session.running.load(Ordering::Acquire),
            streams: session.streams.clone(),
            bridges: session
                .bridges
                .iter()
                .map(|(node, observer)| NativeBridgeObservation {
                    node: *node,
                    counters: observer.snapshot(),
                })
                .collect(),
        })
    }
    #[cfg(any(windows, test))]
    pub(super) fn publish(
        &self,
        generation: u64,
        streams: Vec<NativeStreamObservation>,
        bridges: Vec<(NodeId, BridgeObserver)>,
    ) {
        *self.0.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(Arc::new(Session {
            generation,
            running: AtomicBool::new(false),
            streams,
            bridges,
        }));
    }
    #[cfg(any(windows, test))]
    pub(super) fn running(&self, generation: u64) {
        if let Some(session) = self
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .filter(|session| session.generation == generation)
        {
            session.running.store(true, Ordering::Release);
        }
    }
    #[cfg(any(windows, test))]
    pub(super) fn clear(&self, generation: u64) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if state
            .as_ref()
            .is_some_and(|session| session.generation == generation)
        {
            *state = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_owner_cleanup_cannot_clear_replacement_observations() {
        let observations = NativeObservations::default();
        observations.publish(1, Vec::new(), Vec::new());
        let old = observations.lease(1);
        observations.running(1);
        assert!(observations.snapshot().unwrap().running);
        observations.publish(2, Vec::new(), Vec::new());
        let new = observations.lease(2);
        drop(old);
        observations.running(1);
        let snapshot = observations.snapshot().unwrap();
        assert_eq!(snapshot.owner_generation, 2);
        assert!(!snapshot.running);
        drop(new);
        assert!(observations.snapshot().is_none());
    }
}
