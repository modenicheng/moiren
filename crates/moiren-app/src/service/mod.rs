//! Application intent and bounded workers; the service owns the active control
//! endpoint and native session driver, while the UI receives owned snapshots.

mod backend;
mod core;
mod exports;
mod graph;
mod jobs;
#[cfg(test)]
mod jobs_tests;
mod latency;
mod mailbox;
mod managed;
mod model;
mod recording;
#[cfg(test)]
mod settings_tests;
mod signals;
mod startup;
#[cfg(any(test, feature = "gpui-test"))]
#[path = "managed/test_backend.rs"]
pub(crate) mod test_backend;
#[cfg(windows)]
mod windows;
pub use backend::*;
pub use exports::*;
use graph::primary_node_of;
pub use graph::{
    BuiltinState, CompressorParameter, CompressorState, GraphEdit, GraphFault, GraphNodeState,
    GraphPreparation, GraphSnapshot, GraphSubject, MixerState, NODE_TINT_COUNT, ServiceGraph,
    clock_master, default_label, validate_node_state,
};
pub use latency::AudioLatencyProfile;
use managed::LevelRegistry;
pub use managed::{LevelLease, LevelSource, ManagedDriver};
pub use moiren_engine::audio_config::*;
pub use recording::{
    DateTimeParts, RecordingCard, RecordingPhase, RecordingService, RecordingSnapshot,
};
pub use signals::{SignalTopics, SpectrumTopic, SubscriptionDeliveryError, SubscriptionTicket};
#[cfg(windows)]
pub use windows::WindowsBackend;

