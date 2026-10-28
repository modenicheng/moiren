use super::*;
use crate::clock_bridge::{ClockBridgeConfig, capture_bridge};
#[cfg(windows)]
use crate::{clock_bridge::QueuedFrameStamp, engine_backend::NativeObservations};
#[cfg(windows)]
use moiren_core::graph::{LogicalGraph, NodeKind};

#[test]
fn native_drain_events_remain_separate_from_graph_packets_and_survive_writer_drop() {
    let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    ingress
        .push_packet(
            &[],
            crate::clock_bridge::CapturePacket {
                frames: 64,
                flags: crate::stats::SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
        )
        .unwrap();
    assert!(observer.delivery_events().unwrap().events.is_empty());
    ingress.record_native_delivery_event(64, Some(100));
    ingress.record_native_delivery_event(128, Some(200));
    drop(ingress);
    drop(source);
    let snapshot = observer.delivery_events().unwrap();
    assert_eq!(snapshot.total_events, 2);
    assert_eq!(snapshot.events[0].frames, 64);
    assert_eq!(snapshot.events[1].frames, 128);
    assert_eq!(snapshot.events[1].qpc_100ns, Some(200));
}

#[cfg(windows)]
#[test]
fn native_projection_replacement_never_receives_old_owner_diagnostics() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Source, 2).unwrap();
    let (mut old, _old_source, old_observer) =
        capture_bridge(ClockBridgeConfig::default()).unwrap();
    let (mut fresh, _source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    let observations = NativeObservations::default();
    observations.publish(7, Vec::new(), vec![(node, old_observer)], Vec::new());
    let lease = observations.lease(7);
    observations.publish(8, Vec::new(), vec![(node, observer.clone())], Vec::new());
    old.publish_native_delivery(InputDeliverySnapshot {
        packets: u64::MAX,
        ..Default::default()
    });
    drop(lease);
    observations.running(7);
    let snapshot = observations.snapshot().unwrap();
    assert_eq!(snapshot.owner_generation, 8);
    assert!(!snapshot.running);
    assert_eq!(snapshot.bridges[0].counters.native_delivery, None);
    let delivery = InputDeliverySnapshot {
        packets: 13,
        gap_bound_violations: 5,
        ..Default::default()
    };
    fresh.publish_native_delivery(delivery);
    let stamp = QueuedFrameStamp {
        generation: 4,
        packet_position_frames: 48,
        packet_qpc_100ns: 10_000,
        packet_accepted_frames: 8,
        packet_offset_frames: 0,
        rate_correction_ppm: Some(-1000),
    };
    let age = stamp.estimate_age(48_000, 20_000).unwrap();
    observer.0.oldest_frame_age.publish(Some(age));
    let snapshot = observations.snapshot().unwrap();
    assert_eq!(snapshot.bridges[0].counters.native_delivery, Some(delivery));
    assert_eq!(snapshot.bridges[0].counters.oldest_frame_age, Some(age));
    let json = serde_json::to_value(snapshot).unwrap();
    assert_eq!(
        json["bridges"][0]["counters"]["oldest_frame_age"]["stamp"]["rate_correction_ppm"],
        -1000
    );
}

#[test]
fn delivery_retains_every_counter_without_u64_truncation() {
    let sample = InputDeliverySnapshot {
        packets: u64::MAX,
        max_packet_frames: u32::MAX,
        max_callback_burst_frames: u64::MAX - 1,
        gap_samples: u64::MAX - 2,
        max_gap_100ns: u64::MAX - 3,
        packet_bound_violations: u64::MAX - 4,
        gap_bound_violations: u64::MAX - 5,
        unavailable_clocks: u64::MAX - 6,
        regressed_clocks: u64::MAX - 7,
    };
    let (mut ingress, _source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    assert_eq!(observer.snapshot().native_delivery, None);
    ingress.publish_native_delivery(sample);
    assert_eq!(observer.snapshot().native_delivery, Some(sample));
    ingress.publish_native_delivery(InputDeliverySnapshot::default());
    assert_eq!(
        observer.snapshot().native_delivery,
        Some(InputDeliverySnapshot::default())
    );
}
