//! Bounded external CPU load with explicit completion evidence.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::process::{Command, Stdio};
#[cfg(any(windows, test))]
use std::{fs, process::Child};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const WORKER_SCHEMA_VERSION: u32 = 1;
const MAX_ID_BYTES: usize = 256;
#[cfg(any(windows, test))]
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

// Native supervision is Windows-only; keep evidence validation portable in tests.
#[cfg(any(windows, test))]
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

#[cfg(any(windows, test))]
pub(crate) trait LoadChild {
    fn try_wait(&mut self) -> std::io::Result<Option<Option<i32>>>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<Option<i32>>;
}

#[cfg(any(windows, test))]
impl LoadChild for Child {
    fn try_wait(&mut self) -> std::io::Result<Option<Option<i32>>> {
        Child::try_wait(self).map(|status| status.map(|status| status.code()))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Child::kill(self)
    }
    fn wait(&mut self) -> std::io::Result<Option<i32>> {
        Child::wait(self).map(|status| status.code())
    }
}

#[cfg(any(windows, test))]
pub(crate) struct Supervisor<C: LoadChild = Child> {
    load_id: String,
    requested_seconds: u64,
    started: Instant,
    stop_path: PathBuf,
    result_path: PathBuf,
    child: C,
    child_pid: u32,
    exit_code: Option<Option<i32>>,
    stop_request: Option<std::result::Result<(), String>>,
}

#[cfg(windows)]
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
            child_pid: child.id(),
            child,
            exit_code: None,
            stop_request: None,
        })
    }
}

#[cfg(any(windows, test))]
impl<C: LoadChild> Supervisor<C> {
    pub(crate) fn check(&mut self) -> Result<()> {
        if self.exit_code.is_none()
            && let Some(status) = self.child.try_wait()?
        {
            self.exit_code = Some(status);
        }
        ensure!(
            self.exit_code.is_none(),
            "controlled load worker exited before collection completed"
        );
        Ok(())
    }

    pub(crate) fn request_stop(&mut self) -> Result<()> {
        // A repeated broadcast must neither recreate a token nor erase an earlier
        // write failure after another owner has already begun retirement.
        self.stop_request
            .get_or_insert_with(|| {
                write_stop_signal(&self.stop_path)
                    .context("signal controlled load stop")
                    .map_err(|error| format!("{error:#}"))
            })
            .as_ref()
            .copied()
            .map_err(|error| anyhow::anyhow!(error.clone()))
    }

    pub(crate) fn finish(self, required_window: Duration) -> Result<Evidence> {
        self.finish_with_grace(required_window, STOP_GRACE)
    }

    fn finish_with_grace(mut self, required_window: Duration, grace: Duration) -> Result<Evidence> {
        let mut failure = self.request_stop().err().map(|error| format!("{error:#}"));
        let (timed_out, reap_failure) = self.reap(grace);
        if let Some(error) = reap_failure {
            append_failure(&mut failure, error);
        }
        let active_window = self.started.elapsed();
        let result = match read_worker_result(&self.result_path) {
            Ok(result) => Some(result),
            Err(error) => {
                append_failure(&mut failure, format!("{error:#}"));
                None
            }
        };
        let mut evidence = build_evidence(EvidenceInput {
            load_id: &self.load_id,
            requested_seconds: self.requested_seconds,
            fallback_pid: self.child_pid,
            result,
            exit_code: self.exit_code.flatten(),
            timed_out,
            required_window,
            active_window,
        });
        if let Some(error) = evidence.failure.take() {
            append_failure(&mut failure, error);
        }
        // Valid receipts keep the legacy cleanup contract. Failed raw evidence
        // remains at the caller's known path, including when Drop is needed.
        if matches!(self.stop_request, Some(Ok(())))
            && let Err(error) = fs::remove_file(&self.stop_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            append_failure(
                &mut failure,
                format!("remove controlled load stop token: {error}"),
            );
        }
        if failure.is_none()
            && let Err(error) = fs::remove_file(&self.result_path)
        {
            append_failure(
                &mut failure,
                format!("remove controlled load result: {error}"),
            );
        }
        evidence.cleanup_complete =
            self.exit_code.is_some() && !self.stop_path.exists() && !self.result_path.exists();
        evidence.valid &= failure.is_none() && evidence.cleanup_complete;
        evidence.failure = failure;
        Ok(evidence)
    }

