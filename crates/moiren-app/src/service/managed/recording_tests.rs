use super::*;
use std::{fs, path::PathBuf, sync::atomic::AtomicU64};

struct Destination(PathBuf);

impl Destination {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "moiren-service-recording-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory.join("capture.wav"))
    }

    fn assert_finished(&self, written_frames: u64) {
        self.assert_finished_with_sample(written_frames, 1.0);
    }

    fn assert_finished_with_sample(&self, written_frames: u64, expected_sample: f32) {
        let bytes = fs::read(&self.0).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 32);
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len() - 8
        );
        assert_eq!(
            u32::from_le_bytes(bytes[46..50].try_into().unwrap()) as u64,
            written_frames
        );
        assert_eq!(
            u32::from_le_bytes(bytes[54..58].try_into().unwrap()) as u64,
            written_frames * 8
        );
        assert_eq!(bytes.len() as u64, 58 + written_frames * 8);
        assert!(
            bytes[58..]
                .as_chunks::<4>()
                .0
                .iter()
                .all(|sample| f32::from_le_bytes(*sample) == expected_sample)
        );
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_dir(self.0.parent().unwrap());
    }
}

fn recording_rig(destination: &Destination) -> (Rig, NodeId) {
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    let node = rig.node(NodeKind::Recorder);
    let graph = rig.c.graph.snapshot().graph;
    rig.request(AppRequest::EditGraph(GraphEdit::Connect {
        source: graph
            .get_node(rig.node(NodeKind::Source))
            .unwrap()
            .outputs()[0]
            .id(),
        destination: graph.get_node(node).unwrap().inputs()[0].id(),
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        let graph = rig.c.graph.snapshot();
        graph.active_revision == Some(graph.candidate_revision)
    });
    rig.request(AppRequest::SetRecordingDestination {
        node,
        path: destination.0.clone(),
    });
    (rig, node)
}

fn status(rig: &Rig, node: NodeId) -> RecordingSnapshot {
    snapshot(&rig.c)
        .recordings
        .into_iter()
        .find(|entry| entry.node == node)
        .unwrap()
}

fn start_recording(rig: &mut Rig, node: NodeId) {
    rig.request(AppRequest::StartRecording { node });
    rig.wait(|rig| status(rig, node).written_frames > 0);
}

#[test]
fn recording_writes_real_float_wav_and_stop_keeps_audio_running() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    let running = status(&rig, node);
    assert_eq!(running.phase, RecordingPhase::Recording);
    assert_eq!(running.path.as_ref(), Some(&destination.0));
    assert_eq!(running.sample_rate, Some(48_000));
    assert_eq!(running.channels, Some(2));
    assert_eq!(running.format, RecordingFormat::WavFloat32);
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
    destination.assert_finished(status(&rig, node).written_frames);
}

const BLOCKED_DISK_OBSERVATION: u64 = 123_456;

fn block_first_capacity_probe(
    rig: &mut Rig,
) -> (
    Arc<std::sync::atomic::AtomicUsize>,
    std::sync::mpsc::Sender<()>,
) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let probe_calls = calls.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let wait = std::sync::Mutex::new(wait);
    rig.driver.recordings.set_capacity_probe_for_test(move |_| {
        if probe_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            // Dropping the sender also releases the probe on assertion failure.
            let _ = wait.lock().unwrap().recv();
            Some(BLOCKED_DISK_OBSERVATION)
        } else {
            None
        }
    });
    (calls, release)
}

#[test]
fn blocked_capacity_probe_does_not_delay_stop_or_wav_finalization() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    let (calls, release) = block_first_capacity_probe(&mut rig);
    start_recording(&mut rig, node);
    rig.wait(|_| calls.load(Ordering::SeqCst) == 1);
    assert_eq!(
        status(&rig, node).estimate.basis,
        RecordingEstimateBasis::RiffOnly
    );

    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    assert!(rig.driver.recordings.is_drained());
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
    destination.assert_finished(status(&rig, node).written_frames);
    let finalized = fs::read(&destination.0).unwrap();
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(fs::read(&destination.0).unwrap(), finalized);
    rig.c.core.begin_exit();
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
    rig.wait(|rig| rig.driver.is_drained() && rig.pool.is_idle() && rig.c.pending_jobs() == 0);
    // The independent recorder has stopped and sealed its file while the OS
    // capacity query is still blocked; neither Stop nor exit depends on it.
    release.send(()).unwrap();
}

