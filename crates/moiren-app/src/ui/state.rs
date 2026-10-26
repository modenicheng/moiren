//! UI-side projection of the service runtime state.
//!
//! The service owns audio and publishes [`AppSnapshot`]s; the UI polls them
//! off the audio thread and renders an owned copy. Requests flow back through
//! [`AppHandle`] without ever blocking presentation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_i18n::t;

use crate::service::{
    AppHandle, AppRequest, AppSnapshot, AudioBackend, DeviceCatalog, DeviceRow, GraphEdit,
    GraphPreparation, InputSelection, RequestId, RunLimit, SessionPhase, SessionSpec,
};

/// Instant-apply settles continuous adjustments (slider drags) this long
/// after the last edit; the service receives one Apply instead of one per
/// pointer move. Structural edits apply without this delay.
pub(crate) const AUTO_APPLY_DEBOUNCE: Duration = Duration::from_millis(300);

pub(crate) fn input_rows(catalog: &DeviceCatalog, backend: AudioBackend) -> &[DeviceRow] {
    match backend {
        AudioBackend::Wasapi => &catalog.inputs,
        AudioBackend::Asio => &catalog.asio_drivers,
    }
}

pub(crate) fn output_rows(catalog: &DeviceCatalog, backend: AudioBackend) -> &[DeviceRow] {
    match backend {
        AudioBackend::Wasapi => &catalog.outputs,
        AudioBackend::Asio => &catalog.asio_drivers,
    }
}

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
    backend: AudioBackend,
    input_initialized: bool,
    output_initialized: bool,
}

impl SelectionState {
    pub fn refresh(&mut self, catalog: &DeviceCatalog) {
        self.refresh_for_backend(catalog, self.backend);
    }
    pub fn refresh_for_backend(&mut self, catalog: &DeviceCatalog, backend: AudioBackend) {
        self.backend = backend;
        // WASAPI has system defaults; ASIO requires an explicit stable driver
        // binding. A backend switch preserves IDs without substituting rows.
        if backend == AudioBackend::Asio {
            return;
        }
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
        input_rows(catalog, self.backend)
            .iter()
            .position(|row| Some(&row.endpoint_id) == self.input_id.as_ref())
    }
    pub fn output_index(&self, catalog: &DeviceCatalog) -> Option<usize> {
        output_rows(catalog, self.backend)
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
                .and_then(|id| {
                    input_rows(catalog, self.backend)
                        .iter()
                        .find(|r| r.endpoint_id == id)
                })
                .map(|r| InputSelection::Physical {
                    endpoint_id: r.endpoint_id.clone(),
                }),
        }
    }
}

/// Root gpui entity: owns the service handle and the latest projected state.
/// The text projections and the display-level stale guard live in
/// [`crate::ui::presentation`].
pub struct UiState {
    pub(crate) startup_project: Option<std::sync::Arc<crate::project::ProjectDocument>>,
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
    /// Debounced instant-apply deadline; fired by [`Self::poll`] once the
    /// edit stream settles and the session is still running.
    auto_apply_at: Option<Instant>,
    request_id: Option<RequestId>,
}

impl UiState {
    pub fn new(handle: AppHandle, initial: Arc<AppSnapshot>) -> Self {
        let mut selection = SelectionState::default();
        selection.refresh_for_backend(&initial.catalog, initial.audio_settings.backend);
        let (gain, pan) = primary_parameters(&initial);
        let max_block_frames = initial.audio_settings.processing.block_frames as usize;
        let mut state = Self {
            startup_project: initial.loaded_project.clone(),
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
            max_block_frames,
            run_limit: RunLimit::UntilStopped,
            auto_apply_at: None,
            request_id: None,
        };
        let resources = graph_resources(&state.snapshot);
        state.sync_graph_resources(resources.0, resources.1);
        state
    }

    /// Poll the service snapshot. Returns true when the projection changed.
    pub fn poll(&mut self) -> bool {
        let changed = self.poll_snapshot();
        self.pump_auto_apply();
        changed
    }

