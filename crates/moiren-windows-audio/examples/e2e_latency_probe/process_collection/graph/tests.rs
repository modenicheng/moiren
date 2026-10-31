use super::*;
use moiren_windows_audio::{
    capture::{CaptureError, CaptureSource},
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    latency::{
        NativeStreamTiming, prepare_render_queue,
        transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    },
    stats::SILENT,
};

fn process_identity() -> ProcessIdentity {
    ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    }
}

fn bridge_options(target_frames: u32) -> ProcessBridgeOptions {
    ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 2048,
            target_frames,
            jitter_frames: 0,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 32,
            max_gap_frames: 32,
            source: DeliveryTimingSource::Declared,
        }),
        max_demand_frames: 64,
        max_ingress_elapsed_100ns: Some(123_456),
        ..Default::default()
    }
}

fn parked_render(graph: &Graph) -> PreparedNativeRender {
    let timing = NativeStreamTiming {
        sample_rate: 48_000,
        period_frames: Some(48),
        capacity_frames: 512,
    };
    let queue = prepare_render_queue(timing, RenderQueuePolicy::FullCapacity).unwrap();
    PreparedNativeRender::parked_fixture(
        super::super::super::render::NativeRenderInfo { timing, queue },
        graph.stop.clone(),
        RenderQueuePolicy::FullCapacity,
    )
}

fn retained_render(graph: &Graph) -> Value {
    serde_json::to_value(
        graph
            .render_report
            .as_ref()
            .expect("retained render report"),
    )
    .unwrap()
}

#[test]
fn actual_graph_refuses_segment_only_budget_before_capture_and_retains_parked_render() {
    let mut graph = Graph::new().unwrap();
    let render = parked_render(&graph);
    let called = std::cell::Cell::new(false);
    let options = LiveOptions {
        engine_block_frames: 64,
        ..Default::default()
    };
    assert!(bridge_options(128).prepare().is_ok());
    let result = graph.prepare_capture_after_render(
        render,
        &options,
        bridge_options(128),
        process_identity(),
        || None,
        |_, _, _, _| {
            called.set(true);
            panic!("capture preparation must not run after whole-wake refusal")
        },
    );
    assert!(result.is_err());
    assert!(!called.get());
    assert!(graph.capture.is_none());
    assert!(graph.trace.is_none());
    assert!(graph.aborts.is_empty());
    let evidence = graph.process_admission.as_ref().unwrap();
    assert_eq!(evidence.requested, bridge_options(128));
    assert!(evidence.admitted.is_none());
    assert_eq!(evidence.observed_render_queue.target_frames, 512);
    assert_eq!(
        evidence.failure.as_ref().unwrap().stage,
        super::super::super::process_admission::ProcessPreparationStage::Admission
    );
    let mut native = super::super::super::NativeReport::default();
    graph
        .export_trace(&ArtifactPaths::default(), &mut native)
        .unwrap();
    assert_eq!(native.process_admission, graph.process_admission);
    assert_eq!(native.process_trace.unwrap()["exported"], false);
    graph.request_stop().unwrap();
    graph.finish().unwrap();
    let report = retained_render(&graph);
    assert_eq!(report["timing"]["capacity_frames"], 512);
    assert_eq!(report["queue"]["target_frames"], 512);
    assert_eq!(report["stream_started"], false);
    assert_eq!(report["stop_succeeded"], true);
}

