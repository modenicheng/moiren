//! UI-side projection of the service runtime state.
//!
//! The service owns audio and publishes [`AppSnapshot`]s; the UI polls them
//! off the audio thread and renders an owned copy. Requests flow back through
//! [`AppHandle`] without ever blocking presentation.
//!
//! Design 23 B removed the selection ledger that once lived here: the
//! candidate graph is the only record of routing intent, Start is
//! [`AppRequest::ApplyGraph`] (the service validates and reports faults on
//! the nodes they name), and device availability is a rendering concern of
//! the row lists, not a start precondition.

use std::sync::Arc;

use crate::service::{
    AppHandle, AppRequest, AppSnapshot, AudioBackend, DeviceCatalog, DeviceRow, MixerState,
    RequestId, SessionPhase,
};

/// Whether an endpoint id names an input row the backend currently offers:
/// present among its rows and (on WASAPI) not owned by one of our VAD
/// exports, which are hidden from every selector.
fn backend_offers_input(catalog: &DeviceCatalog, backend: AudioBackend, endpoint_id: &str) -> bool {
    match backend {
        AudioBackend::Wasapi => {
            !catalog.vad_endpoints.values().any(|id| id == endpoint_id)
                && catalog
                    .inputs
                    .iter()
                    .any(|row| row.endpoint_id == endpoint_id)
        }
        AudioBackend::Asio => catalog
            .asio_drivers
            .iter()
            .any(|row| row.endpoint_id == endpoint_id),
    }
}

/// The output-row half of [`backend_offers_input`].
fn backend_offers_output(
    catalog: &DeviceCatalog,
    backend: AudioBackend,
    endpoint_id: &str,
) -> bool {
    match backend {
        AudioBackend::Wasapi => {
            !catalog.vad_endpoints.values().any(|id| id == endpoint_id)
                && catalog
                    .outputs
                    .iter()
                    .any(|row| row.endpoint_id == endpoint_id)
        }
        AudioBackend::Asio => catalog
            .asio_drivers
            .iter()
            .any(|row| row.endpoint_id == endpoint_id),
    }
}

