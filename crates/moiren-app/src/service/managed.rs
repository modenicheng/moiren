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
mod measured;
mod meters;
mod parameters;
#[cfg(windows)]
mod process_traces;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windowed;

type Factory<S> = dyn Fn(
        AudioLatencyProfile,
        &AudioSettings,
        &TransportLatencySettings,
        Option<&super::measured_process::MeasuredProcessProfile>,
    ) -> Result<Box<dyn EngineBackend<S>>, host::BackendError>
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
    transport_latency: TransportLatencySettings,
    measured_process: Option<super::measured_process::MeasuredProcessProfile>,
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
    #[cfg(windows)]
    WindowEvaluated {
        id: u64,
        result: Result<Box<super::measured_process::live::LiveProcessEvaluation>, String>,
    },
    Reaped(SessionCleanupObservation),
    Catalog(Result<DeviceCatalog, BackendError>),
    Exports(Result<super::exports::runtime::Batch, String>),
}

#[derive(Clone)]
struct ApplySettings {
    measured_process: Option<super::measured_process::MeasuredProcessProfile>,
    audio: AudioSettings,
    transport: TransportLatencySettings,
    profile: AudioLatencyProfile,
}
impl ApplySettings {
    fn capture(c: &ServiceContext) -> Self {
        Self {
            measured_process: c.measured_process.clone(),
            audio: c.audio_settings.clone(),
            transport: c.transport_latency,
            profile: c.latency_profile,
        }
    }
}

/// Delay before a structure edit automatically applies; consecutive edits
/// within the window coalesce into one rebuild.
const AUTO_APPLY_DELAY: Duration = Duration::from_millis(200);

