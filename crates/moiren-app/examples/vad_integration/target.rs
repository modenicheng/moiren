//! Control-side coordinator; only ManagedDriver owns the driver control file.
use super::{
    material, native,
    options::Options,
    progress, project,
    report::{Checkpoint, Evidence, IdleHoldEvidence, PcmEvidence, Report},
    soak, validation,
};
use anyhow::{Context, bail, ensure};
use moiren_app::service::{AppHandle, AppRequest, ManagedDriver, ServiceRuntime, SessionPhase};
use moiren_core::graph::NodeKind;
use moiren_windows_audio::vad::StableEndpointId;
use std::{
    cell::RefCell,
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
    updated: Option<Instant>,
}

type Shared = Arc<Mutex<Progress>>;

pub(super) fn exercise(options: Options) -> anyhow::Result<()> {
    let started = Instant::now();
    let timeout = Duration::from_secs(options.timeout_seconds);
    let directory = output_directory(options.output_dir.clone())?;
    let soaking = options.soaking();
    eprintln!("integration evidence directory: {}", directory.display());
    let shared = Arc::new(Mutex::new(Progress::default()));
    let worker_shared = shared.clone();
    let worker_directory = directory.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    // Discovery, startup, and join can cross native waits. The main thread is
    // an overall watchdog; no blocking Drop/join runs on its timeout path.
    let overall_deadline = started
        .checked_add(timeout)
        .context("outer deadline overflow")?;
    let mut progress_sequence = 0;
    persist_progress(
        &directory,
        &Report::default(),
        progress_sequence,
        started,
        false,
    )?;
    let worker = thread::Builder::new()
        .name("vad-integration".into())
        .spawn(move || {
            let report = run(&worker_directory, started, &options, &worker_shared);
            let _ = tx.send(report);
        })?;
    let result: anyhow::Result<Report> = loop {
        let remaining = overall_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err(anyhow::anyhow!("overall watchdog deadline exceeded"));
        }
        match rx.recv_timeout(remaining.min(Duration::from_secs(5))) {
            Ok(report) => break Ok(report),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!("coordinator disconnected"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let report = current_report(&shared, started);
                progress_sequence += 1;
                if let Err(error) =
                    persist_progress(&directory, &report, progress_sequence, started, false)
                {
                    break Err(error.context("durable progress publication failed"));
                }
                if soaking {
                    eprintln!(
                        "soak progress: {}ms phase={}",
                        started.elapsed().as_millis(),
                        report.current_phase
                    );
                }
            }
        }
    };
    match result {
        Ok(report) => {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("probe coordinator panicked"))?;
            emit_report(&directory, &report)?;
            persist_progress(&directory, &report, progress_sequence + 1, started, true)?;
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
            report.status = if Instant::now() >= overall_deadline {
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
            if let Err(error) =
                persist_progress(&directory, &report, progress_sequence + 1, started, true)
            {
                eprintln!("could not persist final failure progress: {error:#}");
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

fn run(directory: &Path, started: Instant, options: &Options, shared: &Shared) -> Report {
    let mut report = Report::default();
    let pcm = options.pcm;
    if pcm || options.soaking() {
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
        if options.soaking() {
            report.stage = "real_app_continuous_graph_soak";
            report.limitations.push("Soak is opt-in target evidence only, not Verifier, sleep/resume, physical endpoint, or release qualification.");
            report.limitations.push("Native packet positions infer capture gaps; private driver drop counters and render underrun counts are unavailable through App/native clients.");
            report.limitations.push("Idle checks actual signal frame_end every observation and two advances per one-second window; snapshot publications are freshness checks only.");
            report.limitations.push("PCM soak alternates source every 30s with a fixed 1s finite-only settling interval; all other capture samples must match gain exactly within 0.0001, with no silence/discontinuities.");
            if options.soak_pcm_file.is_some() {
                report.limitations.push("Music soak replaces the alternating constant source with looping WAV material; per-second alignment-tracked correlation windows (>= 0.95) replace exact constant matching, with >= 0.99 qualified windows, <= 3 consecutive failed windows, and unchanged freshness/discontinuity/coverage gates.");
            }
            if !pcm {
                report.limitations.push("Idle-only soak starts no probe native clients and makes no PCM or native audio clock claim.");
            }
        }
    }
    let mut runtime = None;
    let timeout = Duration::from_secs(options.timeout_seconds);
    // Long soak keeps a fixed cleanup reserve; a ratio would truncate 16h work.
    let work_deadline = started + options.work_budget();
    let overall_deadline = started + timeout;
    let outcome =
        (|| -> anyhow::Result<()> {
            let startup = project::persist(directory)?;
            report.persisted_project_verified = true;
            publish(shared, &report);
            // Load soak material before any driver/service work so a rejected
            // file fails without touching native audio.
            let material = match &options.soak_pcm_file {
                Some(path) => Some(material::load(path).context("music soak material")?),
                None => None,
            };
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
            let apply_deadline = if options.soaking() {
                (Instant::now() + Duration::from_secs(15)).min(work_deadline)
            } else {
                work_deadline
            };
            let running = wait(
                &handle,
                apply_deadline,
                "apply_no_probe_clients",
                &mut report,
                shared,
                |evidence| evidence.running(&ids, applied.0),
            )?;
            ensure!(
                running.active_revision.is_some(),
                "missing Engine confirmation"
            );
            if pcm || options.soaking() {
                let session = LiveSession {
                    handle: &handle,
                    ids: &ids,
                    apply_request: applied.0,
                    revision: running.active_revision.expect("checked"),
                    deadline: work_deadline,
                    started,
                    last_snapshot: RefCell::new(None),
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
                session.hold_without_clients(options.soak_idle_seconds, &mut report, shared)?;
                if pcm {
                    session.exercise_pcm(
                        stable_ids,
                        options.soak_pcm_seconds,
                        material.as_ref(),
                        &mut report,
                        shared,
                    )?;
                    return Ok(());
                }
            }
            let stopped = handle
                .try_request_tracked(AppRequest::Stop)
                .map_err(|error| anyhow::anyhow!("Stop enqueue: {error:?}"))?;
            wait(
                &handle,
                if options.soaking() {
                    (Instant::now() + PCM_PHASE_LIMIT).min(work_deadline)
                } else {
                    work_deadline
                },
                "stop_closed_gates_retained_endpoints",
                &mut report,
                shared,
                |evidence| evidence.stopped(&ids, stopped.0),
            )?;
            ensure!(
                options.soak_idle_seconds == 0
                    || report.soak_idle.as_ref().is_some_and(|soak| soak.completed
                        && soak.elapsed_ms >= options.soak_idle_seconds * 1_000),
                "requested idle soak incomplete"
            );
            Ok(())
        })();
    if let Err(error) = outcome {
        report.error = Some(format!("{error:#}"));
    }
    if report.error.is_none() {
        let pcm_completed = if options.soak_pcm_file.is_some() {
            report
                .soak_music
                .as_ref()
                .map(|soak| (soak.completed, soak.requested_ms, soak.elapsed_ms))
        } else {
            report
                .soak_pcm
                .as_ref()
                .map(|soak| (soak.completed, soak.requested_ms, soak.elapsed_ms))
        };
        for (requested, completed) in [
            (
                options.soak_idle_seconds,
                report
                    .soak_idle
                    .as_ref()
                    .map(|soak| (soak.completed, soak.requested_ms, soak.elapsed_ms)),
            ),
            (options.soak_pcm_seconds, pcm_completed),
        ] {
            if requested != 0
                && !completed.is_some_and(|(done, duration, elapsed)| {
                    done && duration == requested * 1_000 && elapsed >= duration
                })
            {
                report.error = Some("requested soak duration was not completed".into());
            }
        }
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
    last_snapshot: RefCell<Option<(Arc<moiren_app::service::AppSnapshot>, Instant)>>,
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
        if let Some((_, observed)) = self.last_snapshot.borrow().as_ref() {
            ensure!(
                observed.elapsed() <= Duration::from_secs(2),
                "{stage}: service snapshot publication stale"
            );
        }
        if let Some(snapshot) = self.handle.try_snapshot() {
            self.observe_snapshot(&snapshot, report)?;
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
    ) -> anyhow::Result<bool> {
        if let Some((_, observed)) = self.last_snapshot.borrow().as_ref() {
            ensure!(
                observed.elapsed() <= Duration::from_secs(2),
                "stopped service snapshot publication stale"
            );
        }
        let mut observed = false;
        if let Some(snapshot) = self.handle.try_snapshot() {
            self.observe_snapshot(&snapshot, report)?;
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
            observed = true;
        }
        ensure!(Instant::now() < self.deadline, "stopped capture deadline");
        Ok(observed)
    }

    fn observe_snapshot(
        &self,
        snapshot: &Arc<moiren_app::service::AppSnapshot>,
        report: &mut Report,
    ) -> anyhow::Result<()> {
        let now = Instant::now();
        let mut last = self.last_snapshot.borrow_mut();
        if last
            .as_ref()
            .is_none_or(|(previous, _)| !Arc::ptr_eq(previous, snapshot))
        {
            *last = Some((snapshot.clone(), now));
            report.last_snapshot_observed_ms = Some(self.started.elapsed().as_millis());
        }
        ensure!(
            last.as_ref().is_some_and(
                |(_, observed)| now.duration_since(*observed) <= Duration::from_secs(2)
            ),
            "service snapshot publication stale"
        );
        Ok(())
    }

    fn hold_without_clients(
        &self,
        soak_seconds: u64,
        report: &mut Report,
        shared: &Shared,
    ) -> anyhow::Result<()> {
        let duration = IDLE_HOLD.max(Duration::from_secs(soak_seconds));
        eprintln!(
            "no-client hold: {}s, actual Gain node signal headers",
            duration.as_secs()
        );
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
        let hold_deadline = (hold_started + duration + Duration::from_secs(5)).min(self.deadline);
        if soak_seconds > 0 {
            report.soak_idle = Some(soak::IdleSoak::new(soak_seconds * 1_000));
        }
        loop {
            let gates_open =
                self.require_running("no_clients_timeline_hold", report, shared, hold_deadline)?;
            let elapsed = hold_started.elapsed();
            let hold = report
                .idle_hold
                .as_mut()
                .expect("initialized hold evidence");
            hold.observed_wall_ms = elapsed.as_millis();
            let mut header = None;
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
                header = Some(*window.header());
                if elapsed >= Duration::from_secs(3) {
                    hold.after_old_three_second_threshold
                        .observe(*window.header())?;
                }
            }
            if let Some(soak) = &mut report.soak_idle {
                let observed = soak.poll(elapsed.as_millis() as u64, header);
                publish(shared, report);
                observed?;
            }
            publish(shared, report);
            if elapsed >= duration
                && gates_open
                && report
                    .soak_idle
                    .as_ref()
                    .is_none_or(soak::IdleSoak::end_observed)
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let hold = report.idle_hold.as_mut().expect("hold evidence");
        hold.timeline.require_progress()?;
        hold.after_old_three_second_threshold.require_progress()?;
        hold.timeline_verified = true;
        if let Some(soak) = &mut report.soak_idle {
            soak.finish()?;
        }
        publish(shared, report);
        Ok(())
    }

    fn exercise_pcm(
        &self,
        stable_ids: [StableEndpointId; 2],
        soak_seconds: u64,
        material: Option<&material::Material>,
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
            if soak_seconds > 0 {
                self.soak_pcm(&mut clients, soak_seconds, material, report, shared)?;
            }
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
            let stop_deadline = (Instant::now() + PCM_PHASE_LIMIT).min(self.deadline);
            loop {
                ensure!(
                    Instant::now() < stop_deadline,
                    "service Stop deadline with capture RUN"
                );
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
                if !settling
                    && evidence.stopped_silence.require_silence().is_ok()
                    && self.require_stopped(stopped.0, report, shared)?
                {
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
        if soak_seconds > 0 {
            report.native_diagnostics = Some(clients.diagnostics());
        }
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

    fn soak_pcm(
        &self,
        clients: &mut native::Pair,
        seconds: u64,
        material: Option<&material::Material>,
        report: &mut Report,
        shared: &Shared,
    ) -> anyhow::Result<()> {
        if let Some(material) = material {
            return self.soak_music(clients, seconds, material, report, shared);
        }
        eprintln!("PCM soak: {seconds}s, one continuously RUN native pair");
        // Continue the already-qualified second source with the existing pair;
        // every later pattern switch gets exactly the fixed 1s settling budget.
        clients.enable_soak_diagnostics()?;
        let started = Instant::now();
        let duration = Duration::from_secs(seconds);
        let deadline = (started + duration + Duration::from_secs(5)).min(self.deadline);
        report.soak_pcm = Some(soak::PcmSoak::new(
            seconds * 1_000,
            validation::SECOND_EXPECTED,
        ));
        let mut next_pattern_ms = soak::PATTERN_MS;
        let mut next_clock_ms = 0;
        let mut next_control_ms = 0;
        let mut next_publish_ms = 0;
        loop {
            ensure!(Instant::now() < deadline, "PCM soak phase/work deadline");
            let now_ms = started.elapsed().as_millis() as u64;
            if now_ms >= next_control_ms {
                self.require_running("soak_pcm_continuous_native_pair", report, shared, deadline)?;
                next_control_ms = now_ms + 20;
            }
            let progress = report.soak_pcm.as_mut().expect("soak PCM evidence");
            progress.require_fresh(now_ms)?;
            if now_ms >= next_pattern_ms && progress.requested_ms.saturating_sub(now_ms) >= 3_000 {
                let (source, expected) = if progress.source_changes.is_multiple_of(2) {
                    (validation::SIGNAL, validation::EXPECTED)
                } else {
                    (validation::SECOND_SIGNAL, validation::SECOND_EXPECTED)
                };
                progress.change_pattern(now_ms, expected)?;
                clients.set_signal(source);
                next_pattern_ms = now_ms + soak::PATTERN_MS;
            }
            let sampled = clients.pump(|samples, discontinuity| {
                progress.observe(started.elapsed().as_millis() as u64, samples, discontinuity)
            });
            // Preserve packet position gaps and the failed window before cleanup.
            report.native_diagnostics = Some(clients.diagnostics());
            if let Err(error) = sampled {
                publish(shared, report);
                return Err(error.context("continuous PCM soak"));
            }
            let now_ms = started.elapsed().as_millis() as u64;
            if now_ms >= next_clock_ms {
                let sampled = clients.sample_clocks(now_ms);
                report.native_diagnostics = Some(clients.diagnostics());
                publish(shared, report);
                sampled?;
                next_clock_ms = now_ms + 1_000;
            }
            if now_ms >= next_publish_ms {
                publish(shared, report);
                next_publish_ms = now_ms + 100;
            }
            if started.elapsed() >= duration
                && report.soak_pcm.as_ref().expect("soak PCM").end_observed()
                && report
                    .soak_pcm
                    .as_ref()
                    .expect("soak PCM")
                    .current_window
                    .require_signal()
                    .is_ok()
                && clients.require_clocks(now_ms).is_ok()
                && self.require_running(
                    "soak_pcm_continuous_native_pair",
                    report,
                    shared,
                    deadline,
                )?
            {
                report.soak_pcm.as_mut().expect("soak PCM").finish(now_ms)?;
                publish(shared, report);
                return Ok(());
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Continuous music soak: the native render client loops the material and
    /// every completed one-second capture window must correlate with the
    /// alignment-tracked expected waveform. Freshness, discontinuity, packet
    /// shape, clock-rate, and coverage gates are unchanged from the constant
    /// pattern soak.
    fn soak_music(
        &self,
        clients: &mut native::Pair,
        seconds: u64,
        material: &material::Material,
        report: &mut Report,
        shared: &Shared,
    ) -> anyhow::Result<()> {
        eprintln!(
            "music soak: {seconds}s looping {} frames of material",
            material.frames
        );
        clients.enable_soak_diagnostics()?;
        clients.play_material(std::sync::Arc::new(material.samples.clone()));
        let started = Instant::now();
        let duration = Duration::from_secs(seconds);
        let deadline = (started + duration + Duration::from_secs(5)).min(self.deadline);
        report.soak_music = Some(soak::MusicSoak::new(seconds * 1_000, material));
        // The alignment search runs on its own thread: the pumping loop must
        // service exclusive-mode events every few milliseconds.
        let material_mono = std::sync::Arc::new(soak::mono(&material.samples));
        let material_decimated =
            std::sync::Arc::new(soak::decimate(&material_mono, soak::MUSIC_GRID_STRIDE));
        let (job_tx, job_rx) = mpsc::sync_channel::<soak::MusicJob>(2);
        let (result_tx, result_rx) = mpsc::channel::<soak::MusicWindow>();
        let analysis_mono = material_mono.clone();
        let analysis_decimated = material_decimated.clone();
        let analyzer = thread::Builder::new()
            .name("vad-music-analyzer".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let window = soak::analyze_window(&analysis_mono, &analysis_decimated, job);
                    if result_tx.send(window).is_err() {
                        break;
                    }
                }
            })
            .context("start music analyzer thread")?;
        let outcome = self.run_music_loop(
            clients, started, duration, deadline, &job_tx, &result_rx, report, shared,
        );
        // The analyzer exits once every dispatched window is analyzed.
        drop(job_tx);
        let analyzer_result = analyzer.join();
        while let Ok(window) = result_rx.try_recv() {
            report
                .soak_music
                .as_mut()
                .expect("music soak evidence")
                .fold_window(window);
        }
        publish(shared, report);
        outcome?;
        analyzer_result.map_err(|_| anyhow::anyhow!("music analyzer panicked"))?;
        let progress = report.soak_music.as_mut().expect("music soak evidence");
        progress.finish(started.elapsed().as_millis() as u64)?;
        publish(shared, report);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn run_music_loop(
        &self,
        clients: &mut native::Pair,
        started: Instant,
        duration: Duration,
        deadline: Instant,
        job_tx: &mpsc::SyncSender<soak::MusicJob>,
        result_rx: &mpsc::Receiver<soak::MusicWindow>,
        report: &mut Report,
        shared: &Shared,
    ) -> anyhow::Result<()> {
        let mut next_clock_ms = 0;
        let mut next_control_ms = 0;
        let mut next_publish_ms = 0;
        loop {
            ensure!(Instant::now() < deadline, "music soak phase/work deadline");
            let now_ms = started.elapsed().as_millis() as u64;
            if now_ms >= next_control_ms {
                self.require_running(
                    "soak_music_continuous_native_pair",
                    report,
                    shared,
                    deadline,
                )?;
                next_control_ms = now_ms + 20;
            }
            let progress = report.soak_music.as_mut().expect("music soak evidence");
            progress.require_fresh(now_ms)?;
            let sampled = clients.pump(|samples, discontinuity| {
                progress.observe_packet(
                    started.elapsed().as_millis() as u64,
                    samples,
                    discontinuity,
                )
            });
            // Preserve packet position gaps and the failed window before cleanup.
            report.native_diagnostics = Some(clients.diagnostics());
            if let Err(error) = sampled {
                publish(shared, report);
                return Err(error.context("continuous music soak"));
            }
            let now_ms = started.elapsed().as_millis() as u64;
            while let Some(job) = report
                .soak_music
                .as_mut()
                .expect("music soak evidence")
                .take_window(now_ms)
            {
                job_tx
                    .send(job)
                    .context("music analyzer stopped accepting windows")?;
            }
            while let Ok(window) = result_rx.try_recv() {
                report
                    .soak_music
                    .as_mut()
                    .expect("music soak evidence")
                    .fold_window(window);
            }
            if now_ms >= next_clock_ms {
                let sampled = clients.sample_clocks(now_ms);
                report.native_diagnostics = Some(clients.diagnostics());
                publish(shared, report);
                sampled?;
                next_clock_ms = now_ms + 1_000;
            }
            if now_ms >= next_publish_ms {
                publish(shared, report);
                next_publish_ms = now_ms + 100;
            }
            if started.elapsed() >= duration
                && report
                    .soak_music
                    .as_ref()
                    .expect("music soak evidence")
                    .end_observed()
                && clients.require_clocks(now_ms).is_ok()
                && self.require_running(
                    "soak_music_continuous_native_pair",
                    report,
                    shared,
                    deadline,
                )?
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(1));
        }
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
        ensure!(
            Instant::now() < deadline,
            "{stage} timed out; last evidence: {last:?}"
        );
        if let Some(snapshot) = handle.try_snapshot() {
            let evidence = Evidence::from(snapshot.as_ref());
            let complete = ready(&evidence);
            let failure = (evidence.phase == "failed")
                .then(|| evidence.error.clone())
                .flatten();
            last = Some(evidence.clone());
            record(report, stage, evidence, shared);
            ensure!(
                Instant::now() < deadline,
                "{stage}: snapshot arrived after deadline"
            );
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
    report.checkpoint(stage, evidence);
    publish(shared, report);
}

fn publish(shared: &Shared, report: &Report) {
    let mut progress = shared.lock().unwrap_or_else(|poison| poison.into_inner());
    progress.report = report.clone();
    progress.updated = Some(Instant::now());
}

fn current_report(shared: &Shared, started: Instant) -> Report {
    let progress = shared.lock().unwrap_or_else(|poison| poison.into_inner());
    let mut report = progress.report.clone();
    report.status = "in_progress";
    report.elapsed_ms = progress
        .updated
        .unwrap_or(started)
        .saturating_duration_since(started)
        .as_millis();
    report
}

fn persist_progress(
    directory: &Path,
    report: &Report,
    sequence: u64,
    started: Instant,
    finished: bool,
) -> anyhow::Result<()> {
    progress::persist(
        directory,
        report,
        sequence,
        started.elapsed().as_millis(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        finished,
    )
}

fn emit_report(directory: &Path, report: &Report) -> anyhow::Result<()> {
    let json = serde_json::to_vec_pretty(report)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("report.json"))?;
    file.write_all(&json)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    output.write_all(&json)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
