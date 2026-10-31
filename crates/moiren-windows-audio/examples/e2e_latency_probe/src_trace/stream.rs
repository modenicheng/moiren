use anyhow::{Context, Result};
use moiren_windows_audio::engine_backend::NativeSrcTraceChunk;
use serde::Serialize;
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

const QUEUE_CAPACITY: usize = 4;

#[derive(Debug)]
enum Command {
    Chunk(NativeSrcTraceChunk),
    Final(serde_json::Value),
}

#[derive(Serialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum Record<'a> {
    Chunk { chunk: &'a NativeSrcTraceChunk },
    Final { summary: &'a serde_json::Value },
}

#[derive(Debug, Default, Serialize)]
pub(super) struct StreamOutcome {
    pub complete: bool,
    pub chunks: u64,
    pub checkpoints: u64,
    pub bytes: u64,
    pub failure: Option<String>,
    /// Queue/IO failure never silently discards already-drained facts.
    pub failed_chunks: Vec<NativeSrcTraceChunk>,
}

pub(super) struct ChunkWriter {
    sender: Option<mpsc::SyncSender<Command>>,
    worker: Option<JoinHandle<StreamOutcome>>,
    failed: Arc<AtomicBool>,
}

impl ChunkWriter {
    pub(super) fn with_sink(
        sink: impl Write + Send + 'static,
        record_limit: usize,
        byte_limit: u64,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let failed = Arc::new(AtomicBool::new(false));
        let worker_failed = failed.clone();
        let worker = std::thread::Builder::new()
            .name("src-trace-writer".into())
            .spawn(move || write_stream(sink, receiver, worker_failed, record_limit, byte_limit))
            .context("spawn SRC trace writer")?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            failed,
        })
    }

    pub(super) fn try_send(
        &self,
        chunk: NativeSrcTraceChunk,
    ) -> std::result::Result<(), mpsc::TrySendError<NativeSrcTraceChunk>> {
        if self.failed.load(Ordering::Acquire) {
            return Err(mpsc::TrySendError::Disconnected(chunk));
        }
        self.sender
            .as_ref()
            .expect("live writer")
            .try_send(Command::Chunk(chunk))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(Command::Chunk(chunk)) => mpsc::TrySendError::Full(chunk),
                mpsc::TrySendError::Disconnected(Command::Chunk(chunk)) => {
                    mpsc::TrySendError::Disconnected(chunk)
                }
                _ => unreachable!("only a chunk is submitted"),
            })
    }

    /// Only teardown may wait for file IO. Polling uses try_send and keeps a
    /// returned chunk on Full; RT is not connected to this queue or worker.
    #[cfg(test)]
    pub(super) fn finish(
        mut self,
        summary: serde_json::Value,
        pending: Option<NativeSrcTraceChunk>,
    ) -> StreamOutcome {
        self.finish_inner(summary, pending.into_iter().collect())
    }

    pub(super) fn finish_with_tail(
        mut self,
        summary: serde_json::Value,
        pending: Option<NativeSrcTraceChunk>,
        tail: Option<NativeSrcTraceChunk>,
    ) -> StreamOutcome {
        self.finish_inner(summary, pending.into_iter().chain(tail).collect())
    }

    fn finish_inner(
        &mut self,
        summary: serde_json::Value,
        pending: Vec<NativeSrcTraceChunk>,
    ) -> StreamOutcome {
        let mut retained = Vec::new();
        let sender = self.sender.take().expect("live writer");
        for chunk in pending {
            if self.failed.load(Ordering::Acquire) {
                retained.push(chunk);
            } else if let Err(mpsc::SendError(Command::Chunk(chunk))) =
                sender.send(Command::Chunk(chunk))
            {
                retained.push(chunk);
            }
        }
        let final_sent =
            !self.failed.load(Ordering::Acquire) && sender.send(Command::Final(summary)).is_ok();
        drop(sender);
        let mut outcome = self
            .worker
            .take()
            .expect("live worker")
            .join()
            .unwrap_or_else(|_| StreamOutcome {
                failure: Some("SRC trace writer panicked; queued provenance is unavailable".into()),
                ..Default::default()
            });
        outcome.failed_chunks.extend(retained);
        if !final_sent || !outcome.failed_chunks.is_empty() {
            outcome.complete = false;
            outcome
                .failure
                .get_or_insert_with(|| "SRC trace terminal record unavailable".into());
        }
        outcome
    }
}

