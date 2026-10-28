use super::*;
use crate::{
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    latency::events::event_history,
    process_loopback::ProcessIdentity,
};
use moiren_core::graph::{LogicalGraph, NodeKind};

#[test]
fn retained_process_observations_keep_late_native_events_and_original_generation() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Source, 2).unwrap();
    let (mut ingress, mut source, bridge) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    let (mut consumer, consumer_observer) = event_history();
    let process = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    };
    let observations = NativeObservations::default();
    observations.publish_with_process(
        7,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![ProcessDeliverySource {
            node,
            process: process.clone(),
            input_sample_rate: 48_000,
            consumer_sample_rate: 96_000,
            prepared_consumer_demand_frames: 512,
            admitted_bridge: crate::process_loopback::ProcessBridgeOptions {
                engine_sample_rate: 96_000,
                max_demand_frames: 512,
                ..Default::default()
            },
            measured_source_generation: None,
            bridge,
            consumer: Some(consumer_observer),
        }],
    );
    let retained = observations.retain_session().unwrap();
    let before = retained.snapshot();
    let before = &before.process_delivery[0];
    assert!(before.delivery.is_none());
    assert!(before.producer.as_ref().unwrap().events.is_empty());
    assert!(before.consumer.as_ref().unwrap().events.is_empty());

    observations.clear(7);
    observations.publish(8, Vec::new(), Vec::new(), Vec::new());
    ingress
        .push_packet_with_publication_clock(
            &[],
            crate::clock_bridge::CapturePacket {
                frames: 4,
                flags: crate::stats::SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
            || Some(115),
        )
        .unwrap();
    // A final consumption publication can arrive after registry retirement.
    consumer.record(128, Some(110));
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(120))
        .unwrap();
    let final_snapshot = retained.snapshot();
    assert!(!final_snapshot.running);
    assert_eq!(final_snapshot.owner_generation, 7);
    let final_process = &final_snapshot.process_delivery[0];
    assert_eq!(final_process.owner_generation, 7);
    assert_eq!(final_process.node, node);
    assert_eq!(final_process.process, process);
    assert_eq!(final_process.input_sample_rate, 48_000);
    assert_eq!(final_process.consumer_sample_rate, 96_000);
    assert_eq!(final_process.prepared_consumer_demand_frames, 512);
    assert_eq!(final_process.counters.captured_frames, 4);
    let packet_history = final_process.packet_history.as_ref().unwrap();
    assert_eq!(packet_history.total_events, 1);
    assert_eq!(packet_history.events[0].sample.accepted_frames, 4);
    assert_eq!(packet_history.events[0].sample.qpc_before_100ns, Some(115));
    assert_eq!(packet_history.events[0].sample.qpc_after_100ns, Some(115));
    assert!(final_process.producer.as_ref().unwrap().events.is_empty());
    assert_eq!(
        final_process.consumer.as_ref().unwrap().events[0].frames,
        128
    );
    let json = serde_json::to_value(final_process).unwrap();
    assert!(json["delivery"].is_null());
    assert_eq!(json["packet_history"]["published_frames"], 4);
    assert_eq!(
        json["packet_history"]["events"][0]["sample"]["origin"],
        "native_capture"
    );
    assert_eq!(json["process"]["creation_time_100ns"], 123);
    assert!(json["measured_source_generation"].is_null());
    assert_eq!(json["admitted_bridge"]["max_demand_frames"], 512);
    assert!(json["producer"]["events"].as_array().unwrap().is_empty());
    assert_eq!(json["consumer"]["events"][0]["qpc_100ns"], 110);
    assert_eq!(json["queue_history"]["total_events"], 1);
    assert_eq!(
        json["queue_history"]["events"][0]["sample"]["output_frames"],
        0
    );
    assert_eq!(
        json["queue_history"]["events"][0]["sample"]["qpc_before_100ns"],
        120
    );
    assert!(observations.snapshot().unwrap().process_delivery.is_empty());
    assert_eq!(observations.snapshot().unwrap().owner_generation, 8);
}

