//! Control-side orchestration. No device objects or polling enter Engine RT.
use super::{
    MonitorConfig, MonitorError, MonitorLatencyConfig, MonitorLatencyProfile, MonitorObservation,
    MonitorPreparedDiagnostics, prepare_monitor,
};
use moiren_core::graph::NodeId;
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use moiren_windows_audio::{
    ActivationGate, SessionDuration, StopSignal,
    capture::{
        CaptureOptions, CaptureReport, CaptureSession, CaptureStatus, PreparedCapture,
        prepare_capture_native_with_gate,
    },
    clock_bridge::{BridgeObserver, BridgeSnapshot},
    process_loopback::{
        ProcessBridgeOptions, ProcessIdentity, ProcessLoopbackOptions,
        prepare_process_capture_with_gate,
    },
    render::{
        DemandRenderer, PreparedNativeRender, RenderFadeControl, RenderOptions, RenderReport,
        RenderSession, RenderStatus, TimelineObserver, prepare_render_native_with_gate,
        start_render_prepared_with_gate,
    },
};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MonitorOptions {
    pub input_endpoint_id: String,
    pub output_endpoint_id: String,
    pub duration: SessionDuration,
    pub config: MonitorConfig,
}
#[derive(Debug, Clone)]
pub struct ProcessMonitorOptions {
    pub target: ProcessIdentity,
    pub output_endpoint_id: String,
    pub duration: SessionDuration,
    pub config: MonitorConfig,
}

fn native_latency_options(
    latency: MonitorLatencyConfig,
) -> Result<
    (
        moiren_windows_audio::shared_stream::SharedPeriodPolicy,
        moiren_windows_audio::latency::LatencyBudgetPolicy,
    ),
    MonitorError,
> {
    latency.validate()?;
    let shared_period = moiren_windows_audio::shared_stream::SharedPeriodPolicy {
        desired_period_ms: latency.shared_period_ms,
    };
    let budget = match latency.profile {
        MonitorLatencyProfile::Compatible => {
            moiren_windows_audio::latency::LatencyBudgetPolicy::Compatible
        }
        MonitorLatencyProfile::Balanced | MonitorLatencyProfile::Custom => {
            moiren_windows_audio::latency::LatencyBudgetPolicy::Custom {
                input_capacity_frames: 8192,
                input_target_frames: latency
                    .input_target_frames
                    .ok_or(super::MonitorLatencyError::MissingInputTarget)?,
                input_jitter_frames: latency
                    .input_jitter_frames
                    .ok_or(super::MonitorLatencyError::IncompleteProfile)?,
                render: moiren_windows_audio::latency::RenderQueuePolicy::Target {
                    frames: latency
                        .render_target_frames
                        .ok_or(super::MonitorLatencyError::MissingRenderTarget)?,
                    wake_margin_frames: latency
                        .wake_margin_frames
                        .ok_or(super::MonitorLatencyError::IncompleteProfile)?,
                    callback_budget_frames: latency
                        .callback_budget_frames
                        .ok_or(super::MonitorLatencyError::IncompleteProfile)?,
                },
            }
        }
    };
    Ok((shared_period, budget))
}

