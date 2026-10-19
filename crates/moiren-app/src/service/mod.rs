//! Application intent and bounded workers; the service owns the active control
//! endpoint and native session driver, while the UI receives owned snapshots.

mod backend;
mod core;
mod driver;
mod graph;
mod jobs;
mod latency;
mod mailbox;
mod managed;
mod model;
mod parameters;
mod recovery;
#[cfg(test)]
mod settings_tests;
mod signal_preparation;
mod signals;
mod startup;
#[cfg(windows)]
mod windows;
pub use backend::*;
pub use driver::BackendDriver;
pub use graph::{
    BuiltinState, CompressorParameter, CompressorState, GraphEdit, GraphNodeState,
    GraphPreparation, GraphSnapshot, MixerState, ServiceGraph, validate_node_state,
};
pub use latency::AudioLatencyProfile;
pub use managed::ManagedDriver;
pub use moiren_engine::audio_config::*;
pub use signal_preparation::{SignalPreparation, SignalPreparationError};
pub use signals::{LevelTopic, SignalTopics, SubscriptionDeliveryError, SubscriptionTicket};
#[cfg(windows)]
pub use windows::WindowsBackend;

#[derive(Debug, thiserror::Error)]
pub enum ServiceStartError {
    #[error("service or worker spawn failed: {0}")]
    WorkerSpawn(std::io::Error),
    #[error("native application service is only supported on Windows")]
    UnsupportedPlatform,
}
pub fn start_service() -> Result<ServiceRuntime, ServiceStartError> {
    #[cfg(windows)]
    {
        // Automatic catalog/startup can run before the UI is constructed. Seed
        // saved machine-local audio intent before the service starts polling.
        let preferences = crate::project::load_preferences().unwrap_or_default();
        ServiceRuntime::spawn_with_settings(
            ManagedDriver::windows().with_automatic_startup(),
            preferences.audio,
            preferences.latency_profile,
        )
        .map_err(ServiceStartError::WorkerSpawn)
    }
    #[cfg(not(windows))]
    {
        Err(ServiceStartError::UnsupportedPlatform)
    }
}
pub use core::*;
pub use jobs::*;
pub use mailbox::*;
pub use model::*;

use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
pub const MAX_PENDING_JOBS: usize = 16;
/// Fixed management budgets. A4 drains its completion/control/retire channels
/// through these limits; the UI never polls or owns backend objects.
#[derive(Clone, Copy, Debug)]
pub struct RoundBudget {
    pub completions: usize,
    pub applied: usize,
    pub retire: usize,
}
impl Default for RoundBudget {
    fn default() -> Self {
        Self {
            completions: 8,
            applied: 32,
            retire: 8,
        }
    }
}
pub struct ServiceContext {
    #[cfg(windows)]
    pub native_observations: Option<NativeSessionObservations>,
    #[cfg(windows)]
    pub professional_observations: Option<ProfessionalSessionObservations>,
    pub latency_profile: AudioLatencyProfile,
    pub active_latency_profile: Option<AudioLatencyProfile>,
    pub audio_settings: AudioSettings,
    pub active_audio_settings: Option<AudioSettings>,
    pub graph: ServiceGraph,
    pub monitor_latency: Option<MonitorLatencyStatus>,
    pub monitor_diagnostics: Option<MonitorDiagnosticsSnapshot>,
    pub core: ServiceCore,
    pub catalog: Arc<DeviceCatalog>,
    pub catalog_error: Option<String>,
    pub control: ControlSummary,
    pub(crate) signals: signals::SignalService,
    pending: VecDeque<(JobPriority, Job, DeferredPermit)>,
    slots: Arc<AtomicUsize>,
}
impl Default for ServiceContext {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            native_observations: None,
            #[cfg(windows)]
            professional_observations: None,
            latency_profile: AudioLatencyProfile::default(),
            active_latency_profile: None,
            audio_settings: AudioSettings::default(),
            active_audio_settings: None,
            graph: ServiceGraph::default(),
            monitor_latency: None,
            monitor_diagnostics: None,
            core: ServiceCore::default(),
            catalog: Arc::new(DeviceCatalog::default()),
            catalog_error: None,
            control: ControlSummary::default(),
            signals: signals::SignalService::new().expect("fixed application signal topics"),
            pending: VecDeque::with_capacity(MAX_PENDING_JOBS),
            slots: Arc::new(AtomicUsize::new(MAX_PENDING_JOBS)),
        }
    }
}
/// One reserved service storage slot, released after successful worker admission
/// or cancellation. It holds no session and may safely be dropped anywhere.
pub struct DeferredPermit {
    slots: Arc<AtomicUsize>,
}
impl Drop for DeferredPermit {
    fn drop(&mut self) {
        self.slots.fetch_add(1, Ordering::Release);
    }
}
impl ServiceContext {
    /// A complete draft replaces the requested configuration atomically. The
    /// running owner keeps its own snapshot until an explicit Apply or Start.
    pub(crate) fn set_audio_settings(&mut self, settings: AudioSettings) -> bool {
        if let Err(error) = settings.validate() {
            self.graph.record_error(error);
            return false;
        }
        self.audio_settings = settings;
        true
    }

