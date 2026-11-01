//! Optional independent control diagnostic. Disk stalls cannot block service/RT.
use anyhow::{Context, Result};
use moiren_windows_audio::{
    clock_bridge::native_pcm::NativePcmConfig,
    engine_backend::{NativeObservationReadHandle, NativeProcessInterpolationCollector},
    latency::interpolation::InterpolationTraceChunk,
    process_loopback::ProcessIdentity,
};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const PENDING_LIMIT: usize = 4096;
const MAX_SESSIONS: usize = 8;
const PERSIST_INTERVAL: Duration = Duration::from_millis(50);

mod pcm;
use pcm::PcmArchive;

pub(super) struct CursorTraceWorker {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<()>>>,
}

struct Session {
    owner: NativeObservationReadHandle,
    collector: NativeProcessInterpolationCollector,
    persistence: ChunkPersistence,
    finalized: bool,
    last_persist: Instant,
    pcm: Option<PcmArchive>,
}

struct ChunkPersistence {
    directory: PathBuf,
    persisted_events: u64,
    persisted_chunks: u64,
    io_error: Option<String>,
    failed_chunk: Option<InterpolationTraceChunk>,
}

fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, value)?;
    use std::io::Write;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

impl ChunkPersistence {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            persisted_events: 0,
            persisted_chunks: 0,
            io_error: None,
            failed_chunk: None,
        }
    }

    fn persist(&mut self, chunk: InterpolationTraceChunk) {
        debug_assert!(self.io_error.is_none());
        let result = write_new(
            &self
                .directory
                .join(format!("chunk-{:08}.json", chunk.sequence)),
            &chunk,
        );
        match result {
            Ok(()) => {
                self.persisted_events += chunk.events.len() as u64;
                self.persisted_chunks += 1;
            }
            Err(error) => {
                // Retain the failed payload separately in memory until
                // finalization; do not claim its drained count persisted.
                self.io_error = Some(error.to_string());
                self.failed_chunk = Some(chunk);
            }
        }
    }
}

impl Session {
    fn persist(&mut self) {
        // Stop draining after IO failure; the failed payload stays owned and
        // later observations fill only the collector's bounded pending buffer.
        if self.persistence.io_error.is_none()
            && let Some(chunk) = self.collector.drain_chunk()
        {
            self.persistence.persist(chunk);
        }
        self.last_persist = Instant::now();
    }

    fn finish(&mut self, stopping: bool) -> Result<()> {
        if self.finalized {
            return Ok(());
        }
        let closed = self.collector.finish();
        if !stopping && (!closed || self.pcm.as_ref().is_some_and(|pcm| !pcm.writer_finished())) {
            return Ok(());
        }
        self.persist();
        let artifact = self.collector.artifact();
        if let Some(chunk) = &self.persistence.failed_chunk {
            let _ = write_new(&self.persistence.directory.join("failed-chunk.json"), chunk);
        }
        let completed = std::cell::Cell::new(false);
        let result = finish_pcm_and_cursor(self.pcm.as_mut(), |pcm_error, pcm_complete| {
            let result = write_new(
                &self.persistence.directory.join("final.json"),
                &serde_json::json!({
                    "schema_version": 1, "origin": self.collector.origin(),
                    "artifact": artifact, "final_history_available": closed,
                    "unavailable_polls": self.collector.unavailable_polls(),
                    "persisted_events": self.persistence.persisted_events,
                    "persisted_chunks": self.persistence.persisted_chunks,
                    "io_error": self.persistence.io_error,
                    "native_pcm_enabled": pcm_complete.is_some(),
                    "native_pcm_complete": pcm_complete,
                    "native_pcm_error": pcm_error,
                    "persistence_complete": closed && artifact.history_complete && self.persistence.io_error.is_none()
                        && self.persistence.persisted_events == artifact.total_events
                        && pcm_error.is_none()
                        && pcm_complete.unwrap_or(true),
                    "qualified": false,
                }),
            );
            completed.set(result.is_ok());
            result
        });
        self.finalized = completed.get();
        result
    }

    fn persistence_complete(&self) -> bool {
        let artifact = self.collector.artifact();
        self.finalized
            && artifact.history_complete
            && self.persistence.io_error.is_none()
            && self.persistence.persisted_events == artifact.total_events
            && self
                .pcm
                .as_ref()
                .is_none_or(PcmArchive::persistence_complete)
    }
}