#[test]
fn measured_projection_requires_complete_matching_observations() {
    use crate::{
        latency::InputDeliveryRecorder, process_loopback::MeasuredAdmissionError as Error,
    };
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Source, 2).unwrap();
    let process = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    };
    let (mut producer, producer_observer) = event_history();
    let (mut consumer, consumer_observer) = event_history();
    let mut delivery = InputDeliveryRecorder::new(None).unwrap();
    for stamp in [1_000_000, 1_020_000, 1_040_000] {
        producer.record(96, Some(stamp));
        consumer.record(96, Some(stamp + 1000));
        delivery.packet(96, Some(stamp - 1));
        delivery.callback_burst(96);
    }
    let mut observation = NativeProcessDeliveryObservation {
        queue_history: None,
        packet_history: None,
        node,
        process: process.clone(),
        owner_generation: 7,
        input_sample_rate: 48_000,
        consumer_sample_rate: 48_000,
        prepared_consumer_demand_frames: 96,
        delivery: Some(delivery.snapshot()),
        admitted_bridge: crate::process_loopback::ProcessBridgeOptions {
            max_demand_frames: 96,
            ..Default::default()
        },
        measured_source_generation: None,
        producer: producer_observer.snapshot(),
        consumer: consumer_observer.snapshot(),
        counters: BridgeSnapshot {
            native_delivery: Some(delivery.snapshot()),
            ..Default::default()
        },
    };
    let measured = observation.measured(&process, 7).unwrap();
    assert!(measured.process().same_process(&process));
    assert_eq!(measured.owner_generation(), 7);
    assert_eq!(
        observation.measured(&process, 8).unwrap_err(),
        Error::GenerationMismatch
    );
    let mut stale = process.clone();
    stale.creation_time_100ns += 1;
    assert_eq!(
        observation.measured(&stale, 7).unwrap_err(),
        Error::IdentityMismatch
    );
    observation.consumer = None;
    assert_eq!(
        observation.measured(&process, 7).unwrap_err(),
        Error::MissingObservation {
            observation: "consumer"
        }
    );
}

#[test]
fn ordered_replay_projection_requires_exact_process_generation_and_closed_histories() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Source, 2).unwrap();
    let process = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    };
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    let observations = NativeObservations::default();
    observations.publish_with_process(
        7,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![ProcessDeliverySource {
            node,
            process: process.clone(),
            input_sample_rate: 48_000,
            consumer_sample_rate: 48_000,
            prepared_consumer_demand_frames: 4,
            admitted_bridge: crate::process_loopback::ProcessBridgeOptions {
                max_demand_frames: 4,
                ..Default::default()
            },
            measured_source_generation: None,
            bridge: observer,
            consumer: None,
        }],
    );
    let retained = observations.retain_session().unwrap();
    for t in [1000, 2000] {
        ingress
            .push_packet_with_publication_clock(
                &[],
                crate::clock_bridge::CapturePacket {
                    frames: 4,
                    flags: crate::stats::SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(t),
            )
            .unwrap();
        source
            .read_interleaved_with_publication_clock(&mut [0.0; 4], || Some(t + 100))
            .unwrap();
    }
    observations.clear(7);
    assert!(retained.ordered_process_trace(node, &process, 7).is_err());
    let still_live = retained.snapshot();
    assert!(
        still_live.process_delivery[0]
            .ordered_trace(&process, 7)
            .is_err(),
        "registry retirement is not writer closure"
    );
    drop(ingress);
    drop(source);
    let closed = retained.snapshot();
    assert!(
        closed.process_delivery[0]
            .ordered_trace(&process, 7)
            .is_ok()
    );
    assert!(
        closed.process_delivery[0]
            .ordered_trace(&process, 8)
            .is_err()
    );
    let mut wrong = process.clone();
    wrong.creation_time_100ns += 1;
    assert!(closed.process_delivery[0].ordered_trace(&wrong, 7).is_err());
    assert!(retained.ordered_process_trace(node, &process, 8).is_err());
    assert!(retained.ordered_process_trace(node, &wrong, 7).is_err());
    let missing = graph.create_node(NodeKind::Source, 2).unwrap();
    assert!(
        retained
            .ordered_process_trace(missing, &process, 7)
            .is_err()
    );
    let options = crate::process_loopback::ProcessBridgeOptions {
        max_demand_frames: 4,
        ..Default::default()
    };
    // Public DTO labels can be edited. Structural checks must never promote
    // them to the provenance of the private retained native owner.
    let mut edited = closed.process_delivery[0].clone();
    edited.process = wrong.clone();
    edited.owner_generation = 8;
    let edited_report = edited
        .ordered_trace(&wrong, 8)
        .unwrap()
        .replay(&wrong, 8, options)
        .unwrap();
    assert_eq!(
        edited_report.trace_origin,
        crate::process_loopback::ProcessTraceOrigin::CallerSupplied
    );
    observations.publish(8, Vec::new(), Vec::new(), Vec::new());
    let native_report = retained
        .ordered_process_trace(node, &process, 7)
        .unwrap()
        .replay(&process, 7, options)
        .unwrap();
    assert_eq!(native_report.owner_generation, 7);
    assert_eq!(
        native_report.trace_origin,
        crate::process_loopback::ProcessTraceOrigin::RetainedNativeOwner
    );
}
