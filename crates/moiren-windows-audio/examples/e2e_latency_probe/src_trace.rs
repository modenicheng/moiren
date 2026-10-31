//! Control-side bounded export. Imported JSON cannot issue native provenance.
use anyhow::{Context, Result};
use moiren_windows_audio::engine_backend::{
    NativeObservationReadHandle, NativeSrcTraceChunk, NativeSrcTraceCollector,
};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc,
};

const MAX_ARCHIVE_BYTES: usize = 192 * 1024 * 1024;

#[path = "src_trace/stream.rs"]
mod stream;

#[cfg(test)]
#[path = "src_trace/fixture.rs"]
mod fixture;

const PENDING_LIMIT: usize = 4096;
const CHUNK_THRESHOLD: usize = 2048;
const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
const MAX_STREAM_BYTES: u64 = 32 * 1024 * 1024 * 1024;

pub(super) struct NativeSrcStream {
    collector: NativeSrcTraceCollector,
    writer: Option<stream::ChunkWriter>,
    pending: Option<NativeSrcTraceChunk>,
    chunks_path: PathBuf,
    failure: Option<String>,
    finished: bool,
}

impl NativeSrcStream {
    pub(super) fn open(owner: &NativeObservationReadHandle, chunks_path: &Path) -> Result<Self> {
        let collector = owner
            .src_trace_collector(PENDING_LIMIT)
            .map_err(anyhow::Error::msg)?;
        let sink = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(chunks_path)
            .context("create new SRC chunk stream")?;
        let writer = stream::ChunkWriter::with_sink(sink, MAX_RECORD_BYTES, MAX_STREAM_BYTES)?;
        Ok(Self {
            collector,
            writer: Some(writer),
            pending: None,
            chunks_path: chunks_path.into(),
            failure: None,
            finished: false,
        })
    }

    /// No file IO, serialization, wait or thread join on the poll path. A full
    /// control queue retains its chunk; any later history/cap loss stays sticky.
    pub(super) fn poll(&mut self) {
        if self.finished {
            return;
        }
        self.collector.poll();
        if self.failure.is_some() {
            return;
        }
        self.submit_pending();
        if self.pending.is_none()
            && self.collector.artifact().retained_checkpoints >= CHUNK_THRESHOLD
        {
            self.pending = self.collector.drain_chunk();
            self.submit_pending();
        }
    }

    fn submit_pending(&mut self) {
        let Some(chunk) = self.pending.take() else {
            return;
        };
        match self
            .writer
            .as_ref()
            .expect("live SRC writer")
            .try_send(chunk)
        {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(chunk)) => self.pending = Some(chunk),
            Err(mpsc::TrySendError::Disconnected(chunk)) => {
                self.pending = Some(chunk);
                self.failure = Some("SRC writer failed; pending scalar evidence retained".into());
            }
        }
    }

    pub(super) fn finish(
        &mut self,
        path: &Path,
        report: &mut Option<serde_json::Value>,
    ) -> Result<()> {
        if self.finished {
            return self
                .failure
                .as_ref()
                .map_or(Ok(()), |failure| Err(anyhow::anyhow!("{failure}")));
        }
        self.collector.finish();
        // The old Full chunk precedes the final collector chunk. Teardown can
        // wait, unlike ordinary collection polls. Failed chunks go in summary.
        let pending = self.pending.take();
        let final_chunk = self.collector.drain_chunk();
        let writer = self.writer.take().expect("live SRC writer");
        let mut outcome = writer.finish_with_tail(
            serde_json::to_value(self.collector.artifact())?,
            pending,
            final_chunk,
        );
        if let Some(failure) = self.failure.take() {
            outcome.failure.get_or_insert(failure);
            outcome.complete = false;
        }
        let trace = self.collector.artifact();
        let persisted_complete = outcome.complete
            && outcome.checkpoints == trace.drained_checkpoints
            && outcome.chunks == trace.drained_chunks
            && trace.retained_checkpoints == 0;
        let history_complete = trace.history_complete && persisted_complete;
        let artifact = serde_json::json!({
            "schema_version":2, "trace":trace, "stream":outcome,
            "chunks_path":self.chunks_path, "history_complete":history_complete, "qualified":false,
            "limits":{"pending_checkpoints":PENDING_LIMIT,"queued_chunks":4,"record_bytes":MAX_RECORD_BYTES,"stream_bytes":MAX_STREAM_BYTES},
        });
        let result = persist(path, &artifact);
        *report = Some(serde_json::json!({
            "schema_version":2, "path":path, "chunks_path":self.chunks_path,
            "exported":result.is_ok(), "stream_complete":persisted_complete,
            "owner_generation":trace.owner_generation,
            "history_complete":history_complete && result.is_ok(),
            "drained_checkpoints":trace.drained_checkpoints, "checkpoint_limit":trace.checkpoint_limit,
            "refusal":trace.refusal, "qualified":false,
            "stream_failure":outcome.failure,
            "failure":result.as_ref().err().map(|error| format!("{error:#}")),
            "scope":"actual callback scalar history; no source/recorder reference, clean waveform or physical delay claim",
        }));
        let result = result.and_then(|()| {
            anyhow::ensure!(
                persisted_complete,
                "SRC chunk stream incomplete: {}",
                outcome.failure.as_deref().unwrap_or("count mismatch")
            );
            Ok(())
        });
        self.failure = result.as_ref().err().map(|error| format!("{error:#}"));
        self.finished = true;
        result
    }
}

