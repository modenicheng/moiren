use super::*;
use serde_json::json;

fn native(initial: u64, recovery: u64, short_reads: u64) -> Value {
    json!({"owner_generation": 7, "running": true,
        "streams": [{"node": "source", "kind": "virtual_playback"}],
        "boundary_probes": [{"node": "source", "kind": "virtual_playback",
            "capacity_frames": 1024, "level_frames": 0, "applied_milli_ppm": 0,
            "target_frames": 384, "xruns": 0,
            "soft_events": initial + recovery + short_reads,
            "playback_events": {"initial_prime_muted_frames": initial,
                "initial_prime_complete": true,
                "recovery_prime_muted_frames": recovery,
                "short_read_callbacks": short_reads, "saturated": false}}]})
}

fn joined(mut value: Value) -> Value {
    value["running"] = false.into();
    value
}

#[test]
fn boundary_startup_initial_only_uses_real_snapshot_not_fixed_allowance() {
    for initial in [0, 16, 32, 47, (1 << 53) + 9] {
        let mut tracker = Tracker::default();
        let observed = native(initial, 0, 0);
        assert!(tracker.observe(&observed, false).unwrap().is_empty());
        assert!(tracker.observe(&observed, false).unwrap().is_empty());
        assert!(
            tracker
                .observe(&joined(observed.clone()), true)
                .unwrap()
                .is_empty()
        );
        let report = serde_json::to_value(tracker).unwrap();
        assert_eq!(report["startup"]["soft_events"], initial);
        assert_eq!(report["startup"]["initial_prime_muted_frames"], initial);
        assert_eq!(report["steady_delta"]["initial_prime_muted_frames"], 0);
        assert_eq!(report["latest"]["soft_events"], initial);
        assert_eq!(report["final_after_join"], true);
        assert_eq!(observed["boundary_probes"][0]["soft_events"], initial);
    }
}

#[test]
fn boundary_startup_absence_waits_but_later_initial_growth_is_not_rebased() {
    let mut tracker = Tracker::default();
    let mut pending = native(0, 0, 0);
    pending["boundary_probes"][0]["playback_events"] = Value::Null;
    assert!(tracker.observe(&pending, false).unwrap().is_empty());
    assert!(serde_json::to_value(&tracker).unwrap()["startup"].is_null());
    assert!(
        tracker
            .observe(&native(16, 0, 0), false)
            .unwrap()
            .is_empty()
    );
    let faults = tracker.observe(&native(32, 0, 0), false).unwrap();
    assert!(faults.iter().any(|fault| fault.contains("initial")));
    let report = serde_json::to_value(&tracker).unwrap();
    assert_eq!(report["startup"]["initial_prime_muted_frames"], 16);
    assert_eq!(report["steady_delta"]["initial_prime_muted_frames"], 16);
    pending["boundary_probes"][0]["soft_events"] = 32.into();
    assert!(tracker.observe(&pending, false).unwrap().is_empty());
    assert!(Tracker::default().observe(&joined(pending), true).is_err());
}

#[test]
fn boundary_startup_faults_and_trailing_recovery_short_reads_are_never_absorbed() {
    for (initial, recovery, short_reads) in [(32, 2, 0), (32, 0, 1)] {
        let mut tracker = Tracker::default();
        assert!(
            !tracker
                .observe(&native(initial, recovery, short_reads), false)
                .unwrap()
                .is_empty()
        );
    }
    let mut tracker = Tracker::default();
    tracker.observe(&native(32, 0, 0), false).unwrap();
    let faults = tracker.observe(&joined(native(32, 8, 3)), true).unwrap();
    assert!(faults.iter().any(|fault| fault.contains("recovery")));
    assert!(faults.iter().any(|fault| fault.contains("short-read")));
    let report = serde_json::to_value(tracker).unwrap();
    assert_eq!(report["steady_delta"]["recovery_prime_muted_frames"], 8);
    assert_eq!(report["steady_delta"]["short_read_callbacks"], 3);
    assert_eq!(report["steady_delta"]["soft_events"], 11);
}

