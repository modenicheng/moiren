//! A finite CLI session uses the same prepared owner and cleanup as desktop.
use anyhow::Result;
#[cfg(any(windows, test))]
use anyhow::{Context, ensure};
use serde::Serialize;
#[cfg(any(windows, test))]
use std::{
    thread,
    time::{Duration, Instant},
};

use super::options::MonitorOptions;
#[cfg(any(windows, test))]
mod measured;
#[cfg(windows)]
use crate::service::ManagedDriver;
#[cfg(any(windows, test))]
use crate::service::{
    AppSnapshot, BuiltinState, GraphNodeState, InputSelection, RunLimit, ServiceContext,
    ServiceDriver, ServiceRuntime, SessionPhase, SessionSpec, default_label,
};
use crate::service::{AudioSettings, TransportLatencySettings};
#[cfg(any(windows, test))]
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams, edit::connect_to_new_bus_input};

#[derive(Debug, Serialize)]
pub(super) struct MeasuredTarget {
    capacity_frames: u32,
    target_frames: u32,
    jitter_frames: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct MonitorReport {
    pub schema_version: u32,
    pub qualified: bool,
    pub route: &'static str,
    pub process_pid: u32,
    pub process_creation_time_100ns: u64,
    pub output_endpoint_id: String,
    pub requested_audio: AudioSettings,
    pub requested_transport: TransportLatencySettings,
    pub confirmed_active_audio: Option<AudioSettings>,
    pub confirmed_active_transport: Option<TransportLatencySettings>,
    pub confirmed_generation: Option<u64>,
    pub confirmed_revision: Option<u64>,
    pub confirmed_native_generation: Option<u64>,
    pub requested_measurement_seconds: Option<u16>,
    pub requested_measured_target: Option<MeasuredTarget>,
    pub requested_window_policy: Option<crate::service::ProcessWindowPolicy>,
    pub measurement_generation: Option<u64>,
    pub measurement_native_generation: Option<u64>,
    pub observed_measurement_ms: u64,
    pub confirmed_active_measured: Option<crate::service::MeasuredProcessSummary>,
    pub requested_seconds: u16,
    pub observed_running_ms: u64,
    pub run_complete: bool,
    pub cleanup_complete: bool,
    /// These are snapshots taken while Running, not a final joined-owner DTO.
    #[cfg(windows)]
    pub last_running_native:
        Option<moiren_windows_audio::engine_backend::NativeObservationSnapshot>,
    /// Separate post-join evidence; null when no retained counters are available.
    pub final_after_owner_join: Option<serde_json::Value>,
    pub failure: Option<String>,
}

#[cfg(windows)]
impl MonitorReport {
    fn capture_joined_tally(&mut self, cleanup: &crate::service::SessionCleanupObservation) {
        use moiren_engine::host::HostState;
        self.final_after_owner_join = None;
        let Some(joined) = &cleanup.final_observations else {
            return;
        };
        let state = &cleanup.state;
        let owner = state.owner_generation;
        let valid = state.host == HostState::Closed
            && self
                .confirmed_generation
                .is_none_or(|value| value == cleanup.generation.0)
            && self
                .confirmed_revision
                .is_none_or(|value| value == joined.graph_revision)
            && self
                .confirmed_native_generation
                .is_none_or(|value| value == owner)
            && state.desired_revision == joined.host_revision
            && joined
                .native
                .as_ref()
                .is_none_or(|value| !value.running && value.owner_generation == owner)
            && joined
                .backend
                .as_ref()
                .is_none_or(|value| !value.running && value.owner_generation == owner);
        if !valid {
            self.cleanup_complete = false;
            self.failure.get_or_insert_with(|| {
                "monitor final observation identity or closure mismatch".into()
            });
            return;
        }
        if joined.native.is_none() && joined.backend.is_none() {
            return;
        }
        let backend = joined.backend.as_ref().map(|value| {
            serde_json::json!({
                "owner_generation": value.owner_generation,
                "running": value.running,
                "streams": value.streams.iter().map(|stream| serde_json::json!({
                    "node": format!("{:?}", stream.node),
                    "input": stream.input,
                    "master": stream.master,
                    "requested": {
                        "backend": stream.requested.backend,
                        "mode": stream.requested.mode,
                        "hardware_sample_rate": stream.requested.hardware_sample_rate,
                        "sample_format": stream.requested.sample_format,
                        "period_frames": stream.requested.period_frames,
                    },
                    "actual": {
                        "sample_rate": stream.actual.sample_rate,
                        "format": stream.actual.format,
                        "channels": stream.actual.channels,
                        "buffer_frames": stream.actual.buffer_frames,
                    },
                    "dropped_frames": stream.dropped_frames,
                    "silent_frames": stream.silent_frames,
                    "native_callbacks": stream.native_callbacks,
                    "native_wait_wakes": stream.native_wait_wakes,
                    "native_wait_timeouts": stream.native_wait_timeouts,
                    "native_written_frames": stream.native_written_frames,
                    "native_dropped_frames": stream.native_dropped_frames,
                    "native_silent_frames": stream.native_silent_frames,
                    "native_capture_quality": stream.native_capture_quality,
                    "driver_latency": stream.driver_latency.map(|latency| serde_json::json!({
                        "input_frames": latency.input_frames,
                        "output_frames": latency.output_frames,
                    })),
                })).collect::<Vec<_>>(),
            })
        });
        self.final_after_owner_join = Some(serde_json::json!({
            "observation_scope": "final_after_owner_join",
            "app_generation": cleanup.generation.0,
            "graph_revision": joined.graph_revision,
            "host_revision": joined.host_revision,
            "native_owner_generation": owner,
            "resource_generation": state.resource_generation,
            "cleanup_error": state.cleanup_error.as_ref().map(ToString::to_string),
            "fault": state.fault.as_ref().map(|value| format!("{value:?}")),
            "native": joined.native,
            "backend": backend,
        }));
    }
}

#[cfg(any(windows, test))]
fn live_native_generation(snapshot: &AppSnapshot) -> Option<u64> {
    #[cfg(windows)]
    {
        snapshot
            .native_observations
            .as_ref()
            .filter(|native| {
                native.generation == snapshot.generation
                    && native.observation.running
                    && native.observation.owner_generation != 0
            })
            .map(|native| native.observation.owner_generation)
    }
    #[cfg(not(windows))]
    {
        let _ = snapshot;
        None
    }
}

#[cfg(any(windows, test))]
fn window_policy_matches(
    summary: &crate::service::MeasuredProcessSummary,
    requested: Option<crate::service::ProcessWindowPolicy>,
) -> bool {
    summary.observed_window.as_ref().map(|window| window.policy) == requested
}

#[cfg(any(windows, test))]
fn confirmed_run_matches(snapshot: &AppSnapshot, report: &MonitorReport) -> bool {
    let owner_matches = snapshot.phase == SessionPhase::Running
        && Some(snapshot.generation.0) == report.confirmed_generation
        && snapshot.graph.active_revision == report.confirmed_revision
        && snapshot.active_audio_settings == report.confirmed_active_audio
        && snapshot.active_transport_latency == report.confirmed_active_transport;
    owner_matches
        && (report.requested_measurement_seconds.is_none()
            || (report.confirmed_active_measured.is_some()
                && snapshot.active_measured_process == report.confirmed_active_measured
                && snapshot
                    .active_measured_process
                    .as_ref()
                    .is_some_and(|summary| {
                        window_policy_matches(summary, report.requested_window_policy)
                    })
                && report.confirmed_native_generation.is_some()
                && live_native_generation(snapshot) == report.confirmed_native_generation))
}

#[cfg(any(windows, test))]
fn check_confirmed_run(
    report: &mut MonitorReport,
    snapshot: Option<&AppSnapshot>,
    since: Instant,
    duration: Duration,
) -> bool {
    let elapsed = since.elapsed();
    report.observed_running_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    if let Some(snapshot) = snapshot {
        if !confirmed_run_matches(snapshot, report) {
            report.failure =
                Some("confirmed monitor owner/revision/profile stopped or changed".into());
            return true;
        }
        if elapsed >= duration {
            report.run_complete = true;
            return true;
        }
    } else if elapsed >= duration {
        // Request cleanup on time, but never promote absent owner evidence to
        // a completed run. This is separate from potentially blocking joins.
        report.failure = Some("monitor run deadline elapsed; owner observation unavailable".into());
        return true;
    }
    false
}

#[cfg(any(windows, test))]
fn context(
    options: &MonitorOptions,
    source: InputSelection,
) -> Result<(ServiceContext, SessionSpec)> {
    options.validate()?;
    let InputSelection::Process {
        pid,
        creation_time_100ns,
    } = source
    else {
        anyhow::bail!("monitor requires a stable Process identity");
    };
    ensure!(
        Some(pid) == options.pid && creation_time_100ns != 0,
        "resolved process identity does not match requested PID"
    );
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2)?;
    let bus = graph.create_node(NodeKind::Bus, 2)?;
    let output = graph.create_node(NodeKind::Sink, 2)?;
    let source_port = graph.get_node(input)?.outputs()[0].id();
    let bus_port = graph.get_node(bus)?.outputs()[0].id();
    let sink_port = graph.get_node(output)?.inputs()[0].id();
    connect_to_new_bus_input(&mut graph, source_port, bus, SendParams::default())?;
    graph.connect(bus_port, sink_port, SendParams::default())?;
    let output_id = options.output.as_ref().context("missing output")?;
    let source = InputSelection::Process {
        pid,
        creation_time_100ns,
    };
    let nodes = graph
        .nodes()
        .iter()
        .map(|node| GraphNodeState {
            id: node.id(),
            label: default_label(node.kind()).into(),
            gain: 1.0,
            pan: 0.0,
            mono: false,
            swap: false,
            mixer: Default::default(),
            compressor: Default::default(),
            builtin: BuiltinState::for_kind(node.kind()),
            source: (node.id() == input).then(|| source.clone()),
            output_endpoint_id: (node.id() == output).then(|| output_id.clone()),
        })
        .collect();
    let mut context = ServiceContext::default();
    context.audio_settings = options.audio.clone();
    context.transport_latency = options.transport()?;
    context
        .graph
        .replace(graph, nodes)
        .map_err(anyhow::Error::msg)?;
    let spec = SessionSpec {
        source,
        output_endpoint_id: output_id.clone(),
        // Supervise from confirmed Running, not an earlier prepare timestamp.
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: options.audio.processing.block_frames as usize,
    };
    context
        .graph
        .preparation()
        .configured_engine_project(
            spec.max_block_frames,
            &options.audio,
            context.latency_profile,
        )
        .map_err(anyhow::Error::msg)?;
    Ok((context, spec))
}

