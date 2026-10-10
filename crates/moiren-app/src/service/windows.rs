//! Native preparation and worker-only join/retirement. COM stays in its owners.
use super::*;
use crate::{
    monitor::*,
    tone::{ToneConfig, prepare_tone},
};
use moiren_core::{graph::NodeId, protocol::ControlReply};
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use moiren_windows_audio::{
    SessionDuration, catalog_snapshot,
    process_loopback::inspect_process,
    render::{
        DemandRenderer, PreparedRenderSession, RenderOptions, RenderReport, RenderSession,
        RenderStatus, TimelineObserver, TimelineSnapshot, start_render_prepared,
    },
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Keep only the latest full native report for non-RT diagnostics. It is never
/// part of the UI snapshot, and serialization/destruction run on workers.
type Diagnostics = Arc<Mutex<Option<String>>>;
#[derive(Default)]
pub struct WindowsBackend {
    diagnostics: Diagnostics,
}
impl WindowsBackend {
    pub fn last_native_report(&self) -> Option<String> {
        self.diagnostics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
fn save(diagnostics: &Diagnostics, report: impl serde::Serialize) {
    let report = serde_json::to_string(&report).ok();
    *diagnostics.lock().unwrap_or_else(|e| e.into_inner()) = report;
}
fn duration(limit: RunLimit) -> SessionDuration {
    match limit {
        RunLimit::UntilStopped => SessionDuration::UntilStopped,
        RunLimit::Seconds(n) => SessionDuration::For(Duration::from_secs(u64::from(n))),
    }
}
impl BackendAdapter for WindowsBackend {
    fn prepare(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        spec.validate()
            .map_err(|e| BackendError::new("configuration", format!("{e:?}")))?;
        if cancellation.is_cancelled() {
            return Err(BackendError::new("prepare", "cancelled"));
        }
        let config = MonitorConfig {
            gain: spec.gain,
            pan: spec.pan,
            max_block_frames: spec.max_block_frames,
        };
        let duration = duration(spec.limit);
        let session = match spec.source {
            InputSelection::Physical { endpoint_id } => PreparedAppSession::Monitor {
                session: prepare_monitor_with_stop(
                    MonitorOptions {
                        input_endpoint_id: endpoint_id,
                        output_endpoint_id: spec.output_endpoint_id,
                        duration,
                        config,
                    },
                    cancellation.stop_signal(),
                )
                .map_err(|e| BackendError::new("monitor prepare", e))?,
                diagnostics: self.diagnostics.clone(),
            },
            InputSelection::Process {
                pid,
                creation_time_100ns,
            } => {
                let target =
                    inspect_process(pid).map_err(|e| BackendError::new("process identity", e))?;
                if target.creation_time_100ns != creation_time_100ns {
                    return Err(BackendError::new(
                        "process identity",
                        "selected process identity changed",
                    ));
                }
                PreparedAppSession::Monitor {
                    session: prepare_process_monitor_with_stop(
                        ProcessMonitorOptions {
                            target,
                            output_endpoint_id: spec.output_endpoint_id,
                            duration,
                            config,
                        },
                        cancellation.stop_signal(),
                    )
                    .map_err(|e| BackendError::new("process monitor prepare", e))?,
                    diagnostics: self.diagnostics.clone(),
                }
            }
            InputSelection::Tone { frequency_hz } => {
                let tone = prepare_tone(ToneConfig {
                    frequency_hz,
                    gain: spec.gain,
                    pan: spec.pan,
                    max_block_frames: spec.max_block_frames,
                })
                .map_err(|e| BackendError::new("tone graph", e))?;
                let renderer = DemandRenderer::new(tone.compiled.engine, tone.output)
                    .map_err(|e| BackendError::new("tone renderer", e))?;
                let timeline = renderer.timeline_observer();
                let render = start_render_prepared(
                    RenderOptions {
                        endpoint_id: spec.output_endpoint_id,
                        duration,
                    },
                    renderer,
                    cancellation.stop_signal(),
                )
                .map_err(|e| BackendError::new("tone prepare", e))?;
                PreparedAppSession::Tone(PreparedToneSession {
                    render,
                    control: tone.compiled.control,
                    bindings: tone.compiled.bindings,
                    timeline,
                    gain: tone.gain_node,
                    pan: tone.pan_node,
                    diagnostics: self.diagnostics.clone(),
                })
            }
        };
        Ok(Box::new(session))
    }
    fn catalog(&self) -> Result<DeviceCatalog, BackendError> {
        let native = catalog_snapshot().map_err(|e| BackendError::new("catalog", e))?;
        let mut result = DeviceCatalog::default();
        let mut pids = BTreeSet::new();
        for endpoint in native.endpoints {
            for process in endpoint.sessions {
                if process.process_id != 0 {
                    pids.insert(process.process_id);
                }
            }
            let row = DeviceRow {
                name: endpoint.name.unwrap_or_else(|| endpoint.id.clone()),
                endpoint_id: endpoint.id,
            };
            match endpoint.flow {
                "capture" => result.inputs.push(row),
                "render" => result.outputs.push(row),
                _ => {}
            }
        }
        for default in native.defaults {
            if default.role == "multimedia" {
                match default.flow {
                    "capture" => result.default_input_endpoint_id = default.id.clone(),
                    "render" => result.default_output_endpoint_id = default.id.clone(),
                    _ => {}
                }
            }
            result.default_roles.push(DefaultEndpointRow {
                flow: default.flow,
                role: default.role,
                endpoint_id: default.id,
            });
        }
        for pid in pids {
            if let Ok(identity) = inspect_process(pid) {
                result.processes.push(ProcessRow {
                    pid: identity.pid,
                    creation_time_100ns: identity.creation_time_100ns,
                    name: identity.executable_name,
                });
            }
        }
        Ok(result)
    }
}
struct PreparedToneSession {
    render: PreparedRenderSession,
    control: ControlPort,
    bindings: CompiledBindings,
    timeline: TimelineObserver,
    gain: NodeId,
    pan: NodeId,
    diagnostics: Diagnostics,
}
struct ToneOwner {
    render: RenderSession,
    control: ControlPort,
    bindings: CompiledBindings,
    timeline: TimelineObserver,
    gain: NodeId,
    pan: NodeId,
    diagnostics: Diagnostics,
}
enum PreparedAppSession {
    Monitor {
        session: PreparedMonitorSession,
        diagnostics: Diagnostics,
    },
    Tone(PreparedToneSession),
}
enum ActiveAppSession {
    Monitor {
        session: MonitorSession,
        diagnostics: Diagnostics,
    },
    Tone(ToneOwner),
}
impl PreparedSession for PreparedAppSession {
    fn activate(&self) -> Result<(), BackendError> {
        match self {
            Self::Monitor { session, .. } => session
                .activate()
                .map_err(|e| BackendError::new("monitor gate", e)),
            Self::Tone(tone) => tone
                .render
                .activate()
                .map_err(|e| BackendError::new("tone gate", e)),
        }
    }
    fn into_active(self: Box<Self>) -> Box<dyn ActiveSession> {
        Box::new(match *self {
            Self::Monitor {
                session,
                diagnostics,
            } => ActiveAppSession::Monitor {
                session: session.into_session(),
                diagnostics,
            },
            Self::Tone(tone) => ActiveAppSession::Tone(ToneOwner {
                render: tone.render.into_session(),
                control: tone.control,
                bindings: tone.bindings,
                timeline: tone.timeline,
                gain: tone.gain,
                pan: tone.pan,
                diagnostics: tone.diagnostics,
            }),
        })
    }
    fn cancel(&self) -> Result<(), BackendError> {
        match self {
            Self::Monitor { session, .. } => session
                .request_stop()
                .map_err(|e| BackendError::new("monitor cancel", e)),
            Self::Tone(tone) => tone
                .render
                .request_stop()
                .map_err(|e| BackendError::new("tone cancel", e)),
        }
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
        self.into_active().join()
    }
}
impl ActiveSession for ActiveAppSession {
    fn request_stop(&self) -> Result<(), BackendError> {
        match self {
            Self::Monitor { session, .. } => session
                .request_stop()
                .map_err(|e| BackendError::new("monitor stop", e)),
            Self::Tone(tone) => tone
                .render
                .request_stop()
                .map_err(|e| BackendError::new("tone stop", e)),
        }
    }
    fn is_finished(&self) -> bool {
        match self {
            Self::Monitor { session, .. } => session.is_finished(),
            Self::Tone(tone) => tone.render.is_finished(),
        }
    }
    fn poll_started(&self) -> StartedState {
        if self.is_finished() {
            return StartedState::Failed;
        }
        let started = match self {
            Self::Monitor { session, .. } => session.has_started(),
            Self::Tone(tone) => tone.render.has_started(),
        };
        if started {
            StartedState::Running
        } else {
            StartedState::Starting
        }
    }
    fn control(&mut self) -> Option<(&mut ControlPort, &CompiledBindings)> {
        Some(match self {
            Self::Monitor { session, .. } => (&mut session.control, &session.bindings),
            Self::Tone(tone) => (&mut tone.control, &tone.bindings),
        })
    }
    fn timeline(&self) -> Option<TimelineSnapshot> {
        Some(match self {
            Self::Monitor { session, .. } => session.timeline.snapshot(),
            Self::Tone(tone) => tone.timeline.snapshot(),
        })
    }
    fn gain_pan_nodes(&self) -> Option<(NodeId, NodeId)> {
        Some(match self {
            Self::Monitor { session, .. } => (session.gain_node, session.pan_node),
            Self::Tone(tone) => (tone.gain, tone.pan),
        })
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
        match *self {
            Self::Monitor {
                session,
                diagnostics,
            } => {
                let mut exit = session.join_for_cleanup();
                let mut report = SessionReport::stopped();
                if let Some(native) = exit.report {
                    report.status = match native.status {
                        MonitorStatus::Completed => SessionEnd::Completed,
                        MonitorStatus::Stopped => SessionEnd::Stopped,
                        MonitorStatus::TargetExited => SessionEnd::TargetExited,
                        MonitorStatus::Failed => SessionEnd::Failed,
                    };
                    report.error = native
                        .capture
                        .failure
                        .clone()
                        .or_else(|| native.render.failure.clone());
                    save(&diagnostics, &native);
                } else {
                    save(
                        &diagnostics,
                        serde_json::json!({"capture":exit.partial_capture,"render":exit.partial_render}),
                    );
                    report.status = SessionEnd::Failed;
                }
                if let Some(error) = exit.error {
                    report.status = SessionEnd::Failed;
                    report.error = Some(error.to_string());
                }
                retire(
                    exit.renderer.as_mut(),
                    &mut exit.control,
                    &mut report.control_replies,
                );
                Ok(report)
            }
            Self::Tone(mut tone) => {
                let mut report = SessionReport::stopped();
                match tone.render.join_with_renderer() {
                    Ok(mut exit) => {
                        report.status = render_end(&exit.report);
                        report.error = exit.report.failure.clone();
                        save(&tone.diagnostics, &exit.report);
                        retire(
                            Some(&mut exit.renderer),
                            &mut tone.control,
                            &mut report.control_replies,
                        );
                    }
                    Err(error) => {
                        report.status = SessionEnd::Failed;
                        report.error = Some(error.to_string());
                        save(
                            &tone.diagnostics,
                            serde_json::json!({"stage":"render join","failure":error.to_string(),"native_report":null}),
                        );
                        retire(None, &mut tone.control, &mut report.control_replies);
                    }
                }
                Ok(report)
            }
        }
    }
}
fn render_end(report: &RenderReport) -> SessionEnd {
    match report.status {
        RenderStatus::Completed => SessionEnd::Completed,
        RenderStatus::Stopped => SessionEnd::Stopped,
        RenderStatus::Failed => SessionEnd::Failed,
    }
}
/// Alternate rejection and reply draining until both endpoints are retired.
/// The service admits at most 64 unresolved requests; this Vec never grows.
fn retire(
    mut renderer: Option<&mut DemandRenderer>,
    control: &mut ControlPort,
    replies: &mut Vec<ControlReply>,
) {
    loop {
        while let Some(reply) = control.poll_applied() {
            assert!(replies.len() < 64, "bounded accepted ledger");
            replies.push(reply);
        }
        let remaining = renderer
            .as_mut()
            .map_or(0, |renderer| renderer.retire_controls());
        if remaining == 0 {
            break;
        }
    }
    while let Some(reply) = control.poll_applied() {
        assert!(replies.len() < 64, "bounded accepted ledger");
        replies.push(reply);
    }
}