#[test]
fn blocked_capacity_probe_survives_load_without_adopting_reused_node_id() {
    let old_destination = Destination::new();
    let new_destination = Destination::new();
    let (mut rig, node) = recording_rig(&old_destination);
    let (calls, release) = block_first_capacity_probe(&mut rig);
    start_recording(&mut rig, node);
    rig.wait(|_| calls.load(Ordering::SeqCst) == 1);
    let old_host = rig.driver.owner.as_ref().unwrap().observer.read().host_id;
    let project =
        crate::project::ProjectDocument::from_graph("Replacement", &rig.c.graph.snapshot())
            .unwrap();
    rig.request(AppRequest::LoadProject(project));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.driver.recordings.is_drained()
            && rig.c.core.phase() == SessionPhase::Running
            && rig
                .driver
                .owner
                .as_ref()
                .is_some_and(|owner| owner.observer.read().host_id != old_host)
    });
    assert_eq!(rig.node(NodeKind::Recorder), node);
    let old_bytes = fs::read(&old_destination.0).unwrap();
    old_destination
        .assert_finished(u32::from_le_bytes(old_bytes[46..50].try_into().unwrap()) as u64);
    rig.request(AppRequest::SetRecordingDestination {
        node,
        path: new_destination.0.clone(),
    });
    start_recording(&mut rig, node);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "only one live capacity probe"
    );
    assert_eq!(status(&rig, node).estimate.free_disk_bytes, None);
    release.send(()).unwrap();
    rig.wait(|rig| {
        assert_ne!(
            status(rig, node).estimate.free_disk_bytes,
            Some(BLOCKED_DISK_OBSERVATION),
            "the retired session's capacity result must not reach the new session"
        );
        calls.load(Ordering::SeqCst) > 1
    });
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    new_destination.assert_finished(status(&rig, node).written_frames);
    assert_eq!(fs::read(&old_destination.0).unwrap(), old_bytes);
}

#[test]
fn recording_never_overwrites_and_retains_open_errors_in_its_snapshot() {
    let destination = Destination::new();
    fs::write(&destination.0, b"keep existing bytes").unwrap();
    let (mut rig, node) = recording_rig(&destination);
    rig.request(AppRequest::StartRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Failed);
    assert_eq!(fs::read(&destination.0).unwrap(), b"keep existing bytes");
    assert!(status(&rig, node).error.is_some());
    assert_eq!(rig.c.core.phase(), SessionPhase::Running);
}

#[test]
fn duplicate_start_and_destination_edit_cannot_replace_a_live_session() {
    let destination = Destination::new();
    let other = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    rig.request(AppRequest::StartRecording { node });
    rig.request(AppRequest::SetRecordingDestination {
        node,
        path: other.0.clone(),
    });
    assert_eq!(status(&rig, node).path, Some(destination.0.clone()));
    assert_eq!(status(&rig, node).phase, RecordingPhase::Recording);
    assert!(!other.0.exists());
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    destination.assert_finished(status(&rig, node).written_frames);
}

#[test]
fn owner_stop_finalizes_recording_even_when_prepare_lane_is_occupied() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    let (release, wait) = std::sync::mpsc::channel();
    rig.pool
        .try_submit(
            JobPriority::Prepare,
            Box::new(move || {
                let _ = wait.recv();
            }),
        )
        .unwrap();
    rig.request(AppRequest::Stop);
    rig.wait(|rig| rig.driver.is_drained() && status(rig, node).phase == RecordingPhase::Finished);
    destination.assert_finished(status(&rig, node).written_frames);
    release.send(()).unwrap();
}