    fn poll_snapshot(&mut self) -> bool {
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
            self.selection
                .refresh_for_backend(&snapshot.catalog, snapshot.audio_settings.backend);
            if self.snapshot.audio_settings.processing.block_frames
                != snapshot.audio_settings.processing.block_frames
            {
                self.max_block_frames = snapshot.audio_settings.processing.block_frames as usize;
            }
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

    /// Instant-apply edit pipeline: every accepted edit schedules the running
    /// session to adopt the new candidate. Continuous adjustments (drags)
    /// coalesce into one Apply after [`AUTO_APPLY_DEBOUNCE`]; structural edits
    /// apply on the next poll tick. Dispatch failures surface through the
    /// shared error strip.
    pub fn edit_graph(&mut self, edit: GraphEdit) {
        let _ = self.try_edit_graph(edit);
    }

    pub fn try_edit_graph(
        &mut self,
        edit: GraphEdit,
    ) -> Result<RequestId, crate::service::DispatchError> {
        let continuous = edit.is_continuous_adjustment();
        let result = self.try_dispatch(AppRequest::EditGraph(edit));
        if result.is_ok() {
            self.schedule_auto_apply(continuous);
        }
        result
    }

    fn schedule_auto_apply(&mut self, continuous: bool) {
        if self.snapshot.phase != SessionPhase::Running {
            // Idle edits stay candidate; the next Start applies them.
            return;
        }
        let delay = if continuous {
            AUTO_APPLY_DEBOUNCE
        } else {
            Duration::ZERO
        };
        self.auto_apply_at = Some(Instant::now() + delay);
    }

    /// Drop any pending debounce and apply right away (slider release).
    pub fn apply_graph_now(&mut self) {
        self.auto_apply_at = None;
        if self.snapshot.phase == SessionPhase::Running {
            self.dispatch(AppRequest::ApplyGraphIfRunning);
        }
    }

    fn pump_auto_apply(&mut self) {
        let Some(at) = self.auto_apply_at else {
            return;
        };
        if at > Instant::now() {
            return;
        }
        self.auto_apply_at = None;
        // Re-check against the freshest snapshot: a Stop that landed after the
        // edit cancels the follow-up, and the driver ignores it regardless.
        if self.snapshot.phase == SessionPhase::Running {
            self.dispatch(AppRequest::ApplyGraphIfRunning);
        }
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

    /// Restore intent before the first Start; persisted profiles do not start audio.
    pub fn restore_preferences(&mut self, preferences: &crate::project::AppPreferences) {
        if let Err(error) = preferences.audio.validate() {
            self.dispatch_error = Some(error.to_string());
            return;
        }
        if self
            .try_dispatch(AppRequest::SetAudioSettings(preferences.audio.clone()))
            .is_err()
        {
            return;
        }
        self.max_block_frames = preferences.audio.processing.block_frames as usize;
        let _ = self.try_dispatch(AppRequest::SetLatencyProfile(preferences.latency_profile));
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
            self.edit_graph(GraphEdit::SetGain {
                node,
                gain: self.effective_gain(),
            });
        }
        if let Some(node) = pan {
            self.edit_graph(GraphEdit::SetPan {
                node,
                pan: self.pan,
            });
        }
    }

    /// Adopt a restored source: the selection identity, the derived mode, and
    /// the tone frequency always move together.
    fn apply_source(&mut self, source: InputSelection) {
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

    /// The candidate source the current mode and selection describe; None
    /// while a required identity is still missing.
    fn current_selection(&self) -> Option<InputSelection> {
        match self.source_mode {
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
        }
    }

    /// Saving keeps stable identities even when a device temporarily vanishes.
    pub fn candidate_config(&self) -> Option<SessionSpec> {
        let source = self.current_selection()?;
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
        let backend = self.snapshot.audio_settings.backend;
        if !output_rows(catalog, backend)
            .iter()
            .any(|row| row.endpoint_id == spec.output_endpoint_id)
        {
            return None;
        }
        let available = match &spec.source {
            InputSelection::Tone { .. } => true,
            InputSelection::Physical { endpoint_id } => input_rows(catalog, backend)
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

    fn candidate_preparation(&self) -> GraphPreparation {
        let graph = &self.snapshot.graph;
        GraphPreparation {
            exports: graph.exports.clone(),
            graph: graph.graph.clone(),
            nodes: graph.nodes.clone(),
            revision: graph.candidate_revision,
        }
    }

    /// Validate the actual graph before choosing the legacy physical request.
    /// Virtual primary IO must stay in the typed graph path, even when stale
    /// physical selectors are still present in the UI selection state.
    fn transport_request(&self) -> Result<AppRequest, String> {
        let mut preparation = self.candidate_preparation();
        let (source, output) = preparation.primary_io_nodes()?;
        let primary_is_virtual = preparation.exports.iter().any(|export| {
            export.enabled
                && matches!(export.binding, Some(node) if node == source || node == output)
        });
        let (request, block_frames) = if primary_is_virtual {
            (
                AppRequest::ApplyGraph,
                self.snapshot.audio_settings.processing.block_frames as usize,
            )
        } else {
            let spec = self.candidate_spec().ok_or_else(|| {
                t!("Choose an available source and output before starting.").to_string()
            })?;
            // Start applies these selectors to primary IO in the service.
            // Validate the same candidate without editing the shared snapshot.
            let mut graph = crate::service::ServiceGraph::default();
            graph.replace_with_exports(
                preparation.graph,
                preparation.nodes,
                preparation.exports,
            )?;
            graph.configure_session(&spec)?;
            preparation = graph.preparation();
            let block_frames = spec.max_block_frames;
            (AppRequest::Start(spec), block_frames)
        };
        preparation
            .configured_engine_project(block_frames, &self.snapshot.audio_settings)
            .map_err(|error| t!("Graph is not ready: %{error}", error = error).to_string())?;
        Ok(request)
    }

    pub fn can_start_transport(&self) -> bool {
        self.transport_request().is_ok()
    }

    fn sync_graph_resources(&mut self, source: Option<InputSelection>, output: Option<String>) {
        if let Some(source) = source {
            self.apply_source(source);
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
        // A graph-only template has no persisted source intent; absent input
        // selection is a deliberate empty state, not an uninitialized one.
        if let Some(source) = source {
            self.apply_source(source);
        } else {
            self.source_mode = SourceMode::Input;
        }
        let (gain, pan) = primary_parameters(&self.snapshot);
        self.gain = gain;
        self.pan = pan;
        self.muted = false;
        self.dimmed = false;
        self.max_block_frames = self.snapshot.audio_settings.processing.block_frames as usize;
        self.run_limit = RunLimit::UntilStopped;
    }

    pub fn toggle_transport(&mut self) {
        match self.snapshot.phase {
            SessionPhase::Running | SessionPhase::Starting => self.dispatch(AppRequest::Stop),
            SessionPhase::Idle | SessionPhase::Failed => match self.transport_request() {
                Ok(request) => self.dispatch(request),
                Err(error) => self.dispatch_error = Some(error),
            },
            _ => {}
        }
    }

    pub fn restore_candidate(&mut self, spec: &SessionSpec) {
        self.apply_source(spec.source.clone());
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
        let source = self.current_selection();
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
            self.edit_graph(GraphEdit::BindSource { node, source });
        }
        if let Some((node, endpoint_id)) = output.zip(self.selection.output_id.clone()) {
            self.edit_graph(GraphEdit::BindOutput { node, endpoint_id });
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
    output_rows(&snapshot.catalog, snapshot.audio_settings.backend)
        .iter()
        .find(|row| row.endpoint_id == id)
        .map(|row| row.name.clone())
        .unwrap_or_else(|| t!("Unavailable output").into())
}

/// Presentation label for a node. While a Source/Sink still carries its
/// default label (never renamed), it follows the bound input/output device
/// name; renaming it once pins the custom name.
pub fn display_label(
    snapshot: &AppSnapshot,
    entry: &crate::service::GraphNodeState,
    kind: moiren_core::graph::NodeKind,
) -> String {
    use moiren_core::graph::NodeKind;
    if entry.label != crate::service::default_label(kind) {
        return entry.label.clone();
    }
    match kind {
        NodeKind::Source => entry
            .source
            .as_ref()
            .map(|source| source_name(snapshot, source))
            .unwrap_or_else(|| entry.label.clone()),
        NodeKind::Sink => entry
            .output_endpoint_id
            .as_ref()
            .map(|endpoint| output_name(snapshot, endpoint))
            .unwrap_or_else(|| entry.label.clone()),
        _ => entry.label.clone(),
    }
}

pub fn source_name(snapshot: &AppSnapshot, source: &InputSelection) -> String {
    match source {
        InputSelection::Tone { frequency_hz } => t!(
            "Test tone · %{frequency} Hz",
            frequency = format!("{frequency_hz:.0}")
        )
        .to_string(),
        InputSelection::Physical { endpoint_id } => {
            input_rows(&snapshot.catalog, snapshot.audio_settings.backend)
                .iter()
                .find(|row| row.endpoint_id == *endpoint_id)
                .map(|row| row.name.clone())
                .unwrap_or_else(|| t!("Unavailable input").into())
        }
        InputSelection::Process {
            pid,
            creation_time_100ns,
        } => snapshot
            .catalog
            .processes
            .iter()
            .find(|row| row.pid == *pid && row.creation_time_100ns == *creation_time_100ns)
            .map(|row| t!("%{name} (PID %{pid})", name = row.name, pid = pid).to_string())
            .unwrap_or_else(|| t!("Unavailable application (PID %{pid})", pid = pid).to_string()),
    }
}

#[cfg(test)]
#[path = "state/transport_tests.rs"]
mod transport_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ProcessRow, ServiceMailbox};
    use crate::ui::{finish_service, initial_projection, spawn_service, wait_until};
    use moiren_core::graph::NodeKind;

    fn catalog_state(backend: AudioBackend) -> (UiState, ServiceMailbox) {
        let row = |id: &str, name: &str| DeviceRow {
            endpoint_id: id.into(),
            name: name.into(),
        };
        let mut context = crate::service::ServiceContext::default();
        context.audio_settings.backend = backend;
        context.catalog = Arc::new(DeviceCatalog {
            inputs: vec![
                row("wasapi-input", "WASAPI microphone"),
                row("shared-id", "WASAPI shared input"),
            ],
            outputs: vec![
                row("wasapi-output", "WASAPI speakers"),
                row("shared-id", "WASAPI shared output"),
            ],
            asio_drivers: vec![
                row("asio:{driver}", "ASIO interface"),
                row("shared-id", "ASIO shared driver"),
            ],
            processes: vec![ProcessRow {
                pid: 42,
                creation_time_100ns: 7,
                name: "Player".into(),
            }],
            default_input_endpoint_id: Some("wasapi-input".into()),
            default_output_endpoint_id: Some("wasapi-output".into()),
            ..Default::default()
        });
        let (handle, mailbox) = ServiceMailbox::with_snapshot(
            std::thread::current(),
            crate::service::snapshot(&context),
        );
        let initial = handle.try_snapshot().unwrap();
        (UiState::new(handle, initial), mailbox)
    }

    #[test]
    fn asio_candidates_and_selected_rows_use_the_same_driver_in_both_directions() {
        let (mut state, _) = catalog_state(AudioBackend::Asio);
        assert_eq!(state.selection.input_id, None);
        assert_eq!(state.selection.output_id, None);
        state.source_mode = SourceMode::Input;
        state.selection.input_id = Some("asio:{driver}".into());
        state.selection.output_id = Some("asio:{driver}".into());
        let spec = state
            .candidate_spec()
            .expect("installed ASIO duplex driver");
        assert_eq!(spec.output_endpoint_id, "asio:{driver}");
        assert_eq!(
            spec.source,
            InputSelection::Physical {
                endpoint_id: "asio:{driver}".into()
            }
        );
        assert_eq!(
            state.selection.input_index(&state.snapshot.catalog),
            Some(0)
        );
        assert_eq!(
            state.selection.output_index(&state.snapshot.catalog),
            Some(0)
        );
        assert_eq!(
            state.selection.input_selection(&state.snapshot.catalog),
            Some(spec.source.clone())
        );
        assert_eq!(source_name(&state.snapshot, &spec.source), "ASIO interface");
        assert_eq!(
            output_name(&state.snapshot, &spec.output_endpoint_id),
            "ASIO interface"
        );
    }

    #[test]
    fn vanished_asio_driver_keeps_saved_identity_without_wasapi_fallback() {
        let (mut state, _) = catalog_state(AudioBackend::Asio);
        state.source_mode = SourceMode::Input;
        state.selection.input_id = Some("shared-id".into());
        state.selection.output_id = Some("shared-id".into());
        Arc::make_mut(&mut Arc::make_mut(&mut state.snapshot).catalog)
            .asio_drivers
            .clear();
        assert!(state.candidate_config().is_some());
        assert!(state.candidate_spec().is_none());
        assert_eq!(state.selection.input_index(&state.snapshot.catalog), None);
        assert_eq!(state.selection.output_index(&state.snapshot.catalog), None);
        assert_eq!(
            state.selection.input_selection(&state.snapshot.catalog),
            None
        );
        assert_eq!(
            output_name(&state.snapshot, "shared-id"),
            "Unavailable output"
        );
        assert_eq!(
            source_name(
                &state.snapshot,
                &InputSelection::Physical {
                    endpoint_id: "shared-id".into()
                }
            ),
            "Unavailable input"
        );
        assert_eq!(state.selection.input_id.as_deref(), Some("shared-id"));
        assert_eq!(state.selection.output_id.as_deref(), Some("shared-id"));
    }

    #[test]
    fn requested_backend_controls_names_and_row_validation_across_switches() {
        let (mut state, mailbox) = catalog_state(AudioBackend::Wasapi);
        state.source_mode = SourceMode::Input;
        assert!(state.candidate_spec().is_some());
        assert_eq!(
            output_name(&state.snapshot, "shared-id"),
            "WASAPI shared output"
        );
        let source = InputSelection::Physical {
            endpoint_id: "shared-id".into(),
        };
        assert_eq!(source_name(&state.snapshot, &source), "WASAPI shared input");

        let mut snapshot = (*state.snapshot).clone();
        snapshot.active_audio_settings = Some(snapshot.audio_settings.clone());
        snapshot.audio_settings.backend = AudioBackend::Asio;
        mailbox.publish(snapshot);
        state.poll();
        assert!(
            state.candidate_spec().is_none(),
            "WASAPI selections remain unavailable in ASIO"
        );
        assert_eq!(state.selection.input_index(&state.snapshot.catalog), None);
        assert_eq!(state.selection.output_index(&state.snapshot.catalog), None);
        assert_eq!(
            state.selection.input_selection(&state.snapshot.catalog),
            None
        );
        assert_eq!(
            output_name(&state.snapshot, "shared-id"),
            "ASIO shared driver"
        );
        assert_eq!(source_name(&state.snapshot, &source), "ASIO shared driver");
        assert_eq!(
            output_name(&state.snapshot, "wasapi-output"),
            "Unavailable output"
        );

        state.selection.input_id = Some("asio:{driver}".into());
        state.selection.output_id = Some("asio:{driver}".into());
        assert!(state.candidate_spec().is_some());
        let mut snapshot = (*state.snapshot).clone();
        snapshot.audio_settings.backend = AudioBackend::Wasapi;
        mailbox.publish(snapshot);
        state.poll();
        assert!(
            state.candidate_spec().is_none(),
            "ASIO selections remain unavailable in WASAPI"
        );
        assert_eq!(state.selection.input_index(&state.snapshot.catalog), None);
        assert_eq!(state.selection.output_index(&state.snapshot.catalog), None);
        assert_eq!(
            output_name(&state.snapshot, "asio:{driver}"),
            "Unavailable output"
        );
    }

    #[test]
    fn asio_output_accepts_tone_and_live_process_without_substituting_physical_input() {
        let (mut state, _) = catalog_state(AudioBackend::Asio);
        state.selection.output_id = Some("asio:{driver}".into());
        assert!(
            state.candidate_spec().is_some(),
            "software tone uses an ASIO output"
        );
        state.source_mode = SourceMode::Application;
        state.selection.process_key = Some((42, 7));
        let spec = state
            .candidate_spec()
            .expect("live process uses an ASIO output");
        assert_eq!(
            source_name(&state.snapshot, &spec.source),
            "Player (PID 42)"
        );
        assert_eq!(
            state.selection.input_selection(&state.snapshot.catalog),
            Some(spec.source)
        );
        state.selection.process_key = Some((42, 8));
        state.selection.input_id = Some("asio:{driver}".into());
        assert_eq!(
            state.selection.input_selection(&state.snapshot.catalog),
            None
        );
        assert!(state.candidate_spec().is_none());
    }

    #[test]
    fn control_feedback_reports_results_without_internal_protocol_identifiers() {
        use moiren_core::protocol::{ControlReply, ReplyCode};
        let runtime = spawn_service();
        let handle = runtime.handle();
        let snapshot = initial_projection(&handle);
        let mut state = UiState::new(handle.clone(), snapshot);
        for (code, expected) in [
            (ReplyCode::Applied, "Changes applied"),
            (ReplyCode::AppliedLate, "Changes applied"),
            (ReplyCode::Accepted, "Applying changes..."),
            (
                ReplyCode::StaleRevision,
                "Audio routing changed. Try again.",
            ),
        ] {
            Arc::make_mut(&mut state.snapshot).control.last_result = Some(ControlReply {
                code,
                request_id: 1234,
                plan_revision: 5678,
                timeline_epoch: 91011,
                effective_frame: 121314,
            });
            assert_eq!(
                crate::ui::presentation::control_feedback(&state.snapshot, state.pending_request),
                expected
            );
        }
        finish_service(runtime);
    }

    #[test]
    #[cfg(windows)]
    fn professional_timing_uses_actual_format_and_hides_stale_generation() {
        use crate::service::{
            AudioBackend, AudioSettings, ProfessionalSessionObservations, SampleFormat,
            SessionGeneration,
        };
        use moiren_engine::node::DeviceOptions;
        use moiren_windows_audio::professional::{
            ProfessionalObservationSnapshot, ProfessionalStreamObservation,
            native::NativeStreamInfo,
        };
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_projection(&handle));
        let node = state.snapshot.graph.graph.nodes()[0].id();
        let snapshot = Arc::make_mut(&mut state.snapshot);
        snapshot.generation = SessionGeneration(7);
        snapshot.phase = SessionPhase::Running;
        let mut settings = AudioSettings::default();
        settings.processing.sample_rate = 96_000;
        snapshot.active_audio_settings = Some(settings);
        snapshot.professional_observations = Some(ProfessionalSessionObservations {
            generation: SessionGeneration(7),
            observation: ProfessionalObservationSnapshot {
                owner_generation: 3,
                running: true,
                streams: vec![ProfessionalStreamObservation {
                    node,
                    input: false,
                    master: true,
                    requested: DeviceOptions {
                        backend: AudioBackend::Asio,
                        hardware_sample_rate: Some(96_000),
                        sample_format: SampleFormat::Native,
                        period_frames: Some(64),
                        ..Default::default()
                    },
                    actual: NativeStreamInfo {
                        sample_rate: 44_100,
                        format: SampleFormat::Pcm24,
                        buffer_frames: 1024,
                        channels: 2,
                    },
                    dropped_frames: 9,
                    silent_frames: 11,
                    native_callbacks: 100,
                    native_dropped_frames: 3,
                    native_silent_frames: 5,
                    driver_latency: Some(
                        moiren_windows_audio::professional::native::NativeLatency {
                            input_frames: 32,
                            output_frames: 64,
                        },
                    ),
                }],
            },
        });
        let rows = crate::ui::presentation::native_timing_rows(&state.snapshot);
        let actual = &rows
            .iter()
            .find(|(key, _)| key.contains("actual stream"))
            .unwrap()
            .1;
        assert!(actual.contains("44100 Hz"));
        assert!(actual.contains("24-bit PCM"));
        assert!(actual.contains("1024 frames"));
        assert!(!actual.contains("96000"));
        let mode = &rows
            .iter()
            .find(|(key, _)| key.contains("backend / mode"))
            .unwrap()
            .1;
        assert!(mode.contains("driver owns mode"));
        assert!(!mode.contains("Shared"));
        let native = &rows
            .iter()
            .find(|(key, _)| key.contains("ASIO callback counters"))
            .unwrap()
            .1;
        assert!(native.contains("100 callbacks"));
        assert!(native.contains("dropped 3 / render silence 5"));
        assert!(native.contains("startup"));
        let latency = &rows
            .iter()
            .find(|(key, _)| key.contains("driver-reported latency"))
            .unwrap()
            .1;
        assert!(latency.contains("Input 32 / output 64 frames"));
        assert!(latency.contains("excludes application queues"));
        Arc::make_mut(&mut state.snapshot).generation = SessionGeneration(8);
        assert!(
            !crate::ui::presentation::native_timing_rows(&state.snapshot)
                .iter()
                .any(|(key, _)| key.contains("actual stream"))
        );
        finish_service(runtime);
    }

    #[test]
    fn restored_audio_settings_sync_block_size_without_starting_audio() {
        use crate::service::{AudioSettings, ProcessingPrecision};
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_projection(&handle));
        let mut audio = AudioSettings::default();
        audio.processing.sample_rate = 96_000;
        audio.processing.precision = ProcessingPrecision::F64;
        audio.processing.block_frames = 1024;
        state.restore_preferences(&crate::project::AppPreferences {
            audio: audio.clone(),
            ..Default::default()
        });
        assert_eq!(state.max_block_frames, 1024);
        wait_until(std::time::Duration::from_secs(2), || {
            state.poll();
            !state.pending_request
        });
        assert_eq!(state.snapshot.audio_settings, audio);
        assert_eq!(state.snapshot.active_audio_settings, None);
        assert_eq!(state.snapshot.phase, SessionPhase::Idle);
        assert_eq!(
            crate::ui::presentation::latency_apply_text(&state.snapshot),
            "Explicit stream buffers control professional audio latency"
        );
        finish_service(runtime);
    }

    #[test]
    fn invalid_restored_audio_does_not_hide_its_error_with_latency_dispatch() {
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_projection(&handle));
        let mut preferences = crate::project::AppPreferences::default();
        preferences.audio.processing.sample_rate = 192_000;
        preferences.latency_profile = crate::service::AudioLatencyProfile::Low;
        state.restore_preferences(&preferences);
        assert!(
            state
                .dispatch_error
                .as_ref()
                .unwrap()
                .contains("sample rate")
        );
        assert!(!state.pending_request);
        assert_eq!(state.max_block_frames, 256);
        assert_eq!(state.snapshot.consumed_request_id, RequestId::default());
        finish_service(runtime);
    }

    #[test]
    fn restored_latency_preference_selects_next_start_without_starting_audio() {
        use crate::service::AudioLatencyProfile;
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_projection(&handle));
        state.restore_preferences(&crate::project::AppPreferences {
            latency_profile: AudioLatencyProfile::Low,
            ..Default::default()
        });
        wait_until(std::time::Duration::from_secs(2), || {
            state.poll();
            !state.pending_request
        });
        assert_eq!(state.snapshot.latency_profile, AudioLatencyProfile::Low);
        assert_eq!(state.snapshot.phase, SessionPhase::Idle);
        assert_eq!(state.snapshot.active_latency_profile, None);
        assert_eq!(
            crate::ui::presentation::latency_apply_text(&state.snapshot),
            "Selected profile applies on next start"
        );
        let snapshot = Arc::make_mut(&mut state.snapshot);
        snapshot.active_latency_profile = Some(AudioLatencyProfile::Compatible);
        assert!(
            crate::ui::presentation::latency_apply_text(&state.snapshot).starts_with("Pending:")
        );
        Arc::make_mut(&mut state.snapshot).active_latency_profile = Some(AudioLatencyProfile::Low);
        assert_eq!(
            crate::ui::presentation::latency_apply_text(&state.snapshot),
            "Active profile matches selection"
        );
        finish_service(runtime);
    }

