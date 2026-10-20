//! Control-side coordinator only. No ControlClient or native audio client here.
use super::{
    options::Options,
    project,
    report::{Checkpoint, Evidence, Report},
};
use anyhow::{Context, bail, ensure};
use moiren_app::service::{AppHandle, AppRequest, ManagedDriver, ServiceRuntime, SessionPhase};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
struct Progress {
    report: Report,
    handle: Option<AppHandle>,
}

type Shared = Arc<Mutex<Progress>>;

pub(super) fn exercise(options: Options) -> anyhow::Result<()> {
    let started = Instant::now();
    let timeout = Duration::from_secs(options.timeout_seconds);
    let directory = output_directory(options.output_dir)?;
    eprintln!("stage1 evidence directory: {}", directory.display());
    let shared = Arc::new(Mutex::new(Progress::default()));
    let worker_shared = shared.clone();
    let worker_directory = directory.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    // Discovery, startup, and join can cross native waits. The main thread is
    // an overall watchdog; no blocking Drop/join runs on its timeout path.
    let worker = thread::Builder::new()
        .name("vad-integration".into())
        .spawn(move || {
            let report = run(&worker_directory, started, timeout, &worker_shared);
            let _ = tx.send(report);
        })?;
    let remaining = timeout.saturating_sub(started.elapsed());
    match rx.recv_timeout(remaining) {
        Ok(report) => {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("probe coordinator panicked"))?;
            emit_report(&directory, &report)?;
            ensure!(
                report.status == "passed",
                "stage1 failed; inspect report.json"
            );
            Ok(())
        }
        Err(error) => {
            let mut report = {
                let progress = shared.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(handle) = &progress.handle {
                    handle.request_exit();
                }
                progress.report.clone()
            };
            report.status = if matches!(error, mpsc::RecvTimeoutError::Timeout) {
                "timed_out"
            } else {
                "failed"
            };
            report.elapsed_ms = started.elapsed().as_millis();
            report.error = Some(format!(
                "coordinator {error}; graceful cleanup not confirmed"
            ));
            if let Err(error) = emit_report(&directory, &report) {
                eprintln!("could not persist timeout report: {error:#}");
            }
            // This standalone process owns every worker and driver handle it
            // opened. OS file cleanup is preferable to waiting indefinitely;
            // the report deliberately does not call that a graceful teardown.
            std::process::exit(if report.status == "timed_out" { 2 } else { 1 });
        }
    }
}

fn output_directory(requested: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let directory = requested.unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "moiren-vad-integration-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    });
    fs::create_dir(&directory)
        .context("creating fresh output directory (existing paths are refused)")?;
    directory
        .canonicalize()
        .context("resolving output directory")
}

