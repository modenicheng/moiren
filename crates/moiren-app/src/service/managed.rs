//! Desktop adapter for the engine's lifecycle owner. The service only projects
//! receipts and snapshots; compilation, swaps and native ownership stay in host.
use super::*;
use moiren_core::protocol::ParamValue;
use moiren_engine::host::{
    self, EditOutcome, EditReceipt, EngineBackend, EngineControl, EngineHost, EngineObserver,
    ExecutionState, HostConfig, ParameterOutcome, ParameterReceipt, ParameterTarget,
    ParameterTiming, UpdateState,
};
use std::time::Instant;
mod driver;
mod lifecycle;
mod meters;
mod parameters;
#[cfg(test)]
mod tests;

type Factory = dyn Fn(AudioLatencyProfile) -> Result<Box<dyn EngineBackend<f32>>, host::BackendError>
    + Send
    + Sync;
struct Backend(Box<dyn EngineBackend<f32>>);
impl EngineBackend<f32> for Backend {
    fn prepare(
        &self,
        request: host::PrepareRequest<'_>,
        cancel: &host::CancellationToken,
    ) -> Result<host::PreparedBackend<f32>, host::BackendError> {
        self.0.prepare(request, cancel)
    }
    fn poll_resource(&self) -> Option<host::ResourceEvent> {
        self.0.poll_resource()
    }
}
struct Owner {
    host: EngineHost<f32>,
    control: EngineControl,
    observer: EngineObserver,
    generation: SessionGeneration,
    document: u64,
    profile: AudioLatencyProfile,
    active_revision: (u64, u64),
    /// The Apply whose matching ACKs established `active_revision`.
    active_apply_id: u64,
    /// App revision of the latest admitted Apply in this document. Host edits
    /// can finish out of order when a same-structure edit overtakes preparation.
    latest_apply_revision: u64,
    /// Distinguishes repeated Apply calls for the same candidate revision.
    latest_apply_id: u64,
    cleanup: DeferredPermit,
    completion: CompletionPermit<Completion>,
    started: Option<Instant>,
}
struct PendingEdit {
    apply_id: u64,
    revision: u64,
    receipt: EditReceipt,
    values: Vec<(ParameterTarget, ParamValue)>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApplyConfirmation {
    id: u64,
    revision: u64,
    host_revision: u64,
}
struct PendingParameter {
    receipt: ParameterReceipt,
    /// Only this Apply's own target/value request can satisfy its barrier.
    confirmation: Option<ApplyConfirmation>,
}
enum Completion {
    Reaped(SessionGeneration),
    Catalog(Result<DeviceCatalog, BackendError>),
}

pub struct ManagedDriver {
    factory: Arc<Factory>,
    catalog_backend: Option<Arc<dyn BackendAdapter>>,
    host_config: HostConfig,
    owner: Option<Owner>,
    completions: Option<CompletionQueue<Completion>>,
    edits: Vec<PendingEdit>,
    parameters: Vec<PendingParameter>,
    unsent: Vec<(ParameterTarget, ParamValue)>,
    confirm_parameters: Option<ApplyConfirmation>,
    confirmation_values: VecDeque<(ParameterTarget, ParamValue)>,
    document: u64,
    cleaning: usize,
    refresh: bool,
    catalog_running: bool,
    automatic_startup: bool,
    exiting: bool,
    apply: bool,
    meters: Option<meters::MeterBridge>,
    #[cfg(windows)]
    observations: Option<
        Arc<std::sync::Mutex<Option<moiren_windows_audio::engine_backend::NativeObservations>>>,
    >,
}
impl ManagedDriver {
    pub fn new(
        factory: impl Fn(AudioLatencyProfile) -> Result<Box<dyn EngineBackend<f32>>, host::BackendError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            factory: Arc::new(factory),
            catalog_backend: None,
            host_config: HostConfig {
                max_nodes: 4096,
                max_edges: 16384,
                max_parameters: 65536,
                signal_bindings: 8192,
                signals: moiren_signal::BusConfig {
                    max_topics: 8192,
                    ..Default::default()
                },
                ..Default::default()
            },
            owner: None,
            completions: None,
            edits: Vec::with_capacity(16),
            parameters: Vec::with_capacity(64),
            unsent: Vec::with_capacity(64),
            confirm_parameters: None,
            confirmation_values: VecDeque::new(),
            document: 0,
            cleaning: 0,
            refresh: false,
            catalog_running: false,
            automatic_startup: false,
            exiting: false,
            apply: false,
            meters: None,
            #[cfg(windows)]
            observations: None,
        }
    }
    pub fn with_catalog(mut self, backend: Arc<dyn BackendAdapter>) -> Self {
        self.catalog_backend = Some(backend);
        self.refresh = true;
        self
    }
    pub fn with_automatic_startup(mut self) -> Self {
        self.automatic_startup = true;
        self
    }
    #[cfg(windows)]
    pub fn windows() -> Self {
        let observations = Arc::new(std::sync::Mutex::new(None));
        let publisher = observations.clone();
        let mut driver = Self::new(move |profile| {
            let backend = moiren_windows_audio::engine_backend::WindowsBackend::new(
                profile.windows_backend_config(),
            )?;
            *publisher.lock().unwrap_or_else(|p| p.into_inner()) = Some(backend.observations());
            Ok(Box::new(backend))
        })
        .with_catalog(Arc::new(WindowsBackend::default()));
        driver.observations = Some(observations);
        driver
    }
    fn queue(&mut self) -> &CompletionQueue<Completion> {
        self.completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()))
    }
}
