use super::support::*;
use moiren_core::graph::{NodeId, NodeKind, SendParams};
use moiren_engine::{
    host::*,
    recorder::{RecorderSettings, RecorderState, RecordingSession, RecordingState},
};
use std::{
    io::{self, Cursor, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

fn recording_project() -> (EngineProject, NodeId, NodeId, NodeId) {
    let (mut project, input, gain, output) = project();
    project.graph.remove_node(gain).unwrap();
    project.nodes.remove(&gain);
    let recorder = project.graph.create_node(NodeKind::Recorder, 1).unwrap();
    project.nodes.insert(
        recorder,
        NodeConfig::Recorder(RecorderSettings {
            capacity_frames: 256,
        }),
    );
    for (source, destination) in [(input, recorder), (recorder, output)] {
        project
            .graph
            .connect(
                project.graph.get_node(source).unwrap().outputs()[0].id(),
                project.graph.get_node(destination).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
    }
    (project, recorder, input, output)
}

#[derive(Clone, Default)]
struct SharedWriter(Arc<Mutex<Cursor<Vec<u8>>>>);

impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for SharedWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.lock().unwrap().seek(position)
    }
}

fn wait_recording(session: &RecordingSession, predicate: impl Fn(&RecordingSession) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !predicate(session) {
        assert!(
            Instant::now() < deadline,
            "recording did not progress: {:?}",
            session.status()
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn assert_wav(bytes: &[u8], frames: u64) {
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
        bytes.len() - 8
    );
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(
        u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
        48_000
    );
    assert_eq!(
        u32::from_le_bytes(bytes[46..50].try_into().unwrap()) as u64,
        frames
    );
    assert_eq!(
        u32::from_le_bytes(bytes[54..58].try_into().unwrap()) as u64,
        frames * 4
    );
    assert_eq!(bytes.len() as u64, 58 + frames * 4);
    assert!(
        bytes[58..]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|bytes| f32::from_le_bytes(*bytes) == 1.0)
    );
}

#[test]
fn recording_claim_is_exclusive_and_finished_session_returns_stopped_reader() {
    let (project, recorder, _, _) = recording_project();
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let mut reader = observer.recorder_reader(recorder).unwrap();
    wait(&observer, |_| {
        reader.queued_frames().is_some_and(|frames| frames > 0)
    });
    state.pause_audio.store(true, Ordering::SeqCst);
    assert!(reader.stop());
    let buffered = reader.queued_frames().unwrap();
    let writer = SharedWriter::default();
    let session = reader.record_with_writer(writer.clone(), 48_000).unwrap();
    assert_eq!(reader.channel_count(), None);
    assert_eq!(reader.capacity_frames(), None);
    assert_eq!(reader.is_recording(), None);
    assert_eq!(reader.stats(), None);
    assert_eq!(reader.queued_frames(), None);
    assert_eq!(reader.read_interleaved(&mut [0.0; 8]), None);
    assert!(!reader.start());
    assert!(!reader.stop());
    let other = observer.recorder_reader(recorder).unwrap();
    assert!(
        other
            .record_with_writer(SharedWriter::default(), 48_000)
            .is_err()
    );
    wait_recording(&session, |s| s.status().written_frames >= buffered as u64);
    session.stop();
    wait_recording(&session, |s| s.status().state == RecordingState::Finished);
    // Finished must be published after returning the consumer, not before it.
    assert_eq!(reader.is_recording(), Some(false));
    assert_eq!(reader.queued_frames(), Some(0));
    let summary = session.finish().unwrap();
    assert_eq!(summary.stats.state, RecorderState::Stopped);
    assert_wav(writer.0.lock().unwrap().get_ref(), summary.written_frames);
    assert!(reader.start());
}

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "moiren-managed-recording-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
#[cfg_attr(miri, ignore = "requires filesystem access outside Miri isolation")]
fn busy_or_stale_recording_claim_does_not_truncate_destination() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let session = reader
        .record_with_writer(SharedWriter::default(), 48_000)
        .unwrap();
    let file = TemporaryFile::new();
    std::fs::write(&file.0, b"keep existing destination").unwrap();
    assert!(reader.record_wav(&file.0, 48_000).is_err());
    assert_eq!(
        std::fs::read(&file.0).unwrap(),
        b"keep existing destination"
    );
    drop(host);
    session.finish().unwrap();
    assert!(reader.record_wav(&file.0, 48_000).is_err());
    assert_eq!(
        std::fs::read(&file.0).unwrap(),
        b"keep existing destination"
    );
}

#[test]
fn failed_wav_initialization_returns_the_consumer_for_retry() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    assert_eq!(reader.is_recording(), Some(true));
    assert!(
        reader
            .record_with_writer(SharedWriter::default(), 0)
            .is_err()
    );
    assert_eq!(reader.channel_count(), Some(1));
    assert_eq!(reader.is_recording(), Some(true));
    assert!(reader.stop());
    assert!(
        reader
            .record_with_writer(Cursor::new(vec![1_u8]), 48_000)
            .is_err()
    );
    assert_eq!(reader.channel_count(), Some(1));
    assert_eq!(reader.is_recording(), Some(false));
    reader
        .record_with_writer(SharedWriter::default(), 48_000)
        .unwrap()
        .finish()
        .unwrap();
}

