//! Desktop adapter for the engine's lifecycle owner. The service only projects
//! receipts and snapshots; compilation, swaps and native ownership stay in host.
use super::*;
use moiren_core::{graph::NodeId, protocol::ParamValue};
use moiren_engine::host::{
    self, EditOutcome, EditReceipt, EngineBackend, EngineControl, EngineHost, EngineObserver,
    ExecutionState, HostConfig, ParameterOutcome, ParameterReceipt, ParameterTarget,
    ParameterTiming, StripParameter, UpdateState,
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
mod driver;
mod lifecycle;
mod meters;
mod parameters;
#[cfg(test)]
mod tests;

type Factory<S> = dyn Fn(AudioLatencyProfile, &AudioSettings) -> Result<Box<dyn EngineBackend<S>>, host::BackendError>
    + Send
    + Sync;
/// Only the lifecycle owner is sample-typed; control and observations are shared
/// across precisions. Dropping either variant still happens on the cleanup worker.
enum AudioHost {
    F32(EngineHost<f32>),
    F64(EngineHost<f64>),
}
impl AudioHost {
    fn control(&self) -> EngineControl {
        match self {
            Self::F32(host) => host.control(),
            Self::F64(host) => host.control(),
        }
    }
    fn observer(&self) -> EngineObserver {
        match self {
            Self::F32(host) => host.observer(),
            Self::F64(host) => host.observer(),
        }
    }
}
struct Owner {
    host: AudioHost,
    control: EngineControl,
    observer: EngineObserver,
    generation: SessionGeneration,
    document: u64,
    profile: AudioLatencyProfile,
    audio_settings: AudioSettings,
    exports: Vec<ExportIntent>,
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
    target: ParameterTarget,
    receipt: ParameterReceipt,
    /// Only this Apply's own target/value request can satisfy its barrier.
    confirmation: Option<ApplyConfirmation>,
}
enum Completion {
    Reaped(SessionGeneration),
    Catalog(Result<DeviceCatalog, BackendError>),
    Exports(Result<super::exports::runtime::Batch, String>),
}

/// Delay before a structure edit automatically applies; consecutive edits
/// within the window coalesce into one rebuild.
const AUTO_APPLY_DELAY: Duration = Duration::from_millis(200);

pub struct ManagedDriver {
    exports_runtime: super::exports::runtime::Runtime,
    factory: Arc<Factory<f32>>,
    f64_factory: Option<Arc<Factory<f64>>>,
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
    catalog: CatalogRefresher,
    automatic_startup: bool,
    exiting: bool,
    apply: bool,
    apply_audio_settings: Option<AudioSettings>,
    /// Last submitted effective strip values (dim-folded gain, solo-audible
    /// mute) per node; diffs against the candidate drive live submissions.
    strip_live: BTreeMap<NodeId, (f64, bool)>,
    /// Trailing-edge debounce for the automatic apply of structure edits.
    /// Armed only while an owner runs: a stopped session never auto-starts.
    auto_apply_deadline: Option<Instant>,
    meters: Option<meters::MeterBridge>,
    #[cfg(windows)]
    professional_observations: Option<
        Arc<std::sync::Mutex<Option<moiren_windows_audio::professional::ProfessionalObservations>>>,
    >,
    #[cfg(windows)]
    virtual_provider: Option<Arc<moiren_windows_audio::vad::WindowsVadProvider>>,
}
impl ManagedDriver {
    pub fn new(
        factory: impl Fn(AudioLatencyProfile) -> Result<Box<dyn EngineBackend<f32>>, host::BackendError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            exports_runtime: Default::default(),
            factory: Arc::new(move |profile, _| factory(profile)),
            f64_factory: None,
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
            strip_live: BTreeMap::new(),
            auto_apply_deadline: None,
            confirm_parameters: None,
            confirmation_values: VecDeque::new(),
            document: 0,
            cleaning: 0,
            catalog: CatalogRefresher::new(false),
            automatic_startup: false,
            exiting: false,
            apply: false,
            apply_audio_settings: None,
            meters: None,
            #[cfg(windows)]
            professional_observations: None,
            #[cfg(windows)]
            virtual_provider: None,
        }
    }
    /// The original constructor remains an f32 factory for native observation
    /// and lifecycle tests; an f64 request requires an actual typed backend.
    pub fn with_f64_factory(
        mut self,
        factory: impl Fn(AudioLatencyProfile) -> Result<Box<dyn EngineBackend<f64>>, host::BackendError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.f64_factory = Some(Arc::new(move |profile, _| factory(profile)));
        self
    }
    pub fn with_catalog(mut self, backend: Arc<dyn BackendAdapter>) -> Self {
        self.catalog_backend = Some(backend);
        self.catalog.request();
        self
    }
    pub fn with_automatic_startup(mut self) -> Self {
        self.automatic_startup = true;
        self
    }
    #[cfg(windows)]
    pub fn windows() -> Self {
        let professional_observations = Arc::new(std::sync::Mutex::new(None));
        let professional_publisher = professional_observations.clone();
        let f64_publisher = professional_observations.clone();
        let virtual_provider = moiren_windows_audio::vad::WindowsVadProvider::discover().ok();
        let provider_for_factory = virtual_provider.clone();
        let provider_for_f64 = virtual_provider.clone();
        let mut driver = Self::new(|_| unreachable!("Windows factory replaced before use"))
            .with_catalog(Arc::new(WindowsBackend::default()));
        driver.factory = Arc::new(move |_, _| {
            let backend = moiren_windows_audio::professional::ProfessionalBackend::<f32>::new()?;
            let backend = if let Some(provider) = provider_for_factory.as_ref() {
                backend.with_virtual_provider(provider.clone())
            } else {
                backend
            };
            *professional_publisher
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(backend.observations());
            Ok(Box::new(backend))
        });
        driver.f64_factory = Some(Arc::new(move |_, _| {
            let backend = moiren_windows_audio::professional::ProfessionalBackend::<f64>::new()?;
            let backend = if let Some(provider) = provider_for_f64.as_ref() {
                backend.with_virtual_provider(provider.clone())
            } else {
                backend
            };
            *f64_publisher.lock().unwrap_or_else(|p| p.into_inner()) = Some(backend.observations());
            Ok(Box::new(backend))
        }));
        driver.professional_observations = Some(professional_observations);
        driver.virtual_provider = virtual_provider;
        if let Some(provider) = &driver.virtual_provider {
            driver.exports_runtime.adapter = Some(Arc::new(
                super::exports::runtime::WindowsAdapter(provider.clone()),
            ));
        }
        driver
    }
    fn queue(&mut self) -> &CompletionQueue<Completion> {
        self.completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()))
    }
}
