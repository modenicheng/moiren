//! Control-side orchestration. No device objects or polling enter Engine RT.
use super::{MonitorConfig, MonitorError, prepare_monitor};
use moiren_core::graph::NodeId;
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use moiren_windows_audio::{
    capture::{CaptureOptions, CaptureReport, CaptureSession, CaptureStatus, start_capture},
    clock_bridge::{BridgeObserver, BridgeSnapshot},
    render::{
        DemandRenderer, RenderOptions, RenderReport, RenderSession, RenderStatus,
        start_render_with_stop,
    },
};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MonitorOptions {
    pub input_endpoint_id: String,
    pub output_endpoint_id: String,
    pub duration: Duration,
    pub config: MonitorConfig,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorStatus {
    Completed,
    Stopped,
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
impl MonitorSession {
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
        while !self.is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let signals = self.request_stop();
        let capture = self.capture.join();
        let render = self.render.join();
        // Always join both workers before propagating a panic or stop failure.
        let capture = capture?;
        let render = render?;
        signals?;
        let status = report_status(capture.status, render.status);
        Ok(MonitorReport {
            schema_version: 1,
            status,
            capture,
            render,
            bridge: self.observer.snapshot(),
        })
    }
}
fn report_status(capture: CaptureStatus, render: RenderStatus) -> MonitorStatus {
    if capture == CaptureStatus::Failed || render == RenderStatus::Failed {
        MonitorStatus::Failed
    } else if capture == CaptureStatus::Completed || render == RenderStatus::Completed {
        MonitorStatus::Completed
    } else {
        MonitorStatus::Stopped
    }
}
pub fn start_monitor(options: MonitorOptions) -> Result<MonitorSession, MonitorError> {
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
    let capture = start_capture(capture_options)?;
    let graph = prepare_monitor(capture.source, options.config)?;
    let compiled = graph.compiled;
    let renderer = DemandRenderer::new(compiled.engine, graph.output)?;
    // All fallible later preparation is protected by CaptureSession's stop/join
    // Drop. There is no live capture leak if compile or render startup fails.
    let render = start_render_with_stop(render_options, renderer, capture.session.stop_signal())?;
    Ok(MonitorSession {
        capture: capture.session,
        render,
        observer: capture.observer,
        control: compiled.control,
        bindings: compiled.bindings,
        gain_node: graph.gain_node,
        pan_node: graph.pan_node,
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
    }
}