#[test]
fn actual_graph_capture_failure_receives_whole_wake_and_retains_abort_and_render_reports() {
    let mut graph = Graph::new().unwrap();
    let render = parked_render(&graph);
    let requested = bridge_options(546);
    let options = LiveOptions {
        engine_block_frames: 64,
        ..Default::default()
    };
    let peer_stopped = std::cell::Cell::new(false);
    let result = graph.prepare_capture_after_render(
        render,
        &options,
        requested,
        process_identity(),
        || {
            peer_stopped.set(true);
            Some(String::from("injected peer stop evidence"))
        },
        |mut capture, stop, gate, before_join| {
            assert_eq!(
                capture.bridge,
                ProcessBridgeOptions {
                    max_demand_frames: 512,
                    ..requested
                }
            );
            assert_eq!(capture.target, process_identity());
            // The real API rejects invalid identity before COM/native activation,
            // invoking peer cancellation and returning its typed abort evidence.
            capture.target.pid = 0;
            let mut failure = prepare_process_capture_with_gate_returning_with_abort(
                capture,
                stop,
                gate,
                before_join,
            )
            .err()
            .expect("identity refusal must precede native activation");
            assert!(peer_stopped.get());
            assert_eq!(failure.error, CaptureError::InvalidProcess);
            let mut report = CaptureReport::new(
                CaptureSource::ProcessLoopback,
                SessionDuration::UntilStopped,
            );
            report.failure = Some(String::from("injected capture report"));
            failure.report = Some(report);
            failure.join_error = Some(CaptureError::WorkerPanicked);
            Err(failure)
        },
    );
    let error = result.err().expect("capture must fail");
    assert!(error.to_string().contains("prepare owned Process capture"));
    assert!(error.downcast_ref::<CaptureError>().is_some());
    assert!(peer_stopped.get());
    assert!(graph.capture.is_none());
    assert_eq!(graph.aborts.len(), 1);
    assert_eq!(graph.aborts[0]["stage"], "capture_prepare");
    assert_eq!(graph.aborts[0]["abort"], "injected peer stop evidence");
    assert_eq!(graph.aborts[0]["abort_panicked"], false);
    assert!(
        graph.aborts[0]["join_error"]
            .as_str()
            .unwrap()
            .contains("panicked")
    );
    assert!(
        graph.prepared_render.is_none(),
        "capture abort must retire the parked render before startup cleanup returns"
    );
    assert_eq!(retained_render(&graph)["queue"]["target_frames"], 512);
    let evidence = graph.process_admission.as_ref().unwrap();
    assert_eq!(evidence.admitted.unwrap().max_demand_frames, 512);
    assert_eq!(
        evidence.failure.as_ref().unwrap().stage,
        super::super::super::process_admission::ProcessPreparationStage::Capture
    );
    let mut native = super::super::super::NativeReport::default();
    graph
        .export_trace(&ArtifactPaths::default(), &mut native)
        .unwrap();
    assert_eq!(native.process_admission, graph.process_admission);
    assert_eq!(
        graph.capture_report.as_ref().unwrap().failure.as_deref(),
        Some("injected capture report")
    );
    assert_eq!(
        graph
            .trace
            .as_ref()
            .unwrap()
            .artifact()
            .prepared_consumer_demand_frames,
        512
    );
    graph.request_stop().unwrap();
    graph.finish().unwrap();
    let report = retained_render(&graph);
    assert_eq!(report["queue"]["target_frames"], 512);
    assert_eq!(report["stream_started"], false);
    assert_eq!(
        graph.capture_report.as_ref().unwrap().failure.as_deref(),
        Some("injected capture report")
    );
}

#[test]
fn actual_graph_capture_abort_panic_still_retires_parked_render_and_retains_evidence() {
    let mut graph = Graph::new().unwrap();
    let render = parked_render(&graph);
    let options = LiveOptions {
        engine_block_frames: 64,
        ..Default::default()
    };
    let result = graph.prepare_capture_after_render(
        render,
        &options,
        bridge_options(546),
        process_identity(),
        || panic!("injected peer abort panic"),
        |mut capture, stop, gate, before_join| {
            capture.target.pid = 0;
            prepare_process_capture_with_gate_returning_with_abort(capture, stop, gate, before_join)
        },
    );
    assert!(result.is_err());
    assert!(graph.prepared_render.is_none());
    assert_eq!(retained_render(&graph)["queue"]["target_frames"], 512);
    assert_eq!(graph.aborts.len(), 1);
    assert_eq!(graph.aborts[0]["abort_panicked"], true);
    assert_eq!(graph.aborts[0]["abort"], Value::Null);
    graph.request_stop().unwrap();
    graph.finish().unwrap();
    assert_eq!(retained_render(&graph)["stream_started"], false);
}

