//! Preparation records are immutable per owner; RT bridge counters are bounded
//! atomic observations. Snapshot cloning and publication only run on control.
use crate::{
    clock_bridge::{BridgeObserver, BridgeSnapshot},
    latency::shared_period::SharedPeriodReport,
    latency::{NativeStreamTiming, PreparedInputQueue, PreparedRenderQueue},
    virtual_owner::{BoundaryProbeSample, PlaybackBoundaryEvents, PreparedVirtualBudget},
};
use moiren_core::graph::NodeId;
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[path = "native_observations/process_trace.rs"]
mod process_trace;
pub use process_trace::{
    NativeProcessTraceCollector, NativeProcessWindowCandidate, PreparedNativeProcessWindow,
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
    /// Immutable VAD preparation evidence, for playback and microphone alike.
    /// Declared delivery bounds do not establish a negotiated native packet.
    /// None for physical and Process Loopback streams.
    pub virtual_budget: Option<PreparedVirtualBudget>,
}
#[derive(Debug, Clone, Serialize)]
pub struct NativeBridgeObservation {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub counters: BridgeSnapshot,
    pub queue_history: Option<crate::latency::queue::QueueHistorySnapshot>,
    pub packet_history: Option<crate::latency::packets::PacketHistorySnapshot>,
}

fn serialize_node<S: serde::Serializer>(node: &NodeId, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&format_args!("{node:?}"))
}
#[derive(Debug, Clone, Serialize)]
pub struct NativeObservationSnapshot {
    pub owner_generation: u64,
    /// Publication state only. False does not prove workers joined or cleanup
    /// succeeded; callers must establish teardown completion separately.
    pub running: bool,
    pub streams: Vec<NativeStreamObservation>,
    pub bridges: Vec<NativeBridgeObservation>,
    pub boundary_probes: Vec<NativeBoundaryProbe>,
    pub render_padding: Vec<NativeRenderPaddingObservation>,
    pub process_delivery: Vec<NativeProcessDeliveryObservation>,
}

/// Service boundary events belonging to this exact source and transport owner.
/// Independent histories are coherent individually, not a cross-owner audio
/// transaction. Process service timestamps never establish sample age.
#[derive(Debug, Clone, Serialize)]
pub struct NativeProcessDeliveryObservation {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub process: crate::process_loopback::ProcessIdentity,
    pub owner_generation: u64,
    pub input_sample_rate: u32,
    pub consumer_sample_rate: u32,
    pub prepared_consumer_demand_frames: u32,
    /// Exact options admitted for this owner, distinct from collected timing.
    pub admitted_bridge: crate::process_loopback::ProcessBridgeOptions,
    /// Origin of an explicitly reused measured profile, never this new owner.
    pub measured_source_generation: Option<u64>,
    pub delivery: Option<crate::latency::InputDeliverySnapshot>,
    pub producer: Option<crate::latency::events::EventHistorySnapshot>,
    pub consumer: Option<crate::latency::events::EventHistorySnapshot>,
    /// Actual source-owned native-frame inventory. Pre-read QPC does not
    /// timestamp the later post-read boundary or establish source sample age.
    pub queue_history: Option<crate::latency::queue::QueueHistorySnapshot>,
    /// Offered supply and admitted publication with host-QPC envelopes. Not
    /// source sample age, exact commit instants or alternate-target admission.
    pub packet_history: Option<crate::latency::packets::PacketHistorySnapshot>,
    pub counters: BridgeSnapshot,
}

impl NativeProcessDeliveryObservation {
    /// Caller-supplied structural trace only: this public DTO and its labels
    /// are editable, so identity agreement does not authenticate native origin.
    /// Registry retirement/running state does not prove unique-writer closure.
    pub fn ordered_trace(
        &self,
        expected_process: &crate::process_loopback::ProcessIdentity,
        expected_generation: u64,
    ) -> Result<
        crate::process_loopback::ObservedProcessTrace,
        crate::process_loopback::ProcessReplayError,
    > {
        use crate::process_loopback::{ObservedProcessTrace, ProcessReplayError as Error};
        if !self.process.same_process(expected_process) {
            return Err(Error::IdentityMismatch);
        }
        if self.owner_generation != expected_generation {
            return Err(Error::GenerationMismatch);
        }
        ObservedProcessTrace::from_observations(
            self.process.clone(),
            self.owner_generation,
            self.input_sample_rate,
            self.consumer_sample_rate,
            self.prepared_consumer_demand_frames,
            self.packet_history
                .clone()
                .ok_or(Error::IncompleteHistory)?,
            self.queue_history.clone().ok_or(Error::IncompleteHistory)?,
        )
    }

