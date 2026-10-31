//! Driver QUERY counters can still advance after the last Running observation.

use super::*;

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
            &format!("retirement QUERY quality: {error:#}"),
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
        json!({"intent": intent, "endpoint_id": intent, "snapshot": {
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
}