#[test]
fn boundary_live_mixed_publication_is_not_forced_to_equal_but_final_is_checked() {
    let mut tracker = Tracker::default();
    let mut torn = native(32, 0, 0);
    torn["boundary_probes"][0]["soft_events"] = 0.into();
    assert!(tracker.observe(&torn, false).unwrap().is_empty());
    assert!(
        tracker
            .observe(&joined(native(32, 0, 0)), true)
            .unwrap()
            .is_empty()
    );
    let mut tracker = Tracker::default();
    tracker.observe(&native(32, 0, 0), false).unwrap();
    let mut unexplained = joined(native(32, 0, 0));
    unexplained["boundary_probes"][0]["soft_events"] = 33.into();
    assert!(tracker.observe(&unexplained, true).is_err());
}

#[test]
fn boundary_unknown_saturated_regressed_and_changed_identity_refuse() {
    for pointer in [
        "/boundary_probes/0/playback_events/initial_prime_complete",
        "/boundary_probes/0/playback_events/initial_prime_muted_frames",
        "/boundary_probes/0/playback_events/recovery_prime_muted_frames",
        "/boundary_probes/0/playback_events/short_read_callbacks",
        "/boundary_probes/0/playback_events/saturated",
        "/boundary_probes/0/soft_events",
        "/boundary_probes/0/xruns",
        "/boundary_probes/0/target_frames",
    ] {
        let mut tracker = Tracker::default();
        tracker.observe(&native(32, 2, 3), false).unwrap();
        let mut invalid = native(32, 2, 3);
        *invalid.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(tracker.observe(&invalid, false).is_err(), "{pointer}");
    }
    for (pointer, replacement) in [
        ("/owner_generation", json!(8)),
        ("/boundary_probes/0/node", json!("replacement")),
        ("/boundary_probes/0/playback_events/saturated", json!(true)),
        (
            "/boundary_probes/0/playback_events/initial_prime_muted_frames",
            json!(31),
        ),
        (
            "/boundary_probes/0/playback_events/recovery_prime_muted_frames",
            json!(1),
        ),
        (
            "/boundary_probes/0/playback_events/short_read_callbacks",
            json!(2),
        ),
        ("/boundary_probes/0/soft_events", json!(36)),
        ("/boundary_probes/0/soft_events", json!(u64::MAX)),
        ("/boundary_probes/0/target_frames", json!(385)),
    ] {
        let mut tracker = Tracker::default();
        tracker.observe(&native(32, 2, 3), false).unwrap();
        let mut invalid = native(32, 2, 3);
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(tracker.observe(&invalid, false).is_err(), "{pointer}");
    }
    let mut tracker = Tracker::default();
    tracker.observe(&native(32, 0, 0), false).unwrap();
    let mut overflow = joined(native(32, 0, 0));
    overflow["boundary_probes"][0]["playback_events"]["recovery_prime_muted_frames"] =
        (u64::MAX - 1).into();
    assert!(tracker.observe(&overflow, true).is_err());
    assert!(
        Tracker::default()
            .observe(&joined(native(32, 0, 0)), true)
            .is_err(),
        "final cannot invent startup"
    );
}

