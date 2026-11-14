use moiren_windows_audio::{
    capture::CaptureError,
    process_loopback::{ProcessIdentity, ProcessLoopbackOptions},
};
use std::time::Duration;

fn options(pid: u32, creation_time_100ns: u64) -> ProcessLoopbackOptions {
    ProcessLoopbackOptions {
        artifact: None,
        bridge: Default::default(),
        latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
        target: ProcessIdentity {
            pid,
            creation_time_100ns,
            executable_name: "tone.exe".into(),
        },
        duration: moiren_windows_audio::SessionDuration::For(Duration::from_secs(10)),
    }
}
#[test]
fn selection_requires_pid_and_creation_time_and_rejects_self() {
    assert_eq!(options(0, 1).validate(), Err(CaptureError::InvalidProcess));
    assert_eq!(
        options(123, 0).validate(),
        Err(CaptureError::InvalidProcess)
    );
    assert_eq!(
        options(std::process::id(), 1).validate(),
        Err(CaptureError::FeedbackTarget)
    );
    let mut valid = options(123, 1);
    assert!(valid.validate().is_ok());
    valid.duration = moiren_windows_audio::SessionDuration::For(Duration::from_secs(601));
    assert_eq!(valid.validate(), Err(CaptureError::InvalidDuration));
}
#[test]
fn identity_uses_creation_time_rather_than_name() {
    let selected = options(123, 1).target;
    let mut reopened = selected.clone();
    reopened.executable_name = "renamed.exe".into();
    assert!(selected.same_process(&reopened));
    reopened.creation_time_100ns = 2;
    assert!(!selected.same_process(&reopened));
    reopened = selected.clone();
    reopened.pid = 124;
    assert!(!selected.same_process(&reopened));
}

#[test]
fn explicit_process_bridge_admits_delivery_without_fabricating_a_native_period() {
    use moiren_windows_audio::{
        latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
        process_loopback::ProcessBridgeOptions,
    };
    let bridge = ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 8192,
            target_frames: 1024,
            jitter_frames: 96,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 480,
            max_gap_frames: 480,
            source: DeliveryTimingSource::Declared,
        }),
        engine_sample_rate: 48_000,
        max_demand_frames: 256,
        age_limit_100ns: None,
        max_ingress_elapsed_100ns: None,
    };
    let input = bridge.prepare().unwrap();
    assert_eq!(input.prime_frames, 1024);
    assert_eq!(input.servo_reference_frames, 1024);
    assert_eq!(input.modeled_minimum_frames, Some(834));
    let mut selected = options(123, 1);
    selected.bridge = bridge;
    assert!(selected.validate().is_ok());
    selected.bridge.input = TransportInputPolicy::Target {
        capacity_frames: 8192,
        target_frames: 512,
        jitter_frames: 96,
    };
    assert!(
        selected.validate().is_err(),
        "a 480-frame source cannot be shrunk arbitrarily"
    );
}

#[test]
fn process_bridge_defaults_preserve_compatibility_and_require_fixed_native_rate() {
    use moiren_windows_audio::{
        latency::{
            PreparedInputQueue,
            transport::{DeliveryTimingSource, InputDeliveryTiming},
        },
        process_loopback::ProcessBridgeOptions,
    };
    let compatible = ProcessBridgeOptions::default();
    assert_eq!(
        compatible.prepare().unwrap(),
        PreparedInputQueue::compatible()
    );
    for rate in [0, 12345] {
        assert!(
            ProcessBridgeOptions {
                engine_sample_rate: rate,
                ..compatible
            }
            .prepare()
            .is_err()
        );
    }
    assert!(
        ProcessBridgeOptions {
            delivery: Some(InputDeliveryTiming {
                sample_rate: 44_100,
                max_packet_frames: 480,
                max_gap_frames: 480,
                source: DeliveryTimingSource::Empirical,
            }),
            ..compatible
        }
        .prepare()
        .is_err()
    );
}

#[test]
fn process_owner_revalidates_complete_demand_instead_of_a_saved_block_hint() {
    use moiren_windows_audio::{
        latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
        process_loopback::ProcessBridgeOptions,
    };
    let request = ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 8192,
            target_frames: 320,
            jitter_frames: 32,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 128,
            max_gap_frames: 128,
            source: DeliveryTimingSource::Declared,
        }),
        max_demand_frames: 64,
        ..Default::default()
    };
    assert!(request.prepare().is_ok());
    assert_eq!(
        request.for_consumer(48_000, 128).unwrap().max_demand_frames,
        128
    );
    assert!(request.for_consumer(48_000, 256).is_err());
    // At another Engine rate the owner redoes the native-frame conversion.
    assert!(request.for_consumer(44_100, 256).is_err());
}
