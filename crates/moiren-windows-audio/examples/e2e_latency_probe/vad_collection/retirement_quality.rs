//! Driver QUERY and worker counters can advance after the last Running observation.

use super::*;

fn worker_counters(
    before: &serde_json::Value,
    after: &serde_json::Value,
    intent: &str,
    faults: &mut Vec<String>,
) -> Result<()> {
    for view in [before, after] {
        ensure!(
            view.get("diagnostic_provenance")
                .and_then(serde_json::Value::as_str)
                == Some("Fresh")
                && view.get("error").is_some_and(serde_json::Value::is_null),
            "retired {intent} provider observation is stale, failed or unavailable"
        );
        let value = view
            .get("telemetry")
            .context("retired worker telemetry unavailable")?;
        ensure!(
            value
                .get("schema_version")
                .and_then(serde_json::Value::as_u64)
                == Some(5),
            "retired {intent} worker telemetry schema unavailable or unsupported"
        );
    }
    // Read required raw fields rather than the compatibility deserializer,
    // whose defaulted missing fields would manufacture zero-fault evidence.
    for key in [
        "dropped_packets",
        "driver_dropped_packets",
        "quota_dropped_packets",
        "carrier_dropped_packets",
        "stale_packets",
        "silence_packets",
        "silence_frames",
        "completion_clock_regressions",
        "sequence_gap_observations",
        "missing_sequence_slots",
        "epoch_resets",
        "recovery_events",
    ] {
        let counter = |view: &serde_json::Value| -> Result<u64> {
            view.get("telemetry")
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_u64)
                .filter(|count| *count != u64::MAX)
                .with_context(|| {
                    format!("retired {intent} worker {key} unavailable, malformed or saturated")
                })
        };
        let previous = counter(before)?;
        let final_count = counter(after)?;
        ensure!(
            final_count >= previous,
            "retired {intent} worker {key} regressed"
        );
        if matches!(key, "epoch_resets" | "recovery_events") {
            // Closing admission/native clients can advance lifecycle epochs.
            // Keep final totals raw; they do not identify audio faults by
            // themselves. A pre-stop fault must never disappear in retirement.
            if previous != 0 {
                faults.push(format!("last live {intent} worker {key}: {previous}"));
            }
        } else if final_count != 0 {
            faults.push(format!("retired {intent} worker {key}: {final_count}"));
        }
    }
    Ok(())
}

pub(super) fn validate(
    report: &super::super::NativeReport,
    route: super::super::vad_route::RouteKind,
    policy: ValidationPolicy,
    quality: &mut Quality,
) -> Result<()> {
    let result = (|| {
        let mut faults = Vec::new();
        let mut endpoints = vec![("Playback", &report.vad_provider, &report.vad_cleanup)];
        if route == super::super::vad_route::RouteKind::Microphone {
            endpoints.push((
                "Microphone",
                &report.vad_microphone_provider,
                &report.vad_microphone_cleanup,
            ));
        }
        for (intent, before, after) in endpoints {
            let before = before.as_ref().context("last provider QUERY unavailable")?;
            let after = after
                .as_ref()
                .context("retired provider QUERY unavailable")?;
            ensure!(
                before.get("intent").and_then(serde_json::Value::as_str) == Some(intent)
                    && after.get("intent").and_then(serde_json::Value::as_str) == Some(intent),
                "retired provider intent changed or unavailable"
            );
            let identity = |view: &serde_json::Value| -> Result<serde_json::Value> {
                let endpoint = view
                    .get("endpoint_id")
                    .filter(|id| !id.is_null())
                    .context("retired provider endpoint identity unavailable")?;
                let stamp = view
                    .pointer("/snapshot/stamp")
                    .and_then(serde_json::Value::as_array)
                    .filter(|stamp| {
                        stamp.len() == 5 && stamp.iter().all(|part| part.as_u64().is_some())
                    })
                    .context("retired provider stamp unavailable or malformed")?;
                // Native Stop may advance stream/gate epochs; PDO/session/export
                // identity must still belong to the same collected endpoint.
                Ok(serde_json::json!([endpoint, &stamp[..3]]))
            };
            ensure!(
                identity(before)? == identity(after)?,
                "retired provider endpoint identity changed"
            );
            ensure!(
                after
                    .pointer("/snapshot/state")
                    .and_then(serde_json::Value::as_str)
                    == Some("Removed"),
                "retired provider final Removed observation unavailable"
            );
            for key in ["silence_packets", "dropped_packets"] {
                let counter = |view: &serde_json::Value| -> Result<u64> {
                    view.get("snapshot")
                        .and_then(|snapshot| snapshot.get(key))
                        .and_then(serde_json::Value::as_u64)
                        .filter(|count| *count != u64::MAX)
                        .with_context(|| {
                            format!(
                                "retired {intent} QUERY {key} unavailable, malformed or saturated"
                            )
                        })
                };
                let previous = counter(before)?;
                let final_count = counter(after)?;
                ensure!(
                    final_count >= previous,
                    "retired {intent} QUERY {key} regressed"
                );
                if final_count != 0 {
                    faults.push(format!("retired {intent} QUERY {key}: {final_count}"));
                }
            }
            worker_counters(before, after, intent, &mut faults)?;
        }
        for message in &faults {
            record(quality, report, message);
        }
        // Correctness collection retains soft faults without relabelling them
        // as successful quality or a failed Stop/join operation.
        ensure!(
            faults.is_empty() || policy.is_correctness(),
            "{}",
            faults.join("; ")
        );
        Ok(())
    })();
    if let Err(error) = &result {
        record(
            quality,
            report,
            &format!("retirement provider quality: {error:#}"),
        );
    }
    result
}

