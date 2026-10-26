//! Independent native measurement capture; never an E2E qualification result.

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Options {
    input: String,
    directory: PathBuf,
    seconds: u64,
    ring_frames: usize,
    max_pcm_bytes: u64,
    max_metadata_bytes: u64,
    max_packets: u64,
    dry_run: bool,
}

fn parse(arguments: Vec<String>) -> Result<Options> {
    let mut options = BTreeMap::new();
    let mut dry_run = false;
    let mut iter = arguments.into_iter();
    while let Some(flag) = iter.next() {
        if flag == "--dry-run" {
            ensure!(!dry_run, "duplicate --dry-run");
            dry_run = true;
            continue;
        }
        ensure!(
            [
                "--input",
                "--artifact-dir",
                "--seconds",
                "--ring-frames",
                "--max-pcm-bytes",
                "--max-metadata-bytes",
                "--max-packets",
            ]
            .contains(&flag.as_str()),
            "unknown measurement option {flag}"
        );
        let value = iter
            .next()
            .with_context(|| format!("{flag} requires a value"))?;
        ensure!(
            options.insert(flag.clone(), value).is_none(),
            "duplicate {flag}"
        );
    }
    let required = |name: &str| -> Result<&str> {
        options
            .get(name)
            .map(String::as_str)
            .with_context(|| format!("missing {name}"))
    };
    let input = required("--input")?.to_owned();
    let directory = PathBuf::from(required("--artifact-dir")?);
    let seconds = required("--seconds")?
        .parse()
        .context("invalid measurement duration")?;
    let ring_frames: usize = required("--ring-frames")?
        .parse()
        .context("invalid evidence ring capacity")?;
    let max_pcm_bytes = required("--max-pcm-bytes")?
        .parse()
        .context("invalid PCM byte cap")?;
    let max_metadata_bytes = required("--max-metadata-bytes")?
        .parse()
        .context("invalid metadata byte cap")?;
    let max_packets = required("--max-packets")?
        .parse()
        .context("invalid packet cap")?;
    ensure!(
        !input.trim().is_empty() && !input.contains('\0'),
        "--input must be an explicit endpoint ID without NUL"
    );
    ensure!(
        !directory.as_os_str().is_empty() && !required("--artifact-dir")?.trim().is_empty(),
        "--artifact-dir must be nonempty"
    );
    ensure!(
        (1..=7200).contains(&seconds),
        "measurement duration must be 1..7200 seconds"
    );
    ensure!(
        ring_frames > 0
            && ring_frames
                .checked_mul(size_of::<moiren_windows_audio::capture::PcmEvidenceFrame>())
                .is_some_and(|bytes| bytes <= 8 * 1024 * 1024),
        "evidence ring exceeds its nonzero 8 MiB storage budget"
    );
    ensure!(
        (1..=3 * 1024 * 1024 * 1024).contains(&max_pcm_bytes),
        "PCM byte cap must be 1..3GiB"
    );
    ensure!(
        (1..=256 * 1024 * 1024).contains(&max_metadata_bytes),
        "metadata byte cap must be 1..256MiB"
    );
    ensure!(
        (1..=1_000_000).contains(&max_packets),
        "packet cap must be 1..1000000"
    );
    Ok(Options {
        input,
        directory,
        seconds,
        ring_frames,
        max_pcm_bytes,
        max_metadata_bytes,
        max_packets,
        dry_run,
    })
}

const HELP: &str = "record-measurement --input ENDPOINT --artifact-dir NEW_DIR --seconds 1..7200
    --ring-frames N --max-pcm-bytes N --max-metadata-bytes N --max-packets N [--dry-run]
Records independent WASAPI capture as bounded raw f32le + packet JSONL.
Native rate/channels are negotiated; PCM is NOT necessarily 48 kHz/stereo.
No source is launched and no physical boundary/calibration is inferred.
Run alongside the source/graph externally. Exit1 is always unqualified;
exit2 means invalid input or report IO failure.";

#[derive(Serialize)]
struct MeasurementReport {
    schema_version: u32,
    mode: &'static str,
    qualified: bool,
    status: &'static str,
    input_endpoint_id: String,
    requested_seconds: u64,
    dry_run: bool,
    pcm_path: PathBuf,
    packet_path: PathBuf,
    ring_frames: usize,
    max_pcm_bytes: u64,
    max_metadata_bytes: u64,
    max_packets: u64,
    native_capture: Option<Value>,
    sink: Option<Value>,
    failure: Option<String>,
    gaps: Vec<&'static str>,
}

