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
fn prefix_collector_exports_more_than_one_ring_and_exact_final_writer_closure() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    let mut collector = moiren_windows_audio::process_loopback::ProcessTraceCollector::new(
        identity(),
        1,
        48_000,
        48_000,
        4,
    );
    collector.poll(&observer);
    for n in 0..600 {
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
        if n % 64 == 63 {
            collector.poll(&observer);
        }
    }
    collector.poll(&observer);
    assert!(!collector.artifact().packet_history.writer_finished);
    drop(ingress);
    collector.poll(&observer);
    assert!(collector.artifact().packet_history.writer_finished);
    assert!(!collector.artifact().queue_history.writer_finished);
    drop(source);
    collector.poll(&observer);
    let artifact = collector.artifact();
    assert_eq!(artifact.schema_version, 2);
    assert_eq!(artifact.packet_history.events.len(), 600);
    assert_eq!(artifact.queue_history.events.len(), 600);
    assert!(artifact.collection_refusal.is_none());
    assert!(artifact.packet_history.writer_finished);
    assert!(artifact.queue_history.writer_finished);
    assert!(artifact.trace(&identity(), 1).is_ok());
    let directory =
        std::env::temp_dir().join(format!("moiren-prefix-trace-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("trace.json");
    let paths = super::super::ArtifactPaths {
        process_trace: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let mut report = super::super::NativeReport::default();
    export_collector_into(&mut report, &paths, &collector).unwrap();
    assert_eq!(report.process_trace.as_ref().unwrap()["replayable"], true);
    let restored: moiren_windows_audio::process_loopback::ProcessTraceArchive =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(restored.packet_history.events.len(), 600);
    assert!(restored.trace(&identity(), 1).is_ok());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn missed_poll_exports_refusal_and_final_closure_without_stopping_collection() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    let mut collector = moiren_windows_audio::process_loopback::ProcessTraceCollector::new(
        identity(),
        1,
        48_000,
        48_000,
        4,
    );
    collector.poll(&observer);
    for n in 0..300 {
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
    collector.poll(&observer);
    assert!(collector.artifact().collection_refusal.is_some());
    // Refusal is diagnostic: the real producer and source still keep working.
    ingress
        .push_packet_with_publication_clock(
            &[],
            CapturePacket {
                frames: 4,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
            || Some(40_000),
        )
        .unwrap();
    source
        .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(40_010))
        .unwrap();
    drop(ingress);
    drop(source);
    collector.poll(&observer);
    let artifact = collector.artifact();
    assert!(artifact.collection_refusal.is_some());
    assert!(artifact.packet_history.writer_finished);
    assert!(artifact.queue_history.writer_finished);
    assert!(artifact.trace(&identity(), 1).is_err());
    let directory =
        std::env::temp_dir().join(format!("moiren-prefix-refusal-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("trace.json");
    let paths = super::super::ArtifactPaths {
        process_trace: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let mut report = super::super::NativeReport::default();
    export_collector_into(&mut report, &paths, &collector).unwrap();
    let value = report.process_trace.unwrap();
    assert_eq!(value["exported"], true);
    assert_eq!(value["writers_finished"], true);
    assert_eq!(value["replayable"], false);
    assert!(!value["collection_refusal"].is_null());
    assert!(path.exists());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn archive_cap_is_checked_during_serialization_before_creating_a_file() {
    let (ingress, source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    let mut collector = moiren_windows_audio::process_loopback::ProcessTraceCollector::new(
        identity(),
        1,
        48_000,
        48_000,
        4,
    );
    drop(ingress);
    drop(source);
    collector.poll(&observer);
    let mut artifact = collector.artifact();
    // Escaping expands this beyond the limit even though the raw label fits.
    artifact.process.executable_name = "\n".repeat(9 * 1024 * 1024);
    let path = std::env::temp_dir().join(format!("moiren-prefix-cap-{}.json", std::process::id()));
    assert!(persist(&path, &artifact).is_err());
    assert!(!path.exists());
}

#[test]
fn archive_serialization_stops_at_the_cap_instead_of_building_an_unbounded_vec() {
    use serde::ser::SerializeSeq;
    use std::cell::Cell;
    struct LargeArchive<'a> {
        serialized: &'a Cell<usize>,
    }
    impl serde::Serialize for LargeArchive<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let payload = "x".repeat(1024 * 1024);
            let mut sequence = serializer.serialize_seq(Some(20))?;
            for _ in 0..20 {
                self.serialized.set(self.serialized.get() + 1);
                sequence.serialize_element(&payload)?;
            }
            sequence.end()
        }
    }
    let serialized = Cell::new(0);
    let path = std::env::temp_dir().join(format!(
        "moiren-prefix-stream-cap-{}.json",
        std::process::id()
    ));
    assert!(
        persist(
            &path,
            &LargeArchive {
                serialized: &serialized
            }
        )
        .is_err()
    );
    assert!(
        serialized.get() < 20,
        "the serializer exceeded the cap before refusing output"
    );
    assert!(!path.exists());
}

#[test]
fn finite_polling_preserves_natural_completion_and_stops_at_a_bounded_deadline() {
    use std::{cell::Cell, time::Duration};
    let elapsed = Cell::new(Duration::ZERO);
    let polls = Cell::new(0);
    let stops = Cell::new(0);
    poll_finite_with_clock(
        Duration::from_millis(50),
        || polls.set(polls.get() + 1),
        || polls.get() >= 3,
        || {
            stops.set(stops.get() + 1);
            Ok(())
        },
        || elapsed.get(),
        |delay| elapsed.set(elapsed.get() + delay),
    )
    .unwrap();
    assert_eq!(polls.get(), 3);
    assert_eq!(stops.get(), 0);
    assert!(elapsed.get() < Duration::from_millis(50));
    elapsed.set(Duration::ZERO);
    polls.set(0);
    poll_finite_with_clock(
        Duration::from_millis(25),
        || polls.set(polls.get() + 1),
        || false,
        || {
            stops.set(stops.get() + 1);
            Ok(())
        },
        || elapsed.get(),
        |delay| elapsed.set(elapsed.get() + delay),
    )
    .unwrap();
    assert_eq!(elapsed.get(), Duration::from_millis(25));
    assert_eq!(stops.get(), 1);
    assert_eq!(polls.get(), 4);
}

#[test]
fn finite_polling_uses_fresh_time_and_preserves_cancellation_failure() {
    use std::{cell::Cell, time::Duration};
    let elapsed = Cell::new(Duration::ZERO);
    let polls = Cell::new(0);
    let sleeps = Cell::new(0);
    let error = poll_finite_with_clock(
        Duration::from_millis(25),
        || {
            polls.set(polls.get() + 1);
            elapsed.set(Duration::from_millis(40));
        },
        || false,
        || anyhow::bail!("stop failed"),
        || elapsed.get(),
        |_| sleeps.set(sleeps.get() + 1),
    )
    .unwrap_err();
    assert_eq!(polls.get(), 1);
    assert_eq!(sleeps.get(), 0);
    assert!(error.to_string().contains("stop failed"));
}

#[test]
fn archive_export_keeps_failure_dto_and_propagates_new_file_errors() {
    let (ingress, source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
    let mut collector = moiren_windows_audio::process_loopback::ProcessTraceCollector::new(
        identity(),
        1,
        48_000,
        48_000,
        4,
    );
    drop(ingress);
    drop(source);
    collector.poll(&observer);
    let mut report = super::super::NativeReport::default();
    let missing = super::super::ArtifactPaths::default();
    assert!(export_collector_into(&mut report, &missing, &collector).is_err());
    assert_eq!(report.process_trace.as_ref().unwrap()["exported"], false);
    let directory =
        std::env::temp_dir().join(format!("moiren-prefix-existing-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("trace.json");
    std::fs::write(&path, b"existing evidence").unwrap();
    let paths = super::super::ArtifactPaths {
        process_trace: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    assert!(export_collector_into(&mut report, &paths, &collector).is_err());
    assert_eq!(report.process_trace.as_ref().unwrap()["exported"], false);
    assert!(report.process_trace.as_ref().unwrap()["failure"].is_string());
    assert_eq!(std::fs::read(&path).unwrap(), b"existing evidence");
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
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
    huge.process.executable_name = "x".repeat(16 * 1024 * 1024);
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
