//! Native owner orchestration for live Process and VAD probe routes.
use super::*;
use anyhow::{Context, bail, ensure};
use moiren_engine::{
    host::{EngineHost, EngineProject, ExecutionState, HostConfig},
    node::{EndpointSelector, OutputTarget, VirtualEndpointId},
};
use moiren_windows_audio::{
    ActivationGate, SessionDuration, StopSignal,
    engine_backend::ProfessionalBackend,
    latency::RenderQueuePolicy,
    process_loopback::{
        ProcessLoopbackOptions, inspect_process,
        prepare_process_capture_with_gate_returning_with_abort,
    },
    vad::{Channels, Intent, PdoState, WindowsVadProvider},
};
use std::{
    str::FromStr,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use super::render::{
    PreparedNativeRender, RenderOptions, prepare_render_native_with_gate_returning_with_abort,
};

const STARTUP_SAMPLE_LIMIT: usize = 256;
const STARTUP_SAMPLE_INTERVAL: Duration = Duration::from_millis(10);

fn validate_initial_recorder_transition(
    before: [u64; 5],
    before_shape: &(String, String, u32),
    current: [u64; 5],
    current_shape: &(String, String, u32),
) -> Result<()> {
    ensure!(
        current == before && current_shape == before_shape,
        "initial recorder Start changed the prepared endpoint/stream/gate identity or shape"
    );
    Ok(())
}

fn recorder_start_running(
    view: &moiren_windows_audio::vad::VadExportObservation,
    target: VirtualEndpointId,
    before: &vad_collection::EndpointHealth,
) -> Result<bool> {
    let health = endpoint_health(Some(view), target, Intent::Microphone);
    ensure!(
        health.provider_ready && health.provider_error.is_none() && health.quality_known,
        "recorder Start lost microphone identity, gate, readiness or telemetry"
    );
    let snapshot = view.snapshot.context("microphone snapshot missing")?;
    validate_initial_recorder_transition(
        before.stamp.context("microphone baseline stamp missing")?,
        before
            .shape
            .as_ref()
            .context("microphone baseline shape missing")?,
        snapshot_stamp(snapshot),
        &snapshot_shape(snapshot),
    )?;
    match snapshot.stream_state() {
        moiren_windows_audio::vad::StreamState::Paused => Ok(false),
        moiren_windows_audio::vad::StreamState::Running => Ok(true),
        moiren_windows_audio::vad::StreamState::Closed => bail!("prepared recorder stream closed"),
    }
}

fn recorder_confirmation_running(
    before: &vad_collection::EndpointHealth,
    current: &vad_collection::EndpointHealth,
) -> Result<bool> {
    ensure!(
        current.stamp == before.stamp
            && current.shape == before.shape
            && current.provider_ready
            && current.provider_error.is_none()
            && current.quality_known,
        "microphone changed during recorder confirmation"
    );
    Ok(current.stream_running)
}

fn wait_initial_recorder<T>(
    deadline: Instant,
    mut now: impl FnMut() -> Instant,
    mut observe: impl FnMut() -> Result<Option<T>>,
    mut pause: impl FnMut(Duration),
) -> Result<T> {
    loop {
        ensure!(
            now() < deadline,
            "recorder Start admission deadline exceeded"
        );
        let observed = observe()?;
        let current = now();
        ensure!(
            current < deadline,
            "recorder Start admission deadline exceeded"
        );
        if let Some(observed) = observed {
            return Ok(observed);
        }
        pause(STARTUP_SAMPLE_INTERVAL.min(deadline.saturating_duration_since(current)));
    }
}

fn each_owned_export(
    ids: &[VirtualEndpointId],
    mut operation: impl FnMut(VirtualEndpointId) -> Result<()>,
) -> Result<()> {
    let mut result = Ok(());
    for id in ids {
        let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(*id)))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("VAD endpoint {id:?} cleanup panicked")));
        result = soak_cleanup::combine(result, attempted);
    }
    result
}

fn prepare_owned_exports(
    exports: &[(VirtualEndpointId, Intent, String)],
    owned: &mut Vec<VirtualEndpointId>,
    mut observe: impl FnMut(
        VirtualEndpointId,
    ) -> Option<moiren_windows_audio::vad::VadExportObservation>,
    mut create: impl FnMut(VirtualEndpointId, Intent, &str) -> Result<()>,
    mut ready: impl FnMut(VirtualEndpointId, Intent) -> Result<()>,
) -> Result<()> {
    for (id, intent, name) in exports {
        if !owned.contains(id) {
            ensure!(
                observe(*id).is_none_or(|view| view
                    .snapshot
                    .is_some_and(|snapshot| snapshot.state() == PdoState::Removed)),
                "refusing to adopt an existing VAD export for this run"
            );
            // A panic after the provider published its uncertain record must
            // still transfer that endpoint into this run's cleanup ownership.
            let created = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                create(*id, *intent, name)
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("VAD CREATE {id:?} panicked")));
            // CREATE can fail after publishing an uncertain ownership record.
            // Explicit no-resource deferral removes that record; do not own it.
            if observe(*id).is_some() {
                owned.push(*id);
            }
            created.with_context(|| format!("CREATE VAD {intent:?} export"))?;
        }
        ready(*id, *intent)?;
    }
    Ok(())
}

type GateSnapshot = (
    VirtualEndpointId,
    moiren_windows_audio::vad::EndpointSnapshot,
);

fn open_prepared_gates(
    snapshots: &[GateSnapshot],
    revision: u64,
    ticket: u64,
    mut open: impl FnMut(
        VirtualEndpointId,
        [u64; 5],
        u64,
        u64,
    ) -> Result<moiren_windows_audio::vad::EndpointSnapshot>,
) -> Result<Vec<GateSnapshot>> {
    let mut admitted = Vec::with_capacity(snapshots.len());
    for (id, snapshot) in snapshots {
        let current = open(*id, snapshot_stamp(*snapshot), revision, ticket)
            .with_context(|| format!("open VAD export gate {id:?}"))?;
        // ABI v2 SET_GATE(open) preserves the epoch; close/reset advances it.
        // Retain the actual post-open observation, not a synthetic epoch delta.
        ensure!(
            current.gate_open()
                && current.stable_id() == stable_from_virtual(*id)
                && current.intent() == snapshot.intent()
                && current.state() == PdoState::Present
                && current.policy_prepared()
                && current.audio_ready(),
            "opened VAD export has no current prepared identity"
        );
        validate_initial_recorder_transition(
            snapshot_stamp(*snapshot),
            &snapshot_shape(*snapshot),
            snapshot_stamp(current),
            &snapshot_shape(current),
        )?;
        admitted.push((*id, current));
    }
    Ok(admitted)
}

fn retain_native_value(destination: &mut Option<Value>, value: impl Serialize) -> Result<()> {
    *destination = Some(serde_json::to_value(value)?);
    Ok(())
}

fn retain_capture_join(
    native: &mut NativeReport,
    outcome: (
        Option<moiren_windows_audio::capture::CaptureReport>,
        Option<moiren_windows_audio::capture::CaptureError>,
    ),
    persist: impl FnOnce(&moiren_windows_audio::capture::CaptureReport) -> Result<()>,
) -> Result<()> {
    let (report, error) = outcome;
    let mut result = if let Some(error) = error {
        native.capture_failure = Some(serde_json::json!({
            "stage": "capture_join", "error": error.to_string(),
            "partial_report_retained": report.is_some(),
        }));
        Err(anyhow::Error::new(error)).context("join Process Loopback capture")
    } else {
        Ok(())
    };
    if let Some(report) = report {
        result = soak_cleanup::combine(result, persist(&report));
        result = soak_cleanup::combine(result, retain_native_value(&mut native.capture, report));
    } else {
        result = soak_cleanup::combine(result, Err(anyhow::anyhow!("capture report missing")));
    }
    result
}

fn prepare_process_input<T>(
    bridge: ProcessBridgeOptions,
    render: &mut Option<PreparedNativeRender>,
    report: &mut NativeReport,
    prepare: impl FnOnce(
        ProcessBridgeOptions,
        &mut Option<PreparedNativeRender>,
        &mut NativeReport,
    ) -> Result<T>,
) -> Result<(ProcessBridgeOptions, T)> {
    let info = render.as_ref().context("prepared render missing")?.info();
    let mut evidence = None;
    let capture = process_admission::prepare_capture_for_render_recording(
        bridge,
        info.timing,
        info.queue,
        &mut evidence,
        |bridge| prepare(bridge, render, report),
    );
    report.process_admission = evidence;
    match capture {
        Ok(capture) => Ok(capture),
        Err(error) => {
            let mut result = Err(error);
            if let Some(render) = render.take() {
                let (retained, cancelled) = render.cancel_retaining_report();
                result = soak_cleanup::combine(result, cancelled);
                if let Some(retained) = retained {
                    result = soak_cleanup::combine(
                        result,
                        retain_native_value(&mut report.render, retained),
                    );
                }
            }
            result.map(|()| unreachable!("capture preparation failed"))
        }
    }
}

