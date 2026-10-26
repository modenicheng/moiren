//! Bounded control-side diagnostic sampling, not continuous waveform evidence.

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

pub(super) const MAX_RECORD_BYTES: usize = 32 * 1024;

pub(super) struct Journal {
    sender: Option<mpsc::SyncSender<Vec<u8>>>,
    worker: Option<JoinHandle<Result<()>>>,
    cancelled: Arc<AtomicBool>,
    buffer: BoundedRecord,
    samples: Arc<AtomicU64>,
    accepted: u64,
    max_samples: u64,
}

struct BoundedRecord(Vec<u8>);

impl Write for BoundedRecord {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_RECORD_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("diagnostic observation exceeds byte cap"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Journal {
    pub(super) fn create(path: &Path, max_samples: u64) -> Result<Self> {
        ensure!(
            (1..=1442).contains(&max_samples),
            "diagnostic sample cap must be 1..1442"
        );
        Self::from_sink(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| format!("create new diagnostic journal {}", path.display()))?,
            max_samples,
        )
    }

    fn from_sink(sink: impl Write + Send + 'static, max_samples: u64) -> Result<Self> {
        ensure!(
            (1..=1442).contains(&max_samples),
            "diagnostic sample cap must be 1..1442"
        );
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(1);
        let samples = Arc::new(AtomicU64::new(0));
        let written = samples.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        // The sink has one owned, non-RT worker and one pending slot. Control
        // never waits for disk: a stalled sink cannot delay native cancellation.
        let worker = thread::Builder::new()
            .name("moiren-soak-journal".into())
            .spawn(move || {
                let mut sink = sink;
                while let Ok(record) = receiver.recv() {
                    if cancellation.load(Ordering::Acquire) {
                        break;
                    }
                    sink.write_all(&record)
                        .context("write diagnostic JSON record")?;
                    sink.write_all(b"\n")
                        .context("write diagnostic JSON delimiter")?;
                    sink.flush().context("flush diagnostic journal")?;
                    written.fetch_add(1, Ordering::Release);
                }
                Ok(())
            })
            .context("spawn diagnostic journal worker")?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            cancelled,
            buffer: BoundedRecord(Vec::with_capacity(MAX_RECORD_BYTES)),
            samples,
            accepted: 0,
            max_samples,
        })
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("diagnostic journal worker panicked"))??;
        }
        Ok(())
    }

    pub(super) fn check(&self) -> Result<()> {
        ensure!(
            !self.worker.as_ref().is_none_or(JoinHandle::is_finished),
            "diagnostic journal worker stopped before collection finished"
        );
        Ok(())
    }

    pub(super) fn record(&mut self, sample: &impl Serialize) -> Result<()> {
        ensure!(
            self.accepted < self.max_samples,
            "diagnostic sample cap exceeded"
        );
        self.buffer.0.clear();
        // Serialize into bounded control-side scratch first: an oversized
        // observation must not leave a partial JSON record on disk.
        serde_json::to_writer(&mut self.buffer, sample)?;
        self.check()?;
        self.sender
            .as_ref()
            .context("diagnostic journal is closed")?
            .try_send(self.buffer.0.clone())
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    anyhow::anyhow!("diagnostic journal pending-record cap exceeded")
                }
                mpsc::TrySendError::Disconnected(_) => {
                    anyhow::anyhow!("diagnostic journal writer disconnected")
                }
            })?;
        self.accepted += 1;
        Ok(())
    }

    pub(super) fn samples(&self) -> u64 {
        self.samples.load(Ordering::Acquire)
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        // Cancellation closes the bounded input and skips queued records. An
        // in-flight OS write is not force-killed: retain and join its handle.
        // Call finish only after native shutdown; Drop obeys that same order.
        self.cancelled.store(true, Ordering::Release);
        let _ = self.finish();
    }
}