fn process_bridge_options(
    latency: MonitorLatencyConfig,
    max_demand_frames: usize,
) -> Result<ProcessBridgeOptions, MonitorError> {
    use moiren_windows_audio::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };

    let input = match latency.profile {
        MonitorLatencyProfile::Compatible => TransportInputPolicy::Compatible,
        MonitorLatencyProfile::Balanced | MonitorLatencyProfile::Custom => {
            TransportInputPolicy::Target {
                capacity_frames: 8192,
                target_frames: latency
                    .input_target_frames
                    .ok_or(super::MonitorLatencyError::MissingInputTarget)?,
                jitter_frames: latency
                    .input_jitter_frames
                    .ok_or(super::MonitorLatencyError::IncompleteProfile)?,
            }
        }
    };
    let delivery =
        (latency.profile != MonitorLatencyProfile::Compatible).then_some(InputDeliveryTiming {
            sample_rate: 48_000,
            // Process Loopback is virtual; these are declared admission bounds
            // until the live packet recorder supplies empirical observations.
            max_packet_frames: 480,
            max_gap_frames: 480,
            source: DeliveryTimingSource::Declared,
        });
    ProcessBridgeOptions {
        input,
        delivery,
        engine_sample_rate: 48_000,
        max_demand_frames: u32::try_from(max_demand_frames)
            .map_err(|_| MonitorError::InvalidConfig)?,
    }
    .for_consumer(
        48_000,
        u32::try_from(max_demand_frames).map_err(|_| MonitorError::InvalidConfig)?,
    )
    .map_err(MonitorError::from)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorStatus {
    Completed,
    Stopped,
    TargetExited,
    Failed,
}
#[derive(Debug, Serialize)]
pub struct MonitorReport {
    pub schema_version: u32,
    pub status: MonitorStatus,
    pub capture: CaptureReport,
    pub render: RenderReport,
    pub bridge: BridgeSnapshot,
}
pub struct MonitorSession {
    prepared_diagnostics: Option<MonitorPreparedDiagnostics>,
    pub fade: RenderFadeControl,
    pub queue_drain_duration: Duration,
    capture: CaptureSession,
    render: RenderSession,
    pub observer: BridgeObserver,
    pub control: ControlPort,
    pub bindings: CompiledBindings,
    pub gain_node: NodeId,
    pub pan_node: NodeId,
    pub control_nodes: Option<(NodeId, NodeId)>,
    pub timeline: TimelineObserver,
}
/// Blocking cleanup returns both parameter endpoints before either is dropped.
pub struct MonitorOwnerExit {
    pub report: MonitorReport,
    pub renderer: DemandRenderer,
    pub control: ControlPort,
}
/// Unlike the CLI convenience result, failure preserves every available pure
/// Rust endpoint for retirement by the cleanup worker. A panicked render owner
/// is the only case where its renderer cannot be recovered.
pub struct MonitorCleanupExit {
    pub report: Option<MonitorReport>,
    pub partial_capture: Option<CaptureReport>,
    pub partial_render: Option<RenderReport>,
    pub renderer: Option<DemandRenderer>,
    pub control: ControlPort,
    pub error: Option<MonitorError>,
}
impl MonitorCleanupExit {
    pub fn from_join_results(
        capture: Result<(), MonitorError>,
        render: Result<DemandRenderer, MonitorError>,
        control: ControlPort,
    ) -> Self {
        let (renderer, render_error) = match render {
            Ok(renderer) => (Some(renderer), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            report: None,
            partial_capture: None,
            partial_render: None,
            renderer,
            control,
            error: capture.err().or(render_error),
        }
    }
}
pub struct PreparedMonitorSession {
    session: MonitorSession,
    gate: ActivationGate,
}
impl PreparedMonitorSession {
    pub fn request_fade_in(&self, frames: u32) -> Result<(), MonitorError> {
        self.session.fade.request_fade_in(frames)?;
        Ok(())
    }
    pub fn activate(&self) -> Result<(), MonitorError> {
        self.gate.activate()?;
        Ok(())
    }
    pub fn request_stop(&self) -> Result<(), MonitorError> {
        self.session.request_stop()
    }
    pub fn into_session(self) -> MonitorSession {
        self.session
    }
}
impl MonitorSession {
    pub fn observation(&self) -> Option<MonitorObservation> {
        let prepared = self.prepared_diagnostics?;
        let bridge = self.observer.snapshot();
        Some(MonitorObservation {
            prepared,
            processed_frames: self.timeline.snapshot().frame,
            input_fill_frames: bridge.fill_frames as u64,
            correction_ppm: bridge.correction_ppm,
        })
    }
    /// Both native clients must acknowledge Start before reporting Running.
    pub fn has_started(&self) -> bool {
        self.capture.has_started() && self.render.has_started()
    }

