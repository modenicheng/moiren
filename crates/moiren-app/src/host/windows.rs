//! UI/control-domain owner. Native capture workers own their COM clients; the
//! render worker owns only the prepared renderer. All graph edits, joins and
//! resource reclamation stay on the caller of this adapter.
use super::*;
use moiren_windows_audio::{
    capture::{
        CaptureOptions, CaptureReport, CaptureSession, CaptureStatus, PreparedCapture,
        start_capture,
    },
    clock_bridge::{BridgeObserver, BridgeSnapshot},
    process_loopback::{ProcessIdentity, ProcessLoopbackOptions, start_process_capture},
    render::{
        DemandRenderer, RenderObserver, RenderOptions, RenderReport, RenderSession, RenderStatus,
        start_render,
    },
};
use std::time::{Duration, Instant};
mod model;
use model::CaptureOwner;
pub use model::*;
#[cfg(test)]
mod tests;

pub struct HostSession {
    host: Option<AudioHost>,
    render: Option<RenderSession>,
    observer: RenderObserver,
    sources: BTreeMap<SourceId, CaptureOwner>,
    status: SessionStatus,
    output_endpoint_id: String,
    failure: Option<String>,
    runtime: SessionRuntime,
    final_graph: Option<GraphSnapshot>,
    render_report: Option<RenderReport>,
    events: Vec<HostEvent>,
}
impl HostSession {
    /// Prepare every initial strip at gain 0.05, publish once, then hand off.
    /// Every partial-startup owner has a stop/join Drop guard on this caller.
    pub fn start(options: SessionOptions) -> Result<Self, SessionError> {
        RenderOptions::continuous(&options.output_endpoint_id).validate()?;
        if options.config.processing_sr != 48_000.0 {
            return Err(moiren_windows_audio::render::RenderError::UnsupportedFormat.into());
        }
        let (host, renderer) = AudioHost::prepare(options.config)?;
        let mut session = Self {
            host: Some(host),
            render: None,
            observer: RenderObserver::default(),
            sources: BTreeMap::new(),
            status: SessionStatus::Starting,
            output_endpoint_id: options.output_endpoint_id,
            failure: None,
            runtime: SessionRuntime::default(),
            final_graph: None,
            render_report: None,
            events: Vec::new(),
        };
        for selection in options.sources {
            session.add_source(selection)?;
        }
        if session.host()?.runtime_snapshot().dirty {
            session.publish()?;
        }
        let (engine, output) = renderer.into_parts();
        let mut renderer = DemandRenderer::new(engine, output)?;
        renderer.set_observer(session.observer.clone())?;
        // Ordinary start functions allocate independent stop events. Sharing a
        // capture event would let one source failure stop the whole output.
        session.render = Some(start_render(
            RenderOptions::continuous(&session.output_endpoint_id),
            renderer,
        )?);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            session.poll_internal();
            if session.status == SessionStatus::Failed {
                return Err(SessionError::Startup(
                    session.failure.clone().unwrap_or_default(),
                ));
            }
            if session.observer.snapshot().stream_started
                && !session.host()?.runtime_snapshot().pending
            {
                session.status = SessionStatus::Running;
                return Ok(session);
            }
            if Instant::now() >= deadline {
                return Err(SessionError::Startup(
                    "native Start/initial plan handshake timed out".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn host(&self) -> Result<&AudioHost, SessionError> {
        self.host.as_ref().ok_or(SessionError::NotRunning)
    }
    fn host_mut(&mut self) -> Result<&mut AudioHost, SessionError> {
        self.host.as_mut().ok_or(SessionError::NotRunning)
    }
    fn editable(&self) -> Result<(), SessionError> {
        if self.host()?.runtime_snapshot().pending {
            Err(HostError::Busy.into())
        } else {
            Ok(())
        }
    }
    /// Stage a capture; call publish once after one or several graph edits.
    pub fn add_source(&mut self, selection: CaptureSelection) -> Result<SourceId, SessionError> {
        self.editable()?;
        let PreparedCapture {
            session,
            source,
            observer,
            ..
        } = prepare_capture(&selection)?;
        let id = self.host_mut()?.add_source_with_settings(
            source,
            SourceSettings {
                gain: 0.05,
                pan: 0.0,
                available: true,
            },
        )?;
        let gate = self.host()?.source_gate(id)?;
        self.sources.insert(
            id,
            CaptureOwner {
                selection,
                session: Some(session),
                observer,
                gate,
                status: SourceStatus::Staged,
                enabled: true,
                report: None,
                failure: None,
                last_start_failure: None,
            },
        );
        Ok(id)
    }
    pub fn replace_source(
        &mut self,
        id: SourceId,
        selection: CaptureSelection,
    ) -> Result<(), SessionError> {
        self.editable()?;
        if !self.sources.contains_key(&id) {
            return Err(SessionError::UnknownSource(id));
        }
        let capture = match prepare_capture(&selection) {
            Ok(capture) => capture,
            Err(error) => {
                self.sources.get_mut(&id).unwrap().last_start_failure = Some(error.to_string());
                return Err(error);
            }
        };
        self.host_mut()?.replace_source(id, capture.source)?;
        let gate = self.host()?.source_gate(id)?;
        let old = self.sources.get_mut(&id).unwrap();
        let enabled = old.enabled;
        old.join(true);
        gate.set_available(enabled);
        *old = CaptureOwner {
            selection,
            session: Some(capture.session),
            observer: capture.observer,
            gate,
            status: SourceStatus::Staged,
            enabled,
            report: None,
            failure: None,
            last_start_failure: None,
        };
        Ok(())
    }
    pub fn restart_source(&mut self, id: SourceId) -> Result<(), SessionError> {
        let source = self
            .sources
            .get(&id)
            .ok_or(SessionError::UnknownSource(id))?;
        if matches!(source.selection, CaptureSelection::Process { .. })
            && source.status == SourceStatus::TargetExited
        {
            return Err(SessionError::ProcessRestartNeedsSelection);
        }
        self.replace_source(id, source.selection.clone())
    }
    /// Gate first so a stopped capture's already buffered frames stay silent.
    pub fn stop_source(&mut self, id: SourceId) -> Result<(), SessionError> {
        self.host()?;
        self.sources
            .get_mut(&id)
            .ok_or(SessionError::UnknownSource(id))?
            .join(true);
        Ok(())
    }
    pub fn remove_source(&mut self, id: SourceId) -> Result<(), SessionError> {
        self.editable()?;
        self.host_mut()?.remove_source(id)?;
        let source = self
            .sources
            .get_mut(&id)
            .ok_or(SessionError::UnknownSource(id))?;
        source.join(true);
        source.status = SourceStatus::Removed;
        Ok(())
    }
    pub fn enable_source(&mut self, id: SourceId, enabled: bool) -> Result<(), SessionError> {
        self.host()?;
        let source = self
            .sources
            .get_mut(&id)
            .ok_or(SessionError::UnknownSource(id))?;
        source.enabled = enabled;
        source
            .gate
            .set_available(enabled && source.session.is_some());
        if matches!(
            source.status,
            SourceStatus::Running | SourceStatus::Disabled
        ) {
            source.status = if enabled {
                SourceStatus::Running
            } else {
                SourceStatus::Disabled
            };
        }
        Ok(())
    }
    pub fn set_gain(
        &mut self,
        id: SourceId,
        value: f64,
        ramp_frames: u32,
    ) -> Result<ControlReply, SessionError> {
        self.observe();
        self.host_mut()?
            .set_gain(id, value, ramp_frames)
            .map_err(Into::into)
    }
    pub fn set_pan(
        &mut self,
        id: SourceId,
        value: f64,
        ramp_frames: u32,
    ) -> Result<ControlReply, SessionError> {
        self.observe();
        self.host_mut()?
            .set_pan(id, value, ramp_frames)
            .map_err(Into::into)
    }
    pub fn submit_parameter(
        &mut self,
        node: NodeId,
        parameter: ParameterId,
        value: ParamValue,
        at: ApplyAt,
        ramp_frames: u32,
    ) -> Result<ControlReply, SessionError> {
        self.observe();
        self.host_mut()?
            .submit_parameter(node, parameter, value, at, ramp_frames)
            .map_err(Into::into)
    }
    pub fn set_compressor(
        &mut self,
        id: SourceId,
        settings: Option<CompressorSettings>,
    ) -> Result<(), SessionError> {
        self.host_mut()?
            .set_compressor(id, settings)
            .map_err(Into::into)
    }
    pub fn graph_command(&mut self, command: GraphCommand) -> Result<GraphEdit, SessionError> {
        self.host_mut()?.graph_command(command).map_err(Into::into)
    }
    pub fn publish(&mut self) -> Result<u64, SessionError> {
        self.host_mut()?.publish().map_err(Into::into)
    }
    pub fn cancel_pending(&mut self) -> Result<(), SessionError> {
        self.host_mut()?.cancel_pending();
        Ok(())
    }
    pub fn graph_snapshot(&self) -> Result<GraphSnapshot, SessionError> {
        self.host
            .as_ref()
            .map(AudioHost::graph_snapshot)
            .or_else(|| self.final_graph.clone())
            .ok_or(SessionError::NotRunning)
    }
    fn observe(&mut self) {
        let observation = self.observer.snapshot();
        if let Some(host) = &self.host {
            host.observe_timeline(observation.timeline);
            let runtime = host.runtime_snapshot();
            self.runtime = SessionRuntime {
                timeline: observation.timeline,
                peak_amplitude: observation.peak_amplitude,
                rendered_frames: observation.counters.frames,
                rendered_blocks: observation.counters.blocks,
                rendered_segments: observation.counters.segments,
                stream_started: observation.stream_started,
                active_revision: runtime.active_revision,
                desired_revision: runtime.desired_revision,
                pending: runtime.pending,
                dirty: runtime.dirty,
            };
        }
    }
    fn poll_internal(&mut self) {
        self.observe();
        if let Some(host) = &mut self.host {
            let events = host.poll();
            let rejected = events
                .iter()
                .any(|e| matches!(e, HostEvent::PlanRejected { .. }));
            let graph = host.graph_snapshot();
            for (id, source) in &mut self.sources {
                if source
                    .session
                    .as_ref()
                    .is_some_and(CaptureSession::is_finished)
                {
                    source.join(false);
                }
                if source.status == SourceStatus::Staged && !host.runtime_snapshot().pending {
                    if rejected || !graph.sources.iter().any(|s| s.id == *id) {
                        source.join(true);
                        source.status = SourceStatus::Failed;
                        source.failure = Some("source plan rejected; capture stopped".into());
                    } else if !host.runtime_snapshot().dirty {
                        source.status = if source.enabled {
                            SourceStatus::Running
                        } else {
                            SourceStatus::Disabled
                        };
                    }
                }
            }
            self.events.extend(events);
        }
        if self.render.as_ref().is_some_and(RenderSession::is_finished) {
            self.status = SessionStatus::Failed;
            self.failure = Some("output owner ended unexpectedly".into());
            self.shutdown();
        }
        self.observe();
    }
    pub fn poll(&mut self) -> Vec<HostEvent> {
        self.poll_internal();
        std::mem::take(&mut self.events)
    }
    pub fn snapshot(&mut self) -> SessionSnapshot {
        self.observe();
        SessionSnapshot {
            schema_version: 1,
            status: self.status,
            output_endpoint_id: self.output_endpoint_id.clone(),
            failure: self.failure.clone(),
            runtime: self.runtime.clone(),
            sources: self
                .sources
                .iter()
                .map(|(id, source)| source.diagnostic(*id))
                .collect(),
        }
    }
    pub fn runtime_snapshot(&mut self) -> SessionRuntime {
        self.observe();
        self.runtime.clone()
    }
    pub fn source_snapshots(&self) -> Vec<SourceDiagnostic> {
        self.sources
            .iter()
            .map(|(id, source)| source.diagnostic(*id))
            .collect()
    }
    /// Stop and join every independent native owner before finishing graph
    /// ownership. Stop is idempotent; events are returned in the final report.
    pub fn stop(&mut self) -> SessionReport {
        self.poll_internal();
        if self.status != SessionStatus::Failed {
            self.status = SessionStatus::Stopped;
        }
        self.shutdown();
        SessionReport {
            snapshot: self.snapshot(),
            render: self.render_report.take(),
            events: std::mem::take(&mut self.events)
                .into_iter()
                .map(crate::host_cli::event_json)
                .collect(),
        }
    }
    fn shutdown(&mut self) {
        for source in self.sources.values_mut() {
            source.gate.set_available(false);
            if let Some(worker) = &source.session {
                let _ = worker.request_stop();
            }
        }
        if let Some(render) = &self.render {
            let _ = render.request_stop();
        }
        for source in self.sources.values_mut() {
            source.join(false);
        }
        if let Some(render) = self.render.take() {
            match render.join_with_renderer() {
                Ok((report, renderer)) => {
                    if report.status == RenderStatus::Failed {
                        self.status = SessionStatus::Failed;
                        self.failure = report.failure.clone();
                    }
                    self.observe();
                    if let Some(mut host) = self.host.take() {
                        self.events.extend(host.poll());
                        self.final_graph = Some(host.graph_snapshot());
                        let (engine, reader) = renderer.into_parts();
                        self.events.extend(host.finish_parts(engine, reader));
                    }
                    self.runtime.pending = false;
                    self.render_report = Some(report);
                }
                Err(error) => {
                    self.status = SessionStatus::Failed;
                    self.failure = Some(error.to_string());
                }
            }
        }
    }
    pub fn capture_devices()
    -> Result<Vec<moiren_windows_audio::capture::CaptureEndpoint>, SessionError> {
        moiren_windows_audio::capture::list_capture_endpoints().map_err(Into::into)
    }
    pub fn output_devices()
    -> Result<Vec<moiren_windows_audio::render::RenderEndpoint>, SessionError> {
        moiren_windows_audio::render::list_render_endpoints().map_err(Into::into)
    }
    pub fn processes() -> Result<Vec<ProcessIdentity>, SessionError> {
        moiren_windows_audio::process_loopback::list_processes().map_err(Into::into)
    }
}
impl Drop for HostSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn prepare_capture(selection: &CaptureSelection) -> Result<PreparedCapture, SessionError> {
    match selection {
        CaptureSelection::Physical { endpoint_id } => {
            start_capture(CaptureOptions::continuous(endpoint_id)).map_err(Into::into)
        }
        CaptureSelection::Process { identity } => {
            start_process_capture(ProcessLoopbackOptions::continuous(identity.clone()))
                .map_err(Into::into)
        }
    }
}