fn run(directory: &Path, started: Instant, timeout: Duration, shared: &Shared) -> Report {
    let mut report = Report::default();
    let mut runtime = None;
    // Reserve a third of the total interval for stop/exit even after failure.
    let work_deadline = started + timeout.mul_f64(2.0 / 3.0);
    let overall_deadline = started + timeout;
    let outcome = (|| -> anyhow::Result<()> {
        let startup = project::persist(directory)?;
        report.persisted_project_verified = true;
        publish(shared, &report);
        let document = startup
            .project
            .as_ref()
            .context("startup did not restore project")?;
        let ids: [String; 2] = document
            .exports
            .iter()
            .map(|export| export.endpoint_id.to_string())
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| anyhow::anyhow!("fixture must have two exports"))?;
        // Only the production factory discovers/opens the single control owner.
        let service = ServiceRuntime::spawn_with_startup(ManagedDriver::windows(), startup)?;
        let handle = service.handle();
        shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .handle = Some(handle.clone());
        runtime = Some(service);
        let initial = handle.try_snapshot().context("missing startup snapshot")?;
        ensure!(
            initial.phase == SessionPhase::Idle && initial.graph.active_revision.is_none(),
            "restored project started without Apply"
        );
        ensure!(
            initial.exports.iter().all(|export| !export.gate_open),
            "startup gate was open"
        );
        report.checkpoints.push(Checkpoint {
            stage: "startup_restored_before_apply".into(),
            evidence: Evidence::from(initial.as_ref()),
        });
        publish(shared, &report);
        let applied = handle
            .try_request_tracked(AppRequest::ApplyGraph)
            .map_err(|error| anyhow::anyhow!("Apply enqueue: {error:?}"))?;
        let running = wait(
            &handle,
            work_deadline,
            "apply_no_probe_clients",
            &mut report,
            shared,
            |evidence| evidence.running(&ids, applied.0),
        )?;
        ensure!(
            running.active_revision.is_some(),
            "missing Engine confirmation"
        );
        let stopped = handle
            .try_request_tracked(AppRequest::Stop)
            .map_err(|error| anyhow::anyhow!("Stop enqueue: {error:?}"))?;
        wait(
            &handle,
            work_deadline,
            "stop_closed_gates_retained_endpoints",
            &mut report,
            shared,
            |evidence| evidence.stopped(&ids, stopped.0),
        )?;
        Ok(())
    })();
    if let Err(error) = outcome {
        report.error = Some(format!("{error:#}"));
    }
    let mut cleanup_ok = runtime.is_none();
    if let Some(service) = runtime {
        let handle = service.handle();
        handle.request_exit();
        // ServiceRuntime::join has no deadline API. Run it only after the
        // service publishes Exited; the outer watchdog also bounds this join.
        let exited = wait(
            &handle,
            overall_deadline,
            "exit_service_drained",
            &mut report,
            shared,
            |evidence| evidence.phase == "exited",
        );
        match exited {
            Ok(evidence) => match service.join() {
                Ok(()) => {
                    report.service_joined = true;
                    report.control_session_released = true;
                    cleanup_ok = evidence.exited();
                    if !cleanup_ok {
                        report.cleanup_error = Some("service exited, but public lifecycle evidence did not confirm removal requested/closed gates".into());
                    }
                }
                Err(error) => report.cleanup_error = Some(format!("service join: {error:?}")),
            },
            Err(error) => report.cleanup_error = Some(format!("exit deadline: {error:#}")),
        }
    }
    if report.error.is_none() && cleanup_ok && report.service_joined {
        report.status = "passed";
    }
    report.elapsed_ms = started.elapsed().as_millis();
    publish(shared, &report);
    report
}

fn wait(
    handle: &AppHandle,
    deadline: Instant,
    stage: &str,
    report: &mut Report,
    shared: &Shared,
    ready: impl Fn(&Evidence) -> bool,
) -> anyhow::Result<Evidence> {
    eprintln!("stage1: {stage}");
    let mut last = None;
    loop {
        if let Some(snapshot) = handle.try_snapshot() {
            let evidence = Evidence::from(snapshot.as_ref());
            let complete = ready(&evidence);
            let failure = (evidence.phase == "failed")
                .then(|| evidence.error.clone())
                .flatten();
            last = Some(evidence.clone());
            record(report, stage, evidence, shared);
            if complete {
                return Ok(last.expect("observed snapshot"));
            }
            if stage != "exit_service_drained"
                && let Some(error) = failure
            {
                bail!("{stage}: service failed: {error}");
            }
        }
        ensure!(
            Instant::now() < deadline,
            "{stage} timed out; last evidence: {last:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn record(report: &mut Report, stage: &str, evidence: Evidence, shared: &Shared) {
    if let Some(checkpoint) = report
        .checkpoints
        .last_mut()
        .filter(|checkpoint| checkpoint.stage == stage)
    {
        checkpoint.evidence = evidence;
    } else {
        report.checkpoints.push(Checkpoint {
            stage: stage.into(),
            evidence,
        });
    }
    publish(shared, report);
}

fn publish(shared: &Shared, report: &Report) {
    shared
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .report = report.clone();
}

fn emit_report(directory: &Path, report: &Report) -> anyhow::Result<()> {
    let json = serde_json::to_vec_pretty(report)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("report.json"))?;
    file.write_all(&json)?;
    file.write_all(b"\n")?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    output.write_all(&json)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