    /// Validate individually coherent native histories off RT. A contended or
    /// incomplete snapshot is refused; callers can collect another snapshot.
    pub fn measured(
        &self,
        expected_process: &crate::process_loopback::ProcessIdentity,
        expected_generation: u64,
    ) -> Result<
        crate::process_loopback::MeasuredProcessDelivery,
        crate::process_loopback::MeasuredAdmissionError,
    > {
        use crate::process_loopback::{MeasuredAdmissionError as Error, MeasuredProcessDelivery};
        if !self.process.same_process(expected_process) {
            return Err(Error::IdentityMismatch);
        }
        if self.owner_generation != expected_generation {
            return Err(Error::GenerationMismatch);
        }
        if self.admitted_bridge.max_ingress_elapsed_100ns.is_some() {
            return Err(Error::IngressElapsedUnsupported);
        }
        let delivery = self.delivery.ok_or(Error::MissingObservation {
            observation: "packet delivery",
        })?;
        if self.counters.native_delivery != Some(delivery) {
            return Err(Error::InconsistentDelivery);
        }
        MeasuredProcessDelivery::from_observations(
            self.process.clone(),
            self.owner_generation,
            self.input_sample_rate,
            self.consumer_sample_rate,
            self.prepared_consumer_demand_frames,
            delivery,
            self.producer.clone().ok_or(Error::MissingObservation {
                observation: "producer",
            })?,
            self.consumer.clone().ok_or(Error::MissingObservation {
                observation: "consumer",
            })?,
        )
    }
}

#[derive(Clone)]
pub(super) struct ProcessDeliverySource {
    pub node: NodeId,
    pub process: crate::process_loopback::ProcessIdentity,
    pub input_sample_rate: u32,
    pub consumer_sample_rate: u32,
    pub prepared_consumer_demand_frames: u32,
    pub admitted_bridge: crate::process_loopback::ProcessBridgeOptions,
    pub measured_source_generation: Option<u64>,
    pub bridge: BridgeObserver,
    pub consumer: Option<crate::latency::events::EventHistoryObserver>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeRenderPaddingObservation {
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    /// Native queue observation only; None before the first wake, on stop,
    /// or during contention. Never a physical end-to-end latency estimate.
    pub sample: Option<crate::latency::render::RenderPaddingSnapshot>,
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
    pub playback_events: Option<PlaybackBoundaryEvents>,
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
            playback_events: sample.playback_events,
        }
    }
}
#[derive(Clone, Default)]
pub struct NativeObservations(Arc<Mutex<Option<Arc<Session>>>>);

/// Control-side read access to exactly one published owner generation.
/// Does not retain audio resources or republish a cleared session. Counters
/// remain live; read final values only after independently completed teardown.
#[derive(Clone)]
pub struct NativeObservationReadHandle(Arc<Session>);

#[cfg(test)]
pub(crate) struct ObservationLease {
    observations: NativeObservations,
    generation: u64,
}
#[cfg(test)]
impl Drop for ObservationLease {
    fn drop(&mut self) {
        self.observations.clear(self.generation);
    }
}
pub(super) struct Session {
    process_delivery: Vec<ProcessDeliverySource>,
    generation: u64,
    running: AtomicBool,
    streams: Vec<NativeStreamObservation>,
    bridges: Vec<(NodeId, BridgeObserver)>,
    probes: Vec<(
        NodeId,
        NativeStreamKind,
        std::sync::Arc<crate::virtual_owner::BoundaryProbe>,
    )>,
    renders: Vec<(NodeId, crate::latency::render::RenderPaddingObserver)>,
}
impl NativeObservationReadHandle {
    /// Publication state only, not a successful-start or teardown witness.
    /// Immutable owner identity prevents registry replacement from redirecting
    /// admission while avoiding a full history snapshot just to inspect it.
    pub fn running_generation(&self) -> Option<u64> {
        self.0
            .running
            .load(Ordering::Acquire)
            .then_some(self.0.generation)
    }