    #[cfg(windows)]
    #[test]
    fn native_timing_projection_hides_stopped_and_stale_owner_observations() {
        use crate::service::{NativeSessionObservations, SessionGeneration};
        use moiren_windows_audio::{
            engine_backend::{
                NativeBridgeObservation, NativeObservationSnapshot, NativeStreamKind,
                NativeStreamObservation,
            },
            latency::NativeStreamTiming,
        };
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut snapshot = (*initial_projection(&handle)).clone();
        snapshot.phase = SessionPhase::Running;
        snapshot.generation = SessionGeneration(8);
        let node = snapshot.graph.graph.nodes()[0].id();
        snapshot.native_observations = Some(NativeSessionObservations {
            generation: SessionGeneration(8),
            observation: NativeObservationSnapshot {
                owner_generation: 3,
                running: true,
                streams: vec![NativeStreamObservation {
                    node,
                    kind: NativeStreamKind::MasterOutput,
                    timing: Some(NativeStreamTiming {
                        sample_rate: 48_000,
                        period_frames: Some(336),
                        capacity_frames: 960,
                    }),
                    shared_period: Some(moiren_windows_audio::shared_stream::SharedPeriodReport {
                        desired_period_ms: Some(5.0),
                        actual_period_frames: Some(336),
                        ..Default::default()
                    }),
                    render_queue: None,
                    input_queue: None,
                }],
                boundary_probes: vec![],
                bridges: vec![NativeBridgeObservation {
                    node,
                    counters: moiren_windows_audio::clock_bridge::BridgeSnapshot {
                        underrun_frames: 4,
                        dropped_frames: 8,
                        resets: 1,
                        backlog_recoveries: 1,
                        ..Default::default()
                    },
                }],
            },
        });
        let mut state = UiState::new(handle.clone(), Arc::new(snapshot));
        assert!(
            crate::ui::presentation::native_timing_rows(&state.snapshot)[0]
                .1
                .contains("336 frames (7.00 ms)")
        );
        assert!(
            crate::ui::presentation::native_timing_rows(&state.snapshot)
                .iter()
                .any(|(_, value)| value == "5.00 ms requested")
        );
        assert!(
            crate::ui::presentation::native_timing_rows(&state.snapshot)
                .iter()
                .any(|(label, value)| {
                    label == "Stream 1: faults / recovery"
                        && value == "Underrun 4 / dropped 8 frames; resets 1 / backlog recoveries 1"
                })
        );
        Arc::make_mut(&mut state.snapshot).generation = SessionGeneration(9);
        assert_eq!(
            crate::ui::presentation::native_timing_rows(&state.snapshot),
            vec![("Actual native timing".into(), "—".into())]
        );
        Arc::make_mut(&mut state.snapshot).generation = SessionGeneration(8);
        Arc::make_mut(&mut state.snapshot).phase = SessionPhase::Idle;
        assert_eq!(
            crate::ui::presentation::native_timing_rows(&state.snapshot)[0].1,
            "—"
        );
        finish_service(runtime);
    }

