use super::*;
use crate::{
    rename::{IntentEvidence, PropertyReadback},
    validation,
};
use std::{cell::Cell, rc::Rc};

fn runtime(fresh: bool) -> RuntimeEvidence {
    RuntimeEvidence {
        intent: IntentEvidence {
            endpoint_id: "stable".into(),
            direction: "Playback".into(),
            channels: 2,
            binding: Some("node-7".into()),
            enabled: true,
        },
        requested_name: if fresh { "new" } else { "old" }.into(),
        provider_name: Some(if fresh { "new" } else { "old" }.into()),
        windows_name: None,
        windows_name_identity: None,
        windows_name_mmdevice_id: None,
        windows_name_error: None,
        rename_pending: false,
        resource: "present".into(),
        observation_generation: Some(if fresh { 45 } else { 40 }),
        native_stamp: Some(if fresh {
            [12, 7, 13, 1, 1]
        } else {
            [10, 7, 11, 3, 5]
        }),
        native_retirement: None,
        gate_open: true,
        allow_default: false,
        export_active_revision: Some(if fresh { 9 } else { 8 }),
        candidate_revision: if fresh { 9 } else { 8 },
        active_revision: Some(if fresh { 9 } else { 8 }),
        pending_revision: None,
        consumed_request_id: if fresh { 6 } else { 3 },
        phase: "running".into(),
        service_error: None,
        graph_error: None,
        provider_error: None,
    }
}

fn phase(fresh: bool) -> PcmPhase {
    let mut pcm = Observation::default();
    let expected = if fresh {
        validation::SECOND_EXPECTED
    } else {
        validation::EXPECTED
    };
    pcm.observe_packet(
        &expected.repeat(validation::REQUIRED_FRAMES as usize),
        expected,
        false,
    )
    .unwrap();
    PcmPhase {
        initialized: true,
        started: 2,
        released: true,
        release_errors: vec![],
        pair_mmdevice_id: Some(if fresh { "new-mm" } else { "old-mm" }.into()),
        gate_transitions: Vec::new(),
        name: Some(NativeName {
            root_instance: "root".into(),
            stable_id: "stable".into(),
            direction: "Playback".into(),
            mmdevice_id: if fresh { "new-mm" } else { "old-mm" }.into(),
            adapter_instance: "adapter".into(),
            windows_friendly_name: if fresh { "new" } else { "old" }.into(),
            device_desc: PropertyReadback::Value {
                value: "fixed description".into(),
            },
            device_interface_friendly_name: PropertyReadback::Value {
                value: "interface".into(),
            },
            topology: None,
        }),
        admission: Some(runtime(fresh)),
        confirmed: Some(runtime(fresh)),
        pcm,
    }
}

fn complete() -> Case {
    let mut case = Case::new("Playback".into(), "new".into());
    case.before = phase(false);
    case.after = phase(true);
    case.old_removed = Some(RetirementEvidence {
        generation: 41,
        native_stamp: [10, 7, 11, 3, 5],
    });
    case.edit_request = Some(4);
    case.diagnostic_completed = true;
    case.rename_while_started = true;
    case
}

#[test]
fn streaming_completion_requires_actual_start_and_both_pcm_windows() {
    let mut valid = complete();
    valid.finish();
    assert!(valid.completed);
    let mutations: &[fn(&mut Case)] = &[
        |c| c.before.started = 0,
        |c| c.after.started = 1,
        |c| c.before.pcm = Observation::default(),
        |c| c.after.pcm = Observation::default(),
        |c| c.before.initialized = false,
        |c| c.after.released = false,
        |c| c.old_removed = None,
        |c| c.after.name.as_mut().unwrap().windows_friendly_name = "new (fixed description)".into(),
    ];
    for mutate in mutations {
        let mut case = complete();
        mutate(&mut case);
        case.finish();
        assert!(
            !case.completed,
            "incomplete streaming evidence passed: {case:?}"
        );
    }
}

