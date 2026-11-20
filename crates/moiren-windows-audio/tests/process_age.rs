use moiren_windows_audio::{
    latency::{PreparedInputQueue, transport::TransportInputPolicy},
    process_loopback::{ProcessBridgeOptions, ProcessIdentity, ProcessLoopbackOptions},
};
use serde_json::json;

#[test]
fn omitted_process_age_intent_preserves_compatible_defaults() {
    let options: ProcessBridgeOptions = serde_json::from_value(json!({})).unwrap();
    assert_eq!(options, ProcessBridgeOptions::default());
    assert_eq!(options.prepare().unwrap(), PreparedInputQueue::compatible());
    assert!(
        serde_json::to_value(options)
            .unwrap()
            .get("age_limit_100ns")
            .is_none(),
        "an omitted age policy must not change existing serialized defaults"
    );
}

#[test]
fn explicit_process_sample_age_is_refused_for_compatible_and_target_policies() {
    for policy in [
        TransportInputPolicy::Compatible,
        TransportInputPolicy::Target {
            capacity_frames: 8192,
            target_frames: 2048,
            jitter_frames: 96,
        },
    ] {
        for limit in [0, 100_000, u64::MAX] {
            let mut requested = serde_json::to_value(ProcessBridgeOptions::default()).unwrap();
            requested["input"] = serde_json::to_value(policy).unwrap();
            requested["age_limit_100ns"] = json!(limit);
            let options: ProcessBridgeOptions = serde_json::from_value(requested).unwrap();
            let error = options.prepare().unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains("sample-age enforcement unsupported"),
                "{message}"
            );
            assert!(message.contains("age is unknown"), "{message}");
            assert_eq!(options.for_consumer(48_000, 1024).unwrap_err(), error);
            let capture = ProcessLoopbackOptions {
                artifact: None,
                bridge: options,
                latency_telemetry: Default::default(),
                target: ProcessIdentity {
                    pid: if std::process::id() == 1 { 2 } else { 1 },
                    creation_time_100ns: 1,
                    executable_name: "age-request.exe".into(),
                },
                duration: moiren_windows_audio::SessionDuration::UntilStopped,
            };
            assert_eq!(capture.validate().unwrap_err(), error);
            assert_eq!(
                serde_json::to_value(options).unwrap()["age_limit_100ns"],
                limit
            );
        }
    }
}