    /// Read closed histories from this exact retained source/owner, not public
    /// caller-supplied labels. Registry replacement cannot redirect this handle.
    /// Writer closure still does not prove successful transport cleanup.
    pub fn ordered_process_trace(
        &self,
        node: NodeId,
        expected_process: &crate::process_loopback::ProcessIdentity,
        expected_generation: u64,
    ) -> Result<
        crate::process_loopback::ObservedProcessTrace,
        crate::process_loopback::ProcessReplayError,
    > {
        use crate::process_loopback::ProcessReplayError as Error;
        if self.0.generation != expected_generation {
            return Err(Error::GenerationMismatch);
        }
        let observation = self
            .snapshot()
            .process_delivery
            .into_iter()
            .find(|value| value.node == node)
            .ok_or(Error::InvalidIdentity)?;
        Ok(observation
            .ordered_trace(expected_process, expected_generation)?
            .with_retained_owner_origin())
    }

    pub fn snapshot(&self) -> NativeObservationSnapshot {
        let session = &self.0;
        let boundary_probes = session
            .probes
            .iter()
            .map(|(node, kind, probe)| (*node, *kind, probe.snapshot()).into())
            .collect();
        NativeObservationSnapshot {
            owner_generation: session.generation,
            running: session.running.load(Ordering::Acquire),
            process_delivery: session
                .process_delivery
                .iter()
                .map(|source| {
                    let counters = source.bridge.snapshot();
                    NativeProcessDeliveryObservation {
                        node: source.node,
                        process: source.process.clone(),
                        owner_generation: session.generation,
                        input_sample_rate: source.input_sample_rate,
                        consumer_sample_rate: source.consumer_sample_rate,
                        prepared_consumer_demand_frames: source.prepared_consumer_demand_frames,
                        admitted_bridge: source.admitted_bridge,
                        measured_source_generation: source.measured_source_generation,
                        delivery: counters.native_delivery,
                        producer: source.bridge.delivery_events(),
                        consumer: source
                            .consumer
                            .as_ref()
                            .and_then(|consumer| consumer.snapshot()),
                        queue_history: source.bridge.queue_history(),
                        packet_history: source.bridge.packet_history(),
                        counters,
                    }
                })
                .collect(),
            streams: session.streams.clone(),
            bridges: session
                .bridges
                .iter()
                .map(|(node, observer)| NativeBridgeObservation {
                    node: *node,
                    counters: observer.snapshot(),
                    queue_history: observer.queue_history(),
                    packet_history: observer.packet_history(),
                })
                .collect(),
            boundary_probes,
            render_padding: session
                .renders
                .iter()
                .map(|(node, observer)| NativeRenderPaddingObservation {
                    node: *node,
                    sample: observer.try_snapshot(),
                })
                .collect(),
        }
    }
}

impl NativeObservations {
    #[cfg(test)]
    pub(crate) fn lease(&self, generation: u64) -> ObservationLease {
        ObservationLease {
            observations: self.clone(),
            generation,
        }
    }

    /// Retain the current generation's metadata and shared counters off RT.
    /// Clearing or replacing the live session cannot redirect this handle.
    pub fn retain_session(&self) -> Option<NativeObservationReadHandle> {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .map(|session| NativeObservationReadHandle(session.clone()))
    }