pub(crate) fn run(arguments: Vec<String>) -> Result<i32> {
    if arguments.is_empty() || arguments == ["--help"] {
        println!("{HELP}");
        return Ok(0);
    }
    let options = parse(arguments)?;
    // Reserve a new directory, retaining failed artifacts instead of reusing
    // or overwriting a partly completed run's evidence path.
    ensure!(
        !options.directory.exists(),
        "measurement directory already exists"
    );
    fs::create_dir(&options.directory).context("reserve new measurement directory")?;
    let mut report = MeasurementReport {
        schema_version: 1,
        mode: "independent_native_measurement",
        qualified: false,
        status: "unqualified",
        input_endpoint_id: options.input.clone(),
        requested_seconds: options.seconds,
        dry_run: options.dry_run,
        pcm_path: options.directory.join("measurement.f32le"),
        packet_path: options.directory.join("measurement.packets.jsonl"),
        ring_frames: options.ring_frames,
        max_pcm_bytes: options.max_pcm_bytes,
        max_metadata_bytes: options.max_metadata_bytes,
        max_packets: options.max_packets,
        native_capture: None,
        sink: None,
        failure: None,
        gaps: vec![
            "Standalone capture does not launch or supervise a source, graph or controlled load.",
            "Physical boundary, calibration, marker alignment and file hashes require separate verification.",
            "QPC/device positions and capture continuity do not qualify physical end-to-end latency.",
            "The unused compatibility monitoring bridge is not the measurement tap; its overflow is not waveform loss.",
        ],
    };
    if !options.dry_run {
        #[cfg(windows)]
        if let Err(error) = collect_native(&options, &mut report) {
            report.failure = Some(format!("{error:#}"));
        }
        #[cfg(not(windows))]
        {
            report.failure = Some("native measurement requires Windows".into());
        }
    }
    write_report(&options.directory.join("measurement.json"), &report)?;
    Ok(1)
}

pub(crate) fn list_inputs() -> Result<i32> {
    #[cfg(windows)]
    {
        println!(
            "{}",
            serde_json::to_string_pretty(&moiren_windows_audio::capture::list_capture_endpoints()?)?
        );
        Ok(0)
    }
    #[cfg(not(windows))]
    anyhow::bail!("capture endpoint enumeration requires Windows")
}

fn write_report(path: &Path, report: &MeasurementReport) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create measurement report {}", path.display()))?;
    serde_json::to_writer_pretty(&mut file, report).context("serialize measurement report")?;
    file.write_all(b"\n")?;
    file.flush().context("flush measurement report")
}