#[test]
fn actual_graph_renderer_segments_native512_into64_reads_but_archives_whole_demand() {
    let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 2048,
        target_fill_frames: 1024,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    ingress
        .push_packet_with_publication_clock(
            &[],
            CapturePacket {
                frames: 1536,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
            || Some(1000),
        )
        .unwrap();
    let options = LiveOptions {
        engine_block_frames: 64,
        ..Default::default()
    };
    let mut renderer = build_renderer(&options, source).unwrap();
    let timeline = renderer.timeline_observer();
    let before = timeline.try_snapshot().unwrap();
    let mut graph = Graph::new().unwrap();
    graph.bridge = Some(observer.clone());
    graph.trace = Some(ProcessTraceCollector::new(
        process_identity(),
        1,
        48_000,
        48_000,
        512,
    ));
    graph.poll_trace();
    renderer
        .render(&mut [[f64::NAN; 2]; 512], &mut [0.0; 128])
        .unwrap();
    assert_eq!(renderer.counters().frames, 512);
    assert_eq!(renderer.counters().blocks, 8);
    assert_eq!(renderer.counters().shortfall_frames, 0);
    assert_eq!(timeline.try_snapshot().unwrap().frame, before.frame + 512);
    drop(ingress);
    graph.recovered_renderer = Some(renderer);
    graph.finish().unwrap();
    let archive = graph.trace.as_ref().unwrap().artifact();
    assert_eq!(archive.prepared_consumer_demand_frames, 512);
    assert_eq!(archive.queue_history.events.len(), 8);
    assert!(
        archive
            .queue_history
            .events
            .iter()
            .all(|read| { read.sample.requested_frames == 64 && read.sample.output_frames == 64 })
    );
    assert!(archive.queue_history.writer_finished);
    assert!(archive.packet_history.writer_finished);
    assert!(archive.collection_refusal.is_none());
    assert_eq!(
        archive.queue_history.unavailable_clocks, 8,
        "the hardware-free source intentionally has no native QPC"
    );
    assert!(archive.trace(&process_identity(), 1).is_err());
}

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
    graph.trace = Some(ProcessTraceCollector::new(
        process.clone(),
        1,
        48_000,
        48_000,
        4,
    ));
    graph.poll_trace();
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
    let artifact: moiren_windows_audio::process_loopback::ProcessTraceArchive =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(artifact.process, process);
    assert_eq!(artifact.schema_version, 2);
    assert_eq!(artifact.owner_generation, 1);
    assert_eq!(artifact.prepared_consumer_demand_frames, 4);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn actual_graph_control_observations_archive_multiple_rings_before_final_source_drop() {
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
    let mut graph = Graph::new().unwrap();
    graph.bridge = Some(observer.clone());
    graph.trace = Some(ProcessTraceCollector::new(
        process.clone(),
        1,
        48_000,
        48_000,
        4,
    ));
    graph.poll_trace();
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
            graph.observation();
        }
    }
    drop(ingress);
    let options = LiveOptions {
        engine_block_frames: 4,
        ..Default::default()
    };
    graph.recovered_renderer = Some(build_renderer(&options, source).unwrap());
    assert!(!observer.queue_history().unwrap().writer_finished);
    graph.finish().unwrap();
    let artifact = graph.trace.as_ref().unwrap().artifact();
    assert_eq!(artifact.packet_history.events.len(), 600);
    assert_eq!(artifact.queue_history.events.len(), 600);
    assert_eq!(artifact.packet_history.events[0].sequence, 1);
    assert_eq!(artifact.queue_history.events[0].sequence, 1);
    assert!(artifact.packet_history.writer_finished);
    assert!(artifact.queue_history.writer_finished);
    assert!(artifact.collection_refusal.is_none());
    assert!(artifact.trace(&process, 1).is_ok());
}
