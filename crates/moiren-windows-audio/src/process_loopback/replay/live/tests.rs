use super::*;
use crate::{
    clock_bridge::{CaptureIngress, CapturePacket, ClockBridgeConfig, ClockSource, capture_bridge},
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    stats::SILENT,
};

fn process() -> ProcessIdentity {
    ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    }
}

fn options() -> ProcessBridgeOptions {
    ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 2048,
            target_frames: 256,
            jitter_frames: 16,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 96,
            max_gap_frames: 96,
            source: DeliveryTimingSource::Empirical,
        }),
        engine_sample_rate: 48_000,
        max_demand_frames: 96,
        age_limit_100ns: None,
        max_ingress_elapsed_100ns: None,
    }
}

fn policy() -> ReplayWindowPolicy {
    ReplayWindowPolicy {
        max_initial_unserved_frames: 1000,
        max_initial_prime_discarded_frames: 1000,
        min_output_frames: 96,
        min_output_reads: 2,
        min_output_span_100ns: 1,
    }
}

fn packet(ingress: &mut CaptureIngress, stamp: u64) {
    ingress
        .push_packet_with_publication_clock(
            &[],
            CapturePacket {
                frames: 96,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
            || Some(stamp),
        )
        .unwrap();
}

fn read(source: &mut ClockSource, stamp: u64) {
    source
        .read_interleaved_with_publication_clock(&mut [0.0; 192], || Some(stamp))
        .unwrap();
}

fn live(starve: bool) -> (CaptureIngress, ClockSource, ProcessTraceArchive) {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        target_fill_frames: 256,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    for index in 0..12 {
        let stamp = 1_000_000 + index * 20_000;
        packet(&mut ingress, stamp);
        read(&mut source, stamp + 100);
        if starve && index > 3 {
            for offset in 1..12 {
                read(&mut source, stamp + 100 + offset * 100);
            }
        }
    }
    let artifact = ProcessTraceArchive {
        schema_version: 2,
        process: process(),
        owner_generation: 7,
        input_sample_rate: 48_000,
        consumer_sample_rate: 48_000,
        prepared_consumer_demand_frames: 96,
        packet_history: observer.packet_history().unwrap(),
        queue_history: observer.queue_history().unwrap(),
        collection_refusal: None,
    };
    (ingress, source, artifact)
}

#[test]
fn live_prefix_replays_open_owners_and_keeps_closed_replay_strict() {
    let (_ingress, _source, artifact) = live(false);
    assert!(!artifact.packet_history.writer_finished);
    assert!(!artifact.queue_history.writer_finished);
    assert!(artifact.trace(&process(), 7).is_err());
    let report = artifact
        .replay_live_window(&process(), 7, options(), policy())
        .unwrap();
    assert_eq!(
        report.status,
        ReplayWindowStatus::NoViolationInWindow,
        "{report:?}"
    );
    assert!(!report.qualified);
    assert_eq!(report.trace_origin, ProcessTraceOrigin::CallerSupplied);
    let prefix = report.prefix.unwrap();
    assert!(prefix.replayed_packet_events < prefix.observed_packet_events);
    assert!(prefix.replayed_read_events < prefix.observed_read_events);
    assert!(!prefix.packet_writer_finished && !prefix.read_writer_finished);
    assert!(report.replay.unwrap().report.window_end_100ns < prefix.cutoff_100ns);
    assert!(!artifact.packet_history.writer_finished);
}

#[test]
fn retained_live_prefix_keeps_native_origin_without_claiming_closure_or_qualification() {
    let (_ingress, _source, artifact) = live(false);
    let report = artifact
        .replay_retained_live_window(&process(), 7, options(), policy())
        .unwrap();
    assert_eq!(report.status, ReplayWindowStatus::NoViolationInWindow);
    assert_eq!(report.trace_origin, ProcessTraceOrigin::RetainedNativeOwner);
    assert!(!report.qualified);
    let prefix = report.prefix.unwrap();
    assert!(!prefix.packet_writer_finished && !prefix.read_writer_finished);
    let replay = report.replay.unwrap();
    assert_eq!(
        replay.report.trace_origin,
        ProcessTraceOrigin::RetainedNativeOwner
    );
    assert!(!replay.report.qualified);
    assert!(!artifact.packet_history.writer_finished);
    assert!(!artifact.queue_history.writer_finished);
}

#[test]
fn live_prefix_density_loss_dominates_insufficient_exposure() {
    let (_ingress, _source, artifact) = live(true);
    assert!(options().prepare().is_ok(), "per-wake budget must fit");
    let report = artifact
        .replay_live_window(
            &process(),
            7,
            options(),
            ReplayWindowPolicy {
                min_output_frames: u64::MAX,
                ..policy()
            },
        )
        .unwrap();
    assert_eq!(
        report.status,
        ReplayWindowStatus::ObservedViolation,
        "{report:?}"
    );
    assert!(
        report
            .replay
            .unwrap()
            .report
            .inventory
            .post_start_unserved_frames
            > 0
    );
}

#[test]
fn live_prefix_missing_capped_or_malformed_sentinel_is_inconclusive() {
    let (_ingress, _source, artifact) = live(false);
    for variant in 0..7 {
        let mut edited = artifact.clone();
        match variant {
            0 => {
                edited.collection_refusal =
                    Some(super::super::ProcessTraceCollectionRefusal::EventLimit)
            }
            1 => {
                edited.packet_history.events.remove(0);
                edited.packet_history.overwritten_events = 1;
            }
            2 => {
                edited
                    .packet_history
                    .events
                    .last_mut()
                    .unwrap()
                    .sample
                    .buffer_generation = 99
            }
            3 => {
                edited
                    .packet_history
                    .events
                    .last_mut()
                    .unwrap()
                    .sample
                    .qpc_before_100ns = Some(0)
            }
            4 => {
                edited
                    .queue_history
                    .events
                    .last_mut()
                    .unwrap()
                    .sample
                    .before
                    .held_frames += 1
            }
            5 => edited.queue_history.saturated = true,
            _ => {
                edited.queue_history.events[2].sample.before.ring_frames += 1;
                edited.queue_history.events[2].sample.after.ring_frames += 1;
            }
        }
        let report = edited
            .replay_live_window(&process(), 7, options(), policy())
            .unwrap();
        assert_eq!(
            report.status,
            ReplayWindowStatus::Inconclusive,
            "variant {variant}"
        );
        assert!(report.refusal.is_some());
        assert!(report.replay.is_none());
    }
}

#[test]
fn live_prefix_identity_generation_invalid_policy_and_freshness_never_disappear() {
    let (_ingress, _source, artifact) = live(false);
    let mut wrong = process();
    wrong.creation_time_100ns += 1;
    assert!(
        artifact
            .replay_live_window(&wrong, 7, options(), policy())
            .is_err()
    );
    assert!(
        artifact
            .replay_live_window(&process(), 8, options(), policy())
            .is_err()
    );
    assert!(
        artifact
            .replay_live_window(
                &process(),
                7,
                options(),
                ReplayWindowPolicy {
                    min_output_reads: 1,
                    ..policy()
                }
            )
            .is_err()
    );
    let report = artifact
        .replay_live_window(
            &process(),
            7,
            ProcessBridgeOptions {
                max_ingress_elapsed_100ns: Some(100),
                ..options()
            },
            policy(),
        )
        .unwrap();
    assert_eq!(report.status, ReplayWindowStatus::Inconclusive);
    assert!(report.refusal.unwrap().contains("host-ingress"));
}

#[test]
fn live_prefix_unsealed_tail_cannot_supply_or_consume_inside_reported_window() {
    let (_ingress, _source, artifact) = live(false);
    let report = artifact
        .replay_live_window(&process(), 7, options(), policy())
        .unwrap();
    let prefix = report.prefix.unwrap();
    let replay = report.replay.unwrap();
    assert_eq!(prefix.cutoff_100ns, 1_220_000);
    assert_eq!(prefix.replayed_packet_events, 11);
    assert_eq!(prefix.replayed_read_events, 11);
    assert_eq!(replay.report.packet_events, 11);
    assert_eq!(replay.report.read_events, 11);
    assert_eq!(replay.report.start_inventory_frames, 0);
}

#[test]
fn live_prefix_refusal_retains_requested_policy_identity_and_collection_cause() {
    let (_ingress, _source, mut artifact) = live(false);
    artifact.collection_refusal = Some(super::super::ProcessTraceCollectionRefusal::EventLimit);
    let report = artifact
        .replay_live_window(&process(), 7, options(), policy())
        .unwrap();
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(json["process"]["pid"], 42);
    assert_eq!(json["owner_generation"], 7);
    assert_eq!(json["options"]["input"]["target_frames"], 256);
    assert_eq!(json["policy"]["min_output_reads"], 2);
    assert_eq!(json["collection_refusal"], "event_limit");
    assert!(
        !json["refusal"]
            .as_str()
            .unwrap()
            .contains("requires closed")
    );
}

#[test]
fn live_prefix_tied_or_overlapping_cross_writer_evidence_never_becomes_feasible() {
    let (_ingress, _source, artifact) = live(false);
    for overlap in [false, true] {
        let mut edited = artifact.clone();
        // Move the producer's first bracket end onto/over the first read.
        // Each individual writer is still canonical, but cross-writer order
        // cannot establish complete-before-read offered supply.
        edited.packet_history.events[0].sample.qpc_after_100ns =
            Some(if overlap { 1_000_101 } else { 1_000_100 });
        let report = edited
            .replay_live_window(&process(), 7, options(), policy())
            .unwrap();
        assert_eq!(
            report.status,
            ReplayWindowStatus::Inconclusive,
            "{report:?}"
        );
        assert!(report.refusal.unwrap().contains("ambiguous"));
        assert!(report.replay.is_none());
    }
}

#[test]
fn live_prefix_insufficient_output_span_is_inconclusive_without_fabricated_loss() {
    let (_ingress, _source, artifact) = live(false);
    let report = artifact
        .replay_live_window(
            &process(),
            7,
            options(),
            ReplayWindowPolicy {
                min_output_span_100ns: u64::MAX,
                ..policy()
            },
        )
        .unwrap();
    assert_eq!(report.status, ReplayWindowStatus::Inconclusive);
    let replay = report.replay.unwrap();
    assert!(replay.assessment.insufficient_output_span);
    assert!(replay.assessment.violations.is_empty());
    assert!(report.refusal.is_none());
}