struct CappedArchive {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for CappedArchive {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(std::io::Error::other("SRC trace archive byte cap exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn persist(path: &Path, artifact: &impl serde::Serialize) -> Result<()> {
    // Charge JSON escaping/newline before opening; preserve any partial file
    // on IO failure and never overwrite an earlier run's evidence.
    let mut bytes = CappedArchive {
        bytes: Vec::new(),
        limit: MAX_ARCHIVE_BYTES,
    };
    serde_json::to_writer(&mut bytes, artifact).context("serialize bounded SRC trace")?;
    bytes.write_all(b"\n")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("create new SRC trace")?;
    file.write_all(&bytes.bytes).context("write SRC trace")?;
    file.flush().context("flush SRC trace")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn src_trace_export_is_bounded_create_new_and_preserves_first_evidence() {
        let directory = std::env::temp_dir().join(format!(
            "moiren-src-trace-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("src-trace.json");
        persist(
            &path,
            &serde_json::json!({"qualified":false,"history_complete":false}),
        )
        .unwrap();
        let first = std::fs::read(&path).unwrap();
        assert!(persist(&path, &serde_json::json!({"qualified":true})).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), first);
        let mut bounded = CappedArchive {
            bytes: Vec::new(),
            limit: 8,
        };
        assert!(serde_json::to_writer(&mut bounded, &"123456789").is_err());
        assert!(bounded.bytes.len() <= 8);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn src_stream_exports_complete_ordered_history_beyond_pending_capacity() {
        let mut fixture = fixture::Fixture::new();
        let retained = fixture.observations.retain_session().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "moiren-src-stream-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let summary_path = directory.join("src-trace.json");
        let chunk_path = directory.join("src-trace.chunks.jsonl");
        let mut stream = NativeSrcStream::open(&retained, &chunk_path).unwrap();
        for _ in 0..660 {
            fixture.callbacks(100);
            stream.poll();
            assert!(stream.collector.artifact().retained_checkpoints <= PENDING_LIMIT);
        }
        fixture.close();
        let mut report = None;
        stream.finish(&summary_path, &mut report).unwrap();
        assert_eq!(report.as_ref().unwrap()["history_complete"], true);
        assert_eq!(report.as_ref().unwrap()["qualified"], false);
        let first = std::fs::read(&summary_path).unwrap();
        stream.finish(&summary_path, &mut report).unwrap();
        assert_eq!(std::fs::read(&summary_path).unwrap(), first);
        let summary: serde_json::Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(summary["trace"]["drained_checkpoints"], 132000);
        assert_eq!(summary["stream"]["checkpoints"], 132000);
        assert_eq!(summary["stream"]["complete"], true);
        let mut observed = std::collections::BTreeMap::<String, u64>::new();
        use std::io::BufRead;
        let chunks = std::io::BufReader::new(std::fs::File::open(&chunk_path).unwrap());
        let mut last_record = None;
        for line in chunks.lines() {
            let record: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
            if record["record"] == "chunk" {
                for boundary in record["chunk"]["boundaries"].as_array().unwrap() {
                    let count = observed
                        .entry(boundary["node"].as_str().unwrap().into())
                        .or_default();
                    for point in boundary["checkpoints"].as_array().unwrap() {
                        *count += 1;
                        assert_eq!(point["sequence"], *count);
                    }
                }
            }
            last_record = Some(record);
        }
        assert_eq!(observed.len(), 2);
        assert!(observed.values().all(|count| *count == 66000));
        assert_eq!(last_record.unwrap()["record"], "final");
        assert!(NativeSrcStream::open(&retained, &chunk_path).is_err());
        std::fs::remove_file(summary_path).unwrap();
        std::fs::remove_file(chunk_path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn src_stream_summary_failure_is_sticky_and_preserves_existing_evidence() {
        let mut fixture = fixture::Fixture::new();
        let directory = std::env::temp_dir().join(format!(
            "moiren-src-stream-summary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let summary_path = directory.join("src-trace.json");
        let chunk_path = directory.join("src-trace.chunks.jsonl");
        std::fs::write(&summary_path, b"preserve original").unwrap();
        let mut stream =
            NativeSrcStream::open(&fixture.observations.retain_session().unwrap(), &chunk_path)
                .unwrap();
        fixture.callbacks(20);
        stream.poll();
        fixture.close();
        let mut report = None;
        assert!(stream.finish(&summary_path, &mut report).is_err());
        assert_eq!(report.as_ref().unwrap()["exported"], false);
        assert_eq!(report.as_ref().unwrap()["history_complete"], false);
        let first = report.clone();
        assert!(stream.finish(&summary_path, &mut report).is_err());
        assert_eq!(report, first);
        assert_eq!(std::fs::read(&summary_path).unwrap(), b"preserve original");
        assert!(
            std::fs::read_to_string(&chunk_path)
                .unwrap()
                .contains("\"record\":\"final\"")
        );
        std::fs::remove_file(summary_path).unwrap();
        std::fs::remove_file(chunk_path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn src_stream_writer_failure_exports_failed_chunks_and_refuses_complete_history() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("injected archive failure"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut fixture = fixture::Fixture::new();
        let directory = std::env::temp_dir().join(format!(
            "moiren-src-stream-writer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let summary_path = directory.join("src-trace.json");
        let collector = fixture
            .observations
            .retain_session()
            .unwrap()
            .src_trace_collector(PENDING_LIMIT)
            .unwrap();
        let mut stream = NativeSrcStream {
            collector,
            writer: Some(
                stream::ChunkWriter::with_sink(Broken, MAX_RECORD_BYTES, MAX_STREAM_BYTES).unwrap(),
            ),
            pending: None,
            chunks_path: directory.join("injected-stream.jsonl"),
            failure: None,
            finished: false,
        };
        fixture.callbacks(100);
        stream.poll();
        fixture.close();
        let mut report = None;
        assert!(stream.finish(&summary_path, &mut report).is_err());
        let summary: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary["trace"]["history_complete"], true);
        assert_eq!(summary["history_complete"], false);
        assert_eq!(summary["stream"]["complete"], false);
        assert_eq!(
            summary["stream"]["failed_chunks"][0]["boundaries"][0]["checkpoints"]
                .as_array()
                .unwrap()
                .len(),
            100
        );
        assert_eq!(report.as_ref().unwrap()["exported"], true);
        assert_eq!(report.as_ref().unwrap()["stream_complete"], false);
        assert_eq!(report.as_ref().unwrap()["history_complete"], false);
        assert!(stream.finish(&summary_path, &mut report).is_err());
        std::fs::remove_file(summary_path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
