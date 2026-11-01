use super::super::*;
use crate::{
    clock_bridge::{CaptureIngress, CapturePacket, ClockBridgeConfig, ClockSource, capture_bridge},
    process_loopback::{ProcessBridgeOptions, ProcessIdentity},
};
use moiren_core::graph::{LogicalGraph, NodeKind};

struct Fixture {
    registry: NativeObservations,
    node: NodeId,
    process: ProcessIdentity,
    ingress: CaptureIngress,
    source: Option<ClockSource>,
}

impl Fixture {
    fn new() -> Self {
        let node = LogicalGraph::new()
            .create_node(NodeKind::Source, 2)
            .unwrap();
        let process = ProcessIdentity {
            pid: 42,
            creation_time_100ns: 123,
            executable_name: "pinned.exe".into(),
        };
        let (ingress, source, bridge) = capture_bridge(ClockBridgeConfig {
            capacity_frames: 64,
            target_fill_frames: 4,
            trim_on_prime: false,
            max_correction_ppm: 0.0,
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
            source: Some(source),
        }
    }

    fn read(&mut self) {
        self.ingress
            .push_packet(
                &[0; 64],
                CapturePacket {
                    frames: 8,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
            )
            .unwrap();
        self.source
            .as_mut()
            .unwrap()
            .read_interleaved(&mut [0.0; 16])
            .unwrap();
    }
}

#[test]
fn interpolation_pin_rejects_wrong_generation_or_process() {
    let f = Fixture::new();
    let handle = f.registry.retain_session().unwrap();
    assert!(
        handle
            .process_interpolation_collector(f.node, &f.process, 8, 256)
            .is_err()
    );
    let mut other = f.process.clone();
    other.creation_time_100ns += 1;
    assert!(
        handle
            .process_interpolation_collector(f.node, &other, 7, 256)
            .is_err()
    );
    assert!(
        handle
            .process_interpolation_collector(f.node, &f.process, 7, 0)
            .is_err()
    );
}

#[test]
fn interpolation_owner_identity_distinguishes_same_generation_replacements() {
    let f = Fixture::new();
    let original = f.registry.retain_session().unwrap();
    assert!(original.same_owner(&f.registry.retain_session().unwrap()));
    // Generations are local to one backend; replacing it can reuse 1 (or 7).
    f.registry.publish(7, vec![], vec![], vec![]);
    let replacement = f.registry.retain_session().unwrap();
    assert!(!original.same_owner(&replacement));
    let independent = Fixture::new();
    assert!(!original.same_owner(&independent.registry.retain_session().unwrap()));
}

#[test]
fn interpolation_pin_retains_late_original_events_after_registry_replacement() {
    let mut f = Fixture::new();
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_interpolation_collector(f.node, &f.process, 7, 256)
        .unwrap();
    f.read();
    assert!(collector.poll());
    let first = collector.drain_chunk().unwrap();
    f.registry.clear(7);
    f.registry.publish(8, vec![], vec![], vec![]);
    f.read();
    assert!(collector.poll());
    let second = collector.drain_chunk().unwrap();
    assert_eq!(
        second.events[0].sequence,
        first.events.last().unwrap().sequence + 1
    );
    assert_eq!(collector.origin().owner_generation, 7);
    assert!(collector.origin().process.same_process(&f.process));
    assert_eq!(f.registry.snapshot().unwrap().owner_generation, 8);
    assert!(!collector.finish(), "open writer cannot finalize the pin");
    assert!(!collector.artifact().finalized);
    drop(f.source.take());
    assert!(collector.finish());
    assert!(
        collector.artifact().history_complete,
        "{:?}",
        collector.artifact()
    );
    assert!(!collector.artifact().qualified);
    assert_eq!(collector.artifact().started_reads, 2);
    assert_eq!(collector.artifact().requested_frames, 16);
    assert_eq!(
        collector.artifact().drained_events,
        collector.artifact().total_events
    );
}
