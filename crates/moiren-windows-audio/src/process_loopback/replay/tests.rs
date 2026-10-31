use super::*;
#[path = "window_tests.rs"]
mod window_tests;
use crate::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    stats::SILENT,
};

fn process() -> ProcessIdentity {
    ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    }
}

fn collected(capacity: usize) -> (PacketHistorySnapshot, QueueHistorySnapshot) {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: capacity,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    for n in 0..8 {
        let t = 1000 + n * 100;
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 4,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(t),
            )
            .unwrap();
        source
            .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(t + 10))
            .unwrap();
    }
    drop(ingress);
    drop(source);
    (
        observer.packet_history().unwrap(),
        observer.queue_history().unwrap(),
    )
}

fn trace(
    packets: PacketHistorySnapshot,
    reads: QueueHistorySnapshot,
) -> Result<ObservedProcessTrace, ProcessReplayError> {
    ObservedProcessTrace::from_observations(process(), 7, 48_000, 48_000, 4, packets, reads)
}

fn candidate(target: u32, capacity: u32) -> ProcessBridgeOptions {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: capacity,
            target_frames: target,
            jitter_frames: 0,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 4,
            max_gap_frames: 4,
            source: DeliveryTimingSource::Empirical,
        }),
        engine_sample_rate: 48_000,
        max_demand_frames: 4,
        age_limit_100ns: None,
        max_ingress_elapsed_100ns: None,
    }
}

#[test]
fn retained_owner_promotion_changes_origin_without_qualifying_replayed_pcm() {
    let (packets, reads) = collected(8);
    let trace = trace(packets, reads).unwrap();
    let caller = trace.replay(&process(), 7, candidate(12, 32)).unwrap();
    let retained = trace
        .with_retained_owner_origin()
        .replay(&process(), 7, candidate(12, 32))
        .unwrap();
    assert_eq!(caller.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert_eq!(
        retained.trace_origin,
        ProcessTraceOrigin::RetainedNativeOwner
    );
    assert!(!caller.qualified && !retained.qualified);
    let mut expected = serde_json::to_value(caller).unwrap();
    expected["trace_origin"] =
        serde_json::to_value(ProcessTraceOrigin::RetainedNativeOwner).unwrap();
    assert_eq!(serde_json::to_value(retained).unwrap(), expected);
}

#[test]
fn ingress_elapsed_candidate_refuses_replay_instead_of_silently_disabling_enforcement() {
    let (packets, reads) = collected(8);
    let trace = trace(packets, reads).unwrap();
    for limit in [1, 100_000, u64::MAX] {
        let options = ProcessBridgeOptions {
            max_ingress_elapsed_100ns: Some(limit),
            ..candidate(12, 32)
        };
        let error = trace.replay(&process(), 7, options).unwrap_err();
        assert!(
            error.to_string().contains("host-ingress elapsed"),
            "{error}"
        );
    }
}

#[test]
fn ingress_elapsed_baseline_losses_remain_visible_when_candidate_has_no_limit() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 8,
        target_fill_frames: 4,
        detect_position_gaps: false,
        max_ingress_elapsed_100ns: Some(1),
        ..Default::default()
    })
    .unwrap();
    ingress
        .push_packet_with_publication_clock(
            &[],
            CapturePacket {
                frames: 4,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
            || Some(1000),
        )
        .unwrap();
    source
        .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(1100))
        .unwrap();
    drop(ingress);
    drop(source);
    let trace = trace(
        observer.packet_history().unwrap(),
        observer.queue_history().unwrap(),
    )
    .unwrap();
    let report = trace.replay(&process(), 7, candidate(12, 32)).unwrap();
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(json["baseline"]["ingress_elapsed_discarded_frames"], 4);
    assert!(!json["qualified"].as_bool().unwrap());
}

