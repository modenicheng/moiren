//! Capture-only process output diagnostics, without a graph or physical claim.

use anyhow::{Context, Result, ensure};
#[cfg(any(windows, test))]
use moiren_windows_audio::capture::PcmArtifact;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const MAX_PCM_BYTES: usize = 256 * 1024 * 1024;
const FRAME_BYTES: usize = 2 * size_of::<f32>();
const HELP: &str = "record-process --source-pid PID --source-created-100ns N
    --artifact-dir NEW_DIR --seconds 1..600 --max-frames 1..33554432 [--dry-run]
Records only the selected process tree's output, converted by Windows to
48 kHz stereo f32le. The caller must supply the process creation timestamp.
No render client, source process, volume or default-device write is performed.
The unused compatibility bridge is not the recording tap; its overflow is
separate from PCM artifact loss. Process positions do not prove continuity.
Raw PCM, metadata and partial capture reports are retained in new files.
Exit1 is always unqualified; exit2 means invalid input or report IO failure.";

struct Options {
    pid: u32,
    creation_time_100ns: u64,
    directory: PathBuf,
    seconds: u64,
    max_frames: usize,
    dry_run: bool,
}

fn parse(arguments: Vec<String>) -> Result<Options> {
    let mut values = BTreeMap::new();
    let mut dry_run = false;
    let mut arguments = arguments.into_iter();
    while let Some(flag) = arguments.next() {
        if flag == "--dry-run" {
            ensure!(!dry_run, "duplicate --dry-run");
            dry_run = true;
            continue;
        }
        ensure!(
            [
                "--source-pid",
                "--source-created-100ns",
                "--artifact-dir",
                "--seconds",
                "--max-frames",
            ]
            .contains(&flag.as_str()),
            "unknown process recording option {flag}"
        );
        let value = arguments
            .next()
            .with_context(|| format!("{flag} requires a value"))?;
        ensure!(
            values.insert(flag.clone(), value).is_none(),
            "duplicate {flag}"
        );
    }
    let required = |flag: &str| -> Result<&str> {
        values
            .get(flag)
            .map(String::as_str)
            .with_context(|| format!("missing {flag}"))
    };
    let pid = required("--source-pid")?
        .parse()
        .context("invalid process PID")?;
    let creation_time_100ns = required("--source-created-100ns")?
        .parse()
        .context("invalid process creation timestamp")?;
    let seconds = required("--seconds")?
        .parse()
        .context("invalid recording duration")?;
    let max_frames = required("--max-frames")?
        .parse::<usize>()
        .context("invalid PCM frame cap")?;
    ensure!(
        pid != 0 && creation_time_100ns != 0,
        "process identity must be nonzero"
    );
    ensure!(
        (1..=600).contains(&seconds),
        "recording duration must be 1..600 seconds"
    );
    ensure!(
        max_frames > 0
            && max_frames
                .checked_mul(FRAME_BYTES)
                .is_some_and(|bytes| bytes <= MAX_PCM_BYTES),
        "PCM artifact exceeds its nonzero 256 MiB storage budget"
    );
    let directory = required("--artifact-dir")?;
    ensure!(
        !directory.trim().is_empty() && !directory.contains('\0'),
        "artifact directory must be nonempty without NUL"
    );
    Ok(Options {
        pid,
        creation_time_100ns,
        directory: directory.into(),
        seconds,
        max_frames,
        dry_run,
    })
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    mode: &'static str,
    qualified: bool,
    requested_pid: u32,
    requested_creation_time_100ns: u64,
    selected_process: Option<Value>,
    requested_seconds: u64,
    max_frames: usize,
    max_pcm_bytes: usize,
    dry_run: bool,
    recording_complete: bool,
    artifact_persisted: bool,
    native_capture: Option<Value>,
    unused_bridge: Option<Value>,
    failures: Vec<String>,
    boundaries: [&'static str; 4],
}

pub(crate) fn run(arguments: Vec<String>) -> Result<i32> {
    if arguments.is_empty() || arguments == ["--help"] {
        println!("{HELP}");
        return Ok(0);
    }
    let options = parse(arguments)?;
    ensure!(
        !options.directory.exists(),
        "recording directory already exists"
    );
    // create_dir reserves exactly this new evidence directory, including races.
    fs::create_dir(&options.directory).context("reserve process recording directory")?;
    let mut report = Report {
        schema_version: 1,
        mode: "process_output_recording",
        qualified: false,
        requested_pid: options.pid,
        requested_creation_time_100ns: options.creation_time_100ns,
        selected_process: None,
        requested_seconds: options.seconds,
        max_frames: options.max_frames,
        max_pcm_bytes: options.max_frames * FRAME_BYTES,
        dry_run: options.dry_run,
        recording_complete: false,
        artifact_persisted: false,
        native_capture: None,
        unused_bridge: None,
        failures: Vec::new(),
        boundaries: [
            "Process-tree PCM includes every render stream in that tree, before physical presentation.",
            "No independent source-time reference, hardware capture or calibration is acquired.",
            "Positionless Process metadata cannot establish packet/epoch or full-waveform continuity.",
            "The unconsumed compatibility bridge is separate from the native PCM artifact tap.",
        ],
    };
    if !options.dry_run {
        #[cfg(windows)]
        let result = collect(&options, &mut report);
        #[cfg(not(windows))]
        let result: Result<()> = Err(anyhow::anyhow!("process recording requires Windows"));
        if let Err(error) = result {
            report.failures.push(format!("{error:#}"));
        }
    }
    write_new_json(&options.directory.join("report.json"), &report)?;
    Ok(1)
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

#[cfg(any(windows, test))]
fn persist_artifact(directory: &Path, artifact: &PcmArtifact, max_frames: usize) -> Result<()> {
    let metadata = artifact.metadata();
    ensure!(
        metadata.sample_rate == 48_000
            && metadata.channels == 2
            && metadata.capacity_frames == max_frames
            && metadata.stored_frames <= max_frames as u64
            && metadata.stored_frames.checked_mul(FRAME_BYTES as u64)
                == Some(artifact.pcm_bytes().len() as u64),
        "native PCM format, length or capacity differs from the requested recording"
    );
    let pcm = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("capture.f32le"))
            .context("create process PCM")?;
        file.write_all(artifact.pcm_bytes())
            .context("write process PCM")?;
        file.flush().context("flush process PCM")
    })();
    // Retain metadata even when opening/writing PCM fails.
    let metadata = write_new_json(&directory.join("capture.metadata.json"), &metadata);
    let failures: Vec<_> = [pcm, metadata]
        .into_iter()
        .filter_map(Result::err)
        .map(|error| format!("{error:#}"))
        .collect();
    ensure!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}

