//! Post-join boundary diagnostics. Export does not certify cleanup or admission.
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::{
    clock_bridge::BridgeObserver,
    process_loopback::{ProcessIdentity, ProcessTraceArtifact},
};
use std::{fs::OpenOptions, io::Write, path::Path};

pub(super) fn export_into(
    report: &mut super::NativeReport,
    paths: &super::ArtifactPaths,
    observer: &BridgeObserver,
    process: ProcessIdentity,
    prepared_demand: u32,
) -> Result<()> {
    match export(paths, observer, process, prepared_demand) {
        Ok(value) => {
            report.process_trace = Some(value);
            Ok(())
        }
        Err(error) => {
            report.process_trace = Some(serde_json::json!({
                "schema_version":1, "exported":false, "qualified":false,
                "failure":format!("{error:#}")
            }));
            Err(error)
        }
    }
}

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

pub(super) fn persist(path: &Path, artifact: &ProcessTraceArtifact) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(artifact)?;
    ensure!(bytes.len() < 1024 * 1024, "Process trace exceeds 1 MiB");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create new Process trace {}", path.display()))?;
    file.write_all(&bytes).context("write Process trace")?;
    file.write_all(b"\n").context("terminate Process trace")?;
    file.flush().context("flush Process trace")
}

#[cfg(test)]
#[path = "process_trace/tests.rs"]
mod tests;
