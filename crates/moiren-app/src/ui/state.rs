//! UI-side projection of the service runtime state.
//!
//! The service owns audio and publishes [`AppSnapshot`]s; the UI polls them
//! off the audio thread and renders an owned copy. Requests flow back through
//! [`AppHandle`] without ever blocking presentation.

use std::sync::Arc;

use crate::service::{
    AppHandle, AppRequest, AppSnapshot, DeviceCatalog, InputSelection, RequestId, RunLimit,
    SessionPhase, SessionSpec,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SourceMode {
    #[default]
    Tone,
    Input,
    Application,
}

impl SourceMode {
    pub fn title(self) -> &'static str {
        match self {
            Self::Tone => "Test tone",
            Self::Input => "Physical input",
            Self::Application => "Application",
        }
    }
}

/// Startup runs before the UI loop; a concurrent service publish may briefly
/// hold the mailbox. Later UI polls stay nonblocking and retain their last frame.
pub(crate) fn initial_snapshot(handle: &AppHandle) -> anyhow::Result<Arc<AppSnapshot>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(snapshot) = handle.try_snapshot() {
            return Ok(snapshot);
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "service did not publish its initial snapshot"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Selected identities outlive row positions. A vanished identity remains
/// selected but invalid until the user deliberately chooses another row.
#[derive(Default, Debug)]
pub struct SelectionState {
    pub input_id: Option<String>,
    pub output_id: Option<String>,
    pub process_key: Option<(u32, u64)>,
    input_initialized: bool,
    output_initialized: bool,
}

impl SelectionState {
    pub fn refresh(&mut self, catalog: &DeviceCatalog) {
        if !self.input_initialized && !catalog.inputs.is_empty() {
            self.input_id = catalog.default_input_endpoint_id.clone();
            self.input_initialized = true;
        }
        if !self.output_initialized && !catalog.outputs.is_empty() {
            self.output_id = catalog.default_output_endpoint_id.clone();
            self.output_initialized = true;
        }
    }
    pub fn input_index(&self, catalog: &DeviceCatalog) -> Option<usize> {
        catalog
            .inputs
            .iter()
            .position(|row| Some(&row.endpoint_id) == self.input_id.as_ref())
    }
    pub fn output_index(&self, catalog: &DeviceCatalog) -> Option<usize> {
        catalog
            .outputs
            .iter()
            .position(|row| Some(&row.endpoint_id) == self.output_id.as_ref())
    }
    pub fn process_index(&self, catalog: &DeviceCatalog) -> Option<usize> {
        catalog
            .processes
            .iter()
            .position(|p| Some((p.pid, p.creation_time_100ns)) == self.process_key)
    }
    pub fn select_process(&mut self, catalog: &DeviceCatalog, row: Option<usize>) {
        self.process_key = row
            .and_then(|i| catalog.processes.get(i))
            .map(|p| (p.pid, p.creation_time_100ns));
    }
    pub fn input_selection(&self, catalog: &DeviceCatalog) -> Option<InputSelection> {
        match self.process_key {
            Some(key) => catalog
                .processes
                .iter()
                .find(|p| (p.pid, p.creation_time_100ns) == key)
                .map(|p| InputSelection::Process {
                    pid: p.pid,
                    creation_time_100ns: p.creation_time_100ns,
                }),
            None => self
                .input_id
                .as_deref()
                .and_then(|id| catalog.inputs.iter().find(|r| r.endpoint_id == id))
                .map(|r| InputSelection::Physical {
                    endpoint_id: r.endpoint_id.clone(),
                }),
        }
    }
}

/// Root gpui entity: owns the service handle and the latest projected state.
pub struct UiState {
    pub handle: AppHandle,
    pub selection: SelectionState,
    pub snapshot: Arc<AppSnapshot>,
    pub pending_request: bool,
    pub dispatch_error: Option<String>,
    pub source_mode: SourceMode,
    pub gain: f64,
    pub pan: f64,
    pub muted: bool,
    pub dimmed: bool,
    pub tone_frequency: f64,
    pub max_block_frames: usize,
    pub run_limit: RunLimit,
    request_id: Option<RequestId>,
}

impl UiState {
    pub fn new(handle: AppHandle, initial: Arc<AppSnapshot>) -> Self {
        let mut selection = SelectionState::default();
        selection.refresh(&initial.catalog);
        let (gain, pan) = primary_parameters(&initial);
        let mut state = Self {
            handle,
            selection,
            snapshot: initial,
            pending_request: false,
            dispatch_error: None,
            source_mode: SourceMode::Tone,
            gain,
            pan,
            muted: false,
            dimmed: false,
            tone_frequency: 440.,
            max_block_frames: 256,
            run_limit: RunLimit::UntilStopped,
            request_id: None,
        };
        let resources = graph_resources(&state.snapshot);
        state.sync_graph_resources(resources.0, resources.1);
        state
    }

    /// Poll the service snapshot. Returns true when the projection changed.
    pub fn poll(&mut self) -> bool {
        let Some(snapshot) = self.handle.try_snapshot() else {
            return false;
        };
        let changed = !Arc::ptr_eq(&self.snapshot, &snapshot);
        if changed {
            let previous_resources = graph_resources(&self.snapshot);
            let resources = graph_resources(&snapshot);
            if resources != previous_resources {
                self.sync_graph_resources(resources.0, resources.1);
            }
            let before = primary_parameters(&self.snapshot);
            let after = primary_parameters(&snapshot);
            if before.0 != after.0 && !self.muted && !self.dimmed {
                self.gain = after.0;
            }
            if before.1 != after.1 {
                self.pan = after.1;
            }
            self.selection.refresh(&snapshot.catalog);
            self.snapshot = snapshot;
            if self
                .request_id
                .is_some_and(|id| self.snapshot.consumed_request_id >= id)
            {
                self.pending_request = false;
            }
        }
        changed
    }

    pub fn dispatch(&mut self, request: AppRequest) {
        let _ = self.try_dispatch(request);
    }

    pub fn try_dispatch(
        &mut self,
        request: AppRequest,
    ) -> Result<RequestId, crate::service::DispatchError> {
        match self.handle.try_request_tracked(request) {
            Ok(id) => {
                self.pending_request = true;
                self.request_id = Some(id);
                self.dispatch_error = None;
                Ok(id)
            }
            Err(error) => {
                self.dispatch_error = Some(format!("{error:?}"));
                Err(error)
            }
        }
    }

    pub fn phase_text(&self) -> &'static str {
        use crate::service::SessionPhase;
        match self.snapshot.phase {
            SessionPhase::Idle => "Idle",
            SessionPhase::Starting => "Starting",
            SessionPhase::Running => "Running",
            SessionPhase::Stopping => "Stopping",
            SessionPhase::Failed => "Failed",
            SessionPhase::Exiting => "Exiting",
            SessionPhase::Exited => "Exited",
        }
    }

    pub fn elapsed_text(&self) -> String {
        let secs = self.snapshot.elapsed.as_secs();
        if secs == 0 && self.snapshot.phase != crate::service::SessionPhase::Running {
            "—".into()
        } else {
            format!("{:02}:{:02}", secs / 60, secs % 60)
        }
    }

    pub fn effective_gain(&self) -> f64 {
        if self.muted {
            0.0
        } else if self.dimmed {
            self.gain * 0.25
        } else {
            self.gain
        }
    }

    /// Explicit node targets let the service distinguish a loaded candidate
    /// from a still-running project whose NodeId numbers happen to coincide.
    pub fn request_parameters(&mut self) {
        use moiren_core::graph::NodeKind;
        let gain = self
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Gain)
            .map(|n| n.id());
        let pan = self
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Pan)
            .map(|n| n.id());
        if let Some(node) = gain {
            self.dispatch(AppRequest::EditGraph(crate::service::GraphEdit::SetGain {
                node,
                gain: self.effective_gain(),
            }));
        }
        if let Some(node) = pan {
            self.dispatch(AppRequest::EditGraph(crate::service::GraphEdit::SetPan {
                node,
                pan: self.pan,
            }));
        }
    }

    /// Saving keeps stable identities even when a device temporarily vanishes.
    pub fn candidate_config(&self) -> Option<SessionSpec> {
        let source = match self.source_mode {
            SourceMode::Tone => InputSelection::Tone {
                frequency_hz: self.tone_frequency,
            },
            SourceMode::Input => InputSelection::Physical {
                endpoint_id: self.selection.input_id.clone()?,
            },
            SourceMode::Application => {
                let (pid, creation_time_100ns) = self.selection.process_key?;
                InputSelection::Process {
                    pid,
                    creation_time_100ns,
                }
            }
        };
        let spec = SessionSpec {
            source,
            output_endpoint_id: self.selection.output_id.clone()?,
            limit: self.run_limit,
            gain: self.effective_gain(),
            pan: self.pan,
            max_block_frames: self.max_block_frames,
        };
        spec.validate().ok()?;
        Some(spec)
    }

    pub fn candidate_spec(&self) -> Option<SessionSpec> {
        let spec = self.candidate_config()?;
        let catalog = &self.snapshot.catalog;
        if !catalog
            .outputs
            .iter()
            .any(|row| row.endpoint_id == spec.output_endpoint_id)
        {
            return None;
        }
        let available = match &spec.source {
            InputSelection::Tone { .. } => true,
            InputSelection::Physical { endpoint_id } => catalog
                .inputs
                .iter()
                .any(|row| &row.endpoint_id == endpoint_id),
            InputSelection::Process {
                pid,
                creation_time_100ns,
            } => catalog
                .processes
                .iter()
                .any(|row| (row.pid, row.creation_time_100ns) == (*pid, *creation_time_100ns)),
        };
        available.then_some(spec)
    }

    fn sync_graph_resources(&mut self, source: Option<InputSelection>, output: Option<String>) {
        if let Some(source) = source {
            self.source_mode = match source {
                InputSelection::Tone { frequency_hz } => {
                    self.tone_frequency = frequency_hz;
                    SourceMode::Tone
                }
                InputSelection::Physical { endpoint_id } => {
                    self.selection.input_id = Some(endpoint_id);
                    self.selection.input_initialized = true;
                    SourceMode::Input
                }
                InputSelection::Process {
                    pid,
                    creation_time_100ns,
                } => {
                    self.selection.process_key = Some((pid, creation_time_100ns));
                    SourceMode::Application
                }
            };
        }
        if let Some(output) = output {
            self.selection.output_id = Some(output);
            self.selection.output_initialized = true;
        }
    }

    pub fn restore_graph_candidate(&mut self) {
        use moiren_core::graph::NodeKind;
        let graph = &self.snapshot.graph;
        let source = graph
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Source)
            .and_then(|n| graph.node(n.id()))
            .and_then(|n| n.source.clone());
        let output = graph
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Sink)
            .and_then(|n| graph.node(n.id()))
            .and_then(|n| n.output_endpoint_id.clone());
        self.selection.input_id = None;
        self.selection.process_key = None;
        self.selection.input_initialized = true;
        self.selection.output_initialized = true;
        self.selection.output_id = output;
        self.source_mode = match source {
            Some(InputSelection::Tone { frequency_hz }) => {
                self.tone_frequency = frequency_hz;
                SourceMode::Tone
            }
            Some(InputSelection::Physical { endpoint_id }) => {
                self.selection.input_id = Some(endpoint_id);
                SourceMode::Input
            }
            Some(InputSelection::Process {
                pid,
                creation_time_100ns,
            }) => {
                self.selection.process_key = Some((pid, creation_time_100ns));
                SourceMode::Application
            }
            None => SourceMode::Input,
        };
        let (gain, pan) = primary_parameters(&self.snapshot);
        self.gain = gain;
        self.pan = pan;
        self.muted = false;
        self.dimmed = false;
        self.max_block_frames = 256;
        self.run_limit = RunLimit::UntilStopped;
    }

    pub fn toggle_transport(&mut self) {
        match self.snapshot.phase {
            SessionPhase::Running | SessionPhase::Starting => self.dispatch(AppRequest::Stop),
            SessionPhase::Idle | SessionPhase::Failed => {
                if let Some(spec) = self.candidate_spec() {
                    self.dispatch(AppRequest::Start(spec));
                } else {
                    self.dispatch_error =
                        Some("Choose an available source and output before starting.".into());
                }
            }
            _ => {}
        }
    }

    pub fn control_feedback(&self) -> String {
        let control = &self.snapshot.control;
        if self.pending_request {
            return "Requested · waiting for service".into();
        }
        if control.accepted_pending > 0 {
            return format!(
                "Accepted · {} parameter updates pending",
                control.accepted_pending
            );
        }
        if let Some(reply) = control.last_result {
            use moiren_core::protocol::ReplyCode;
            return match reply.code {
                ReplyCode::Applied | ReplyCode::AppliedLate => format!(
                    "{:?} · revision {} · frame {}",
                    reply.code, reply.plan_revision, reply.effective_frame
                ),
                code => format!("{code:?} · request {}", reply.request_id),
            };
        }
        if self.snapshot.phase == SessionPhase::Running {
            "No parameter changes requested".into()
        } else {
            "Next start · engine stopped".into()
        }
    }

    pub fn restore_candidate(&mut self, spec: &SessionSpec) {
        self.source_mode = match &spec.source {
            InputSelection::Tone { frequency_hz } => {
                self.tone_frequency = *frequency_hz;
                SourceMode::Tone
            }
            InputSelection::Physical { endpoint_id } => {
                self.selection.input_id = Some(endpoint_id.clone());
                self.selection.input_initialized = true;
                SourceMode::Input
            }
            InputSelection::Process {
                pid,
                creation_time_100ns,
            } => {
                self.selection.process_key = Some((*pid, *creation_time_100ns));
                SourceMode::Application
            }
        };
        self.selection.output_id = Some(spec.output_endpoint_id.clone());
        self.selection.output_initialized = true;
        self.gain = spec.gain;
        self.pan = spec.pan;
        self.muted = false;
        self.dimmed = false;
        self.max_block_frames = spec.max_block_frames;
        self.run_limit = spec.limit;
    }

    /// Resource selection edits the candidate bindings, never the active owner.
    pub fn bind_candidate_resources(&mut self) {
        let source = match self.source_mode {
            SourceMode::Tone => Some(InputSelection::Tone {
                frequency_hz: self.tone_frequency,
            }),
            SourceMode::Input => self
                .selection
                .input_id
                .clone()
                .map(|endpoint_id| InputSelection::Physical { endpoint_id }),
            SourceMode::Application => {
                self.selection
                    .process_key
                    .map(|(pid, creation_time_100ns)| InputSelection::Process {
                        pid,
                        creation_time_100ns,
                    })
            }
        };
        use moiren_core::graph::NodeKind;
        let input = self
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Source)
            .map(|n| n.id());
        let output = self
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Sink)
            .map(|n| n.id());
        if let Some((node, source)) = input.zip(source) {
            self.dispatch(AppRequest::EditGraph(
                crate::service::GraphEdit::BindSource { node, source },
            ));
        }
        if let Some((node, endpoint_id)) = output.zip(self.selection.output_id.clone()) {
            self.dispatch(AppRequest::EditGraph(
                crate::service::GraphEdit::BindOutput { node, endpoint_id },
            ));
        }
    }
}