#[test]
fn boundary_live_unknown_after_startup_recovers_without_fabricating_counters() {
    use super::super::validate_health_with_policy;
    for policy in [
        super::super::ValidationPolicy::default(),
        super::super::tests::correctness(),
    ] {
        let health = super::super::tests::healthy();
        let mut quality = tracked_quality();
        for initial in [16, 32] {
            let mut observed = native(initial, 0, 0);
            observed["boundary_probes"][0]["playback_events"]["initial_prime_complete"] =
                (initial == 32).into();
            validate_health_with_policy(
                &health,
                &health,
                &report(observed),
                &mut None,
                policy,
                &mut quality,
            )
            .unwrap();
        }
        let mut unknown = native(32, 0, 0);
        unknown["boundary_probes"][0]["playback_events"] = Value::Null;
        for _ in 0..2 {
            validate_health_with_policy(
                &health,
                &health,
                &report(unknown.clone()),
                &mut None,
                policy,
                &mut quality,
            )
            .unwrap();
        }
        let evidence = quality.boundary_evidence();
        assert_eq!(evidence["playback"]["unknown_observations"], 2);
        assert_eq!(evidence["playback"]["classification_available"], false);
        assert_eq!(
            evidence["playback"]["latest"]["initial_prime_muted_frames"],
            32
        );
        assert!(quality.failure.is_none());
        validate_health_with_policy(
            &health,
            &health,
            &report(native(32, 0, 0)),
            &mut None,
            policy,
            &mut quality,
        )
        .unwrap();
        assert_eq!(
            quality.boundary_evidence()["playback"]["classification_available"],
            true
        );
    }
    let mut tracker = Tracker::default();
    tracker.observe(&native(32, 0, 0), false).unwrap();
    let mut unknown = joined(native(32, 0, 0));
    unknown["boundary_probes"][0]["playback_events"] = Value::Null;
    assert!(tracker.observe(&unknown, true).is_err());
}