/// A PCM receipt failure must not suppress an independently retained cursor
/// receipt. Neither failed payloads nor their successful prefix are rewritten.
fn finish_pcm_and_cursor(
    pcm: Option<&mut PcmArchive>,
    write_cursor: impl FnOnce(Option<&str>, Option<bool>) -> Result<()>,
) -> Result<()> {
    let mut result = Ok(());
    let complete = pcm.map(|pcm| {
        if let Err(error) = pcm.finish() {
            retain_error(&mut result, anyhow::anyhow!("PCM final receipt: {error:#}"));
        }
        pcm.persistence_complete()
    });
    let error = result.as_ref().err().map(|error| format!("{error:#}"));
    if let Err(error) = write_cursor(error.as_deref(), complete) {
        retain_error(&mut result, error);
    }
    result
}

impl CursorTraceWorker {
    pub(super) fn spawn(
        directory: &Path,
        read: impl Fn() -> Option<NativeObservationReadHandle> + Send + 'static,
        process: ProcessIdentity,
        expected_owners: usize,
        pcm_config: Option<NativePcmConfig>,
    ) -> Result<Self> {
        if let Some(config) = pcm_config {
            config.validate().map_err(anyhow::Error::msg)?;
        }
        fs::create_dir(directory).context("create new cursor trace directory")?;
        let directory = directory.to_owned();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::Builder::new().name("process-cursor-archive".into()).spawn(move || {
            let mut sessions: Vec<Session> = Vec::new();
            let mut result = (|| -> Result<()> {
                loop {
                    if let Some(owner) = read() {
                        // Metadata discovery happens only for a new owner. Per
                        // poll, collectors read only their pinned scalar rings.
                        if let Some(generation) = owner.running_generation()
                            && !sessions.iter().any(|session| session.owner.same_owner(&owner))
                        {
                            let snapshot = owner.snapshot();
                            let sources: Vec<_> = snapshot.process_delivery.iter().filter(|source| source.process.same_process(&process)).collect();
                            anyhow::ensure!(sources.len() <= 1, "cursor archive requires one Process source per owner");
                            for source in sources {
                                anyhow::ensure!(sessions.len() < MAX_SESSIONS, "cursor archive owner limit reached");
                                let collector = owner.process_interpolation_collector(source.node, &process, generation, PENDING_LIMIT)
                                    .map_err(anyhow::Error::msg)?;
                                let path = directory.join(format!("owner-{:04}-generation-{generation}", sessions.len() + 1));
                                fs::create_dir(&path)?;
                                write_new(&path.join("origin.json"), collector.origin())?;
                                let pcm = pcm_config.map(|_| {
                                    let reader = owner.process_native_pcm_reader(source.node, &process, generation)
                                        .map_err(anyhow::Error::msg)?;
                                    PcmArchive::new(&path, reader)
                                }).transpose()?;
                                sessions.push(Session {owner: owner.clone(), collector, persistence: ChunkPersistence::new(path), finalized: false, last_persist: Instant::now(), pcm});
                            }
                        }
                    }
                    for session in &mut sessions {
                        if session.finalized { continue; }
                        session.collector.poll();
                        if let Some(pcm) = &mut session.pcm { pcm.poll(); }
                        if session.collector.artifact().writer_finished {
                            session.finish(false)?;
                        } else if session.last_persist.elapsed() >= PERSIST_INTERVAL {
                            session.persist();
                        }
                    }
                    if stopping.load(Ordering::Acquire) { break; }
                    thread::sleep(Duration::from_millis(2));
                }
                Ok(())
            })();
            // Preserve every owner's final receipt even if discovery/IO failed
            // for a different owner. Native audio is already independent.
            for session in &mut sessions {
                if let Err(error) = session.finish(true) { retain_error(&mut result, error); }
                if let Some(error) = &session.persistence.io_error { retain_error(&mut result, anyhow::anyhow!("owner cursor persistence: {error}")); }
                if !session.persistence_complete() { retain_error(&mut result, anyhow::anyhow!("owner cursor archive incomplete: {}", session.persistence.directory.display())); }
            }
            if let Err(error) = validate_owner_count(sessions.len(), expected_owners) { retain_error(&mut result, error); }
            let summary = write_new(&directory.join("summary.json"), &serde_json::json!({
                "schema_version": 1, "sessions": sessions.len(),
                "expected_owners": expected_owners,
                "native_pcm_enabled": pcm_config.is_some(),
                "error": result.as_ref().err().map(|error| error.to_string()),
                "qualified": false,
                "scope": if pcm_config.is_some() {
                    "actual Process SRC cursors and offered/accepted native PCM mapping; not replay or physical qualification"
                } else {
                    "actual Process SRC cursor archive only; not PCM mapping or physical qualification"
                },
            }));
            if let Err(error) = summary { retain_error(&mut result, error); }
            result
        })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }

    pub(super) fn finish(mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .expect("worker owned")
            .join()
            .map_err(|_| anyhow::anyhow!("cursor diagnostic worker panicked"))?
    }
}

impl Drop for CursorTraceWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn retain_error(result: &mut Result<()>, error: anyhow::Error) {
    *result = Err(match std::mem::replace(result, Ok(())) {
        Ok(()) => error,
        Err(previous) => anyhow::anyhow!("{previous:#}; {error:#}"),
    });
}

fn validate_owner_count(actual: usize, expected: usize) -> Result<()> {
    anyhow::ensure!(
        (1..=MAX_SESSIONS).contains(&expected) && actual == expected,
        "cursor archive owner coverage incomplete: expected {expected}, observed {actual}; no matching owner may be silently omitted"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_windows_audio::latency::interpolation::{
        InterpolationEvent, InterpolationSpan, InterpolationTraceChunk,
    };

    #[test]
    fn missing_replacement_owner_refuses_even_with_a_complete_baseline() {
        assert!(
            validate_owner_count(1, 2)
                .unwrap_err()
                .to_string()
                .contains("owner coverage")
        );
        assert!(validate_owner_count(2, 2).is_ok());
        assert!(validate_owner_count(3, 2).is_err());
        assert!(validate_owner_count(0, 1).is_err());
    }

    struct TemporaryDirectory(PathBuf);
    impl TemporaryDirectory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "moiren-cursor-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn chunk() -> InterpolationTraceChunk {
        InterpolationTraceChunk {
            schema_version: 1,
            sequence: 1,
            events: vec![InterpolationEvent {
                sequence: 1,
                span: InterpolationSpan {
                    read_sequence: 1,
                    output_start_frame: 0,
                    input_start_frame: 0,
                    frames: 1,
                    generation: 0,
                    phase: 0.0,
                    step: 1.0,
                    fade_remaining: 0,
                },
            }],
        }
    }

    #[test]
    fn successful_chunk_is_counted_only_after_new_file_flush() {
        let directory = TemporaryDirectory::new();
        let mut persistence = ChunkPersistence::new(directory.0.clone());
        persistence.persist(chunk());
        assert_eq!(persistence.persisted_events, 1);
        assert_eq!(persistence.persisted_chunks, 1);
        assert!(persistence.io_error.is_none());
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.0.join("chunk-00000001.json")).unwrap())
                .unwrap();
        assert_eq!(saved["events"][0]["sequence"], 1);
        assert_eq!(
            saved["events"][0]["span"]["step_bits"],
            "4607182418800017408"
        );
    }

    #[test]
    fn existing_chunk_is_never_overwritten_and_failed_payload_stays_owned() {
        let directory = TemporaryDirectory::new();
        let path = directory.0.join("chunk-00000001.json");
        fs::write(&path, b"preserve original receipt").unwrap();
        let mut persistence = ChunkPersistence::new(directory.0.clone());
        persistence.persist(chunk());
        assert_eq!(persistence.persisted_events, 0);
        assert_eq!(persistence.persisted_chunks, 0);
        assert!(persistence.io_error.is_some());
        assert_eq!(persistence.failed_chunk.as_ref().unwrap().events.len(), 1);
        assert_eq!(fs::read(path).unwrap(), b"preserve original receipt");
    }

    #[test]
    fn worker_refuses_existing_directory_without_touching_contents() {
        let directory = TemporaryDirectory::new();
        let process = ProcessIdentity {
            pid: 42,
            creation_time_100ns: 1,
            executable_name: "test.exe".into(),
        };
        assert!(CursorTraceWorker::spawn(&directory.0, || None, process, 1, None).is_err());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn stopping_without_a_native_owner_joins_and_records_failure() {
        let directory = TemporaryDirectory::new();
        let path = directory.0.join("archive");
        let process = ProcessIdentity {
            pid: 42,
            creation_time_100ns: 1,
            executable_name: "test.exe".into(),
        };
        let worker = CursorTraceWorker::spawn(&path, || None, process, 1, None).unwrap();
        assert!(
            worker
                .finish()
                .unwrap_err()
                .to_string()
                .contains("no matching")
        );
        let summary: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("summary.json")).unwrap()).unwrap();
        assert_eq!(summary["sessions"], 0);
        assert!(summary["error"].as_str().unwrap().contains("no matching"));
        assert_eq!(summary["qualified"], false);
    }
}