#[cfg(windows)]
fn collect(options: &Options, report: &mut Report) -> Result<()> {
    use moiren_windows_audio::{
        ActivationGate, SessionDuration, StopSignal,
        capture::{CaptureStatus, PcmArtifactConfig},
        latency::TelemetryMode,
        process_loopback::{
            ProcessBridgeOptions, ProcessLoopbackOptions, inspect_process,
            prepare_process_capture_with_gate_returning_with_abort,
        },
    };
    use std::{
        thread,
        time::{Duration, Instant},
    };

    let target = inspect_process(options.pid).context("pin process recording target")?;
    report.selected_process = Some(serde_json::to_value(&target)?);
    ensure!(
        target.creation_time_100ns == options.creation_time_100ns,
        "process recording identity changed"
    );
    let stop = StopSignal::new()?;
    let gate = ActivationGate::new(stop.clone())?;
    let prepared = prepare_process_capture_with_gate_returning_with_abort(
        ProcessLoopbackOptions {
            artifact: Some(PcmArtifactConfig::new(options.max_frames)),
            bridge: ProcessBridgeOptions::default(),
            latency_telemetry: TelemetryMode::Summary,
            target,
            duration: SessionDuration::For(Duration::from_secs(options.seconds)),
        },
        stop,
        gate.clone(),
        || (),
    );
    let mut native;
    match prepared {
        Err(mut failure) => {
            for error in [Some(failure.error), failure.stop_error, failure.join_error]
                .into_iter()
                .flatten()
            {
                report.failures.push(error.to_string());
            }
            native = failure.report.take();
        }
        Ok(prepared) => {
            // The artifact copies native packets independently of this graph
            // bridge. Do not run another render/servo just to consume its FIFO.
            drop(prepared.source);
            let activation = gate.activate();
            if let Err(error) = activation {
                report.failures.push(error.to_string());
                if let Err(error) = prepared.session.request_stop() {
                    report.failures.push(error.to_string());
                }
            }
            let deadline = Instant::now() + Duration::from_secs(options.seconds + 15);
            while !prepared.session.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if !prepared.session.is_finished() {
                report
                    .failures
                    .push("capture exceeded its control deadline; Stop requested".into());
                if let Err(error) = prepared.session.request_stop() {
                    report.failures.push(error.to_string());
                }
            }
            let (joined, error) = prepared.session.join_retaining_report();
            native = joined;
            if let Some(error) = error {
                report.failures.push(error.to_string());
            }
            report.unused_bridge = Some(serde_json::to_value(prepared.observer.snapshot())?);
        }
    }
    if let Some(capture) = native.as_mut() {
        let completed = capture.status == CaptureStatus::Completed
            && capture.stream_started
            && capture.stop_succeeded
            && capture.failure.is_none();
        if let Some(artifact) = capture.artifact.take() {
            let metadata = artifact.metadata();
            if let Err(error) = persist_artifact(&options.directory, &artifact, options.max_frames)
            {
                report.failures.push(format!("{error:#}"));
            } else {
                report.artifact_persisted = true;
            }
            report.recording_complete = completed
                && report.failures.is_empty()
                && report.artifact_persisted
                && metadata.stored_frames > 0
                && metadata.dropped_frames == 0
                && metadata.overflow_events == 0;
        }
        report.native_capture = Some(serde_json::to_value(capture)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::Fixture;
    use moiren_windows_audio::capture::{PcmArtifactConfig, PcmRecorder};

    fn artifact() -> PcmArtifact {
        let mut recorder = PcmRecorder::new(PcmArtifactConfig::new(4), 48_000, 2).unwrap();
        let pcm: Vec<_> = [0.25_f32, -0.25, 0.5, -0.5]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        recorder.record_packet(&pcm, 2, 0, 0).unwrap();
        recorder.finish()
    }

    #[test]
    fn process_recording_persists_exact_native_pcm_and_frame_metadata() {
        let fixture = Fixture::new();
        let recording = artifact();
        persist_artifact(&fixture.path("."), &recording, 4).unwrap();
        assert_eq!(
            fs::read(fixture.path("capture.f32le")).unwrap(),
            recording.pcm_bytes()
        );
        let metadata: Value =
            serde_json::from_slice(&fs::read(fixture.path("capture.metadata.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["sample_rate"], 48_000);
        assert_eq!(metadata["stored_frames"], 2);
    }

    #[test]
    fn process_recording_retains_metadata_after_pcm_write_refusal() {
        let fixture = Fixture::new();
        let pcm = fixture.path("capture.f32le");
        fs::write(&pcm, b"retain").unwrap();
        assert!(persist_artifact(&fixture.path("."), &artifact(), 4).is_err());
        assert_eq!(fs::read(pcm).unwrap(), b"retain");
        assert!(fixture.path("capture.metadata.json").exists());
    }

    #[test]
    fn process_recording_refuses_mismatched_native_capacity_before_writing() {
        let fixture = Fixture::new();
        assert!(persist_artifact(&fixture.path("."), &artifact(), 3).is_err());
        assert!(!fixture.path("capture.f32le").exists());
        assert!(!fixture.path("capture.metadata.json").exists());
    }
}