fn record(quality: &mut Quality, report: &super::super::NativeReport, message: &str) {
    let first = quality.first_observations.is_none();
    quality.record(message, None, None);
    if first {
        quality.first_observations = Some(serde_json::json!({
            "observation": message,
            "provider": report.vad_provider,
            "microphone_provider": report.vad_microphone_provider,
            "playback_cleanup": report.vad_cleanup,
            "microphone_cleanup": report.vad_microphone_cleanup,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observation(intent: &str, silence: u64, dropped: u64) -> serde_json::Value {
        json!({"intent": intent, "endpoint_id": intent,
            "diagnostic_provenance": "Fresh", "error": null,
            "telemetry": {"schema_version": 5,
                "dropped_packets": 0, "driver_dropped_packets": 0,
                "quota_dropped_packets": 0, "carrier_dropped_packets": 0,
                "stale_packets": 0, "silence_packets": 0, "silence_frames": 0,
                "completion_clock_regressions": 0,
                "sequence_gap_observations": 0, "missing_sequence_slots": 0,
                "epoch_resets": 0, "recovery_events": 0},
            "snapshot": {
            "state": "Removed", "stamp": [1, 2, 3, 4, 5],
            "silence_packets": silence, "dropped_packets": dropped
        }})
    }

    fn report() -> super::super::super::NativeReport {
        super::super::super::NativeReport {
            vad_provider: Some(observation("Playback", 0, 0)),
            vad_microphone_provider: Some(observation("Microphone", 0, 0)),
            vad_cleanup: Some(observation("Playback", 0, 0)),
            vad_microphone_cleanup: Some(observation("Microphone", 0, 0)),
            ..Default::default()
        }
    }

    #[test]
    fn final_query_tail_faults_cannot_keep_quality_passed() {
        for microphone in [false, true] {
            for key in ["silence_packets", "dropped_packets"] {
                for policy in [
                    ValidationPolicy::default(),
                    super::super::tests::correctness(),
                ] {
                    let mut report = report();
                    let final_view = if microphone {
                        report.vad_microphone_cleanup.as_mut().unwrap()
                    } else {
                        report.vad_cleanup.as_mut().unwrap()
                    };
                    final_view["snapshot"][key] = json!(1);
                    let mut quality = Quality::default();
                    let result = validate(
                        &report,
                        super::super::super::vad_route::RouteKind::Microphone,
                        policy,
                        &mut quality,
                    );
                    assert_eq!(result.is_ok(), policy.is_correctness());
                    assert!(quality.failure.as_ref().unwrap().contains(key));
                    let retained = quality.first_observations.unwrap();
                    assert_eq!(
                        retained["microphone_cleanup"],
                        report.vad_microphone_cleanup.unwrap()
                    );
                    assert_eq!(retained["playback_cleanup"], report.vad_cleanup.unwrap());
                }
            }
        }
    }

    #[test]
    fn final_query_checks_fail_closed_for_missing_malformed_or_regressed_counters() {
        for key in ["silence_packets", "dropped_packets"] {
            for invalid in [json!(null), json!("0"), json!(-1), json!(u64::MAX)] {
                let mut report = report();
                report.vad_microphone_cleanup.as_mut().unwrap()["snapshot"][key] = invalid;
                assert!(
                    validate(
                        &report,
                        super::super::super::vad_route::RouteKind::Microphone,
                        super::super::tests::correctness(),
                        &mut Quality::default()
                    )
                    .is_err()
                );
            }
            let mut report = report();
            report.vad_microphone_provider.as_mut().unwrap()["snapshot"][key] = json!(2);
            assert!(
                validate(
                    &report,
                    super::super::super::vad_route::RouteKind::Microphone,
                    super::super::tests::correctness(),
                    &mut Quality::default()
                )
                .is_err()
            );
        }
        let mut report = report();
        report.vad_microphone_cleanup = None;
        assert!(
            validate(
                &report,
                super::super::super::vad_route::RouteKind::Microphone,
                super::super::tests::correctness(),
                &mut Quality::default()
            )
            .is_err()
        );
    }

    #[test]
    fn final_query_zero_counters_and_physical_route_do_not_invent_microphone_faults() {
        let mut report = report();
        for route in [
            super::super::super::vad_route::RouteKind::Microphone,
            super::super::super::vad_route::RouteKind::Physical,
        ] {
            if route == super::super::super::vad_route::RouteKind::Physical {
                report.vad_microphone_provider = None;
                report.vad_microphone_cleanup = None;
            }
            let mut quality = Quality::default();
            validate(&report, route, ValidationPolicy::default(), &mut quality).unwrap();
            assert!(quality.failure.is_none());
        }
    }

    #[test]
    fn final_query_does_not_subtract_existing_faults_or_replace_first_failure() {
        let mut report = report();
        report.vad_microphone_provider = Some(observation("Microphone", 3, 2));
        report.vad_microphone_cleanup = Some(observation("Microphone", 3, 2));
        let mut quality = Quality::default();
        quality.record("original live fault", None, None);
        let first = quality.first_observations.clone();
        validate(
            &report,
            super::super::super::vad_route::RouteKind::Microphone,
            super::super::tests::correctness(),
            &mut quality,
        )
        .unwrap();
        let failure = quality.failure.unwrap();
        assert!(failure.contains("silence_packets: 3"));
        assert!(failure.contains("dropped_packets: 2"));
        assert_eq!(quality.first_observations, first);
    }

    #[test]
    fn final_query_requires_same_export_but_allows_native_stop_epoch_advance() {
        let mut report = report();
        report.vad_microphone_cleanup.as_mut().unwrap()["snapshot"]["stamp"] =
            json!([1, 2, 3, 6, 7]);
        validate(
            &report,
            super::super::super::vad_route::RouteKind::Microphone,
            ValidationPolicy::default(),
            &mut Quality::default(),
        )
        .unwrap();
        for pointer in [
            "/endpoint_id",
            "/snapshot/stamp/0",
            "/snapshot/stamp/1",
            "/snapshot/stamp/2",
            "/snapshot/state",
        ] {
            let mut changed = self::report();
            *changed
                .vad_microphone_cleanup
                .as_mut()
                .unwrap()
                .pointer_mut(pointer)
                .unwrap() = json!(99);
            assert!(
                validate(
                    &changed,
                    super::super::super::vad_route::RouteKind::Microphone,
                    super::super::tests::correctness(),
                    &mut Quality::default()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn retired_worker_tail_faults_are_retained_under_both_routes_and_policies() {
        for microphone in [false, true] {
            for key in [
                "dropped_packets",
                "driver_dropped_packets",
                "quota_dropped_packets",
                "carrier_dropped_packets",
                "stale_packets",
                "silence_packets",
                "silence_frames",
                "completion_clock_regressions",
                "sequence_gap_observations",
                "missing_sequence_slots",
            ] {
                for policy in [
                    ValidationPolicy::default(),
                    super::super::tests::correctness(),
                ] {
                    let mut report = report();
                    let route = if microphone {
                        report.vad_microphone_cleanup.as_mut().unwrap()["telemetry"][key] =
                            json!(1);
                        super::super::super::vad_route::RouteKind::Microphone
                    } else {
                        report.vad_cleanup.as_mut().unwrap()["telemetry"][key] = json!(1);
                        report.vad_microphone_provider = None;
                        report.vad_microphone_cleanup = None;
                        super::super::super::vad_route::RouteKind::Physical
                    };
                    let original = serde_json::to_value(&report).unwrap();
                    let mut quality = Quality::default();
                    let result = validate(&report, route, policy, &mut quality);
                    assert_eq!(
                        result.is_ok(),
                        policy.is_correctness(),
                        "{microphone} {key}"
                    );
                    assert!(
                        quality
                            .failure
                            .as_deref()
                            .is_some_and(|text| text.contains(key))
                    );
                    let retained = quality.first_observations.as_ref().unwrap();
                    assert_eq!(retained["playback_cleanup"], original["vad_cleanup"]);
                    assert_eq!(
                        retained["microphone_cleanup"],
                        original["vad_microphone_cleanup"]
                    );
                    assert_eq!(serde_json::to_value(&report).unwrap(), original);
                }
            }
        }
    }

    #[test]
    fn retired_worker_missing_malformed_saturated_or_regressed_evidence_is_hard() {
        let keys = [
            "dropped_packets",
            "driver_dropped_packets",
            "quota_dropped_packets",
            "carrier_dropped_packets",
            "stale_packets",
            "silence_packets",
            "silence_frames",
            "completion_clock_regressions",
            "sequence_gap_observations",
            "missing_sequence_slots",
            "epoch_resets",
            "recovery_events",
        ];
        for before in [false, true] {
            for key in keys {
                for invalid in [
                    json!(null),
                    json!("0"),
                    json!(-1),
                    json!(0.5),
                    json!(true),
                    json!(u64::MAX),
                ] {
                    let mut report = report();
                    let view = if before {
                        &mut report.vad_microphone_provider
                    } else {
                        &mut report.vad_microphone_cleanup
                    };
                    view.as_mut().unwrap()["telemetry"][key] = invalid;
                    let mut quality = Quality::default();
                    assert!(
                        validate(
                            &report,
                            super::super::super::vad_route::RouteKind::Microphone,
                            super::super::tests::correctness(),
                            &mut quality
                        )
                        .is_err(),
                        "{before} {key}"
                    );
                    assert!(quality.failure.is_some());
                    assert!(quality.first_observations.is_some());
                }
                let mut report = report();
                let view = if before {
                    &mut report.vad_microphone_provider
                } else {
                    &mut report.vad_microphone_cleanup
                };
                view.as_mut().unwrap()["telemetry"]
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
                assert!(
                    validate(
                        &report,
                        super::super::super::vad_route::RouteKind::Microphone,
                        super::super::tests::correctness(),
                        &mut Quality::default()
                    )
                    .is_err(),
                    "{before} missing {key}"
                );
            }
            for invalid in [
                json!(null),
                json!({}),
                json!({"schema_version": 4}),
                json!({"schema_version": 6}),
            ] {
                let mut report = report();
                let view = if before {
                    &mut report.vad_microphone_provider
                } else {
                    &mut report.vad_microphone_cleanup
                };
                view.as_mut().unwrap()["telemetry"] = invalid;
                assert!(
                    validate(
                        &report,
                        super::super::super::vad_route::RouteKind::Microphone,
                        super::super::tests::correctness(),
                        &mut Quality::default()
                    )
                    .is_err()
                );
            }
        }
        for key in keys {
            let mut report = report();
            report.vad_microphone_provider.as_mut().unwrap()["telemetry"][key] = json!(2);
            report.vad_microphone_cleanup.as_mut().unwrap()["telemetry"][key] = json!(1);
            assert!(
                validate(
                    &report,
                    super::super::super::vad_route::RouteKind::Microphone,
                    super::super::tests::correctness(),
                    &mut Quality::default()
                )
                .is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn retired_worker_lifecycle_totals_do_not_erase_pre_stop_faults() {
        for key in ["epoch_resets", "recovery_events"] {
            for policy in [
                ValidationPolicy::default(),
                super::super::tests::correctness(),
            ] {
                let mut report = report();
                report.vad_microphone_provider.as_mut().unwrap()["telemetry"][key] = json!(1);
                report.vad_microphone_cleanup.as_mut().unwrap()["telemetry"][key] = json!(2);
                let mut quality = Quality::default();
                assert_eq!(
                    validate(
                        &report,
                        super::super::super::vad_route::RouteKind::Microphone,
                        policy,
                        &mut quality
                    )
                    .is_ok(),
                    policy.is_correctness()
                );
                assert!(
                    quality
                        .failure
                        .as_deref()
                        .is_some_and(|text| text.contains(key))
                );
            }
        }
    }

    #[test]
    fn retired_worker_requires_fresh_error_free_provider_views() {
        for before in [false, true] {
            for (key, value) in [
                ("diagnostic_provenance", json!("CachedAfterQueryFailure")),
                ("diagnostic_provenance", json!(null)),
                ("error", json!("QUERY failed")),
            ] {
                let mut report = report();
                let view = if before {
                    &mut report.vad_microphone_provider
                } else {
                    &mut report.vad_microphone_cleanup
                };
                view.as_mut().unwrap()[key] = value;
                assert!(
                    validate(
                        &report,
                        super::super::super::vad_route::RouteKind::Microphone,
                        super::super::tests::correctness(),
                        &mut Quality::default()
                    )
                    .is_err()
                );
            }
            for key in ["diagnostic_provenance", "error"] {
                let mut report = report();
                let view = if before {
                    &mut report.vad_microphone_provider
                } else {
                    &mut report.vad_microphone_cleanup
                };
                view.as_mut().unwrap().as_object_mut().unwrap().remove(key);
                assert!(
                    validate(
                        &report,
                        super::super::super::vad_route::RouteKind::Microphone,
                        super::super::tests::correctness(),
                        &mut Quality::default()
                    )
                    .is_err(),
                    "{before} missing {key}"
                );
            }
        }
    }

    #[test]
    fn retired_worker_keeps_raw_lifecycle_totals_and_original_first_failure() {
        let mut report = report();
        for view in [&mut report.vad_cleanup, &mut report.vad_microphone_cleanup] {
            let view = view.as_mut().unwrap();
            view["snapshot"]["stamp"] = json!([1, 2, 3, 4, 8]);
            view["telemetry"]["epoch_resets"] = json!(1);
            view["telemetry"]["recovery_events"] = json!(1);
        }
        let mut quality = Quality::default();
        validate(
            &report,
            super::super::super::vad_route::RouteKind::Microphone,
            ValidationPolicy::default(),
            &mut quality,
        )
        .unwrap();
        assert!(quality.failure.is_none());
        quality.record("original live fault", None, None);
        let first = quality.first_observations.clone();
        report.vad_microphone_cleanup.as_mut().unwrap()["telemetry"]["silence_frames"] = json!(32);
        validate(
            &report,
            super::super::super::vad_route::RouteKind::Microphone,
            super::super::tests::correctness(),
            &mut quality,
        )
        .unwrap();
        assert!(
            quality
                .failure
                .as_deref()
                .unwrap()
                .contains("silence_frames: 32")
        );
        assert_eq!(quality.first_observations, first);
        assert_eq!(
            report.vad_microphone_cleanup.as_ref().unwrap()["telemetry"]["epoch_resets"],
            1
        );
        assert_eq!(quality.boundary_evidence()["quality_complete"], false);
    }
}
