//! Bounded external CPU load with explicit completion evidence.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const WORKER_SCHEMA_VERSION: u32 = 1;
const MAX_ID_BYTES: usize = 256;
const STOP_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerResult {
    pub(crate) schema_version: u32,
    pub(crate) load_id: String,
    pub(crate) pid: u32,
    pub(crate) started_unix_seconds: u64,
    pub(crate) ended_unix_seconds: u64,
    pub(crate) requested_seconds: u64,
    pub(crate) elapsed_seconds: f64,
    pub(crate) checksum: u64,
    pub(crate) stop_requested: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Evidence {
    pub(crate) schema_version: u32,
    pub(crate) load_id: String,
    pub(crate) worker_pid: u32,
    pub(crate) requested_seconds: u64,
    pub(crate) elapsed_seconds: f64,
    pub(crate) started_unix_seconds: u64,
    pub(crate) ended_unix_seconds: u64,
    pub(crate) checksum: u64,
    pub(crate) worker_exit_code: Option<i32>,
    pub(crate) required_window_seconds: f64,
    pub(crate) window_complete: bool,
    pub(crate) stop_requested: bool,
    pub(crate) timed_out: bool,
    pub(crate) cleanup_complete: bool,
    pub(crate) valid: bool,
    pub(crate) failure: Option<String>,
}

pub(crate) struct Supervisor {
    load_id: String,
    requested_seconds: u64,
    started: Instant,
    stop_path: PathBuf,
    result_path: PathBuf,
    child: Child,
    exit_code: Option<Option<i32>>,
}

impl Supervisor {
    pub(crate) fn start(
        load_id: &str,
        requested_seconds: u64,
        stop_path: PathBuf,
        result_path: PathBuf,
    ) -> Result<Self> {
        validate_id(load_id)?;
        ensure!(
            (1..=7200).contains(&requested_seconds),
            "controlled load duration must be 1..7200 seconds"
        );
        validate_output_path(&stop_path)?;
        validate_output_path(&result_path)?;
        ensure!(
            stop_path != result_path,
            "controlled load output paths must differ"
        );
        ensure!(
            !stop_path.exists() && !result_path.exists(),
            "controlled load outputs must be new"
        );
        let executable =
            std::env::current_exe().context("resolve controlled load worker executable")?;
        let child = Command::new(executable)
            .arg("controlled-load-worker")
            .arg("--id")
            .arg(load_id)
            .arg("--seconds")
            .arg(requested_seconds.to_string())
            .arg("--stop-file")
            .arg(&stop_path)
            .arg("--result-file")
            .arg(&result_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn controlled load worker")?;
        Ok(Self {
            load_id: load_id.to_owned(),
            requested_seconds,
            started: Instant::now(),
            stop_path,
            result_path,
            child,
            exit_code: None,
        })
    }

    pub(crate) fn check(&mut self) -> Result<()> {
        if self.exit_code.is_none()
            && let Some(status) = self.child.try_wait()?
        {
            self.exit_code = Some(status.code());
        }
        ensure!(
            self.exit_code.is_none(),
            "controlled load worker exited before collection completed"
        );
        Ok(())
    }

    pub(crate) fn finish(mut self, required_window: Duration) -> Result<Evidence> {
        write_stop_signal(&self.stop_path)?;
        let deadline = Instant::now() + STOP_GRACE;
        let mut timed_out = false;
        let exit_code = if let Some(exit_code) = self.exit_code {
            exit_code
        } else {
            loop {
                if let Some(status) = self.child.try_wait()? {
                    break status.code();
                }
                if Instant::now() >= deadline {
                    timed_out = true;
                    self.child
                        .kill()
                        .context("terminate timed-out controlled load worker")?;
                    break self.child.wait()?.code();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let active_window = self.started.elapsed();
        let result = read_worker_result(&self.result_path).ok();
        let mut evidence = build_evidence(EvidenceInput {
            load_id: &self.load_id,
            requested_seconds: self.requested_seconds,
            fallback_pid: self.child.id(),
            result,
            exit_code,
            timed_out,
            required_window,
            active_window,
        });
        let _ = fs::remove_file(&self.stop_path);
        let _ = fs::remove_file(&self.result_path);
        evidence.cleanup_complete = !self.stop_path.exists() && !self.result_path.exists();
        if !evidence.cleanup_complete {
            evidence.valid = false;
            evidence.failure = Some("controlled load temporary files were not cleaned".into());
        }
        Ok(evidence)
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if self.exit_code.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = fs::remove_file(&self.stop_path);
        let _ = fs::remove_file(&self.result_path);
    }
}

pub(crate) fn run_worker(arguments: Vec<String>) -> Result<i32> {
    let (load_id, seconds, stop_path, result_path) = parse_worker(arguments)?;
    let started_unix_seconds = unix_seconds()?;
    let started = Instant::now();
    let mut checksum = 0x1234_5678_9abc_def0u64;
    let mut stop_requested = false;
    while started.elapsed() < Duration::from_secs(seconds) {
        for _ in 0..100_000 {
            checksum ^= checksum << 13;
            checksum ^= checksum >> 7;
            checksum ^= checksum << 17;
        }
        if stop_path.exists() {
            stop_requested = true;
            break;
        }
    }
    let result = WorkerResult {
        schema_version: WORKER_SCHEMA_VERSION,
        load_id,
        pid: std::process::id(),
        started_unix_seconds,
        ended_unix_seconds: unix_seconds()?,
        requested_seconds: seconds,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        checksum,
        stop_requested,
    };
    write_json_new(&result_path, &result)?;
    Ok(0)
}

fn parse_worker(arguments: Vec<String>) -> Result<(String, u64, PathBuf, PathBuf)> {
    ensure!(
        arguments.len() == 8,
        "controlled-load-worker requires four option pairs"
    );
    let mut id = None;
    let mut seconds = None;
    let mut stop_path = None;
    let mut result_path = None;
    let (pairs, remainder) = arguments.as_chunks::<2>();
    ensure!(
        remainder.is_empty(),
        "controlled load worker requires option pairs"
    );
    for pair in pairs {
        match pair[0].as_str() {
            "--id" => ensure!(id.replace(pair[1].clone()).is_none(), "duplicate --id"),
            "--seconds" => ensure!(
                seconds.replace(pair[1].parse()?).is_none(),
                "duplicate --seconds"
            ),
            "--stop-file" => ensure!(
                stop_path.replace(PathBuf::from(&pair[1])).is_none(),
                "duplicate --stop-file"
            ),
            "--result-file" => ensure!(
                result_path.replace(PathBuf::from(&pair[1])).is_none(),
                "duplicate --result-file"
            ),
            flag => bail!("unknown controlled load worker option {flag}"),
        }
    }
    let id = id.context("missing --id")?;
    validate_id(&id)?;
    let seconds = seconds.context("missing --seconds")?;
    ensure!(
        (1..=7200).contains(&seconds),
        "controlled load duration must be 1..7200 seconds"
    );
    let stop_path = stop_path.context("missing --stop-file")?;
    let result_path = result_path.context("missing --result-file")?;
    validate_output_path(&stop_path)?;
    validate_output_path(&result_path)?;
    Ok((id, seconds, stop_path, result_path))
}

fn validate_worker_result(
    result: &WorkerResult,
    expected_id: &str,
    requested_seconds: u64,
    exit_code: Option<i32>,
    timed_out: bool,
) -> Result<()> {
    ensure!(
        result.schema_version == WORKER_SCHEMA_VERSION,
        "unsupported worker result schema"
    );
    ensure!(result.load_id == expected_id, "worker load ID mismatch");
    ensure!(result.pid != 0, "worker PID is missing");
    ensure!(
        result.started_unix_seconds > 0 && result.ended_unix_seconds >= result.started_unix_seconds,
        "worker timestamps are invalid"
    );
    ensure!(
        result.requested_seconds == requested_seconds,
        "worker duration mismatch"
    );
    ensure!(
        result.elapsed_seconds.is_finite() && result.elapsed_seconds > 0.0,
        "worker elapsed duration is invalid"
    );
    ensure!(result.checksum != 0, "worker checksum is empty");
    ensure!(exit_code == Some(0), "worker exit code is not zero");
    ensure!(!timed_out, "worker exceeded supervisor stop grace");
    ensure!(
        result.stop_requested,
        "worker did not observe the supervisor stop"
    );
    Ok(())
}

struct EvidenceInput<'a> {
    load_id: &'a str,
    requested_seconds: u64,
    fallback_pid: u32,
    result: Option<WorkerResult>,
    exit_code: Option<i32>,
    timed_out: bool,
    required_window: Duration,
    active_window: Duration,
}

fn build_evidence(input: EvidenceInput<'_>) -> Evidence {
    let EvidenceInput {
        load_id,
        requested_seconds,
        fallback_pid,
        result,
        exit_code,
        timed_out,
        required_window,
        active_window,
    } = input;
    let elapsed = active_window.as_secs_f64();
    let (worker_pid, started, ended, checksum, stop_requested, failure) = match result {
        Some(result) => (
            result.pid,
            result.started_unix_seconds,
            result.ended_unix_seconds,
            result.checksum,
            result.stop_requested,
            validate_worker_result(&result, load_id, requested_seconds, exit_code, timed_out)
                .err()
                .map(|error| format!("{error:#}")),
        ),
        None => (
            fallback_pid,
            0,
            0,
            0,
            false,
            Some("worker result missing or invalid".into()),
        ),
    };
    let window_complete = active_window >= required_window;
    let valid = failure.is_none()
        && !timed_out
        && exit_code == Some(0)
        && stop_requested
        && window_complete;
    Evidence {
        schema_version: WORKER_SCHEMA_VERSION,
        load_id: load_id.to_owned(),
        worker_pid,
        requested_seconds,
        elapsed_seconds: elapsed,
        required_window_seconds: required_window.as_secs_f64(),
        window_complete,
        started_unix_seconds: started,
        ended_unix_seconds: ended,
        checksum,
        worker_exit_code: exit_code,
        stop_requested,
        timed_out,
        cleanup_complete: false,
        valid,
        failure: if valid {
            None
        } else {
            failure.or_else(|| {
                Some(if !window_complete {
                    "controlled load did not cover the measured collection window".into()
                } else {
                    "controlled load worker did not complete validly".into()
                })
            })
        },
    }
}

fn read_worker_result(path: &Path) -> Result<WorkerResult> {
    let bytes = fs::read(path).with_context(|| format!("read worker result {}", path.display()))?;
    ensure!(
        bytes.len() <= 64 * 1024,
        "worker result exceeds bounded size"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_json_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn write_stop_signal(path: &Path) -> Result<()> {
    write_json_new(path, &serde_json::json!({"stop": true}))
}

fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.contains('\0'),
        "controlled load ID is invalid"
    );
    Ok(())
}

fn validate_output_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty() && !path.to_string_lossy().contains('\0'),
        "controlled load path is invalid"
    );
    ensure!(
        path.parent().is_some_and(|parent| parent.is_dir()),
        "controlled load path parent must exist"
    );
    Ok(())
}

fn unix_seconds() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> WorkerResult {
        WorkerResult {
            schema_version: WORKER_SCHEMA_VERSION,
            load_id: "load-1".into(),
            pid: 42,
            started_unix_seconds: 10,
            ended_unix_seconds: 11,
            requested_seconds: 5,
            elapsed_seconds: 1.0,
            checksum: 7,
            stop_requested: true,
        }
    }

    #[test]
    fn worker_result_requires_clean_supervised_completion() {
        assert!(validate_worker_result(&result(), "load-1", 5, Some(0), false).is_ok());
        for (exit_code, timed_out, stop_requested) in [
            (None, false, true),
            (Some(1), false, true),
            (Some(0), true, true),
            (Some(0), false, false),
        ] {
            let mut value = result();
            value.stop_requested = stop_requested;
            assert!(validate_worker_result(&value, "load-1", 5, exit_code, timed_out).is_err());
        }
    }

    #[test]
    fn worker_result_rejects_stale_identity_and_invalid_shape() {
        for mutate in [
            |value: &mut WorkerResult| value.load_id = "other".into(),
            |value: &mut WorkerResult| value.pid = 0,
            |value: &mut WorkerResult| value.checksum = 0,
            |value: &mut WorkerResult| value.elapsed_seconds = f64::NAN,
            |value: &mut WorkerResult| value.ended_unix_seconds = 9,
        ] {
            let mut value = result();
            mutate(&mut value);
            assert!(validate_worker_result(&value, "load-1", 5, Some(0), false).is_err());
        }
    }

    #[test]
    fn evidence_rejects_missing_result_and_short_coverage() {
        let missing = build_evidence(EvidenceInput {
            load_id: "load-1",
            requested_seconds: 5,
            fallback_pid: 42,
            result: None,
            exit_code: Some(0),
            timed_out: false,
            required_window: Duration::from_secs(2),
            active_window: Duration::from_secs(2),
        });
        assert!(!missing.valid);
        assert_eq!(missing.worker_pid, 42);
        assert_eq!(
            missing.failure.as_deref(),
            Some("worker result missing or invalid")
        );

        let short = build_evidence(EvidenceInput {
            load_id: "load-1",
            requested_seconds: 5,
            fallback_pid: 42,
            result: Some(result()),
            exit_code: Some(0),
            timed_out: false,
            required_window: Duration::from_secs(2),
            active_window: Duration::from_secs(1),
        });
        assert!(!short.valid);
        assert!(!short.window_complete);
        assert_eq!(
            short.failure.as_deref(),
            Some("controlled load did not cover the measured collection window")
        );
    }

    #[test]
    fn evidence_accepts_clean_stop_with_complete_coverage() {
        let evidence = build_evidence(EvidenceInput {
            load_id: "load-1",
            requested_seconds: 5,
            fallback_pid: 42,
            result: Some(result()),
            exit_code: Some(0),
            timed_out: false,
            required_window: Duration::from_secs(1),
            active_window: Duration::from_secs(1),
        });
        assert!(evidence.valid);
        assert!(evidence.window_complete);
        assert!(evidence.failure.is_none());
    }
}