#[test]
fn replay_uses_fresh_empty_candidate_and_full_requests_not_baseline_retirement() {
    let (packets, reads) = collected(8);
    let baseline_underruns = reads
        .events
        .iter()
        .map(|event| event.sample.underrun_frames)
        .sum::<u64>();
    assert!(baseline_underruns > 0);
    let trace = trace(packets, reads).unwrap();
    let report = trace.replay(&process(), 7, candidate(12, 32)).unwrap();
    assert_eq!(report.status, ProcessReplayStatus::Replayed);
    assert!(!report.qualified);
    assert_eq!(report.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert_eq!(report.start_inventory_frames, 0);
    assert_eq!(report.requested_frames, 32);
    assert!(report.startup_unserved_frames > 0);
    assert_eq!(report.candidate.underrun_frames, 0);
    assert_eq!(report.baseline.underrun_frames, baseline_underruns);
    assert_eq!(report.candidate.captured_frames, 32);
    assert_eq!(
        report.offered_supply_assumption,
        OfferedSupplyAssumption::CompletePacketAtObservedOrder
    );
}

#[test]
fn replay_refuses_unclosed_truncated_unknown_or_edited_evidence() {
    let (packets, reads) = collected(8);
    for edit in 0..7 {
        let mut p = packets.clone();
        let mut r = reads.clone();
        match edit {
            0 => p.writer_finished = false,
            1 => r.writer_finished = false,
            2 => p.events[0].sample.qpc_after_100ns = None,
            3 => r.events[0].sample.qpc_after_100ns = None,
            4 => p.events[0].published_before_frames = 1,
            5 => r.events[0].sample.before.removed_frames = 1,
            _ => {
                p.overwritten_events = 1;
                p.total_events += 1;
            }
        }
        assert!(trace(p, r).is_err(), "edit={edit}");
    }
}

#[test]
fn replay_refuses_ambiguous_cross_owner_order_and_false_baseline_frontiers() {
    let (mut packets, reads) = collected(8);
    packets.events[0].sample.qpc_after_100ns = reads.events[0].sample.qpc_before_100ns;
    assert_eq!(
        trace(packets, reads).unwrap_err(),
        ProcessReplayError::AmbiguousOrder
    );
    let (packets, mut reads) = collected(8);
    // Coherent inventory alone cannot authorize a fabricated initial prefix.
    for event in &mut reads.events {
        event.sample.before.ring_frames += 1;
        event.sample.after.ring_frames += 1;
    }
    assert_eq!(
        trace(packets, reads).unwrap_err(),
        ProcessReplayError::InconsistentPublication
    );
}

#[test]
fn replay_requires_expected_identity_generation_and_admitted_candidate() {
    let (packets, reads) = collected(8);
    let trace = trace(packets, reads).unwrap();
    let mut wrong = process();
    wrong.creation_time_100ns += 1;
    assert_eq!(
        trace.replay(&wrong, 7, candidate(12, 32)).unwrap_err(),
        ProcessReplayError::IdentityMismatch
    );
    assert_eq!(
        trace.replay(&process(), 8, candidate(12, 32)).unwrap_err(),
        ProcessReplayError::GenerationMismatch
    );
    assert!(trace.replay(&process(), 7, candidate(2, 32)).is_err());
    let mut rebound = candidate(12, 32);
    rebound.engine_sample_rate = 96_000;
    assert!(trace.replay(&process(), 7, rebound).is_err());
}

#[test]
fn replay_rejects_candidate_delivery_bounds_that_do_not_cover_observed_supply() {
    let (packets, reads) = collected(8);
    let trace = trace(packets, reads).unwrap();
    let mut too_small = candidate(12, 32);
    too_small.delivery.as_mut().unwrap().max_packet_frames = 1;
    assert!(trace.replay(&process(), 7, too_small).is_err());
}

#[test]
fn replay_rejects_overflowing_untrusted_reset_totals_instead_of_panicking() {
    let (packets, mut reads) = collected(8);
    for event in &mut reads.events {
        event.sample.resets = u64::MAX;
    }
    assert!(trace(packets, reads).is_err());
}

#[test]
fn replay_uses_whole_offered_packets_instead_of_baseline_accepted_prefixes() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 8,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    for (frames, t) in [(6, 1000), (6, 1100)] {
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(t),
            )
            .unwrap();
    }
    source
        .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(1200))
        .unwrap();
    drop(ingress);
    drop(source);
    let trace = trace(
        observer.packet_history().unwrap(),
        observer.queue_history().unwrap(),
    )
    .unwrap();
    let mut options = candidate(12, 32);
    options.delivery.as_mut().unwrap().max_packet_frames = 6;
    options.delivery.as_mut().unwrap().max_gap_frames = 6;
    let report = trace.replay(&process(), 7, options).unwrap();
    assert_eq!(report.baseline.published_frames, 8);
    assert_eq!(report.baseline.dropped_frames, 4);
    assert_eq!(report.candidate.written_frames, 12);
    assert_eq!(report.candidate.dropped_frames, 0);
    assert_eq!(report.candidate.output_frames, 4);
    assert_eq!(report.terminal_inventory_frames, 9);
}

