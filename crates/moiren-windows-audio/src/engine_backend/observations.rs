//! Preparation records are immutable per owner; RT bridge counters are bounded
//! atomic observations. Snapshot cloning and publication only run on control.
use crate::{
    clock_bridge::{BridgeObserver, BridgeSnapshot},
    latency::{NativeStreamTiming, PreparedInputQueue, PreparedRenderQueue},
    shared_stream::SharedPeriodReport,
    virtual_owner::BoundaryProbeSample,
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
    VirtualPlayback,
    VirtualMicrophone,
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
    pub boundary_probes: Vec<NativeBoundaryProbe>,
}

/// Live control-side sample of one virtual boundary's carrier probe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct NativeBoundaryProbe {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub kind: NativeStreamKind,
    /// Prepared storage headroom in native 48 kHz frames, not latency.
    pub capacity_frames: u32,
    /// Total pre-transfer content target; absent without drift adaptation.
    pub target_frames: Option<u32>,
    /// Current post-transfer occupancy in native 48 kHz frames.
    pub level_frames: u32,
    pub applied_milli_ppm: i32,
    pub xruns: u64,
    pub soft_events: u64,
}

impl From<(NodeId, NativeStreamKind, BoundaryProbeSample)> for NativeBoundaryProbe {
    fn from((node, kind, sample): (NodeId, NativeStreamKind, BoundaryProbeSample)) -> Self {
        Self {
            node,
            kind,
            capacity_frames: sample.capacity_frames,
            target_frames: sample.target_frames,
            level_frames: sample.level_frames,
            applied_milli_ppm: sample.applied_milli_ppm,
            xruns: sample.xruns,
            soft_events: sample.soft_events,
        }
    }
}
#[derive(Clone, Default)]
pub struct NativeObservations(Arc<Mutex<Option<Arc<Session>>>>);
#[cfg(any(windows, test))]
pub(crate) struct ObservationLease {
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
    probes: Vec<(
        NodeId,
        NativeStreamKind,
        std::sync::Arc<crate::virtual_owner::BoundaryProbe>,
    )>,
}
impl NativeObservations {
    #[cfg(any(windows, test))]
    pub(crate) fn lease(&self, generation: u64) -> ObservationLease {
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
        let boundary_probes = session
            .probes
            .iter()
            .map(|(node, kind, probe)| (*node, *kind, probe.snapshot()).into())
            .collect();
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
            boundary_probes,
        })
    }
    #[cfg(any(windows, test))]
    pub(crate) fn publish(
        &self,
        generation: u64,
        streams: Vec<NativeStreamObservation>,
        bridges: Vec<(NodeId, BridgeObserver)>,
        probes: Vec<(
            NodeId,
            NativeStreamKind,
            std::sync::Arc<crate::virtual_owner::BoundaryProbe>,
        )>,
    ) {
        *self.0.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(Arc::new(Session {
            generation,
            running: AtomicBool::new(false),
            streams,
            probes,
            bridges,
        }));
    }
    #[cfg(any(windows, test))]
    pub(crate) fn running(&self, generation: u64) {
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
    pub(crate) fn clear(&self, generation: u64) {
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
    use crate::virtual_owner::{BoundaryProbe, VirtualMicrophoneSink};
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use moiren_engine::{
        boundary::{RtAudioSink, audio_bridge},
        buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
        processor::ProcessContext,
    };

    fn sampled_boundary(
        adapted: bool,
    ) -> (
        NativeObservationSnapshot,
        crate::virtual_owner::BoundaryProbeSample,
    ) {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::Sink, 2).unwrap();
        let (writer, _reader) = audio_bridge(2, 512, 1 << 20).unwrap();
        let probe = Arc::new(BoundaryProbe::default());
        let mut sink = VirtualMicrophoneSink::new(writer, 2, 32, 48_000, 1 << 20)
            .unwrap()
            .with_probe(probe.clone());
        if adapted {
            sink = sink.with_clock_drift_target(128).unwrap();
        }
        let observations = NativeObservations::default();
        observations.publish(
            7,
            Vec::new(),
            Vec::new(),
            vec![(node, NativeStreamKind::VirtualMicrophone, probe.clone())],
        );
        observations.running(7);
        let mut arena = BufferArena::<f32>::new(
            &[BufferSlotLayout {
                channels: 2,
                capacity_frames: 32,
            }],
            1 << 20,
        )
        .unwrap();
        let read = arena
            .prepare_io(&[PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            }])
            .unwrap();
        // An undrained real carrier overflows and publishes cumulative events.
        for block in 0..64 {
            arena
                .with_io(&read, 0..32, |io| {
                    let ProcessIo::ReadOnly { inputs } = io else {
                        unreachable!()
                    };
                    sink.write(
                        &ProcessContext {
                            timeline_epoch: 1,
                            timeline_start: block * 32,
                            frames: 32,
                            processing_sr: 48_000.0,
                        },
                        inputs.get(0).unwrap(),
                    );
                })
                .unwrap();
        }
        (observations.snapshot().unwrap(), probe.snapshot())
    }

    #[test]
    fn native_boundary_snapshot_preserves_prepared_geometry_and_live_events() {
        let (snapshot, sample) = sampled_boundary(true);
        assert_eq!(snapshot.owner_generation, 7);
        assert!(snapshot.running);
        assert_eq!(sample.capacity_frames, 512);
        assert_eq!(sample.target_frames, Some(128));
        assert!(sample.level_frames > 0);
        assert!(sample.applied_milli_ppm != 0);
        assert!(sample.xruns > 0);
        assert!(sample.soft_events > 0);
        let native = snapshot.boundary_probes[0];
        assert_eq!(native.kind, NativeStreamKind::VirtualMicrophone);
        assert_eq!(native.level_frames, sample.level_frames);
        assert_eq!(native.applied_milli_ppm, sample.applied_milli_ppm);
        assert_eq!(native.xruns, sample.xruns);
        assert_eq!(native.soft_events, sample.soft_events);
        let json = serde_json::to_value(native).unwrap();
        assert_eq!(json["capacity_frames"], 512);
        assert_eq!(json["target_frames"], 128);
    }

    #[test]
    fn native_boundary_snapshot_keeps_unadapted_target_unknown() {
        let (snapshot, sample) = sampled_boundary(false);
        assert_eq!(sample.target_frames, None);
        let json = serde_json::to_value(snapshot.boundary_probes[0]).unwrap();
        assert_eq!(json["capacity_frames"], 512);
        assert!(
            json.as_object().unwrap().contains_key("target_frames"),
            "unknown target must be retained explicitly"
        );
        assert!(json["target_frames"].is_null());
    }

    #[test]
    fn stale_owner_cleanup_cannot_clear_replacement_observations() {
        let observations = NativeObservations::default();
        observations.publish(1, Vec::new(), Vec::new(), Vec::new());
        let old = observations.lease(1);
        observations.running(1);
        assert!(observations.snapshot().unwrap().running);
        observations.publish(2, Vec::new(), Vec::new(), Vec::new());
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
