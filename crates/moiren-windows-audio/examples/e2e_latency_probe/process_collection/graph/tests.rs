use super::*;
use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    stats::SILENT,
};

#[test]
fn actual_graph_finish_drops_source_before_exporting_its_closed_histories() {
    let process = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    };
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
    let options = LiveOptions {
        engine_block_frames: 4,
        ..Default::default()
    };
    let renderer = build_renderer(&options, source).unwrap();
    let mut graph = Graph::new().unwrap();
    graph.bridge = Some(observer.clone());
    graph.process = Some(process.clone());
    graph.prepared_demand = 4;
    graph.recovered_renderer = Some(renderer);
    assert!(!observer.queue_history().unwrap().writer_finished);
    graph.finish().unwrap();
    assert!(observer.queue_history().unwrap().writer_finished);
    let directory = std::env::temp_dir().join(format!("moiren-graph-trace-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("trace.json");
    let paths = ArtifactPaths {
        process_trace: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let mut native = super::super::super::NativeReport::default();
    graph.export_trace(&paths, &mut native).unwrap();
    let report = native.process_trace.unwrap();
    assert_eq!(report["exported"], true);
    assert_eq!(report["writers_finished"], true);
    assert_eq!(report["replayable"], true);
    assert_eq!(report["cleanup_verified"], false);
    let artifact: moiren_windows_audio::process_loopback::ProcessTraceArtifact =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(artifact.process, process);
    assert_eq!(artifact.owner_generation, 1);
    assert_eq!(artifact.prepared_consumer_demand_frames, 4);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}
