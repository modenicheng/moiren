use super::*;

fn policy() -> ReplayWindowPolicy {
    ReplayWindowPolicy {
        max_initial_unserved_frames: 1000,
        max_initial_prime_discarded_frames: 1000,
        min_output_frames: 8,
        min_output_reads: 2,
        min_output_span_100ns: 100,
    }
}

fn assessed(steps: &[ExpandedStep]) -> AssessedProcessReplay {
    let options = expanded_candidate(48_000, 4, 4, 4, 12, 32);
    let (packets, reads, _) = expanded_collected(options.clock_config().unwrap(), steps);
    trace(packets, reads)
        .unwrap()
        .replay_window(&process(), 7, options, policy())
        .unwrap()
}

#[test]
fn finite_window_requires_started_output_exposure_and_keeps_provenance_unqualified() {
    let never = assessed(&[expanded_packet(4), ExpandedStep::Read(4)]);
    assert_eq!(never.assessment.status, ReplayWindowStatus::Inconclusive);
    assert_eq!(never.report.inventory.output_reads, 0);
    assert_eq!(never.report.inventory.output_span_lower_100ns, None);
    assert!(never.assessment.violations.is_empty());
    let good = assessed(&[
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
    ]);
    assert_eq!(
        good.assessment.status,
        ReplayWindowStatus::NoViolationInWindow
    );
    assert!(good.assessment.violations.is_empty());
    assert!(!good.report.qualified);
    assert_eq!(good.report.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert_eq!(good.assessment.policy, policy());
    assert_eq!(good.report.inventory.output_span_lower_100ns, Some(199));
    assert_eq!(good.report.inventory.output_span_upper_100ns, Some(201));
}

#[test]
fn finite_window_loss_dominates_inconclusive_and_later_reprime_is_not_startup() {
    let bad = assessed(&[
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
    ]);
    assert_eq!(bad.assessment.status, ReplayWindowStatus::ObservedViolation);
    assert!(
        bad.assessment
            .violations
            .contains(&ReplayWindowViolation::PostStartUnserved)
    );
    assert!(
        bad.assessment
            .violations
            .contains(&ReplayWindowViolation::Reset)
    );
    assert!(bad.report.inventory.post_start_unserved_frames > 0);
    let reset = assessed(&[
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
        ExpandedStep::Packet {
            frames: 4,
            flags: SILENT | DISCONTINUITY,
        },
        ExpandedStep::Read(4),
        ExpandedStep::Read(4),
    ]);
    assert_eq!(
        reset.assessment.status,
        ReplayWindowStatus::ObservedViolation
    );
    assert!(
        reset
            .assessment
            .violations
            .contains(&ReplayWindowViolation::ResetDiscard)
    );
}

#[test]
fn finite_window_policy_is_explicit_and_refuses_zero_exposure_thresholds() {
    let (packets, reads) = collected(8);
    let trace = trace(packets, reads).unwrap();
    for invalid in [
        ReplayWindowPolicy {
            min_output_reads: 1,
            ..policy()
        },
        ReplayWindowPolicy {
            min_output_frames: 0,
            ..policy()
        },
        ReplayWindowPolicy {
            min_output_span_100ns: 0,
            ..policy()
        },
    ] {
        assert!(matches!(
            trace.replay_window(&process(), 7, candidate(12, 32), invalid),
            Err(ProcessReplayError::InvalidWindowPolicy)
        ));
    }
    let mut json = serde_json::to_value(policy()).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("max_initial_unserved_frames");
    assert!(serde_json::from_value::<ReplayWindowPolicy>(json).is_err());
}

#[test]
fn finite_window_samples_pretrim_peak_and_gates_each_startup_allowance_explicitly() {
    let options = expanded_candidate(48_000, 4, 4, 4, 12, 64);
    let steps = [
        ExpandedStep::Read(4),
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
    ];
    let (packets, reads, _) = expanded_collected(options.clock_config().unwrap(), &steps);
    let trace = trace(packets, reads).unwrap();
    let accepted = trace
        .replay_window(&process(), 7, options, policy())
        .unwrap();
    assert_eq!(
        accepted.assessment.status,
        ReplayWindowStatus::NoViolationInWindow
    );
    assert_eq!(
        accepted.report.inventory.event_boundary_max_pending_frames,
        20
    );
    assert!(accepted.report.candidate.max_fill_frames < 20);
    assert_eq!(accepted.report.inventory.initial_unserved_frames, 4);
    assert_eq!(accepted.report.inventory.initial_prime_discarded_frames, 8);
    for (limits, reason) in [
        (
            ReplayWindowPolicy {
                max_initial_unserved_frames: 3,
                ..policy()
            },
            ReplayWindowViolation::InitialUnserved,
        ),
        (
            ReplayWindowPolicy {
                max_initial_prime_discarded_frames: 7,
                ..policy()
            },
            ReplayWindowViolation::InitialPrimeDiscard,
        ),
    ] {
        let assessed = trace.replay_window(&process(), 7, options, limits).unwrap();
        assert_eq!(
            assessed.assessment.status,
            ReplayWindowStatus::ObservedViolation
        );
        assert!(assessed.assessment.violations.contains(&reason));
    }
}

#[test]
fn finite_window_first_partial_output_is_not_excused_as_initial_silence() {
    // The target is met, but a single large first request consumes the queue.
    // Raw legacy startup_unserved includes its tail; strict assessment does not.
    let options = ProcessBridgeOptions::default();
    let mut steps = vec![];
    for _ in 0..3 {
        steps.push(expanded_packet(1024));
    }
    steps.push(ExpandedStep::Read(4096));
    let (packets, reads, _) = expanded_collected(options.clock_config().unwrap(), &steps);
    let trace =
        ObservedProcessTrace::from_observations(process(), 7, 48_000, 48_000, 4096, packets, reads)
            .unwrap();
    let result = trace
        .replay_window(
            &process(),
            7,
            ProcessBridgeOptions {
                max_demand_frames: 4096,
                ..options
            },
            policy(),
        )
        .unwrap();
    assert!(result.report.startup_unserved_frames > 0);
    assert_eq!(result.report.inventory.initial_unserved_frames, 0);
    assert_eq!(
        result.report.inventory.post_start_unserved_frames,
        result.report.unserved_frames
    );
    assert_eq!(
        result.assessment.status,
        ReplayWindowStatus::ObservedViolation
    );
    assert!(
        result
            .assessment
            .violations
            .contains(&ReplayWindowViolation::PostStartUnserved)
    );
    assert!(result.assessment.insufficient_output_reads);
}

#[test]
fn finite_window_conservation_keeps_input_and_output_units_separate() {
    for rate in [48_000, 96_000] {
        let demand = rate / 100;
        let options = expanded_candidate(rate, demand, 480, 480, 1024, 2048);
        let steps = (0..10)
            .flat_map(|_| [expanded_packet(480), ExpandedStep::Read(demand as usize)])
            .collect::<Vec<_>>();
        let (packets, reads, _) = expanded_collected(options.clock_config().unwrap(), &steps);
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
        let result = trace
            .replay_window(
                &process(),
                7,
                options,
                ReplayWindowPolicy {
                    max_initial_unserved_frames: 2000,
                    ..policy()
                },
            )
            .unwrap();
        assert_eq!(
            result.assessment.status,
            ReplayWindowStatus::NoViolationInWindow
        );
        let c = &result.report.candidate;
        assert_eq!(
            c.written_frames,
            c.retired_input_frames + result.report.terminal_inventory_frames
        );
        assert_eq!(
            c.retired_input_frames,
            c.consumed_input_frames
                + c.prime_discarded_frames
                + c.backlog_discarded_frames
                + c.age_discarded_frames
                + c.reset_discarded_frames
        );
        if rate == 96_000 {
            assert_ne!(c.consumed_input_frames, c.output_frames);
        }
    }
}

#[test]
fn finite_window_producer_only_tail_does_not_increase_output_exposure() {
    let mut steps = vec![
        expanded_packet(4),
        expanded_packet(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
        expanded_packet(4),
        ExpandedStep::Read(4),
    ];
    let before = assessed(&steps);
    steps.extend([expanded_packet(4), expanded_packet(4)]);
    let after = assessed(&steps);
    assert_eq!(after.report.inventory.trailing_offered_frames, 8);
    assert_eq!(
        after.report.inventory.output_span_lower_100ns,
        before.report.inventory.output_span_lower_100ns
    );
    assert_eq!(
        after.report.inventory.last_output_pending_frames,
        before.report.inventory.last_output_pending_frames
    );
    assert!(after.report.terminal_inventory_frames > before.report.terminal_inventory_frames);
    assert_eq!(after.assessment.status, before.assessment.status);
}