#[derive(Debug, thiserror::Error)]
pub enum ServiceStartError {
    #[error(transparent)]
    Persistence(#[from] crate::project::PersistenceError),
    #[error("invalid startup intent: {0}")]
    InvalidStartup(String),
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
        let startup = crate::project::StartupDocuments::load()?;
        let restored = startup.project.is_some();
        let context = startup
            .into_context()
            .map_err(ServiceStartError::InvalidStartup)?;
        let driver = ManagedDriver::windows();
        // A restored project carries its own device bindings; automatic
        // startup would rebind the catalog defaults over them, so it resumes
        // instead and the engine still comes up on launch.
        let driver = if restored {
            driver.with_automatic_resume()
        } else {
            driver.with_automatic_startup()
        };
        ServiceRuntime::spawn_with_context(driver, context).map_err(ServiceStartError::WorkerSpawn)
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
    pub exports: ExportService,
    pub loaded_project: Option<Arc<crate::project::ProjectDocument>>,
    #[cfg(windows)]
    pub professional_observations: Option<ProfessionalSessionObservations>,
    pub latency_profile: AudioLatencyProfile,
    pub active_latency_profile: Option<AudioLatencyProfile>,
    pub audio_settings: AudioSettings,
    pub active_audio_settings: Option<AudioSettings>,
    pub graph: ServiceGraph,
    pub core: ServiceCore,
    /// Runtime recording state per node; see [`recording`] for why it stays
    /// out of the editable graph domain.
    pub recordings: RecordingService,
    pub catalog: Arc<DeviceCatalog>,
    pub catalog_error: Option<String>,
    /// Content token for the snapshot publisher: bumped whenever the catalog
    /// Arc is swapped for different content (see [`Self::replace_catalog`]).
    catalog_epoch: u64,
    pub control: ControlSummary,
    signals: signals::SignalService,
    /// The level subscription registry (decision D-4 route B): holds the
    /// engine's level subscriptions directly against the leases the UI
    /// holds, replacing the relay's copy machinery.
    levels: LevelRegistry,
    /// Whether the active session routes its levels into the tone topic
    /// instead of the physical monitor pair. The meter bridge resolves it
    /// from the running host graph and retires it with the owner.
    pub tone_session: bool,
    pending: VecDeque<(JobPriority, Job, DeferredPermit)>,
    slots: Arc<AtomicUsize>,
    /// Test observability for the snapshot publisher: counts every
    /// `snapshot(&c)` build so tests can assert the loop skips rebuilds
    /// while no input changes.
    #[cfg(test)]
    snapshot_builds: std::sync::atomic::AtomicU64,
}
impl Default for ServiceContext {
    fn default() -> Self {
        let signals = signals::SignalService::new().expect("fixed application signal topics");
        let levels = LevelRegistry::new(&signals.topics);
        Self {
            exports: ExportService::default(),
            loaded_project: None,
            #[cfg(windows)]
            professional_observations: None,
            latency_profile: AudioLatencyProfile::default(),
            active_latency_profile: None,
            audio_settings: AudioSettings::default(),
            active_audio_settings: None,
            graph: ServiceGraph::default(),
            core: ServiceCore::default(),
            recordings: RecordingService::default(),
            catalog: Arc::new(DeviceCatalog::default()),
            catalog_error: None,
            catalog_epoch: 0,
            control: ControlSummary::default(),
            signals,
            levels,
            tone_session: false,
            pending: VecDeque::with_capacity(MAX_PENDING_JOBS),
            slots: Arc::new(AtomicUsize::new(MAX_PENDING_JOBS)),
            #[cfg(test)]
            snapshot_builds: std::sync::atomic::AtomicU64::new(0),
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
    pub(crate) fn with_settings(
        audio_settings: AudioSettings,
        latency_profile: AudioLatencyProfile,
    ) -> Self {
        Self {
            audio_settings,
            latency_profile,
            ..Self::default()
        }
    }

    /// Decode and validate the entire candidate before publishing any part of
    /// it. Project imports never change the machine-local export authorization.
    pub fn load_project(
        &mut self,
        document: crate::project::ProjectDocument,
    ) -> Result<(), String> {
        document.validate().map_err(|error| error.to_string())?;
        let mut audio = self.audio_settings.clone();
        if let Some(processing) = document.processing {
            audio.processing = processing;
        } else if let Some(session) = &document.session {
            // Design 23 §4.3: old documents carry the block size on the
            // session because the deleted UI ledger used to ferry it into a
            // Start request. Explicit project processing stays authoritative
            // when both forms are present; a session without it migrates.
            audio.processing.block_frames = session.max_block_frames as u32;
        }
        audio.validate().map_err(|error| error.to_string())?;
        if let Some(graph) = &document.graph {
            let restored = document
                .restore_graph()
                .map_err(|error| error.to_string())?;
            debug_assert_eq!(restored.graph.nodes().len(), graph.nodes.len());
            self.graph
                .replace_with_exports(restored.graph, restored.nodes, restored.exports)?;
        } else if let Some(spec) = document.session_spec().map_err(|error| error.to_string())? {
            let mut candidate = ServiceGraph::default();
            // The early-demo run limit is no longer honored (design 23 §7):
            // the migrated bindings start like any other graph.
            candidate.configure_session(&SessionSpec {
                limit: RunLimit::UntilStopped,
                ..spec
            })?;
            let candidate = candidate.snapshot();
            self.graph.replace(candidate.graph, candidate.nodes)?;
        }
        self.audio_settings = audio;
        self.loaded_project = Some(Arc::new(document));
        Ok(())
    }

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

    /// Swap in a refreshed device catalog. The snapshot catalog is never
    /// mutated in place, so the content token advances exactly when the
    /// content differs; a content-equal refresh keeps the old Arc and lets
    /// the publish dedup hold UI identity stable.
    pub(crate) fn replace_catalog(&mut self, catalog: DeviceCatalog) {
        if *self.catalog != catalog {
            self.catalog = Arc::new(catalog);
            self.catalog_epoch = self.catalog_epoch.wrapping_add(1);
        }
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

    /// Test intake for the deferred-job budget: production paths reserve
    /// cleanup storage first (`try_reserve_cleanup`) and submit through the
    /// pool directly. Refusal preserves ownership; callers must retain an
    /// owned session job.
    #[cfg(test)]
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
    /// Drain probe for the tests above; production observes idleness through
    /// `JobPool::is_idle` and the driver's `is_drained` instead.
    #[cfg(test)]
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
            AppRequest::LoadProject(document) => {
                if let Err(error) = c.load_project(document) {
                    c.graph.record_error(error);
                }
            }
            AppRequest::SetExportAuthorizations(values) => {
                if let Err(error) = c.exports.set_authorizations(&values) {
                    c.graph.record_error(error);
                }
            }
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
            AppRequest::UndoGraph => {
                c.graph.undo();
            }
            AppRequest::RedoGraph => {
                c.graph.redo();
            }
            AppRequest::ReplaceGraph { graph, nodes } => {
                if let Err(error) = c.graph.replace(graph, nodes) {
                    c.graph.record_error(error);
                }
            }
            AppRequest::ApplyGraph => {
                c.graph
                    .record_error("No native audio backend is configured");
            }
            AppRequest::StartRecording { .. }
            | AppRequest::PauseRecording { .. }
            | AppRequest::ResumeRecording { .. }
            | AppRequest::StopRecording { .. } => {
                c.graph
                    .record_error("No native audio backend is configured");
            }
        }
    }
}
pub(crate) fn snapshot(c: &ServiceContext) -> AppSnapshot {
    #[cfg(test)]
    c.snapshot_builds
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut graph = c.graph.snapshot();
    // Runtime card state rides the graph projection but never its editing
    // domain: cards repaint on phase transitions only, and the ticking
    // duration stays out of it entirely (graph-page keys must stay 1 Hz-free).
    graph.recording = c.recordings.cards();
    AppSnapshot {
        loaded_project: c.loaded_project.clone(),
        exports: c.exports.statuses(&graph),
        export_plan: c.exports.reconcile(&graph),
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
        graph: Arc::new(graph),
        consumed_request_id: RequestId::default(),
        generation: c.core.generation(),
        phase: c.core.phase(),
        desired: c.core.desired().cloned(),
        error: c.core.error().map(str::to_owned),
        recordings: c.recordings.snapshots(),
        catalog: c.catalog.clone(),
        catalog_error: c.catalog_error.clone(),
        control: c.control.clone(),
        signal_topics: c.signals.topics.clone(),
        tone_session: c.tone_session,
        // The chrome clock renders whole seconds; publishing sub-second
        // precision would change the snapshot on every poll and keep the UI
        // repainting for as long as the session runs.
        elapsed: Duration::from_secs(c.core.elapsed().as_secs()),
    }
}

/// Content tokens for everything [`snapshot`] projects whose construction or
/// comparison is expensive: the graph deep copy, the export statuses/plan
/// projections, the session core fields, and the device catalog. Each token
/// is advanced by the domain's own mutation paths (graph.rs, core.rs,
/// exports.rs/runtime.rs, and `replace_catalog` here), so a change in any of
/// them is guaranteed to be reflected in the next published snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SnapshotRevision {
    graph: u64,
    core: u64,
    exports: u64,
    catalog: u64,
    recordings: u64,
    /// Whole seconds of the session clock: the only snapshot input that
    /// advances on its own (the published elapsed is quantized to seconds,
    /// so the published content moves at 1 Hz at most).
    elapsed_secs: u64,
}

/// Cheap, allocation-free change probe for the tokened snapshot domains.
pub(crate) fn snapshot_revision(c: &ServiceContext) -> SnapshotRevision {
    SnapshotRevision {
        graph: c.graph.state_epoch(),
        core: c.core.state_epoch(),
        exports: c.exports.state_epoch(),
        catalog: c.catalog_epoch,
        recordings: c.recordings.state_epoch(),
        elapsed_secs: c.core.elapsed().as_secs(),
    }
}

/// Whether the small, directly comparable snapshot inputs still match the
/// last published snapshot. These inputs are all cheap `PartialEq` values,
/// so they are compared by value instead of carrying tokens; they never
/// require building the graph deep copy or the export projections.
pub(crate) fn small_snapshot_inputs_match(c: &ServiceContext, last: &AppSnapshot) -> bool {
    let loaded_project_same = match (&last.loaded_project, &c.loaded_project) {
        (None, None) => true,
        (Some(published), Some(current)) => Arc::ptr_eq(published, current),
        _ => false,
    };
    // The published observations/settings pass through freshness filters;
    // project the same view without cloning before comparing.
    let observations_same = {
        #[cfg(windows)]
        {
            let projected = c
                .professional_observations
                .as_ref()
                .filter(|value| c.fresh(value.generation));
            last.professional_observations.as_ref() == projected
        }
        #[cfg(not(windows))]
        {
            true
        }
    };
    let active_settings_same = {
        let projected = c
            .active_audio_settings
            .as_ref()
            .filter(|_| c.fresh(c.core.generation()));
        last.active_audio_settings.as_ref() == projected
    };
    loaded_project_same
        && observations_same
        && active_settings_same
        && last.latency_profile == c.latency_profile
        && last.active_latency_profile == c.active_latency_profile
        && last.audio_settings == c.audio_settings
        && last.catalog_error == c.catalog_error
        && last.control == c.control
        && last.signal_topics == c.signals.topics
        && last.tone_session == c.tone_session
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
        driver: impl ServiceDriver,
        audio_settings: AudioSettings,
        latency_profile: AudioLatencyProfile,
    ) -> io::Result<Self> {
        Self::spawn_with_context(
            driver,
            ServiceContext {
                audio_settings,
                latency_profile,
                ..Default::default()
            },
        )
    }

