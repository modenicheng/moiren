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

fn live_options() -> ProcessBridgeOptions {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 64,
            target_frames: 12,
            jitter_frames: 0,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 4,
            max_gap_frames: 4,
            source: DeliveryTimingSource::Empirical,
        }),
        max_demand_frames: 4,
        ..Default::default()
    }
}

fn live_policy() -> crate::process_loopback::ReplayWindowPolicy {
    crate::process_loopback::ReplayWindowPolicy {
        max_initial_unserved_frames: 1000,
        max_initial_prime_discarded_frames: 1000,
        min_output_frames: 4,
        min_output_reads: 2,
        min_output_span_100ns: 1,
    }
}

#[test]
fn pinned_live_prefix_keeps_original_identity_after_registry_replacement_without_closure() {
    let mut f = Fixture::new();
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    for n in 0..24 {
        f.event(n);
    }
    f.registry.clear(7);
    f.registry.publish(8, vec![], vec![], vec![]);
    f.event(24);
    let report = collector
        .replay_live_window(live_options(), live_policy())
        .unwrap();
    assert_eq!(
        report.status,
        crate::process_loopback::ReplayWindowStatus::NoViolationInWindow,
        "{report:?}"
    );
    assert_eq!(report.trace_origin, ProcessTraceOrigin::RetainedNativeOwner);
    let prefix = report.prefix.unwrap();
    assert_eq!(prefix.observed_packet_events, 25);
    assert!(!prefix.packet_writer_finished && !prefix.read_writer_finished);
    let replay = report.replay.unwrap();
    assert_eq!(replay.report.owner_generation, 7);
    assert_eq!(
        replay.report.trace_origin,
        ProcessTraceOrigin::RetainedNativeOwner
    );
    assert!(replay.report.process.same_process(&f.process));
    assert!(collector.ordered_trace().is_err());
    let imported = collector
        .artifact()
        .replay_live_window(&f.process, 7, live_options(), live_policy())
        .unwrap();
    assert_eq!(imported.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert_eq!(f.registry.snapshot().unwrap().owner_generation, 8);
}

#[test]
fn pinned_live_prefix_cannot_drop_original_ingress_elapsed_intent() {
    let mut f = Fixture::new();
    let retained = f.registry.retain_session().unwrap();
    let mut source = retained.0.process_delivery[0].clone();
    source.admitted_bridge.max_ingress_elapsed_100ns = Some(100);
    f.registry
        .publish_with_process(7, vec![], vec![], vec![], vec![], vec![source]);
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    for n in 0..4 {
        f.event(n);
    }
    let result = collector.replay_live_window(live_options(), live_policy());
    assert!(
        matches!(result, Err(ProcessReplayError::IngressElapsedUnsupported)),
        "{result:?}"
    );
}

#[test]
fn pinned_live_prefix_stays_immutable_when_only_the_unsealed_consumer_tail_advances() {
    let mut f = Fixture::new();
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    for n in 0..24 {
        f.event(n);
    }
    let first = collector
        .replay_live_window(live_options(), live_policy())
        .unwrap();
    for stamp in [5000, 5100, 5200] {
        f.source
            .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(stamp))
            .unwrap();
    }
    let later = collector
        .replay_live_window(live_options(), live_policy())
        .unwrap();
    assert_eq!(
        first.prefix.as_ref().unwrap().cutoff_100ns,
        later.prefix.as_ref().unwrap().cutoff_100ns
    );
    assert_eq!(
        later.prefix.as_ref().unwrap().observed_read_events,
        first.prefix.as_ref().unwrap().observed_read_events + 3
    );
    assert_eq!(
        serde_json::to_value(first.replay).unwrap(),
        serde_json::to_value(later.replay).unwrap()
    );
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

#[test]
fn pinned_final_measurement_keeps_original_whole_wake_and_late_delivery_after_registry_replacement()
{
    use crate::latency::{
        InputDeliveryRecorder, events::event_history, transport::TransportInputPolicy,
    };
    let mut f = Fixture::new();
    let handle = f.registry.retain_session().unwrap();
    let mut source = handle.0.process_delivery[0].clone();
    let (mut consumer, consumer_observer) = event_history();
    source.consumer = Some(consumer_observer);
    source.prepared_consumer_demand_frames = 64;
    f.registry
        .publish_with_process(7, vec![], vec![], vec![], vec![], vec![source]);
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    let mut delivery = InputDeliveryRecorder::new(None).unwrap();
    for n in 0..4 {
        f.event(n);
        let stamp = 1000 + n * 100;
        delivery.packet(4, Some(stamp));
        delivery.callback_burst(4);
        f.ingress.record_native_delivery_event(4, Some(stamp));
        f.ingress.publish_native_delivery(delivery.snapshot());
        consumer.record(64, Some(stamp + 10));
    }
    assert!(
        collector.final_measurement().is_err(),
        "live trace is not final evidence"
    );
    f.registry.clear(7);
    f.registry.publish(8, vec![], vec![], vec![]);
    f.event(4);
    delivery.packet(4, Some(1400));
    delivery.callback_burst(4);
    f.ingress.record_native_delivery_event(4, Some(1400));
    f.ingress.publish_native_delivery(delivery.snapshot());
    consumer.record(128, Some(1410));
    drop(f.ingress);
    drop(f.source);
    drop(consumer);
    collector.poll();
    let measured = collector.final_measurement().unwrap();
    let options = measured
        .admit_for_consumer(
            &f.process,
            7,
            TransportInputPolicy::Target {
                capacity_frames: 2048,
                target_frames: 256,
                jitter_frames: 0,
            },
            48_000,
            1,
        )
        .unwrap();
    assert_eq!(
        options.max_demand_frames, 128,
        "late whole-wake demand survives smaller caller estimate"
    );
    assert_eq!(measured.owner_generation(), 7);
    assert_eq!(f.registry.snapshot().unwrap().owner_generation, 8);
    assert_eq!(collector.artifact().packet_history.total_events, 5);
}

#[test]
fn pinned_final_measurement_refuses_missing_whole_wake_observer() {
    let mut f = Fixture::new();
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    f.event(0);
    f.event(1);
    drop(f.ingress);
    drop(f.source);
    collector.poll();
    assert!(collector.ordered_trace().is_ok());
    assert!(
        collector.final_measurement().is_err(),
        "queue segments must not fabricate whole-wake evidence"
    );
}

#[test]
fn ingress_elapsed_pinned_measurement_refuses_without_erasing_closed_raw_archive() {
    use crate::latency::InputDeliveryRecorder;
    for limit in [None, Some(1), Some(u64::MAX)] {
        let mut f = Fixture::new();
        let mut metadata = f.registry.retain_session().unwrap().0.process_delivery[0].clone();
        let (mut consumer, observer) = crate::latency::events::event_history();
        metadata.consumer = Some(observer);
        metadata.admitted_bridge.max_ingress_elapsed_100ns = limit;
        f.registry
            .publish_with_process(7, vec![], vec![], vec![], vec![], vec![metadata]);
        let mut collector = f
            .registry
            .retain_session()
            .unwrap()
            .process_trace_collector(f.node, &f.process, 7)
            .unwrap();
        let mut delivery = InputDeliveryRecorder::new(None).unwrap();
        for n in 0..4 {
            f.event(n);
            let stamp = 1000 + n * 100;
            delivery.packet(4, Some(stamp + 1));
            delivery.callback_burst(4);
            f.ingress.record_native_delivery_event(4, Some(stamp + 2));
            f.ingress.publish_native_delivery(delivery.snapshot());
            consumer.record(4, Some(stamp + 10));
        }
        f.registry.clear(7);
        f.registry.publish(8, vec![], vec![], vec![]);
        drop(f.ingress);
        drop(f.source);
        drop(consumer);
        collector.poll();
        let raw = collector.artifact();
        assert_eq!(raw.packet_history.total_events, 4);
        assert!(raw.packet_history.writer_finished && raw.queue_history.writer_finished);
        assert!(collector.ordered_trace().is_ok());
        let measured = collector.final_measurement();
        if limit.is_some() {
            assert!(measured.unwrap_err().contains("ingress"));
        } else {
            assert!(measured.is_ok());
        }
        assert_eq!(f.registry.snapshot().unwrap().owner_generation, 8);
    }
}

#[test]
fn pinned_final_measurement_replays_exact_empirical_options_with_retained_origin() {
    use crate::{
        latency::{
            InputDeliveryRecorder,
            events::event_history,
            transport::{DeliveryTimingSource, TransportInputPolicy},
        },
        process_loopback::{ReplayWindowPolicy, ReplayWindowStatus},
    };
    let f = Fixture::new();
    let (mut ingress, mut source, bridge) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 2048,
        target_fill_frames: 128,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    let (mut consumer, observer) = event_history();
    let mut metadata = f.registry.retain_session().unwrap().0.process_delivery[0].clone();
    metadata.bridge = bridge;
    metadata.consumer = Some(observer);
    metadata.prepared_consumer_demand_frames = 96;
    f.registry
        .publish_with_process(7, vec![], vec![], vec![], vec![], vec![metadata]);
    let mut collector = f
        .registry
        .retain_session()
        .unwrap()
        .process_trace_collector(f.node, &f.process, 7)
        .unwrap();
    let mut delivery = InputDeliveryRecorder::new(None).unwrap();
    for n in 0..8 {
        let stamp = 1_000_000 + n * 20_000;
        let mut publication = stamp;
        ingress
            .push_packet_with_publication_clock(
                &[],
                crate::clock_bridge::CapturePacket {
                    frames: 96,
                    flags: crate::stats::SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || {
                    let now = publication;
                    publication += 1;
                    Some(now)
                },
            )
            .unwrap();
        delivery.packet(96, Some(stamp + 20));
        delivery.callback_burst(96);
        ingress.record_native_delivery_event(96, Some(stamp + 100));
        ingress.publish_native_delivery(delivery.snapshot());
        consumer.record(96, Some(stamp + 1000));
        source
            .read_interleaved_with_publication_clock(&mut [0.; 192], || Some(stamp + 1000))
            .unwrap();
    }
    drop(ingress);
    drop(source);
    drop(consumer);
    collector.poll();
    let options = collector
        .final_measurement()
        .unwrap()
        .admit_for_consumer(
            &f.process,
            7,
            TransportInputPolicy::Target {
                capacity_frames: 2048,
                target_frames: 256,
                jitter_frames: 0,
            },
            48_000,
            96,
        )
        .unwrap();
    let policy = ReplayWindowPolicy {
        max_initial_unserved_frames: 4096,
        max_initial_prime_discarded_frames: 4096,
        min_output_frames: 192,
        min_output_reads: 2,
        min_output_span_100ns: 10_000,
    };
    let result = collector
        .ordered_trace()
        .unwrap()
        .replay_window(&f.process, 7, options, policy)
        .unwrap();
    assert_eq!(result.report.options, options);
    assert_eq!(options.delivery.unwrap().max_gap_frames, 97);
    assert_eq!(
        options.delivery.unwrap().source,
        DeliveryTimingSource::Empirical
    );
    assert_eq!(
        result.report.trace_origin,
        ProcessTraceOrigin::RetainedNativeOwner
    );
    assert_eq!(
        result.assessment.status,
        ReplayWindowStatus::NoViolationInWindow
    );
    assert!(
        !result.report.qualified,
        "observation-owner provenance is not hardware qualification"
    );
}

struct WindowFixture {
    f: Fixture,
    consumer: crate::latency::events::EventHistoryWriter,
    delivery: crate::latency::InputDeliveryRecorder,
}

impl WindowFixture {
    fn new() -> Self {
        let mut f = Fixture::new();
        let (ingress, source, bridge) = capture_bridge(ClockBridgeConfig {
            capacity_frames: 2048,
            target_fill_frames: 128,
            detect_position_gaps: false,
            ..Default::default()
        })
        .unwrap();
        f.ingress = ingress;
        f.source = source;
        let (consumer, observer) = crate::latency::events::event_history();
        let mut metadata = f.registry.retain_session().unwrap().0.process_delivery[0].clone();
        metadata.bridge = bridge;
        metadata.consumer = Some(observer);
        metadata.prepared_consumer_demand_frames = 96;
        f.registry
            .publish_with_process(7, vec![], vec![], vec![], vec![], vec![metadata]);
        Self {
            f,
            consumer,
            delivery: crate::latency::InputDeliveryRecorder::new(None).unwrap(),
        }
    }

    fn collector(&self) -> NativeProcessTraceCollector {
        self.f
            .registry
            .retain_session()
            .unwrap()
            .process_trace_collector(self.f.node, &self.f.process, 7)
            .unwrap()
    }

    fn packet(&mut self, n: u64) {
        self.packet_with_bracket(n, 1);
    }

    fn packet_with_bracket(&mut self, n: u64, width_100ns: u64) {
        let stamp = 1_000_000 + n * 20_000;
        let mut publication = stamp;
        self.f
            .ingress
            .push_packet_with_publication_clock(
                &[],
                crate::clock_bridge::CapturePacket {
                    frames: 96,
                    flags: crate::stats::SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || {
                    let now = publication;
                    publication += width_100ns;
                    Some(now)
                },
            )
            .unwrap();
    }

    fn complete_delivery(&mut self, n: u64) {
        let stamp = 1_000_000 + n * 20_000;
        self.delivery.packet(96, Some(stamp + 20));
        self.delivery.callback_burst(96);
        self.f
            .ingress
            .record_native_delivery_event(96, Some(stamp + 100));
        self.f
            .ingress
            .publish_native_delivery(self.delivery.snapshot());
    }

    fn read(&mut self, n: u64) {
        let stamp = 1_001_000 + n * 20_000;
        // Two graph segments still represent one complete native wake.
        self.f
            .source
            .read_interleaved_with_publication_clock(&mut [0.; 96], || Some(stamp))
            .unwrap();
        self.f
            .source
            .read_interleaved_with_publication_clock(&mut [0.; 96], || Some(stamp + 1))
            .unwrap();
    }

    fn event(&mut self, n: u64) {
        self.packet(n);
        self.complete_delivery(n);
        self.read(n);
        self.consumer.record(96, Some(1_001_000 + n * 20_000));
    }

    fn populate(&mut self) {
        for n in 0..12 {
            self.event(n);
        }
    }
}

fn window_input() -> crate::latency::transport::TransportInputPolicy {
    crate::latency::transport::TransportInputPolicy::Target {
        capacity_frames: 2048,
        target_frames: 256,
        jitter_frames: 0,
    }
}

#[test]
fn prepared_native_window_pins_source_and_freezes_evidence_before_worker_evaluation() {
    let mut f = WindowFixture::new();
    let mut collector = f.collector();
    f.populate();
    f.f.registry.clear(7);
    f.f.registry.publish(8, vec![], vec![], vec![]);
    let prepared = collector
        .prepare_live_window(window_input(), live_policy())
        .unwrap();
    // In-flight malformed later data must not be sampled by the worker.
    f.packet(12);
    f.read(12);
    let candidate = std::thread::spawn(move || prepared.evaluate())
        .join()
        .unwrap()
        .unwrap();
    let report = candidate.report();
    assert_eq!(report.trace_origin, ProcessTraceOrigin::RetainedNativeOwner);
    assert_eq!(report.owner_generation, 7);
    assert!(report.process.same_process(&f.f.process));
    assert!(!report.qualified);
    assert_eq!(
        report.status,
        crate::process_loopback::ReplayWindowStatus::NoViolationInWindow
    );
    assert_eq!(report.prefix.as_ref().unwrap().observed_packet_events, 12);
    assert!(!report.prefix.as_ref().unwrap().packet_writer_finished);
    assert!(!report.prefix.as_ref().unwrap().read_writer_finished);
    let options = candidate.options();
    assert_eq!(options, report.options);
    assert_eq!(options, report.replay.as_ref().unwrap().report.options);
    assert_eq!(
        options.max_demand_frames, 96,
        "whole wake, not 48-frame segments"
    );
    assert_eq!(options.delivery.unwrap().max_gap_frames, 97);
    assert_eq!(
        options,
        candidate
            .measurement()
            .admit_for_consumer(&f.f.process, 7, window_input(), 48_000, 96)
            .unwrap()
    );
    assert_eq!(candidate.clone().measurement().owner_generation(), 7);
    assert!(
        collector.final_measurement().is_err(),
        "closure is still required"
    );
}

#[test]
fn prepared_native_window_refuses_original_ingress_intent() {
    let mut f = WindowFixture::new();
    let mut metadata = f.f.registry.retain_session().unwrap().0.process_delivery[0].clone();
    metadata.admitted_bridge.max_ingress_elapsed_100ns = Some(1);
    f.f.registry
        .publish_with_process(7, vec![], vec![], vec![], vec![], vec![metadata]);
    let mut collector = f.collector();
    f.populate();
    let error = collector
        .prepare_live_window(window_input(), live_policy())
        .err()
        .expect("original freshness intent must refuse replacement");
    assert!(error.contains("ingress"), "{error}");
}

#[test]
fn prepared_native_window_refuses_missing_delivery_or_whole_wake_evidence() {
    for missing_consumer in [false, true] {
        let mut f = WindowFixture::new();
        if missing_consumer {
            let mut metadata = f.f.registry.retain_session().unwrap().0.process_delivery[0].clone();
            metadata.consumer = None;
            f.f.registry
                .publish_with_process(7, vec![], vec![], vec![], vec![], vec![metadata]);
        }
        let mut collector = f.collector();
        for n in 0..12 {
            f.packet(n);
            if missing_consumer {
                f.complete_delivery(n);
            }
            f.read(n);
            f.consumer.record(96, Some(1_001_000 + n * 20_000));
        }
        let error = collector
            .prepare_live_window(window_input(), live_policy())
            .err()
            .expect("queue segments cannot replace native measurement");
        let expected = if missing_consumer {
            "whole-wake"
        } else {
            "delivery"
        };
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn prepared_native_window_refuses_target_that_only_fits_graph_segments() {
    let mut f = WindowFixture::new();
    let mut collector = f.collector();
    f.populate();
    let input = crate::latency::transport::TransportInputPolicy::Target {
        capacity_frames: 2048,
        target_frames: 160,
        jitter_frames: 0,
    };
    assert!(
        collector.prepare_live_window(input, live_policy()).is_err(),
        "160 fits a 48-frame segment plus 97-frame gap, but not a 96-frame wake"
    );
}

#[test]
fn prepared_native_window_requires_full_archive_delivery_and_whole_wake_coverage() {
    for missing_delivery in [false, true] {
        let mut f = WindowFixture::new();
        let mut collector = f.collector();
        f.populate();
        f.packet(12);
        if !missing_delivery {
            f.complete_delivery(12);
        }
        f.read(12);
        if missing_delivery {
            f.consumer.record(96, Some(1_241_000));
        }
        let error = collector
            .prepare_live_window(window_input(), live_policy())
            .err()
            .expect("an incomplete tail must not understate admission evidence");
        assert!(error.contains("cover"), "{error}");
    }
}

#[test]
fn prepared_native_window_refuses_violation_and_inconclusive_actual_replays() {
    for inconclusive in [false, true] {
        let mut f = WindowFixture::new();
        let mut collector = f.collector();
        f.populate();
        let mut policy = live_policy();
        if inconclusive {
            policy.min_output_frames = u64::MAX;
        } else {
            policy.max_initial_unserved_frames = 0;
        }
        let prepared = collector
            .prepare_live_window(window_input(), policy)
            .unwrap();
        let error = prepared.evaluate().unwrap_err();
        let expected = if inconclusive {
            "Inconclusive"
        } else {
            "ObservedViolation"
        };
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn prepared_native_window_refuses_either_measured_history_overwrite_below_archive_limit() {
    for overwrite_producer in [false, true] {
        let mut f = WindowFixture::new();
        let mut collector = f.collector();
        for n in 0..257 {
            if overwrite_producer || n < 12 {
                f.packet(n);
                f.complete_delivery(n);
            }
            if !overwrite_producer || n % 2 == 0 {
                f.read(n);
                f.consumer.record(96, Some(1_001_000 + n * 20_000));
            }
            if n.is_multiple_of(32) {
                collector.poll();
            }
        }
        let error = collector
            .prepare_live_window(window_input(), live_policy())
            .err()
            .expect("suffix evidence must not cover an 8192-event archive");
        assert!(
            error.contains("overwrite") && error.contains("256"),
            "{error}"
        );
        let artifact = collector.artifact();
        assert_eq!(artifact.collection_refusal, None);
        assert!(artifact.packet_history.events.len() < 8192);
        assert!(artifact.queue_history.events.len() < 8192);
    }
}

#[test]
fn prepared_native_window_widens_gap_using_unsealed_full_artifact_tail() {
    let mut f = WindowFixture::new();
    let mut collector = f.collector();
    for n in 0..11 {
        f.event(n);
    }
    f.packet_with_bracket(11, 501);
    f.complete_delivery(11);
    f.read(11);
    f.consumer.record(96, Some(1_221_000));
    let candidate = collector
        .prepare_live_window(window_input(), live_policy())
        .unwrap()
        .evaluate()
        .unwrap();
    assert_eq!(candidate.options().delivery.unwrap().max_gap_frames, 99);
    let prefix = candidate.report().prefix.as_ref().unwrap();
    assert_eq!(prefix.observed_packet_events, 12);
    assert_eq!(prefix.replayed_packet_events, 11);
}
