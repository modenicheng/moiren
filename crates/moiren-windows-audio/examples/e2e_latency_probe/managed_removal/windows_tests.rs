//! Only harmless test-executable subprocesses. The native CLI is never invoked.
use super::*;
use std::{
    io::Write,
    process::{Child, Command, Stdio},
};

const FIXTURE: &str = "managed_removal::native::tests::process_fixture";
const ENV: &str = "MOIREN_MANAGED_REMOVAL_TEST_FIXTURE";

struct ChildOwner(Child);

impl Drop for ChildOwner {
    fn drop(&mut self) {
        // Fixture code has its own finite lifetime. Never terminate/reacquire a
        // process by PID, including an assertion/unwind path in the parent.
        let _ = self.0.wait();
    }
}

fn fixture_command(mode: &str, directory: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", FIXTURE, "--nocapture"])
        .env(ENV, mode)
        .env("MOIREN_MANAGED_REMOVAL_TEST_DIRECTORY", directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn wait_file(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "fixture receipt missing: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn process_fixture() {
    let Ok(mode) = std::env::var(ENV) else { return };
    let directory =
        PathBuf::from(std::env::var_os("MOIREN_MANAGED_REMOVAL_TEST_DIRECTORY").unwrap());
    if mode == "worker" {
        fs::write(directory.join("worker-ready"), b"1").unwrap();
        let start = Instant::now();
        while !directory.join("worker-stop").exists() && start.elapsed() < Duration::from_secs(12) {
            std::thread::sleep(Duration::from_millis(5));
        }
        return;
    }
    let arguments: Vec<String> =
        serde_json::from_str(&std::env::var("MOIREN_MANAGED_REMOVAL_TEST_ARGUMENTS").unwrap())
            .unwrap();
    let result = run(arguments, |_| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("cli-stub-only"))
            .unwrap();
        file.write_all(b"1").unwrap();
        Ok(1)
    });
    if mode == "success" {
        assert_eq!(result.unwrap(), 1);
    } else {
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains(match mode.as_str() {
                "timeout" => "timed out",
                "mismatch" => "identity differs",
                "owner-exit" => "owner exited",
                _ => panic!("unexpected fixture mode"),
            }),
            "{error}"
        );
        assert!(!directory.join("cli-stub-only").exists());
    }
}

#[test]
fn actual_process_handle_creation_session_and_wrong_creation() {
    let process = unsafe { GetCurrentProcess() };
    let ticks = creation_ticks(process).unwrap();
    assert!(ticks > FILETIME_TO_DATETIME_TICKS && ticks <= MAX_TICKS);
    let owner = open_owner(std::process::id(), ticks).unwrap();
    assert_eq!(
        creation_ticks(HANDLE(owner.as_raw_handle())).unwrap(),
        ticks
    );
    assert_eq!(
        unsafe { WaitForSingleObject(HANDLE(owner.as_raw_handle()), 0) },
        WAIT_TIMEOUT
    );
    assert!(open_owner(std::process::id(), ticks + 1).is_err());
    let session = session_id().unwrap();
    assert!(session <= 65535);
}

#[test]
fn actual_owner_handle_remains_exact_after_natural_exit() {
    let directory = crate::tests::Fixture::new();
    let directory = directory.path(".");
    let mut worker = ChildOwner(fixture_command("worker", &directory).spawn().unwrap());
    wait_file(&directory.join("worker-ready"));
    let ticks = creation_ticks(HANDLE(worker.0.as_raw_handle())).unwrap();
    let owner = open_owner(worker.0.id(), ticks).unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(HANDLE(owner.as_raw_handle()), 0) },
        WAIT_TIMEOUT
    );
    fs::write(directory.join("worker-stop"), b"1").unwrap();
    assert!(worker.0.wait().unwrap().success());
    assert_eq!(
        creation_ticks(HANDLE(owner.as_raw_handle())).unwrap(),
        ticks
    );
    assert_ne!(
        unsafe { WaitForSingleObject(HANDLE(owner.as_raw_handle()), 0) },
        WAIT_TIMEOUT
    );
}

#[test]
fn actual_managed_gate_timeout_mismatch_owner_exit_and_stub_success() {
    for mode in ["timeout", "mismatch", "owner-exit", "success"] {
        let fixture = crate::tests::Fixture::new();
        let directory = fixture.path(".");
        let mut worker = ChildOwner(fixture_command("worker", &directory).spawn().unwrap());
        wait_file(&directory.join("worker-ready"));
        let controller_ticks = creation_ticks(unsafe { GetCurrentProcess() }).unwrap();
        let worker_ticks = creation_ticks(HANDLE(worker.0.as_raw_handle())).unwrap();
        let args = vec![
            "managed-removal".into(),
            "--run-dir".into(),
            directory.to_str().unwrap().into(),
            "--nonce".into(),
            "0123456789abcdef0123456789abcdef".into(),
            "--controller-pid".into(),
            std::process::id().to_string(),
            "--controller-creation-ticks".into(),
            controller_ticks.to_string(),
            "--worker-pid".into(),
            worker.0.id().to_string(),
            "--worker-creation-ticks".into(),
            worker_ticks.to_string(),
            "--admission-timeout-ms".into(),
            "1500".into(),
            "--".into(),
            "live-vad".into(),
        ];
        let mut command = fixture_command(mode, &directory);
        command.env(
            "MOIREN_MANAGED_REMOVAL_TEST_ARGUMENTS",
            serde_json::to_string(&args).unwrap(),
        );
        let mut gate = ChildOwner(command.spawn().unwrap());
        wait_file(&directory.join("probe-ready.json"));
        let mut ready: Receipt =
            serde_json::from_slice(&fs::read(directory.join("probe-ready.json")).unwrap()).unwrap();
        assert_eq!(ready.probe_pid, gate.0.id());
        assert_eq!(
            ready.probe_creation_utc_ticks,
            creation_ticks(HANDLE(gate.0.as_raw_handle())).unwrap()
        );
        assert_eq!(ready.controller_creation_utc_ticks, controller_ticks);
        assert_eq!(ready.worker_creation_utc_ticks, worker_ticks);
        assert_eq!(ready.session_id, session_id().unwrap());
        if mode == "owner-exit" {
            fs::write(directory.join("worker-stop"), b"1").unwrap();
            assert!(worker.0.wait().unwrap().success());
        } else if mode != "timeout" {
            if mode == "mismatch" {
                ready.probe_creation_utc_ticks += 1;
            }
            let partial = directory.join("probe-admit.json.partial");
            fs::write(&partial, serde_json::to_vec(&ready).unwrap()).unwrap();
            fs::rename(partial, directory.join("probe-admit.json")).unwrap();
        }
        assert!(
            gate.0.wait().unwrap().success(),
            "gate fixture {mode} failed"
        );
        assert_eq!(directory.join("cli-stub-only").exists(), mode == "success");
        fs::write(directory.join("worker-stop"), b"1").unwrap();
        assert!(worker.0.wait().unwrap().success());
    }
}