#[test]
fn removing_a_recorder_finishes_its_file_without_waiting_for_auto_apply() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    rig.request(AppRequest::EditGraph(GraphEdit::RemoveNode(node)));
    // Do not require the replacement graph to finish before finalizing the file.
    rig.driver.auto_apply_deadline = None;
    rig.wait(|rig| rig.driver.recordings.is_drained());
    let bytes = fs::read(&destination.0).unwrap();
    let frames = u32::from_le_bytes(bytes[46..50].try_into().unwrap()) as u64;
    assert!(frames > 0);
    destination.assert_finished(frames);
    assert!(
        !snapshot(&rig.c)
            .recordings
            .iter()
            .any(|entry| entry.node == node)
    );
}

#[test]
fn loading_same_node_ids_cancels_old_files_and_does_not_inherit_destinations() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    let project =
        crate::project::ProjectDocument::from_graph("Replacement", &rig.c.graph.snapshot())
            .unwrap();
    rig.request(AppRequest::LoadProject(project));
    rig.wait(|rig| rig.driver.recordings.is_drained());
    let fresh = status(&rig, node);
    assert_eq!(fresh.path, None);
    assert_eq!(fresh.phase, RecordingPhase::Idle);
    assert_eq!(fresh.written_frames, 0);
    let bytes = fs::read(&destination.0).unwrap();
    destination.assert_finished(u32::from_le_bytes(bytes[46..50].try_into().unwrap()) as u64);
    rig.request(AppRequest::SetRecordingDestination {
        node,
        path: destination.0.with_file_name("new.wav"),
    });
    rig.request(AppRequest::StartRecording { node });
    assert_eq!(status(&rig, node).phase, RecordingPhase::Failed);
    assert!(!destination.0.with_file_name("new.wav").exists());
}

#[test]
fn exit_waits_for_recording_finalization_before_reporting_drained() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    rig.c.core.begin_exit();
    rig.driver.begin_exit(&mut rig.c, &mut rig.pool);
    rig.wait(|rig| rig.driver.is_drained() && rig.pool.is_idle() && rig.c.pending_jobs() == 0);
    assert_eq!(status(&rig, node).phase, RecordingPhase::Finished);
    destination.assert_finished(status(&rig, node).written_frames);
}

#[test]
fn recording_uses_confirmed_sample_rate_instead_of_unapplied_settings() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    let mut settings = rig.c.audio_settings.clone();
    settings.processing.sample_rate = 44_100;
    rig.request(AppRequest::SetAudioSettings(settings));
    start_recording(&mut rig, node);
    assert_eq!(status(&rig, node).sample_rate, Some(48_000));
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    destination.assert_finished(status(&rig, node).written_frames);
}

#[test]
fn replacing_the_owner_finalizes_existing_file_without_auto_restarting_recording() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    let generation = rig.c.core.generation();
    let mut settings = rig.c.audio_settings.clone();
    settings.processing.block_frames = 512;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.core.phase() == SessionPhase::Running
            && rig.c.core.generation() != generation
            && status(rig, node).phase == RecordingPhase::Finished
    });
    destination.assert_finished(status(&rig, node).written_frames);
    assert!(rig.driver.recordings.is_drained());
}

#[test]
fn stop_while_starting_is_not_lost_and_never_waits_on_shared_cleanup_lane() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    let (release, wait) = std::sync::mpsc::channel();
    rig.pool
        .try_submit(
            JobPriority::Cleanup,
            Box::new(move || {
                let _ = wait.recv();
            }),
        )
        .unwrap();
    rig.request(AppRequest::StartRecording { node });
    rig.request(AppRequest::StopRecording { node });
    assert_eq!(status(&rig, node).phase, RecordingPhase::Stopping);
    rig.wait(|rig| rig.driver.recordings.is_drained());
    assert!(rig.driver.recordings.is_drained());
    let stopped = status(&rig, node);
    // Do not infer a saved file from Stop alone: only the worker knows whether
    // cancellation preceded create_new or finalized an already-created WAV.
    match stopped.phase {
        RecordingPhase::Canceled => {
            assert!(!destination.0.exists());
            assert_eq!(stopped.written_frames, 0);
            assert!(stopped.error.is_none());
        }
        RecordingPhase::Finished => destination.assert_finished(stopped.written_frames),
        phase => panic!("unexpected stop result: {phase:?}"),
    }
    release.send(()).unwrap();
}