impl Drop for ChunkWriter {
    fn drop(&mut self) {
        // No detached writer may outlive the evidence owner, even on an early
        // setup/collection failure. Without Final the stream stays incomplete.
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_stream(
    mut sink: impl Write,
    receiver: mpsc::Receiver<Command>,
    failed: Arc<AtomicBool>,
    record_limit: usize,
    byte_limit: u64,
) -> StreamOutcome {
    let mut outcome = StreamOutcome::default();
    let mut generation = None;
    for command in receiver {
        if outcome.failure.is_some() {
            if let Command::Chunk(chunk) = command {
                outcome.failed_chunks.push(chunk);
            }
            continue;
        }
        let terminal = matches!(command, Command::Final(_));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            let mut record = super::CappedArchive {
                bytes: Vec::new(),
                limit: record_limit,
            };
            match &command {
                Command::Chunk(chunk) => {
                    anyhow::ensure!(
                        chunk.sequence == outcome.chunks + 1,
                        "SRC chunk sequence mismatch"
                    );
                    anyhow::ensure!(
                        generation.is_none_or(|value| value == chunk.owner_generation),
                        "SRC chunk owner changed"
                    );
                    serde_json::to_writer(&mut record, &Record::Chunk { chunk })?;
                }
                Command::Final(summary) => {
                    serde_json::to_writer(&mut record, &Record::Final { summary })?;
                }
            }
            record.write_all(b"\n")?;
            anyhow::ensure!(
                record.bytes.len() as u64 <= byte_limit.saturating_sub(outcome.bytes),
                "SRC stream byte cap exceeded"
            );
            let mut remaining = record.bytes.as_slice();
            while !remaining.is_empty() {
                match sink.write(remaining) {
                    Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into()),
                    Ok(written) => {
                        outcome.bytes += written as u64;
                        remaining = &remaining[written..];
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error.into()),
                }
            }
            if terminal {
                sink.flush()?;
            }
            Ok(())
        }))
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "SRC trace writer panicked while writing a record"
            ))
        });
        if let Err(error) = result {
            outcome.failure = Some(format!("{error:#}"));
            failed.store(true, Ordering::Release);
            if let Command::Chunk(chunk) = command {
                outcome.failed_chunks.push(chunk);
            }
        } else if let Command::Chunk(chunk) = command {
            generation = Some(chunk.owner_generation);
            outcome.chunks += 1;
            outcome.checkpoints += chunk
                .boundaries
                .iter()
                .map(|boundary| boundary.checkpoints.len() as u64)
                .sum::<u64>();
        } else {
            outcome.complete = true;
            break;
        }
    }
    if !outcome.complete {
        outcome
            .failure
            .get_or_insert_with(|| "SRC stream ended without a terminal record".into());
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};

    fn chunk(sequence: u64) -> NativeSrcTraceChunk {
        NativeSrcTraceChunk {
            schema_version: 1,
            owner_generation: 7,
            sequence,
            boundaries: Vec::new(),
        }
    }

    #[test]
    fn stream_archive_keeps_ordered_chunks_and_a_terminal_record() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let writer = ChunkWriter::with_sink(Sink(bytes.clone()), 4096, 1 << 20).unwrap();
        for sequence in 1..=600 {
            writer
                .sender
                .as_ref()
                .unwrap()
                .send(Command::Chunk(chunk(sequence)))
                .unwrap();
        }
        let outcome = writer.finish(serde_json::json!({"history_complete":true}), None);
        assert!(outcome.complete, "{:?}", outcome.failure);
        assert_eq!(outcome.chunks, 600);
        let bytes = bytes.lock().unwrap();
        assert_eq!(outcome.bytes, bytes.len() as u64);
        let records: Vec<serde_json::Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 601);
        assert_eq!(records[599]["chunk"]["sequence"], 600);
        assert_eq!(records[600]["record"], "final");
    }

    #[test]
    fn stream_archive_full_queue_returns_the_same_chunk_without_blocking() {
        struct Gate {
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        }
        impl Write for Gate {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer = ChunkWriter::with_sink(
            Gate {
                entered: entered_tx,
                release: release_rx,
            },
            4096,
            1 << 20,
        )
        .unwrap();
        writer.try_send(chunk(1)).unwrap();
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        for sequence in 2..=5 {
            writer.try_send(chunk(sequence)).unwrap();
        }
        let Err(mpsc::TrySendError::Full(pending)) = writer.try_send(chunk(6)) else {
            panic!("bounded queue must refuse")
        };
        assert_eq!(pending.sequence, 6);
        for _ in 0..7 {
            release_tx.send(()).unwrap();
        }
        let outcome = writer.finish(serde_json::json!({"history_complete":true}), Some(pending));
        assert!(outcome.complete);
        assert_eq!(outcome.chunks, 6);
    }

    #[test]
    fn stream_archive_byte_cap_preserves_prior_records_and_failed_chunks() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let writer = ChunkWriter::with_sink(Sink(bytes.clone()), 4096, 180).unwrap();
        writer
            .sender
            .as_ref()
            .unwrap()
            .send(Command::Chunk(chunk(1)))
            .unwrap();
        writer
            .sender
            .as_ref()
            .unwrap()
            .send(Command::Chunk(chunk(2)))
            .unwrap();
        let outcome = writer.finish(serde_json::json!({}), None);
        assert!(!outcome.complete);
        assert!(outcome.failure.is_some());
        assert_eq!(outcome.chunks, 1);
        assert_eq!(outcome.failed_chunks[0].sequence, 2);
        assert!(bytes.lock().unwrap().len() <= 180);
    }

    #[test]
    fn stream_archive_partial_io_flush_and_record_cap_failures_never_claim_complete() {
        struct Broken {
            written: usize,
            partial: bool,
        }
        impl Write for Broken {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if !self.partial {
                    return Ok(data.len());
                }
                if self.written == 0 {
                    self.written = 13;
                    Ok(data.len().min(13))
                } else {
                    Err(std::io::Error::other("injected partial IO failure"))
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("injected flush failure"))
            }
        }
        let writer = ChunkWriter::with_sink(
            Broken {
                written: 0,
                partial: true,
            },
            4096,
            1 << 20,
        )
        .unwrap();
        writer
            .sender
            .as_ref()
            .unwrap()
            .send(Command::Chunk(chunk(1)))
            .unwrap();
        let outcome = writer.finish(serde_json::json!({}), None);
        assert!(!outcome.complete);
        assert_eq!(outcome.bytes, 13);
        assert_eq!(outcome.failed_chunks[0].sequence, 1);
        assert!(outcome.failure.unwrap().contains("partial IO"));

        let writer = ChunkWriter::with_sink(
            Broken {
                written: 0,
                partial: false,
            },
            4096,
            1 << 20,
        )
        .unwrap();
        writer
            .sender
            .as_ref()
            .unwrap()
            .send(Command::Chunk(chunk(1)))
            .unwrap();
        let outcome = writer.finish(serde_json::json!({}), None);
        assert!(!outcome.complete);
        assert_eq!(outcome.chunks, 1);
        assert!(outcome.failure.unwrap().contains("flush failure"));

        let writer = ChunkWriter::with_sink(std::io::sink(), 8, 1 << 20).unwrap();
        writer
            .sender
            .as_ref()
            .unwrap()
            .send(Command::Chunk(chunk(1)))
            .unwrap();
        let outcome = writer.finish(serde_json::json!({}), None);
        assert!(!outcome.complete);
        assert_eq!(outcome.bytes, 0);
        assert_eq!(outcome.failed_chunks[0].sequence, 1);
    }

    #[test]
    fn stream_archive_owner_sequence_and_unfinalized_close_refuse_without_losing_chunks() {
        for changed_owner in [false, true] {
            let writer = ChunkWriter::with_sink(std::io::sink(), 4096, 1 << 20).unwrap();
            writer
                .sender
                .as_ref()
                .unwrap()
                .send(Command::Chunk(chunk(1)))
                .unwrap();
            let mut changed = chunk(if changed_owner { 2 } else { 3 });
            if changed_owner {
                changed.owner_generation = 8;
            }
            writer
                .sender
                .as_ref()
                .unwrap()
                .send(Command::Chunk(changed))
                .unwrap();
            let outcome = writer.finish(serde_json::json!({}), None);
            assert!(!outcome.complete);
            assert_eq!(outcome.chunks, 1);
            assert_eq!(outcome.failed_chunks.len(), 1);
        }
        let (sender, receiver) = mpsc::sync_channel(4);
        sender.send(Command::Chunk(chunk(1))).unwrap();
        drop(sender);
        let outcome = write_stream(
            std::io::sink(),
            receiver,
            Arc::new(AtomicBool::new(false)),
            4096,
            1 << 20,
        );
        assert!(!outcome.complete);
        assert_eq!(outcome.chunks, 1);
        assert!(outcome.failure.unwrap().contains("without a terminal"));
    }

    #[test]
    fn stream_archive_panicked_sink_keeps_the_failed_chunk_and_joins() {
        struct PanickingSink;
        impl Write for PanickingSink {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                panic!("injected sink panic")
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let writer = ChunkWriter::with_sink(PanickingSink, 4096, 1 << 20).unwrap();
        writer
            .sender
            .as_ref()
            .unwrap()
            .send(Command::Chunk(chunk(1)))
            .unwrap();
        let outcome = writer.finish(serde_json::json!({}), None);
        assert!(!outcome.complete);
        assert_eq!(outcome.chunks, 0);
        assert_eq!(outcome.failed_chunks[0].sequence, 1);
        assert!(outcome.failure.unwrap().contains("panicked"));
    }
}
