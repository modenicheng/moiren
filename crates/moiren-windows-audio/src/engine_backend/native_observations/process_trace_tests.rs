use super::*;
use crate::{
    clock_bridge::{CaptureIngress, ClockBridgeConfig, ClockSource, capture_bridge},
    process_loopback::{
        ProcessBridgeOptions, ProcessIdentity, ProcessReplayError, ProcessTraceCollectionRefusal,
        ProcessTraceOrigin,
    },
};
use moiren_core::graph::{LogicalGraph, NodeKind};

struct Fixture {
    registry: NativeObservations,
    node: NodeId,
    process: ProcessIdentity,
    ingress: CaptureIngress,
    source: ClockSource,
}

impl Fixture {
    fn new() -> Self {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::Source, 2).unwrap();
        let process = ProcessIdentity {
            pid: 42,
            creation_time_100ns: 123,
            executable_name: "pinned.exe".into(),
        };
        let (ingress, source, bridge) = capture_bridge(ClockBridgeConfig {
            capacity_frames: 32,
            target_fill_frames: 4,
            detect_position_gaps: false,
            ..Default::default()
        })
        .unwrap();
        let registry = NativeObservations::default();
        registry.publish_with_process(
            7,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![ProcessDeliverySource {
                node,
                process: process.clone(),
                input_sample_rate: 48_000,
                consumer_sample_rate: 48_000,
                prepared_consumer_demand_frames: 4,
                admitted_bridge: ProcessBridgeOptions {
                    max_demand_frames: 4,
                    ..Default::default()
                },
                measured_source_generation: None,
                bridge,
                consumer: None,
            }],
        );
        Self {
            registry,
            node,
            process,
            ingress,
            source,
        }
    }

    fn event(&mut self, n: u64) {
        let stamp = 1000 + n * 100;
        self.ingress
            .push_packet_with_publication_clock(
                &[],
                crate::clock_bridge::CapturePacket {
                    frames: 4,
                    flags: crate::stats::SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(stamp),
            )
            .unwrap();
        self.source
            .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(stamp + 10))
            .unwrap();
    }
}

#[test]
fn pinned_archive_survives_registry_clear_and_late_writes_beyond_native_suffix() {
    let mut f = Fixture::new();
    let handle = f.registry.retain_session().unwrap();
    let mut collector = handle
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    for n in 0..600 {
        f.event(n);
        if n.is_multiple_of(64) {
            collector.poll();
        }
    }
    f.registry.clear(7);
    f.registry.publish(8, vec![], vec![], vec![]);
    f.event(600);
    collector.poll();
    assert!(collector.ordered_trace().is_err(), "clear is not closure");
    drop(f.ingress);
    drop(f.source);
    collector.poll();
    let artifact = collector.artifact();
    assert_eq!(artifact.owner_generation, 7);
    assert_eq!(artifact.collection_refusal, None);
    assert_eq!(artifact.packet_history.events.len(), 601);
    assert_eq!(artifact.queue_history.events.len(), 601);
    assert!(artifact.packet_history.writer_finished && artifact.queue_history.writer_finished);
    assert_eq!(artifact.packet_history.events[0].sequence, 1);
    assert_eq!(artifact.packet_history.events[600].sequence, 601);
    let options = ProcessBridgeOptions {
        max_demand_frames: 4,
        ..Default::default()
    };
    let report = collector
        .ordered_trace()
        .unwrap()
        .replay(&f.process, 7, options)
        .unwrap();
    assert_eq!(report.trace_origin, ProcessTraceOrigin::RetainedNativeOwner);
    assert!(!report.qualified);
    let imported = artifact
        .trace(&f.process, 7)
        .unwrap()
        .replay(&f.process, 7, options)
        .unwrap();
    assert_eq!(imported.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert_eq!(f.registry.snapshot().unwrap().owner_generation, 8);
}

#[test]
fn pinned_archive_refuses_mismatched_origin_before_reading_and_never_restarts_a_missed_prefix() {
    let mut f = Fixture::new();
    let handle = f.registry.retain_session().unwrap();
    assert!(matches!(
        handle.process_trace_collector(f.node, &f.process, 8),
        Err(ProcessReplayError::GenerationMismatch)
    ));
    let mut wrong = f.process.clone();
    wrong.creation_time_100ns += 1;
    assert!(matches!(
        handle.process_trace_collector(f.node, &wrong, 7),
        Err(ProcessReplayError::IdentityMismatch)
    ));
    let mut graph = LogicalGraph::new();
    graph.create_node(NodeKind::Source, 2).unwrap();
    let missing = graph.create_node(NodeKind::Source, 2).unwrap();
    assert!(matches!(
        handle.process_trace_collector(missing, &f.process, 7),
        Err(ProcessReplayError::InvalidIdentity)
    ));
    for n in 0..300 {
        f.event(n);
    }
    let mut collector = handle
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    drop(f.ingress);
    drop(f.source);
    collector.poll();
    let artifact = collector.artifact();
    assert_eq!(
        artifact.collection_refusal,
        Some(ProcessTraceCollectionRefusal::MissedEvents)
    );
    assert_eq!(collector.collection_refusal(), artifact.collection_refusal);
    assert_eq!(collector.owner_generation(), 7);
    assert!(collector.process().same_process(&f.process));
    assert_eq!(artifact.packet_history.total_events, 300);
    assert!(artifact.packet_history.writer_finished && artifact.queue_history.writer_finished);
    assert!(matches!(
        collector.ordered_trace(),
        Err(ProcessReplayError::IncompleteHistory)
    ));
}

#[test]
fn pinned_archive_limit_is_diagnostic_and_preserves_final_writer_state() {
    let mut f = Fixture::new();
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    for n in 0..8320 {
        f.event(n);
        if n.is_multiple_of(64) {
            collector.poll();
        }
    }
    drop(f.ingress);
    drop(f.source);
    collector.poll();
    let artifact = collector.artifact();
    assert_eq!(
        artifact.collection_refusal,
        Some(ProcessTraceCollectionRefusal::EventLimit)
    );
    assert_eq!(collector.collection_refusal(), artifact.collection_refusal);
    assert!(artifact.packet_history.events.len() <= 8192);
    assert!(artifact.queue_history.events.len() <= 8192);
    assert_eq!(artifact.packet_history.total_events, 8320);
    assert!(artifact.packet_history.writer_finished && artifact.queue_history.writer_finished);
    assert!(collector.ordered_trace().is_err());
}

#[test]
fn pinned_archive_constructor_captures_current_prefix_before_the_next_control_poll() {
    let mut f = Fixture::new();
    for n in 0..200 {
        f.event(n);
    }
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    for n in 200..400 {
        f.event(n);
    }
    drop(f.ingress);
    drop(f.source);
    collector.poll();
    assert_eq!(collector.artifact().packet_history.events.len(), 400);
    assert_eq!(collector.artifact().collection_refusal, None);
    assert!(collector.ordered_trace().is_ok());
}