pub(crate) fn input_rows(catalog: &DeviceCatalog, backend: AudioBackend) -> Vec<DeviceRow> {
    match backend {
        AudioBackend::Wasapi => catalog
            .inputs
            .iter()
            .filter(|row| backend_offers_input(catalog, backend, &row.endpoint_id))
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
            .filter(|row| backend_offers_output(catalog, backend, &row.endpoint_id))
            .cloned()
            .collect(),
        AudioBackend::Asio => catalog.asio_drivers.clone(),
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

/// Why the last control-side attempt failed. Storage stays typed and
/// locale-free; the one translation point is
/// [`crate::ui::presentation::failure_text`] at paint, so a stored failure
/// follows a locale switch without being rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UiFailure {
    /// The service rejected the request outright.
    /// [`crate::service::DispatchError`] is a closed set, stored whole —
    /// never Debug-formatted again.
    Dispatch(crate::service::DispatchError),
    /// Restored audio preferences failed validation.
    /// [`crate::service::AudioConfigError`] is a closed set, stored whole.
    InvalidAudioSettings(crate::service::AudioConfigError),
    /// The edited virtual exports failed validation. The producer hands
    /// free-form prose, not a closed set, so the diagnostic string travels
    /// as data and renders untranslated by design.
    ExportInvalid { reason: String },
}

/// Root gpui entity: owns the service handle and the latest projected state.
/// The text projections and the display-level stale guard live in
/// [`crate::ui::presentation`].
pub struct UiState {
    pub(crate) startup_project: Option<std::sync::Arc<crate::project::ProjectDocument>>,
    pub handle: AppHandle,
    pub snapshot: Arc<AppSnapshot>,
    /// The one meter subscription table for the whole application: the main
    /// window and the tray flyout both pump it with their own visibility, and
    /// the seen-table grace unions the two demands (design 17, data plane).
    pub node_meters: crate::ui::monitoring::NodeMeters,
    /// The spectrum analyzers' subscription table, pumped with the same
    /// visible set as the meters and feeding the graph card's live body.
    pub node_spectra: crate::ui::monitoring::NodeSpectra,
    /// The status bar's runtime statistics — XRUN history and DSP load —
    /// derived here from consecutive observation snapshots so the counters'
    /// deltas turn into one consistent model for the bar and its panels.
    pub(crate) runtime: crate::ui::monitoring::runtime::RuntimeStats,
    pub pending_request: bool,
    pub(crate) dispatch_error: Option<UiFailure>,
    request_id: Option<RequestId>,
}

impl UiState {
    pub fn new(handle: AppHandle, initial: Arc<AppSnapshot>) -> Self {
        let node_meters = crate::ui::monitoring::NodeMeters::new(handle.clone());
        let node_spectra = crate::ui::monitoring::NodeSpectra::new(handle.clone());
        Self {
            startup_project: initial.loaded_project.clone(),
            handle,
            snapshot: initial,
            node_meters,
            node_spectra,
            runtime: Default::default(),
            pending_request: false,
            dispatch_error: None,
            request_id: None,
        }
    }

    /// Poll the service snapshot. Returns true when the projection changed.
    pub fn poll(&mut self) -> bool {
        let Some(snapshot) = self.handle.try_snapshot() else {
            return false;
        };
        let changed = !Arc::ptr_eq(&self.snapshot, &snapshot);
        if changed {
            self.ingest_runtime(&snapshot);
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

    /// Feeds the runtime statistics from a freshly drained snapshot: the
    /// observation snapshot reduces to counters here, and the deltas become
    /// events and load figures inside the model. One shared entry point —
    /// only the view extraction below knows about platforms, so the model's
    /// whole surface stays compiled on every one of them.
    fn ingest_runtime(&mut self, snapshot: &AppSnapshot) {
        self.runtime
            .ingest(Self::runtime_view(snapshot), std::time::Instant::now());
    }

    /// The observation snapshot reduced to the runtime counters. `None`
    /// when the session carries no observations — and always off Windows,
    /// where the native backend does not exist.
    #[cfg(windows)]
    fn runtime_view(
        snapshot: &AppSnapshot,
    ) -> Option<crate::ui::monitoring::runtime::ObservationView> {
        use crate::ui::monitoring::runtime::{ObservationView, RenderCounters, StreamCounters};
        let status = snapshot.professional_observations.as_ref()?;
        let streams: Vec<StreamCounters> = status
            .observation
            .streams
            .iter()
            .map(|stream| StreamCounters {
                node: stream.node,
                xruns: stream.xruns,
                native_xruns: stream.native_xruns,
                engine_blocks: stream.engine_blocks,
            })
            .collect();
        // The engine render accounting rides on the master output's pump.
        let render = status
            .observation
            .streams
            .iter()
            .find(|stream| stream.master && !stream.input)
            .map(|stream| RenderCounters {
                blocks: stream.engine_blocks,
                busy_100ns: stream.engine_busy_100ns,
                peak_block_100ns: stream.engine_peak_block_100ns,
            });
        let processing = snapshot
            .active_audio_settings
            .as_ref()
            .unwrap_or(&snapshot.audio_settings)
            .processing;
        Some(ObservationView {
            generation: status.generation.0,
            streams,
            render,
            block_frames: processing.block_frames,
            sample_rate: processing.sample_rate,
        })
    }

    #[cfg(not(windows))]
    fn runtime_view(
        _snapshot: &AppSnapshot,
    ) -> Option<crate::ui::monitoring::runtime::ObservationView> {
        None
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
                self.dispatch_error = Some(UiFailure::Dispatch(error.clone()));
                Err(error)
            }
        }
    }

    /// Restore intent before the first Start; persisted profiles do not start audio.
    pub fn restore_preferences(&mut self, preferences: &crate::project::AppPreferences) {
        if let Err(error) = preferences.audio.validate() {
            self.dispatch_error = Some(UiFailure::InvalidAudioSettings(error));
            return;
        }
        if self
            .try_dispatch(AppRequest::SetAudioSettings(preferences.audio.clone()))
            .is_err()
        {
            return;
        }
        let _ = self.try_dispatch(AppRequest::SetLatencyProfile(preferences.latency_profile));
    }

    /// Start is start-by-graph (design 23 §4.2): the candidate graph is the
    /// whole request, the service is the one place that validates it, and a
    /// rejected graph reports its faults on the nodes they name. There is no
    /// UI-side validation to shadow the service's verdict or to grey the
    /// button over.
    ///
    /// The run model is always-on: there is no stop action, so the toggle can
    /// only (re-)apply the graph while the session is idle or has failed. A
    /// running session ignores it.
    pub fn toggle_transport(&mut self) {
        match self.snapshot.phase {
            SessionPhase::Idle | SessionPhase::Failed => self.dispatch(AppRequest::ApplyGraph),
            _ => {}
        }
    }
}

#[cfg(all(test, windows))]
#[path = "state/boundary_tests.rs"]
mod boundary_tests;
#[cfg(all(test, windows))]
#[path = "state/export_diagnostics_tests.rs"]
mod export_diagnostics_tests;
#[cfg(all(test, windows))]
#[path = "state/native_live_tests.rs"]
mod native_live_tests;
#[cfg(test)]
#[path = "state/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "state/transport_tests.rs"]
mod transport_tests;