fn graph_resources(snapshot: &AppSnapshot) -> (Option<InputSelection>, Option<String>) {
    use moiren_core::graph::NodeKind;
    let source = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Source)
        .and_then(|n| snapshot.graph.node(n.id()))
        .and_then(|n| n.source.clone());
    let output = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Sink)
        .and_then(|n| snapshot.graph.node(n.id()))
        .and_then(|n| n.output_endpoint_id.clone());
    (source, output)
}

fn primary_parameters(snapshot: &AppSnapshot) -> (f64, f64) {
    use moiren_core::graph::NodeKind;
    let gain = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Gain)
        .and_then(|n| snapshot.graph.node(n.id()))
        .map_or(0., |n| n.gain);
    let pan = snapshot
        .graph
        .graph
        .nodes()
        .iter()
        .find(|n| n.kind() == NodeKind::Pan)
        .and_then(|n| snapshot.graph.node(n.id()))
        .map_or(0., |n| n.pan);
    (gain, pan)
}

pub fn output_name(snapshot: &AppSnapshot, id: &str) -> String {
    snapshot
        .catalog
        .outputs
        .iter()
        .find(|row| row.endpoint_id == id)
        .map(|row| row.name.clone())
        .unwrap_or_else(|| format!("Unavailable output ({id})"))
}