pub fn run(
    options: &LiveOptions,
    bridge: &ProcessBridgeOptions,
    queue: RenderQueuePolicy,
    paths: &ArtifactPaths,
    native_report: &mut NativeReport,
    gaps: &mut Vec<String>,
) -> Result<()> {
    let identity =
        inspect_process(options.source_pid).context("inspect selected source process")?;
    let duration = if options.diagnostic_soak || options.controlled_load_id.is_some() {
        SessionDuration::UntilStopped
    } else {
        SessionDuration::For(Duration::from_secs(options.seconds))
    };
    let mut journal = diagnostic_journal(options, paths)?;
    let stop = StopSignal::new().context("create shared stop signal")?;
    let gate = ActivationGate::new(stop.clone()).context("create activation gate")?;
    let render = match prepare_render_native_with_gate_returning_with_abort(
        RenderOptions::from_live(options, queue, duration),
        stop.clone(),
        gate.clone(),
        || (),
    ) {
        Ok(render) => render,
        Err(error) => {
            native_report.render_failure = Some(error.evidence("render_prepare"));
            let mut result = Err(error.error).context("prepare WASAPI render");
            if let Some(report) = error.report {
                result = soak_cleanup::combine(
                    result,
                    retain_native_value(&mut native_report.render, report),
                );
            }
            return soak_cleanup::combine(result, finish_journal(&mut journal, native_report));
        }
    };
    let render_info = render.info();
    let padding = render.padding_observer();
    let mut render = Some(render);
    let capture = prepare_process_input(
        *bridge,
        &mut render,
        native_report,
        |bridge, render, native_report| {
            let capture = prepare_process_capture_with_gate_returning_with_abort(
                ProcessLoopbackOptions {
                    artifact: if options.diagnostic_soak {
                        None
                    } else {
                        Some(moiren_windows_audio::capture::PcmArtifactConfig::new(
                            usize::try_from(options.seconds)?
                                .checked_mul(48_000)
                                .context("capture artifact size overflow")?,
                        ))
                    },
                    bridge,
                    latency_telemetry: options.telemetry,
                    target: identity.clone(),
                    duration,
                },
                stop.clone(),
                gate.clone(),
                // Disconnect and stop the parked peer before capture join.
                || render.take().map(|render| render.cancel_retaining_report()),
            );
            match capture {
                Ok(capture) => Ok(capture),
                Err(mut failure) => {
                    native_report.capture_failure = Some(serde_json::json!({
                        "stage":"capture_prepare", "error":failure.error.to_string(),
                        "stop_error":failure.stop_error.as_ref().map(ToString::to_string),
                        "join_error":failure.join_error.as_ref().map(ToString::to_string),
                        "abort_panicked":failure.abort_panicked,
                    }));
                    let mut result = Err(anyhow::anyhow!(failure.error))
                        .context("prepare Process Loopback capture");
                    if let Some(report) = failure.report.take() {
                        result = soak_cleanup::combine(
                            result,
                            retain_native_value(&mut native_report.capture, report),
                        );
                    }
                    if let Some(Some((report, cancelled))) = failure.abort.take() {
                        result = soak_cleanup::combine(result, cancelled);
                        if let Some(report) = report {
                            result = soak_cleanup::combine(
                                result,
                                retain_native_value(&mut native_report.render, report),
                            );
                        }
                    }
                    if let Some(error) = failure.stop_error {
                        result = soak_cleanup::combine(result, Err(error.into()));
                    }
                    if let Some(error) = failure.join_error {
                        result = soak_cleanup::combine(result, Err(error.into()));
                    }
                    if failure.abort_panicked {
                        result = soak_cleanup::combine(
                            result,
                            Err(anyhow::anyhow!("capture preparation abort panicked")),
                        );
                    }
                    result.map(|()| unreachable!("capture preparation failed"))
                }
            }
        },
    );
    let (admitted_bridge, prepared) = match capture {
        Ok(prepared) => prepared,
        Err(error) => {
            return soak_cleanup::combine(Err(error), finish_journal(&mut journal, native_report));
        }
    };
    // Keep full-wake admission metadata even though actual source reads
    // are segmented. Archive from the empty gated bridge, not a late tail.
    let bridge_observer = prepared.observer.clone();
    let mut trace = moiren_windows_audio::process_loopback::ProcessTraceCollector::new(
        identity.clone(),
        1,
        48_000,
        48_000,
        admitted_bridge.max_demand_frames,
    );
    trace.poll(&bridge_observer);
    let capture_session = prepared.session;
    let mut render_session = None;
    let mut controlled_load = None;
    // Keep every native owner outside the fallible continuation. A graph,
    // attachment, activation, or load-start error still reaches explicit
    // cancellation/joins and retains reports instead of relying on Drop.
    let collection = (|| -> Result<()> {
        ensure!(
            prepared.sample_rate == 48_000 && prepared.channels == 2,
            "Process graph negotiation is not 48k stereo"
        );
        let renderer = process_collection::graph::build_renderer(options, prepared.source)?
            .with_bridge_observer(bridge_observer.clone());
        let timeline = renderer.timeline_observer();
        match render
            .take()
            .expect("prepared render retained until attachment")
            .attach_returning_with_abort(renderer, queue, || {
                stop.request_stop().err().map(|error| error.to_string())
            }) {
            Ok(render) => render_session = Some(render.into_session()),
            Err(mut error) => {
                native_report.render_failure = Some(error.evidence("render_attach"));
                let mut result = Err(error.error).context("attach Engine renderer");
                if let Some(report) = error.report.take() {
                    result = soak_cleanup::combine(
                        result,
                        retain_native_value(&mut native_report.render, report),
                    );
                }
                if let Some(error) = error.stop_error {
                    result = soak_cleanup::combine(result, Err(error));
                }
                if let Some(error) = error.join_error {
                    result = soak_cleanup::combine(result, Err(error));
                }
                if let Some(Some(error_stop)) = error.abort {
                    result = soak_cleanup::combine(result, Err(anyhow::anyhow!(error_stop)));
                }
                if error.abort_panicked {
                    result = soak_cleanup::combine(
                        result,
                        Err(anyhow::anyhow!("render attachment abort panicked")),
                    );
                }
                return result;
            }
        }
        let render_session = render_session.as_ref().expect("attached render session");
        trace.poll(&bridge_observer);
        gate.activate().context("activate capture/render gate")?;
        if let Some(load_id) = options.controlled_load_id.as_deref() {
            let startup_deadline = Instant::now() + Duration::from_secs(5);
            while !(capture_session.has_started() && render_session.has_started()) {
                trace.poll(&bridge_observer);
                ensure!(
                    !capture_session.is_finished() && !render_session.is_finished(),
                    "native owner stopped before controlled load startup"
                );
                ensure!(
                    Instant::now() < startup_deadline,
                    "native owners did not start before controlled load startup deadline"
                );
                thread::sleep(Duration::from_millis(10));
            }
            let directory = Path::new(&paths.directory);
            controlled_load = Some(controlled_load::Supervisor::start(
                load_id,
                options.seconds,
                directory.join("controlled-load.stop"),
                directory.join("controlled-load.result.json"),
            )?);
        }
        trace.poll(&bridge_observer);
        if journal.is_some() || controlled_load.is_some() {
            let duration = Duration::from_secs(options.seconds);
            let interval = Duration::from_secs(options.observation_interval_seconds);
            let mut next_observation = Duration::ZERO;
            soak_control::run_until(
                duration,
                |elapsed| {
                    trace.poll(&bridge_observer);
                    if let Some(journal) = journal.as_mut() {
                        journal.check()?;
                        if elapsed >= next_observation {
                            journal.record(&serde_json::json!({
                            "schema_version": 1, "source_kind": options.source_kind,
                            "elapsed_seconds": elapsed.as_secs_f64(),
                            "capture_started": capture_session.has_started(),
                            "render_started": render_session.has_started(),
                            "capture_finished": capture_session.is_finished(),
                            "render_finished": render_session.is_finished(),
                            "bridge": bridge_observer.snapshot(),
                            "render_padding": padding.as_ref().map(soak::process_padding_observation),
                            "timeline": timeline.try_snapshot().map(|value| serde_json::json!({
                                "frame": value.frame, "epoch": value.epoch, "revision": value.revision,
                            })),
                        }))?;
                            next_observation = elapsed.saturating_add(interval);
                        }
                    }
                    if let Some(load) = controlled_load.as_mut() {
                        load.check()?;
                    }
                    ensure!(
                        !capture_session.is_finished() && !render_session.is_finished(),
                        "native owner stopped before supervised deadline"
                    );
                    Ok(())
                },
                || stop.request_stop().context("stop supervised native owners"),
            )
        } else {
            process_trace::poll_finite(
                Duration::from_secs(options.seconds),
                || trace.poll(&bridge_observer),
                || capture_session.is_finished() && render_session.is_finished(),
                || {
                    stop.request_stop()
                        .context("stop finite Process owners at deadline")
                },
            )
        }
    })();
    trace.poll(&bridge_observer);
    let mut collection = soak_cleanup::combine(
        collection,
        stop.request_stop()
            .context("stop Process owners before cleanup"),
    );
    if let Some(load) = controlled_load.take() {
        // This blocking wait, like the joins below, can outlast the realtime
        // history window. Polling is best-effort: the next poll latches any
        // gap as diagnostic refusal; final closure cannot prove completeness.
        match load.finish(Duration::from_secs(options.seconds)) {
            Ok(evidence) => {
                if !evidence.valid {
                    let failure = evidence
                        .failure
                        .clone()
                        .unwrap_or_else(|| "controlled load evidence is invalid".into());
                    collection = soak_cleanup::combine(collection, Err(anyhow::anyhow!(failure)));
                }
                collection = soak_cleanup::combine(
                    collection,
                    retain_native_value(&mut native_report.controlled_load, evidence),
                );
            }
            Err(error) => {
                native_report.controlled_load = Some(serde_json::json!({
                    "load_id": options.controlled_load_id,
                    "requested_seconds": options.seconds,
                    "valid": false,
                    "cleanup_complete": false,
                    "failure": format!("finish controlled load: {error:#}"),
                }));
                collection = soak_cleanup::combine(
                    collection,
                    Err(anyhow::anyhow!("finish controlled load: {error:#}")),
                );
            }
        }
    }
    if let Some(render) = render.take() {
        let (report, cancelled) = render.cancel_retaining_report();
        collection = soak_cleanup::combine(collection, cancelled);
        if let Some(report) = report {
            collection = soak_cleanup::combine(
                collection,
                retain_native_value(&mut native_report.render, report),
            );
        }
    }
    trace.poll(&bridge_observer);
    let capture_result = capture_session.join_retaining_report();
    trace.poll(&bridge_observer);
    let render_result = render_session.map(|render| render.join());
    trace.poll(&bridge_observer);
    collection = soak_cleanup::combine(
        collection,
        retain_capture_join(native_report, capture_result, |report| {
            persist_capture_artifact(paths, report)
        }),
    );
    match render_result {
        Some(Ok(report)) => {
            collection = soak_cleanup::combine(
                collection,
                retain_native_value(&mut native_report.render, report),
            );
        }
        Some(Err(error)) => gaps.push(format!("join WASAPI render: {error}")),
        None => {}
    }
    // Keep the observer alive across both joins so this is the final
    // runtime state, rather than the all-zero pre-activation snapshot.
    collection = soak_cleanup::combine(
        collection,
        retain_native_value(&mut native_report.bridge, bridge_observer.snapshot()),
    );
    // The source and renderer have been destroyed by both joins. Preserve
    // actual writer-closure/truncation facts, never infer successful cleanup
    // or complete history from the registry/owner being stopped.
    let trace_export = process_trace::export_collector_into(native_report, paths, &trace);
    // Keep the metadata read above in the report path even if the source
    // process exits during the finite run.
    if render_info.timing.capacity_frames == 0 {
        gaps.push("render reported zero native capacity".into());
    }
    let owners = if native_report.capture.is_none() || native_report.render.is_none() {
        Err(anyhow::anyhow!(
            "one or more native owners did not return a report"
        ))
    } else {
        Ok(())
    };
    // Preserve final owner/bridge reports before any wait on the sink.
    // A stalled OS write may delay exit, never native cancellation.
    let result = soak_cleanup::combine(collection, owners);
    let result = soak_cleanup::combine(result, trace_export);
    soak_cleanup::combine(result, finish_journal(&mut journal, native_report))
}

fn finish_journal(journal: &mut Option<soak::Journal>, report: &mut NativeReport) -> Result<()> {
    let Some(journal) = journal else {
        return Ok(());
    };
    let result = journal.finish();
    report.diagnostic_observations = journal.samples();
    result
}