    pub fn request_stop(&self) -> Result<(), MonitorError> {
        // Attempt both signals even if the first fails; never strand one owner.
        let capture = self.capture.request_stop();
        let render = self.render.request_stop();
        capture?;
        render?;
        Ok(())
    }
    pub fn is_finished(&self) -> bool {
        self.capture.is_finished() || self.render.is_finished()
    }
    /// Blocking control-side wait. A stopped/failed owner cancels its peer;
    /// Capture failure cannot silently turn into a successful silent render.
    pub fn join(self) -> Result<MonitorReport, MonitorError> {
        Ok(self.join_with_renderer()?.report)
    }
    /// Run on a cleanup worker. Drain control replies and retire the returned
    /// renderer before releasing it or the returned ControlPort.
    pub fn join_with_renderer(self) -> Result<MonitorOwnerExit, MonitorError> {
        let exit = self.join_for_cleanup();
        if let Some(error) = exit.error {
            return Err(error);
        }
        Ok(MonitorOwnerExit {
            report: exit.report.expect("both native reports"),
            renderer: exit.renderer.expect("successful render join"),
            control: exit.control,
        })
    }
    pub fn join_for_cleanup(self) -> MonitorCleanupExit {
        while !self.is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let signals = self.request_stop();
        let capture = self.capture.join();
        let render = self.render.join_with_renderer();
        // Always join both workers before propagating a panic or stop failure.
        let (capture_report, capture_result) = match capture {
            Ok(report) => (Some(report), Ok(())),
            Err(error) => (None, Err(error.into())),
        };
        let (render_report, renderer_result) = match render {
            Ok(exit) => (Some(exit.report), Ok(exit.renderer)),
            Err(error) => (None, Err(error.into())),
        };
        let mut exit =
            MonitorCleanupExit::from_join_results(capture_result, renderer_result, self.control);
        exit.error = exit.error.or(signals.err());
        match (capture_report, render_report) {
            (Some(capture), Some(render)) => {
                exit.report = Some(MonitorReport {
                    schema_version: 7,
                    status: report_status(capture.status, render.status),
                    capture,
                    render,
                    bridge: self.observer.snapshot(),
                })
            }
            (capture, render) => {
                exit.partial_capture = capture;
                exit.partial_render = render;
            }
        }
        exit
    }
}
fn report_status(capture: CaptureStatus, render: RenderStatus) -> MonitorStatus {
    if capture == CaptureStatus::Failed || render == RenderStatus::Failed {
        MonitorStatus::Failed
    } else if capture == CaptureStatus::TargetExited {
        MonitorStatus::TargetExited
    } else if capture == CaptureStatus::Completed || render == RenderStatus::Completed {
        MonitorStatus::Completed
    } else {
        MonitorStatus::Stopped
    }
}
fn new_stop() -> Result<StopSignal, MonitorError> {
    StopSignal::new().map_err(|error| {
        moiren_windows_audio::capture::CaptureError::Api {
            stage: "CreateEvent(monitor stop)",
            hresult: error.code().0,
        }
        .into()
    })
}
pub fn start_monitor(options: MonitorOptions) -> Result<MonitorSession, MonitorError> {
    let prepared = prepare_monitor_with_stop(options, new_stop()?)?;
    prepared.activate()?;
    Ok(prepared.into_session())
}
/// Worker-side preparation shares one gate across capture and render. Device
/// negotiation and graph compilation finish before either stream can start.
pub fn prepare_monitor_with_stop(
    options: MonitorOptions,
    stop: StopSignal,
) -> Result<PreparedMonitorSession, MonitorError> {
    prepare_monitor_inner(options, stop, None, None)
}
pub(crate) fn prepare_monitor_with_signal_host(
    options: MonitorOptions,
    stop: StopSignal,
    signals: &mut crate::service::SignalPreparation,
) -> Result<PreparedMonitorSession, MonitorError> {
    prepare_monitor_inner(options, stop, Some(signals), None)
}
pub(crate) fn prepare_routed_monitor(
    options: MonitorOptions,
    stop: StopSignal,
    signals: &mut crate::service::SignalPreparation,
    routing: &crate::service::GraphPreparation,
) -> Result<PreparedMonitorSession, MonitorError> {
    prepare_monitor_inner(options, stop, Some(signals), Some(routing))
}
fn prepare_monitor_inner(
    options: MonitorOptions,
    stop: StopSignal,
    signals: Option<&mut crate::service::SignalPreparation>,
    routing: Option<&crate::service::GraphPreparation>,
) -> Result<PreparedMonitorSession, MonitorError> {
    options.config.validate()?;
    let (shared_period, latency_budget) = native_latency_options(options.config.latency)?;
    let capture_options = CaptureOptions {
        artifact: None,
        latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
        shared_period,
        endpoint_id: options.input_endpoint_id,
        duration: options.duration,
    };
    let render_options = RenderOptions {
        queue: match latency_budget {
            moiren_windows_audio::latency::LatencyBudgetPolicy::Compatible => {
                moiren_windows_audio::latency::RenderQueuePolicy::FullCapacity
            }
            moiren_windows_audio::latency::LatencyBudgetPolicy::Custom { render, .. } => render,
        },
        latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
        shared_period,
        endpoint_id: options.output_endpoint_id,
        duration: options.duration,
    };
    capture_options.validate()?;
    render_options.validate()?;
    let gate = ActivationGate::new(stop.clone())?;
    let capture = prepare_capture_native_with_gate(capture_options, stop.clone(), gate.clone())?;
    let render = prepare_render_native_with_gate(render_options.clone(), stop, gate.clone())?;
    let input_info = capture.info();
    let output_info = render.info();
    let budget = moiren_windows_audio::latency::prepare_latency_budget(
        input_info.timing,
        output_info.timing,
        latency_budget,
    )?;
    let diagnostics = MonitorPreparedDiagnostics::from_native(
        budget,
        input_info.shared_period,
        output_info.shared_period,
    );
    let capture = capture.attach(budget.input)?;
    attach_monitor(
        capture,
        render_options,
        options.config,
        gate,
        signals,
        Some((render, diagnostics)),
        routing,
    )
}
pub fn start_process_monitor(
    options: ProcessMonitorOptions,
) -> Result<MonitorSession, MonitorError> {
    start_process_monitor_with_stop(options, new_stop()?)
}
pub fn start_process_monitor_with_stop(
    options: ProcessMonitorOptions,
    stop: StopSignal,
) -> Result<MonitorSession, MonitorError> {
    let prepared = prepare_process_monitor_with_stop(options, stop)?;
    prepared.activate()?;
    Ok(prepared.into_session())
}
pub fn prepare_process_monitor_with_stop(
    options: ProcessMonitorOptions,
    stop: StopSignal,
) -> Result<PreparedMonitorSession, MonitorError> {
    prepare_process_monitor_inner(options, stop, None, None)
}
pub(crate) fn prepare_process_monitor_with_signal_host(
    options: ProcessMonitorOptions,
    stop: StopSignal,
    signals: &mut crate::service::SignalPreparation,
) -> Result<PreparedMonitorSession, MonitorError> {
    prepare_process_monitor_inner(options, stop, Some(signals), None)
}
pub(crate) fn prepare_routed_process_monitor(
    options: ProcessMonitorOptions,
    stop: StopSignal,
    signals: &mut crate::service::SignalPreparation,
    routing: &crate::service::GraphPreparation,
) -> Result<PreparedMonitorSession, MonitorError> {
    prepare_process_monitor_inner(options, stop, Some(signals), Some(routing))
}
fn prepare_process_monitor_inner(
    options: ProcessMonitorOptions,
    stop: StopSignal,
    signals: Option<&mut crate::service::SignalPreparation>,
    routing: Option<&crate::service::GraphPreparation>,
) -> Result<PreparedMonitorSession, MonitorError> {
    options.config.validate()?;
    let (shared_period, latency_budget) = native_latency_options(options.config.latency)?;
    let render_options = RenderOptions {
        queue: match latency_budget {
            moiren_windows_audio::latency::LatencyBudgetPolicy::Compatible => {
                moiren_windows_audio::latency::RenderQueuePolicy::FullCapacity
            }
            moiren_windows_audio::latency::LatencyBudgetPolicy::Custom { render, .. } => render,
        },
        latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
        shared_period,
        endpoint_id: options.output_endpoint_id,
        duration: options.duration,
    };
    render_options.validate()?;
    let gate = ActivationGate::new(stop.clone())?;
    let capture = prepare_process_capture_with_gate(
        ProcessLoopbackOptions {
            artifact: None,
            bridge: process_bridge_options(
                options.config.latency,
                options.config.max_block_frames,
            )?,
            latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
            target: options.target,
            duration: options.duration,
        },
        stop,
        gate.clone(),
    )?;
    attach_monitor(
        capture,
        render_options,
        options.config,
        gate,
        signals,
        None,
        routing,
    )
}
fn attach_monitor(
    capture: PreparedCapture,
    render_options: RenderOptions,
    config: MonitorConfig,
    gate: ActivationGate,
    signals: Option<&mut crate::service::SignalPreparation>,
    native_render: Option<(PreparedNativeRender, MonitorPreparedDiagnostics)>,
    routing: Option<&crate::service::GraphPreparation>,
) -> Result<PreparedMonitorSession, MonitorError> {
    let (compiled, output, gain_node, pan_node, control_nodes) = if let Some(routing) = routing {
        let graph = crate::session_graph::prepare_routing(
            routing,
            capture.source,
            config.max_block_frames,
            signals,
        )?;
        let (source, _) = routing
            .io_nodes()
            .map_err(|_| MonitorError::InvalidConfig)?;
        let (gain, pan) = graph.controls.unwrap_or((source, source));
        (graph.compiled, graph.output, gain, pan, graph.controls)
    } else {
        let graph = if let Some(signals) = signals {
            super::prepare_monitor_with_signals(capture.source, config, signals)?
        } else {
            prepare_monitor(capture.source, config)?
        };
        let controls = Some((graph.gain_node, graph.pan_node));
        (
            graph.compiled,
            graph.output,
            graph.gain_node,
            graph.pan_node,
            controls,
        )
    };
    let renderer = DemandRenderer::new(compiled.engine, output)?;
    let fade = renderer.fade_control();
    // Submission acknowledges generated silence, not presentation. Allow a full
    // native capacity plus one period to drain before requesting Stop.
    let prepared_diagnostics = native_render.as_ref().map(|(_, diagnostics)| *diagnostics);
    let queue_drain_duration = native_render
        .as_ref()
        .map_or(Duration::ZERO, |(render, _)| {
            let timing = render.info().timing;
            let frames =
                u64::from(timing.capacity_frames) + u64::from(timing.period_frames.unwrap_or(0));
            Duration::from_nanos((frames * 1_000_000_000).div_ceil(u64::from(timing.sample_rate)))
        });
    let timeline = renderer.timeline_observer();
    // All fallible later preparation is protected by CaptureSession's stop/join
    // Drop. There is no live capture leak if compile or render startup fails.
    let render = if let Some((render, _)) = native_render {
        render.attach(renderer, render_options.queue)?
    } else {
        start_render_prepared_with_gate(
            render_options,
            renderer,
            capture.session.stop_signal(),
            gate.clone(),
        )?
    };
    Ok(PreparedMonitorSession {
        gate,
        session: MonitorSession {
            prepared_diagnostics,
            fade,
            queue_drain_duration,
            capture: capture.session,
            render: render.into_session(),
            observer: capture.observer,
            control: compiled.control,
            bindings: compiled.bindings,
            gain_node,
            pan_node,
            control_nodes,
            timeline,
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn either_owner_failure_dominates_peer_completion_or_stop() {
        assert_eq!(
            report_status(CaptureStatus::Failed, RenderStatus::Completed),
            MonitorStatus::Failed
        );
        assert_eq!(
            report_status(CaptureStatus::Completed, RenderStatus::Failed),
            MonitorStatus::Failed
        );
        assert_eq!(
            report_status(CaptureStatus::Completed, RenderStatus::Stopped),
            MonitorStatus::Completed
        );
        assert_eq!(
            report_status(CaptureStatus::Stopped, RenderStatus::Stopped),
            MonitorStatus::Stopped
        );
        assert_eq!(
            report_status(CaptureStatus::TargetExited, RenderStatus::Completed),
            MonitorStatus::TargetExited
        );
        assert_eq!(
            report_status(CaptureStatus::TargetExited, RenderStatus::Failed),
            MonitorStatus::Failed
        );
    }

    #[test]
    fn process_balanced_profile_builds_an_explicit_input_budget() {
        let bridge = process_bridge_options(MonitorLatencyConfig::balanced(), 256).unwrap();
        assert_eq!(bridge.max_demand_frames, 256);
        assert_eq!(
            bridge.input,
            moiren_windows_audio::latency::transport::TransportInputPolicy::Target {
                capacity_frames: 8192,
                target_frames: 1024,
                jitter_frames: 96,
            }
        );
        assert!(bridge.delivery.is_some());
    }

    #[test]
    fn process_compatible_profile_keeps_legacy_unqualified_delivery() {
        let bridge = process_bridge_options(MonitorLatencyConfig::default(), 256).unwrap();
        assert_eq!(
            bridge.input,
            moiren_windows_audio::latency::transport::TransportInputPolicy::Compatible
        );
        assert!(bridge.delivery.is_none());
    }
}
