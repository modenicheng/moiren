//! Poll scalar history independently of synchronous native control queries.
use super::*;
use std::{thread::JoinHandle, time::Duration};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) struct NativeSrcSampler {
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<NativeSrcStream>>,
    archive: Option<NativeSrcStream>,
    failure: Option<String>,
    #[cfg(test)]
    progress: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl NativeSrcSampler {
    pub(crate) fn open(owner: &NativeObservationReadHandle, path: &Path) -> Result<Self> {
        let mut archive = NativeSrcStream::open(owner, path)?;
        let (stop, receiver) = mpsc::channel();
        #[cfg(test)]
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        #[cfg(test)]
        let observed = progress.clone();
        let worker = std::thread::Builder::new()
            .name("src-trace-sampler".into())
            .spawn(move || {
                loop {
                    archive.poll();
                    #[cfg(test)]
                    observed.store(
                        archive
                            .collector
                            .artifact()
                            .boundaries
                            .iter()
                            .map(|boundary| boundary.total_callbacks)
                            .min()
                            .unwrap_or(0),
                        std::sync::atomic::Ordering::Release,
                    );
                    match receiver.recv_timeout(POLL_INTERVAL) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                archive
            })
            .context("spawn SRC trace sampler")?;
        Ok(Self {
            stop: Some(stop),
            worker: Some(worker),
            archive: None,
            failure: None,
            #[cfg(test)]
            progress,
        })
    }

    fn stop_and_join(&mut self) -> Result<()> {
        // The audio owner must join first on normal cleanup. This thread only
        // reads history; joining it does not close or authenticate RT writers.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(archive) => self.archive = Some(archive),
                Err(_) => self.failure = Some("SRC trace sampler panicked".into()),
            }
        }
        if let Some(failure) = &self.failure {
            anyhow::bail!("{failure}");
        }
        Ok(())
    }

    pub(crate) fn finish(
        &mut self,
        path: &Path,
        report: &mut Option<serde_json::Value>,
    ) -> Result<()> {
        if let Err(error) = self.stop_and_join() {
            *report = Some(serde_json::json!({
                "exported":false, "history_complete":false, "qualified":false,
                "failure":format!("{error:#}"),
            }));
            return Err(error);
        }
        self.archive
            .as_mut()
            .context("SRC sampler archive unavailable")?
            .finish(path, report)
    }

    #[cfg(test)]
    pub(super) fn wait_until_collected(&self, sequence: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while self.progress.load(std::sync::atomic::Ordering::Acquire) < sequence {
            assert!(
                std::time::Instant::now() < deadline,
                "sampler did not advance"
            );
            std::thread::yield_now();
        }
    }
}

impl Drop for NativeSrcSampler {
    fn drop(&mut self) {
        // No detached sampler/writer, and no fabricated terminal receipt on an
        // early error. Dropping an unfinished archive retains an incomplete file.
        let _ = self.stop_and_join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moiren-src-sampler-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn early_drop_joins_sampler_without_fabricating_a_terminal_receipt() {
        let mut fixture = fixture::Fixture::new();
        let directory = directory();
        let chunks = directory.join("chunks.jsonl");
        let sampler =
            NativeSrcSampler::open(&fixture.observations.retain_session().unwrap(), &chunks)
                .unwrap();
        fixture.callbacks(50);
        sampler.wait_until_collected(50);
        let progress = sampler.progress.clone();
        drop(sampler);
        assert_eq!(std::sync::Arc::strong_count(&progress), 1);
        assert!(
            !std::fs::read_to_string(&chunks)
                .unwrap()
                .contains("\"record\":\"final\"")
        );
        fixture.close();
        std::fs::remove_file(chunks).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn sampler_join_does_not_replace_boundary_writer_closure() {
        let mut fixture = fixture::Fixture::new();
        let directory = directory();
        let chunks = directory.join("chunks.jsonl");
        let summary = directory.join("summary.json");
        let mut sampler =
            NativeSrcSampler::open(&fixture.observations.retain_session().unwrap(), &chunks)
                .unwrap();
        fixture.callbacks(50);
        sampler.wait_until_collected(50);
        let mut report = None;
        sampler.finish(&summary, &mut report).unwrap();
        assert_eq!(report.as_ref().unwrap()["stream_complete"], true);
        assert_eq!(report.as_ref().unwrap()["history_complete"], false);
        assert_eq!(report.as_ref().unwrap()["refusal"], "open_writer");
        let first = std::fs::read(&summary).unwrap();
        fixture.close();
        sampler.finish(&summary, &mut report).unwrap();
        assert_eq!(std::fs::read(&summary).unwrap(), first);
        std::fs::remove_file(summary).unwrap();
        std::fs::remove_file(chunks).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn sampler_panic_is_sticky_and_cannot_export_success() {
        let mut sampler = NativeSrcSampler {
            stop: None,
            worker: Some(std::thread::spawn(|| panic!("injected sampler failure"))),
            archive: None,
            failure: None,
            progress: Default::default(),
        };
        let directory = directory();
        let summary = directory.join("summary.json");
        let mut report = None;
        for _ in 0..2 {
            assert!(sampler.finish(&summary, &mut report).is_err());
            assert_eq!(report.as_ref().unwrap()["history_complete"], false);
            assert_eq!(report.as_ref().unwrap()["qualified"], false);
            assert!(!summary.exists());
        }
        std::fs::remove_dir(directory).unwrap();
    }
}