#[test]
#[cfg_attr(miri, ignore = "requires filesystem access outside Miri isolation")]
fn managed_record_wav_writes_and_finalizes_a_real_file() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let file = TemporaryFile::new();
    let session = reader.record_wav(&file.0, 48_000).unwrap();
    wait_recording(&session, |s| s.status().written_frames > 0);
    let summary = session.finish().unwrap();
    assert_wav(&std::fs::read(&file.0).unwrap(), summary.written_frames);
    assert_eq!(reader.is_recording(), Some(false));
}

#[test]
fn compatible_update_keeps_recording_worker_and_queue_active() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    let initial = wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let writer = SharedWriter::default();
    let session = reader.record_with_writer(writer.clone(), 48_000).unwrap();
    wait_recording(&session, |s| s.status().written_frames > 0);
    let edit = host
        .control()
        .edit(|project| {
            let node = project.graph.create_node(NodeKind::Gain, 1)?;
            project
                .nodes
                .insert(node, NodeConfig::Gain { initial: 0.5 });
            Ok(())
        })
        .unwrap();
    assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
    assert_eq!(
        observer.snapshot().owner_generation,
        initial.owner_generation
    );
    assert_eq!(session.status().state, RecordingState::Recording);
    assert_eq!(reader.stats(), None);
    let before = session.status().written_frames;
    wait_recording(&session, |s| s.status().written_frames > before);
    let summary = session.finish().unwrap();
    assert_wav(writer.0.lock().unwrap().get_ref(), summary.written_frames);
    assert_eq!(reader.is_recording(), Some(false));
}

#[test]
fn discarded_recorder_replacement_does_not_revoke_the_active_lease() {
    let (project, recorder, _, _) = recording_project();
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let session = reader
        .record_with_writer(SharedWriter::default(), 48_000)
        .unwrap();
    wait_recording(&session, |s| s.status().written_frames > 0);
    state.pause_audio.store(true, Ordering::SeqCst);
    let cancelled = host
        .control()
        .edit(move |project| {
            project.nodes.insert(
                recorder,
                NodeConfig::Recorder(RecorderSettings {
                    capacity_frames: 128,
                }),
            );
            Ok(())
        })
        .unwrap();
    wait(&observer, |snapshot| {
        matches!(snapshot.update, UpdateState::Pending(_))
    });
    let retained = host
        .control()
        .edit(move |project| {
            project.nodes.insert(
                recorder,
                NodeConfig::Recorder(RecorderSettings {
                    capacity_frames: 256,
                }),
            );
            Ok(())
        })
        .unwrap();
    wait(&observer, |snapshot| snapshot.desired_revision == 3);
    assert!(matches!(
        cancelled.outcome(),
        EditOutcome::Superseded { .. }
    ));
    state.pause_audio.store(false, Ordering::SeqCst);
    assert!(matches!(
        retained.wait(TIMEOUT),
        EditOutcome::Activated { .. }
    ));
    assert_eq!(session.status().state, RecordingState::Recording);
    let before = session.status().written_frames;
    wait_recording(&session, |s| s.status().written_frames > before);
    session.finish().unwrap();
    assert_eq!(reader.capacity_frames(), Some(256));
    assert_eq!(reader.is_recording(), Some(false));
}