#[test]
fn missing_destination_and_stopped_audio_do_not_start_a_file_worker() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    assert!(status(&rig, node).estimate.preflight_pending);
    assert!(!rig.driver.recordings.is_drained());
    // Removing intent also leaves its queued preflight obsolete. Rejected
    // admission must cancel it immediately, not rely on a later service poll.
    rig.c.recordings.clear();
    rig.request(AppRequest::StartRecording { node });
    assert_eq!(status(&rig, node).phase, RecordingPhase::Failed);
    assert!(!status(&rig, node).estimate.preflight_pending);
    assert!(!rig.driver.recordings.is_busy(node));
    assert!(rig.driver.recordings.is_drained());
    assert!(!destination.0.exists());
    rig.request(AppRequest::SetRecordingDestination {
        node,
        path: destination.0.clone(),
    });
    rig.request(AppRequest::Stop);
    rig.wait(|rig| rig.driver.is_drained());
    rig.request(AppRequest::StartRecording { node });
    assert_eq!(status(&rig, node).phase, RecordingPhase::Failed);
    assert!(!status(&rig, node).estimate.preflight_pending);
    assert!(!rig.driver.recordings.is_busy(node));
    assert!(rig.driver.recordings.is_drained());
    assert!(!destination.0.exists());
}

#[test]
fn rejected_start_cancels_only_its_capacity_preflight_without_polling() {
    let rejected_path = Destination::new();
    let pending_path = Destination::new();
    let mut rig = Rig::new();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    let rejected = rig.node(NodeKind::Recorder);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    let pending = rig
        .c
        .graph
        .snapshot()
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Recorder && node.id() != rejected)
        .unwrap()
        .id();
    for (node, path) in [(rejected, &rejected_path), (pending, &pending_path)] {
        rig.request(AppRequest::SetRecordingDestination {
            node,
            path: path.0.clone(),
        });
        assert!(status(&rig, node).estimate.preflight_pending);
    }
    rig.request(AppRequest::StartRecording { node: rejected });
    let failure = status(&rig, rejected);
    assert_eq!(failure.phase, RecordingPhase::Failed);
    assert!(!failure.estimate.preflight_pending);
    assert_eq!(failure.sample_rate, None);
    assert_eq!(failure.written_frames, 0);
    assert!(!rig.driver.recordings.is_busy(rejected));
    assert!(status(&rig, pending).estimate.preflight_pending);
    assert!(
        !rig.driver.recordings.is_drained(),
        "other preflight still pending"
    );
    rig.wait(|rig| !status(rig, pending).estimate.preflight_pending);
    assert!(rig.driver.recordings.is_drained());
    assert_eq!(status(&rig, rejected), failure);
    assert!(!rejected_path.0.exists());
    assert!(!pending_path.0.exists());
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn second_recording_is_independent_of_the_first_nodes_stop() {
    let first_path = Destination::new();
    let second_path = Destination::new();
    let (mut rig, first) = recording_rig(&first_path);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    let graph = rig.c.graph.snapshot().graph;
    let second = graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Recorder && node.id() != first)
        .unwrap()
        .id();
    rig.request(AppRequest::EditGraph(GraphEdit::Connect {
        source: graph
            .get_node(rig.node(NodeKind::Source))
            .unwrap()
            .outputs()[0]
            .id(),
        destination: graph.get_node(second).unwrap().inputs()[0].id(),
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        let graph = rig.c.graph.snapshot();
        graph.active_revision == Some(graph.candidate_revision)
    });
    rig.request(AppRequest::SetRecordingDestination {
        node: second,
        path: second_path.0.clone(),
    });
    start_recording(&mut rig, first);
    start_recording(&mut rig, second);
    rig.request(AppRequest::StopRecording { node: first });
    rig.wait(|rig| status(rig, first).phase == RecordingPhase::Finished);
    let second_frames = status(&rig, second).written_frames;
    rig.wait(|rig| status(rig, second).written_frames > second_frames);
    assert_eq!(status(&rig, second).phase, RecordingPhase::Recording);
    rig.request(AppRequest::StopRecording { node: second });
    rig.wait(|rig| status(rig, second).phase == RecordingPhase::Finished);
    first_path.assert_finished(status(&rig, first).written_frames);
    second_path.assert_finished(status(&rig, second).written_frames);
}

