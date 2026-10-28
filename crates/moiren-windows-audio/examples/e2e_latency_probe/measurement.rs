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

#[cfg(any(windows, test))]
#[path = "measurement_session.rs"]
pub(crate) mod session;

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

#[cfg(windows)]
impl Options {
    fn config(&self) -> session::MeasurementConfig {
        session::MeasurementConfig {
            input_endpoint_id: self.input.clone(),
            max_active_duration: std::time::Duration::from_secs(self.seconds),
            ring_frames: self.ring_frames,
            max_pcm_bytes: self.max_pcm_bytes,
            max_metadata_bytes: self.max_metadata_bytes,
            max_packets: self.max_packets,
        }
    }
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
        let catalog = moiren_windows_audio::catalog_snapshot()?;
        let inputs: Vec<_> = catalog
            .endpoints
            .into_iter()
            .filter(|endpoint| endpoint.flow == "capture")
            .collect();
        println!("{}", serde_json::to_string_pretty(&inputs)?);
        ensure!(
            catalog.errors.is_empty(),
            "capture endpoint enumeration had errors: {}",
            serde_json::to_string(&catalog.errors)?
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
    use session::{MeasurementPaths, MeasurementSession};
    let mut owner = match MeasurementSession::prepare(
        options.config(),
        MeasurementPaths {
            pcm: report.pcm_path.clone(),
            packets: report.packet_path.clone(),
        },
    ) {
        Ok(owner) => owner,
        Err(mut error) => {
            report.native_capture = error
                .native_capture
                .take()
                .map(|capture| serde_json::to_value(*capture))
                .transpose()?;
            return Err(error.error);
        }
    };
    ensure!(
        owner.info().input_endpoint_id == options.input
            && owner.info().ring_frames == options.ring_frames,
        "prepared measurement identity differs from requested input"
    );
    drive_session(
        &mut owner,
        std::time::Duration::from_secs(options.seconds),
        report,
    )
}

#[cfg(any(windows, test))]
trait CollectionSession {
    fn activate(&mut self) -> Result<()>;
    fn check(&self) -> Result<session::MeasurementProgress>;
    fn request_stop(&mut self) -> Result<()>;
    fn finish(&mut self) -> Result<session::MeasurementOutcome>;
}

#[cfg(any(windows, test))]
impl CollectionSession for session::MeasurementSession {
    fn activate(&mut self) -> Result<()> {
        self.activate()
    }
    fn check(&self) -> Result<session::MeasurementProgress> {
        self.check()
    }
    fn request_stop(&mut self) -> Result<()> {
        self.request_stop()
    }
    fn finish(&mut self) -> Result<session::MeasurementOutcome> {
        self.finish()
    }
}

#[cfg(any(windows, test))]
fn drive_session(
    owner: &mut impl CollectionSession,
    duration: std::time::Duration,
    report: &mut MeasurementReport,
) -> Result<()> {
    let started = std::time::Instant::now();
    drive_session_with_clock(
        owner,
        duration,
        report,
        || started.elapsed(),
        std::thread::sleep,
    )
}

#[cfg(any(windows, test))]
fn drive_session_with_clock(
    owner: &mut impl CollectionSession,
    duration: std::time::Duration,
    report: &mut MeasurementReport,
    elapsed: impl Fn() -> std::time::Duration,
    mut sleep: impl FnMut(std::time::Duration),
) -> Result<()> {
    let collection = (|| {
        owner.activate()?;
        let mut polled = false;
        loop {
            // The adapter clock starts before activation, not after its return.
            // Both this conservative ceiling and the session deadline may stop
            // collection; neither grants the other a fresh duration interval.
            if elapsed() >= duration {
                ensure!(
                    polled,
                    "incomplete interval: activation returned after collection deadline"
                );
                return Ok(());
            }
            match owner.check()? {
                session::MeasurementProgress::DeadlineReached => {
                    ensure!(
                        elapsed() >= duration,
                        "incomplete interval: session deadline preceded requested collection interval"
                    );
                    return Ok(());
                }
                session::MeasurementProgress::Starting
                | session::MeasurementProgress::Running { .. } => {}
                session::MeasurementProgress::Prepared | session::MeasurementProgress::Stopping => {
                    anyhow::bail!("incomplete interval: measurement is not collecting");
                }
            }
            polled = true;
            // Re-read after the poll so controller work cannot add a stale sleep.
            let remaining = duration.saturating_sub(elapsed());
            if !remaining.is_zero() {
                sleep(std::time::Duration::from_millis(10).min(remaining));
            }
        }
    })();
    let stopped = owner.request_stop();
    // Cleanup and typed report retention precede any potentially fallible JSON
    // conversion, even when activation, health or the stop request failed.
    let finished = owner.finish();
    for result in [collection, stopped] {
        if let Err(error) = result {
            merge_failure(&mut report.failure, &format!("{error:#}"));
        }
    }
    match finished {
        Ok(outcome) => {
            if let Some(failure) = outcome.failure {
                merge_failure(&mut report.failure, &failure);
            }
            for (destination, result) in [
                (
                    &mut report.native_capture,
                    outcome.native_capture.map(serde_json::to_value).transpose(),
                ),
                (
                    &mut report.sink,
                    outcome.sink.map(serde_json::to_value).transpose(),
                ),
            ] {
                match result {
                    Ok(value) => *destination = value,
                    Err(error) => merge_failure(
                        &mut report.failure,
                        &format!("serialize measurement outcome: {error}"),
                    ),
                }
            }
        }
        Err(error) => merge_failure(
            &mut report.failure,
            &format!("finish measurement session: {error:#}"),
        ),
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn merge_failure(failure: &mut Option<String>, message: &str) {
    if let Some(failure) = failure {
        failure.push_str("; ");
        failure.push_str(message);
    } else {
        *failure = Some(message.to_owned());
    }
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

    #[test]
    fn measurement_adapter_finalizes_after_poll_error_and_retains_unqualified_partial_report() {
        use session::{MeasurementInfo, MeasurementOutcome, MeasurementProgress};
        use std::sync::Mutex;
        struct FakeCollection {
            log: Mutex<Vec<&'static str>>,
        }
        impl CollectionSession for FakeCollection {
            fn activate(&mut self) -> Result<()> {
                self.log.lock().unwrap().push("activate");
                Ok(())
            }
            fn check(&self) -> Result<MeasurementProgress> {
                self.log.lock().unwrap().push("check");
                anyhow::bail!("injected polling failure")
            }
            fn request_stop(&mut self) -> Result<()> {
                self.log.lock().unwrap().push("stop");
                Ok(())
            }
            fn finish(&mut self) -> Result<MeasurementOutcome> {
                self.log.lock().unwrap().push("finish");
                Ok(MeasurementOutcome {
                    info: MeasurementInfo {
                        input_endpoint_id: "independent".into(),
                        sample_rate: 96_000,
                        channels: 1,
                        ring_frames: 2,
                        max_frames: 2,
                    },
                    native_capture: None,
                    sink: Some(crate::measurement_sink::SinkReport {
                        schema_version: 1,
                        sample_rate: 96_000,
                        channels: 1,
                        written_frames: 2,
                        written_pcm_bytes: 8,
                        written_metadata_bytes: 180,
                        complete_packets: 1,
                        sequence_gaps: 0,
                        missing_packets: 0,
                        incomplete_packets: 0,
                        nonfinite_samples: 0,
                        counters_saturated: false,
                        cancelled: false,
                        failure: None,
                    }),
                    activation_elapsed: Some(std::time::Duration::from_secs(1)),
                    failure: Some("retained native failure".into()),
                })
            }
        }
        let mut owner = FakeCollection {
            log: Mutex::new(Vec::new()),
        };
        let mut report = MeasurementReport {
            schema_version: 1,
            mode: "independent_native_measurement",
            qualified: false,
            status: "unqualified",
            input_endpoint_id: "independent".into(),
            requested_seconds: 1,
            dry_run: false,
            pcm_path: "measurement.f32le".into(),
            packet_path: "measurement.packets.jsonl".into(),
            ring_frames: 2,
            max_pcm_bytes: 8,
            max_metadata_bytes: 1024,
            max_packets: 1,
            native_capture: None,
            sink: None,
            failure: None,
            gaps: Vec::new(),
        };
        drive_session(&mut owner, std::time::Duration::from_secs(1), &mut report).unwrap();
        assert_eq!(
            &*owner.log.lock().unwrap(),
            &["activate", "check", "stop", "finish"]
        );
        assert_eq!(report.sink.as_ref().unwrap()["sample_rate"], 96_000);
        assert_eq!(report.sink.as_ref().unwrap()["written_frames"], 2);
        assert!(!report.qualified);
        assert_eq!(report.status, "unqualified");
        let failure = report.failure.unwrap();
        assert!(failure.contains("polling failure") && failure.contains("retained native failure"));
    }

    struct DeadlineCollection {
        log: std::cell::RefCell<Vec<&'static str>>,
        polls: std::cell::Cell<usize>,
        activation_clock: Option<std::rc::Rc<std::cell::Cell<std::time::Duration>>>,
        early_deadline: bool,
    }

    impl CollectionSession for DeadlineCollection {
        fn activate(&mut self) -> Result<()> {
            self.log.borrow_mut().push("activate");
            if let Some(clock) = &self.activation_clock {
                clock.set(std::time::Duration::from_secs(2));
            }
            Ok(())
        }

        fn check(&self) -> Result<session::MeasurementProgress> {
            self.log.borrow_mut().push("check");
            let polls = self.polls.get();
            self.polls.set(polls + 1);
            ensure!(polls == 0, "collection polled after session deadline");
            Ok(if self.early_deadline {
                session::MeasurementProgress::DeadlineReached
            } else {
                session::MeasurementProgress::Running {
                    elapsed: std::time::Duration::ZERO,
                }
            })
        }

        fn request_stop(&mut self) -> Result<()> {
            self.log.borrow_mut().push("stop");
            Ok(())
        }

        fn finish(&mut self) -> Result<session::MeasurementOutcome> {
            self.log.borrow_mut().push("finish");
            Ok(session::MeasurementOutcome {
                info: session::MeasurementInfo {
                    input_endpoint_id: "independent".into(),
                    sample_rate: 96_000,
                    channels: 1,
                    ring_frames: 2,
                    max_frames: 2,
                },
                native_capture: None,
                sink: Some(crate::measurement_sink::SinkReport {
                    schema_version: 1,
                    sample_rate: 96_000,
                    channels: 1,
                    written_frames: 2,
                    written_pcm_bytes: 8,
                    written_metadata_bytes: 180,
                    complete_packets: 1,
                    sequence_gaps: 0,
                    missing_packets: 0,
                    incomplete_packets: 0,
                    nonfinite_samples: 0,
                    counters_saturated: false,
                    cancelled: false,
                    failure: None,
                }),
                activation_elapsed: Some(std::time::Duration::from_secs(1)),
                failure: Some("retained partial native failure".into()),
            })
        }
    }

    fn deadline_report() -> MeasurementReport {
        MeasurementReport {
            schema_version: 1,
            mode: "independent_native_measurement",
            qualified: false,
            status: "unqualified",
            input_endpoint_id: "independent".into(),
            requested_seconds: 1,
            dry_run: false,
            pcm_path: "measurement.f32le".into(),
            packet_path: "measurement.packets.jsonl".into(),
            ring_frames: 2,
            max_pcm_bytes: 8,
            max_metadata_bytes: 1024,
            max_packets: 1,
            native_capture: None,
            sink: None,
            failure: None,
            gaps: Vec::new(),
        }
    }

    #[test]
    fn measurement_adapter_early_session_deadline_stops_without_another_poll() {
        let mut owner = DeadlineCollection {
            log: std::cell::RefCell::new(Vec::new()),
            polls: std::cell::Cell::new(0),
            activation_clock: None,
            early_deadline: true,
        };
        let mut report = deadline_report();
        drive_session_with_clock(
            &mut owner,
            std::time::Duration::from_secs(1),
            &mut report,
            || std::time::Duration::ZERO,
            |_| panic!("must not sleep after session deadline"),
        )
        .unwrap();
        assert_eq!(
            owner.polls.get(),
            1,
            "DeadlineReached must end collection immediately"
        );
        let log = owner.log.borrow();
        let stopped = log.iter().position(|&event| event == "stop").unwrap();
        let finished = log.iter().position(|&event| event == "finish").unwrap();
        assert!(stopped < finished);
        assert_eq!(report.sink.as_ref().unwrap()["written_frames"], 2);
        let failure = report.failure.as_ref().unwrap();
        assert!(failure.contains("incomplete interval"), "{failure}");
        assert!(failure.contains("retained partial native failure"));
        assert!(!failure.contains("polled after session deadline"));
        assert!(!report.qualified);
        assert_eq!(report.status, "unqualified");
    }

    #[test]
    fn measurement_adapter_delayed_activation_gets_no_fresh_interval_or_polls() {
        use std::{cell::Cell, rc::Rc, time::Duration};
        let clock = Rc::new(Cell::new(Duration::ZERO));
        let mut owner = DeadlineCollection {
            log: std::cell::RefCell::new(Vec::new()),
            polls: Cell::new(0),
            activation_clock: Some(clock.clone()),
            early_deadline: false,
        };
        let mut report = deadline_report();
        drive_session_with_clock(
            &mut owner,
            Duration::from_secs(1),
            &mut report,
            || clock.get(),
            |_| panic!("must not sleep after activation consumed the interval"),
        )
        .unwrap();
        assert_eq!(
            owner.polls.get(),
            0,
            "delayed activation must not receive a fresh interval"
        );
        assert_eq!(&*owner.log.borrow(), &["activate", "stop", "finish"]);
        assert!(
            report
                .failure
                .as_ref()
                .unwrap()
                .contains("incomplete interval")
        );
        assert_eq!(report.sink.as_ref().unwrap()["written_frames"], 2);
        assert!(!report.qualified);
    }

    #[test]
    fn measurement_adapter_shorter_cli_bound_stops_before_later_session_deadline() {
        use std::{cell::Cell, time::Duration};
        let clock = Cell::new(Duration::ZERO);
        let sleeps = Cell::new(0);
        let mut owner = DeadlineCollection {
            log: std::cell::RefCell::new(Vec::new()),
            polls: Cell::new(0),
            activation_clock: None,
            early_deadline: false,
        };
        let mut report = deadline_report();
        drive_session_with_clock(
            &mut owner,
            Duration::from_millis(10),
            &mut report,
            || clock.get(),
            |duration| {
                assert_eq!(duration, Duration::from_millis(10));
                sleeps.set(sleeps.get() + 1);
                clock.set(Duration::from_millis(10));
            },
        )
        .unwrap();
        assert_eq!(owner.polls.get(), 1);
        assert_eq!(sleeps.get(), 1);
        assert_eq!(
            &*owner.log.borrow(),
            &["activate", "check", "stop", "finish"]
        );
        assert_eq!(report.sink.as_ref().unwrap()["written_frames"], 2);
        assert!(
            !report
                .failure
                .as_ref()
                .unwrap()
                .contains("incomplete interval")
        );
        assert!(!report.qualified);
    }
}