/// Run the production VAD playback -> Engine -> physical render path.
///
/// The native VAD client is deliberately kept separate from EngineHost:
/// it is the external application source, while the provider owns the
/// driver's PCM worker and the backend owns the graph/render owner.
pub fn run_vad(
    options: &LiveOptions,
    paths: &ArtifactPaths,
    source_plan: Option<source::PreparedSource>,
    collection_config: Option<&vad_collection::Config>,
    native_report: &mut NativeReport,
    _gaps: &mut Vec<String>,
) -> Result<()> {
    let mut journal = diagnostic_journal(options, paths)?;
    let stable_text = options
        .stable_endpoint_id
        .as_deref()
        .context("--stable-endpoint-id is required for live-vad")?;
    let endpoint_id = VirtualEndpointId::from_str(stable_text)
        .map_err(|_| anyhow::anyhow!("--stable-endpoint-id must be exactly 32 hex digits"))?;
    let name = options
        .source_name
        .as_deref()
        .context("--name is required for live-vad")?;
    let (interface, root) = vad::discover_root().context("discover VAD control root")?;
    let provider = WindowsVadProvider::from_interface(&interface)
        .context("open VAD provider on the selected control root")?;
    // From CREATE onward every failure takes the same best-effort cleanup
    // path. The journal was declared first, so even unwinding stops native
    // owners before joining a potentially stalled filesystem worker.
    let mut cleanup = VadCleanup {
        provider: &provider,
        endpoint_id,
        microphone: options.microphone_target()?,
        owned_exports: Vec::with_capacity(2),
        exports_prepared: false,
        recorder_confirmed: false,
        initial_recorder: None,
        playback: None,
        host: None,
        observer: None,
        observations: None,
        professional_observations: None,
        retained_native: None,
        retained_professional: None,
        playback_info: None,
        startup_samples: Vec::with_capacity(STARTUP_SAMPLE_LIMIT),
        observation_clock: control_observation::ControlObservationClock::prepare(),
        report: native_report,
        playback_outcome: None,
        admission: None,
        last_health: None,
    };
    if let Some(config) = collection_config {
        return vad_collection::collect(
            options,
            paths,
            config,
            source_plan.context("collection source missing")?,
            root,
            name,
            cleanup,
        );
    }
    let collection = (|| -> Result<()> {
        cleanup.prepare(options, name, root, source_plan, None)?;
        let observer = cleanup.observer.as_ref().expect("prepared host observer");
        let observations = cleanup
            .observations
            .as_ref()
            .expect("prepared native observations");
        let revision = cleanup
            .admission
            .as_ref()
            .expect("admission")
            .current
            .revision
            .context("confirmed revision missing")?;
        let playback = cleanup.playback.as_mut().expect("prepared VAD playback");
        playback.activate()?;
        cleanup.playback_info = Some(serde_json::to_value(&playback.info)?);
        // The control side owns the interval and samples while the source still
        // runs. Native client retirement advances stream generations and must
        // not overwrite these running observations with cleanup counters.
        // Retain a bounded startup trace to distinguish initial admission from
        // later starvation. Sampling and serialization stay on this control
        // thread; the final runtime counters still cover the entire interval.
        let duration = Duration::from_secs(options.seconds);
        let observation_interval = Duration::from_secs(options.observation_interval_seconds);
        let mut next_observation = Duration::ZERO;
        soak_control::run_until(
            duration,
            |elapsed| {
                if let Some(journal) = journal.as_ref() {
                    journal.check()?;
                }
                if elapsed >= next_observation
                    && let Some(journal) = journal.as_mut()
                {
                    let runtime = provider
                        .export_observations()
                        .into_iter()
                        .find(|view| view.endpoint_id == endpoint_id);
                    let engine = observer.snapshot();
                    journal.record(&serde_json::json!({
                        "schema_version": 1, "source_kind": options.source_kind,
                        "elapsed_seconds": elapsed.as_secs_f64(),
                        "provider": runtime.as_ref().map(vad_observation_json),
                        "engine": engine_snapshot_value(&engine),
                        "native_observations": observations.snapshot(),
                    }))?;
                    ensure!(
                        runtime.as_ref().is_some_and(|view| view.error.is_none()),
                        "VAD provider failed during diagnostic collection"
                    );
                    ensure!(
                        engine.execution == ExecutionState::Running
                            && engine.active_revision == Some(revision),
                        "Engine stopped or revision changed during diagnostic collection"
                    );
                    next_observation = elapsed.saturating_add(observation_interval);
                }
                if cleanup.startup_samples.len() < STARTUP_SAMPLE_LIMIT {
                    let ((runtime, engine, native), control_observation) =
                        cleanup.observation_clock.observe(|| {
                            let runtime = provider
                                .export_observations()
                                .into_iter()
                                .find(|view| view.endpoint_id == endpoint_id);
                            let engine = observer.snapshot();
                            let native = observations.snapshot();
                            (runtime, engine, native)
                        });
                    cleanup.startup_samples.push(serde_json::json!({
                        "elapsed_seconds": elapsed.as_secs_f64(),
                        "control_observation": control_observation,
                        "timeline": engine_timeline_value(engine.timeline),
                        "provider": runtime.as_ref().map(vad_observation_json),
                        "boundary_probes": native.map(|snapshot| snapshot.boundary_probes),
                    }));
                }
                Ok(())
            },
            || Ok(()),
        )
    })();
    let result = soak_cleanup::run(collection, &mut cleanup);
    drop(cleanup);
    soak_cleanup::combine(result, finish_journal(&mut journal, native_report))
}

pub(super) struct VadCleanup<'a> {
    provider: &'a Arc<WindowsVadProvider>,
    endpoint_id: VirtualEndpointId,
    pub(super) microphone: Option<vad_route::MicrophoneTarget>,
    owned_exports: Vec<VirtualEndpointId>,
    exports_prepared: bool,
    recorder_confirmed: bool,
    initial_recorder: Option<moiren_windows_audio::vad::EndpointSnapshot>,
    pub(super) playback: Option<vad::PlaybackSession>,
    host: Option<EngineHost<f32>>,
    observer: Option<moiren_engine::host::EngineObserver>,
    observations: Option<moiren_windows_audio::engine_backend::NativeObservations>,
    professional_observations:
        Option<moiren_windows_audio::engine_backend::ProfessionalObservations>,
    retained_native: Option<moiren_windows_audio::engine_backend::NativeObservationReadHandle>,
    retained_professional:
        Option<moiren_windows_audio::engine_backend::ProfessionalObservationReadHandle>,
    playback_info: Option<Value>,
    startup_samples: Vec<Value>,
    observation_clock: control_observation::ControlObservationClock,
    pub(super) report: &'a mut NativeReport,
    pub(super) playback_outcome: Option<vad::PlaybackOutcome>,
    pub(super) admission: Option<vad_collection::Admission>,
    pub(super) last_health: Option<vad_collection::Health>,
}

impl VadCleanup<'_> {
    pub(super) fn current_health(&mut self) -> Result<vad_collection::Health> {
        use moiren_windows_audio::engine_backend::NativeStreamKind;
        let ((exports, engine, native), control_observation) =
            self.observation_clock.observe(|| {
                let exports = self.provider.export_observations();
                let engine = self.observer.as_ref().map(|observer| observer.snapshot());
                let native = self.observations.as_ref().and_then(|view| view.snapshot());
                (exports, engine, native)
            });
        let runtime = exports
            .iter()
            .find(|view| view.endpoint_id == self.endpoint_id);
        let microphone_runtime = self.microphone.as_ref().and_then(|target| {
            exports
                .iter()
                .find(|view| view.endpoint_id == target.endpoint_id)
        });
        let playback = endpoint_health(runtime, self.endpoint_id, Intent::Playback);
        let microphone = self.microphone.as_ref().map(|target| {
            endpoint_health(microphone_runtime, target.endpoint_id, Intent::Microphone)
        });
        let mut faults = playback.quality_faults.saturating_add(
            microphone
                .as_ref()
                .map_or(0, |health| health.quality_faults),
        );
        if let Some(value) = &native {
            for probe in &value.boundary_probes {
                faults = faults.saturating_add(probe.xruns);
            }
            for bridge in &value.bridges {
                let count = bridge.counters;
                for fault in [
                    count.dropped_frames,
                    count.backlog_discarded_frames,
                    count.age_discarded_frames,
                    count.underrun_frames,
                    count.nonfinite_samples,
                    count.discontinuities,
                    count.timestamp_errors,
                    count.resets,
                ] {
                    faults = faults.saturating_add(fault);
                }
            }
        }
        let route = if self.microphone.is_some() {
            vad_route::RouteKind::Microphone
        } else {
            vad_route::RouteKind::Physical
        };
        let route_quality_known = native.as_ref().is_some_and(|native| {
            let has_kind = |kind| native.streams.iter().any(|stream| stream.kind == kind);
            let has_boundary = |kind| {
                let streams: Vec<_> = native
                    .streams
                    .iter()
                    .filter(|stream| stream.kind == kind)
                    .collect();
                streams.len() == 1
                    && native
                        .boundary_probes
                        .iter()
                        .any(|probe| probe.kind == kind && probe.node == streams[0].node)
            };
            has_boundary(NativeStreamKind::VirtualPlayback)
                && match route {
                    vad_route::RouteKind::Physical => has_kind(NativeStreamKind::MasterOutput),
                    vad_route::RouteKind::Microphone => {
                        !has_kind(NativeStreamKind::MasterOutput)
                            && has_boundary(NativeStreamKind::VirtualMicrophone)
                    }
                }
        });
        let health = vad_collection::Health {
            route,
            microphone,
            stamp: playback.stamp,
            shape: playback.shape,
            provider_ready: playback.provider_ready,
            provider_error: playback.provider_error,
            revision: engine.as_ref().and_then(|engine| engine.active_revision),
            generations: engine.as_ref().map_or([0, 0], |engine| {
                [engine.owner_generation, engine.resource_generation]
            }),
            engine_running: engine.as_ref().is_some_and(|engine| {
                engine.execution == ExecutionState::Running
                    && engine.timeline.is_some_and(|timeline| timeline.frame > 0)
            }),
            engine_fault: engine
                .as_ref()
                .is_some_and(|engine| engine.fault.is_some() || engine.cleanup_error.is_some()),
            native_generation: native.as_ref().map(|native| native.owner_generation),
            native_running: native.as_ref().is_some_and(|native| native.running),
            quality_known: playback.quality_known
                && microphone_runtime.is_none_or(|view| view.telemetry.is_some())
                && (route == vad_route::RouteKind::Physical || microphone_runtime.is_some())
                && route_quality_known,
            quality_faults: faults,
        };
        // Keep the failing generation before teardown advances either stream.
        self.report.vad_provider = runtime.map(vad_observation_json);
        self.report.vad_microphone_provider = microphone_runtime.map(vad_observation_json);
        if let Some(engine) = &engine {
            let mut value = engine_snapshot_value(engine);
            value["observations"] = serde_json::json!(native);
            replace_engine_observation(&mut self.report.engine, value);
        }
        if self.startup_samples.len() < STARTUP_SAMPLE_LIMIT {
            self.startup_samples.push(serde_json::json!({
                "control_observation": control_observation,
                "timeline": engine_timeline_value(engine.as_ref().and_then(|engine| engine.timeline)),
                "health": health, "provider": self.report.vad_provider,
                "microphone_provider": self.report.vad_microphone_provider,
                "native_observations": native,
            }));
        }
        self.last_health = Some(health.clone());
        Ok(health)
    }

    pub(super) fn prepare_exports(&mut self, options: &LiveOptions, name: &str) -> Result<()> {
        if self.exports_prepared {
            return Ok(());
        }
        let requested = options.microphone_target()?;
        ensure!(
            requested
                .as_ref()
                .map(|target| (&target.endpoint_id, &target.name))
                == self
                    .microphone
                    .as_ref()
                    .map(|target| (&target.endpoint_id, &target.name)),
            "VAD route changed after cleanup ownership was installed"
        );
        let mut exports = vec![(self.endpoint_id, Intent::Playback, name.to_owned())];
        if let Some(target) = &self.microphone {
            exports.push((target.endpoint_id, Intent::Microphone, target.name.clone()));
        }
        prepare_owned_exports(
            &exports,
            &mut self.owned_exports,
            |id| {
                self.provider
                    .export_observations()
                    .into_iter()
                    .find(|view| view.endpoint_id == id)
            },
            |id, intent, name| {
                self.provider
                    .create_export(id, intent, Channels::Stereo, name, false)
                    .map_err(Into::into)
            },
            |id, intent| {
                let snapshot = wait_vad_ready(self.provider, id)?;
                ensure!(
                    snapshot.stable_id() == stable_from_virtual(id) && snapshot.intent() == intent,
                    "VAD export readiness belongs to another identity/direction"
                );
                Ok(())
            },
        )?;
        self.exports_prepared = true;
        self.report.vad_route = options.route_intent();
        Ok(())
    }
    pub(super) fn prepare(
        &mut self,
        options: &LiveOptions,
        name: &str,
        root: String,
        source_plan: Option<source::PreparedSource>,
        before_join: Option<&mut dyn FnMut()>,
    ) -> Result<()> {
        let supervised = before_join.is_some();
        self.prepare_exports(options, name)?;
        self.playback = Some(
            if let Some(abort) = before_join {
                vad::PlaybackSession::prepare_with_abort(
                    stable_from_virtual(self.endpoint_id),
                    root,
                    source_plan,
                    options.vad_minimum_period,
                    abort,
                    &mut self.playback_outcome,
                )
            } else {
                vad::PlaybackSession::prepare(
                    stable_from_virtual(self.endpoint_id),
                    root,
                    source_plan,
                    options.vad_minimum_period,
                    &mut self.playback_outcome,
                )
            }
            .context("prepare VAD native playback")?,
        );
        self.playback_info = self
            .playback
            .as_ref()
            .map(|playback| serde_json::to_value(&playback.info))
            .transpose()?;
        let output = EndpointSelector::PinnedEndpoint {
            endpoint_id: options.output_endpoint.clone(),
            stable_id: None,
        };
        let mut project = if let Some(target) = &self.microphone {
            vad_route::minimal_project(
                self.endpoint_id,
                OutputTarget::VirtualMicrophone {
                    endpoint_id: target.endpoint_id,
                },
            )?
        } else {
            minimal_vad_project(self.endpoint_id, output)?
        };
        let physical_output = vad_route::apply_physical_render_request(
            &mut project,
            options.physical_render_request,
        )?;
        project.processing.engine.max_block_frames = options.engine_block_frames as usize;
        let backend = ProfessionalBackend::<f32>::new()
            .context("prepare Professional backend")?
            .with_render_queue(render_queue(options)?)?
            .with_virtual_buffer(virtual_buffer(options)?)?
            .with_telemetry(options.telemetry)
            .with_virtual_provider(self.provider.clone());
        self.observations = Some(backend.native_observations());
        self.professional_observations = Some(backend.observations());
        let host = EngineHost::open(HostConfig::default(), project, backend)
            .context("open VAD EngineHost")?;
        self.observer = Some(host.observer());
        self.host = Some(host);
        let observer = self.observer.as_ref().expect("prepared host observer");
        wait_engine_running(observer)?;
        let active = observer.snapshot();
        if let Some(expected_node) = physical_output {
            let native = self.observations.as_ref().and_then(|view| view.snapshot());
            retain_and_validate_physical_render(
                options.physical_render_request,
                expected_node,
                &active,
                native.as_ref(),
                &mut self.report.engine,
            )?;
        }
        let revision = active
            .active_revision
            .context("VAD EngineHost has no active revision")?;
        let current = self
            .provider
            .export_observations()
            .into_iter()
            .find(|view| view.endpoint_id == self.endpoint_id)
            .and_then(|view| view.snapshot)
            .context("VAD export disappeared before gate admission")?;
        ensure!(
            current.state() == PdoState::Present,
            "VAD export is not Present"
        );
        ensure!(
            current.policy_prepared() && current.audio_ready(),
            "VAD export is not audio-ready"
        );
        let before_gate = snapshot_stamp(current);
        let before_shape = snapshot_shape(current);
        let mut gate_snapshots = vec![(self.endpoint_id, current)];
        if let Some(target) = &self.microphone {
            let microphone = self
                .provider
                .export_observations()
                .into_iter()
                .find(|view| view.endpoint_id == target.endpoint_id)
                .context("VAD microphone disappeared before gate admission")?;
            let snapshot = microphone
                .snapshot
                .context("VAD microphone shape unavailable")?;
            ensure!(
                microphone.error.is_none()
                    && !microphone.removing
                    && snapshot.stable_id() == stable_from_virtual(target.endpoint_id)
                    && snapshot.intent() == Intent::Microphone
                    && snapshot.state() == PdoState::Present
                    && snapshot.policy_prepared()
                    && snapshot.audio_ready()
                    && snapshot.channels() == Channels::Stereo
                    && snapshot.sample_format().is_some()
                    && snapshot.packet_frames() > 0,
                "VAD microphone is not ready with its prepared recorder shape"
            );
            self.initial_recorder = Some(snapshot);
            gate_snapshots.push((target.endpoint_id, snapshot));
        }
        // Both endpoint gates share one pinned Engine admission authority.
        let ticket = self.provider.set_active_revision(Some(revision));
        let opened = open_prepared_gates(
            &gate_snapshots,
            revision,
            ticket,
            |id, stamp, revision, ticket| {
                self.provider
                    .set_export_gate(id, stamp, true, revision, ticket)?;
                let observed = self
                    .provider
                    .export_observations()
                    .into_iter()
                    .find(|view| view.endpoint_id == id)
                    .context("opened VAD export disappeared")?;
                ensure!(
                    observed.error.is_none() && !observed.removing,
                    "opened VAD export observation failed: {:?}",
                    observed.error
                );
                observed
                    .snapshot
                    .context("opened VAD export snapshot unavailable")
            },
        )?;
        let admitted = self.current_health()?;
        self.retained_native = self
            .observations
            .as_ref()
            .and_then(|view| view.retain_session());
        self.retained_professional = self
            .professional_observations
            .as_ref()
            .and_then(|view| view.retain_session());
        self.admission = Some(vad_collection::Admission {
            before_gate,
            current: admitted,
        });
        if supervised {
            let admitted = &self.admission.as_ref().expect("retained admission").current;
            ensure!(
                admitted
                    .stamp
                    .is_some_and(|stamp| stamp == snapshot_stamp(opened[0].1))
                    && admitted.shape.as_ref() == Some(&before_shape)
                    && admitted.revision == Some(revision)
                    && admitted.generations
                        == [active.owner_generation, active.resource_generation],
                "VAD stream/shape or graph changed during gate admission"
            );
            if let Some((_, initial)) = opened.get(1) {
                let microphone = admitted
                    .microphone
                    .as_ref()
                    .context("microphone admission missing")?;
                ensure!(
                    microphone
                        .stamp
                        .is_some_and(|stamp| stamp == snapshot_stamp(*initial))
                        && microphone.shape.as_ref() == Some(&snapshot_shape(*initial)),
                    "VAD microphone changed during initial gate admission"
                );
            }
        }
        Ok(())
    }

    pub(super) fn confirm_recorder_started(&mut self, deadline: Instant) -> Result<()> {
        if self.microphone.is_none() {
            return Ok(());
        }
        ensure!(
            !self.recorder_confirmed,
            "recorder Start was already confirmed"
        );
        // Spend the one startup window even on error. Never adopt a later epoch.
        self.recorder_confirmed = true;
        let previous = self
            .admission
            .as_ref()
            .context("initial VAD admission missing")?
            .current
            .clone();
        let initial = self
            .initial_recorder
            .context("initial recorder snapshot missing")?;
        ensure!(
            matches!(
                initial.stream_state(),
                moiren_windows_audio::vad::StreamState::Paused
                    | moiren_windows_audio::vad::StreamState::Running
            ),
            "recorder was not prepared before initial admission"
        );
        let target = self
            .microphone
            .as_ref()
            .expect("microphone route")
            .endpoint_id;
        // export_observations issues fresh QUERYs, not a cached-only read. The
        // recorder's Running notification can still precede native RUN visibility.
        // AcquireStream already ran during preparation; first RUN changes state
        // only (Children.cpp / Streams.cpp). Changed stream/gate epochs are not
        // proven initial Start, even if the gate stayed open: refuse, never rebase.
        wait_initial_recorder(
            deadline,
            Instant::now,
            || {
                let exports = self.provider.export_observations();
                let microphone = exports
                    .iter()
                    .find(|view| view.endpoint_id == target)
                    .context("microphone disappeared during recorder Start")?;
                let running = recorder_start_running(
                    microphone,
                    target,
                    previous
                        .microphone
                        .as_ref()
                        .context("microphone baseline missing")?,
                )?;
                let current = self.current_health()?;
                ensure!(
                    current.generations == previous.generations
                        && current.revision == previous.revision
                        && current.native_generation == previous.native_generation
                        && current.native_running
                        && current.engine_running
                        && !current.engine_fault
                        && current.stamp == previous.stamp
                        && current.shape == previous.shape
                        && current.provider_ready
                        && current.provider_error.is_none()
                        && current.quality_known,
                    "recorder confirmation changed graph/playback admission or lost evidence"
                );
                let microphone = current
                    .microphone
                    .as_ref()
                    .context("confirmed microphone missing")?;
                let before = previous
                    .microphone
                    .as_ref()
                    .context("microphone baseline missing")?;
                // Pause does not advance native stamps. Confirm Running on
                // this newer health query, never on the earlier QUERY alone.
                let latest_running = recorder_confirmation_running(before, microphone)?;
                // Keep measured faults; caller applies latency/correctness policy.
                // The original Admission remains unchanged, including all five stamps.
                Ok((running && latest_running).then_some(()))
            },
            thread::sleep,
        )
    }
}