#[test]
fn sample_rate_apply_finalizes_the_old_file_before_running_a_new_format() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    let mut settings = rig.c.audio_settings.clone();
    settings.processing.sample_rate = 44_100;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        if status(rig, node).phase.is_busy() {
            assert_ne!(
                rig.c
                    .active_audio_settings
                    .as_ref()
                    .map(|settings| settings.processing.sample_rate),
                Some(44_100)
            );
        }
        rig.c.core.phase() == SessionPhase::Running
            && rig
                .c
                .active_audio_settings
                .as_ref()
                .map(|settings| settings.processing.sample_rate)
                == Some(44_100)
    });
    assert_eq!(status(&rig, node).phase, RecordingPhase::Finished);
    assert_eq!(status(&rig, node).sample_rate, Some(48_000));
    destination.assert_finished(status(&rig, node).written_frames);
}

#[test]
fn recorder_configuration_edit_is_refused_until_recording_finalizes() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    start_recording(&mut rig, node);
    let before = rig.c.graph.snapshot().node(node).unwrap().builtin.clone();
    rig.request(AppRequest::EditGraph(GraphEdit::SetBuiltin {
        node,
        state: BuiltinState::Recorder(moiren_engine::recorder::RecorderSettings {
            capacity_frames: 256,
        }),
    }));
    assert_eq!(rig.c.graph.snapshot().node(node).unwrap().builtin, before);
    assert_eq!(status(&rig, node).phase, RecordingPhase::Recording);
    assert!(
        rig.c
            .graph
            .snapshot()
            .last_error
            .as_deref()
            .unwrap()
            .contains("finalize")
    );
}

#[test]
fn f64_graph_still_writes_the_supported_float32_wav_format() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    let backend = FakeBackend(rig.state.clone());
    rig.driver.f64_factory = Some(Arc::new(move |_, _| Ok(Box::new(backend.clone()))));
    let mut settings = rig.c.audio_settings.clone();
    settings.processing.precision = ProcessingPrecision::F64;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.core.phase() == SessionPhase::Running
            && rig
                .c
                .active_audio_settings
                .as_ref()
                .is_some_and(|settings| settings.processing.precision == ProcessingPrecision::F64)
    });
    start_recording(&mut rig, node);
    assert_eq!(status(&rig, node).format, RecordingFormat::WavFloat32);
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    destination.assert_finished(status(&rig, node).written_frames);
}

#[test]
fn recording_start_excludes_pre_start_sentinel_frames() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    let reader = rig
        .driver
        .owner
        .as_ref()
        .unwrap()
        .observer
        .recorder_reader(node)
        .unwrap();
    rig.wait(|_| reader.queued_frames().is_some_and(|frames| frames > 0));
    let sentinel_frames = reader.queued_frames().unwrap();
    // The queue already contains 1.0 samples. Switch live audio to 0.25 and
    // wait for its RT ramp to settle, without consuming the old sentinel queue.
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: rig.node(NodeKind::Source),
        state: MixerState {
            gain: 0.25,
            ..Default::default()
        },
    }));
    rig.wait(|rig| rig.sample() == 0.25 && rig.c.control.accepted_pending == 0);
    assert!(reader.queued_frames().unwrap() >= sentinel_frames);
    start_recording(&mut rig, node);
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    let recorded = status(&rig, node);
    assert!(recorded.written_frames > 0);
    assert!(recorded.stats.unwrap().read_frames > recorded.written_frames);
    destination.assert_finished_with_sample(recorded.written_frames, 0.25);
}

