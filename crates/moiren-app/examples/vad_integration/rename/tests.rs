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
        windows_name: Some("Windows old label".into()),
        windows_name_identity: Some([10, 7, 11]),
        windows_name_mmdevice_id: Some("mmdevice".into()),
        windows_name_error: None,
        rename_pending: false,
        resource: "present".into(),
        observation_generation: Some(40),
        native_stamp: Some([10, 7, 11, 3, 5]),
        native_retirement: None,
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
        device_desc: PropertyReadback::Value {
            value: "Old description".into(),
        },
        device_interface_friendly_name: PropertyReadback::Value {
            value: "Adapter label".into(),
        },
        topology: None,
    }
}

#[test]
fn opaque_topology_and_setup_rejection_are_optional_diagnostics_not_gate_authority() {
    let opaque = r"{2}.\\?\Mixed#Opaque Adapter ";
    let diagnostic = TopologyEvidence {
        adapter_id: TopologyRead::Value {
            value: opaque.into(),
        },
        setup_interface: Some(SetupInterfaceEvidence::Rejected {
            hresult: -2147024809,
        }),
    };
    let mut before = name("Playback");
    before.topology = Some(diagnostic.clone());
    let mut after = name("Playback");
    after.topology = Some(TopologyEvidence {
        adapter_id: TopologyRead::Error {
            error: TopologyReadError::Hresult {
                stage: "GetDeviceIdConnectedTo",
                hresult: -2147467259,
            },
        },
        setup_interface: None,
    });
    same_held_device(&before, &after).unwrap();
    assert!(windows_name(&before, &after, "new").is_err());
    after.windows_friendly_name = "new".into();
    windows_name(&before, &after, "new").unwrap();
    let json = serde_json::to_value(diagnostic).unwrap();
    assert_eq!(json["adapter_id"]["value"], opaque);
    assert_eq!(json["setup_interface"]["status"], "rejected");
    assert_eq!(json["setup_interface"]["hresult"], -2147024809i32);
    let json = serde_json::to_value(after).unwrap();
    assert_eq!(
        json["topology"]["adapter_id"]["error"]["stage"],
        "GetDeviceIdConnectedTo"
    );
    assert_eq!(
        json["topology"]["adapter_id"]["error"]["hresult"],
        -2147467259i32
    );
    assert!(json["topology"]["setup_interface"].is_null());
    assert!(
        serde_json::to_value(name("Playback"))
            .unwrap()
            .get("topology")
            .is_none()
    );
}

