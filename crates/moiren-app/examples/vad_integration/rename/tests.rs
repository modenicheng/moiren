use super::*;

fn baseline(direction: &str) -> RuntimeEvidence {
    RuntimeEvidence {
        intent: IntentEvidence {
            endpoint_id: "stable".into(),
            direction: direction.into(),
            channels: 2,
            binding: Some("node-7".into()),
            enabled: true,
        },
        requested_name: "old".into(),
        provider_name: Some("old".into()),
        rename_pending: false,
        resource: "present".into(),
        observation_generation: Some(40),
        native_stamp: Some([10, 7, 11, 3, 5]),
        gate_open: false,
        allow_default: false,
        export_active_revision: None,
        candidate_revision: 8,
        active_revision: None,
        pending_revision: None,
        consumed_request_id: 3,
        phase: "idle".into(),
        service_error: None,
        graph_error: None,
        provider_error: None,
    }
}

fn name(direction: &str) -> NativeName {
    NativeName {
        root_instance: "root".into(),
        stable_id: "stable".into(),
        direction: direction.into(),
        mmdevice_id: "mmdevice".into(),
        adapter_instance: "adapter".into(),
        windows_friendly_name: "Windows old label".into(),
    }
}

#[test]
fn held_rename_requires_explicit_production_pending_removal_and_unchanged_identity() {
    for direction in ["Playback", "Microphone"] {
        let old = baseline(direction);
        let mut pending = old.clone();
        pending.requested_name = "new".into();
        pending.rename_pending = true;
        pending.resource = "removing".into();
        pending.observation_generation = Some(41);
        pending.candidate_revision = 9;
        pending.consumed_request_id = 4;
        held(&old, &pending, "new", 4).unwrap();
        let mutations: &[fn(&mut RuntimeEvidence)] = &[
            |e| e.rename_pending = false,
            |e| e.resource = "present".into(),
            |e| e.provider_name = Some("new".into()),
            |e| e.requested_name = "other".into(),
            |e| e.intent.binding = None,
            |e| e.intent.direction = "other".into(),
            |e| e.native_stamp = None,
            |e| e.native_stamp = Some([12, 7, 13, 3, 5]),
            |e| e.observation_generation = Some(40),
            |e| e.gate_open = true,
            |e| e.active_revision = Some(9),
            |e| e.export_active_revision = Some(9),
            |e| e.consumed_request_id = 3,
        ];
        for mutate in mutations {
            let mut invalid = pending.clone();
            mutate(&mut invalid);
            assert!(held(&old, &invalid, "new", 4).is_err(), "{invalid:?}");
        }
    }
}

#[test]
fn held_native_property_and_exact_topology_cannot_drift() {
    for direction in ["Playback", "Microphone"] {
        let old = name(direction);
        same_held_device(&old, &old).unwrap();
        let mutations: &[fn(&mut NativeName)] = &[
            |e| e.windows_friendly_name = "new".into(),
            |e| e.mmdevice_id = "other".into(),
            |e| e.adapter_instance = "other".into(),
            |e| e.root_instance = "other".into(),
            |e| e.stable_id = "other".into(),
            |e| e.direction = "other".into(),
        ];
        for mutate in mutations {
            let mut invalid = old.clone();
            mutate(&mut invalid);
            assert!(same_held_device(&old, &invalid).is_err(), "{invalid:?}");
        }
    }
}