#[test]
fn dropping_session_joins_worker_and_returns_stopped_reader() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let writer = SharedWriter::default();
    let session = reader.record_with_writer(writer.clone(), 48_000).unwrap();
    wait_recording(&session, |s| s.status().written_frames > 0);
    drop(session);
    assert_eq!(reader.is_recording(), Some(false));
    let bytes = writer.0.lock().unwrap();
    let frames = u32::from_le_bytes(bytes.get_ref()[46..50].try_into().unwrap());
    assert_wav(bytes.get_ref(), u64::from(frames));
}

#[test]
fn deleting_recorder_finalizes_session_and_stale_reader_cannot_reclaim() {
    let (project, recorder, input, output) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let writer = SharedWriter::default();
    let session = reader.record_with_writer(writer.clone(), 48_000).unwrap();
    wait_recording(&session, |s| s.status().written_frames > 0);
    let edit = host
        .control()
        .edit(move |project| {
            project.graph.remove_node(recorder)?;
            project.graph.connect(
                project.graph.get_node(input)?.outputs()[0].id(),
                project.graph.get_node(output)?.inputs()[0].id(),
                SendParams::default(),
            )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
    wait_recording(&session, |s| s.status().state == RecordingState::Finished);
    let summary = session.finish().unwrap();
    assert_wav(writer.0.lock().unwrap().get_ref(), summary.written_frames);
    assert_eq!(reader.stats(), None);
    assert!(observer.recorder_reader(recorder).is_err());
}

#[test]
fn replacing_recorder_config_revokes_only_old_endpoint_in_same_owner_generation() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    let initial = wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let session = reader
        .record_with_writer(SharedWriter::default(), 48_000)
        .unwrap();
    let edit = host
        .control()
        .edit(move |project| {
            project.nodes.insert(
                recorder,
                NodeConfig::Recorder(RecorderSettings {
                    capacity_frames: 128,
                }),
            );
            Ok(())
        })
        .unwrap();
    assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
    assert_eq!(
        observer.snapshot().owner_generation,
        initial.owner_generation
    );
    wait_recording(&session, |s| s.status().state == RecordingState::Finished);
    session.finish().unwrap();
    let current = observer.recorder_reader(recorder).unwrap();
    assert_eq!(current.capacity_frames(), Some(128));
    current
        .record_with_writer(SharedWriter::default(), 48_000)
        .unwrap()
        .finish()
        .unwrap();
}

#[test]
fn retained_session_and_reader_do_not_keep_host_alive() {
    let (project, recorder, _, _) = recording_project();
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let host = EngineHost::<f64>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let writer = SharedWriter::default();
    let session = reader.record_with_writer(writer.clone(), 48_000).unwrap();
    wait_recording(&session, |s| s.status().written_frames > 0);
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    wait_recording(&session, |s| s.status().state == RecordingState::Finished);
    let summary = session.finish().unwrap();
    assert_wav(writer.0.lock().unwrap().get_ref(), summary.written_frames);
    assert_eq!(reader.stats(), None);
    assert!(!reader.start());
}

#[derive(Default)]
struct WriteGate {
    entered: AtomicBool,
    release: Mutex<bool>,
    wake: Condvar,
}

struct BlockingWriter {
    bytes: Cursor<Vec<u8>>,
    gate: Arc<WriteGate>,
}

impl Write for BlockingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.position() >= 58 {
            self.gate.entered.store(true, Ordering::SeqCst);
            let mut release = self.gate.release.lock().unwrap();
            while !*release {
                release = self.gate.wake.wait(release).unwrap();
            }
        }
        self.bytes.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for BlockingWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(position)
    }
}

struct ReleaseGate(Arc<WriteGate>);

