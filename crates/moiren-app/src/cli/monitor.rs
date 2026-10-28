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
#[cfg(windows)]
use crate::service::ManagedDriver;
use crate::service::{AudioSettings, TransportLatencySettings};
#[cfg(any(windows, test))]
use crate::service::{
    BuiltinState, GraphNodeState, InputSelection, RunLimit, ServiceContext, ServiceDriver,
    ServiceRuntime, SessionPhase, SessionSpec, default_label,
};
#[cfg(any(windows, test))]
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams, edit::connect_to_new_bus_input};

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
    pub requested_seconds: u16,
    pub observed_running_ms: u64,
    pub run_complete: bool,
    pub cleanup_complete: bool,
    /// These are snapshots taken while Running, not a final joined-owner DTO.
    #[cfg(windows)]
    pub last_running_native:
        Option<moiren_windows_audio::engine_backend::NativeObservationSnapshot>,
    pub failure: Option<String>,
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
    let InputSelection::Process {
        pid,
        creation_time_100ns,
    } = spec.source
    else {
        unreachable!("validated Process context")
    };
    let mut report = MonitorReport {
        schema_version: 1,
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
        requested_seconds: options.seconds,
        observed_running_ms: 0,
        run_complete: false,
        cleanup_complete: false,
        #[cfg(windows)]
        last_running_native: None,
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
        if let Some(snapshot) = handle.try_snapshot() {
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
                    report.observed_running_ms =
                        u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
                    if snapshot.phase != SessionPhase::Running
                        || Some(snapshot.generation.0) != report.confirmed_generation
                        || snapshot.graph.active_revision != report.confirmed_revision
                    {
                        report.failure =
                            Some("confirmed monitor owner/revision stopped or changed".into());
                        exiting = true;
                    } else if Instant::now().duration_since(since) >= duration {
                        report.run_complete = true;
                        exiting = true;
                    }
                } else if snapshot.phase == SessionPhase::Running
                    && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                    && snapshot.active_audio_settings.as_ref() == Some(&report.requested_audio)
                    && snapshot.active_transport_latency == Some(report.requested_transport)
                {
                    report.confirmed_active_audio = snapshot.active_audio_settings.clone();
                    report.confirmed_active_transport = snapshot.active_transport_latency;
                    report.confirmed_generation = Some(snapshot.generation.0);
                    report.confirmed_revision = snapshot.graph.active_revision;
                    running_since = Some(Instant::now());
                } else if began.elapsed() >= Duration::from_millis(options.startup_timeout_ms) {
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
        if exiting {
            handle.request_exit();
        }
        // Snapshot availability is not permission to renew the start deadline.
        if !exiting
            && running_since.is_none()
            && began.elapsed() >= Duration::from_millis(options.startup_timeout_ms)
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
