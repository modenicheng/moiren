use super::*;

#[test]
fn canceled_before_open_reports_canceled_without_creating_a_file() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let path = std::env::temp_dir().join(format!(
        "moiren-canceled-recording-{}-{}.wav",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    assert!(!path.exists());
    let mut snapshot = RecordingSnapshot::idle(node);
    snapshot.phase = RecordingPhase::Starting;
    snapshot.path = Some(path.clone());
    let shared = Shared {
        stop: AtomicBool::new(true),
        snapshot: Mutex::new(snapshot),
    };
    assert!(open_destination(&shared).unwrap().is_none());
    let canceled = shared.snapshot.lock().unwrap();
    assert_eq!(canceled.phase, RecordingPhase::Canceled);
    assert!(!canceled.phase.is_busy());
    assert_eq!(canceled.path.as_ref(), Some(&path));
    assert_eq!(canceled.written_frames, 0);
    assert!(canceled.error.is_none());
    assert!(
        !path.exists(),
        "canceled admission must not advertise a saved WAV"
    );
}

#[test]
fn canceled_phase_is_published_only_after_its_worker_is_reaped() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let mut snapshot = RecordingSnapshot::idle(node);
    snapshot.phase = RecordingPhase::Canceled;
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(true),
        snapshot: Mutex::new(snapshot.clone()),
    });
    let (release, wait) = std::sync::mpsc::channel();
    let thread = thread::spawn(move || {
        let _ = wait.recv();
    });
    let mut runtime = Runtime::default();
    runtime.tasks.insert(
        node,
        Task {
            shared,
            thread,
            last_snapshot: snapshot,
            disk: capacity::Snapshot::default(),
        },
    );
    let mut service = RecordingService::default();
    runtime.poll(&mut service);
    assert_eq!(service.entries[&node].phase, RecordingPhase::Stopping);
    assert!(runtime.is_busy(node));
    assert!(!runtime.is_drained());
    release.send(()).unwrap();
    while !runtime.tasks[&node].thread.is_finished() {
        thread::yield_now();
    }
    runtime.poll(&mut service);
    assert_eq!(service.entries[&node].phase, RecordingPhase::Canceled);
    assert!(!runtime.is_busy(node));
    assert!(runtime.is_drained());
}

#[test]
fn poisoned_worker_publication_is_reaped_instead_of_hanging_exit() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        snapshot: Mutex::new(RecordingSnapshot::idle(node)),
    });
    let worker_shared = shared.clone();
    let thread = thread::spawn(move || {
        let _guard = worker_shared.snapshot.lock().unwrap();
        panic!("inject poisoned publication");
    });
    while !thread.is_finished() {
        std::thread::yield_now();
    }
    let mut runtime = Runtime::default();
    runtime.tasks.insert(
        node,
        Task {
            shared,
            thread,
            last_snapshot: RecordingSnapshot::idle(node),
            disk: capacity::Snapshot::default(),
        },
    );
    let mut service = RecordingService::default();
    runtime.poll(&mut service);
    assert!(runtime.is_drained());
    assert_eq!(service.entries[&node].phase, RecordingPhase::Failed);
    assert!(
        service.entries[&node]
            .error
            .as_deref()
            .unwrap()
            .contains("panicked")
    );
}

#[test]
fn busy_worker_publication_never_blocks_the_service() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        snapshot: Mutex::new(RecordingSnapshot::idle(node)),
    });
    let thread = thread::spawn(|| {});
    while !thread.is_finished() {
        std::thread::yield_now();
    }
    let mut runtime = Runtime::default();
    runtime.tasks.insert(
        node,
        Task {
            shared: shared.clone(),
            thread,
            last_snapshot: RecordingSnapshot::idle(node),
            disk: capacity::Snapshot::default(),
        },
    );
    let mut service = RecordingService::default();
    let guard = shared.snapshot.lock().unwrap();
    runtime.poll(&mut service);
    assert!(!runtime.is_drained());
    drop(guard);
    runtime.poll(&mut service);
    assert!(runtime.is_drained());
}

#[test]
fn retired_failure_reaches_diagnostics_without_overwriting_new_node_state() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let mut snapshot = RecordingSnapshot::idle(node);
    snapshot.phase = RecordingPhase::Failed;
    snapshot.path = Some(PathBuf::from("old.wav"));
    snapshot.error = Some("injected flush failure".into());
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        snapshot: Mutex::new(snapshot),
    });
    let thread = thread::spawn(|| {});
    while !thread.is_finished() {
        std::thread::yield_now();
    }
    let mut runtime = Runtime::default();
    runtime.retired.push(Task {
        shared,
        thread,
        last_snapshot: RecordingSnapshot::idle(node),
        disk: capacity::Snapshot::default(),
    });
    let mut service = RecordingService::default();
    service.entries.insert(node, RecordingSnapshot::idle(node));
    let errors = runtime.poll(&mut service);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("injected flush failure"));
    assert!(errors[0].contains("old.wav"));
    assert_eq!(service.entries[&node].phase, RecordingPhase::Idle);
    assert_eq!(service.entries[&node].path, None);
}