#[test]
fn replay_independent_capacity_and_bounded_target_candidates_are_not_presets() {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 4096,
        target_fill_frames: 2048,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    for n in 0..24 {
        let t = 1_000_000 + n * 10_000;
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 48,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(t),
            )
            .unwrap();
        source
            .read_interleaved_with_publication_clock(&mut [0.0; 96], || Some(t + 100))
            .unwrap();
    }
    drop(ingress);
    drop(source);
    let trace = ObservedProcessTrace::from_observations(
        process(),
        7,
        48_000,
        48_000,
        48,
        observer.packet_history().unwrap(),
        observer.queue_history().unwrap(),
    )
    .unwrap();
    for target in [256, 512, 1024] {
        let mut first = None;
        for capacity in [2048, 4096] {
            let report = trace
                .replay(
                    &process(),
                    7,
                    ProcessBridgeOptions {
                        input: TransportInputPolicy::Target {
                            capacity_frames: capacity,
                            target_frames: target,
                            jitter_frames: 0,
                        },
                        delivery: Some(InputDeliveryTiming {
                            sample_rate: 48_000,
                            max_packet_frames: 48,
                            max_gap_frames: 48,
                            source: DeliveryTimingSource::Empirical,
                        }),
                        engine_sample_rate: 48_000,
                        max_demand_frames: 48,
                        age_limit_100ns: None,
                        max_ingress_elapsed_100ns: None,
                    },
                )
                .unwrap();
            assert!(!report.qualified);
            assert_eq!(report.candidate.dropped_frames, 0);
            assert_eq!(report.candidate.underrun_frames, 0);
            let values = (
                report.startup_unserved_frames,
                report.candidate.prime_discarded_frames,
                report.candidate.output_frames,
                report.terminal_inventory_frames,
            );
            if let Some(first) = first {
                assert_eq!(values, first);
            }
            first = Some(values);
        }
    }
}

#[test]
fn replay_rejects_graph_origin_missing_sequence_and_oversized_work() {
    let (packets, reads) = collected(8);
    for edit in 0..5 {
        let mut p = packets.clone();
        let mut r = reads.clone();
        match edit {
            0 => p.events[0].sample.origin = PacketPublicationOrigin::GraphOutput,
            1 => p.events[0].sequence = 2,
            2 => p.events[0].sample.buffer_generation = 0,
            3 => r.events[0].sample.requested_frames = 65_537,
            _ => p.events[0].sample.offered_frames = 65_537,
        }
        assert!(trace(p, r).is_err());
    }
}

#[test]
fn replay_reports_overflow_and_discontinuity_without_claiming_content_or_sample_age() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 64,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    for n in 0..8 {
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 4,
                    flags: SILENT | if n == 4 { DISCONTINUITY } else { 0 },
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(1000 + n * 100),
            )
            .unwrap();
    }
    source
        .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(2000))
        .unwrap();
    drop(ingress);
    drop(source);
    let trace = trace(
        observer.packet_history().unwrap(),
        observer.queue_history().unwrap(),
    )
    .unwrap();
    let report = trace.replay(&process(), 7, candidate(12, 24)).unwrap();
    assert!(report.candidate.dropped_frames > 0);
    assert!(report.candidate.prime_discarded_frames > 0);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["qualified"], false);
    assert!(json["candidate"].get("silent_frames").is_none());
    assert!(json["candidate"].get("timestamp_errors").is_none());
    assert!(json["candidate"].get("oldest_frame_age").is_none());
}

