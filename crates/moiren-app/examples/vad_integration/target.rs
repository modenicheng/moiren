//! Control-side coordinator; only ManagedDriver owns the driver control file.
use super::{
    native,
    options::Options,
    project,
    report::{Checkpoint, Evidence, IdleHoldEvidence, PcmEvidence, Report},
    validation,
};
use anyhow::{Context, bail, ensure};
use moiren_app::service::{AppHandle, AppRequest, ManagedDriver, ServiceRuntime, SessionPhase};
use moiren_core::graph::NodeKind;
use moiren_windows_audio::vad::StableEndpointId;
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
    let pcm = options.pcm;
    eprintln!("integration evidence directory: {}", directory.display());
    let shared = Arc::new(Mutex::new(Progress::default()));
    let worker_shared = shared.clone();
    let worker_directory = directory.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    // Discovery, startup, and join can cross native waits. The main thread is
    // an overall watchdog; no blocking Drop/join runs on its timeout path.
    let worker = thread::Builder::new()
        .name("vad-integration".into())
        .spawn(move || {
            let report = run(&worker_directory, started, timeout, pcm, &worker_shared);
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
                "integration failed; inspect report.json"
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

fn run(
    directory: &Path,
    started: Instant,
    timeout: Duration,
    pcm: bool,
    shared: &Shared,
) -> Report {
    let mut report = Report::default();
    if pcm {
        report.stage = "stage2_real_graph_native_pcm";
        report.limitations = vec![
            "Only exact 48kHz stereo Float32 exclusive event-driven clients are exercised; no timer/shared-mode/physical coverage.",
            "Signal headers prove Engine timeline advances; AppSnapshot.elapsed is wall time only and is not used.",
            "Native identity is root parent plus stable ID plus flow; this probe never opens a ControlClient or PcmOwner.",
            "Public native stamps correlate sampled stream advances with gate transitions; intermediate native states and external client counts are not observable.",
            "Running-phase re-admission is bounded to 5s and the existing phase deadline; gate epoch alone never permits closure.",
            "Gate transition timestamps are sampled monotonic wall times, not Engine progress. Intentional Stop/exit gate closures are separate checkpoints.",
            "Known PCM and Stop silence are bounded observations, not long stress/Verifier/release qualification.",
            "Exited plus join proves service teardown/control-file release, not final PDO absence.",
        ];
    }
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
        let stable_ids: [StableEndpointId; 2] = document
            .exports
            .iter()
            .map(|export| StableEndpointId::from_virtual_endpoint_id(export.endpoint_id))
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| anyhow::anyhow!("fixture needs two stable IDs"))?;
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
        if pcm {
            let session = LiveSession {
                handle: &handle,
                ids: &ids,
                apply_request: applied.0,
                revision: running.active_revision.expect("checked"),
                deadline: work_deadline,
                started,
            };
            report.gate_transitions = running
                .exports
                .iter()
                .map(|export| {
                    validation::GateProgress::new(
                        export.endpoint_id.clone(),
                        export.native_stamp,
                        started.elapsed().as_millis(),
                    )
                })
                .collect::<anyhow::Result<_>>()?;
            publish(shared, &report);
            session.hold_without_clients(&mut report, shared)?;
            session.exercise_pcm(stable_ids, &mut report, shared)?;
            return Ok(());
        }
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

const IDLE_HOLD: Duration = Duration::from_secs(4);
const SETTLE: Duration = Duration::from_secs(1);
const PCM_PHASE_LIMIT: Duration = Duration::from_secs(12);

struct LiveSession<'a> {
    handle: &'a AppHandle,
    ids: &'a [String; 2],
    apply_request: u64,
    revision: u64,
    deadline: Instant,
    started: Instant,
}

impl LiveSession<'_> {
    fn require_running(
        &self,
        stage: &str,
        report: &mut Report,
        shared: &Shared,
        phase_deadline: Instant,
    ) -> anyhow::Result<bool> {
        let now_ms = self.started.elapsed().as_millis();
        ensure!(
            Instant::now() < phase_deadline.min(self.deadline),
            "{stage}: phase/work deadline"
        );
        // This is a control-operation deadline, independent of PCM availability
        // and signal publications, including when the snapshot mutex is busy.
        for transition in &report.gate_transitions {
            transition.require_deadline(now_ms)?;
        }
        if let Some(snapshot) = self.handle.try_snapshot() {
            let evidence = Evidence::from(snapshot.as_ref());
            record(report, stage, evidence.clone(), shared);
            ensure!(
                evidence.owner_running(self.ids, self.apply_request)
                    && evidence.active_revision == Some(self.revision),
                "{stage}: Running/identity/confirmed revision changed: {evidence:?}"
            );
            for export in &evidence.exports {
                let transition = report
                    .gate_transitions
                    .iter_mut()
                    .find(|transition| transition.endpoint_id == export.endpoint_id)
                    .context("missing initial gate/stamp baseline")?;
                let observed = transition.observe(
                    export.native_stamp,
                    export.gate_open,
                    now_ms,
                    phase_deadline
                        .min(self.deadline)
                        .saturating_duration_since(self.started)
                        .as_millis(),
                );
                // Keep partial transition events even if this observation is
                // rejected and later service cleanup reaches the watchdog.
                publish(shared, report);
                observed?;
            }
            publish(shared, report);
            return Ok(evidence.gates_open());
        }
        // A missing current observation cannot satisfy end-of-phase readiness.
        Ok(false)
    }

    fn require_stopped(
        &self,
        request: u64,
        report: &mut Report,
        shared: &Shared,
    ) -> anyhow::Result<()> {
        if let Some(snapshot) = self.handle.try_snapshot() {
            let evidence = Evidence::from(snapshot.as_ref());
            record(
                report,
                "capture_running_after_stop",
                evidence.clone(),
                shared,
            );
            ensure!(
                evidence.stopped(self.ids, request),
                "Stop state/gates/endpoints changed: {evidence:?}"
            );
        }
        ensure!(Instant::now() < self.deadline, "stopped capture deadline");
        Ok(())
    }

    fn hold_without_clients(&self, report: &mut Report, shared: &Shared) -> anyhow::Result<()> {
        eprintln!("stage2: 4s no-client hold, actual Gain node signal headers");
        let started = Instant::now();
        let subscription_deadline = (started + Duration::from_secs(5)).min(self.deadline);
        report.idle_hold = Some(IdleHoldEvidence {
            confirmed_revision: self.revision,
            ..Default::default()
        });
        let topic = loop {
            self.require_running(
                "no_clients_waiting_for_gain_topic",
                report,
                shared,
                subscription_deadline,
            )?;
            if let Some(snapshot) = self.handle.try_snapshot()
                && let Some(node) = snapshot
                    .graph
                    .graph
                    .nodes()
                    .iter()
                    .find(|node| node.kind() == NodeKind::Gain)
                && let Some(topic) = snapshot.signal_topics.node_levels.get(&node.id())
            {
                break topic.clone();
            }
            ensure!(
                Instant::now() < subscription_deadline,
                "Gain signal topic not published"
            );
            thread::sleep(Duration::from_millis(5));
        };
        let mut ticket = self
            .handle
            .subscribe_topic(topic, Default::default())
            .map_err(|error| anyhow::anyhow!("Gain signal subscribe: {error:?}"))?;
        let mut subscriber = loop {
            self.require_running(
                "no_clients_subscribing_gain",
                report,
                shared,
                subscription_deadline,
            )?;
            if let Some(subscriber) = ticket
                .try_take()
                .map_err(|error| anyhow::anyhow!("Gain subscription: {error:?}"))?
            {
                break subscriber;
            }
            ensure!(
                Instant::now() < subscription_deadline,
                "Gain subscriber not delivered"
            );
            thread::sleep(Duration::from_millis(5));
        };
        let hold_started = Instant::now();
        loop {
            let gates_open =
                self.require_running("no_clients_four_second_hold", report, shared, self.deadline)?;
            let elapsed = hold_started.elapsed();
            let hold = report
                .idle_hold
                .as_mut()
                .expect("initialized hold evidence");
            hold.observed_wall_ms = elapsed.as_millis();
            if let Some(window) = subscriber.read_latest()
                && window.header().frame_end > window.header().frame_start
            {
                ensure!(
                    window.elements().len() == 2
                        && window
                            .elements()
                            .iter()
                            .all(|level| level.peak.is_finite() && level.rms.is_finite()),
                    "invalid stereo Gain signal window"
                );
                hold.timeline.observe(*window.header())?;
                if elapsed >= Duration::from_secs(3) {
                    hold.after_old_three_second_threshold
                        .observe(*window.header())?;
                }
            }
            publish(shared, report);
            if elapsed >= IDLE_HOLD && gates_open {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let hold = report.idle_hold.as_mut().expect("hold evidence");
        hold.timeline.require_progress()?;
        hold.after_old_three_second_threshold.require_progress()?;
        hold.timeline_verified = true;
        publish(shared, report);
        Ok(())
    }

    fn exercise_pcm(
        &self,
        stable_ids: [StableEndpointId; 2],
        report: &mut Report,
        shared: &Shared,
    ) -> anyhow::Result<()> {
        report.pcm = Some(PcmEvidence {
            source: validation::SIGNAL,
            expected_after_gain: validation::EXPECTED,
            second_source: validation::SECOND_SIGNAL,
            second_expected_after_gain: validation::SECOND_EXPECTED,
            settle_ms_per_phase: SETTLE.as_millis() as u64,
            ..Default::default()
        });
        publish(shared, report);
        eprintln!("stage2: read-only exact root/stable ID -> MMDevice mapping");
        let mut clients = native::Pair::prepare(
            stable_ids,
            (Instant::now() + Duration::from_secs(10)).min(self.deadline),
        )?;
        report.pcm.as_mut().expect("PCM evidence").identity = Some(clients.identity.clone());
        publish(shared, report);
        self.require_running("before_native_client_start", report, shared, self.deadline)?;
        let outcome = (|| -> anyhow::Result<()> {
            let started = clients.start(&mut report.native_clients_started_by_probe);
            // Publish partial success before any error cleanup can wait on native
            // calls; capture Start success must survive a later render failure.
            publish(shared, report);
            started?;
            self.signal_phase(&mut clients, false, report, shared)?;
            clients.set_signal(validation::SECOND_SIGNAL);
            self.signal_phase(&mut clients, true, report, shared)?;
            let evidence = report.pcm.as_mut().expect("PCM evidence");
            evidence.capture_running_at_stop_request = clients.capture_running();
            ensure!(
                evidence.capture_running_at_stop_request,
                "capture stopped before service Stop"
            );
            let stopped = self
                .handle
                .try_request_tracked(AppRequest::Stop)
                .map_err(|error| anyhow::anyhow!("Stop enqueue: {error:?}"))?;
            eprintln!("stage2: Stop while capture RUN, retaining native endpoints");
            loop {
                // Do not stop capture/render during service Stop; drain while the
                // graph retires so a later silent packet is not a stopped client.
                clients.pump(|samples, _| validate_finite(samples))?;
                if let Some(snapshot) = self.handle.try_snapshot() {
                    let evidence = Evidence::from(snapshot.as_ref());
                    record(
                        report,
                        "stop_with_capture_running",
                        evidence.clone(),
                        shared,
                    );
                    if evidence.stopped(self.ids, stopped.0) {
                        break;
                    }
                    ensure!(
                        evidence.phase != "failed",
                        "service Stop failed: {evidence:?}"
                    );
                }
                ensure!(
                    Instant::now() < self.deadline,
                    "service Stop timed out with capture RUN"
                );
                thread::sleep(Duration::from_millis(1));
            }
            report
                .pcm
                .as_mut()
                .expect("PCM evidence")
                .capture_running_after_stop_ack = clients.capture_running();
            let started = Instant::now();
            let deadline = (started + PCM_PHASE_LIMIT).min(self.deadline);
            loop {
                self.require_stopped(stopped.0, report, shared)?;
                ensure!(
                    clients.capture_running(),
                    "capture left RUN before silence verification"
                );
                let evidence = report.pcm.as_mut().expect("PCM evidence");
                let settling = started.elapsed() < SETTLE;
                evidence.silence_elapsed_ms = started.elapsed().as_millis() as u64;
                clients.pump(|samples, discontinuity| {
                    if settling {
                        validate_finite(samples)
                    } else {
                        observe_at(
                            &mut evidence.stopped_silence,
                            samples,
                            [0.0; 2],
                            discontinuity,
                            started.elapsed(),
                        )
                    }
                })?;
                evidence.capture_running_during_silence = true;
                if !settling && evidence.stopped_silence.require_silence().is_ok() {
                    break;
                }
                publish(shared, report);
                ensure!(
                    Instant::now() < deadline,
                    "insufficient post-Stop real silent frames: {:?}",
                    report.pcm
                );
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })();
        let evidence = report.pcm.as_mut().expect("PCM evidence");
        evidence.native_discontinuities = clients.discontinuities();
        let stopped = clients.stop();
        evidence.native_clients_stopped = stopped.is_ok();
        // Pair releases both WASAPI leases before service exit/control removal.
        drop(clients);
        publish(shared, report);
        outcome?;
        stopped?;
        report.pcm_verified = true;
        publish(shared, report);
        Ok(())
    }

    fn signal_phase(
        &self,
        clients: &mut native::Pair,
        second: bool,
        report: &mut Report,
        shared: &Shared,
    ) -> anyhow::Result<()> {
        let stage = if second {
            "pcm_second_signal_gain_half"
        } else {
            "pcm_signal_gain_half"
        };
        eprintln!("stage2: {stage}, require 48000 consecutive matching frames");
        let started = Instant::now();
        let deadline = (started + PCM_PHASE_LIMIT).min(self.deadline);
        loop {
            self.require_running(stage, report, shared, deadline)?;
            let settling = started.elapsed() < SETTLE;
            let pcm = report.pcm.as_mut().expect("PCM evidence");
            if second {
                pcm.second_signal_elapsed_ms = started.elapsed().as_millis() as u64;
            } else {
                pcm.signal_elapsed_ms = started.elapsed().as_millis() as u64;
            }
            let (observation, expected) = if second {
                (&mut pcm.second_signal, validation::SECOND_EXPECTED)
            } else {
                (&mut pcm.signal, validation::EXPECTED)
            };
            clients.pump(|samples, discontinuity| {
                if settling {
                    validate_finite(samples)
                } else {
                    observe_at(
                        observation,
                        samples,
                        expected,
                        discontinuity,
                        started.elapsed(),
                    )
                }
            })?;
            if !settling && observation.require_signal().is_ok() {
                // Gate readiness never substitutes for actual matching PCM.
                // Require a current, validated observation at successful exit.
                if self.require_running(stage, report, shared, deadline)? {
                    break;
                }
            }
            publish(shared, report);
            ensure!(
                Instant::now() < deadline,
                "{stage}: insufficient matching signal: {:?}",
                report.pcm
            );
            thread::sleep(Duration::from_millis(1));
        }
        publish(shared, report);
        Ok(())
    }
}

fn observe_at(
    observation: &mut validation::Observation,
    samples: &[f32],
    expected: [f32; 2],
    discontinuity: bool,
    elapsed: Duration,
) -> anyhow::Result<()> {
    if !samples.is_empty() {
        observation
            .first_frame_ms
            .get_or_insert(elapsed.as_millis());
        observation.last_frame_ms = Some(elapsed.as_millis());
    }
    observation.observe_packet(samples, expected, discontinuity)
}

fn validate_finite(samples: &[f32]) -> anyhow::Result<()> {
    ensure!(
        samples.iter().all(|sample| sample.is_finite()),
        "nonfinite PCM during bounded settling"
    );
    Ok(())
}

fn wait(
    handle: &AppHandle,
    deadline: Instant,
    stage: &str,
    report: &mut Report,
    shared: &Shared,
    ready: impl Fn(&Evidence) -> bool,
) -> anyhow::Result<Evidence> {
    eprintln!("integration: {stage}");
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