#[test]
fn load_during_recording_start_cannot_adopt_a_reused_node_id() {
    let old_destination = Destination::new();
    let new_destination = Destination::new();
    let (mut rig, node) = recording_rig(&old_destination);
    let old_observer = rig.driver.owner.as_ref().unwrap().observer.clone();
    let old_host = old_observer.read().host_id;
    let project =
        crate::project::ProjectDocument::from_graph("Replacement", &rig.c.graph.snapshot())
            .unwrap();
    rig.request(AppRequest::StartRecording { node });
    assert_eq!(status(&rig, node).phase, RecordingPhase::Starting);
    rig.request(AppRequest::LoadProject(project));
    assert_eq!(rig.node(NodeKind::Recorder), node);
    assert!(
        rig.driver.recordings.is_busy(node),
        "retired worker retains the old identity until reaped"
    );
    assert_eq!(status(&rig, node).path, None);
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.driver.recordings.is_drained()
            && rig.c.core.phase() == SessionPhase::Running
            && rig
                .driver
                .owner
                .as_ref()
                .is_some_and(|owner| owner.observer.read().host_id != old_host)
    });
    assert!(old_observer.recorder_reader(node).is_err());
    let old_bytes = fs::read(&old_destination.0).ok();
    if let Some(bytes) = &old_bytes {
        old_destination
            .assert_finished(u32::from_le_bytes(bytes[46..50].try_into().unwrap()) as u64);
    }
    assert_eq!(status(&rig, node).path, None);
    assert_eq!(status(&rig, node).written_frames, 0);
    rig.request(AppRequest::SetRecordingDestination {
        node,
        path: new_destination.0.clone(),
    });
    start_recording(&mut rig, node);
    assert_eq!(status(&rig, node).path, Some(new_destination.0.clone()));
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    new_destination.assert_finished(status(&rig, node).written_frames);
    assert_eq!(fs::read(&old_destination.0).ok(), old_bytes);
}

#[test]
fn configured_destination_estimates_proposed_capacity_without_starting_audio() {
    let destination = Destination::new();
    let mut rig = Rig::new();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Recorder,
        channels: 2,
    }));
    let node = rig.node(NodeKind::Recorder);
    let mut settings = rig.c.audio_settings.clone();
    settings.processing.sample_rate = 44_100;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.request(AppRequest::SetRecordingDestination {
        node,
        path: destination.0.clone(),
    });
    let proposed = status(&rig, node);
    assert_eq!(proposed.phase, RecordingPhase::Idle);
    assert_eq!(proposed.sample_rate, None);
    assert_eq!(proposed.channels, None);
    assert_eq!(proposed.estimate.proposed_sample_rate, Some(44_100));
    assert_eq!(proposed.estimate.proposed_channels, Some(2));
    assert_eq!(proposed.estimate.bytes_per_second, Some(44_100 * 2 * 4));
    assert!(proposed.estimate.remaining_seconds.is_some());
    assert!(proposed.estimate.preflight_pending);
    rig.wait(|rig| !status(rig, node).estimate.preflight_pending);
    let observed = status(&rig, node);
    assert_eq!(observed.sample_rate, None);
    assert_eq!(observed.channels, None);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 0);
    assert!(
        !destination.0.exists(),
        "preflight never creates the destination"
    );
    assert_eq!(
        observed.estimate.basis,
        if observed.estimate.free_disk_bytes.is_some() {
            RecordingEstimateBasis::DiskAndRiff
        } else {
            RecordingEstimateBasis::RiffOnly
        }
    );
}

#[test]
fn preflight_refreshes_after_proposed_rate_changes_but_start_uses_confirmed_rate() {
    let destination = Destination::new();
    let (mut rig, node) = recording_rig(&destination);
    rig.wait(|rig| !status(rig, node).estimate.preflight_pending);
    let mut settings = rig.c.audio_settings.clone();
    settings.processing.sample_rate = 44_100;
    rig.request(AppRequest::SetAudioSettings(settings));
    rig.wait(|rig| {
        status(rig, node).estimate.proposed_sample_rate == Some(44_100)
            && !status(rig, node).estimate.preflight_pending
    });
    assert_eq!(status(&rig, node).sample_rate, None);
    start_recording(&mut rig, node);
    let live = status(&rig, node);
    assert_eq!(live.sample_rate, Some(48_000));
    assert_eq!(live.estimate.proposed_sample_rate, None);
    assert!(!live.estimate.preflight_pending);
    rig.request(AppRequest::StopRecording { node });
    rig.wait(|rig| status(rig, node).phase == RecordingPhase::Finished);
    destination.assert_finished(status(&rig, node).written_frames);
}