    #[test]
    fn service_resource_binding_changes_update_the_shared_session_projection() {
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_projection(&handle));
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
        wait_until(std::time::Duration::from_secs(2), || {
            state.poll();
            !state.pending_request
        });
        assert_eq!(state.source_mode, SourceMode::Input);
        assert_eq!(
            state.selection.input_id.as_deref(),
            Some("service-selected-input")
        );
        finish_service(runtime);
    }

    #[test]
    fn unavailable_resources_remain_in_saved_candidate_configuration() {
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_projection(&handle));
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
        finish_service(runtime);
    }

    #[test]
    fn graph_only_template_clears_previous_project_resource_selection() {
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut state = UiState::new(handle.clone(), initial_projection(&handle));
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
        finish_service(runtime);
    }

    #[test]
    fn running_projection_edits_candidate_nodes_instead_of_legacy_session_pair() {
        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut snapshot = (*initial_projection(&handle)).clone();
        snapshot.phase = SessionPhase::Running;
        let mut state = UiState::new(handle.clone(), Arc::new(snapshot));
        state.gain = 0.25;
        state.pan = 0.4;
        state.request_parameters();
        wait_until(std::time::Duration::from_secs(2), || {
            state.poll();
            !state.pending_request
        });
        for node in state.snapshot.graph.graph.nodes() {
            let entry = state.snapshot.graph.node(node.id()).unwrap();
            match node.kind() {
                NodeKind::Gain => assert_eq!(entry.gain, 0.25),
                NodeKind::Pan => assert_eq!(entry.pan, 0.4),
                _ => {}
            }
        }
        assert!(state.snapshot.control.pending_gain_pan.is_none());
        finish_service(runtime);
    }
}
