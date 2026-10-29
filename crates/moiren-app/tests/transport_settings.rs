use moiren_app::project::AppPreferences;
use moiren_app::service::{DeliveryProvenance, ExpertInputBuffer, TransportLatencySettings};
use serde_json::json;

#[test]
fn expert_transport_preferences_round_trip_without_entering_engine_audio_settings() {
    let mut saved = serde_json::to_value(AppPreferences::default()).unwrap();
    let transport = json!({
        "process": {
            "capacity_frames": 8192,
            "target_frames": 2048,
            "jitter_frames": 96,
            "max_packet_frames": 480,
            "max_gap_frames": 480,
            "provenance": "declared"
        },
        "vad": {
            "capacity_frames": 4096,
            "target_frames": 1024,
            "jitter_frames": 64,
            "max_packet_frames": 256,
            "max_gap_frames": 256,
            "provenance": "empirical"
        }
    });
    saved["transport_latency"] = transport.clone();
    let preferences: AppPreferences = serde_json::from_value(saved).unwrap();
    preferences.validate().unwrap();
    let restored = serde_json::to_value(preferences).unwrap();
    assert_eq!(restored["transport_latency"], transport);
    assert!(restored["audio"].get("transport_latency").is_none());
}

#[test]
fn old_preferences_default_to_compatible_input_buffers() {
    let preferences: AppPreferences = serde_json::from_str(r#"{"version":2}"#).unwrap();
    let saved = serde_json::to_value(preferences).unwrap();
    assert_eq!(
        saved["transport_latency"],
        json!({"process": null, "vad": null})
    );
}

#[test]
fn vad_recovery_cap_persists_independently_of_expert_input_policy() {
    for cap in 1..=8 {
        let mut saved = serde_json::to_value(AppPreferences::default()).unwrap();
        saved["transport_latency"]["vad_recovery_cap_periods"] = json!(cap);
        let preferences: AppPreferences = serde_json::from_value(saved).unwrap();
        preferences.validate().unwrap();
        let restored = serde_json::to_value(&preferences).unwrap();
        assert_eq!(
            restored["transport_latency"]["vad_recovery_cap_periods"],
            cap
        );
        assert!(restored["transport_latency"]["vad"].is_null());
        assert!(restored["audio"].get("vad_recovery_cap_periods").is_none());
        let context = moiren_app::project::StartupDocuments {
            preferences,
            project: None,
        }
        .into_context()
        .unwrap();
        assert_eq!(
            serde_json::to_value(context.transport_latency).unwrap()["vad_recovery_cap_periods"],
            cap
        );
        assert_eq!(context.active_transport_latency, None);
    }
}

#[test]
fn vad_recovery_cap_rejects_unsupported_values_instead_of_clamping() {
    for cap in [json!(0), json!(9), json!(u64::MAX)] {
        let mut saved = serde_json::to_value(TransportLatencySettings::default()).unwrap();
        saved["vad_recovery_cap_periods"] = cap;
        let settings: TransportLatencySettings = serde_json::from_value(saved).unwrap();
        let error = settings.validate().unwrap_err();
        assert!(error.contains("VAD"), "{error}");
        assert!(error.contains("1..=8"), "{error}");
    }
    for cap in [json!(-1), json!(1.5), json!("1")] {
        let mut saved = serde_json::to_value(TransportLatencySettings::default()).unwrap();
        saved["vad_recovery_cap_periods"] = cap;
        assert!(serde_json::from_value::<TransportLatencySettings>(saved).is_err());
    }
}

#[test]
fn invalid_transport_ranges_and_incomplete_delivery_evidence_are_rejected() {
    let valid = ExpertInputBuffer::default();
    for invalid in [
        ExpertInputBuffer {
            capacity_frames: 0,
            ..valid
        },
        ExpertInputBuffer {
            capacity_frames: u32::MAX,
            ..valid
        },
        ExpertInputBuffer {
            target_frames: 0,
            ..valid
        },
        ExpertInputBuffer {
            target_frames: 8192 - 480,
            ..valid
        },
        ExpertInputBuffer {
            jitter_frames: u32::MAX,
            ..valid
        },
        ExpertInputBuffer {
            max_packet_frames: 0,
            ..valid
        },
        ExpertInputBuffer {
            max_gap_frames: 0,
            ..valid
        },
        ExpertInputBuffer {
            max_gap_frames: 8192,
            ..valid
        },
        ExpertInputBuffer {
            target_frames: 480 + 96,
            ..valid
        },
    ] {
        for (process, vad) in [(Some(invalid), None), (None, Some(invalid))] {
            let preferences = AppPreferences {
                transport_latency: TransportLatencySettings {
                    process,
                    vad,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(preferences.validate().is_err(), "{invalid:?}");
        }
    }
    assert!(serde_json::from_value::<ExpertInputBuffer>(json!({"target_frames": 128})).is_err());
    let mut evidence = serde_json::to_value(valid).unwrap();
    evidence["provenance"] = json!("negotiated_hardware");
    assert!(serde_json::from_value::<ExpertInputBuffer>(evidence).is_err());
}

#[test]
fn startup_preserves_both_transport_requests_without_inventing_an_active_owner() {
    let transport = TransportLatencySettings {
        process: Some(ExpertInputBuffer::default()),
        vad: Some(ExpertInputBuffer {
            provenance: DeliveryProvenance::Empirical,
            ..Default::default()
        }),
        vad_recovery_cap_periods: Some(2),
    };
    let context = moiren_app::project::StartupDocuments {
        preferences: AppPreferences {
            transport_latency: transport,
            ..Default::default()
        },
        project: None,
    }
    .into_context()
    .unwrap();
    assert_eq!(context.transport_latency, transport);
    assert_eq!(context.active_transport_latency, None);
    assert_eq!(context.audio_settings, Default::default());
}

#[test]
fn process_age_intent_persists_without_claiming_an_active_or_measurable_age() {
    let mut saved = serde_json::to_value(AppPreferences::default()).unwrap();
    let mut input = serde_json::to_value(ExpertInputBuffer::default()).unwrap();
    input["age_limit_100ns"] = json!(100_000);
    saved["transport_latency"]["process"] = input;
    let preferences: AppPreferences = serde_json::from_value(saved).unwrap();
    preferences.validate().unwrap();
    let restored = serde_json::to_value(&preferences).unwrap();
    assert_eq!(
        restored["transport_latency"]["process"]["age_limit_100ns"],
        100_000
    );
    let context = moiren_app::project::StartupDocuments {
        preferences,
        project: None,
    }
    .into_context()
    .unwrap();
    assert_eq!(context.active_transport_latency, None);
    assert_eq!(
        serde_json::to_value(context.transport_latency).unwrap()["process"]["age_limit_100ns"],
        100_000
    );
}

#[test]
fn process_age_intent_cannot_be_silently_used_as_a_vad_setting() {
    let mut requested = serde_json::to_value(TransportLatencySettings::default()).unwrap();
    let mut input = serde_json::to_value(ExpertInputBuffer::default()).unwrap();
    input["age_limit_100ns"] = json!(100_000);
    requested["vad"] = input;
    let settings: TransportLatencySettings = serde_json::from_value(requested).unwrap();
    let error = settings.validate().unwrap_err();
    assert!(error.contains("VAD"), "{error}");
    assert!(error.contains("Process"), "{error}");
}