impl soak_cleanup::Cleanup for VadCleanup<'_> {
    fn observe_final(&mut self) -> Result<()> {
        if let Some(observer) = &self.observer {
            let mut engine = engine_snapshot_value(&observer.snapshot());
            engine["observations"] = serde_json::json!(
                self.observations
                    .as_ref()
                    .and_then(|handle| handle.snapshot())
            );
            engine["playback_info"] = serde_json::json!(self.playback_info);
            engine["startup_trace"] = serde_json::json!({
                "sample_limit": STARTUP_SAMPLE_LIMIT,
                "sample_interval_ms": STARTUP_SAMPLE_INTERVAL.as_millis(),
                "scope": "bounded startup diagnostics; final counters cover the full run",
                "timestamp_scope": control_observation::SCOPE,
                "samples": self.startup_samples,
            });
            replace_engine_observation(&mut self.report.engine, engine);
        }
        let exports = self.provider.export_observations();
        each_owned_export(&self.owned_exports, |id| {
            let observation = exports
                .iter()
                .find(|view| view.endpoint_id == id)
                .context("VAD export disappeared during measurement")?;
            if id == self.endpoint_id {
                self.report.vad_provider = Some(vad_observation_json(observation));
            } else {
                self.report.vad_microphone_provider = Some(vad_observation_json(observation));
            }
            if let Some(error) = &observation.error {
                bail!("VAD final observation {id:?}: {error}");
            }
            Ok(())
        })
    }

    fn stop_source(&mut self) -> Result<()> {
        if let Some(playback) = self.playback.take() {
            let report = playback.stop().context("stop VAD native playback")?;
            self.playback_outcome = Some(report);
        }
        if let Some(outcome) = &self.playback_outcome {
            self.report.vad_playback = Some(serde_json::to_value(outcome)?);
            ensure!(
                outcome.failure.is_none(),
                "VAD native playback: {}",
                outcome.failure.as_deref().unwrap_or_default()
            );
        }
        Ok(())
    }

    fn close_gate(&mut self) -> Result<()> {
        each_owned_export(&self.owned_exports, |id| close_vad_gate(self.provider, id))
    }

    fn shutdown_host(&mut self) -> Result<()> {
        // Pin this exact owner before live registries are cleared. Reading
        // after the host joins retains trailing counters, not a pre-stop
        // estimate or another generation's observations.
        let native = self.retained_native.take().or_else(|| {
            self.observations
                .as_ref()
                .and_then(|view| view.retain_session())
        });
        let professional = self.retained_professional.take().or_else(|| {
            self.professional_observations
                .as_ref()
                .and_then(|view| view.retain_session())
        });
        let expected_generation = self
            .admission
            .as_ref()
            .map(|admission| admission.current.generations[0]);
        drop(self.host.take());
        let closed = self.observer.as_ref().map(|observer| observer.snapshot());
        self.report.backend_diagnostics = Some(final_backend_evidence(
            native.map(|view| view.snapshot()),
            professional.map(|view| view.snapshot()),
            closed.as_ref(),
            expected_generation,
        ));
        if let Some(snapshot) = closed {
            self.report.engine_cleanup = Some(engine_snapshot_value(&snapshot));
            if let Some(error) = snapshot.cleanup_error {
                bail!("VAD EngineHost cleanup: {error}");
            }
            ensure!(
                snapshot.host == moiren_engine::host::HostState::Closed,
                "VAD EngineHost did not confirm Closed after shutdown"
            );
            ensure!(
                snapshot.fault.is_none(),
                "VAD EngineHost retained a terminal fault after shutdown"
            );
        }
        Ok(())
    }

    fn invalidate_admission(&mut self) -> Result<()> {
        // Shared atomic stop intent for both exports, before native Stop. Gate
        // closure alone performs QUERY/IO and does not invalidate this ticket.
        self.provider.set_active_revision(None);
        Ok(())
    }

    fn retire_endpoint(&mut self) -> Result<()> {
        each_owned_export(&self.owned_exports, |id| {
            let receipt = if id == self.endpoint_id {
                &mut self.report.vad_cleanup
            } else {
                &mut self.report.vad_microphone_cleanup
            };
            let observation = retire_vad(self.provider, id, receipt)?;
            if let Some(error) = observation.error {
                bail!("VAD provider cleanup {id:?}: {error}");
            }
            Ok(())
        })
    }
}

fn replace_engine_observation(evidence: &mut Option<Value>, mut observation: Value) {
    // Admission belongs to the pre-gate decision, not the latest poll or
    // teardown generation. Keep it separate while existing live fields advance.
    if let Some(previous) = evidence.as_ref() {
        for key in ["physical_render_request", "physical_render_admission"] {
            if let Some(retained) = previous.get(key) {
                observation[key] = retained.clone();
            }
        }
    }
    *evidence = Some(observation);
}