pub(super) fn process_padding_observation(
    observer: &moiren_windows_audio::render::RenderPaddingObserver,
) -> serde_json::Value {
    serde_json::json!(observer.try_snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn process_padding_is_unknown_before_publication_and_after_stop_without_stale_fallback() {
        use moiren_windows_audio::render::{RenderPaddingObserver, RenderPaddingSample};
        let observer = RenderPaddingObserver::new();
        let mut owner = observer.new_owner();
        assert!(process_padding_observation(&observer).is_null());
        assert!(owner.publish(RenderPaddingSample {
            padding_frames: 17,
            capacity_frames: 480,
            sample_rate: Some(48_000),
            period_frames: None,
        }));
        let observed = process_padding_observation(&observer);
        assert_eq!(observed["padding_frames"], 17);
        assert_eq!(observed["capacity_frames"], 480);
        assert_eq!(observed["sample_rate"], 48_000);
        assert!(observed["period_frames"].is_null());
        drop(owner);
        assert!(process_padding_observation(&observer).is_null());
    }

    #[test]
    fn soak_control_cancels_native_owners_while_journal_sink_is_blocked_then_joins_sink() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        use std::time::Duration;
        struct BlockedSink {
            entered: Option<mpsc::SyncSender<()>>,
            release: mpsc::Receiver<()>,
            dropped: Arc<AtomicBool>,
        }
        impl Write for BlockedSink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(entered) = self.entered.take() {
                    entered.send(()).unwrap();
                    self.release.recv().unwrap();
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl Drop for BlockedSink {
            fn drop(&mut self) {
                self.dropped.store(true, Ordering::Release);
            }
        }
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (stopped_tx, stopped_rx) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicBool::new(false));
        let sink = BlockedSink {
            entered: Some(entered_tx),
            release: release_rx,
            dropped: dropped.clone(),
        };
        let control = std::thread::spawn(move || {
            let mut journal = Journal::from_sink(sink, 4).unwrap();
            let mut sampled = false;
            let result = super::super::soak_control::run_until(
                Duration::from_millis(80),
                |_| {
                    if !sampled {
                        journal.record(&json!({"faults": u64::MAX}))?;
                        sampled = true;
                    }
                    Ok(())
                },
                || {
                    stopped_tx.send(()).unwrap();
                    Ok(())
                },
            );
            let written = journal.finish();
            result.and(written).map(|()| journal.samples())
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let independent_stop = stopped_rx.recv_timeout(Duration::from_millis(400));
        release_tx.send(()).unwrap();
        let result = control.join().unwrap();
        assert!(
            independent_stop.is_ok(),
            "blocked sink prevented finite native cancellation"
        );
        assert_eq!(result.unwrap(), 1);
        assert!(
            dropped.load(Ordering::Acquire),
            "sink worker outlived its owner"
        );
    }

    #[test]
    fn journal_drop_cancels_pending_work_and_joins_the_in_flight_sink() {
        use std::time::Duration;
        struct BlockedSink {
            entered: Option<mpsc::SyncSender<()>>,
            release: mpsc::Receiver<()>,
            bytes: Arc<AtomicU64>,
        }
        impl Write for BlockedSink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(entered) = self.entered.take() {
                    entered.send(()).unwrap();
                    self.release.recv().unwrap();
                }
                self.bytes.fetch_add(bytes.len() as u64, Ordering::Release);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (dropping_tx, dropping_rx) = mpsc::sync_channel(1);
        let (dropped_tx, dropped_rx) = mpsc::sync_channel(1);
        let bytes = Arc::new(AtomicU64::new(0));
        let mut journal = Journal::from_sink(
            BlockedSink {
                entered: Some(entered_tx),
                release: release_rx,
                bytes: bytes.clone(),
            },
            4,
        )
        .unwrap();
        journal.record(&json!({"first": 1})).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        journal.record(&json!({"queued": 2})).unwrap();
        let error = journal.record(&json!({"excess": 3})).unwrap_err();
        assert!(error.to_string().contains("pending-record cap"));
        let cancellation = journal.cancelled.clone();
        let owner = thread::spawn(move || {
            dropping_tx.send(()).unwrap();
            drop(journal);
            dropped_tx.send(()).unwrap();
        });
        dropping_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !cancellation.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        let joined_before_release = dropped_rx.try_recv().is_ok();
        release_tx.send(()).unwrap();
        owner.join().unwrap();
        assert!(!joined_before_release, "Drop detached a blocked worker");
        dropped_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            bytes.load(Ordering::Acquire),
            b"{\"first\":1}\n".len() as u64
        );
    }

    #[test]
    fn journal_io_failure_survives_join_and_combined_cleanup_failure() {
        struct FailingSink;
        impl Write for FailingSink {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("injected disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut journal = Journal::from_sink(FailingSink, 1).unwrap();
        journal.record(&json!({"sample": 1})).unwrap();
        let result = super::super::soak_cleanup::combine(
            Err(anyhow::anyhow!("source stop failed; gate close failed")),
            journal.finish(),
        );
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("source stop failed"));
        assert!(error.contains("gate close failed"));
        assert!(error.contains("write diagnostic JSON record"));
        assert!(error.contains("injected disk full"));
        assert_eq!(journal.samples(), 0);
        assert!(journal.check().is_err());
    }

    #[test]
    fn journal_is_collision_safe_and_caps_samples_and_record_bytes() {
        let root = std::env::temp_dir().join(format!("moiren-soak-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let path = root.join("observations.jsonl");
        let mut journal = Journal::create(&path, 2).unwrap();
        assert!(Journal::create(&path, 2).is_err());
        assert!(
            journal
                .record(&json!({"payload": "x".repeat(MAX_RECORD_BYTES)}))
                .is_err()
        );
        journal
            .record(&json!({"sequence": 1, "faults": u64::MAX}))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while journal.samples() < 1 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        journal
            .record(&json!({"sequence": 2, "faults": 7}))
            .unwrap();
        assert!(journal.record(&json!({"sequence": 3})).is_err());
        journal.finish().unwrap();
        drop(journal);
        let contents = fs::read_to_string(&path).unwrap();
        let rows: Vec<serde_json::Value> = contents
            .lines()
            .map(|row| serde_json::from_str(row).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["faults"].as_u64(), Some(u64::MAX));
        assert_eq!(rows[1]["sequence"], 2);
        fs::remove_dir_all(root).unwrap();
    }
}
