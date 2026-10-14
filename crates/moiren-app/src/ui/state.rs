//! UI-side projection of the service runtime state.
//!
//! The service owns audio and publishes [`AppSnapshot`]s; the UI polls them
//! off the audio thread and renders an owned copy. Requests flow back through
//! [`AppHandle`] without ever blocking presentation.

use std::sync::Arc;

use crate::service::{AppHandle, AppRequest, AppSnapshot, DeviceCatalog, InputSelection};

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
}

impl UiState {
    pub fn new(handle: AppHandle, initial: Arc<AppSnapshot>) -> Self {
        Self {
            handle,
            selection: SelectionState::default(),
            snapshot: initial,
            pending_request: false,
            dispatch_error: None,
        }
    }

    /// Poll the service snapshot. Returns true when the projection changed.
    pub fn poll(&mut self) -> bool {
        let Some(snapshot) = self.handle.try_snapshot() else {
            return false;
        };
        let changed = !Arc::ptr_eq(&self.snapshot, &snapshot);
        if changed {
            self.selection.refresh(&snapshot.catalog);
            self.snapshot = snapshot;
        }
        changed
    }

    pub fn dispatch(&mut self, request: AppRequest) {
        match self.handle.try_request_tracked(request) {
            Ok(_) => self.pending_request = true,
            Err(error) => self.dispatch_error = Some(format!("{error:?}")),
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
}