fn retain_and_validate_physical_render(
    request: render_request::PhysicalRenderRequest,
    expected_node: moiren_core::graph::NodeId,
    active: &moiren_engine::host::HostSnapshot,
    native: Option<&moiren_windows_audio::engine_backend::NativeObservationSnapshot>,
    evidence: &mut Option<Value>,
) -> Result<()> {
    use moiren_windows_audio::engine_backend::NativeStreamKind;
    let mut retained = engine_snapshot_value(active);
    retained["observations"] = serde_json::json!(native);
    retained["physical_render_request"] = serde_json::to_value(request)?;
    let mut admission = retained.clone();
    admission["expected_output_node"] = format!("{expected_node:?}").into();
    admission["scope"] = "pre_gate_physical_render_validation".into();
    let validation = (|| -> Result<()> {
        let native =
            native.context("VAD physical render observations unavailable before gate admission")?;
        ensure!(
            native.running
                && native.owner_generation == active.owner_generation
                && active.execution == ExecutionState::Running
                && active.active_revision.is_some()
                && active.fault.is_none()
                && active.cleanup_error.is_none(),
            "VAD physical render observations do not belong to the current Running owner"
        );
        let mut physical = native.streams.iter().filter(|stream| {
            matches!(
                stream.kind,
                NativeStreamKind::MasterOutput | NativeStreamKind::FollowerOutput
            )
        });
        let output = physical
            .next()
            .context("VAD physical render observation missing")?;
        ensure!(
            physical.next().is_none()
                && output.node == expected_node
                && output.kind == NativeStreamKind::MasterOutput,
            "VAD physical render observation is ambiguous or not the selected MasterOutput node"
        );
        request.validate_observed(
            output
                .timing
                .context("VAD physical render timing unavailable")?,
            output.shared_period,
        )
    })();
    admission["validation_accepted"] = validation.is_ok().into();
    admission["failure"] =
        serde_json::json!(validation.as_ref().err().map(|error| format!("{error:#}")));
    retained["physical_render_admission"] = admission;
    *evidence = Some(retained);
    validation
}

fn final_backend_evidence(
    native: Option<moiren_windows_audio::engine_backend::NativeObservationSnapshot>,
    professional: Option<moiren_windows_audio::engine_backend::ProfessionalObservationSnapshot>,
    closed: Option<&moiren_engine::host::HostSnapshot>,
    expected_generation: Option<u64>,
) -> Value {
    let failure = (|| -> Result<()> {
        let expected = expected_generation.context("VAD final admitted generation missing")?;
        let native = native
            .as_ref()
            .context("VAD final native observations missing")?;
        let professional = professional
            .as_ref()
            .context("VAD final professional observations missing")?;
        let closed = closed.context("VAD final host snapshot missing")?;
        ensure!(
            closed.host == moiren_engine::host::HostState::Closed
                && closed.cleanup_error.is_none()
                && closed.fault.is_none(),
            "VAD final host did not join cleanly"
        );
        ensure!(
            native.owner_generation == expected
                && professional.owner_generation == expected
                && closed.owner_generation == expected,
            "VAD final observation owner generation mismatch"
        );
        ensure!(
            !native.running && !professional.running,
            "VAD final observations still report Running"
        );
        Ok(())
    })()
    .err()
    .map(|error| format!("{error:#}"));
    let professional = professional.as_ref().map(|snapshot| {
        serde_json::json!({
            "owner_generation": snapshot.owner_generation,
            "running": snapshot.running,
            "streams": snapshot.streams.iter().map(|stream| serde_json::json!({
                "node": format!("{:?}", stream.node),
                "input": stream.input, "master": stream.master,
                "actual": {"sample_rate": stream.actual.sample_rate,
                    "channels": stream.actual.channels,
                    "buffer_frames": stream.actual.buffer_frames},
                "native_callbacks": stream.native_callbacks,
                "native_wait_wakes": stream.native_wait_wakes,
                "native_wait_timeouts": stream.native_wait_timeouts,
                "native_written_frames": stream.native_written_frames,
                "dropped_frames": stream.dropped_frames,
                "silent_frames": stream.silent_frames,
                "native_dropped_frames": stream.native_dropped_frames,
                "native_silent_frames": stream.native_silent_frames,
            })).collect::<Vec<_>>(),
        })
    });
    serde_json::json!({
        "schema_version": 2,
        "observation_scope": "final_after_owner_join",
        "owner_generation": expected_generation,
        "failure": failure,
        "professional": professional,
        "native": native,
    })
}

#[cfg(test)]
mod final_evidence_tests {
    use super::*;
    use moiren_windows_audio::engine_backend::{
        NativeObservationSnapshot, ProfessionalObservationSnapshot,
    };

    fn closed() -> moiren_engine::host::HostSnapshot {
        moiren_engine::host::HostSnapshot {
            host: moiren_engine::host::HostState::Closed,
            execution: ExecutionState::Dormant,
            desired_revision: 1,
            active_revision: None,
            plan_revision: None,
            owner_generation: 7,
            resource_generation: 1,
            timeline: None,
            update: moiren_engine::host::UpdateState::Idle,
            fault: None,
            cleanup_error: None,
        }
    }

    fn native() -> NativeObservationSnapshot {
        NativeObservationSnapshot {
            process_delivery: Vec::new(),
            owner_generation: 7,
            running: false,
            streams: Vec::new(),
            bridges: Vec::new(),
            boundary_probes: Vec::new(),
            render_padding: Vec::new(),
        }
    }

    fn professional() -> ProfessionalObservationSnapshot {
        ProfessionalObservationSnapshot {
            owner_generation: 7,
            running: false,
            streams: Vec::new(),
        }
    }

    #[test]
    fn final_diagnostics_require_joined_host_and_matching_retained_sessions() {
        let value = final_backend_evidence(
            Some(native()),
            Some(professional()),
            Some(&closed()),
            Some(7),
        );
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["observation_scope"], "final_after_owner_join");
        assert!(value["failure"].is_null());
        assert_eq!(value["native"]["owner_generation"], 7);
        assert_eq!(value["professional"]["owner_generation"], 7);
        assert!(
            value.get("render").is_none(),
            "do not synthesize the legacy render report"
        );
        for case in 0..7 {
            let mut native = Some(native());
            let mut professional = Some(professional());
            let mut closed = closed();
            let mut expected = Some(7);
            match case {
                0 => native = None,
                1 => professional = None,
                2 => native.as_mut().unwrap().owner_generation = 8,
                3 => professional.as_mut().unwrap().running = true,
                4 => closed.host = moiren_engine::host::HostState::Open,
                5 => {
                    closed.cleanup_error = Some(moiren_engine::host::BackendError::Failed(
                        "Stop failed".into(),
                    ))
                }
                _ => expected = None,
            }
            assert!(
                final_backend_evidence(native, professional, Some(&closed), expected)["failure"]
                    .is_string(),
                "accepted invalid final evidence case {case}"
            );
        }
    }
}

fn endpoint_health(
    view: Option<&moiren_windows_audio::vad::VadExportObservation>,
    id: VirtualEndpointId,
    intent: Intent,
) -> vad_collection::EndpointHealth {
    let snapshot = view.and_then(|view| view.snapshot);
    let telemetry = view.and_then(|view| view.telemetry);
    let quality_faults = telemetry.map_or(0, |value| {
        [
            value.dropped_packets,
            value.driver_dropped_packets,
            value.quota_dropped_packets,
            value.carrier_dropped_packets,
            value.stale_packets,
            value.epoch_resets,
            value.recovery_events,
            value.completion_clock_regressions,
            value.missing_sequence_slots,
        ]
        .into_iter()
        .fold(0u64, u64::saturating_add)
    });
    vad_collection::EndpointHealth {
        stamp: snapshot.map(snapshot_stamp),
        shape: snapshot.map(snapshot_shape),
        stream_running: snapshot.is_some_and(|snapshot| {
            snapshot.stream_state() == moiren_windows_audio::vad::StreamState::Running
        }),
        provider_ready: snapshot.is_some_and(|snapshot| {
            snapshot.stable_id() == stable_from_virtual(id)
                && snapshot.intent() == intent
                && snapshot.state() == PdoState::Present
                && snapshot.policy_prepared()
                && snapshot.audio_ready()
                && snapshot.gate_open()
                && snapshot.channels() == Channels::Stereo
                && snapshot.sample_format().is_some()
                && snapshot.packet_frames() > 0
        }) && view
            .is_some_and(|view| view.endpoint_id == id && view.intent == intent && !view.removing),
        provider_error: view.and_then(|view| view.error.clone()),
        quality_known: telemetry.is_some(),
        quality_faults,
    }
}

fn snapshot_shape(snapshot: moiren_windows_audio::vad::EndpointSnapshot) -> (String, String, u32) {
    (
        format!("{:?}", snapshot.channels()),
        format!("{:?}", snapshot.sample_format()),
        snapshot.packet_frames(),
    )
}

fn stable_from_virtual(id: VirtualEndpointId) -> moiren_windows_audio::vad::StableEndpointId {
    moiren_windows_audio::vad::StableEndpointId::from_virtual_endpoint_id(id)
}

fn vad_observation_json(observation: &moiren_windows_audio::vad::VadExportObservation) -> Value {
    serde_json::json!({
        "endpoint_id": observation.endpoint_id,
        "spec_name": observation.spec_name,
        "transport_prepared": observation.transport_prepared,
        "removing": observation.removing,
        "diagnostic_provenance": format!("{:?}", observation.diagnostic_provenance),
        "telemetry": observation.telemetry,
        "error": observation.error,
        "snapshot": observation.snapshot.map(|snapshot| serde_json::json!({
            "state": format!("{:?}", snapshot.state()),
            "gate_open": snapshot.gate_open(),
            "policy_prepared": snapshot.policy_prepared(),
            "audio_ready": snapshot.audio_ready(),
            "packet_frames": snapshot.packet_frames(),
            "stream_generation": snapshot.stream_generation(),
            "gate_epoch": snapshot.gate_epoch(),
            "dropped_packets": snapshot.dropped_packets(),
            "silence_packets": snapshot.silence_packets(),
        })),
    })
}

fn snapshot_stamp(snapshot: moiren_windows_audio::vad::EndpointSnapshot) -> [u64; 5] {
    [
        snapshot.pdo_token().get(),
        snapshot.session_epoch(),
        snapshot.endpoint_generation(),
        snapshot.stream_generation(),
        snapshot.gate_epoch(),
    ]
}

