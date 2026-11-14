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
mod recordings;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windowed;

pub(crate) use self::meters::LevelRegistry;
pub use self::meters::{LevelLease, LevelSource};

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

#[cfg(all(test, windows))]
type MeasuredSnapshotReader = dyn Fn() -> Option<moiren_windows_audio::engine_backend::NativeObservationSnapshot>
    + Send
    + Sync;

pub struct ManagedDriver {
    #[cfg(all(test, windows))]
    measured_snapshot_reader: Option<Arc<MeasuredSnapshotReader>>,
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
    /// Live recording sessions, one per recorder node at most; each reserves
    /// its cleanup slot upfront (see `recordings::OwnedRecording`).
    recording_sessions: BTreeMap<NodeId, recordings::OwnedRecording>,
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
    /// Startup with a restored project: once the first catalog arrives the
    /// restored graph starts as saved. IO nodes whose devices are absent
    /// stay in the graph and run as silent null streams until the device
    /// returns (the driver's resume path).
    automatic_resume: bool,
    /// What resume stripped, published as a fault only after the resumed
    /// session has activated, since activation clears the fault group.
    resume_notice: Option<GraphFault>,
    exiting: bool,
    apply: bool,
    apply_settings: Option<ApplySettings>,
    /// Last submitted effective strip values (dim-folded gain, solo-audible
    /// mute) per node; diffs against the candidate drive live submissions.
    strip_live: BTreeMap<NodeId, (f64, bool)>,
    /// Trailing-edge debounce for the automatic apply of structure edits.
    /// Always armed on a structural change: the session is always-on, so a
    /// graph without a running owner re-opens through the same path.
    auto_apply_deadline: Option<Instant>,
    /// The spectrum relay. Levels are not relayed since decision D-4
    /// route B: the level subscription registry lives on the
    /// [`ServiceContext`](super::ServiceContext) and the UI reads its
    /// leases directly.
    spectra: Option<meters::SpectrumBridge>,
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
    pub(crate) fn set_measured_snapshot_reader_for_test(
        &mut self,
        read: impl Fn() -> Option<moiren_windows_audio::engine_backend::NativeObservationSnapshot>
        + Send
        + Sync
        + 'static,
    ) {
        self.measured_snapshot_reader = Some(Arc::new(read));
    }

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
            recording_sessions: BTreeMap::new(),
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
            automatic_resume: false,
            resume_notice: None,
            exiting: false,
            apply: false,
            apply_settings: None,
            spectra: None,
            #[cfg(all(test, windows))]
            measured_snapshot_reader: None,
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
    /// Startup with a restored project resumes it instead of binding the
    /// catalog defaults, which would overwrite the restored devices.
    pub fn with_automatic_resume(mut self) -> Self {
        self.automatic_resume = true;
        self
    }
    #[cfg(windows)]
    pub fn windows() -> Self {
        Self::windows_with_provider(
            moiren_windows_audio::vad::WindowsVadProvider::discover().ok(),
            None,
            None,
        )
    }

    /// Diagnostic-only native Process tap. Both precision factories and every
    /// measured replacement retain this config; saved settings stay unchanged.
    #[cfg(windows)]
    pub fn windows_with_process_pcm_capture(
        config: moiren_windows_audio::clock_bridge::native_pcm::NativePcmConfig,
    ) -> Result<Self, host::BackendError> {
        config
            .validate()
            .map_err(|error| host::BackendError::Unsupported(error.into()))?;
        Ok(Self::windows_with_provider(
            moiren_windows_audio::vad::WindowsVadProvider::discover().ok(),
            Some(config),
            None,
        ))
    }

