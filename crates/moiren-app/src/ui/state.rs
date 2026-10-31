//! UI-side projection of the service runtime state.
//!
//! The service owns audio and publishes [`AppSnapshot`]s; the UI polls them
//! off the audio thread and renders an owned copy. Requests flow back through
//! [`AppHandle`] without ever blocking presentation.

use std::sync::Arc;

use rust_i18n::t;

use crate::service::{
    AppHandle, AppRequest, AppSnapshot, AudioBackend, DeviceCatalog, DeviceRow, GraphPreparation,
    InputSelection, MixerState, RequestId, RunLimit, SessionPhase, SessionSpec,
};

pub(crate) fn input_rows(catalog: &DeviceCatalog, backend: AudioBackend) -> Vec<DeviceRow> {
    match backend {
        AudioBackend::Wasapi => catalog
            .inputs
            .iter()
            .filter(|row| {
                !catalog
                    .vad_endpoints
                    .values()
                    .any(|id| id == &row.endpoint_id)
            })
            .cloned()
            .collect(),
        AudioBackend::Asio => catalog.asio_drivers.clone(),
    }
}

pub(crate) fn output_rows(catalog: &DeviceCatalog, backend: AudioBackend) -> Vec<DeviceRow> {
    match backend {
        AudioBackend::Wasapi => catalog
            .outputs
            .iter()
            .filter(|row| {
                !catalog
                    .vad_endpoints
                    .values()
                    .any(|id| id == &row.endpoint_id)
            })
            .cloned()
            .collect(),
        AudioBackend::Asio => catalog.asio_drivers.clone(),
    }
}

/// Application-facing directions differ from Windows flow only for owned VAD exports.
pub(crate) fn device_rows(
    catalog: &DeviceCatalog,
    exports: &[crate::service::ExportStatus],
    backend: AudioBackend,
    input: bool,
) -> Vec<DeviceRow> {
    let mut rows = if input {
        input_rows(catalog, backend)
    } else {
        output_rows(catalog, backend)
    };
    if backend == AudioBackend::Wasapi {
        let direction = if input {
            crate::service::ExportDirection::Playback
        } else {
            crate::service::ExportDirection::Microphone
        };
        rows.extend(
            exports
                .iter()
                .filter(|status| {
                    status.intent.enabled
                        && status.intent.direction == direction
                        && status.resource == crate::service::ExportResourceState::Present
                })
                .filter_map(|status| {
                    let name = status.windows_name.as_ref()?;
                    if name.error.is_some() {
                        return None;
                    }
                    Some(DeviceRow {
                        endpoint_id: name.mmdevice_id.clone()?,
                        name: name
                            .name
                            .clone()
                            .unwrap_or_else(|| status.intent.name.clone()),
                    })
                }),
        );
    }
    rows
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
                        .into_iter()
                        .find(|r| r.endpoint_id == id)
                })
                .map(|r| InputSelection::Physical {
                    endpoint_id: r.endpoint_id.clone(),
                }),
        }
    }
}

/// Memo key for [`UiState::can_start_transport`]: every input the
/// validation consumes, cheap to copy and compare.
type TransportCheckFingerprint = ((u64, u32, u32, bool, bool), bool);

/// Root gpui entity: owns the service handle and the latest projected state.
/// The text projections and the display-level stale guard live in
/// [`crate::ui::presentation`].
pub struct UiState {
    pub(crate) startup_project: Option<std::sync::Arc<crate::project::ProjectDocument>>,
    pub handle: AppHandle,
    pub selection: SelectionState,
    pub snapshot: Arc<AppSnapshot>,
    /// The one meter subscription table for the whole application: the main
    /// window and the tray flyout both pump it with their own visibility, and
    /// the seen-table grace unions the two demands (design 17, data plane).
    pub node_meters: crate::ui::monitoring::NodeMeters,
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
    /// Memo for [`Self::can_start_transport`]: the toolbar asks per rebuild,
    /// and the answer clones the whole candidate graph to validate. The key
    /// carries every input of the validation — candidate revision plus the
    /// processing settings `configured_engine_project` consumes — so a
    /// settings change without a graph edit still invalidates it.
    transport_check: std::cell::Cell<Option<TransportCheckFingerprint>>,
}