fn wait_vad_ready(
    provider: &WindowsVadProvider,
    endpoint_id: VirtualEndpointId,
) -> Result<moiren_windows_audio::vad::EndpointSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(view) = provider
            .export_observations()
            .into_iter()
            .find(|view| view.endpoint_id == endpoint_id)
        {
            if let Some(snapshot) = view.snapshot
                && snapshot.state() == PdoState::Present
                && snapshot.policy_prepared()
                && snapshot.audio_ready()
                && view.error.is_none()
            {
                return Ok(snapshot);
            }
            if let Some(error) = view.error {
                bail!("VAD CREATE failed: {error}");
            }
        }
        ensure!(
            Instant::now() < deadline,
            "VAD export did not become audio-ready"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_engine_running(observer: &moiren_engine::host::EngineObserver) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = observer.snapshot();
        if snapshot.execution == ExecutionState::Running
            && snapshot.active_revision == Some(1)
            && snapshot.timeline.is_some_and(|timeline| timeline.frame > 0)
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "VAD EngineHost did not reach Running; last snapshot: {:?}",
            observer.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn engine_timeline_value(timeline: Option<moiren_engine::host::OwnerTimeline>) -> Value {
    serde_json::json!(timeline.map(|timeline| serde_json::json!({
        "epoch": timeline.epoch,
        "revision": timeline.revision,
        "frame": timeline.frame,
    })))
}

fn engine_snapshot_value(snapshot: &moiren_engine::host::HostSnapshot) -> Value {
    serde_json::json!({
        "host": format!("{:?}", snapshot.host),
        "execution": format!("{:?}", snapshot.execution),
        "desired_revision": snapshot.desired_revision,
        "active_revision": snapshot.active_revision,
        "plan_revision": snapshot.plan_revision,
        "owner_generation": snapshot.owner_generation,
        "resource_generation": snapshot.resource_generation,
        "timeline": engine_timeline_value(snapshot.timeline),
        "update": format!("{:?}", snapshot.update),
        "fault": format!("{:?}", snapshot.fault),
        "cleanup_error": format!("{:?}", snapshot.cleanup_error),
    })
}

fn minimal_vad_project(
    endpoint_id: VirtualEndpointId,
    output: EndpointSelector,
) -> Result<EngineProject> {
    vad_route::minimal_project(endpoint_id, OutputTarget::Physical(output))
}

fn close_vad_gate(provider: &WindowsVadProvider, endpoint_id: VirtualEndpointId) -> Result<()> {
    let view = provider
        .export_observations()
        .into_iter()
        .find(|view| view.endpoint_id == endpoint_id)
        .context("VAD export disappeared before gate close")?;
    let observed = view.error.map_or(Ok(()), |error| {
        Err(anyhow::anyhow!(
            "VAD gate-close observation {endpoint_id:?}: {error}"
        ))
    });
    let closed = (|| {
        let current = view.snapshot.context("VAD gate-close snapshot unknown")?;
        if current.gate_open() {
            provider
                .set_export_gate(endpoint_id, snapshot_stamp(current), false, 0, 0)
                .context("close VAD export gate")?;
        }
        Ok(())
    })();
    soak_cleanup::combine(observed, closed)
}

fn retire_vad(
    provider: &WindowsVadProvider,
    endpoint_id: VirtualEndpointId,
    last_observation: &mut Option<Value>,
) -> Result<moiren_windows_audio::vad::VadExportObservation> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let view = provider
            .export_observations()
            .into_iter()
            .find(|view| view.endpoint_id == endpoint_id)
            .context("VAD export missing during cleanup")?;
        *last_observation = Some(vad_observation_json(&view));
        let Some(snapshot) = view.snapshot else {
            ensure!(
                Instant::now() < deadline,
                "VAD cleanup snapshot became uncertain"
            );
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        if snapshot.state() == PdoState::Removed {
            return Ok(view);
        }
        if snapshot.gate_open() {
            provider.set_export_gate(endpoint_id, snapshot_stamp(snapshot), false, 0, 0)?;
        } else if snapshot.state() != PdoState::Removing {
            provider.retire_export(endpoint_id, snapshot_stamp(snapshot))?;
        }
        ensure!(
            Instant::now() < deadline,
            "VAD cleanup did not reach Removed"
        );
        thread::sleep(Duration::from_millis(50));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::{boundary::ClockRole, host::NodeConfig};
    use moiren_windows_audio::vad::{
        Capabilities, EndpointSnapshot, StreamState, VadExportDiagnosticProvenance,
        VadExportObservation, VadPcmTelemetrySnapshot,
    };
    use std::cell::{Cell, RefCell};

    fn physical_admission_fixture() -> (
        moiren_core::graph::NodeId,
        moiren_engine::host::HostSnapshot,
        moiren_windows_audio::engine_backend::NativeObservationSnapshot,
    ) {
        use moiren_windows_audio::engine_backend::{
            NativeObservationSnapshot, NativeStreamKind, NativeStreamObservation,
        };
        use moiren_windows_audio::latency::{
            NativeStreamTiming,
            shared_period::{SharedInitializationApi, SharedPeriodReport},
        };
        let project = minimal_vad_project(
            VirtualEndpointId::from_bytes([1; 16]),
            EndpointSelector::PinnedEndpoint {
                endpoint_id: "physical target".into(),
                stable_id: None,
            },
        )
        .unwrap();
        let expected = project
            .nodes
            .iter()
            .find_map(|(node, config)| matches!(config, NodeConfig::Output(_)).then_some(*node))
            .unwrap();
        let active = moiren_engine::host::HostSnapshot {
            host: moiren_engine::host::HostState::Open,
            execution: ExecutionState::Running,
            desired_revision: 1,
            active_revision: Some(1),
            plan_revision: Some(1),
            owner_generation: 7,
            resource_generation: 1,
            timeline: Some(moiren_engine::host::OwnerTimeline {
                epoch: 1,
                revision: 1,
                frame: 64,
            }),
            update: moiren_engine::host::UpdateState::Idle,
            fault: None,
            cleanup_error: None,
        };
        let native = NativeObservationSnapshot {
            owner_generation: 7,
            running: true,
            process_delivery: Vec::new(),
            bridges: Vec::new(),
            boundary_probes: Vec::new(),
            render_padding: Vec::new(),
            streams: vec![NativeStreamObservation {
                node: expected,
                kind: NativeStreamKind::MasterOutput,
                timing: Some(NativeStreamTiming {
                    sample_rate: 48_000,
                    period_frames: Some(128),
                    capacity_frames: 512,
                }),
                shared_period: Some(SharedPeriodReport {
                    requested_period_frames: Some(128),
                    actual_period_frames: Some(128),
                    initialization_api: Some(SharedInitializationApi::Client3),
                    ..Default::default()
                }),
                render_queue: None,
                input_queue: None,
                virtual_budget: None,
            }],
        };
        (expected, active, native)
    }

    #[test]
    fn physical_render_vad_admission_refuses_missing_ambiguous_wrong_or_stale_observations() {
        use moiren_windows_audio::engine_backend::NativeStreamKind;
        let request = render_request::PhysicalRenderRequest {
            period_frames: Some(128),
            ..Default::default()
        };
        for case in 0..8 {
            let (expected, active, mut native) = physical_admission_fixture();
            match case {
                0 => native.streams.clear(),
                1 => native.streams.push(native.streams[0].clone()),
                2 => native.streams[0].kind = NativeStreamKind::FollowerOutput,
                3 => {
                    let other = minimal_vad_project(
                        VirtualEndpointId::from_bytes([2; 16]),
                        EndpointSelector::FollowDefault(
                            moiren_engine::node::DefaultDeviceRole::Console,
                        ),
                    )
                    .unwrap();
                    native.streams[0].node = other
                        .graph
                        .nodes()
                        .iter()
                        .find(|node| matches!(other.nodes[&node.id()], NodeConfig::Input(_)))
                        .unwrap()
                        .id();
                }
                4 => native.owner_generation += 1,
                5 => native.running = false,
                6 => native.streams[0].timing = None,
                _ => native.streams[0].shared_period = None,
            }
            let mut retained = None;
            assert!(
                retain_and_validate_physical_render(
                    request,
                    expected,
                    &active,
                    Some(&native),
                    &mut retained
                )
                .is_err(),
                "case {case}"
            );
            let retained = retained.unwrap();
            assert_eq!(
                retained["observations"]["owner_generation"],
                native.owner_generation
            );
            assert_eq!(retained["physical_render_request"]["period_frames"], 128);
            assert_eq!(
                retained["physical_render_admission"]["validation_accepted"],
                false
            );
            assert!(retained["physical_render_admission"]["failure"].is_string());
        }
        let (expected, active, _) = physical_admission_fixture();
        let mut retained = None;
        assert!(
            retain_and_validate_physical_render(request, expected, &active, None, &mut retained)
                .is_err()
        );
        let retained = retained.unwrap();
        assert!(retained["observations"].is_null());
        assert_eq!(
            retained["physical_render_admission"]["validation_accepted"],
            false
        );
        assert!(
            retained["physical_render_admission"]["failure"]
                .as_str()
                .unwrap()
                .contains("unavailable")
        );
    }

    #[test]
    fn physical_render_vad_admission_retains_exact_observation_before_fallback_refusal() {
        use moiren_windows_audio::latency::shared_period::SharedFallbackReason;
        let (expected, active, mut native) = physical_admission_fixture();
        let request = render_request::PhysicalRenderRequest {
            period_frames: Some(128),
            ..Default::default()
        };
        let mut retained = None;
        retain_and_validate_physical_render(
            request,
            expected,
            &active,
            Some(&native),
            &mut retained,
        )
        .unwrap();
        native.streams[0]
            .shared_period
            .as_mut()
            .unwrap()
            .fallback_reason = Some(SharedFallbackReason::WindowsFormatConversion);
        assert!(
            retain_and_validate_physical_render(
                request,
                expected,
                &active,
                Some(&native),
                &mut retained
            )
            .is_err()
        );
        assert_eq!(
            retained.unwrap()["observations"]["streams"][0]["shared_period"]["fallback_reason"],
            "windows_format_conversion"
        );
    }

    fn assert_physical_render_admission_survives_live_and_cleanup(refused: bool) {
        use moiren_windows_audio::latency::shared_period::SharedFallbackReason;
        let (expected, active, mut native) = physical_admission_fixture();
        let request = render_request::PhysicalRenderRequest {
            period_frames: Some(128),
            ..Default::default()
        };
        if refused {
            native.streams[0]
                .shared_period
                .as_mut()
                .unwrap()
                .fallback_reason = Some(SharedFallbackReason::WindowsFormatConversion);
        }
        let mut evidence = None;
        let result = retain_and_validate_physical_render(
            request,
            expected,
            &active,
            Some(&native),
            &mut evidence,
        );
        assert_eq!(result.is_err(), refused);
        let before_gate = evidence.as_ref().unwrap().clone();
        // Model the actual current_health then observe_final replacements:
        // their observations must advance, but cannot relabel admission proof.
        for stage in ["live", "cleanup", "unavailable"] {
            let mut later = engine_snapshot_value(&active);
            later["owner_generation"] = 99.into();
            later["observations"] = serde_json::json!({
                "owner_generation": 99, "running": false, "streams": [],
            });
            if stage == "unavailable" {
                later["observations"] = Value::Null;
            }
            later["stage"] = stage.into();
            if stage == "cleanup" {
                later["startup_trace"] = serde_json::json!({"samples": []});
            }
            replace_engine_observation(&mut evidence, later.clone());
            let retained = evidence.as_ref().unwrap();
            assert_eq!(retained["observations"], later["observations"]);
            assert_eq!(retained["stage"], stage);
            assert_eq!(
                retained["physical_render_request"],
                before_gate["physical_render_request"]
            );
            let admission = &retained["physical_render_admission"];
            assert_eq!(admission["observations"], before_gate["observations"]);
            assert_eq!(admission["owner_generation"], active.owner_generation);
            assert_eq!(
                admission["physical_render_request"],
                before_gate["physical_render_request"]
            );
            assert_eq!(admission["expected_output_node"], format!("{expected:?}"));
            assert_eq!(admission["scope"], "pre_gate_physical_render_validation");
            assert_eq!(admission["validation_accepted"], !refused);
            if refused {
                assert!(admission["failure"].as_str().unwrap().contains("fallback"));
            } else {
                assert!(admission["failure"].is_null());
            }
            assert_eq!(admission, &before_gate["physical_render_admission"]);
            assert!(admission.get("physical_render_admission").is_none());
            if stage == "cleanup" {
                assert_eq!(retained["startup_trace"], later["startup_trace"]);
            }
        }
    }

    #[test]
    fn physical_render_admission_success_survives_live_and_cleanup_engine_replacement() {
        assert_physical_render_admission_survives_live_and_cleanup(false);
    }

    #[test]
    fn physical_render_admission_refusal_survives_live_and_cleanup_engine_replacement() {
        assert_physical_render_admission_survives_live_and_cleanup(true);
    }

    #[test]
    fn engine_replacement_without_physical_admission_keeps_existing_schema() {
        let mut evidence = Some(serde_json::json!({"observations": {"old": true}}));
        let current = serde_json::json!({"observations": {"current": true}});
        replace_engine_observation(&mut evidence, current.clone());
        assert_eq!(evidence, Some(current));
    }

    fn endpoint_view(intent: Intent, state: StreamState) -> VadExportObservation {
        endpoint_view_at(intent, state, true, 17, 19)
    }

    fn endpoint_view_at(
        intent: Intent,
        state: StreamState,
        gate_open: bool,
        stream_generation: u64,
        gate_epoch: u64,
    ) -> VadExportObservation {
        let id = VirtualEndpointId::from_bytes([intent as u8; 16]);
        let mut caps = [0u8; 64];
        for (offset, value) in [
            (0, 64u32),
            (4, 2),
            (8, 31),
            (12, 64),
            (16, 1),
            (20, 48000),
            (24, 3),
            (28, 6),
            (32, 6),
            (36, 4),
            (48, 4800),
        ] {
            caps[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        caps[40..48].copy_from_slice(&7u64.to_le_bytes());
        let mut bytes = [0u8; 128];
        for (offset, value) in [
            (0, 128u32),
            (4, 2),
            (16, PdoState::Present as u32),
            (20, intent as u32),
            (24, 2),
            (28, u32::from(gate_open)),
            (32, 1),
            (36, 1),
            (88, 2),
            (92, 48),
            (100, state as u32),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (8, 13u64),
            (40, 7),
            (48, 13),
            (56, stream_generation),
            (64, gate_epoch),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[72..88].copy_from_slice(&stable_from_virtual(id).to_le_bytes());
        VadExportObservation {
            endpoint_id: id,
            spec_name: "owned fixture".into(),
            intent,
            channels: Channels::Stereo,
            allow_default: false,
            snapshot: Some(
                EndpointSnapshot::decode(&bytes, Capabilities::decode(&caps).unwrap()).unwrap(),
            ),
            removing: false,
            transport_prepared: true,
            telemetry: Some(VadPcmTelemetrySnapshot::default()),
            error: None,
            diagnostic_provenance: VadExportDiagnosticProvenance::Fresh,
        }
    }

    #[test]
    fn recorder_start_waits_only_for_same_stamp_paused_observations() {
        let origin = Instant::now();
        let clock = Cell::new(origin);
        let observations = Cell::new(0);
        let elapsed = wait_initial_recorder(
            origin + Duration::from_millis(25),
            || clock.get(),
            || {
                observations.set(observations.get() + 1);
                let paused = endpoint_view(Intent::Microphone, StreamState::Paused);
                let baseline =
                    endpoint_health(Some(&paused), paused.endpoint_id, Intent::Microphone);
                let view = endpoint_view(
                    Intent::Microphone,
                    if observations.get() == 3 {
                        StreamState::Running
                    } else {
                        StreamState::Paused
                    },
                );
                Ok(recorder_start_running(&view, view.endpoint_id, &baseline)?
                    .then_some(clock.get().duration_since(origin)))
            },
            |duration| clock.set(clock.get() + duration),
        )
        .unwrap();
        assert_eq!(observations.get(), 3);
        assert_eq!(elapsed, Duration::from_millis(20));
    }

    #[test]
    fn recorder_start_wait_honors_deadline_even_if_last_query_returns_running() {
        let origin = Instant::now();
        let clock = Cell::new(origin);
        let result = wait_initial_recorder(
            origin + Duration::from_millis(5),
            || clock.get(),
            || {
                clock.set(origin + Duration::from_millis(5));
                Ok(Some(()))
            },
            |_| panic!("late observation must not sleep"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn recorder_start_wait_never_retries_epoch_or_provider_failure() {
        let origin = Instant::now();
        let mut attempts = 0;
        let result = wait_initial_recorder::<()>(
            origin + Duration::from_secs(1),
            || origin,
            || {
                attempts += 1;
                Err(anyhow::anyhow!("pinned epoch changed"))
            },
            |_| panic!("identity errors must not be retried"),
        );
        assert_eq!(attempts, 1);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("pinned epoch changed")
        );
    }

    #[test]
    fn recorder_start_rejects_open_gate_acquire_or_rebase_without_baseline_adoption() {
        let view = endpoint_view(Intent::Microphone, StreamState::Running);
        let baseline = endpoint_health(Some(&view), view.endpoint_id, Intent::Microphone);
        assert!(recorder_start_running(&view, view.endpoint_id, &baseline).unwrap());
        for slot in [3, 4] {
            let mut previous = baseline.clone();
            previous.stamp.as_mut().unwrap()[slot] -= 1;
            assert!(view.snapshot.unwrap().gate_open());
            assert!(recorder_start_running(&view, view.endpoint_id, &previous).is_err());
            assert_eq!(
                previous.stamp.unwrap()[slot] + 1,
                baseline.stamp.unwrap()[slot]
            );
        }
    }

    #[test]
    fn same_stamp_pause_in_newer_health_cannot_confirm_earlier_running() {
        let running = endpoint_view(Intent::Microphone, StreamState::Running);
        let paused = endpoint_view(Intent::Microphone, StreamState::Paused);
        let baseline = endpoint_health(Some(&paused), paused.endpoint_id, Intent::Microphone);
        assert!(recorder_start_running(&running, running.endpoint_id, &baseline).unwrap());
        let newer = endpoint_health(Some(&paused), paused.endpoint_id, Intent::Microphone);
        assert_eq!(baseline.stamp, newer.stamp);
        assert!(
            newer.provider_ready,
            "Paused is valid during initial preparation/polling"
        );
        assert!(
            !recorder_confirmation_running(&baseline, &newer).unwrap(),
            "a prior Running query cannot authorize a newer same-stamp Pause"
        );
        let latest = endpoint_health(Some(&running), running.endpoint_id, Intent::Microphone);
        assert!(recorder_confirmation_running(&baseline, &latest).unwrap());
    }

    #[test]
    fn endpoint_health_exports_stream_running_separately_from_prepared_readiness() {
        for intent in [Intent::Playback, Intent::Microphone] {
            for state in [StreamState::Paused, StreamState::Running] {
                let view = endpoint_view(intent, state);
                let health = endpoint_health(Some(&view), view.endpoint_id, intent);
                assert!(health.provider_ready);
                assert_eq!(
                    serde_json::to_value(health).unwrap()["stream_running"],
                    state == StreamState::Running
                );
            }
            assert_eq!(
                serde_json::to_value(endpoint_health(
                    None,
                    VirtualEndpointId::from_bytes([3; 16]),
                    intent
                ))
                .unwrap()["stream_running"],
                false
            );
        }
    }

    #[test]
    fn recorder_start_still_paused_stops_at_exact_deadline_without_more_queries() {
        let origin = Instant::now();
        let clock = Cell::new(origin);
        let observations = Cell::new(0);
        let result = wait_initial_recorder::<()>(
            origin + Duration::from_millis(15),
            || clock.get(),
            || {
                observations.set(observations.get() + 1);
                Ok(None)
            },
            |duration| clock.set(clock.get() + duration),
        );
        assert!(result.is_err());
        assert_eq!(observations.get(), 2);
        assert_eq!(clock.get(), origin + Duration::from_millis(15));
        assert!(
            wait_initial_recorder::<()>(
                origin,
                || origin,
                || panic!("expired deadline cannot query"),
                |_| panic!("expired deadline cannot wait")
            )
            .is_err()
        );
    }

    #[test]
    fn second_create_failure_retains_first_and_only_introduced_uncertain_exports() {
        let playback = endpoint_view(Intent::Playback, StreamState::Paused);
        let microphone = endpoint_view(Intent::Microphone, StreamState::Paused);
        let exports = [
            (playback.endpoint_id, Intent::Playback, "Playback".into()),
            (
                microphone.endpoint_id,
                Intent::Microphone,
                "Microphone".into(),
            ),
        ];
        for uncertain in [false, true] {
            let views = RefCell::new(Vec::<VadExportObservation>::new());
            let mut owned = Vec::new();
            let mut ready_ids = Vec::new();
            let error = prepare_owned_exports(
                &exports,
                &mut owned,
                |id| {
                    views
                        .borrow()
                        .iter()
                        .find(|view| view.endpoint_id == id)
                        .cloned()
                },
                |id, intent, _| {
                    if intent == Intent::Microphone {
                        if uncertain {
                            let mut unknown = microphone.clone();
                            unknown.snapshot = None;
                            views.borrow_mut().push(unknown);
                        }
                        bail!("second CREATE failed");
                    }
                    assert_eq!(id, playback.endpoint_id);
                    views.borrow_mut().push(playback.clone());
                    Ok(())
                },
                |id, _| {
                    ready_ids.push(id);
                    Ok(())
                },
            )
            .unwrap_err();
            assert!(format!("{error:#}").contains("second CREATE failed"));
            assert_eq!(ready_ids, [playback.endpoint_id]);
            assert_eq!(
                owned,
                if uncertain {
                    vec![playback.endpoint_id, microphone.endpoint_id]
                } else {
                    vec![playback.endpoint_id]
                }
            );
        }
    }

    #[test]
    fn second_preexisting_export_is_never_adopted_or_retired() {
        let playback = endpoint_view(Intent::Playback, StreamState::Paused);
        let microphone = endpoint_view(Intent::Microphone, StreamState::Paused);
        let views = RefCell::new(vec![microphone.clone()]);
        let mut owned = Vec::new();
        let result = prepare_owned_exports(
            &[
                (playback.endpoint_id, Intent::Playback, "Playback".into()),
                (
                    microphone.endpoint_id,
                    Intent::Microphone,
                    "Microphone".into(),
                ),
            ],
            &mut owned,
            |id| {
                views
                    .borrow()
                    .iter()
                    .find(|view| view.endpoint_id == id)
                    .cloned()
            },
            |id, _, _| {
                assert_eq!(id, playback.endpoint_id);
                views.borrow_mut().push(playback.clone());
                Ok(())
            },
            |_, _| Ok(()),
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("refusing to adopt")
        );
        assert_eq!(owned, [playback.endpoint_id]);
    }

    #[test]
    fn create_panic_after_publication_retains_uncertain_endpoint_for_cleanup() {
        let playback = endpoint_view(Intent::Playback, StreamState::Paused);
        let microphone = endpoint_view(Intent::Microphone, StreamState::Paused);
        let views = RefCell::new(vec![playback.clone()]);
        let mut owned = vec![playback.endpoint_id];
        let error = prepare_owned_exports(
            &[
                (playback.endpoint_id, Intent::Playback, "Playback".into()),
                (
                    microphone.endpoint_id,
                    Intent::Microphone,
                    "Microphone".into(),
                ),
            ],
            &mut owned,
            |id| {
                views
                    .borrow()
                    .iter()
                    .find(|view| view.endpoint_id == id)
                    .cloned()
            },
            |id, _, _| {
                assert_eq!(id, microphone.endpoint_id);
                let mut uncertain = microphone.clone();
                uncertain.snapshot = None;
                views.borrow_mut().push(uncertain);
                panic!("injected CREATE panic after uncertain publication");
            },
            |_, _| Ok(()),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("panicked"));
        assert_eq!(owned, [playback.endpoint_id, microphone.endpoint_id]);
        let mut retired = Vec::new();
        each_owned_export(&owned, |id| {
            retired.push(id);
            Ok(())
        })
        .unwrap();
        assert_eq!(retired, owned);
    }

    #[test]
    fn second_gate_failure_keeps_first_gate_and_single_pinned_ticket() {
        let playback = endpoint_view(Intent::Playback, StreamState::Paused);
        let microphone = endpoint_view(Intent::Microphone, StreamState::Paused);
        let mut attempted = Vec::new();
        let mut opened = Vec::new();
        let snapshots = [
            (playback.endpoint_id, playback.snapshot.unwrap()),
            (microphone.endpoint_id, microphone.snapshot.unwrap()),
        ];
        let error = open_prepared_gates(&snapshots, 7, 23, |id, stamp, revision, ticket| {
            attempted.push((id, stamp, revision, ticket));
            if id == microphone.endpoint_id {
                bail!("second gate failed");
            }
            opened.push(id);
            Ok(playback.snapshot.unwrap())
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("second gate failed"));
        assert_eq!(opened, [playback.endpoint_id]);
        assert_eq!(
            attempted,
            snapshots.map(|(id, snapshot)| (id, snapshot_stamp(snapshot), 7, 23))
        );
    }

    #[test]
    fn physical_and_microphone_gate_admission_keep_exact_epoch_on_open() {
        for intents in [
            &[Intent::Playback][..],
            &[Intent::Playback, Intent::Microphone][..],
        ] {
            for already_open in [false, true] {
                // A preceding driver CloseGate advances 19 -> 20. Opening the
                // resulting closed gate keeps 20; opening an open gate is also
                // epoch-idempotent (client.rs set_gate validates exact equality).
                let snapshots: Vec<_> = intents
                    .iter()
                    .map(|intent| {
                        let view =
                            endpoint_view_at(*intent, StreamState::Paused, already_open, 17, 20);
                        (view.endpoint_id, view.snapshot.unwrap())
                    })
                    .collect();
                let admitted =
                    open_prepared_gates(&snapshots, 7, 23, |id, stamp, revision, ticket| {
                        assert_eq!((revision, ticket), (7, 23));
                        let (_, initial) = snapshots
                            .iter()
                            .find(|(current, _)| *current == id)
                            .unwrap();
                        let opened =
                            endpoint_view_at(initial.intent(), StreamState::Paused, true, 17, 20)
                                .snapshot
                                .unwrap();
                        assert_eq!(stamp, snapshot_stamp(opened));
                        Ok(opened)
                    })
                    .unwrap();
                assert_eq!(admitted.len(), intents.len());
                for ((id, initial), (admitted_id, current)) in snapshots.iter().zip(admitted) {
                    assert_eq!(*id, admitted_id);
                    assert_eq!(snapshot_stamp(*initial), snapshot_stamp(current));
                    assert!(current.gate_open());
                }
            }
        }
    }

    #[test]
    fn gate_admission_refuses_fabricated_open_epoch_increment() {
        for intent in [Intent::Playback, Intent::Microphone] {
            let initial = endpoint_view_at(intent, StreamState::Paused, false, 17, 19);
            let incremented = endpoint_view_at(intent, StreamState::Paused, true, 17, 20);
            assert!(
                open_prepared_gates(
                    &[(initial.endpoint_id, initial.snapshot.unwrap())],
                    7,
                    23,
                    |_, _, _, _| Ok(incremented.snapshot.unwrap())
                )
                .is_err()
            );
        }
    }

    #[test]
    fn sibling_export_cleanup_survives_panic_and_retains_both_failures() {
        let ids = [
            VirtualEndpointId::from_bytes([1; 16]),
            VirtualEndpointId::from_bytes([2; 16]),
        ];
        let mut attempted = Vec::new();
        let error = each_owned_export(&ids, |id| {
            attempted.push(id);
            if id == ids[0] {
                panic!("injected first endpoint cleanup panic");
            }
            Err(anyhow::anyhow!("injected second endpoint cleanup failure"))
        })
        .unwrap_err();
        assert_eq!(attempted, ids);
        assert!(error.to_string().contains("panicked"));
        assert!(
            error
                .to_string()
                .contains("second endpoint cleanup failure")
        );
    }

    #[test]
    fn endpoint_health_preserves_unknown_and_requires_exact_current_export() {
        for intent in [Intent::Playback, Intent::Microphone] {
            let view = endpoint_view(intent, StreamState::Running);
            let id = view.endpoint_id;
            let healthy = endpoint_health(Some(&view), id, intent);
            assert!(healthy.provider_ready && healthy.quality_known);
            assert_eq!(healthy.quality_faults, 0);
            let unknown = endpoint_health(None, id, intent);
            assert!(!unknown.provider_ready && !unknown.quality_known);
            assert!(unknown.stamp.is_none() && unknown.shape.is_none());
            for case in 0..5 {
                let mut invalid = view.clone();
                match case {
                    0 => invalid.endpoint_id = VirtualEndpointId::from_bytes([3; 16]),
                    1 => {
                        invalid.intent = if intent == Intent::Playback {
                            Intent::Microphone
                        } else {
                            Intent::Playback
                        }
                    }
                    2 => invalid.removing = true,
                    3 => invalid.snapshot = None,
                    _ => invalid.telemetry = None,
                }
                let health = endpoint_health(Some(&invalid), id, intent);
                if case == 4 {
                    assert!(!health.quality_known);
                } else {
                    assert!(!health.provider_ready, "invalid endpoint case {case}");
                }
            }
            let mut failed = view;
            failed.error = Some("query failed".into());
            assert_eq!(
                endpoint_health(Some(&failed), id, intent)
                    .provider_error
                    .as_deref(),
                Some("query failed")
            );
        }
    }

    #[test]
    fn endpoint_health_counts_each_provider_fault_with_saturation() {
        for intent in [Intent::Playback, Intent::Microphone] {
            let mut view = endpoint_view(intent, StreamState::Running);
            view.telemetry = Some(VadPcmTelemetrySnapshot {
                dropped_packets: 1,
                driver_dropped_packets: 2,
                quota_dropped_packets: 3,
                carrier_dropped_packets: 4,
                stale_packets: 5,
                epoch_resets: 6,
                recovery_events: 7,
                completion_clock_regressions: 8,
                missing_sequence_slots: 9,
                ..Default::default()
            });
            assert_eq!(
                endpoint_health(Some(&view), view.endpoint_id, intent).quality_faults,
                45
            );
            view.telemetry.as_mut().unwrap().dropped_packets = u64::MAX;
            assert_eq!(
                endpoint_health(Some(&view), view.endpoint_id, intent).quality_faults,
                u64::MAX
            );
        }
    }

    #[test]
    fn initial_recorder_start_keeps_exact_prepared_stream_and_gate_epochs() {
        let shape = ("Stereo".into(), "Some(Float32)".into(), 48);
        let before = [7, 11, 13, 17, 19];
        validate_initial_recorder_transition(before, &shape, before, &shape).unwrap();
        for current in [
            [7, 11, 13, 17, 20],
            [7, 11, 13, 18, 19],
            [7, 11, 13, 18, 21],
            [8, 11, 13, 17, 19],
            [7, 12, 13, 17, 19],
            [7, 11, 14, 17, 19],
            [7, 11, 13, 16, 19],
            [7, 11, 13, 19, 21],
            [7, 11, 13, 17, 18],
        ] {
            assert!(validate_initial_recorder_transition(before, &shape, current, &shape).is_err());
        }
        assert!(
            validate_initial_recorder_transition(
                before,
                &shape,
                before,
                &("Stereo".into(), "Some(Float32)".into(), 64),
            )
            .is_err()
        );
        assert!(
            validate_initial_recorder_transition(
                [7, 11, 13, u64::MAX, 19],
                &shape,
                [7, 11, 13, 0, 20],
                &shape,
            )
            .is_err()
        );
    }

    fn parked_render() -> PreparedNativeRender {
        let timing = moiren_windows_audio::latency::NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(48),
            capacity_frames: 512,
        };
        let queue = moiren_windows_audio::latency::prepare_render_queue(
            timing,
            RenderQueuePolicy::FullCapacity,
        )
        .unwrap();
        PreparedNativeRender::parked_fixture(
            super::super::render::NativeRenderInfo { timing, queue },
            StopSignal::new().unwrap(),
            RenderQueuePolicy::FullCapacity,
        )
    }

    fn bridge_target(target_frames: u32) -> ProcessBridgeOptions {
        ProcessBridgeOptions {
            input: TransportInputPolicy::Target {
                capacity_frames: 2048,
                target_frames,
                jitter_frames: 0,
            },
            delivery: Some(InputDeliveryTiming {
                sample_rate: 48_000,
                max_packet_frames: 32,
                max_gap_frames: 32,
                source: DeliveryTimingSource::Declared,
            }),
            max_demand_frames: 64,
            ..Default::default()
        }
    }

    #[test]
    fn ordinary_process_refusal_never_prepares_capture_and_joins_parked_render() {
        let mut render = Some(parked_render());
        let mut report = NativeReport::default();
        let mut called = false;
        let result =
            prepare_process_input(bridge_target(128), &mut render, &mut report, |_, _, _| {
                called = true;
                Ok(())
            });
        assert!(result.is_err());
        assert!(!called);
        assert!(render.is_none());
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["render"]["stream_started"], false);
        assert_eq!(value["render"]["queue"]["target_frames"], 512);
        assert_eq!(value["process_admission"]["failure"]["stage"], "admission");
        assert!(value["process_admission"]["admitted_complete_native_wake_fixed_48khz"].is_null());
    }

    #[test]
    fn ordinary_process_capture_failure_keeps_admission_partial_capture_and_render_reports() {
        let mut render = Some(parked_render());
        let mut report = NativeReport::default();
        let result = prepare_process_input::<()>(
            bridge_target(546),
            &mut render,
            &mut report,
            |admitted, _, report| {
                assert_eq!(admitted.max_demand_frames, 512);
                report.capture = Some(serde_json::json!({"partial":true}));
                Err(anyhow::anyhow!("injected capture preparation refusal"))
            },
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("injected capture"));
        assert!(render.is_none());
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["capture"]["partial"], true);
        assert_eq!(value["render"]["stream_started"], false);
        assert_eq!(value["process_admission"]["failure"]["stage"], "capture");
        assert_eq!(
            value["process_admission"]["admitted_complete_native_wake_fixed_48khz"]["max_demand_frames"],
            512
        );
    }

    #[test]
    fn ordinary_process_success_leaves_render_parked_for_real_attachment() {
        let mut render = Some(parked_render());
        let mut report = NativeReport::default();
        let (admitted, capture) = prepare_process_input(
            bridge_target(546),
            &mut render,
            &mut report,
            |admitted, render, _| {
                assert!(render.is_some());
                assert_eq!(admitted.max_demand_frames, 512);
                Ok("injected capture")
            },
        )
        .unwrap();
        assert_eq!(capture, "injected capture");
        assert_eq!(admitted.max_demand_frames, 512);
        assert!(report.render.is_none());
        assert!(report.process_admission.unwrap().failure.is_none());
        let (report, result) = render.take().unwrap().cancel_retaining_report();
        result.unwrap();
        assert!(report.is_some());
    }

    #[test]
    fn ordinary_capture_join_retains_panic_report_even_if_artifact_persistence_fails() {
        use moiren_windows_audio::capture::{CaptureError, CaptureReport, CaptureSource};
        let mut report = CaptureReport::new(
            CaptureSource::ProcessLoopback,
            SessionDuration::UntilStopped,
        );
        report.failure = Some("partial report from post-ready panic".into());
        let mut native = NativeReport::default();
        let mut persisted = false;
        let error = retain_capture_join(
            &mut native,
            (Some(report), Some(CaptureError::WorkerPanicked)),
            |report| {
                persisted = true;
                assert_eq!(
                    report.failure.as_deref(),
                    Some("partial report from post-ready panic")
                );
                Err(anyhow::anyhow!("injected PCM disk failure"))
            },
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("panicked"));
        assert!(message.contains("injected PCM disk failure"));
        assert!(persisted);
        assert_eq!(
            native.capture.unwrap()["failure"],
            "partial report from post-ready panic"
        );
        let failure = native.capture_failure.unwrap();
        assert_eq!(failure["stage"], "capture_join");
        assert_eq!(failure["partial_report_retained"], true);
    }

    #[test]
    fn ordinary_capture_join_without_report_refuses_and_never_persists() {
        use moiren_windows_audio::capture::CaptureError;
        let mut native = NativeReport::default();
        let error = retain_capture_join(
            &mut native,
            (None, Some(CaptureError::WorkerPanicked)),
            |_| panic!("missing report cannot supply PCM"),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("capture report missing"));
        assert!(native.capture.is_none());
        assert_eq!(
            native.capture_failure.unwrap()["partial_report_retained"],
            false
        );
    }

    #[test]
    fn startup_timeline_keeps_epoch_revision_and_frame_or_explicitly_unknown() {
        let timeline = engine_timeline_value(Some(moiren_engine::host::OwnerTimeline {
            epoch: 3,
            revision: 7,
            frame: 480,
        }));
        assert_eq!(
            timeline,
            serde_json::json!({"epoch": 3, "revision": 7, "frame": 480})
        );
        assert!(engine_timeline_value(None).is_null());
    }

    #[test]
    fn minimal_vad_project_declares_physical_output_master() {
        let project = minimal_vad_project(
            VirtualEndpointId::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]),
            EndpointSelector::PinnedEndpoint {
                endpoint_id: "render".into(),
                stable_id: None,
            },
        )
        .unwrap();
        let output = project
            .nodes
            .values()
            .find_map(|node| match node {
                NodeConfig::Output(output) => Some(output),
                _ => None,
            })
            .expect("minimal VAD project output");
        assert_eq!(output.device.clock_role, ClockRole::Master);
    }
}