#[test]
fn dto_name_success_needs_fresh_native_pdo_endpoint_and_engine_confirmation() {
    for direction in ["Playback", "Microphone"] {
        let old = baseline(direction);
        let mut fresh = old.clone();
        fresh.requested_name = "new".into();
        fresh.provider_name = Some("new".into());
        fresh.native_stamp = Some([12, 7, 13, 1, 1]);
        fresh.observation_generation = Some(45);
        fresh.gate_open = true;
        fresh.candidate_revision = 9;
        fresh.active_revision = Some(9);
        fresh.export_active_revision = Some(9);
        fresh.consumed_request_id = 6;
        fresh.phase = "running".into();
        replacement(&old, &fresh, "new", 6).unwrap();
        let mutations: &[fn(&mut RuntimeEvidence)] = &[
            |e| e.native_stamp = None,
            |e| e.native_stamp = Some([10, 7, 11, 9, 10]),
            |e| e.native_stamp = Some([12, 7, 11, 1, 1]),
            |e| e.native_stamp = Some([10, 7, 13, 1, 1]),
            |e| e.native_stamp = Some([12, 8, 13, 1, 1]),
            |e| e.native_stamp = Some([12, 7, 13, 0, 1]),
            |e| e.observation_generation = None,
            |e| e.observation_generation = Some(40),
            |e| e.provider_name = Some("old".into()),
            |e| e.rename_pending = true,
            |e| e.resource = "removing".into(),
            |e| e.intent.endpoint_id = "other".into(),
            |e| e.intent.binding = None,
            |e| e.gate_open = false,
            |e| e.allow_default = true,
            |e| e.active_revision = Some(8),
            |e| e.export_active_revision = None,
            |e| e.pending_revision = Some(9),
            |e| e.consumed_request_id = 5,
            |e| e.phase = "idle".into(),
            |e| e.provider_error = Some("QUERY failed".into()),
            |e| e.graph_error = Some("CREATE failed".into()),
        ];
        for mutate in mutations {
            let mut invalid = fresh.clone();
            mutate(&mut invalid);
            assert!(
                replacement(&old, &invalid, "new", 6).is_err(),
                "{invalid:?}"
            );
        }
    }
}

#[test]
fn actual_windows_property_must_match_requested_name_exactly() {
    for direction in ["Playback", "Microphone"] {
        let old = name(direction);
        let mut new = old.clone();
        // Windows may reuse both textual IDs. Native generations prove recreation.
        new.windows_friendly_name = "Requested new name".into();
        windows_name(&old, &new, "Requested new name").unwrap();
        for wrong in [
            "requested new name",
            "Requested new name ",
            "Generic speakers",
            "",
        ] {
            new.windows_friendly_name = wrong.into();
            assert!(windows_name(&old, &new, "Requested new name").is_err());
        }
        new.windows_friendly_name = "Requested new name".into();
        new.root_instance = "unrelated-root".into();
        assert!(windows_name(&old, &new, "Requested new name").is_err());
    }
}

fn completed_case() -> Case {
    let mut case = Case::new("Playback".into(), "new".into());
    case.held_client_initialized = true;
    case.held_client_released = true;
    case.replacement_client_initialized = true;
    case.replacement_client_released = true;
    case.held_completion_refused = true;
    case.fresh_replacement_confirmed = true;
    case.windows_name_verified = true;
    case
}

#[test]
fn case_completion_requires_every_native_owner_released_and_all_evidence_gates() {
    let mut complete = completed_case();
    complete.finish();
    assert!(complete.completed);
    let mutations: &[fn(&mut Case)] = &[
        |e| e.held_client_initialized = false,
        |e| e.held_client_released = false,
        |e| e.replacement_client_initialized = false,
        |e| e.replacement_client_released = false,
        |e| e.held_completion_refused = false,
        |e| e.fresh_replacement_confirmed = false,
        |e| e.windows_name_verified = false,
    ];
    for mutate in mutations {
        let mut invalid = completed_case();
        mutate(&mut invalid);
        invalid.finish();
        assert!(!invalid.completed && invalid.error.is_some(), "{invalid:?}");
    }
}

#[test]
fn cleanup_failures_are_retained_separately_from_the_primary_failure() {
    let mut case = completed_case();
    case.error = Some("primary Windows name mismatch".into());
    case.cleanup_errors.push("held release failed".into());
    case.cleanup_errors
        .push("replacement release failed".into());
    case.finish();
    let json = serde_json::to_value(&case).unwrap();
    assert!(!case.completed);
    assert_eq!(json["error"], "primary Windows name mismatch");
    assert_eq!(json["cleanup_errors"].as_array().unwrap().len(), 2);

    let mut cleanup_only = completed_case();
    cleanup_only.cleanup_errors.push("release failed".into());
    cleanup_only.finish();
    assert!(!cleanup_only.completed && cleanup_only.error.is_some());

    // The existing default report schema acquires no rename result/claim.
    let default_report = serde_json::to_value(crate::report::Report::default()).unwrap();
    assert!(default_report.get("rename").is_none());
}