#[derive(Clone, Copy)]
enum ExpandedStep {
    Packet { frames: usize, flags: u32 },
    Read(usize),
}

fn expanded_packet(frames: usize) -> ExpandedStep {
    ExpandedStep::Packet {
        frames,
        flags: SILENT,
    }
}

fn expanded_collected(
    config: ClockBridgeConfig,
    steps: &[ExpandedStep],
) -> (
    PacketHistorySnapshot,
    QueueHistorySnapshot,
    crate::clock_bridge::BridgeSnapshot,
) {
    let (mut ingress, mut source, observer) = capture_bridge(config).unwrap();
    for (index, step) in steps.iter().enumerate() {
        let mut now = 1_000_000 + index as u64 * 100;
        let clock = || {
            let stamp = now;
            now += 1;
            Some(stamp)
        };
        match *step {
            ExpandedStep::Packet { frames, flags } => {
                ingress
                    .push_packet_with_publication_clock(
                        &[],
                        CapturePacket {
                            frames,
                            flags,
                            device_position_frames: 0,
                            qpc_100ns: 0,
                        },
                        clock,
                    )
                    .unwrap();
            }
            ExpandedStep::Read(frames) => {
                source
                    .read_interleaved_with_publication_clock(&mut vec![0.0; frames * 2], clock)
                    .unwrap();
                // Publication brackets cannot qualify these positionless
                // packets as source-age observations, even during recovery.
                assert!(source.oldest_frame_stamp().is_none());
                assert!(source.snapshot().oldest_frame_age.is_none());
            }
        }
    }
    let terminal = source.snapshot();
    drop(ingress);
    drop(source);
    let packets = observer.packet_history().unwrap();
    let reads = observer.queue_history().unwrap();
    assert!(packets.writer_finished);
    assert!(reads.writer_finished);
    for event in &packets.events {
        assert_eq!(
            event.sample.qpc_after_100ns,
            event.sample.qpc_before_100ns.map(|stamp| stamp + 1)
        );
    }
    for event in &reads.events {
        assert_eq!(
            event.sample.qpc_after_100ns,
            event.sample.qpc_before_100ns.map(|stamp| stamp + 1)
        );
    }
    (packets, reads, terminal)
}

fn expanded_candidate(
    consumer_rate: u32,
    demand: u32,
    packet: u32,
    gap: u32,
    target: u32,
    capacity: u32,
) -> ProcessBridgeOptions {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    let options = ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: capacity,
            target_frames: target,
            jitter_frames: 0,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: packet,
            max_gap_frames: gap,
            // Deterministic software envelopes are experiment assumptions,
            // not authenticated native delivery or empirical admission proof.
            source: DeliveryTimingSource::Declared,
        }),
        engine_sample_rate: consumer_rate,
        max_demand_frames: demand,
        age_limit_100ns: None,
        max_ingress_elapsed_100ns: None,
    };
    options.prepare().unwrap();
    options
}