pub(super) fn run(options: MonitorOptions) -> Result<MonitorReport> {
    #[cfg(windows)]
    {
        // Stable identity and unsupported-budget refusal precede service/native
        // preparation. Native capture rechecks creation time before admission.
        options.validate()?;
        let process = moiren_windows_audio::process_loopback::inspect_process(
            options.pid.context("missing process PID")?,
        )
        .context("resolve Process monitor identity")?;
        let source = InputSelection::Process {
            pid: process.pid,
            creation_time_100ns: process.creation_time_100ns,
        };
        let duration = Duration::from_secs(u64::from(options.seconds));
        run_with_driver(options, source, ManagedDriver::windows(), duration)
    }
    #[cfg(not(windows))]
    {
        let _ = options;
        anyhow::bail!("Process monitor is only supported on Windows")
    }
}

#[cfg(any(windows, test))]
fn run_with_driver(
    options: MonitorOptions,
    source: InputSelection,
    driver: impl ServiceDriver,
    duration: Duration,
) -> Result<MonitorReport> {
    let (context, spec) = context(&options, source)?;
    let mut measurement = measured::Measurement::new(&options);
    let requested_measured_target =
        options
            .measured_target()?
            .map(
                |(capacity_frames, target_frames, jitter_frames)| MeasuredTarget {
                    capacity_frames,
                    target_frames,
                    jitter_frames,
                },
            );
    let requested_window_policy = options.window_policy()?;
    let InputSelection::Process {
        pid,
        creation_time_100ns,
    } = spec.source
    else {
        unreachable!("validated Process context")
    };
    let mut report = MonitorReport {
        schema_version: 2,
        qualified: false,
        route: "Process Loopback -> Bus -> physical render",
        process_pid: pid,
        process_creation_time_100ns: creation_time_100ns,
        output_endpoint_id: spec.output_endpoint_id.clone(),
        requested_audio: options.audio,
        requested_transport: context.transport_latency,
        confirmed_active_audio: None,
        confirmed_active_transport: None,
        confirmed_generation: None,
        confirmed_revision: None,
        confirmed_native_generation: None,
        requested_measurement_seconds: options.measure_seconds,
        requested_measured_target,
        requested_window_policy,
        measurement_generation: None,
        measurement_native_generation: None,
        observed_measurement_ms: 0,
        confirmed_active_measured: None,
        requested_seconds: options.seconds,
        observed_running_ms: 0,
        run_complete: false,
        cleanup_complete: false,
        #[cfg(windows)]
        last_running_native: None,
        final_after_owner_join: None,
        failure: None,
    };
    let runtime = ServiceRuntime::spawn_with_context(driver, context)?;
    let handle = runtime.handle();
    let began = Instant::now();
    let mut running_since: Option<Instant> = None;
    let mut exiting = false;
    let mut cleanup_failed = false;
    if let Err(error) = handle.try_request(crate::service::AppRequest::Start(spec)) {
        report.failure = Some(format!("monitor Start request refused: {error:?}"));
        exiting = true;
        handle.request_exit();
    }
    loop {
        let mut received_snapshot = false;
        if let Some(snapshot) = handle.try_snapshot() {
            received_snapshot = true;
            if let Some(cleanup) = &snapshot.last_cleanup {
                let state = &cleanup.state;
                if state.host != moiren_engine::host::HostState::Closed
                    || state.cleanup_error.is_some()
                {
                    cleanup_failed = true;
                    report.failure.get_or_insert_with(|| format!(
                        "monitor owner cleanup failed: host={:?}, cleanup_error={:?}, fault={:?}",
                        state.host, state.cleanup_error, state.fault,
                    ));
                    exiting = true;
                }
                if let Some(fault) = &state.fault {
                    report
                        .failure
                        .get_or_insert_with(|| format!("monitor final owner fault: {fault:?}"));
                    exiting = true;
                }
            }
            if let Some(error) = snapshot
                .error
                .as_ref()
                .or(snapshot.graph.last_error.as_ref())
            {
                report.failure.get_or_insert_with(|| error.clone());
                exiting = true;
            }
            if !exiting {
                if let Some(since) = running_since {
                    exiting = check_confirmed_run(&mut report, Some(&snapshot), since, duration);
                } else if let Err(error) = measurement.observe(
                    &snapshot,
                    &handle,
                    &mut report,
                    began,
                    options.startup_timeout_ms,
                ) {
                    report.failure = Some(error.to_string());
                    exiting = true;
                } else if measurement.ready()
                    && snapshot.phase == SessionPhase::Running
                    && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                    && snapshot.active_audio_settings.as_ref() == Some(&report.requested_audio)
                    && snapshot.active_transport_latency == Some(report.requested_transport)
                {
                    report.confirmed_active_audio = snapshot.active_audio_settings.clone();
                    report.confirmed_active_transport = snapshot.active_transport_latency;
                    report.confirmed_generation = Some(snapshot.generation.0);
                    report.confirmed_revision = snapshot.graph.active_revision;
                    report.confirmed_native_generation = live_native_generation(&snapshot);
                    report.confirmed_active_measured = snapshot.active_measured_process.clone();
                    running_since = Some(Instant::now());
                } else if measurement.startup_expired(began, options.startup_timeout_ms) {
                    report.failure = Some(
                        "monitor startup deadline elapsed before matching active confirmation"
                            .into(),
                    );
                    exiting = true;
                }
            }
            #[cfg(windows)]
            if snapshot.phase == SessionPhase::Running
                && Some(snapshot.generation.0) == report.confirmed_generation
            {
                report.last_running_native = snapshot
                    .native_observations
                    .as_ref()
                    .map(|observed| observed.observation.clone());
            }
            if snapshot.phase == SessionPhase::Exited {
                report.cleanup_complete = !cleanup_failed;
                break;
            }
        }
        if !exiting
            && !received_snapshot
            && let Some(since) = running_since
        {
            exiting = check_confirmed_run(&mut report, None, since, duration);
        }
        if exiting {
            handle.request_exit();
        }
        // Snapshot availability is not permission to renew the start deadline.
        if !exiting
            && running_since.is_none()
            && measurement.startup_expired(began, options.startup_timeout_ms)
        {
            report.failure =
                Some("monitor startup deadline elapsed before matching active confirmation".into());
            exiting = true;
            handle.request_exit();
        }
        if runtime.is_finished() {
            // A terminal thread without Exited is not a successful cleanup.
            if handle
                .try_snapshot()
                .is_some_and(|snapshot| snapshot.phase == SessionPhase::Exited)
            {
                report.cleanup_complete = !cleanup_failed;
            } else {
                report.failure.get_or_insert_with(|| {
                    "monitor service terminated before cleanup confirmation".into()
                });
            }
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    handle.request_exit();
    if let Err(error) = runtime.join() {
        report.cleanup_complete = false;
        report
            .failure
            .get_or_insert_with(|| format!("monitor service join failed: {error:?}"));
    }
    // Check the terminal snapshot again after join: the service can publish its
    // final cleanup between the last poll and the thread-finished observation.
    if let Some(snapshot) = handle.try_snapshot() {
        if let Some(cleanup) = &snapshot.last_cleanup {
            #[cfg(windows)]
            report.capture_joined_tally(cleanup);
            let state = &cleanup.state;
            if state.host != moiren_engine::host::HostState::Closed || state.cleanup_error.is_some()
            {
                report.cleanup_complete = false;
                report.failure.get_or_insert_with(|| {
                    format!("monitor owner cleanup failed: {:?}", state.cleanup_error)
                });
            }
            if let Some(fault) = &state.fault {
                report
                    .failure
                    .get_or_insert_with(|| format!("monitor final owner fault: {fault:?}"));
            }
            if report
                .confirmed_generation
                .is_some_and(|generation| generation != cleanup.generation.0)
            {
                report.cleanup_complete = false;
                report
                    .failure
                    .get_or_insert_with(|| "monitor final cleanup generation mismatch".into());
            }
        } else if report.confirmed_generation.is_some() {
            report.cleanup_complete = false;
            report
                .failure
                .get_or_insert_with(|| "monitor final owner cleanup evidence missing".into());
        }
        if let Some(error) = snapshot
            .error
            .as_ref()
            .or(snapshot.graph.last_error.as_ref())
        {
            report.failure.get_or_insert_with(|| error.clone());
        }
    } else {
        report.cleanup_complete = false;
        report
            .failure
            .get_or_insert_with(|| "monitor terminal snapshot unavailable".into());
    }
    if !report.run_complete {
        report
            .failure
            .get_or_insert_with(|| "monitor exited before completing its confirmed run".into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