pub fn source_name(snapshot: &AppSnapshot, source: &InputSelection) -> String {
    match source {
        InputSelection::Tone { frequency_hz } => format!("Test tone · {frequency_hz:.0} Hz"),
        InputSelection::Physical { endpoint_id } => snapshot
            .catalog
            .inputs
            .iter()
            .find(|row| row.endpoint_id == *endpoint_id)
            .map(|row| row.name.clone())
            .unwrap_or_else(|| "Unavailable input".into()),
        InputSelection::Process {
            pid,
            creation_time_100ns,
        } => snapshot
            .catalog
            .processes
            .iter()
            .find(|row| row.pid == *pid && row.creation_time_100ns == *creation_time_100ns)
            .map(|row| format!("{} (PID {pid})", row.name))
            .unwrap_or_else(|| format!("Unavailable application (PID {pid})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ServiceRuntime;
    use moiren_core::graph::NodeKind;

    #[test]
    fn service_resource_binding_changes_update_the_shared_session_projection() {
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_snapshot(&handle).unwrap());
        let source = state
            .snapshot
            .graph
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == NodeKind::Source)
            .unwrap()
            .id();
        state.dispatch(AppRequest::EditGraph(
            crate::service::GraphEdit::BindSource {
                node: source,
                source: InputSelection::Physical {
                    endpoint_id: "service-selected-input".into(),
                },
            },
        ));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.pending_request {
            state.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(state.source_mode, SourceMode::Input);
        assert_eq!(
            state.selection.input_id.as_deref(),
            Some("service-selected-input")
        );
        handle.request_exit();
        runtime.join().unwrap();
    }

    #[test]
    fn unavailable_resources_remain_in_saved_candidate_configuration() {
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_snapshot(&handle).unwrap());
        let spec = SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "unplugged-input".into(),
            },
            output_endpoint_id: "unplugged-output".into(),
            limit: RunLimit::Seconds(35),
            gain: 0.25,
            pan: -0.4,
            max_block_frames: 512,
        };
        state.restore_candidate(&spec);
        assert_eq!(state.candidate_config(), Some(spec));
        assert!(
            state.candidate_spec().is_none(),
            "unavailable devices cannot start"
        );
        handle.request_exit();
        runtime.join().unwrap();
    }

    #[test]
    fn graph_only_template_clears_previous_project_resource_selection() {
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_snapshot(&handle).unwrap());
        state.selection.output_id = Some("old-output".into());
        state.selection.input_id = Some("old-input".into());
        state.source_mode = SourceMode::Input;
        state.max_block_frames = 1024;
        state.run_limit = RunLimit::Seconds(60);
        state.muted = true;
        state.restore_graph_candidate();
        assert_eq!(state.source_mode, SourceMode::Tone);
        assert_eq!(state.tone_frequency, 440.);
        assert!(state.selection.output_id.is_none());
        assert!(state.selection.input_id.is_none());
        assert_eq!(state.max_block_frames, 256);
        assert_eq!(state.run_limit, RunLimit::UntilStopped);
        assert!(!state.muted);
        assert!(state.candidate_config().is_none());
        handle.request_exit();
        runtime.join().unwrap();
    }

    #[test]
    fn running_projection_edits_candidate_nodes_instead_of_legacy_session_pair() {
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let mut snapshot = (*initial_snapshot(&handle).unwrap()).clone();
        snapshot.phase = SessionPhase::Running;
        let mut state = UiState::new(handle.clone(), Arc::new(snapshot));
        state.gain = 0.25;
        state.pan = 0.4;
        state.request_parameters();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.pending_request && std::time::Instant::now() < deadline {
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!state.pending_request);
        for node in state.snapshot.graph.graph.nodes() {
            let entry = state.snapshot.graph.node(node.id()).unwrap();
            match node.kind() {
                NodeKind::Gain => assert_eq!(entry.gain, 0.25),
                NodeKind::Pan => assert_eq!(entry.pan, 0.4),
                _ => {}
            }
        }
        assert!(state.snapshot.control.pending_gain_pan.is_none());
        handle.request_exit();
        runtime.join().unwrap();
    }
}