    pub fn snapshot(&self) -> Option<NativeObservationSnapshot> {
        self.retain_session().map(|handle| handle.snapshot())
    }
    #[cfg(test)]
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
        self.publish_with_renders(generation, streams, bridges, probes, Vec::new());
    }
    #[cfg(test)]
    pub(crate) fn publish_with_renders(
        &self,
        generation: u64,
        streams: Vec<NativeStreamObservation>,
        bridges: Vec<(NodeId, BridgeObserver)>,
        probes: Vec<(
            NodeId,
            NativeStreamKind,
            std::sync::Arc<crate::virtual_owner::BoundaryProbe>,
        )>,
        renders: Vec<(NodeId, crate::latency::render::RenderPaddingObserver)>,
    ) {
        self.publish_with_process(generation, streams, bridges, probes, renders, Vec::new());
    }
    #[cfg(any(windows, test))]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_with_process(
        &self,
        generation: u64,
        streams: Vec<NativeStreamObservation>,
        bridges: Vec<(NodeId, BridgeObserver)>,
        probes: Vec<(
            NodeId,
            NativeStreamKind,
            std::sync::Arc<crate::virtual_owner::BoundaryProbe>,
        )>,
        renders: Vec<(NodeId, crate::latency::render::RenderPaddingObserver)>,
        process_delivery: Vec<ProcessDeliverySource>,
    ) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(previous) = state.as_ref() {
            previous.running.store(false, Ordering::Release);
        }
        *state = Some(Arc::new(Session {
            process_delivery,
            generation,
            running: AtomicBool::new(false),
            streams,
            probes,
            bridges,
            renders,
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
#[path = "native_observations/process_tests.rs"]
mod process_tests;

#[cfg(test)]
#[path = "native_observations/process_trace_tests.rs"]
mod process_trace_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
        virtual_owner::{BoundaryProbe, VirtualBufferOptions, VirtualMicrophoneSink},
    };
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use moiren_engine::{
        boundary::{RtAudioSink, audio_bridge},
        buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
        processor::ProcessContext,
    };

    #[test]
    fn retained_session_survives_clear_without_resurrection_or_replacement_mixing() {
        let observations = NativeObservations::default();
        assert!(observations.retain_session().is_none());
        observations.publish(1, Vec::new(), Vec::new(), Vec::new());
        observations.running(1);
        let old = observations.retain_session().unwrap();
        assert!(old.snapshot().running);
        observations.publish(2, Vec::new(), Vec::new(), Vec::new());
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
    fn virtual_budget_survives_snapshot_serialization_without_native_packet_inference() {
        let mut graph = LogicalGraph::new();
        let playback = graph.create_node(NodeKind::Source, 2).unwrap();
        let microphone = graph.create_node(NodeKind::Sink, 2).unwrap();
        let physical = graph.create_node(NodeKind::Source, 2).unwrap();
        let target = VirtualBufferOptions {
            input: TransportInputPolicy::Target {
                capacity_frames: 1024,
                target_frames: 321,
                jitter_frames: 32,
            },
            delivery: Some(InputDeliveryTiming {
                sample_rate: 48_000,
                max_packet_frames: 128,
                max_gap_frames: 128,
                source: DeliveryTimingSource::Declared,
            }),
            recovery_cap_periods: Some(1),
        };
        for options in [VirtualBufferOptions::default(), target] {
            let budget = options.prepare_recovery(48_000, 128).unwrap();
            let streams = [
                (playback, NativeStreamKind::VirtualPlayback, Some(budget)),
                (
                    microphone,
                    NativeStreamKind::VirtualMicrophone,
                    Some(budget),
                ),
                (physical, NativeStreamKind::PhysicalInput, None),
            ]
            .into_iter()
            .map(|(node, kind, virtual_budget)| NativeStreamObservation {
                node,
                kind,
                timing: None,
                shared_period: None,
                render_queue: None,
                input_queue: virtual_budget.and_then(|budget| budget.queue),
                virtual_budget,
            })
            .collect();
            let observations = NativeObservations::default();
            observations.publish(7, streams, Vec::new(), Vec::new());
            observations.running(7);
            let retained = observations.retain_session().unwrap();
            observations.clear(7);
            assert!(observations.snapshot().is_none());
            let mut snapshot = retained.snapshot();
            assert_eq!(snapshot.owner_generation, 7);
            assert!(!snapshot.running);
            let json = serde_json::to_value(&snapshot).unwrap();
            for (index, kind) in [
                NativeStreamKind::VirtualPlayback,
                NativeStreamKind::VirtualMicrophone,
            ]
            .into_iter()
            .enumerate()
            {
                let stream = &snapshot.streams[index];
                assert_eq!(stream.kind, kind);
                assert_eq!(stream.virtual_budget, Some(budget));
                assert_eq!(stream.input_queue, budget.queue);
                let published = &json["streams"][index]["virtual_budget"];
                assert_eq!(published, &serde_json::to_value(budget).unwrap());
                assert!(
                    published
                        .as_object()
                        .unwrap()
                        .contains_key("native_packet_frames")
                );
                assert!(published["native_packet_frames"].is_null());
            }
            assert_eq!(snapshot.streams[2].virtual_budget, None);
            assert!(json["streams"][2]["virtual_budget"].is_null());
            // A caller's owned snapshot cannot modify the session's admission.
            snapshot.streams[0].virtual_budget = None;
            assert_eq!(retained.snapshot().streams[0].virtual_budget, Some(budget));
        }
    }

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
        let retained = observations.retain_session().unwrap();
        observations.clear(7);
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
        assert!(observations.snapshot().is_none());
        (retained.snapshot(), probe.snapshot())
    }

    #[test]
    fn native_boundary_snapshot_preserves_prepared_geometry_and_live_events() {
        let (snapshot, sample) = sampled_boundary(true);
        assert_eq!(snapshot.owner_generation, 7);
        assert!(!snapshot.running);
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
    fn native_playback_event_classes_keep_exact_counts_and_unknown() {
        let node = LogicalGraph::new()
            .create_node(NodeKind::Source, 2)
            .unwrap();
        for events in [
            None,
            Some(PlaybackBoundaryEvents {
                initial_prime_muted_frames: 32,
                recovery_prime_muted_frames: (1 << 53) + 7,
                short_read_callbacks: u64::MAX,
                saturated: true,
            }),
        ] {
            let sample = BoundaryProbeSample {
                soft_events: 47,
                playback_events: events,
                ..Default::default()
            };
            let native =
                NativeBoundaryProbe::from((node, NativeStreamKind::VirtualPlayback, sample));
            assert_eq!(native.playback_events, events);
            assert_eq!(native.soft_events, 47);
            let json = serde_json::to_value(native).unwrap();
            assert_eq!(
                json["playback_events"],
                serde_json::to_value(events).unwrap()
            );
        }
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

    #[test]
    fn native_padding_projection_is_live_and_old_owner_cleanup_cannot_clear_it() {
        use crate::latency::render::{RenderPaddingObserver, RenderPaddingSample};
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::Sink, 2).unwrap();
        let observer = RenderPaddingObserver::new();
        let mut owner = observer.new_owner();
        let observations = NativeObservations::default();
        observations.publish_with_renders(
            1,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![(node, observer)],
        );
        let retained = observations.retain_session().unwrap();
        observations.running(1);
        let old_lease = observations.lease(1);
        let first = observations.snapshot().unwrap();
        assert_eq!(first.render_padding.len(), 1);
        assert!(first.render_padding[0].sample.is_none());
        owner.publish(RenderPaddingSample {
            padding_frames: 21,
            capacity_frames: 240,
            sample_rate: Some(48_000),
            period_frames: Some(48),
        });
        let live = observations.snapshot().unwrap();
        assert_eq!(live.render_padding[0].node, node);
        assert_eq!(live.render_padding[0].sample.unwrap().padding_frames, 21);
        let replacement = RenderPaddingObserver::new();
        let mut new_owner = replacement.new_owner();
        new_owner.publish(RenderPaddingSample {
            padding_frames: 17,
            capacity_frames: 128,
            sample_rate: None,
            period_frames: None,
        });
        observations.publish_with_renders(
            2,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![(node, replacement)],
        );
        drop(old_lease);
        assert!(!retained.snapshot().running);
        assert_eq!(retained.snapshot().owner_generation, 1);
        assert_eq!(
            retained.snapshot().render_padding[0]
                .sample
                .unwrap()
                .padding_frames,
            21
        );
        owner.publish(RenderPaddingSample {
            padding_frames: 23,
            capacity_frames: 240,
            sample_rate: Some(48_000),
            period_frames: Some(48),
        });
        assert_eq!(
            retained.snapshot().render_padding[0]
                .sample
                .unwrap()
                .padding_frames,
            23
        );
        owner.invalidate();
        assert!(retained.snapshot().render_padding[0].sample.is_none());
        let current = observations.snapshot().unwrap();
        assert_eq!(current.owner_generation, 2);
        assert_eq!(current.render_padding[0].sample.unwrap().padding_frames, 17);
        new_owner.invalidate();
        assert!(
            observations.snapshot().unwrap().render_padding[0]
                .sample
                .is_none()
        );
    }
}