#[test]
fn boundary_source_completion_not_frame_allowance_freezes_startup() {
    for (first, complete) in [(16, 32), (3, 9), (0, 0)] {
        let mut tracker = Tracker::default();
        let mut incomplete = native(first, 0, 0);
        incomplete["boundary_probes"][0]["playback_events"]["initial_prime_complete"] =
            false.into();
        assert!(tracker.observe(&incomplete, false).unwrap().is_empty());
        let evidence = serde_json::to_value(&tracker).unwrap();
        assert!(evidence["startup"].is_null());
        assert!(evidence["steady_delta"].is_null());
        assert_eq!(evidence["pending_initial_observations"], 1);
        assert!(
            tracker
                .observe(&native(complete, 0, 0), false)
                .unwrap()
                .is_empty()
        );
        assert!(
            tracker
                .observe(&joined(native(complete, 0, 0)), true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            serde_json::to_value(tracker).unwrap()["startup"]["initial_prime_muted_frames"],
            complete
        );
    }
    for fault in [native(16, 1, 0), native(16, 0, 1), {
        let mut value = native(16, 0, 0);
        value["boundary_probes"][0]["xruns"] = 1.into();
        value
    }] {
        let mut tracker = Tracker::default();
        let mut incomplete = fault;
        incomplete["boundary_probes"][0]["playback_events"]["initial_prime_complete"] =
            false.into();
        assert!(!tracker.observe(&incomplete, false).unwrap().is_empty());
        assert!(serde_json::to_value(tracker).unwrap()["startup"].is_null());
    }
}

#[test]
fn boundary_completed_prime_cannot_regress_or_finish_while_incomplete() {
    let mut tracker = Tracker::default();
    tracker.observe(&native(32, 0, 0), false).unwrap();
    let mut incomplete = native(32, 0, 0);
    incomplete["boundary_probes"][0]["playback_events"]["initial_prime_complete"] = false.into();
    assert!(tracker.observe(&incomplete, false).is_err());
    let mut tracker = Tracker::default();
    tracker.observe(&incomplete, false).unwrap();
    assert!(tracker.observe(&joined(incomplete), true).is_err());
}

#[test]
fn boundary_pending_initial_and_unknown_do_not_hide_faults_from_either_policy() {
    use super::super::validate_health_with_policy;
    for policy in [
        super::super::ValidationPolicy::default(),
        super::super::tests::correctness(),
    ] {
        for fault in ["recovery", "short-read", "xrun", "unknown-xrun"] {
            let health = super::super::tests::healthy();
            let mut quality = tracked_quality();
            let mut observed = match fault {
                "recovery" => native(16, 1, 0),
                "short-read" => native(16, 0, 1),
                _ => native(16, 0, 0),
            };
            observed["boundary_probes"][0]["playback_events"]["initial_prime_complete"] =
                false.into();
            if fault == "xrun" || fault == "unknown-xrun" {
                observed["boundary_probes"][0]["xruns"] = 1.into();
            }
            if fault == "unknown-xrun" {
                observed["boundary_probes"][0]["playback_events"] = Value::Null;
            }
            let result = validate_health_with_policy(
                &health,
                &health,
                &report(observed),
                &mut None,
                policy,
                &mut quality,
            );
            assert_eq!(result.is_ok(), policy.is_correctness(), "{fault}");
            assert!(quality.failure.is_some(), "{fault}");
            let evidence = quality.boundary_evidence();
            assert!(evidence["playback"]["startup"].is_null());
            assert!(evidence["playback"]["steady_delta"].is_null());
            assert_eq!(evidence["quality_complete"], false);
        }
    }
}

#[test]
fn boundary_unknown_and_pending_diagnostic_counters_cannot_wrap() {
    let mut tracker = Tracker {
        unknown_observations: u64::MAX,
        ..Default::default()
    };
    let mut unknown = native(0, 0, 0);
    unknown["boundary_probes"][0]["playback_events"] = Value::Null;
    assert!(tracker.observe(&unknown, false).is_err());
    assert_eq!(tracker.unknown_observations, u64::MAX);
    let mut tracker = Tracker {
        pending_initial_observations: u64::MAX,
        ..Default::default()
    };
    let mut incomplete = native(16, 0, 0);
    incomplete["boundary_probes"][0]["playback_events"]["initial_prime_complete"] = false.into();
    assert!(tracker.observe(&incomplete, false).is_err());
    assert_eq!(tracker.pending_initial_observations, u64::MAX);
}

fn tracked_quality() -> super::super::Quality {
    super::super::Quality {
        playback_boundary: Some(Tracker::default()),
        ..Default::default()
    }
}

fn report(observed: Value) -> super::super::super::NativeReport {
    let mut report = super::super::tests::raw_health_report();
    let live = report
        .engine
        .as_mut()
        .unwrap()
        .get_mut("observations")
        .unwrap();
    live["boundary_probes"] = observed["boundary_probes"].clone();
    report
}

#[test]
fn boundary_collector_live_and_final_initial_only_preserve_raw_and_unqualified() {
    use super::super::{graph_summary_health_with_policy, validate_health_with_policy};
    for policy in [
        super::super::ValidationPolicy::default(),
        super::super::tests::correctness(),
    ] {
        let mut quality = tracked_quality();
        let health = super::super::tests::healthy();
        let report = report(native(47, 0, 0));
        let original = report.engine.clone();
        validate_health_with_policy(&health, &health, &report, &mut None, policy, &mut quality)
            .unwrap();
        let mut final_report = super::super::tests::final_schema2();
        final_report["native"]["boundary_probes"] = native(47, 0, 0)["boundary_probes"].clone();
        graph_summary_health_with_policy(Some(&final_report), policy, &mut quality).unwrap();
        assert!(quality.failure.is_none());
        assert_eq!(report.engine, original);
        assert_eq!(
            final_report["native"]["boundary_probes"][0]["soft_events"],
            47
        );
        let evidence = quality.boundary_evidence();
        assert_eq!(evidence["qualified"], false);
        assert_eq!(evidence["quality_complete"], false);
        assert_eq!(
            evidence["playback"]["startup"]["initial_prime_muted_frames"],
            47
        );
    }
}

#[test]
fn boundary_collector_retains_other_faults_and_strict_correctness_separation() {
    use super::super::{graph_summary_health_with_policy, validate_health_with_policy};
    for policy in [
        super::super::ValidationPolicy::default(),
        super::super::tests::correctness(),
    ] {
        for fault in ["initial", "recovery", "short-read", "native-timeout"] {
            let mut quality = tracked_quality();
            let health = super::super::tests::healthy();
            validate_health_with_policy(
                &health,
                &health,
                &report(native(32, 0, 0)),
                &mut None,
                policy,
                &mut quality,
            )
            .unwrap();
            let observed = match fault {
                "initial" => native(33, 0, 0),
                "recovery" => native(32, 1, 0),
                "short-read" => native(32, 0, 1),
                _ => native(32, 0, 0),
            };
            let mut final_report = super::super::tests::final_schema2();
            final_report["native"]["boundary_probes"] = observed["boundary_probes"].clone();
            if fault == "native-timeout" {
                final_report["professional"]["streams"][0]["native_wait_timeouts"] = 1.into();
            }
            let result =
                graph_summary_health_with_policy(Some(&final_report), policy, &mut quality);
            assert_eq!(result.is_ok(), policy.is_correctness(), "{fault}");
            assert!(quality.failure.is_some(), "{fault}");
        }
        let mut quality = tracked_quality();
        let mut health = super::super::tests::healthy();
        health.quality_faults = 4;
        let result = validate_health_with_policy(
            &health,
            &health,
            &report(native(32, 0, 0)),
            &mut None,
            policy,
            &mut quality,
        );
        assert_eq!(result.is_ok(), policy.is_correctness());
        assert!(
            quality.failure.is_some(),
            "non-boundary aggregate must not be forgiven"
        );
    }
}

#[test]
fn boundary_collector_requires_final_classification_not_legacy_summary() {
    use super::super::graph_summary_health_with_policy;
    let mut quality = tracked_quality();
    let legacy = json!({"schema_version": 1, "failure": null,
        "render": {"failure": null, "stream_started": true, "stop_succeeded": true,
            "stats": {"bridge_shortfall_frames": 0, "timeout_wakes": 0}}});
    assert!(
        graph_summary_health_with_policy(
            Some(&legacy),
            super::super::tests::correctness(),
            &mut quality
        )
        .is_err()
    );
}

#[test]
fn boundary_final_hard_refusal_retains_raw_quality_evidence() {
    use super::super::{graph_summary_health_with_policy, validate_health_with_policy};
    let policy = super::super::tests::correctness();
    let mut quality = tracked_quality();
    let health = super::super::tests::healthy();
    validate_health_with_policy(
        &health,
        &health,
        &report(native(32, 0, 0)),
        &mut None,
        policy,
        &mut quality,
    )
    .unwrap();
    let mut final_report = super::super::tests::final_schema2();
    final_report["native"]["boundary_probes"] = native(32, 0, 0)["boundary_probes"].clone();
    final_report["native"]["boundary_probes"][0]["soft_events"] = 33.into();
    assert!(graph_summary_health_with_policy(Some(&final_report), policy, &mut quality).is_err());
    assert!(quality.failure.is_some());
    assert_eq!(
        quality.first_observations.as_ref().unwrap()["native_summary"],
        final_report
    );
}

#[test]
fn boundary_unadapted_short_read_legacy_counter_is_not_a_frame_sum() {
    let mut tracker = Tracker::default();
    let mut observed = native(0, 0, 3);
    observed["boundary_probes"][0]["target_frames"] = Value::Null;
    observed["boundary_probes"][0]["soft_events"] = 0.into();
    assert!(
        tracker
            .observe(&observed, false)
            .unwrap()
            .iter()
            .any(|fault| fault.contains("short-read"))
    );
    assert!(
        tracker
            .observe(&joined(observed.clone()), true)
            .unwrap()
            .iter()
            .any(|fault| fault.contains("short-read"))
    );
    let evidence = serde_json::to_value(tracker).unwrap();
    assert_eq!(evidence["latest"]["soft_events"], 0);
    assert_eq!(evidence["latest"]["short_read_callbacks"], 3);
    assert_eq!(observed["boundary_probes"][0]["soft_events"], 0);
}