impl UiState {
    pub fn new(handle: AppHandle, initial: Arc<AppSnapshot>) -> Self {
        let mut selection = SelectionState::default();
        selection.refresh_for_backend(&initial.catalog, initial.audio_settings.backend);
        let node_meters = crate::ui::monitoring::NodeMeters::new(handle.clone());
        let (gain, pan) = primary_parameters(&initial);
        let max_block_frames = initial.audio_settings.processing.block_frames as usize;
        let mut state = Self {
            startup_project: initial.loaded_project.clone(),
            handle,
            selection,
            snapshot: initial,
            node_meters,
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
            request_id: None,
            transport_check: std::cell::Cell::new(None),
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

    /// One channel-strip edit: read the requested strip state from the live
    /// snapshot, apply `change`, and dispatch `SetMixer`. A node that vanished
    /// since the surface was rendered simply skips. Callers own the repaint
    /// notification, because the edited surface differs (mixer page, tray
    /// flyout, inspector editor).
    pub fn edit_mixer(&mut self, id: moiren_core::graph::NodeId, change: impl Fn(&mut MixerState)) {
        if let Some(node) = self.snapshot.graph.node(id) {
            let mut requested = node.mixer;
            change(&mut requested);
            self.dispatch(AppRequest::EditGraph(crate::service::GraphEdit::SetMixer {
                node: id,
                state: requested,
            }));
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
            .node_of_kind(NodeKind::Gain)
            .map(|(node, _)| node.id());
        let pan = self
            .snapshot
            .graph
            .node_of_kind(NodeKind::Pan)
            .map(|(node, _)| node.id());
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
            .configured_engine_project(
                block_frames,
                &self.snapshot.audio_settings,
                self.snapshot.latency_profile,
            )
            .map_err(|error| t!("Graph is not ready: %{error}", error = error).to_string())?;
        Ok(request)
    }

    pub fn can_start_transport(&self) -> bool {
        let fingerprint = (
            self.snapshot.graph.candidate_revision,
            self.snapshot.audio_settings.processing.sample_rate,
            self.snapshot.audio_settings.processing.block_frames,
            matches!(
                self.snapshot.audio_settings.processing.precision,
                moiren_engine::audio_config::ProcessingPrecision::F64
            ),
            self.snapshot.latency_profile == crate::service::AudioLatencyProfile::default(),
        );
        if let Some((seen, ok)) = self.transport_check.get()
            && seen == fingerprint
        {
            return ok;
        }
        let ok = self.transport_request().is_ok();
        self.transport_check.set(Some((fingerprint, ok)));
        ok
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
            .node_of_kind(NodeKind::Source)
            .and_then(|(_, state)| state.source.clone());
        let output = graph
            .node_of_kind(NodeKind::Sink)
            .and_then(|(_, state)| state.output_endpoint_id.clone());
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
            .node_of_kind(NodeKind::Source)
            .map(|(node, _)| node.id());
        let output = self
            .snapshot
            .graph
            .node_of_kind(NodeKind::Sink)
            .map(|(node, _)| node.id());
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
        .node_of_kind(NodeKind::Source)
        .and_then(|(_, state)| state.source.clone());
    let output = snapshot
        .graph
        .node_of_kind(NodeKind::Sink)
        .and_then(|(_, state)| state.output_endpoint_id.clone());
    (source, output)
}

fn primary_parameters(snapshot: &AppSnapshot) -> (f64, f64) {
    use moiren_core::graph::NodeKind;
    let gain = snapshot
        .graph
        .node_of_kind(NodeKind::Gain)
        .map_or(0., |(_, state)| state.gain);
    let pan = snapshot
        .graph
        .node_of_kind(NodeKind::Pan)
        .map_or(0., |(_, state)| state.pan);
    (gain, pan)
}

pub fn output_name(snapshot: &AppSnapshot, id: &str) -> String {
    output_rows(&snapshot.catalog, snapshot.audio_settings.backend)
        .into_iter()
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
    if let Some(export) = snapshot
        .graph
        .exports
        .iter()
        .find(|export| export.enabled && export.binding == Some(entry.id))
    {
        return export.name.clone();
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
                .into_iter()
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
#[path = "state/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "state/transport_tests.rs"]
mod transport_tests;