#[test]
fn setup_interface_metadata_retains_independent_read_errors_and_raw_property_shape() {
    let evidence = SetupInterfaceEvidence::Opened {
        interface_class: "6994ad04-93ef-11d0-a3cc-00a0c9223196".into(),
        pdo_instance: TopologyRead::Value {
            value: r"MOIREN\VAD_RAW\owned".into(),
        },
        parent_instance: TopologyRead::Error {
            error: TopologyReadError::ConfigRet {
                stage: "CM_Get_Parent",
                code: 13,
            },
        },
        friendly_name: TypedInterfaceName {
            property_type: Some(25),
            required_bytes: Some(20),
            readback: TopologyRead::Error {
                error: TopologyReadError::PropertyType { property_type: 25 },
            },
        },
    };
    let json = serde_json::to_value(evidence).unwrap();
    assert_eq!(json["status"], "opened");
    assert_eq!(json["pdo_instance"]["value"], r"MOIREN\VAD_RAW\owned");
    assert_eq!(json["parent_instance"]["error"]["code"], 13);
    assert_eq!(json["friendly_name"]["property_type"], 25);
    assert_eq!(json["friendly_name"]["required_bytes"], 20);
    assert_eq!(
        json["friendly_name"]["readback"]["error"]["kind"],
        "property_type"
    );
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
            "Requested new name (Moiren virtual audio endpoint)",
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
    case.direct_removed_verified = true;
    case.old_removed = Some(RetirementEvidence {
        generation: 41,
        native_stamp: [10, 7, 11, 3, 5],
    });
    case.diagnostic_completed = true;
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
        |e| e.direct_removed_verified = false,
        |e| e.old_removed = None,
        |e| e.diagnostic_completed = false,
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

#[test]
fn diagnostic_continuation_keeps_strict_failure_and_still_releases_initializes_and_reads_name() {
    for direction in ["Playback", "Microphone"] {
        let mut case = Case::new(direction.into(), "new".into());
        case.held_client_initialized = true;
        let mut emit = |_: &Case| {};
        let old = baseline(direction);
        let mut absent = old.clone();
        absent.requested_name = "new".into();
        absent.resource = "absent".into();
        absent.provider_name = None;
        absent.rename_pending = true;
        absent.candidate_revision = 9;
        absent.consumed_request_id = 4;
        absent.observation_generation = None;
        absent.native_stamp = None;
        step(&mut case, Phase::StrictHeld, &mut emit, |case, _| {
            case.held = Some(absent.clone());
            held(&old, &absent, "new", 4)
        });
        assert_eq!(
            case.error.as_deref(),
            Some("missing current observation generation")
        );
        continue_after_held(
            &mut case,
            &mut emit,
            |case, _| {
                case.held_client_released = true;
                Ok(())
            },
            |case, emit| {
                assert!(case.held_client_released);
                step(case, Phase::ReplacementNativeInit, emit, |case, _| {
                    case.replacement_client_initialized = true;
                    case.after = Some(name(direction));
                    Ok(())
                });
                step(case, Phase::ExactWindowsName, emit, |case, _| {
                    windows_name(&name(direction), case.after.as_ref().unwrap(), "new")
                });
                Ok(())
            },
        );
        assert!(case.held_client_released && case.replacement_client_initialized);
        assert!(case.diagnostic_completed);
        assert!(case.after.is_some());
        assert_eq!(
            case.error.as_deref(),
            Some("missing current observation generation")
        );
        assert_eq!(case.acceptance_failures.len(), 2);
        assert_eq!(case.acceptance_failures[0].phase, Phase::StrictHeld);
        assert_eq!(case.acceptance_failures[1].phase, Phase::ExactWindowsName);
        assert!(!case.qualified && case.status == "unqualified");
        // Even all later successful evidence/cleanup cannot upgrade this run.
        case.direct_removed_verified = true;
        case.old_removed = Some(RetirementEvidence {
            generation: 41,
            native_stamp: [10, 7, 11, 3, 5],
        });
        case.replacement_client_released = true;
        case.fresh_replacement_confirmed = true;
        case.windows_name_verified = true;
        case.finish();
        let json = serde_json::to_value(&case).unwrap();
        assert!(!case.completed && !case.qualified);
        assert_eq!(json["status"], "unqualified");
        assert_eq!(json["acceptance_failures"][0]["phase"], "strict_held");
        assert_eq!(
            case.error.as_deref(),
            Some("missing current observation generation")
        );
    }
}

#[test]
fn diagnostic_progress_is_bounded_and_cannot_erase_the_first_failure_for_a_phase() {
    let mut case = Case::new("Playback".into(), "new".into());
    let mut emit = |_: &Case| {};
    step(
        &mut case,
        Phase::StrictHeld,
        &mut emit,
        |_, _| -> anyhow::Result<()> { anyhow::bail!("original strict failure") },
    );
    for _ in 0..100 {
        step(&mut case, Phase::StrictHeld, &mut emit, |_, _| Ok(()));
        step(
            &mut case,
            Phase::StrictHeld,
            &mut emit,
            |_, _| -> anyhow::Result<()> { anyhow::bail!("later failure") },
        );
    }
    assert_eq!(case.error.as_deref(), Some("original strict failure"));
    assert_eq!(case.acceptance_failures.len(), 1);
    assert_eq!(case.acceptance_failures[0].error, "original strict failure");
    assert_eq!(case.progress.len(), 1);
    assert_eq!(case.progress[0].status, "failed");
}

#[test]
fn native_release_error_is_retained_but_continuation_requires_actual_release() {
    for released in [false, true] {
        let mut case = Case::new("Microphone".into(), "new".into());
        case.held_client_initialized = true;
        let mut emit = |_: &Case| {};
        continue_after_held(
            &mut case,
            &mut emit,
            |case, _| {
                case.held_client_released = released;
                case.cleanup_errors.push("native release error".into());
                anyhow::bail!("native release error")
            },
            |case, _| {
                case.replacement_client_initialized = true;
                Ok(())
            },
        );
        assert_eq!(case.replacement_client_initialized, released);
        assert_eq!(case.cleanup_errors, ["native release error"]);
        assert_eq!(case.error.as_deref(), Some("native release error"));
    }
}

#[test]
fn successful_diagnostic_replacement_never_substitutes_for_direct_old_removed() {
    let mut case = completed_case();
    case.direct_removed_verified = false;
    case.old_removed = None;
    case.finish();
    assert!(!case.completed && !case.qualified);
    assert_eq!(case.status, "unqualified");
    assert!(!case.direct_removed_verified);
    assert!(
        case.acceptance_failures
            .iter()
            .any(|f| f.phase == Phase::DirectRemoved)
    );
    assert!(
        case.error
            .as_deref()
            .is_some_and(|e| e.contains("direct old-Removed"))
    );
}

#[test]
fn direct_removed_requires_exact_old_identity_fresh_generation_and_nonregressing_epochs() {
    for direction in ["Playback", "Microphone"] {
        let old = baseline(direction);
        let valid = RetirementEvidence {
            generation: 41,
            native_stamp: [10, 7, 11, 3, 5],
        };
        direct_removed(&old, Some(&valid)).unwrap();
        assert!(direct_removed(&old, None).is_err());
        let mutations: &[fn(&mut RetirementEvidence)] = &[
            |e| e.generation = 40,
            |e| e.generation = 39,
            |e| e.native_stamp[0] = 12,
            |e| e.native_stamp[1] = 8,
            |e| e.native_stamp[2] = 13,
            |e| e.native_stamp[3] = 2,
            |e| e.native_stamp[4] = 4,
        ];
        for mutate in mutations {
            let mut invalid = valid;
            mutate(&mut invalid);
            assert!(direct_removed(&old, Some(&invalid)).is_err(), "{invalid:?}");
        }
        let advanced = RetirementEvidence {
            generation: 45,
            native_stamp: [10, 7, 11, 9, 10],
        };
        direct_removed(&old, Some(&advanced)).unwrap();
        let mut missing_baseline = old.clone();
        missing_baseline.native_stamp = None;
        assert!(direct_removed(&missing_baseline, Some(&valid)).is_err());
        missing_baseline = old.clone();
        missing_baseline.observation_generation = None;
        assert!(direct_removed(&missing_baseline, Some(&valid)).is_err());
        missing_baseline = old.clone();
        missing_baseline.native_stamp = Some([0, 7, 11, 3, 5]);
        assert!(direct_removed(&missing_baseline, Some(&valid)).is_err());
    }
}

#[test]
fn absent_and_fresh_replacement_stamp_are_not_direct_removed_evidence() {
    let old = baseline("Playback");
    let mut current = old.clone();
    current.resource = "absent".into();
    current.native_stamp = None;
    current.observation_generation = None;
    assert!(direct_removed(&old, current.native_retirement.as_ref()).is_err());
    current.resource = "present".into();
    current.native_stamp = Some([12, 7, 13, 9, 10]);
    current.observation_generation = Some(45);
    assert!(direct_removed(&old, current.native_retirement.as_ref()).is_err());
}

#[test]
fn service_retirement_maps_verbatim_to_serialized_example_evidence() {
    let native = moiren_app::service::ExportRetirementEvidence {
        generation: 43,
        native_stamp: [10, 7, 11, 4, 6],
    };
    let mut runtime = baseline("Microphone");
    runtime.native_retirement = Some(native.into());
    let json = serde_json::to_value(runtime).unwrap();
    assert_eq!(json["native_retirement"]["generation"], 43);
    assert_eq!(
        json["native_retirement"]["native_stamp"],
        serde_json::json!([10, 7, 11, 4, 6])
    );
}

#[test]
fn direction_budget_reserves_half_of_remaining_work_for_the_second_direction() {
    let started = Instant::now();
    for timeout in [60, 90, 120] {
        let work = Duration::from_secs(timeout).mul_f64(2.0 / 3.0);
        let global_deadline = started + work;
        let now = started + Duration::from_secs(2);
        let first_deadline = shared_deadline(now, global_deadline, 2);
        assert_eq!(first_deadline - now, (global_deadline - now) / 2);
        assert!(global_deadline - first_deadline >= Duration::from_secs(19));
        assert_eq!(
            shared_deadline(first_deadline, global_deadline, 1),
            global_deadline
        );
    }
}

#[test]
fn missing_direct_removed_wait_preserves_native_readback_and_second_direction_budget() {
    let now = Instant::now();
    for remaining in [
        Duration::from_secs(2),
        Duration::from_secs(20),
        Duration::from_secs(60),
    ] {
        let global_deadline = now + remaining;
        let first_deadline = shared_deadline(now, global_deadline, 2);
        let retired_at = retirement_deadline(now, first_deadline);
        assert!(retired_at - now <= Duration::from_millis(500));
        assert!(retired_at - now <= (first_deadline - now) / 4);
        // Simulate every asynchronous wait using its whole allotted share.
        // A missing tombstone stays failed, but cannot spend later phases' time.
        let present_at = shared_deadline(retired_at, first_deadline, 3);
        let initialized_at = shared_deadline(present_at, first_deadline, 2);
        assert!(now < retired_at && retired_at < present_at);
        assert!(present_at < initialized_at && initialized_at < first_deadline);
        assert!(first_deadline < global_deadline);
        assert!(first_deadline - initialized_at >= (first_deadline - retired_at) / 3);
    }
}

#[test]
fn exhausted_diagnostic_budget_never_extends_its_parent_deadline() {
    let deadline = Instant::now();
    for now in [deadline, deadline + Duration::from_secs(1)] {
        assert_eq!(shared_deadline(now, deadline, 2), deadline);
        assert_eq!(retirement_deadline(now, deadline), deadline);
    }
}

#[test]
fn actual_mmdevice_metadata_serializes_independently_in_before_and_after() {
    for direction in ["Playback", "Microphone"] {
        let mut case = Case::new(direction.into(), "new".into());
        case.before = Some(name(direction));
        let mut after = name(direction);
        after.device_desc = PropertyReadback::Value {
            value: "New description".into(),
        };
        after.device_interface_friendly_name = PropertyReadback::Value {
            value: "New adapter label".into(),
        };
        case.after = Some(after);
        let json = serde_json::to_value(&case).unwrap();
        assert_eq!(
            json["before"]["device_desc"],
            serde_json::json!({"status": "value", "value": "Old description"})
        );
        assert_eq!(json["after"]["device_desc"]["value"], "New description");
        assert_eq!(
            json["before"]["device_interface_friendly_name"]["value"],
            "Adapter label"
        );
        assert_eq!(
            json["after"]["device_interface_friendly_name"]["value"],
            "New adapter label"
        );
        assert_eq!(json["after"]["windows_friendly_name"], "Windows old label");
    }
}

#[test]
fn optional_mmdevice_errors_preserve_typed_failure_evidence_after_exact_name_failure() {
    let failures = [
        (
            PropertyReadError::OpenPropertyStore {
                hresult: -2147024891,
                message: "open denied".into(),
            },
            "open_property_store",
        ),
        (
            PropertyReadError::GetValue {
                hresult: -2147023728,
                message: "property unavailable".into(),
            },
            "get_value",
        ),
        (
            PropertyReadError::UnexpectedType { vartype: 0 },
            "unexpected_type",
        ),
        (PropertyReadError::NullString, "null_string"),
        (
            PropertyReadError::InvalidUtf16 {
                message: "unpaired surrogate".into(),
            },
            "invalid_utf16",
        ),
    ];
    for (error, kind) in failures {
        let mut case = Case::new("Playback".into(), "new".into());
        case.before = Some(name("Playback"));
        let mut after = name("Playback");
        after.device_desc = PropertyReadback::Error {
            error: error.clone(),
        };
        after.device_interface_friendly_name = PropertyReadback::Error {
            error: error.clone(),
        };
        case.after = Some(after);
        step(
            &mut case,
            Phase::ExactWindowsName,
            &mut |_: &Case| {},
            |case, _| {
                windows_name(
                    case.before.as_ref().unwrap(),
                    case.after.as_ref().unwrap(),
                    "new",
                )
            },
        );
        case.finish();
        let json = serde_json::to_value(&case).unwrap();
        assert_eq!(json["after"]["device_desc"]["status"], "error");
        assert_eq!(json["after"]["device_desc"]["error"]["kind"], kind);
        assert_eq!(
            json["after"]["device_interface_friendly_name"]["error"],
            serde_json::to_value(error).unwrap()
        );
        assert!(
            case.error
                .as_deref()
                .unwrap()
                .contains("Windows FriendlyName mismatch")
        );
        assert!(!case.windows_name_verified && !case.qualified);
        assert_eq!(json["after"]["windows_friendly_name"], "Windows old label");
    }
}

#[test]
fn optional_mmdevice_metadata_never_changes_held_or_exact_friendly_name_gates() {
    for direction in ["Playback", "Microphone"] {
        let before = name(direction);
        let mut current = before.clone();
        current.device_desc = PropertyReadback::Error {
            error: PropertyReadError::GetValue {
                hresult: -2147023728,
                message: "not available".into(),
            },
        };
        current.device_interface_friendly_name = PropertyReadback::Error {
            error: PropertyReadError::UnexpectedType { vartype: 0 },
        };
        same_held_device(&before, &current).unwrap();
        current.windows_friendly_name = "new".into();
        windows_name(&before, &current, "new").unwrap();
        current.device_desc = PropertyReadback::Value {
            value: "new".into(),
        };
        current.device_interface_friendly_name = PropertyReadback::Value {
            value: "new".into(),
        };
        current.windows_friendly_name = before.windows_friendly_name.clone();
        assert!(windows_name(&before, &current, "new").is_err());
        current.windows_friendly_name = "new (Adapter label)".into();
        assert!(windows_name(&before, &current, "new").is_err());
    }
}
