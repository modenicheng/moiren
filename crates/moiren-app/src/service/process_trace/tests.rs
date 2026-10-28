//! Actual software bridge histories remain CallerSupplied, not native evidence.
use super::*;
use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    latency::transport::{DeliveryTimingSource, TransportInputPolicy},
    process_loopback::{
        ProcessTraceCollector, ProcessTraceOrigin, ReplayWindowPolicy, ReplayWindowStatus,
    },
};

pub(crate) fn evidence(node: NodeId) -> ProcessTraceSessionEvidence {
    evidence_with_count(node, 8)
}

fn evidence_with_count(node: NodeId, count: u64) -> ProcessTraceSessionEvidence {
    evidence_with_shape(node, count, 96, 2048)
}

fn evidence_with_shape(
    node: NodeId,
    count: u64,
    packet_frames: usize,
    capacity: usize,
) -> ProcessTraceSessionEvidence {
    let context = crate::service::ServiceContext::default();
    let process = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "software-window.exe".into(),
    };
    let (mut writer, mut reader, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: capacity,
        target_fill_frames: 128,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    for n in 0..count {
        let stamp = 1_000_000 + n * 20_000;
        writer
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: packet_frames,
                    flags: moiren_windows_audio::stats::SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(stamp),
            )
            .unwrap();
        reader
            .read_interleaved_with_publication_clock(&mut [0.; 192], || Some(stamp + 1000))
            .unwrap();
    }
    drop(writer);
    drop(reader);
    let mut collector = ProcessTraceCollector::new(process.clone(), 7, 48_000, 48_000, 96);
    collector.poll(&observer);
    let artifact = collector.artifact();
    let trace = artifact.trace(&process, 7).unwrap();
    ProcessTraceSessionEvidence {
        origin: ProcessTraceSessionOrigin {
            generation: SessionGeneration(3),
            document: 9,
            node,
            process,
            native_owner_generation: 7,
            host_revision: 5,
            graph_revision: 11,
            audio_settings: context.audio_settings,
            transport_latency: context.transport_latency,
            latency_profile: context.latency_profile,
        },
        artifact,
        refusal: None,
        trace: Some(trace),
    }
}

#[test]
fn process_window_offered_supply_including_dropped_tails_is_never_shrunk_to_accepted() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let evidence = evidence_with_shape(node, 8, 512, 256);
    assert!(
        evidence
            .artifact
            .packet_history
            .events
            .iter()
            .all(|e| e.sample.accepted_frames < e.sample.offered_frames)
    );
    let result = evidence
        .evaluate_candidate(4096, 1024, 0, policy())
        .unwrap();
    assert_eq!(
        result.report.options.delivery.unwrap().max_packet_frames,
        512
    );
    assert_eq!(result.report.baseline.offered_frames, 4096);
    assert!(result.report.baseline.dropped_frames > 0);
    assert_eq!(result.report.packet_events, 8);
    assert!(!result.report.qualified);
}

#[test]
fn process_window_outer_envelopes_round_up_and_keep_original_consumer_shape() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let mut evidence = evidence(node);
    for event in &mut evidence.artifact.packet_history.events {
        event.sample.qpc_after_100ns = event.sample.qpc_after_100ns.map(|stamp| stamp + 1);
    }
    evidence.artifact.consumer_sample_rate = 96_000;
    evidence.artifact.prepared_consumer_demand_frames = 1024;
    evidence.trace = Some(
        evidence
            .artifact
            .trace(&evidence.origin.process, 7)
            .unwrap(),
    );
    let result = evidence
        .evaluate_candidate(4096, 1024, 0, policy())
        .unwrap();
    assert_eq!(result.report.options.delivery.unwrap().max_gap_frames, 97);
    assert_eq!(result.report.options.engine_sample_rate, 96_000);
    assert_eq!(result.report.options.max_demand_frames, 1024);
    assert!(evidence.evaluate_candidate(4096, 512, 0, policy()).is_err());
    assert!(
        evidence
            .evaluate_candidate(262_145, 1024, 0, policy())
            .is_err()
    );
    let singleton = evidence_with_count(node, 1);
    assert!(
        singleton
            .evaluate_candidate(2048, 256, 0, policy())
            .unwrap_err()
            .contains("two packet")
    );
}

pub(crate) fn policy() -> ReplayWindowPolicy {
    ReplayWindowPolicy {
        max_initial_unserved_frames: 4096,
        max_initial_prime_discarded_frames: 4096,
        min_output_frames: 192,
        min_output_reads: 2,
        min_output_span_100ns: 10_000,
    }
}

#[test]
fn process_window_evaluates_exact_candidate_without_promoting_origin_or_admission() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let evidence = evidence(node);
    let result = evidence
        .evaluate_candidate(2048, 256, 32, policy())
        .unwrap();
    assert_eq!(
        result.report.options.input,
        TransportInputPolicy::Target {
            capacity_frames: 2048,
            target_frames: 256,
            jitter_frames: 32,
        }
    );
    assert_eq!(
        result.report.options.delivery.unwrap().source,
        DeliveryTimingSource::Declared
    );
    assert_eq!(
        result.report.options.delivery.unwrap().max_packet_frames,
        96
    );
    assert_eq!(result.report.options.delivery.unwrap().max_gap_frames, 96);
    assert_eq!(result.report.options.max_demand_frames, 96);
    assert_eq!(result.report.options.engine_sample_rate, 48_000);
    assert_eq!(result.report.options.age_limit_100ns, None);
    assert_eq!(
        result.report.trace_origin,
        ProcessTraceOrigin::CallerSupplied
    );
    assert!(!result.report.qualified);
    assert_eq!(
        result.assessment.status,
        ReplayWindowStatus::NoViolationInWindow
    );
    assert_eq!(result.assessment.policy, policy());
    assert_eq!(result.report.packet_events, 8);
    assert_eq!(result.report.read_events, 8);
}

#[test]
fn process_window_preserves_exposure_loss_and_hard_refusals() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::Source, 2)
        .unwrap();
    let mut evidence = evidence(node);
    let result = evidence
        .evaluate_candidate(
            2048,
            256,
            0,
            ReplayWindowPolicy {
                min_output_frames: 10_000,
                ..policy()
            },
        )
        .unwrap();
    assert_eq!(result.assessment.status, ReplayWindowStatus::Inconclusive);
    let result = evidence
        .evaluate_candidate(
            2048,
            256,
            0,
            ReplayWindowPolicy {
                max_initial_unserved_frames: 0,
                ..policy()
            },
        )
        .unwrap();
    assert_eq!(
        result.assessment.status,
        ReplayWindowStatus::ObservedViolation
    );
    assert!(
        evidence.evaluate_candidate(2048, 128, 0, policy()).is_err(),
        "whole prepared wake cannot be reduced to one segment"
    );
    assert!(
        evidence
            .evaluate_candidate(
                2048,
                256,
                0,
                ReplayWindowPolicy {
                    min_output_reads: 1,
                    ..policy()
                }
            )
            .is_err()
    );
    evidence.refusal = Some("cleanup failed".into());
    assert_eq!(
        evidence
            .evaluate_candidate(2048, 256, 0, policy())
            .unwrap_err(),
        "cleanup failed"
    );
}