pub struct ManagedDriver {
    #[cfg(windows)]
    window_evaluation: Option<windowed::PendingWindowEvaluation>,
    #[cfg(windows)]
    window_evaluation_id: u64,
    #[cfg(windows)]
    process_trace: Option<process_traces::SessionTrace>,
    #[cfg(windows)]
    measured_selection_id: u64,
    exports_runtime: super::exports::runtime::Runtime,
    factory: Arc<Factory<f32>>,
    f64_factory: Option<Arc<Factory<f64>>>,
    catalog_backend: Option<Arc<dyn CatalogBackend>>,
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
    apply_settings: Option<ApplySettings>,
    /// Last submitted effective strip values (dim-folded gain, solo-audible
    /// mute) per node; diffs against the candidate drive live submissions.
    strip_live: BTreeMap<NodeId, (f64, bool)>,
    /// Trailing-edge debounce for the automatic apply of structure edits.
    /// Armed only while an owner runs: a stopped session never auto-starts.
    auto_apply_deadline: Option<Instant>,
    meters: Option<meters::MeterBridge>,
    #[cfg(windows)]
    native_observations: Option<
        Arc<std::sync::Mutex<Option<moiren_windows_audio::engine_backend::NativeObservations>>>,
    >,
    #[cfg(windows)]
    professional_observations: Option<
        Arc<
            std::sync::Mutex<
                Option<moiren_windows_audio::engine_backend::ProfessionalObservations>,
            >,
        >,
    >,
    /// Tick-batch gate for [`driver::poll_observations`]: raw stream counters
    /// advance with every audio block, so the displayed snapshot refreshes at
    /// 1 Hz instead of per service poll.
    #[cfg(windows)]
    observation_refreshed_at: Option<Instant>,
    #[cfg(windows)]
    observation_identity: Option<(SessionGeneration, u64, ExecutionState)>,
    #[cfg(windows)]
    virtual_provider: Option<Arc<moiren_windows_audio::vad::WindowsVadProvider>>,
}
impl ManagedDriver {
    #[cfg(all(test, windows))]
    pub(crate) fn set_measured_test_factory(&mut self, backend: super::test_backend::FakeBackend) {
        let first = backend.clone();
        self.factory = Arc::new(move |_, _, _, measured| {
            if let Some(profile) = measured {
                profile.configure(moiren_windows_audio::engine_backend::ProfessionalBackend::<
                    f32,
                >::new()?)?;
            }
            Ok(Box::new(first.clone()))
        });
        self.f64_factory = Some(Arc::new(move |_, _, _, measured| {
            if let Some(profile) = measured {
                profile.configure(moiren_windows_audio::engine_backend::ProfessionalBackend::<
                    f64,
                >::new()?)?;
            }
            Ok(Box::new(backend.clone()))
        }));
    }
    pub fn new(
        factory: impl Fn(AudioLatencyProfile) -> Result<Box<dyn EngineBackend<f32>>, host::BackendError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            #[cfg(windows)]
            window_evaluation: None,
            #[cfg(windows)]
            window_evaluation_id: 0,
            #[cfg(windows)]
            process_trace: None,
            #[cfg(windows)]
            measured_selection_id: 0,
            exports_runtime: Default::default(),
            factory: Arc::new(move |profile, _, _, measured| {
                if measured.is_some() {
                    return Err(host::BackendError::Unsupported(
                        "backend does not accept measured Process profiles".into(),
                    ));
                }
                factory(profile)
            }),
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
            apply_settings: None,
            meters: None,
            #[cfg(windows)]
            native_observations: None,
            #[cfg(windows)]
            professional_observations: None,
            #[cfg(windows)]
            observation_refreshed_at: None,
            #[cfg(windows)]
            observation_identity: None,
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
        self.f64_factory = Some(Arc::new(move |profile, _, _, measured| {
            if measured.is_some() {
                return Err(host::BackendError::Unsupported(
                    "backend does not accept measured Process profiles".into(),
                ));
            }
            factory(profile)
        }));
        self
    }
    pub fn with_catalog(mut self, backend: Arc<dyn CatalogBackend>) -> Self {
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
        let native_observations = Arc::new(std::sync::Mutex::new(None));
        let native_publisher = native_observations.clone();
        let f64_native_publisher = native_observations.clone();
        let professional_observations = Arc::new(std::sync::Mutex::new(None));
        let professional_publisher = professional_observations.clone();
        let f64_publisher = professional_observations.clone();
        let virtual_provider = moiren_windows_audio::vad::WindowsVadProvider::discover().ok();
        let provider_for_factory = virtual_provider.clone();
        let provider_for_f64 = virtual_provider.clone();
        let mut driver = Self::new(|_| unreachable!("Windows factory replaced before use"))
            .with_catalog(Arc::new(WindowsBackend));
        driver.factory = Arc::new(move |_, _, transport, measured| {
            transport
                .validate()
                .map_err(host::BackendError::Unsupported)?;
            let backend = moiren_windows_audio::engine_backend::ProfessionalBackend::<f32>::new()?
                .with_process_bridge(transport.process_options())?
                .with_virtual_buffer(transport.virtual_options())?;
            let backend = if let Some(profile) = measured {
                profile.configure(backend)?
            } else {
                backend
            };
            let backend = if let Some(provider) = provider_for_factory.as_ref() {
                backend.with_virtual_provider(provider.clone())
            } else {
                backend
            };
            *professional_publisher
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(backend.observations());
            *native_publisher.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(backend.native_observations());
            Ok(Box::new(backend))
        });
        driver.f64_factory = Some(Arc::new(move |_, _, transport, measured| {
            transport
                .validate()
                .map_err(host::BackendError::Unsupported)?;
            let backend = moiren_windows_audio::engine_backend::ProfessionalBackend::<f64>::new()?
                .with_process_bridge(transport.process_options())?
                .with_virtual_buffer(transport.virtual_options())?;
            let backend = if let Some(profile) = measured {
                profile.configure(backend)?
            } else {
                backend
            };
            let backend = if let Some(provider) = provider_for_f64.as_ref() {
                backend.with_virtual_provider(provider.clone())
            } else {
                backend
            };
            *f64_publisher.lock().unwrap_or_else(|p| p.into_inner()) = Some(backend.observations());
            *f64_native_publisher
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(backend.native_observations());
            Ok(Box::new(backend))
        }));
        driver.native_observations = Some(native_observations);
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
