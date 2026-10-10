//! Control-side orchestration. No device objects or polling enter Engine RT.
use super::{MonitorConfig, MonitorError, prepare_monitor};
use moiren_core::graph::NodeId;
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use moiren_windows_audio::{
    ActivationGate, SessionDuration, StopSignal,
    capture::{
        CaptureOptions, CaptureReport, CaptureSession, CaptureStatus, PreparedCapture,
        prepare_capture_with_gate,
    },
    clock_bridge::{BridgeObserver, BridgeSnapshot},
    process_loopback::{
        ProcessIdentity, ProcessLoopbackOptions, prepare_process_capture_with_gate,
    },
    render::{
        DemandRenderer, RenderOptions, RenderReport, RenderSession, RenderStatus,
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
    capture: CaptureSession,
    render: RenderSession,
    pub observer: BridgeObserver,
    pub control: ControlPort,
    pub bindings: CompiledBindings,
    pub gain_node: NodeId,
    pub pan_node: NodeId,
}
/// Blocking cleanup returns both parameter endpoints before either is dropped.
pub struct MonitorOwnerExit {
    pub report: MonitorReport,
    pub renderer: DemandRenderer,
    pub control: ControlPort,
}
pub struct PreparedMonitorSession {
    session: MonitorSession,
    gate: ActivationGate,
}
impl PreparedMonitorSession {
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
        while !self.is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let signals = self.request_stop();
        let capture = self.capture.join();
        let render = self.render.join_with_renderer();
        // Always join both workers before propagating a panic or stop failure.
        let capture = capture?;
        let render = render?;
        signals?;
        let status = report_status(capture.status, render.report.status);
        Ok(MonitorOwnerExit {
            renderer: render.renderer,
            control: self.control,
            report: MonitorReport {
                schema_version: 2,
                status,
                capture,
                render: render.report,
                bridge: self.observer.snapshot(),
            },
        })
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
    options.config.validate()?;
    let capture_options = CaptureOptions {
        endpoint_id: options.input_endpoint_id,
        duration: options.duration,
    };
    let render_options = RenderOptions {
        endpoint_id: options.output_endpoint_id,
        duration: options.duration,
    };
    capture_options.validate()?;
    render_options.validate()?;
    let gate = ActivationGate::new(stop.clone())?;
    let capture = prepare_capture_with_gate(capture_options, stop, gate.clone())?;
    attach_monitor(capture, render_options, options.config, gate)
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
    options.config.validate()?;
    let render_options = RenderOptions {
        endpoint_id: options.output_endpoint_id,
        duration: options.duration,
    };
    render_options.validate()?;
    let gate = ActivationGate::new(stop.clone())?;
    let capture = prepare_process_capture_with_gate(
        ProcessLoopbackOptions {
            target: options.target,
            duration: options.duration,
        },
        stop,
        gate.clone(),
    )?;
    attach_monitor(capture, render_options, options.config, gate)
}
fn attach_monitor(
    capture: PreparedCapture,
    render_options: RenderOptions,
    config: MonitorConfig,
    gate: ActivationGate,
) -> Result<PreparedMonitorSession, MonitorError> {
    let graph = prepare_monitor(capture.source, config)?;
    let compiled = graph.compiled;
    let renderer = DemandRenderer::new(compiled.engine, graph.output)?;
    // All fallible later preparation is protected by CaptureSession's stop/join
    // Drop. There is no live capture leak if compile or render startup fails.
    let render = start_render_prepared_with_gate(
        render_options,
        renderer,
        capture.session.stop_signal(),
        gate.clone(),
    )?;
    Ok(PreparedMonitorSession {
        gate,
        session: MonitorSession {
            capture: capture.session,
            render: render.into_session(),
            observer: capture.observer,
            control: compiled.control,
            bindings: compiled.bindings,
            gain_node: graph.gain_node,
            pan_node: graph.pan_node,
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
}