#[cfg(windows)]
fn collect_native(options: &Options, report: &mut MeasurementReport) -> Result<()> {
    use crate::measurement_sink::{EvidenceSink, SinkLimits};
    use moiren_windows_audio::{
        ActivationGate, SessionDuration, StopSignal,
        capture::{CaptureOptions, PcmEvidenceConfig, prepare_capture_native_with_gate},
        latency::{PreparedInputQueue, TelemetryMode},
    };
    use std::time::Duration;
    // Declare sink ownership first: unwinding also drops/joins the native
    // session before a potentially stalled sink worker can be joined.
    let mut sink_owner = None;
    let stop = StopSignal::new().context("create independent measurement stop")?;
    let gate = ActivationGate::new(stop.clone()).context("create measurement activation gate")?;
    let native = prepare_capture_native_with_gate(
        CaptureOptions {
            artifact: None,
            latency_telemetry: TelemetryMode::Summary,
            shared_period: Default::default(),
            endpoint_id: options.input.clone(),
            duration: SessionDuration::UntilStopped,
        },
        stop.clone(),
        gate.clone(),
    )
    .context("prepare independent native capture")?;
    let info = native.info();
    let (capture, reader) = native
        .attach_with_evidence(
            PreparedInputQueue::compatible(),
            PcmEvidenceConfig {
                capacity_frames: options.ring_frames,
                detect_position_gaps: true,
            },
        )
        .context("attach bounded measurement tap")?;
    // Duration is controlled independently. Derive the frame cap from the
    // caller's byte budget, not a nominal clock count that drifts over hours.
    let max_frames = options.max_pcm_bytes / (info.channels as u64 * 4);
    sink_owner.replace(
        EvidenceSink::create(
            reader,
            info.timing.sample_rate,
            info.channels,
            &report.pcm_path,
            &report.packet_path,
            SinkLimits {
                max_frames,
                max_pcm_bytes: options.max_pcm_bytes,
                max_metadata_bytes: options.max_metadata_bytes,
                max_packets: options.max_packets,
            },
        )
        .context("prepare bounded non-RT measurement sink")?,
    );
    let sink = sink_owner.as_mut().expect("prepared measurement sink");
    // Source/observer stay alive through native join, but never enter a graph.
    let collection = (|| {
        gate.activate()
            .context("activate independent measurement")?;
        crate::live::soak_control::run_until(
            Duration::from_secs(options.seconds),
            |_| {
                sink.check()?;
                ensure!(
                    !capture.session.is_finished(),
                    "measurement owner stopped before deadline"
                );
                Ok(())
            },
            || {
                stop.request_stop()
                    .context("stop independent measurement owner")
            },
        )
    })();
    // Native cancellation and join precede sink join even with blocked disk IO.
    let stopped = stop
        .request_stop()
        .context("cancel measurement after collection");
    let native_result = capture.session.join().context("join measurement owner");
    let mut failures = Vec::new();
    for result in [collection, stopped] {
        if let Err(error) = result {
            failures.push(format!("{error:#}"));
        }
    }
    let mut evidence_summary = None;
    match native_result {
        Ok(native) => {
            if let Some(failure) = &native.failure {
                failures.push(failure.clone());
            }
            if !native.stream_started || !native.stop_succeeded {
                failures.push("measurement native stream did not start and stop cleanly".into());
            }
            if let Some(summary) = native.evidence_metadata {
                evidence_summary = Some(summary);
                if summary.dropped_frames != 0
                    || summary.overflow_packets != 0
                    || summary.reader_abandoned_packets != 0
                    || summary.gap_events != 0
                    || summary.discontinuity_events != 0
                    || summary.timestamp_error_packets != 0
                    || summary.position_overflow_packets != 0
                    || summary.counters_saturated
                    || summary.sequence_exhausted
                {
                    failures.push(
                        "measurement evidence has loss or invalid continuity/timestamps".into(),
                    );
                }
            } else {
                failures.push("measurement evidence summary is missing".into());
            }
            report.native_capture = Some(serde_json::to_value(native)?);
        }
        Err(error) => failures.push(format!("{error:#}")),
    }
    // Native EOF is now final. Drain even a failed run instead of discarding
    // queued waveform merely because a quality counter failed.
    match sink.finish() {
        Ok(sink_report) => {
            if let Some(failure) = &sink_report.failure {
                failures.push(failure.clone());
            }
            if let Some(summary) = evidence_summary
                && (summary.stored_frames == 0
                    || sink_report.written_frames != summary.stored_frames
                    || sink_report.complete_packets != summary.packets
                    || sink_report.nonfinite_samples != 0
                    || sink_report.missing_packets != 0
                    || sink_report.incomplete_packets != 0
                    || sink_report.counters_saturated
                    || sink_report.cancelled)
            {
                failures.push(
                    "measurement sink and final native evidence are not complete and consistent"
                        .into(),
                );
            }
            report.sink = Some(serde_json::to_value(sink_report)?);
        }
        Err(error) => failures.push(format!("join measurement sink: {error:#}")),
    }
    ensure!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Vec<String> {
        [
            "--input",
            "explicit-device",
            "--artifact-dir",
            "new-run",
            "--seconds",
            "7200",
            "--max-pcm-bytes",
            "2764800000",
            "--max-metadata-bytes",
            "200000000",
            "--max-packets",
            "800000",
            "--ring-frames",
            "48000",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn measurement_admits_explicit_bounded_two_hour_capture_without_fake_physical_claim() {
        let options = parse(args()).unwrap();
        assert_eq!(options.input, "explicit-device");
        assert_eq!(options.directory, PathBuf::from("new-run"));
        assert_eq!(options.seconds, 7200);
        assert_eq!(options.ring_frames, 48000);
        assert_eq!(options.max_pcm_bytes, 2_764_800_000);
        assert_eq!(options.max_metadata_bytes, 200_000_000);
        assert_eq!(options.max_packets, 800_000);
    }

    #[test]
    fn measurement_refuses_implicit_unbounded_and_duplicate_inputs() {
        for (flag, value) in [
            ("--seconds", "0"),
            ("--seconds", "7201"),
            ("--ring-frames", "0"),
            ("--ring-frames", "999999999"),
            ("--max-pcm-bytes", "0"),
            ("--max-pcm-bytes", "3221225473"),
            ("--max-metadata-bytes", "268435457"),
            ("--max-packets", "1000001"),
            ("--input", " "),
        ] {
            let mut arguments = args();
            let index = arguments.iter().position(|arg| arg == flag).unwrap();
            arguments[index + 1] = value.into();
            assert!(parse(arguments).is_err(), "accepted {flag} {value}");
        }
        let mut duplicate = args();
        duplicate.extend(["--seconds".into(), "1".into()]);
        assert!(parse(duplicate).is_err());
        let mut missing = args();
        missing.drain(..2);
        assert!(parse(missing).is_err());
        let mut unknown = args();
        unknown.extend(["--physical-qualified".into(), "true".into()]);
        assert!(parse(unknown).is_err());
    }

    #[test]
    fn measurement_dry_run_retains_unqualified_manifest_and_never_overwrites() {
        let directory =
            std::env::temp_dir().join(format!("moiren-measurement-dry-{}", std::process::id()));
        assert!(!directory.exists());
        let mut arguments = args();
        let index = arguments
            .iter()
            .position(|arg| arg == "--artifact-dir")
            .unwrap();
        arguments[index + 1] = directory.to_string_lossy().into_owned();
        arguments.push("--dry-run".into());
        assert_eq!(run(arguments.clone()).unwrap(), 1);
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("measurement.json")).unwrap())
                .unwrap();
        assert_eq!(report["qualified"], false);
        assert_eq!(report["status"], "unqualified");
        assert_eq!(report["requested_seconds"], 7200);
        assert_eq!(report["input_endpoint_id"], "explicit-device");
        assert!(report["native_capture"].is_null());
        assert!(report["sink"].is_null());
        assert!(!directory.join("measurement.f32le").exists());
        assert!(run(arguments).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
