use super::*;
use crate::service::{InputSelection, SessionGeneration};
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_windows_audio::{
    clock_bridge::BridgeSnapshot,
    engine_backend::NativeProcessDeliveryObservation,
    latency::{InputDeliveryRecorder, events::event_history},
    process_loopback::{ProcessBridgeOptions, ProcessIdentity},
};

pub(crate) fn observation(node: moiren_core::graph::NodeId) -> NativeProcessDeliveryObservation {
    let (mut producer, producer_observer) = event_history();
    let (mut consumer, consumer_observer) = event_history();
    let mut delivery = InputDeliveryRecorder::new(None).unwrap();
    for stamp in [1_000_000, 1_020_000, 1_040_000, 1_060_000] {
        producer.record(96, Some(stamp));
        consumer.record(96, Some(stamp + 1000));
        delivery.packet(96, Some(stamp - 1));
        delivery.callback_burst(96);
    }
    NativeProcessDeliveryObservation {
        node,
        process: ProcessIdentity {
            pid: 42,
            creation_time_100ns: 123,
            executable_name: "source.exe".into(),
        },
        owner_generation: 7,
        input_sample_rate: 48_000,
        consumer_sample_rate: 48_000,
        prepared_consumer_demand_frames: 96,
        admitted_bridge: ProcessBridgeOptions {
            max_demand_frames: 96,
            ..Default::default()
        },
        measured_source_generation: None,
        delivery: Some(delivery.snapshot()),
        producer: producer_observer.snapshot(),
        consumer: consumer_observer.snapshot(),
        queue_history: None,
        packet_history: None,
        counters: BridgeSnapshot {
            native_delivery: Some(delivery.snapshot()),
            ..Default::default()
        },
    }
}

#[test]
fn selection_uses_typed_evidence_and_keeps_exact_origin() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Source, 2).unwrap();
    let request = MeasuredProcessRequest {
        node,
        session_generation: SessionGeneration(3),
        native_owner_generation: 7,
        graph_revision: 11,
        capacity_frames: 2048,
        target_frames: 256,
        jitter_frames: 0,
    };
    let profile = MeasuredProcessProfile::from_observation(1, request, &observation(node)).unwrap();
    assert_eq!(profile.summary().request, request);
    assert_eq!(profile.summary().selection_id, 1);
    assert_eq!(profile.summary().process_pid, 42);
    assert_eq!(profile.summary().process_creation_time_100ns, 123);
    assert_eq!(profile.summary().delivery.max_packet_frames, 96);
    assert_eq!(profile.summary().delivery.max_gap_frames, 96);
    let json = serde_json::to_value(profile.summary()).unwrap();
    assert_eq!(json["selection_id"], 1);
    assert_eq!(json["request"]["native_owner_generation"], 7);
    assert!(json.get("qualified").is_none());
    profile
        .validate_source(&InputSelection::Process {
            pid: 42,
            creation_time_100ns: 123,
        })
        .unwrap();
    assert!(
        profile
            .validate_source(&InputSelection::Process {
                pid: 42,
                creation_time_100ns: 124
            })
            .is_err()
    );
    assert!(
        profile
            .validate_source(&InputSelection::Tone {
                frequency_hz: 440.0
            })
            .is_err()
    );
}

#[test]
fn selection_refuses_stale_origin_unknown_observations_and_insufficient_target() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Source, 2).unwrap();
    let mut request = MeasuredProcessRequest {
        node,
        session_generation: SessionGeneration(3),
        native_owner_generation: 7,
        graph_revision: 11,
        capacity_frames: 2048,
        target_frames: 256,
        jitter_frames: 0,
    };
    request.native_owner_generation = 8;
    assert!(MeasuredProcessProfile::from_observation(1, request, &observation(node)).is_err());
    request.native_owner_generation = 7;
    request.target_frames = 128;
    assert!(MeasuredProcessProfile::from_observation(1, request, &observation(node)).is_err());
    request.target_frames = 256;
    let mut unknown = observation(node);
    unknown.consumer = None;
    assert!(MeasuredProcessProfile::from_observation(1, request, &unknown).is_err());
    request.session_generation = SessionGeneration(0);
    assert!(request.validate().is_err());
}
