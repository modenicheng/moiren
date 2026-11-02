//! Control-side projection of retained Playback decode-call envelopes.
//! This is neither packet arrival nor a carrier publication/delivery witness.

use super::{LiveOptions, NativeReport, bridge_options};
use moiren_windows_audio::latency::{
    qualification::EvidenceSourceKind, transport::InputDeliveryTiming,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Comparison {
    NotApplicable,
    NoDeclaredBound,
    Unavailable,
    Inconclusive,
    ObservedRoundedEnvelopeWithinBound,
    ObservedRoundedEnvelopeExceedsBound,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RoundedEnvelopeComparison {
    WithinBound,
    ExceedsBound,
}

#[derive(Debug, Serialize)]
pub(super) struct HostDecodeCallGapDiagnostic {
    scope: &'static str,
    snapshot_scope: &'static str,
    declared_gap: Option<InputDeliveryTiming>,
    declared_jitter_frames: Option<u32>,
    publication: Option<Value>,
    comparison: Comparison,
    rounded_envelope_comparison: Option<RoundedEnvelopeComparison>,
    reasons: Vec<&'static str>,
    complete_delivery_coverage: Option<bool>,
    carrier_publication_effect_known: Option<bool>,
}

pub(super) fn project(options: &LiveOptions, native: &NativeReport) -> HostDecodeCallGapDiagnostic {
    let applicable = options.source_kind == EvidenceSourceKind::VadPlayback;
    let declared_gap = applicable
        .then(|| {
            bridge_options(options)
                .expect("validated live options")
                .delivery
        })
        .flatten();
    let publication = applicable
        .then(|| {
            native
                .vad_provider
                .as_ref()?
                .pointer("/telemetry/playback_publication")
                .filter(|value| !value.is_null())
                .cloned()
        })
        .flatten();
    let mut report = HostDecodeCallGapDiagnostic {
        scope: "previous successful decode-call before -> current successful call after; maximum rounded UP to 100ns, not start-to-start, native arrival, carrier publication, source age or physical latency; exceeding the declared gap does not prove a supply gap, and within does not certify delivery",
        snapshot_scope: "last retained running Playback provider snapshot, cumulative for its original owner; excludes separate teardown snapshots and cannot certify a later subwindow or prefix/suffix",
        declared_gap,
        declared_jitter_frames: declared_gap.map(|_| options.input_jitter_frames),
        publication,
        comparison: Comparison::Unavailable,
        rounded_envelope_comparison: None,
        reasons: Vec::new(),
        complete_delivery_coverage: None,
        carrier_publication_effect_known: None,
    };
    if !applicable {
        report.comparison = Comparison::NotApplicable;
        return report;
    }
    let Some(bound) = declared_gap else {
        report.comparison = Comparison::NoDeclaredBound;
        return report;
    };
    let Some(snapshot) = report.publication.as_ref() else {
        report.reasons.push("missing_playback_publication_snapshot");
        return report;
    };
    let number = |key: &str| snapshot.get(key).and_then(Value::as_u64);
    let maximum = number("max_call_interval_100ns");
    if bound.sample_rate == 0 || bound.max_gap_frames == 0 {
        report.reasons.push("invalid_declared_gap");
    }
    if number("successful_decode_calls").is_none_or(|n| n < 2)
        || number("valid_call_intervals").is_none_or(|n| n == 0)
        || maximum.is_none_or(|n| n == 0)
        || number("qpc_frequency_hz").is_none_or(|n| n == 0)
    {
        report
            .reasons
            .push("missing_or_insufficient_call_intervals");
    }
    if !report.reasons.is_empty() {
        return report;
    }
    // The source maximum is an upward-rounded upper envelope. Use exact
    // rational arithmetic, keep jitter separate, and never infer a fault count.
    let exceeds = u128::from(maximum.unwrap()) * u128::from(bound.sample_rate)
        > u128::from(bound.max_gap_frames) * 10_000_000;
    report.rounded_envelope_comparison = Some(if exceeds {
        RoundedEnvelopeComparison::ExceedsBound
    } else {
        RoundedEnvelopeComparison::WithinBound
    });
    let provider = native.vad_provider.as_ref().unwrap();
    let identity = snapshot.get("identity");
    let revision = native
        .engine
        .as_ref()
        .and_then(|engine| engine.get("active_revision"))
        .and_then(Value::as_u64);
    let stamp = provider.pointer("/snapshot/stamp").filter(|stamp| {
        stamp.as_array().is_some_and(|values| {
            values.len() == 5 && values.iter().all(|value| value.as_u64().is_some())
        })
    });
    if provider.get("endpoint_id").and_then(Value::as_str) != options.stable_endpoint_id.as_deref()
        || options.stable_endpoint_id.is_none()
        || provider.get("intent").and_then(Value::as_str) != Some("Playback")
        || provider
            .get("diagnostic_provenance")
            .and_then(Value::as_str)
            != Some("Fresh")
        || stamp.is_none()
        || identity.and_then(|id| id.get("native_stamp")) != stamp
        || revision.is_none_or(|n| n == 0)
        || identity
            .and_then(|id| id.get("active_revision"))
            .and_then(Value::as_u64)
            != revision
        || identity
            .and_then(|id| id.get("admission_ticket"))
            .and_then(Value::as_u64)
            .is_none_or(|n| n == 0)
        || identity.and_then(|id| id.get("packet_frames"))
            != provider.pointer("/snapshot/packet_frames")
        || provider
            .pointer("/snapshot/packet_frames")
            .and_then(Value::as_u64)
            .is_none_or(|n| n == 0)
    {
        report
            .reasons
            .push("missing_or_mismatched_running_owner_identity");
    }
    for key in [
        "carrier_full_packets",
        "decode_error_packets",
        "stale_packets",
        "missing_sequence_slots",
        "ambiguous_packets",
        "unavailable_clocks",
        "regressed_clocks",
        "conversion_overflows",
        "segment_breaks",
        "owner_reopens",
    ] {
        if number(key) != Some(0) {
            report.reasons.push(key);
        }
    }
    for key in ["invalidated", "saturated"] {
        if snapshot.get(key).and_then(Value::as_bool) != Some(false) {
            report.reasons.push(key);
        }
    }
    if number("valid_call_intervals").and_then(|n| n.checked_add(1))
        != number("successful_decode_calls")
    {
        report.reasons.push("incomplete_success_interval_sequence");
    }
    report.comparison = if !report.reasons.is_empty() {
        Comparison::Inconclusive
    } else if exceeds {
        Comparison::ObservedRoundedEnvelopeExceedsBound
    } else {
        Comparison::ObservedRoundedEnvelopeWithinBound
    };
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(maximum: u64) -> (LiveOptions, NativeReport) {
        let options = LiveOptions {
            source_kind: EvidenceSourceKind::VadPlayback,
            stable_endpoint_id: Some("playback".into()),
            input_target_frames: Some(2048),
            ..LiveOptions::default()
        };
        let provider = json!({
            "endpoint_id": "playback", "intent": "Playback",
            "diagnostic_provenance": "Fresh",
            "snapshot": {"stamp": [40, 2901948765373_u64, 40, 6, 16], "packet_frames": 480},
            "telemetry": {"playback_publication": {
                "identity": {"native_stamp": [40, 2901948765373_u64, 40, 6, 16],
                    "reset_epoch": 0, "admission_ticket": 2, "active_revision": 1,
                    "format": 2, "channels": 2, "packet_frames": 480},
                "qpc_frequency_hz": 10_000_000, "successful_decode_calls": 2050,
                "decode_ok_frames": 984000, "valid_call_intervals": 2049,
                "max_call_interval_100ns": maximum,
                "first_sequence": 1, "last_sequence": 2050,
                "first_before_qpc": 2901981357351_u64,
                "last_after_qpc": 2902186260207_u64,
                "carrier_full_packets": 0, "decode_error_packets": 0,
                "stale_packets": 0, "missing_sequence_slots": 0, "ambiguous_packets": 0,
                "unavailable_clocks": 0, "regressed_clocks": 0, "conversion_overflows": 0,
                "segment_breaks": 0, "owner_reopens": 0, "invalidated": false,
                "saturated": false, "complete_delivery_coverage": null,
                "carrier_publication_effect_known": null
            }}
        });
        (
            options,
            NativeReport {
                vad_provider: Some(provider),
                engine: Some(json!({"active_revision": 1})),
                ..NativeReport::default()
            },
        )
    }

    fn snapshot(native: &mut NativeReport) -> &mut Value {
        native
            .vad_provider
            .as_mut()
            .unwrap()
            .pointer_mut("/telemetry/playback_publication")
            .unwrap()
    }

    fn projected(options: &LiveOptions, native: &NativeReport) -> Value {
        serde_json::to_value(project(options, native)).unwrap()
    }

    #[test]
    fn retained_compatible_maximum_is_not_a_measured_delivery_bound_or_violation_count() {
        let (mut options, native) = fixture(106_424);
        let value = projected(&options, &native);
        assert_eq!(
            value["comparison"],
            "observed_rounded_envelope_exceeds_bound"
        );
        assert_eq!(value["rounded_envelope_comparison"], "exceeds_bound");
        assert_eq!(value["declared_gap"]["source"], "declared");
        assert_eq!(value["declared_jitter_frames"], 96);
        assert_eq!(value["publication"]["max_call_interval_100ns"], 106_424);
        assert!(value["complete_delivery_coverage"].is_null());
        assert!(value["carrier_publication_effect_known"].is_null());
        assert_eq!(super::super::native_delivery_violations(&native), 0);
        options.input_target_frames = None;
        let value = projected(&options, &native);
        assert_eq!(value["comparison"], "no_declared_bound");
        assert!(value["declared_gap"].is_null());
        assert!(value["rounded_envelope_comparison"].is_null());
        assert_eq!(value["publication"]["max_call_interval_100ns"], 106_424);
    }

    #[test]
    fn exact_boundary_and_next_100ns_are_compared_without_jitter_or_overflow() {
        for (gap, maximum, expected) in [
            (480, 100_000, "within_bound"),
            (480, 100_001, "exceeds_bound"),
            // 64 frames = 13333.333... x 100ns: the UP-rounded maximum
            // can exceed while the underlying envelope need not exceed.
            (64, 13_334, "exceeds_bound"),
            (u32::MAX, u64::MAX, "exceeds_bound"),
        ] {
            let (mut options, native) = fixture(maximum);
            options.max_gap_frames = gap;
            let value = projected(&options, &native);
            assert_eq!(value["rounded_envelope_comparison"], expected);
            assert_eq!(
                value["comparison"],
                format!("observed_rounded_envelope_{expected}")
            );
            assert!(value["complete_delivery_coverage"].is_null());
        }
    }

    #[test]
    fn missing_or_insufficient_measurement_remains_unavailable() {
        for (key, value) in [
            ("qpc_frequency_hz", Value::Null),
            ("qpc_frequency_hz", json!(0)),
            ("max_call_interval_100ns", Value::Null),
            ("max_call_interval_100ns", json!(0)),
            ("successful_decode_calls", json!(0)),
            ("successful_decode_calls", json!(1)),
            ("valid_call_intervals", json!(0)),
        ] {
            let (options, mut native) = fixture(100_000);
            snapshot(&mut native)[key] = value;
            let value = projected(&options, &native);
            assert_eq!(value["comparison"], "unavailable", "{key}");
            assert!(value["rounded_envelope_comparison"].is_null());
        }
        let (options, mut native) = fixture(100_000);
        native.vad_provider = None;
        assert_eq!(projected(&options, &native)["comparison"], "unavailable");
    }

    #[test]
    fn bad_or_missing_evidence_retains_numerical_comparison_but_never_within_verdict() {
        for key in [
            "carrier_full_packets",
            "decode_error_packets",
            "stale_packets",
            "missing_sequence_slots",
            "ambiguous_packets",
            "unavailable_clocks",
            "regressed_clocks",
            "conversion_overflows",
            "segment_breaks",
            "owner_reopens",
            "invalidated",
            "saturated",
        ] {
            for missing in [false, true] {
                let (options, mut native) = fixture(100_000);
                let value = snapshot(&mut native);
                if missing {
                    value.as_object_mut().unwrap().remove(key);
                } else {
                    value[key] = if ["invalidated", "saturated"].contains(&key) {
                        json!(true)
                    } else {
                        json!(1)
                    };
                }
                let value = projected(&options, &native);
                assert_eq!(value["comparison"], "inconclusive", "{key}");
                assert_eq!(value["rounded_envelope_comparison"], "within_bound");
                assert!(value["reasons"].as_array().unwrap().contains(&json!(key)));
                assert!(value["complete_delivery_coverage"].is_null());
            }
        }
    }

    #[test]
    fn different_owner_or_revision_cannot_authorize_within_verdict() {
        for pointer in [
            "/endpoint_id",
            "/intent",
            "/diagnostic_provenance",
            "/snapshot/stamp/4",
            "/telemetry/playback_publication/identity/native_stamp/4",
            "/telemetry/playback_publication/identity/active_revision",
            "/telemetry/playback_publication/identity/admission_ticket",
            "/telemetry/playback_publication/identity/packet_frames",
        ] {
            let (options, mut native) = fixture(100_000);
            *native
                .vad_provider
                .as_mut()
                .unwrap()
                .pointer_mut(pointer)
                .unwrap() = Value::Null;
            assert_eq!(
                projected(&options, &native)["comparison"],
                "inconclusive",
                "{pointer}"
            );
        }
        let (options, mut native) = fixture(100_000);
        native.engine = None;
        assert_eq!(projected(&options, &native)["comparison"], "inconclusive");
    }

    #[test]
    fn failed_call_envelope_and_non_10mhz_clock_do_not_reinterpret_recorded_units() {
        let (options, mut native) = fixture(106_424);
        snapshot(&mut native)["qpc_frequency_hz"] = json!(3_579_545);
        assert_eq!(
            projected(&options, &native)["comparison"],
            "observed_rounded_envelope_exceeds_bound"
        );
        snapshot(&mut native)["carrier_full_packets"] = json!(1);
        let value = projected(&options, &native);
        assert_eq!(value["comparison"], "inconclusive");
        assert_eq!(value["publication"]["max_call_interval_100ns"], 106_424);
        assert_eq!(value["rounded_envelope_comparison"], "exceeds_bound");
        snapshot(&mut native)["carrier_full_packets"] = json!(0);
        snapshot(&mut native)["valid_call_intervals"] = json!(2048);
        assert_eq!(projected(&options, &native)["comparison"], "inconclusive");
    }

    #[test]
    fn microphone_and_process_telemetry_are_not_playback_decode_evidence() {
        let (mut options, mut native) = fixture(100_000);
        native.vad_microphone_provider = native.vad_provider.take();
        assert_eq!(projected(&options, &native)["comparison"], "unavailable");
        options.source_kind = EvidenceSourceKind::ProcessLoopback;
        let value = projected(&options, &native);
        assert_eq!(value["comparison"], "not_applicable");
        assert!(value["publication"].is_null());
    }
}
