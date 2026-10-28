use super::*;
use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    process_loopback::{ProcessIdentity, ProcessTraceOrigin},
    stats::SILENT,
};

fn identity() -> ProcessIdentity {
    ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    }
}

#[test]
fn final_trace_export_reads_actual_histories_without_inventing_closure_or_native_origin() {
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
    let running = snapshot(&observer, identity(), 1, 4).unwrap();
    assert!(!running.packet_history.writer_finished);
    assert!(!running.queue_history.writer_finished);
    assert!(running.trace(&identity(), 1).is_err());
    drop(ingress);
    drop(source);
    let closed = snapshot(&observer, identity(), 1, 4).unwrap();
    assert_eq!(closed.packet_history.offered_frames, 8);
    assert_eq!(closed.queue_history.events[0].sample.requested_frames, 4);
    assert!(closed.packet_history.writer_finished);
    assert!(closed.queue_history.writer_finished);
    let report = closed
        .trace(&identity(), 1)
        .unwrap()
        .replay(
            &identity(),
            1,
            moiren_windows_audio::process_loopback::ProcessBridgeOptions {
                max_demand_frames: 4,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(report.trace_origin, ProcessTraceOrigin::CallerSupplied);
    assert!(!report.qualified);
}

#[test]
fn trace_file_is_bounded_new_only_and_retains_truncated_diagnostics() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    for n in 0..257 {
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 4,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(1000 + n * 100),
            )
            .unwrap();
        source
            .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(1010 + n * 100))
            .unwrap();
    }
    drop(ingress);
    drop(source);
    let artifact = snapshot(&observer, identity(), 1, 4).unwrap();
    assert_eq!(artifact.packet_history.overwritten_events, 1);
    assert!(artifact.trace(&identity(), 1).is_err());
    let directory =
        std::env::temp_dir().join(format!("moiren-process-trace-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("trace.json");
    persist(&path, &artifact).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() <= 1024 * 1024);
    let restored: moiren_windows_audio::process_loopback::ProcessTraceArtifact =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored.packet_history, artifact.packet_history);
    assert!(persist(&path, &artifact).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut huge = artifact;
    huge.process.executable_name = "x".repeat(1024 * 1024);
    assert!(persist(&directory.join("large.json"), &huge).is_err());
    assert!(!directory.join("large.json").exists());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn trace_path_is_reserved_before_directory_creation() {
    for manifest in [false, true] {
        let directory = std::env::temp_dir().join(format!(
            "moiren-trace-path-{}-{manifest}",
            std::process::id()
        ));
        let mut options = super::super::LiveOptions {
            artifact_dir: directory.clone(),
            source_kind:
                moiren_windows_audio::latency::qualification::EvidenceSourceKind::ProcessLoopback,
            ..Default::default()
        };
        let reserved = directory.join("child/../process-trace.json");
        if manifest {
            options.manifest_path = Some(reserved);
        } else {
            options.report_path = Some(reserved);
        }
        assert!(super::super::prepare_paths(&options).is_err());
        assert!(!directory.exists());
    }
}

#[test]
fn exporting_refused_shape_is_diagnostic_but_storage_failure_is_hard() {
    let (ingress, source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    drop(ingress);
    drop(source);
    let mut report = super::super::NativeReport::default();
    let failure = super::super::ArtifactPaths {
        process_trace: None,
        ..Default::default()
    };
    assert!(export_into(&mut report, &failure, &observer, identity(), 4).is_err());
    assert_eq!(report.process_trace.as_ref().unwrap()["exported"], false);
    assert!(report.process_trace.as_ref().unwrap()["failure"].is_string());
    let directory = std::env::temp_dir().join(format!("moiren-empty-trace-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("empty.json");
    let paths = super::super::ArtifactPaths {
        process_trace: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    export_into(&mut report, &paths, &observer, identity(), 4).unwrap();
    let value = report.process_trace.as_ref().unwrap();
    assert_eq!(value["exported"], true);
    assert_eq!(value["writers_finished"], true);
    assert_eq!(value["replayable"], false);
    assert!(value["refusal"].is_string());
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}