impl Drop for ReleaseGate {
    fn drop(&mut self) {
        *self.0.release.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}

#[test]
fn blocked_file_io_does_not_hold_reader_or_hub_locks_or_block_host_close() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let gate = Arc::new(WriteGate::default());
    let session = reader
        .record_with_writer(
            BlockingWriter {
                bytes: Cursor::new(Vec::new()),
                gate: gate.clone(),
            },
            48_000,
        )
        .unwrap();
    // On test failure release the writer before Session's stop+join Drop runs.
    let release = ReleaseGate(gate.clone());
    let deadline = Instant::now() + TIMEOUT;
    while !gate.entered.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(reader.stats(), None);
    assert!(observer.recorder_reader(recorder).is_ok());
    let edit = host
        .control()
        .edit(|project| {
            let node = project.graph.create_node(NodeKind::Gain, 1)?;
            project
                .nodes
                .insert(node, NodeConfig::Gain { initial: 1.0 });
            Ok(())
        })
        .unwrap();
    assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
    let (closed, receive) = std::sync::mpsc::channel();
    let close = std::thread::spawn(move || {
        drop(host);
        closed.send(()).unwrap();
    });
    assert!(
        receive.recv_timeout(TIMEOUT).is_ok(),
        "host close waited for blocked file IO"
    );
    drop(release);
    close.join().unwrap();
    session.finish().unwrap();
}

#[test]
fn old_lease_cannot_overwrite_replacement_endpoint_while_its_writer_is_blocked() {
    for replace_owner in [false, true] {
        let (project, recorder, _, _) = recording_project();
        let host = EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default())
            .unwrap();
        let observer = host.observer();
        let initial = wait(&observer, |snapshot| {
            snapshot.execution == ExecutionState::Running
        });
        let old = observer.recorder_reader(recorder).unwrap();
        let gate = Arc::new(WriteGate::default());
        let session = old
            .record_with_writer(
                BlockingWriter {
                    bytes: Cursor::new(Vec::new()),
                    gate: gate.clone(),
                },
                48_000,
            )
            .unwrap();
        let release = ReleaseGate(gate.clone());
        let deadline = Instant::now() + TIMEOUT;
        while !gate.entered.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let edit = host
            .control()
            .edit(move |project| {
                if replace_owner {
                    project.processing.engine.processing_sr = 44_100.0;
                } else {
                    project.nodes.insert(
                        recorder,
                        NodeConfig::Recorder(RecorderSettings {
                            capacity_frames: 128,
                        }),
                    );
                }
                Ok(())
            })
            .unwrap();
        assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
        assert_eq!(
            observer.snapshot().owner_generation != initial.owner_generation,
            replace_owner
        );
        let current = observer.recorder_reader(recorder).unwrap();
        let capacity = if replace_owner { 256 } else { 128 };
        assert_eq!(current.capacity_frames(), Some(capacity));
        // Keep the new endpoint claimed when the retired lease finally drops.
        let next = current
            .record_with_writer(
                SharedWriter::default(),
                if replace_owner { 44_100 } else { 48_000 },
            )
            .unwrap();
        drop(release);
        wait_recording(&session, |s| s.status().state == RecordingState::Finished);
        session.finish().unwrap();
        assert_eq!(current.stats(), None);
        assert!(
            current
                .record_with_writer(SharedWriter::default(), 48_000)
                .is_err()
        );
        next.finish().unwrap();
        assert_eq!(current.capacity_frames(), Some(capacity));
        assert_eq!(current.is_recording(), Some(false));
        if replace_owner {
            assert_eq!(old.channel_count(), None);
            assert!(
                old.record_with_writer(SharedWriter::default(), 48_000)
                    .is_err()
            );
        }
    }
}

struct FailingWriter(Cursor<Vec<u8>>);

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.position() >= 58 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "injected data write failure",
            ));
        }
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for FailingWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.seek(position)
    }
}

#[test]
fn failed_session_returns_stopped_consumer_before_publishing_failure() {
    let (project, recorder, _, _) = recording_project();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let reader = observer.recorder_reader(recorder).unwrap();
    let session = reader
        .record_with_writer(FailingWriter(Cursor::new(Vec::new())), 48_000)
        .unwrap();
    wait_recording(&session, |s| s.status().state == RecordingState::Failed);
    assert_eq!(reader.is_recording(), Some(false));
    assert_eq!(
        session.status().error.unwrap().kind,
        io::ErrorKind::WriteZero
    );
    assert_eq!(
        session.finish().unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    reader
        .record_with_writer(SharedWriter::default(), 48_000)
        .unwrap()
        .finish()
        .unwrap();
}