    /// Opt-in evidence for the monitor's exact owners, including replacements.
    /// Render evidence also times pure-virtual Engine callbacks, which have no
    /// physical render stream to supply native operation envelopes.
    #[cfg(windows)]
    pub fn windows_with_native_evidence(
        process_pcm: Option<moiren_windows_audio::clock_bridge::native_pcm::NativePcmConfig>,
        render_capacity: Option<usize>,
    ) -> Result<Self, host::BackendError> {
        if let Some(config) = process_pcm {
            config
                .validate()
                .map_err(|error| host::BackendError::Unsupported(error.into()))?;
        }
        if render_capacity.is_some_and(|frames| !(1..=131_072).contains(&frames)) {
            return Err(host::BackendError::Unsupported(
                "render evidence capacity must be 1..131072 frames".into(),
            ));
        }
        Ok(Self::windows_with_provider(
            moiren_windows_audio::vad::WindowsVadProvider::discover().ok(),
            process_pcm,
            render_capacity,
        ))
    }

    /// Explicit bounded evidence only. Discovery happens once; the factories,
    /// lifecycle adapter and capture all use that exact provider/control owner.
    #[cfg(windows)]
    pub fn windows_with_input_capture(
        endpoint: moiren_engine::node::VirtualEndpointId,
        frames: u32,
    ) -> Result<(Self, moiren_windows_audio::vad::VadInputCaptureHandle), host::BackendError> {
        let (provider, capture) =
            moiren_windows_audio::vad::WindowsVadProvider::discover_with_input_capture(
                endpoint, frames,
            )?;
        Ok((
            Self::windows_with_provider(Some(provider), None, None),
            capture,
        ))
    }

    #[cfg(windows)]
    fn windows_with_provider(
        virtual_provider: Option<Arc<moiren_windows_audio::vad::WindowsVadProvider>>,
        process_pcm: Option<moiren_windows_audio::clock_bridge::native_pcm::NativePcmConfig>,
        render_evidence: Option<usize>,
    ) -> Self {
        let process_pcm_frames = process_pcm.map(|config| config.capacity_frames);
        let native_observations = Arc::new(std::sync::Mutex::new(None));
        let native_publisher = native_observations.clone();
        let f64_native_publisher = native_observations.clone();
        let professional_observations = Arc::new(std::sync::Mutex::new(None));
        let professional_publisher = professional_observations.clone();
        let f64_publisher = professional_observations.clone();
        let provider_for_factory = virtual_provider.clone();
        let provider_for_f64 = virtual_provider.clone();
        let mut driver = Self::new(|_| unreachable!("Windows factory replaced before use"))
            .with_catalog(Arc::new(WindowsBackend));
        driver.factory = Arc::new(move |_, _, transport, measured| {
            transport
                .validate()
                .map_err(host::BackendError::Unsupported)?;
            let backend = moiren_windows_audio::engine_backend::ProfessionalBackend::<f32>::new()?
                .with_render_queue(transport.render_options())?
                .with_render_evidence(render_evidence)?
                .with_virtual_callback_timing(render_evidence.is_some())
                .with_process_bridge(transport.process_options())?
                .with_process_pcm_capture(process_pcm_frames.map(|capacity_frames| {
                    moiren_windows_audio::clock_bridge::native_pcm::NativePcmConfig {
                        capacity_frames,
                    }
                }))?
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
                .with_render_queue(transport.render_options())?
                .with_render_evidence(render_evidence)?
                .with_virtual_callback_timing(render_evidence.is_some())
                .with_process_bridge(transport.process_options())?
                .with_process_pcm_capture(process_pcm_frames.map(|capacity_frames| {
                    moiren_windows_audio::clock_bridge::native_pcm::NativePcmConfig {
                        capacity_frames,
                    }
                }))?
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
    /// Read-only diagnostic pin supplier. No audio resource or editable DTO
    /// can construct a native collector through this control-only reader.
    #[cfg(windows)]
    pub fn native_observation_reader(
        &self,
    ) -> impl Fn() -> Option<moiren_windows_audio::engine_backend::NativeObservationReadHandle>
    + Send
    + 'static {
        let registry = self.native_observations.clone();
        move || {
            registry.as_ref().and_then(|registry| {
                registry
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .and_then(|registry| registry.retain_session())
            })
        }
    }

    fn queue(&mut self) -> &CompletionQueue<Completion> {
        self.completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()))
    }
}