    fn reap(&mut self, grace: Duration) -> (bool, Option<String>) {
        if self.exit_code.is_some() {
            return (false, None);
        }
        let deadline = Instant::now() + grace;
        let mut failure = None;
        let timed_out = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.exit_code = Some(status);
                    return (false, None);
                }
                Ok(None) => {}
                Err(error) => {
                    append_failure(
                        &mut failure,
                        format!("poll controlled load worker: {error}"),
                    );
                    break false;
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break true;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(20)));
        };
        // Even a failed token, poll or kill must not bypass the wait attempt.
        if let Err(error) = self.child.kill() {
            append_failure(
                &mut failure,
                format!("terminate controlled load worker: {error}"),
            );
        }
        match self.child.wait() {
            Ok(status) => self.exit_code = Some(status),
            Err(error) => append_failure(
                &mut failure,
                format!("wait for controlled load worker: {error}"),
            ),
        }
        (timed_out, failure)
    }
}

#[cfg(any(windows, test))]
impl<C: LoadChild> Drop for Supervisor<C> {
    fn drop(&mut self) {
        if self.exit_code.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if matches!(self.stop_request, Some(Ok(()))) {
            let _ = fs::remove_file(&self.stop_path);
        }
    }
}

#[cfg(any(windows, test))]
fn append_failure(failure: &mut Option<String>, error: String) {
    match failure {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&error);
        }
        None => *failure = Some(error),
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

#[cfg(any(windows, test))]
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

#[cfg(any(windows, test))]
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

#[cfg(any(windows, test))]
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
                .and_then(|()| {
                    ensure!(
                        result.pid == fallback_pid,
                        "worker PID mismatch: expected {fallback_pid}, received {}",
                        result.pid
                    );
                    Ok(())
                })
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

#[cfg(any(windows, test))]
fn read_worker_result(path: &Path) -> Result<WorkerResult> {
    let file =
        fs::File::open(path).with_context(|| format!("read worker result {}", path.display()))?;
    let length = file.metadata()?.len();
    read_worker_result_from(file, length)
}

#[cfg(any(windows, test))]
fn read_worker_result_from(reader: impl std::io::Read, length: u64) -> Result<WorkerResult> {
    use std::io::Read;

    const MAX_RESULT_BYTES: usize = 64 * 1024;
    ensure!(
        length <= MAX_RESULT_BYTES as u64,
        "worker result exceeds bounded size"
    );
    // Metadata is not a growth guarantee. Bound both allocation and subsequent
    // reads, retaining one extra byte solely to detect a growing receipt.
    let mut bytes = vec![0; MAX_RESULT_BYTES + 1];
    let mut reader = reader.take(bytes.len() as u64);
    let mut length = 0;
    loop {
        match reader.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(read) => length += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("read bounded worker result"),
        }
    }
    ensure!(
        length <= MAX_RESULT_BYTES,
        "worker result exceeds bounded size"
    );
    serde_json::from_slice(&bytes[..length]).context("decode controlled load worker result")
}

fn write_json_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