    pub fn spawn_with_startup(
        driver: impl ServiceDriver,
        startup: crate::project::StartupDocuments,
    ) -> io::Result<Self> {
        let context = startup
            .into_context()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Self::spawn_with_context(driver, context)
    }

    fn spawn_with_context(
        mut driver: impl ServiceDriver,
        mut c: ServiceContext,
    ) -> io::Result<Self> {
        c.audio_settings
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut pool = JobPool::new()?;
        let (hand_tx, hand_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("app-service".into())
            .spawn(move || {
                let (handle, mailbox) =
                    ServiceMailbox::with_snapshot(thread::current(), snapshot(&c));
                // Snapshot publishing is token-gated: `snapshot(&c)` deep-copies
                // the graph and rebuilds the export projections, so it runs only
                // when a content token moved, a directly compared input changed,
                // or a request was consumed (its receipt id is published). The
                // mailbox's own equality dedup stays as the backstop for token
                // bumps whose content turns out equivalent.
                let mut published_revision = snapshot_revision(&c);
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
                    let revision = snapshot_revision(&c);
                    if handled > 0
                        || revision != published_revision
                        || !small_snapshot_inputs_match(&c, &mailbox.last_published())
                    {
                        mailbox.publish(snapshot(&c));
                        published_revision = revision;
                    }
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

#[cfg(test)]
mod snapshot_publish_tests {
    use super::*;

    fn running_context() -> ServiceContext {
        let mut c = ServiceContext::default();
        let spec = SessionSpec {
            source: InputSelection::Physical {
                endpoint_id: "probe-input".into(),
            },
            output_endpoint_id: "probe-output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.0,
            pan: 0.0,
            max_block_frames: 64,
        };
        let generation = c.core.start(spec).expect("probe spec validates");
        c.core.managed_execution(generation, true, None);
        c
    }

    #[test]
    fn snapshot_elapsed_is_quantized_to_whole_seconds() {
        let c = running_context();
        let first = snapshot(&c);
        assert_eq!(first.phase, SessionPhase::Running);
        assert_eq!(
            first.elapsed.subsec_nanos(),
            0,
            "sub-second precision would churn every publish while running"
        );
    }

    #[test]
    fn same_second_snapshots_are_equal_for_publish_dedup() {
        let c = running_context();
        // Fresh graph Arcs must compare by content so an unchanged topology
        // keeps equality across snapshot builds.
        let first = snapshot(&c);
        let second = snapshot(&c);
        assert_eq!(first, second);
    }

    fn probe_session() -> SessionSpec {
        SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "render".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.0,
            pan: 0.0,
            max_block_frames: 256,
        }
    }

    /// Reports the context's snapshot-build counter every service round.
    /// `is_drained == false` keeps the loop cycling on its 10 ms cadence
    /// without any signal demand, so the test observes pure idle rounds.
    struct BuildProbeDriver {
        builds: std::sync::mpsc::SyncSender<u64>,
        exiting: bool,
    }
    impl ServiceDriver for BuildProbeDriver {
        fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
            // The probe applies exactly one domain change so the test can
            // watch a real content change propagate to the publication.
            if let AppRequest::SetLatencyProfile(profile) = request {
                c.latency_profile = profile;
            }
        }
        fn poll(&mut self, c: &mut ServiceContext, _: &mut JobPool, _: RoundBudget) {
            let _ = self
                .builds
                .try_send(c.snapshot_builds.load(std::sync::atomic::Ordering::Relaxed));
        }
        fn begin_exit(&mut self, _: &mut ServiceContext, _: &mut JobPool) {
            self.exiting = true;
        }
        fn is_drained(&self) -> bool {
            // Undrained keeps the loop cycling on its 10 ms cadence without
            // demand; exit must still be able to retire the service.
            self.exiting
        }
    }

    fn collect_rounds(
        receiver: &std::sync::mpsc::Receiver<u64>,
        count: usize,
        timeout: Duration,
    ) -> Vec<u64> {
        let deadline = std::time::Instant::now() + timeout;
        let mut rounds = Vec::with_capacity(count);
        while rounds.len() < count {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or_default();
            assert!(
                !remaining.is_zero(),
                "service loop stopped reporting rounds: {rounds:?}"
            );
            match receiver.recv_timeout(remaining) {
                Ok(builds) => rounds.push(builds),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    panic!("no round report within budget: {rounds:?}")
                }
                Err(error) => panic!("service loop disconnected: {error}: {rounds:?}"),
            }
        }
        rounds
    }

    /// B-1 regression: rounds whose inputs did not change must not rebuild
    /// (and republish) the snapshot at all; a real change must rebuild and
    /// reach the published snapshot.
    #[test]
    fn service_loop_skips_snapshot_rebuilds_while_inputs_unchanged() {
        let (builds_tx, builds_rx) = std::sync::mpsc::sync_channel(256);
        let runtime = ServiceRuntime::spawn_with_driver(BuildProbeDriver {
            builds: builds_tx,
            exiting: false,
        })
        .unwrap();
        let handle = runtime.handle();
        // Steady run: every report carries the same build count. The old
        // build-and-compare loop incremented it on every round.
        let steady = collect_rounds(&builds_rx, 10, Duration::from_secs(5));
        let baseline = steady[0];
        assert!(
            steady.iter().all(|&builds| builds == baseline),
            "unchanged rounds must skip snapshot rebuilds: {steady:?}"
        );
        // A real change: the request round rebuilds exactly once and the
        // published snapshot carries it to the UI.
        handle
            .try_request(AppRequest::SetLatencyProfile(AudioLatencyProfile::Low))
            .unwrap();
        let mut rebuilt = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while rebuilt.is_none() {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or_default();
            assert!(!remaining.is_zero(), "the edit never rebuilt the snapshot");
            match builds_rx.recv_timeout(remaining) {
                Ok(builds) if builds > baseline => rebuilt = Some(builds),
                Ok(_) => continue,
                Err(error) => panic!("service loop disconnected: {error}"),
            }
        }
        let rebuilt = rebuilt.expect("edit rebuild observed");
        // The round's publish lands right after the poll report above, so
        // wait for it instead of racing the snapshot slot read.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "the rebuilt snapshot never reached the UI"
            );
            if handle.try_snapshot().unwrap().latency_profile == AudioLatencyProfile::Low {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // And the loop falls silent again until the next real change.
        let steady = collect_rounds(&builds_rx, 10, Duration::from_secs(5));
        assert!(
            steady.iter().all(|&builds| builds == rebuilt),
            "rounds after the change must skip snapshot rebuilds again: {steady:?}"
        );
        crate::ui::finish_service(runtime);
    }

    /// Every tokened domain must move the revision exactly when its content
    /// can change the published snapshot; idempotent re-projections must not
    /// churn it (the hot per-round paths depend on that).
    #[test]
    fn snapshot_revision_tracks_every_tokened_domain() {
        let mut c = ServiceContext::default();
        let initial = snapshot_revision(&c);
        // A projection that changes nothing (wrong generation) is inert.
        c.core.managed_execution(SessionGeneration(7), true, None);
        assert_eq!(
            snapshot_revision(&c),
            initial,
            "an inert projection must not churn any token"
        );

        // Graph domain: an accepted edit advances the candidate revision.
        let before = snapshot_revision(&c);
        let gain = primary_node_of(
            &c.graph.snapshot_ref().graph,
            moiren_core::graph::NodeKind::Gain,
        )
        .expect("fixed chain gain")
        .id();
        c.graph
            .edit(GraphEdit::SetGain {
                node: gain,
                gain: 0.5,
            })
            .expect("gain edit validates");
        assert_ne!(snapshot_revision(&c), before, "graph edit must bump");

        // Core domain: desired intent and execution projection.
        let before = snapshot_revision(&c);
        c.core
            .managed_desired(probe_session())
            .expect("spec validates");
        assert_ne!(snapshot_revision(&c), before, "desired change must bump");
        let generation = c.core.generation();
        c.core.managed_execution(generation, false, None);
        let starting = snapshot_revision(&c);
        // The driver re-projects the same execution facts every round.
        c.core.managed_execution(generation, false, None);
        assert_eq!(
            snapshot_revision(&c),
            starting,
            "steady re-projection must not churn the core token"
        );
        c.core.managed_execution(generation, true, None);
        assert_ne!(
            snapshot_revision(&c),
            starting,
            "running transition must bump"
        );

        // Exports domain: availability drives every projected resource state.
        let before = snapshot_revision(&c);
        c.exports.set_available(true);
        assert_ne!(snapshot_revision(&c), before, "availability flip must bump");
        let available = snapshot_revision(&c);
        c.exports.set_available(true);
        assert_eq!(
            snapshot_revision(&c),
            available,
            "idempotent availability must not churn"
        );
        c.exports
            .set_authorizations(&[ExportAuthorization {
                endpoint_id: moiren_engine::node::VirtualEndpointId::from_bytes([0x11; 16]),
                allow_default: true,
            }])
            .expect("authorization validates");
        assert_ne!(
            snapshot_revision(&c),
            available,
            "authorization change must bump"
        );

        // Catalog domain: content-equal refresh keeps the token, a real
        // refresh moves it.
        let before = snapshot_revision(&c);
        c.replace_catalog(DeviceCatalog::default());
        assert_eq!(
            snapshot_revision(&c),
            before,
            "content-equal catalog refresh must not churn"
        );
        let catalog = DeviceCatalog {
            default_output_endpoint_id: Some("probe-output".into()),
            ..DeviceCatalog::default()
        };
        c.replace_catalog(catalog);
        assert_ne!(
            snapshot_revision(&c),
            before,
            "catalog content change must bump"
        );
    }

    /// The directly compared snapshot inputs must decide exactly like a full
    /// rebuild followed by the publish dedup: `small_snapshot_inputs_match`
    /// against the published snapshot must agree with `snapshot(&c) ==
    /// published`, including the freshness filters applied at publication.
    #[test]
    fn small_snapshot_inputs_match_tracks_directly_compared_fields() {
        let mut c = ServiceContext::default();
        let published = snapshot(&c);
        let oracle = |c: &ServiceContext, published: &AppSnapshot| {
            small_snapshot_inputs_match(c, published) == (snapshot(c) == *published)
        };
        assert!(oracle(&c, &published), "unchanged context: both agree");

        c.latency_profile = AudioLatencyProfile::Low;
        assert!(!small_snapshot_inputs_match(&c, &published));
        assert!(oracle(&c, &published), "latency profile: both agree");
        let published = snapshot(&c);

        c.control.pending_gain_pan = Some((0.5, 0.0));
        assert!(!small_snapshot_inputs_match(&c, &published));
        assert!(oracle(&c, &published), "control: both agree");
        let published = snapshot(&c);

        c.audio_settings.processing.block_frames = 128;
        assert!(!small_snapshot_inputs_match(&c, &published));
        assert!(oracle(&c, &published), "audio settings: both agree");
        let published = snapshot(&c);

        c.catalog_error = Some("probe failure".into());
        assert!(!small_snapshot_inputs_match(&c, &published));
        assert!(oracle(&c, &published), "catalog error: both agree");
        let published = snapshot(&c);

        c.tone_session = true;
        assert!(!small_snapshot_inputs_match(&c, &published));
        assert!(oracle(&c, &published), "tone session: both agree");

        // The active-settings publication applies the same freshness filter
        // as `snapshot`, so both views must agree through the lifecycle.
        c.core
            .managed_desired(probe_session())
            .expect("spec validates");
        let generation = c.core.generation();
        c.core.managed_execution(generation, true, None);
        assert!(oracle(&c, &published), "running, no active settings: agree");
        c.active_audio_settings = Some(c.audio_settings.clone());
        assert!(!small_snapshot_inputs_match(&c, &published));
        assert!(oracle(&c, &published), "fresh active settings: agree");
        let published = snapshot(&c);
        assert!(small_snapshot_inputs_match(&c, &published));
        c.core.stop().expect("stop validates");
        assert!(
            oracle(&c, &published),
            "stale active settings filtered: agree"
        );
    }
}
