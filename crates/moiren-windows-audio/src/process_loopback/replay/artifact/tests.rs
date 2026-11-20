use crate::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    process_loopback::{
        ObservedProcessTrace, ProcessBridgeOptions, ProcessTraceArtifact, ProcessTraceOrigin,
    },
    stats::SILENT,
};

fn artifact() -> ProcessTraceArtifact {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    ingress
        .push_packet_with_publication_clock(
            &[],
            CapturePacket {
                frames: 8,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
            || Some(1000),
        )
        .unwrap();
    source
        .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(1100))
        .unwrap();
    drop(ingress);
    drop(source);
    ProcessTraceArtifact {
        schema_version: 1,
        process: crate::process_loopback::ProcessIdentity {
            pid: 42,
            creation_time_100ns: 123,
            executable_name: "source.exe".into(),
        },
        owner_generation: 7,
        input_sample_rate: 48_000,
        consumer_sample_rate: 48_000,
        prepared_consumer_demand_frames: 4,
        packet_history: observer.packet_history().unwrap(),
        queue_history: observer.queue_history().unwrap(),
    }
}

#[test]
fn artifact_round_trip_has_explicit_schema_and_never_imports_native_origin() {
    let source = artifact();
    let value = serde_json::to_value(&source).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert!(value.get("trace_origin").is_none());
    let imported: ProcessTraceArtifact = serde_json::from_value(value).unwrap();
    let trace: ObservedProcessTrace = imported.trace(&source.process, 7).unwrap();
    let report = trace
        .replay(
            &source.process,
            7,
            ProcessBridgeOptions {
                max_demand_frames: 4,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(report.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert!(!report.qualified);
    assert_eq!(report.requested_frames, 4);
}

#[test]
fn artifact_refuses_missing_unknown_or_oversized_shape_without_fallback() {
    let original = serde_json::to_value(artifact()).unwrap();
    for field in [
        "schema_version",
        "process",
        "owner_generation",
        "input_sample_rate",
        "consumer_sample_rate",
        "prepared_consumer_demand_frames",
        "packet_history",
        "queue_history",
    ] {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ProcessTraceArtifact>(value).is_err());
    }
    for edit in 0..4 {
        let mut value = original.clone();
        match edit {
            0 => value["trace_origin"] = "retained_native_owner".into(),
            1 => value["packet_history"]["writer_finished"] = serde_json::Value::Null,
            2 => {
                value["packet_history"]["events"] = serde_json::Value::Array(vec![
                value["packet_history"]["events"][0].clone();
                257
            ])
            }
            _ => {
                value["queue_history"]["events"] =
                    serde_json::Value::Array(vec![value["queue_history"]["events"][0].clone(); 257])
            }
        }
        assert!(serde_json::from_value::<ProcessTraceArtifact>(value).is_err());
    }
}

#[test]
fn artifact_preserves_incomplete_diagnostics_but_refuses_replay_and_wrong_expectations() {
    let source = artifact();
    let mut wrong = source.process.clone();
    wrong.creation_time_100ns += 1;
    assert_eq!(
        source.trace(&wrong, 7).unwrap_err(),
        crate::process_loopback::ProcessReplayError::IdentityMismatch
    );
    assert_eq!(
        source.trace(&source.process, 8).unwrap_err(),
        crate::process_loopback::ProcessReplayError::GenerationMismatch
    );
    let mut unknown_version = source.clone();
    unknown_version.schema_version = 2;
    assert!(unknown_version.trace(&source.process, 7).is_err());
    let mut incomplete = source.clone();
    incomplete.packet_history.writer_finished = false;
    let restored: ProcessTraceArtifact =
        serde_json::from_str(&serde_json::to_string(&incomplete).unwrap()).unwrap();
    assert!(!restored.packet_history.writer_finished);
    assert!(restored.trace(&source.process, 7).is_err());
}
