//! Control-polled boundary diagnostics. Export certifies neither cleanup nor admission.
use anyhow::{Context, Result};
use moiren_windows_audio::process_loopback::ProcessTraceCollector;
#[cfg(test)]
use moiren_windows_audio::{
    clock_bridge::BridgeObserver,
    process_loopback::{ProcessIdentity, ProcessTraceArtifact},
};
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn export_collector_into(
    report: &mut super::NativeReport,
    paths: &super::ArtifactPaths,
    collector: &ProcessTraceCollector,
) -> Result<()> {
    let result = (|| {
        let artifact = collector.artifact();
        let path = paths
            .process_trace
            .as_deref()
            .context("Process trace path missing")?;
        persist(Path::new(path), &artifact)?;
        let validation = artifact.trace(&artifact.process, artifact.owner_generation);
        Ok(serde_json::json!({
            "schema_version": 2, "exported": true, "path": path,
            "qualified": false, "origin": "caller_supplied_on_import",
            "owner_generation_scope": "single_probe_invocation",
            "writers_finished": artifact.packet_history.writer_finished && artifact.queue_history.writer_finished,
            "collection_refusal": artifact.collection_refusal,
            "replayable": validation.is_ok(), "refusal": validation.err().map(|e| e.to_string()),
            "cleanup_verified": false,
            "prepared_consumer_demand_frames": artifact.prepared_consumer_demand_frames,
            "meaning": "bounded control-polled boundary prefix; prepared whole-wake demand with segmented read events; not sustained admission or sample age; cleanup remains in native reports"
        }))
    })();
    retain_export_result(report, result)
}

fn retain_export_result(
    report: &mut super::NativeReport,
    result: Result<serde_json::Value>,
) -> Result<()> {
    match result {
        Ok(value) => {
            report.process_trace = Some(value);
            Ok(())
        }
        Err(error) => {
            report.process_trace = Some(serde_json::json!({
                "schema_version":2, "exported":false, "qualified":false,
                "failure":format!("{error:#}")
            }));
            Err(error)
        }
    }
}

#[cfg(test)]
pub(super) fn export_into(
    report: &mut super::NativeReport,
    paths: &super::ArtifactPaths,
    observer: &BridgeObserver,
    process: ProcessIdentity,
    prepared_demand: u32,
) -> Result<()> {
    retain_export_result(report, export(paths, observer, process, prepared_demand))
}

#[cfg(test)]
pub(super) fn export(
    paths: &super::ArtifactPaths,
    observer: &BridgeObserver,
    process: ProcessIdentity,
    prepared_demand: u32,
) -> Result<serde_json::Value> {
    // This probe constructs exactly one immutable graph/owner per invocation.
    // Generation 1 is run-scoped, never a production EngineHost generation.
    let artifact = snapshot(observer, process, 1, prepared_demand)?;
    let path = paths
        .process_trace
        .as_deref()
        .context("Process trace path missing")?;
    persist(Path::new(path), &artifact)?;
    let validation = artifact.trace(&artifact.process, artifact.owner_generation);
    Ok(serde_json::json!({
        "schema_version": 1, "exported": true, "path": path,
        "qualified": false, "origin": "caller_supplied_on_import",
        "owner_generation_scope": "single_probe_invocation",
        "writers_finished": artifact.packet_history.writer_finished && artifact.queue_history.writer_finished,
        "replayable": validation.is_ok(), "refusal": validation.err().map(|e|e.to_string()),
        "cleanup_verified": false,
        "meaning": "bounded boundary diagnostics; cleanup remains in native reports; not sustained admission or sample age"
    }))
}

#[cfg(test)]
pub(super) fn snapshot(
    observer: &BridgeObserver,
    process: ProcessIdentity,
    owner_generation: u64,
    prepared_demand: u32,
) -> Result<ProcessTraceArtifact> {
    Ok(ProcessTraceArtifact {
        schema_version: 1,
        process,
        owner_generation,
        input_sample_rate: 48_000,
        consumer_sample_rate: 48_000,
        prepared_consumer_demand_frames: prepared_demand,
        packet_history: observer
            .packet_history()
            .context("packet history contended")?,
        queue_history: observer
            .queue_history()
            .context("queue history contended")?,
    })
}

struct CappedArchive(Vec<u8>);
impl Write for CappedArchive {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_ARCHIVE_BYTES - self.0.len() {
            return Err(std::io::Error::other("Process trace exceeds 16 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn persist(path: &Path, artifact: &impl serde::Serialize) -> Result<()> {
    // Include JSON escaping and the newline before opening any output file.
    let mut bytes = CappedArchive(Vec::new());
    serde_json::to_writer_pretty(&mut bytes, artifact)
        .context("serialize bounded Process trace")?;
    bytes.write_all(b"\n")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create new Process trace {}", path.display()))?;
    file.write_all(&bytes.0).context("write Process trace")?;
    file.flush().context("flush Process trace")
}

/// Finite sessions may finish naturally; only the bounded deadline requests a
/// stop. Supervised sessions keep their existing early-owner-exit hard gate.
pub(super) fn poll_finite(
    duration: Duration,
    poll: impl FnMut(),
    finished: impl FnMut() -> bool,
    stop: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let started = Instant::now();
    poll_finite_with_clock(
        duration,
        poll,
        finished,
        stop,
        || started.elapsed(),
        std::thread::sleep,
    )
}

fn poll_finite_with_clock(
    duration: Duration,
    mut poll: impl FnMut(),
    mut finished: impl FnMut() -> bool,
    stop: impl FnOnce() -> Result<()>,
    elapsed: impl Fn() -> Duration,
    mut sleep: impl FnMut(Duration),
) -> Result<()> {
    loop {
        poll();
        if finished() {
            return Ok(());
        }
        let remaining = duration.saturating_sub(elapsed());
        if remaining.is_zero() {
            return stop();
        }
        sleep(remaining.min(Duration::from_millis(10)));
    }
}

#[cfg(test)]
#[path = "process_trace/tests.rs"]
mod tests;