#[test]
fn streaming_post_pcm_cannot_reuse_old_identity_revision_or_observation() {
    let mutations: &[fn(&mut Case)] = &[
        |c| c.after = c.before.clone(),
        |c| c.after.confirmed.as_mut().unwrap().native_stamp = Some([10, 7, 11, 3, 5]),
        |c| c.after.confirmed.as_mut().unwrap().observation_generation = Some(40),
        |c| c.after.confirmed.as_mut().unwrap().active_revision = Some(8),
        |c| c.after.pair_mmdevice_id = Some("old-mm".into()),
        |c| c.after.admission = c.before.admission.clone(),
    ];
    for mutate in mutations {
        let mut case = complete();
        mutate(&mut case);
        case.finish();
        assert!(!case.completed, "stale post evidence passed: {case:?}");
    }
}

#[test]
fn partial_start_release_calls_stop_and_drops_owner_even_when_stop_fails() {
    struct Owner(Rc<Cell<bool>>);
    impl Drop for Owner {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let dropped = Rc::new(Cell::new(false));
    let stopped = Cell::new(false);
    let mut owner = Some(Owner(dropped.clone()));
    let mut evidence = PcmPhase {
        initialized: true,
        started: 1,
        ..Default::default()
    };
    release_owner(&mut owner, &mut evidence, |_| {
        stopped.set(true);
        vec!["capture Stop failed".into()]
    });
    assert!(stopped.get(), "partial Start owner was not stopped");
    assert!(dropped.get() && owner.is_none() && evidence.released);
    assert_eq!(evidence.release_errors, ["capture Stop failed"]);
}

#[test]
fn streaming_diagnostic_completion_cannot_erase_a_failure() {
    let mut case = complete();
    case.fail("exact Windows name failed".into());
    case.finish();
    assert!(!case.completed && case.failures == ["exact Windows name failed"]);
    case.finish();
    assert!(!case.completed && case.failures == ["exact Windows name failed"]);
}

#[test]
fn streaming_pcm_callbacks_require_both_starts_and_the_phase_specific_pattern() {
    let mut phase = PcmPhase::default();
    assert!(
        phase
            .observe_pcm(&validation::EXPECTED, false, false, false, 1)
            .is_err()
    );
    phase.started = 2;
    assert!(
        phase
            .observe_pcm(&validation::EXPECTED, false, false, false, 1)
            .is_ok()
    );
    let mut fresh = PcmPhase {
        started: 2,
        ..Default::default()
    };
    assert!(
        fresh
            .observe_pcm(&validation::EXPECTED, false, true, false, 1)
            .is_err()
    );
    assert_eq!(fresh.pcm.unexpected_frames, 1);
    assert!(
        fresh
            .observe_pcm(&validation::SECOND_EXPECTED, false, true, false, 2)
            .is_ok()
    );
    assert!(fresh.pcm.require_signal().is_err());
    let mut settling = PcmPhase {
        started: 2,
        ..Default::default()
    };
    assert!(
        settling
            .observe_pcm(&validation::EXPECTED, true, true, true, 1)
            .is_ok()
    );
    assert_eq!(settling.pcm.total_frames, 0);
    assert!(
        settling
            .observe_pcm(&[f32::NAN, 0.0], false, true, true, 2)
            .is_err()
    );
}

#[test]
fn stop_failures_prevent_completion_even_after_native_owner_drop() {
    let mut case = complete();
    case.before
        .release_errors
        .push("render Stop invalidated".into());
    case.finish();
    assert!(!case.completed);
    assert!(case.before.released);
    assert_eq!(case.before.release_errors, ["render Stop invalidated"]);
}

#[test]
fn default_report_keeps_streaming_and_legacy_results_absent() {
    let json = serde_json::to_value(crate::report::Report::default()).unwrap();
    assert!(json.get("rename").is_none() && json.get("rename_streaming").is_none());
}

#[test]
fn retirement_poll_preserves_first_unknown_and_last_actual_observations() {
    let mut case = complete();
    case.old_removed = None;
    let unknown = runtime(true);
    assert!(case.observe_retirement(unknown.clone()).is_err());
    let mut wrong = runtime(true);
    wrong.native_retirement = Some(RetirementEvidence {
        generation: 41,
        native_stamp: [99, 7, 11, 3, 5],
    });
    assert!(case.observe_retirement(wrong.clone()).is_err());
    assert!(
        case.retirement_first
            .as_ref()
            .unwrap()
            .native_retirement
            .is_none()
    );
    assert_eq!(
        case.retirement_last.as_ref().unwrap().native_retirement,
        wrong.native_retirement
    );
    case.fail("direct Removed deadline expired".into());
    let mut fresh = runtime(true);
    fresh.native_retirement = Some(RetirementEvidence {
        generation: 41,
        native_stamp: [10, 7, 11, 3, 5],
    });
    case.observe_retirement(fresh.clone()).unwrap();
    assert!(
        case.retirement_first
            .as_ref()
            .unwrap()
            .native_retirement
            .is_none()
    );
    assert_eq!(
        case.retirement_last.as_ref().unwrap().native_retirement,
        fresh.native_retirement
    );
    case.finish();
    assert!(!case.completed);
    assert_eq!(case.failures, ["direct Removed deadline expired"]);
}

#[test]
fn streaming_rename_keeps_old_pair_until_actual_transition_not_candidate_name_only() {
    let mut case = complete();
    let mut pending = runtime(false);
    pending.requested_name = "new".into();
    pending.rename_pending = true;
    pending.candidate_revision = 9;
    pending.consumed_request_id = 4;
    assert!(
        !case.observe_transition(pending.clone(), 4),
        "pending candidate alone released a running native Pair"
    );
    assert!(case.interruption_runtime.is_none());
    let mut closed = pending.clone();
    closed.gate_open = false;
    assert!(case.observe_transition(closed, 4));
    assert!(case.interruption_first.as_ref().unwrap().gate_open);
    assert!(!case.interruption_runtime.as_ref().unwrap().gate_open);
    let mut stale = runtime(true);
    stale.consumed_request_id = 3;
    let mut other = complete();
    assert!(!other.observe_transition(stale, 4));
    assert!(other.observe_transition(runtime(true), 4));
}

#[test]
fn streaming_transition_errors_remain_failed_after_a_clean_replacement() {
    for error_kind in 0..3 {
        let mut case = complete();
        let mut observed = runtime(false);
        observed.consumed_request_id = 4;
        match error_kind {
            0 => observed.service_error = Some("service failed during rename".into()),
            1 => observed.graph_error = Some("rename REMOVE failed".into()),
            _ => observed.provider_error = Some("provider QUERY failed".into()),
        }
        // An error can be published before the gate closes, then disappear
        // from the later successful Apply snapshot. It is still run evidence.
        assert!(!case.observe_transition(observed.clone(), 4));
        assert!(!case.observe_transition(observed, 4));
        assert!(case.observe_transition(runtime(true), 4));
        case.finish();
        assert!(!case.completed, "forgot transition error {error_kind}");
        assert_eq!(case.failures.len(), 1, "duplicate poll failure");
    }
}

#[test]
fn streaming_retirement_errors_remain_failed_after_direct_removed_recovers() {
    for error_kind in 0..3 {
        let mut case = complete();
        let mut observed = runtime(true);
        observed.native_retirement = case.old_removed.clone();
        match error_kind {
            0 => observed.service_error = Some("service failed during retirement".into()),
            1 => observed.graph_error = Some("retirement failed".into()),
            _ => observed.provider_error = Some("provider retirement QUERY failed".into()),
        }
        case.observe_retirement(observed.clone()).unwrap();
        case.observe_retirement(observed).unwrap();
        let mut recovered = runtime(true);
        recovered.native_retirement = case.old_removed.clone();
        case.observe_retirement(recovered).unwrap();
        case.finish();
        assert!(!case.completed, "forgot retirement error {error_kind}");
        assert_eq!(case.failures.len(), 1, "duplicate poll failure");
    }
}