#[cfg(any(windows, test))]
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
    use std::{cell::Cell, io, rc::Rc};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "moiren-controlled-load-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            // Only this test-created directory and its known files are removed.
            for name in ["stop.json", "result.json"] {
                let _ = std::fs::remove_file(self.0.join(name));
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    #[derive(Default)]
    struct ChildCalls {
        polls: Cell<usize>,
        kills: Cell<usize>,
        waits: Cell<usize>,
    }

    struct FakeChild {
        calls: Rc<ChildCalls>,
        ready: bool,
        poll_error: bool,
        kill_error: bool,
        wait_error: bool,
        code: i32,
    }

    impl LoadChild for FakeChild {
        fn try_wait(&mut self) -> io::Result<Option<Option<i32>>> {
            self.calls.polls.set(self.calls.polls.get() + 1);
            if self.poll_error {
                return Err(io::Error::other("injected poll error"));
            }
            Ok(self.ready.then_some(Some(self.code)))
        }
        fn kill(&mut self) -> io::Result<()> {
            self.calls.kills.set(self.calls.kills.get() + 1);
            if self.kill_error {
                return Err(io::Error::other("injected kill error"));
            }
            Ok(())
        }
        fn wait(&mut self) -> io::Result<Option<i32>> {
            self.calls.waits.set(self.calls.waits.get() + 1);
            if self.wait_error {
                return Err(io::Error::other("injected wait error"));
            }
            Ok(Some(self.code))
        }
    }

    fn supervisor(directory: &TestDirectory) -> (Supervisor<FakeChild>, Rc<ChildCalls>) {
        let calls = Rc::new(ChildCalls::default());
        (
            Supervisor {
                load_id: "load-1".into(),
                requested_seconds: 5,
                started: Instant::now() - Duration::from_secs(1),
                stop_path: directory.0.join("stop.json"),
                result_path: directory.0.join("result.json"),
                child: FakeChild {
                    calls: calls.clone(),
                    ready: true,
                    poll_error: false,
                    kill_error: false,
                    wait_error: false,
                    code: 0,
                },
                child_pid: 42,
                exit_code: None,
                stop_request: None,
            },
            calls,
        )
    }

    #[test]
    fn failed_stop_token_still_reaps_each_owner_and_retains_exit_and_raw_result() {
        let directories = [TestDirectory::new(), TestDirectory::new()];
        for directory in &directories {
            let (mut owner, calls) = supervisor(directory);
            owner.stop_path = directory.0.join("missing-parent/stop.json");
            owner.child.code = 23;
            write_json_new(&owner.result_path, &result()).unwrap();
            let outcome = owner.finish(Duration::ZERO);
            assert_eq!(calls.polls.get(), 1, "stop failure skipped child reap");
            assert_eq!(
                calls.kills.get(),
                0,
                "already exited child was killed in Drop"
            );
            let evidence = outcome.expect("stop failure must return partial evidence");
            assert_eq!(evidence.worker_exit_code, Some(23));
            assert!(!evidence.valid);
            assert!(evidence.failure.as_deref().unwrap().contains("stop"));
            assert!(directory.0.join("result.json").is_file());
        }
    }

    #[test]
    fn repeated_stop_signals_once_and_does_not_join_before_finish() {
        let directory = TestDirectory::new();
        let (mut owner, calls) = supervisor(&directory);
        owner.request_stop().unwrap();
        let original = std::fs::read(&owner.stop_path).unwrap();
        owner
            .request_stop()
            .expect("second stop must be idempotent");
        assert_eq!(std::fs::read(&owner.stop_path).unwrap(), original);
        assert_eq!(calls.polls.get() + calls.waits.get() + calls.kills.get(), 0);
        write_json_new(&owner.result_path, &result()).unwrap();
        let evidence = owner.finish(Duration::ZERO).unwrap();
        assert!(evidence.valid);
        assert!(evidence.cleanup_complete);
        assert!(!directory.0.join("result.json").exists());
        assert_eq!(calls.polls.get(), 1);
    }

    #[test]
    fn failed_stop_is_latched_even_if_path_becomes_writable() {
        let directory = TestDirectory::new();
        let (mut owner, calls) = supervisor(&directory);
        write_json_new(&owner.stop_path, &serde_json::json!({"foreign": true})).unwrap();
        let first = owner.request_stop().unwrap_err().to_string();
        std::fs::remove_file(&owner.stop_path).unwrap();
        assert_eq!(owner.request_stop().unwrap_err().to_string(), first);
        assert!(!owner.stop_path.exists());
        let evidence = owner.finish(Duration::ZERO).unwrap();
        assert!(!evidence.valid);
        assert!(evidence.failure.as_deref().unwrap().contains(&first));
        assert_eq!(calls.polls.get(), 1);
    }

    #[test]
    fn evidence_rejects_receipt_from_different_nonzero_pid() {
        let mut value = result();
        value.pid = 99;
        let evidence = build_evidence(EvidenceInput {
            load_id: "load-1",
            requested_seconds: 5,
            fallback_pid: 42,
            result: Some(value),
            exit_code: Some(0),
            timed_out: false,
            required_window: Duration::ZERO,
            active_window: Duration::from_secs(1),
        });
        assert!(!evidence.valid, "receipt PID must match the owned child");
        let failure = evidence.failure.unwrap();
        assert!(failure.contains("42") && failure.contains("99"));
    }

    #[test]
    fn oversized_receipt_metadata_is_rejected_before_reading_or_deleting() {
        let directory = TestDirectory::new();
        let path = directory.0.join("result.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(64 * 1024 + 1).unwrap();
        drop(file);
        assert!(
            read_worker_result(&path)
                .unwrap_err()
                .to_string()
                .contains("bounded")
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 64 * 1024 + 1);
    }

    #[test]
    fn oversized_receipt_does_not_read_body() {
        struct MustNotRead;
        impl io::Read for MustNotRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("oversized metadata must be rejected before body read/allocation");
            }
        }
        assert!(read_worker_result_from(MustNotRead, 64 * 1024 + 1).is_err());
    }

    #[test]
    fn growing_receipt_reads_at_most_one_over_limit_byte() {
        struct Growing(Rc<Cell<usize>>);
        impl io::Read for Growing {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let count = bytes.len().min((1024 * 1024) - self.0.get());
                bytes[..count].fill(b' ');
                self.0.set(self.0.get() + count);
                Ok(count)
            }
        }
        let read = Rc::new(Cell::new(0));
        assert!(read_worker_result_from(Growing(read.clone()), 1).is_err());
        assert_eq!(read.get(), 64 * 1024 + 1, "file growth bypassed read cap");
    }

    #[test]
    fn polling_and_kill_errors_do_not_skip_wait_or_overwrite_each_other() {
        let directory = TestDirectory::new();
        let (mut owner, calls) = supervisor(&directory);
        owner.child.poll_error = true;
        owner.child.kill_error = true;
        owner.child.code = 17;
        let evidence = owner
            .finish_with_grace(Duration::ZERO, Duration::ZERO)
            .expect("cleanup errors must return evidence after reaping");
        assert_eq!(calls.kills.get(), 1);
        assert_eq!(calls.waits.get(), 1);
        assert_eq!(evidence.worker_exit_code, Some(17));
        assert!(!evidence.valid);
        let failure = evidence.failure.unwrap();
        assert!(failure.contains("injected poll error"));
        assert!(failure.contains("injected kill error"));
    }

    #[test]
    fn forced_stop_records_timeout_and_waited_exit_without_drop_rekill() {
        let directory = TestDirectory::new();
        let (mut owner, calls) = supervisor(&directory);
        owner.child.ready = false;
        owner.child.code = 19;
        let evidence = owner
            .finish_with_grace(Duration::ZERO, Duration::ZERO)
            .unwrap();
        assert_eq!(calls.kills.get(), 1);
        assert_eq!(calls.waits.get(), 1);
        assert!(evidence.timed_out);
        assert_eq!(evidence.worker_exit_code, Some(19));
        assert!(!evidence.valid);
    }

    #[test]
    fn failed_wait_keeps_drop_reap_fallback_and_retains_raw_receipt() {
        let directory = TestDirectory::new();
        let (mut owner, calls) = supervisor(&directory);
        owner.child.ready = false;
        owner.child.wait_error = true;
        write_json_new(&owner.result_path, &result()).unwrap();
        let evidence = owner
            .finish_with_grace(Duration::ZERO, Duration::ZERO)
            .expect("failed wait must not discard partial evidence");
        assert_eq!(
            calls.kills.get(),
            2,
            "Drop must retry for the retained handle"
        );
        assert_eq!(calls.waits.get(), 2);
        assert_eq!(evidence.worker_exit_code, None);
        assert!(!evidence.valid);
        assert!(!evidence.cleanup_complete);
        assert!(
            evidence
                .failure
                .as_deref()
                .unwrap()
                .contains("injected wait error")
        );
        assert!(directory.0.join("result.json").is_file());
    }

    #[test]
    fn malformed_raw_receipt_is_retained_with_parse_error_and_actual_exit() {
        let directory = TestDirectory::new();
        let (owner, _) = supervisor(&directory);
        std::fs::write(&owner.result_path, b"not-json").unwrap();
        let evidence = owner.finish(Duration::ZERO).unwrap();
        assert_eq!(evidence.worker_exit_code, Some(0));
        assert!(!evidence.valid);
        assert!(evidence.failure.as_deref().unwrap().contains("decode"));
        assert_eq!(
            std::fs::read(directory.0.join("result.json")).unwrap(),
            b"not-json"
        );
    }

    #[test]
    fn drop_reaps_without_silently_deleting_partial_receipt() {
        let directory = TestDirectory::new();
        let (owner, calls) = supervisor(&directory);
        write_json_new(&owner.result_path, &result()).unwrap();
        drop(owner);
        assert_eq!(calls.kills.get(), 1);
        assert_eq!(calls.waits.get(), 1);
        assert!(directory.0.join("result.json").is_file());
    }

    #[test]
    fn check_latches_real_exit_for_finish_without_waiting_twice() {
        let directory = TestDirectory::new();
        let (mut owner, calls) = supervisor(&directory);
        owner.child.code = 29;
        assert!(owner.check().is_err());
        let evidence = owner.finish(Duration::ZERO).unwrap();
        assert_eq!(evidence.worker_exit_code, Some(29));
        assert_eq!(calls.polls.get(), 1);
        assert_eq!(calls.kills.get() + calls.waits.get(), 0);
    }

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