    /// Stale-observation guard shared by every snapshot projection: a value is
    /// published only while the Running phase is owned by its own generation.
    /// The UI's display-level fallback mirrors this shape without the owner
    /// identity, which is not published.
    fn fresh(&self, generation: SessionGeneration) -> bool {
        self.core.phase() == SessionPhase::Running
            && self.core.owner_generation() == Some(generation)
            && self.core.generation() == generation
    }

    /// Refusal preserves ownership; callers must retain an owned session job.
    pub fn try_defer(
        &mut self,
        priority: JobPriority,
        job: Job,
    ) -> Result<(), (DispatchError, Job)> {
        let permit = match self.try_reserve_cleanup() {
            Ok(permit) => permit,
            Err(error) => return Err((error, job)),
        };
        self.pending.push_back((priority, job, permit));
        Ok(())
    }
    /// Reserve one service cleanup slot before submitting preparation. The
    /// reservation travels with the prepared result, so a later replacement or
    /// stop cannot require storage that was already consumed by unrelated jobs.
    pub fn try_reserve_cleanup(&self) -> Result<DeferredPermit, DispatchError> {
        self.slots
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .map_err(|_| DispatchError::Busy)?;
        Ok(DeferredPermit {
            slots: self.slots.clone(),
        })
    }
    pub fn defer_reserved_cleanup(
        &mut self,
        permit: DeferredPermit,
        job: Job,
    ) -> Result<(), (DispatchError, Job)> {
        if !Arc::ptr_eq(&permit.slots, &self.slots) {
            return Err((DispatchError::InvalidConfig, job));
        }
        self.pending.push_back((JobPriority::Cleanup, job, permit));
        Ok(())
    }
    pub fn pending_jobs(&self) -> usize {
        self.pending.len()
    }
    fn dispatch(&mut self, pool: &mut JobPool) {
        let count = self.pending.len();
        for _ in 0..count {
            let (priority, job, permit) = self.pending.pop_front().expect("bounded count");
            if let Err((_, job)) = pool.try_submit_owned(priority, job) {
                self.pending.push_back((priority, job, permit));
            }
        }
    }
}
/// Backend hooks belong to A4. Implementations reserve completion credits before
/// preparing, dispose failed handoffs on the worker, and report drained only
/// after every completion and owned resource has moved to cleanup/reaping.
pub trait ServiceDriver: Send + 'static {
    fn request(&mut self, context: &mut ServiceContext, pool: &mut JobPool, request: AppRequest);
    fn poll(&mut self, _context: &mut ServiceContext, _pool: &mut JobPool, _budget: RoundBudget) {}
    fn begin_exit(&mut self, _context: &mut ServiceContext, _pool: &mut JobPool) {}
    /// True only when no owned completion, preparation or session remains.
    fn is_drained(&self) -> bool;
}
struct CoreDriver;
impl ServiceDriver for CoreDriver {
    fn is_drained(&self) -> bool {
        true
    }
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, r: AppRequest) {
        match r {
            AppRequest::Start(s) => {
                let _ = c.core.start(s);
            }
            AppRequest::Stop => {
                let _ = c.core.stop();
            }
            AppRequest::SetGainPan { gain, pan } => c.control.pending_gain_pan = Some((gain, pan)),
            AppRequest::RefreshCatalog => {}
            AppRequest::SetLatencyProfile(profile) => c.latency_profile = profile,
            AppRequest::SetAudioSettings(settings) => {
                c.set_audio_settings(settings);
            }
            AppRequest::EditGraph(edit) => {
                let _ = c.graph.edit(edit);
            }
            AppRequest::ReplaceGraph { graph, nodes } => {
                if let Err(error) = c.graph.replace(graph, nodes) {
                    c.graph.record_error(error);
                }
            }
            AppRequest::ApplyGraph => {
                c.graph.record_error("未配置原生音频驱动");
            }
        }
    }
}
pub(crate) fn snapshot(c: &ServiceContext) -> AppSnapshot {
    AppSnapshot {
        #[cfg(windows)]
        native_observations: c
            .native_observations
            .clone()
            .filter(|value| c.fresh(value.generation)),
        #[cfg(windows)]
        professional_observations: c
            .professional_observations
            .clone()
            .filter(|value| c.fresh(value.generation)),
        latency_profile: c.latency_profile,
        active_latency_profile: c.active_latency_profile,
        audio_settings: c.audio_settings.clone(),
        active_audio_settings: c
            .active_audio_settings
            .clone()
            .filter(|_| c.fresh(c.core.generation())),
        graph: Arc::new(c.graph.snapshot()),
        monitor_latency: c.monitor_latency.clone(),
        monitor_diagnostics: c
            .monitor_diagnostics
            .filter(|diagnostics| c.fresh(diagnostics.generation)),
        consumed_request_id: RequestId::default(),
        generation: c.core.generation(),
        phase: c.core.phase(),
        desired: c.core.desired().cloned(),
        error: c.core.error().map(str::to_owned),
        catalog: c.catalog.clone(),
        catalog_error: c.catalog_error.clone(),
        control: c.control.clone(),
        signal_topics: c.signals.topics.clone(),
        elapsed: c.core.elapsed(),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceJoinError {
    ThreadPanicked,
}
pub struct ServiceRuntime {
    handle: AppHandle,
    thread: JoinHandle<()>,
}
impl ServiceRuntime {
    pub fn spawn() -> io::Result<Self> {
        Self::spawn_with_driver(CoreDriver)
    }
    pub fn spawn_with_driver(driver: impl ServiceDriver) -> io::Result<Self> {
        Self::spawn_with_settings(
            driver,
            AudioSettings::default(),
            AudioLatencyProfile::default(),
        )
    }
    fn spawn_with_settings(
        mut driver: impl ServiceDriver,
        audio_settings: AudioSettings,
        latency_profile: AudioLatencyProfile,
    ) -> io::Result<Self> {
        audio_settings
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut pool = JobPool::new()?;
        let (hand_tx, hand_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("app-service".into())
            .spawn(move || {
                let mut c = ServiceContext {
                    audio_settings,
                    latency_profile,
                    ..ServiceContext::default()
                };
                let (handle, mailbox) =
                    ServiceMailbox::with_snapshot(thread::current(), snapshot(&c));
                if hand_tx.send(handle).is_err() {
                    return;
                }
                let mut exiting = false;
                loop {
                    let mut handled = 0;
                    if !exiting {
                        for _ in 0..16 {
                            if mailbox.exit_requested() {
                                break;
                            }
                            let Some(request) = mailbox.try_dispatch(&mut c.signals) else {
                                break;
                            };
                            if let Some(request) = request {
                                driver.request(&mut c, &mut pool, request);
                            }
                            handled += 1;
                        }
                    }
                    // A request hook may have yielded while UI exit was set.
                    // Revoke activation before the next backend poll, including
                    // when the draining loop already observed the exit flag.
                    if mailbox.exit_requested() && !exiting {
                        exiting = true;
                        c.core.begin_exit();
                        driver.begin_exit(&mut c, &mut pool);
                    }
                    driver.poll(&mut c, &mut pool, RoundBudget::default());
                    c.dispatch(&mut pool);
                    pool.reap_finished();
                    if exiting
                        && driver.is_drained()
                        && pool.is_idle()
                        && c.pending.is_empty()
                        && c.core.finish_exit()
                    {
                        pool.shutdown();
                    }
                    c.signals.pump_if_due(std::time::Instant::now());
                    mailbox.publish(snapshot(&c));
                    if c.core.phase() == SessionPhase::Exited {
                        break;
                    }
                    if handled == 16 {
                        continue;
                    }
                    let signal_wait = c.signals.wait_duration(std::time::Instant::now());
                    if !driver.is_drained()
                        || c.core.owner_generation().is_some()
                        || !pool.is_idle()
                        || !c.pending.is_empty()
                        || exiting
                        || signal_wait.is_some()
                    {
                        thread::park_timeout(
                            signal_wait
                                .unwrap_or(Duration::from_millis(10))
                                .min(Duration::from_millis(10)),
                        );
                    } else {
                        thread::park();
                    }
                }
            })?;
        let handle = hand_rx
            .recv()
            .map_err(|_| io::Error::other("service failed to initialize"))?;
        Ok(Self { handle, thread })
    }
    pub fn handle(&self) -> AppHandle {
        self.handle.clone()
    }
    /// Call after an Exited snapshot; this is the only UI-side service join.
    pub fn join(self) -> Result<(), ServiceJoinError> {
        self.thread
            .join()
            .map_err(|_| ServiceJoinError::ThreadPanicked)
    }
}