fn expanded_assert_no_age_or_qualification(report: &ProcessReplayReport) {
    assert!(!report.qualified);
    assert_eq!(report.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert_eq!(report.options.age_limit_100ns, None);
    assert_eq!(report.baseline.age_discarded_frames, 0);
    assert_eq!(report.candidate.age_discarded_frames, 0);
    let json = serde_json::to_value(report).unwrap();
    assert!(json["candidate"].get("oldest_frame_age").is_none());
    assert!(json["candidate"].get("last_qpc_100ns").is_none());
    assert!(json["candidate"].get("silent_frames").is_none());
}

#[test]
fn expanded_ordered_all_supported_consumer_rates_preserve_native_supply_and_actual_src_requests() {
    for rate in [44_100, 48_000, 88_200, 96_000] {
        let demand = rate / 100;
        let options = expanded_candidate(rate, demand, 480, 480, 1024, 2048);
        let steps = (0..10)
            .flat_map(|_| [expanded_packet(480), ExpandedStep::Read(demand as usize)])
            .collect::<Vec<_>>();
        let (packets, reads, terminal) =
            expanded_collected(options.clock_config().unwrap(), &steps);
        assert_eq!(packets.offered_frames, 4800);
        assert_eq!(packets.published_frames, 4800);
        assert_eq!(reads.events[0].sample.output_frames, 0);
        assert_eq!(reads.events[1].sample.output_frames, 0);
        assert!(
            reads.events[2..]
                .iter()
                .all(|event| event.sample.output_frames == u64::from(demand))
        );
        let trace = ObservedProcessTrace::from_observations(
            process(),
            7,
            48_000,
            rate,
            demand,
            packets,
            reads,
        )
        .unwrap();
        let report = trace.replay(&process(), 7, options).unwrap();
        assert_eq!(report.packet_events, 10);
        assert_eq!(report.read_events, 10);
        assert_eq!(report.requested_frames, u64::from(demand) * 10);
        assert_eq!(report.startup_unserved_frames, u64::from(demand) * 2);
        assert_eq!(report.unserved_frames, report.startup_unserved_frames);
        assert_eq!(report.candidate.output_frames, u64::from(demand) * 8);
        assert_eq!(report.candidate.output_frames, terminal.output_frames);
        assert_eq!(report.candidate.written_frames, 4800);
        assert_eq!(report.candidate.dropped_frames, 0);
        assert_eq!(report.candidate.underrun_frames, 0);
        assert_eq!(report.candidate.prime_discarded_frames, 416);
        assert_eq!(report.candidate.resets, 0);
        assert_eq!(
            report.terminal_inventory_frames,
            terminal.fill_frames as u64
        );
        assert_eq!(report.candidate.correction_ppm, terminal.correction_ppm);
        expanded_assert_no_age_or_qualification(&report);
    }
}

#[test]
fn expanded_ordered_repeated_input_discontinuities_reset_then_recover_without_interpolation_across_epochs()
 {
    let options = expanded_candidate(48_000, 4, 4, 4, 12, 32);
    let mut steps = vec![
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
    ];
    for _ in 0..2 {
        steps.extend([
            ExpandedStep::Packet {
                frames: 4,
                flags: SILENT | DISCONTINUITY,
            },
            ExpandedStep::Read(4),
            ExpandedStep::Read(4),
            expanded_packet(4),
            expanded_packet(4),
            expanded_packet(4),
            ExpandedStep::Read(4),
        ]);
    }
    let (packets, reads, terminal) = expanded_collected(options.clock_config().unwrap(), &steps);
    assert_eq!(packets.events[3].sample.buffer_generation, 2);
    assert_eq!(packets.events[7].sample.buffer_generation, 3);
    for index in [2, 5] {
        let interrupted = reads.events[index].sample;
        assert_eq!(interrupted.output_frames, 3);
        assert_eq!(interrupted.resets, 1);
        assert_eq!(interrupted.reset_discarded_frames, 2);
        // An epoch boundary leaves an unserved request suffix, not a
        // fabricated starvation count or proof of lost/non-silent PCM.
        assert_eq!(interrupted.underrun_frames, 0);
        assert_eq!(reads.events[index + 1].sample.output_frames, 4);
    }
    let report = trace(packets, reads)
        .unwrap()
        .replay(&process(), 7, options)
        .unwrap();
    assert_eq!(report.baseline.resets, 2);
    assert_eq!(report.baseline.reset_discarded_frames, 4);
    assert_eq!(report.baseline.underrun_frames, 0);
    assert_eq!(report.candidate.resets, 2);
    assert_eq!(report.candidate.reset_discarded_frames, 4);
    assert_eq!(
        report.candidate.written_frames,
        report.candidate.retired_input_frames + report.terminal_inventory_frames
    );
    assert_eq!(report.candidate.output_frames, 26);
    assert_eq!(report.candidate.output_frames, terminal.output_frames);
    assert_eq!(report.candidate.underrun_frames, 0);
    assert_eq!(report.candidate.dropped_frames, 0);
    assert_eq!(report.startup_unserved_frames, 0);
    assert_eq!(report.unserved_frames, 2);
    assert_eq!(report.inventory.initial_unserved_frames, 0);
    assert_eq!(report.inventory.post_start_unserved_frames, 2);
    assert_eq!(
        report.terminal_inventory_frames,
        terminal.fill_frames as u64
    );
    expanded_assert_no_age_or_qualification(&report);
}

#[test]
fn expanded_ordered_trailing_packet_is_terminal_inventory_not_an_unrecorded_final_read() {
    let options = expanded_candidate(48_000, 4, 4, 4, 12, 32);
    let prefix = [
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
    ];
    let mut reports = Vec::new();
    for trailing in [false, true] {
        let mut steps = prefix.to_vec();
        if trailing {
            steps.push(expanded_packet(4));
        }
        let (packets, reads, terminal) =
            expanded_collected(options.clock_config().unwrap(), &steps);
        let last_read = reads.events.last().unwrap().sample;
        let end = if trailing {
            packets.events.last().unwrap().sample.qpc_after_100ns
        } else {
            last_read.qpc_after_100ns
        };
        let pending_after_read = last_read.after.pending_frames().unwrap();
        let report = trace(packets, reads)
            .unwrap()
            .replay(&process(), 7, options)
            .unwrap();
        assert_eq!(Some(report.window_end_100ns), end);
        assert_eq!(report.requested_frames, 4);
        assert_eq!(report.candidate.output_frames, 4);
        assert_eq!(report.read_events, 1);
        assert_eq!(
            report.terminal_inventory_frames,
            pending_after_read + if trailing { 4 } else { 0 }
        );
        assert_eq!(
            report.terminal_inventory_frames,
            terminal.fill_frames as u64
        );
        expanded_assert_no_age_or_qualification(&report);
        reports.push(report);
    }
    let before = &reports[0];
    let after = &reports[1];
    assert_eq!(after.packet_events, before.packet_events + 1);
    assert_eq!(
        after.candidate.captured_frames,
        before.candidate.captured_frames + 4
    );
    assert_eq!(
        after.candidate.written_frames,
        before.candidate.written_frames + 4
    );
    assert_eq!(
        after.terminal_inventory_frames,
        before.terminal_inventory_frames + 4
    );
    assert_eq!(
        after.candidate.output_frames,
        before.candidate.output_frames
    );
    assert_eq!(
        after.candidate.priming_frames,
        before.candidate.priming_frames
    );
    assert_eq!(
        after.candidate.prime_discarded_frames,
        before.candidate.prime_discarded_frames
    );
    assert_eq!(after.unserved_frames, before.unserved_frames);
    assert!(after.window_end_100ns > before.window_end_100ns);
    assert_eq!(after.inventory.trailing_offered_frames, 4);
    assert_eq!(before.inventory.trailing_offered_frames, 0);
    assert_eq!(after.inventory.output_reads, 1);
    assert_eq!(after.inventory.output_span_lower_100ns, None);
    assert_eq!(
        after.inventory.last_output_pending_frames,
        before.inventory.last_output_pending_frames
    );
    assert_eq!(
        after.candidate.written_frames,
        after.candidate.retired_input_frames + after.terminal_inventory_frames
    );
}

#[test]
fn expanded_ordered_fully_rejected_baseline_packet_is_still_whole_offered_candidate_supply() {
    let (packets, reads, terminal) = expanded_collected(
        ClockBridgeConfig {
            capacity_frames: 8,
            target_fill_frames: 4,
            trim_on_prime: false,
            detect_position_gaps: false,
            ..Default::default()
        },
        &[
            expanded_packet(8),
            expanded_packet(4),
            ExpandedStep::Read(4),
        ],
    );
    let rejected = packets.events[1];
    assert_eq!(rejected.offered_before_frames, 8);
    assert_eq!(rejected.published_before_frames, 8);
    assert_eq!(rejected.sample.offered_frames, 4);
    assert_eq!(rejected.sample.accepted_frames, 0);
    assert_eq!(terminal.dropped_frames, 4);
    let options = expanded_candidate(48_000, 4, 8, 4, 12, 32);
    let report = trace(packets, reads)
        .unwrap()
        .replay(&process(), 7, options)
        .unwrap();
    assert_eq!(
        report.offered_supply_assumption,
        OfferedSupplyAssumption::CompletePacketAtObservedOrder
    );
    assert_eq!(report.baseline.offered_frames, 12);
    assert_eq!(report.baseline.published_frames, 8);
    assert_eq!(report.baseline.dropped_frames, 4);
    assert_eq!(report.candidate.captured_frames, 12);
    assert_eq!(report.candidate.written_frames, 12);
    assert_eq!(report.candidate.dropped_frames, 0);
    assert_eq!(report.candidate.output_frames, 4);
    assert_eq!(report.terminal_inventory_frames, 9);
    assert_eq!(report.inventory.event_boundary_max_pending_frames, 12);
    assert_eq!(report.inventory.event_boundary_min_pending_frames, 0);
    expanded_assert_no_age_or_qualification(&report);
}

#[test]
fn expanded_ordered_post_start_starvation_and_reprime_are_not_counted_as_startup() {
    let options = expanded_candidate(48_000, 4, 4, 4, 12, 32);
    let (packets, reads, terminal) = expanded_collected(
        options.clock_config().unwrap(),
        &[
            ExpandedStep::Read(4),
            expanded_packet(4),
            expanded_packet(4),
            expanded_packet(4),
            ExpandedStep::Read(4),
            ExpandedStep::Read(4),
            ExpandedStep::Read(4),
            ExpandedStep::Read(4),
            expanded_packet(4),
            expanded_packet(4),
            expanded_packet(4),
            ExpandedStep::Read(4),
        ],
    );
    assert_eq!(reads.events[0].sample.output_frames, 0);
    assert_eq!(reads.events[0].sample.underrun_frames, 0);
    assert_eq!(reads.events[1].sample.output_frames, 4);
    assert_eq!(reads.events[3].sample.output_frames, 3);
    assert_eq!(reads.events[3].sample.underrun_frames, 1);
    assert_eq!(reads.events[3].sample.resets, 1);
    assert_eq!(reads.events[4].sample.output_frames, 0);
    assert_eq!(reads.events[4].sample.underrun_frames, 0);
    assert_eq!(reads.events[5].sample.output_frames, 4);
    let report = trace(packets, reads)
        .unwrap()
        .replay(&process(), 7, options)
        .unwrap();
    assert_eq!(report.requested_frames, 24);
    assert_eq!(report.startup_unserved_frames, 4);
    assert_eq!(report.unserved_frames, 9);
    assert_eq!(report.inventory.initial_unserved_frames, 4);
    assert_eq!(report.inventory.post_start_unserved_frames, 5);
    assert_eq!(report.candidate.priming_frames, 8);
    assert_eq!(report.candidate.underrun_frames, 1);
    assert_eq!(report.candidate.resets, 1);
    assert_eq!(report.candidate.output_frames, 15);
    assert_eq!(report.candidate.output_frames, terminal.output_frames);
    assert_eq!(
        report.candidate.underrun_frames,
        report.baseline.underrun_frames
    );
    assert_eq!(
        report.unserved_frames,
        report.candidate.priming_frames + report.candidate.underrun_frames
    );
    assert_eq!(
        report.terminal_inventory_frames,
        terminal.fill_frames as u64
    );
    expanded_assert_no_age_or_qualification(&report);
}