#[test]
fn aggregate_includes_same_volume_and_unknown_writers_but_not_other_volumes() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let mut entries = Vec::new();
    for volume in [Some("volume-a"), Some("volume-a"), Some("volume-b"), None] {
        let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
        let mut snapshot = RecordingSnapshot::idle(node);
        snapshot.phase = RecordingPhase::Recording;
        snapshot.estimate = RecordingEstimate::float_wav(48_000, 2, 0, Some(1024 * 1024 * 1024));
        snapshot.estimate.disk_volume = volume.map(str::to_owned);
        entries.push(snapshot);
    }
    let mut service = RecordingService::default();
    service
        .entries
        .extend(entries.iter().map(|entry| (entry.node, entry.clone())));
    aggregate_estimates(&mut service, &entries.iter().collect::<Vec<_>>());
    assert_eq!(
        service.entries[&entries[0].node]
            .estimate
            .aggregate_bytes_per_second,
        Some(384_000 * 3)
    );
    assert_eq!(
        service.entries[&entries[1].node]
            .estimate
            .aggregate_bytes_per_second,
        Some(384_000 * 3)
    );
    assert_eq!(
        service.entries[&entries[2].node]
            .estimate
            .aggregate_bytes_per_second,
        Some(384_000 * 2)
    );
    assert_eq!(
        service.entries[&entries[3].node]
            .estimate
            .aggregate_bytes_per_second,
        Some(384_000 * 4)
    );
}

#[test]
fn same_volume_estimate_uses_the_smallest_observed_free_space() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let mut first = RecordingSnapshot::idle(node);
    first.phase = RecordingPhase::Recording;
    first.estimate = RecordingEstimate::float_wav(48_000, 2, 0, Some(256 * 1024 * 1024));
    first.estimate.disk_volume = Some("volume-a".into());
    let mut second = first.clone();
    second.node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    second.estimate.free_disk_bytes = Some(MIN_DISK_RESERVE_BYTES);
    let mut service = RecordingService::default();
    service.entries.insert(node, first.clone());
    aggregate_estimates(&mut service, &[&first, &second]);
    assert_eq!(
        service.entries[&node].estimate.free_disk_bytes,
        Some(MIN_DISK_RESERVE_BYTES)
    );
    assert_eq!(
        service.entries[&node].estimate.disk_remaining_seconds,
        Some(0.0)
    );
}

#[test]
fn graph_channel_change_stops_and_waits_for_recording_reaping() {
    let project = |channels| {
        let mut project = moiren_engine::host::EngineProject::default();
        let node = project
            .graph
            .create_node(NodeKind::Recorder, channels)
            .unwrap();
        project.nodes.insert(
            node,
            moiren_engine::host::NodeConfig::Recorder(Default::default()),
        );
        (project, node)
    };
    let (stereo, node) = project(2);
    let (mono, same_node) = project(1);
    assert_eq!(same_node, node);
    let mut snapshot = RecordingSnapshot::idle(node);
    snapshot.phase = RecordingPhase::Recording;
    snapshot.sample_rate = Some(48_000);
    snapshot.channels = Some(2);
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        snapshot: Mutex::new(snapshot.clone()),
    });
    let thread = thread::spawn(|| {});
    while !thread.is_finished() {
        std::thread::yield_now();
    }
    let mut runtime = Runtime::default();
    runtime.tasks.insert(
        node,
        Task {
            shared: shared.clone(),
            thread,
            last_snapshot: snapshot.clone(),
            disk: capacity::Snapshot::default(),
        },
    );
    let mut service = RecordingService::default();
    service.entries.insert(node, snapshot);
    assert!(runtime.prepare_apply(&mut service, &stereo, Some(&stereo)));
    assert!(!runtime.prepare_apply(&mut service, &mono, Some(&stereo)));
    assert!(shared.stop.load(Ordering::Acquire));
    assert_eq!(service.entries[&node].phase, RecordingPhase::Stopping);
    runtime.poll(&mut service);
    assert!(runtime.is_drained());
    assert!(runtime.prepare_apply(&mut service, &mono, Some(&stereo)));
}

#[test]
fn proposed_capacity_includes_its_rate_plus_actual_same_volume_writers() {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let mut proposed = RecordingSnapshot::idle(node);
    proposed.estimate = RecordingEstimate::float_wav(48_000, 2, 0, Some(1024 * 1024 * 1024));
    proposed.estimate.proposed_sample_rate = Some(48_000);
    proposed.estimate.proposed_channels = Some(2);
    proposed.estimate.disk_volume = Some("volume-a".into());
    let mut writer = RecordingSnapshot::idle(graph.create_node(NodeKind::Recorder, 2).unwrap());
    writer.phase = RecordingPhase::Recording;
    writer.estimate = RecordingEstimate::float_wav(48_000, 2, 0, Some(1024 * 1024 * 1024));
    writer.estimate.disk_volume = Some("volume-a".into());
    let mut service = RecordingService::default();
    service.entries.insert(node, proposed);
    service.entries.insert(writer.node, writer.clone());
    aggregate_estimates(&mut service, &[&writer]);
    assert_eq!(
        service.entries[&node].estimate.aggregate_bytes_per_second,
        Some(384_000 * 2)
    );
    assert_eq!(
        service.entries[&writer.node]
            .estimate
            .aggregate_bytes_per_second,
        Some(384_000)
    );
    assert_eq!(service.entries[&node].sample_rate, None);
    assert_eq!(service.entries[&node].channels, None);
}
