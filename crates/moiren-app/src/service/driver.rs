//! Generation checks and resource handoff; no blocking session destruction here.
use super::*;
use crate::monitor::{
    MonitorError, MonitorLatencyConfig, MonitorLatencyProfile, MonitorRecoveryPolicy,
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

struct SessionCredits {
    graph_revision: u64,
    deferred: DeferredPermit,
    completion: CompletionPermit<Completion>,
    signals: SignalPreparation,
}
/// The legacy driver cannot honor professional audio settings; every refusal
/// shares one message so the recovery hint never drifts.
const PROFESSIONAL_SETTINGS_UNSUPPORTED: &str =
    "legacy audio driver cannot apply professional audio settings; use the managed engine driver";
enum Completion {
    Prepared {
        generation: SessionGeneration,
        result: Result<Box<dyn PreparedSession>, BackendError>,
        credits: SessionCredits,
    },
    Reaped {
        generation: SessionGeneration,
        owner: bool,
        report: Result<SessionReport, BackendError>,
    },
    Catalog(Result<DeviceCatalog, BackendError>),
}
struct Owner {
    generation: SessionGeneration,
    session: Box<dyn ActiveSession>,
    credits: SessionCredits,
    recovery: Option<recovery::OwnerRecovery>,
}
struct Reaping {
    generation: SessionGeneration,
    status: SessionEnd,
    error: Option<String>,
    replies: std::vec::IntoIter<moiren_core::protocol::ControlReply>,
}
enum Cleanup {
    Prepared(Box<dyn PreparedSession>),
    Active(Box<dyn ActiveSession>),
    SignalsOnly,
}
impl Cleanup {
    fn join(self) -> Result<SessionReport, BackendError> {
        match self {
            Self::Prepared(s) => s.join(),
            Self::Active(s) => s.join(),
            Self::SignalsOnly => Ok(SessionReport::stopped()),
        }
    }
}
pub struct BackendDriver {
    candidate_identity_matches_session: bool,
    desired_graph: Option<GraphPreparation>,
    monitor_latency: Option<MonitorLatencyConfig>,
    recovery_policy: Option<MonitorRecoveryPolicy>,
    backend: Arc<dyn BackendAdapter>,
    completions: Option<CompletionQueue<Completion>>,
    preparing: Option<(SessionGeneration, Cancellation)>,
    pending_prepare: Option<Job>,
    active: Option<Owner>,
    cleaning: usize,
    catalog: CatalogRefresher,
    automatic_startup: bool,
    startup_retry_at: Option<Instant>,
    exiting: bool,
    refused_cleanup: Option<Job>,
    parameters: parameters::Parameters,
    reaping: Option<Reaping>,
}
impl BackendDriver {
    pub fn new(backend: Arc<dyn BackendAdapter>) -> Self {
        Self {
            candidate_identity_matches_session: false,
            desired_graph: None,
            monitor_latency: None,
            recovery_policy: None,
            backend,
            completions: None,
            preparing: None,
            pending_prepare: None,
            active: None,
            cleaning: 0,
            catalog: CatalogRefresher::new(true),
            automatic_startup: false,
            startup_retry_at: None,
            exiting: false,
            refused_cleanup: None,
            parameters: parameters::Parameters::default(),
            reaping: None,
        }
    }
    /// The desktop starts a muted, unbounded default-device session after the
    /// first usable catalog. Explicit user intent always takes precedence.
    pub fn with_automatic_startup(mut self) -> Self {
        self.automatic_startup = true;
        self
    }
    pub fn with_monitor_latency(
        mut self,
        latency: MonitorLatencyConfig,
        recovery: Option<MonitorRecoveryPolicy>,
    ) -> Result<Self, MonitorError> {
        latency.validate()?;
        if let Some(policy) = recovery {
            policy.validate()?;
        }
        self.monitor_latency = Some(latency);
        self.recovery_policy = recovery;
        Ok(self)
    }
    fn queue(&mut self) -> &CompletionQueue<Completion> {
        self.completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()))
    }
    fn cancel_prepare(&self, c: &mut ServiceContext) {
        if let Some((g, cancel)) = &self.preparing
            && (self.exiting || *g != c.core.generation() || c.core.desired().is_none())
            && let Err(error) = cancel.cancel()
        {
            c.core.start_failed(*g, error);
        }
    }
    /// Ownership-transfer block shared by Start and ApplyGraph: the candidate
    /// becomes the desired topology and all startup bookkeeping resets.
    /// Restart the legacy session from the current candidate — the shared
    /// body of ApplyGraph and the automatic apply of structural edits.
    fn apply_candidate(&mut self, c: &mut ServiceContext) {
        match c
            .exports
            .preflight_audio(&c.graph.snapshot().exports)
            .and_then(|()| c.graph.session_spec(c.core.desired()))
        {
            Ok(spec) => match c.core.start(spec) {
                Ok(_) => self.session_started(c),
                Err(error) => c.graph.record_error(format!("{error:?}")),
            },
            Err(error) => c.graph.record_error(error),
        }
    }
    fn session_started(&mut self, c: &mut ServiceContext) {
        self.candidate_identity_matches_session = true;
        c.graph.clear_pending();
        self.desired_graph = Some(c.graph.preparation());
        self.automatic_startup = false;
        self.startup_retry_at = None;
        c.monitor_latency = None;
        self.parameters.clear_intent(&mut c.control);
    }
    /// Bookkeeping reset shared by Stop and begin_exit, after each has dropped
    /// its own session-specific state.
    fn session_stopped(&mut self, c: &mut ServiceContext) {
        c.graph.clear_pending();
        self.automatic_startup = false;
        self.startup_retry_at = None;
        self.parameters.clear_intent(&mut c.control);
    }
    fn cleanup(
        &mut self,
        c: &mut ServiceContext,
        generation: SessionGeneration,
        owner: bool,
        session: Cleanup,
        credits: SessionCredits,
    ) {
        self.cleaning += 1;
        let SessionCredits {
            graph_revision: _,
            deferred,
            completion,
            signals,
        } = credits;
        let job: Job = Box::new(move || {
            // If an adapter panics, captured ownership still unwinds on this
            // worker and the completion terminalizes its accepted ledger.
            let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.join()))
                .unwrap_or_else(|_| Err(BackendError::new("join", "session cleanup panicked")));
            drop(signals);
            completion.send_or_dispose(
                Completion::Reaped {
                    generation,
                    owner,
                    report,
                },
                drop,
            );
        });
        // The permit was reserved before preparation and belongs to this same
        // context. Refusal would violate that construction invariant.
        if let Err((_, job)) = c.defer_reserved_cleanup(deferred, job) {
            // Even an invalid reservation must not destroy the owned closure
            // here. Keep it until ordinary deferred storage becomes available.
            assert!(self.refused_cleanup.is_none());
            self.refused_cleanup = Some(job);
        }
    }
    fn prepare(&mut self, c: &mut ServiceContext, pool: &mut JobPool) {
        if let Some(job) = self.pending_prepare.take() {
            if let Err((_, job)) = pool.try_submit_owned(JobPriority::Prepare, job) {
                self.pending_prepare = Some(job);
            }
            return;
        }
        if self.exiting
            || self.preparing.is_some()
            || self.active.is_some()
            || self.cleaning != 0
            || c.core.owner_generation().is_some()
        {
            return;
        }
        let generation = c.core.generation();
        if !c.core.may_activate(generation) {
            return;
        }
        let Some(spec) = c.core.desired().cloned() else {
            return;
        };
        let graph = self
            .desired_graph
            .clone()
            .unwrap_or_else(|| c.graph.preparation());
        let graph_revision = graph.revision;
        let latency = if matches!(spec.source, InputSelection::Physical { .. }) {
            self.monitor_latency.map(|requested| {
                c.monitor_latency
                    .get_or_insert(MonitorLatencyStatus {
                        requested,
                        selected: requested,
                        fallback: None,
                    })
                    .selected
            })
        } else {
            None
        };
        let fade_in_frames = c
            .monitor_latency
            .as_ref()
            .and_then(|status| status.fallback)
            .filter(|fallback| fallback.to_generation == generation)
            .and(self.recovery_policy)
            .map_or(0, |policy| policy.fade_frames);
        let Ok(deferred) = c.try_reserve_cleanup() else {
            return;
        };
        let port = self.queue().port();
        let Ok(completion) = port.try_reserve() else {
            return;
        };
        let Ok(cleanup_completion) = port.try_reserve() else {
            return;
        };
        let cancellation = match Cancellation::new() {
            Ok(token) => token,
            Err(error) => {
                c.core.start_failed(generation, error);
                return;
            }
        };
        let token = cancellation.clone();
        let backend = self.backend.clone();
        let mut signals = match c.signals.prepare_session(&spec.source) {
            Ok(signals) => signals,
            Err(error) => {
                c.core
                    .start_failed(generation, BackendError::new("signal host", error));
                return;
            }
        };
        let job: Job = Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                backend.prepare_graph(spec, token, &mut signals, &graph, latency, fade_in_frames)
            }))
            .unwrap_or_else(|_| Err(BackendError::new("prepare", "backend panicked")));
            completion.send_or_dispose(
                Completion::Prepared {
                    generation,
                    result,
                    credits: SessionCredits {
                        graph_revision,
                        deferred,
                        completion: cleanup_completion,
                        signals,
                    },
                },
                |result| {
                    if let Completion::Prepared {
                        result, credits, ..
                    } = result
                    {
                        if let Ok(session) = result {
                            let _ = session.cancel();
                            let _ = session.join();
                        }
                        drop(credits);
                    }
                },
            );
        });
        self.preparing = Some((generation, cancellation));
        c.graph.preparing(generation, graph_revision);
        if let Err((_, job)) = pool.try_submit_owned(JobPriority::Prepare, job) {
            self.pending_prepare = Some(job);
        }
    }
    fn catalog(&mut self, pool: &mut JobPool) {
        let completions = self
            .completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()));
        self.catalog.poll(
            self.exiting,
            Some(&self.backend),
            completions,
            pool,
            Completion::Catalog,
        );
    }
    fn failure_generation(c: &ServiceContext, owner: SessionGeneration) -> SessionGeneration {
        c.monitor_latency
            .as_ref()
            .and_then(|status| status.fallback)
            .filter(|fallback| {
                fallback.from_generation == owner && fallback.to_generation == c.core.generation()
            })
            .map_or(owner, |fallback| fallback.to_generation)
    }
    fn recover_monitor(&mut self, c: &mut ServiceContext) {
        let Some(owner) = self.active.as_mut() else {
            return;
        };
        if owner.generation != c.core.generation()
            || c.core.phase() != SessionPhase::Running
            || owner.session.is_finished()
        {
            return;
        }
        let Some(recovery) = owner.recovery.as_mut() else {
            return;
        };
        let result = recovery.poll(owner.session.as_ref());
        match result {
            Err(error) => {
                c.core.start_failed(owner.generation, error);
            }
            Ok(Some((reason, fade_timed_out))) => {
                let Some(mut spec) = c.core.desired().cloned() else {
                    return;
                };
                let Some(limit) = recovery::remaining_limit(spec.limit, c.core.elapsed()) else {
                    return;
                };
                spec.limit = limit;
                match c.core.start(spec) {
                    Ok(generation) => {
                        let status = c
                            .monitor_latency
                            .as_mut()
                            .expect("configured physical monitor");
                        status.selected = MonitorLatencyConfig::compatible_fallback();
                        status.fallback = Some(MonitorFallback {
                            from_generation: owner.generation,
                            to_generation: generation,
                            reason,
                            fade_timed_out,
                        });
                    }
                    Err(error) => {
                        c.core.start_failed(
                            owner.generation,
                            BackendError::new("monitor fallback", format!("{error:?}")),
                        );
                    }
                }
            }
            Ok(None) => {}
        }
    }
}
impl ServiceDriver for BackendDriver {
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        match request {
            AppRequest::SetExportAuthorizations(values) => {
                if let Err(error) = c.exports.set_authorizations(&values) {
                    c.graph.record_error(error);
                }
            }
            AppRequest::LoadProject(document) => match c.load_project(document) {
                Ok(()) => self.candidate_identity_matches_session = false,
                Err(error) => c.graph.record_error(error),
            },
            AppRequest::SetLatencyProfile(profile) => c.latency_profile = profile,
            AppRequest::SetAudioSettings(settings) => {
                if c.set_audio_settings(settings) && c.audio_settings != AudioSettings::default() {
                    c.graph.record_error(PROFESSIONAL_SETTINGS_UNSUPPORTED);
                }
            }
            AppRequest::Start(spec) => {
                if let Err(error) = c.exports.preflight_audio(&c.graph.snapshot().exports) {
                    c.graph.record_error(error);
                    return;
                }
                if c.audio_settings != AudioSettings::default() {
                    c.graph.record_error(PROFESSIONAL_SETTINGS_UNSUPPORTED);
                    return;
                }
                if let Err(error) = c.graph.configure_session(&spec) {
                    c.graph.record_error(error);
                    return;
                }
                if c.core.start(spec).is_ok() {
                    self.session_started(c);
                }
            }
            AppRequest::Stop => {
                c.active_audio_settings = None;
                if c.core.stop().is_ok() {
                    self.desired_graph = None;
                    c.graph.stopped();
                    self.session_stopped(c);
                }
            }
            AppRequest::RefreshCatalog => self.catalog.request(),
            AppRequest::EditGraph(edit) => {
                // The legacy session exposes only node-addressed control
                // bindings. Edge- and strip-targeted values (SetSend,
                // SetMixer) have no live mapping here, so any such edit
                // rebuilds the running session automatically; the generation
                // lifecycle coalesces bursts. The managed driver consumes the
                // full canonical mapping instead.
                let legacy_rebuild = matches!(
                    edit,
                    super::GraphEdit::SetMixer { .. } | super::GraphEdit::SetSend { .. }
                );
                let parameter = match graph::edit_parameter_values(&edit).as_slice() {
                    [(moiren_engine::host::ParameterTarget::Node { node, parameter }, value)] => {
                        Some((*node, *parameter, *value))
                    }
                    _ => None,
                };
                if let Ok(structural) = c.graph.edit(edit)
                    && self.candidate_identity_matches_session
                    && c.core.phase() == SessionPhase::Running
                    && (structural || legacy_rebuild)
                    && c.audio_settings == AudioSettings::default()
                {
                    self.apply_candidate(c);
                } else if self.candidate_identity_matches_session
                    && c.core.phase() == SessionPhase::Running
                    && let Some((node, parameter, value)) = parameter
                    && let Some(owner) = self.active.as_mut()
                    && owner.generation == c.core.generation()
                    && let Some((_, bindings)) = owner.session.control()
                    && bindings.node(node).is_some()
                {
                    if self
                        .parameters
                        .set_node_value(&mut c.control, node, parameter, value)
                    {
                        if let Some((gain, pan)) = self
                            .desired_graph
                            .as_ref()
                            .and_then(GraphPreparation::gain_pan_nodes)
                        {
                            if node == gain && parameter == moiren_engine::processor::Gain::LEVEL {
                                self.parameters.clear_primary_intent(&mut c.control, 0);
                            }
                            if node == pan && parameter == moiren_engine::processor::Pan::POSITION {
                                self.parameters.clear_primary_intent(&mut c.control, 1);
                            }
                        }
                        // Recovery must preserve the latest admitted value,
                        // without accidentally adopting unapplied topology edits.
                        if let Some(prep) = self.desired_graph.as_mut()
                            && let Some(state) =
                                prep.nodes.iter_mut().find(|state| state.id == node)
                        {
                            match (parameter, value) {
                                (parameter, moiren_core::protocol::ParamValue::Float(value))
                                    if prep.graph.get_node(node).is_ok_and(|node| {
                                        node.kind() == moiren_core::graph::NodeKind::Compressor
                                    }) =>
                                {
                                    if let Some(parameter) = super::CompressorParameter::ALL
                                        .into_iter()
                                        .find(|entry| entry.id() == parameter)
                                    {
                                        state.compressor.set(parameter, value);
                                    }
                                }
                                (
                                    moiren_engine::processor::Gain::LEVEL,
                                    moiren_core::protocol::ParamValue::Float(value),
                                ) if prep.graph.get_node(node).is_ok_and(|node| {
                                    node.kind() == moiren_core::graph::NodeKind::Gain
                                }) =>
                                {
                                    state.gain = value
                                }
                                (
                                    moiren_engine::processor::Pan::POSITION,
                                    moiren_core::protocol::ParamValue::Float(value),
                                ) => state.pan = value,
                                (
                                    moiren_engine::processor::Pan::MONO,
                                    moiren_core::protocol::ParamValue::Bool(value),
                                ) => state.mono = value,
                                (
                                    moiren_engine::processor::Pan::SWAP,
                                    moiren_core::protocol::ParamValue::Bool(value),
                                ) => state.swap = value,
                                _ => {}
                            }
                        }
                    } else {
                        c.graph
                            .record_error("实时参数队列繁忙；候选值已保留，可稍后重试或应用图");
                    }
                }
            }
            AppRequest::ReplaceGraph { graph, nodes } => match c.graph.replace(graph, nodes) {
                Ok(()) => self.candidate_identity_matches_session = false,
                Err(error) => c.graph.record_error(error),
            },
            AppRequest::ApplyGraph if c.audio_settings != AudioSettings::default() => {
                c.graph.record_error(PROFESSIONAL_SETTINGS_UNSUPPORTED);
            }
            AppRequest::ApplyGraph => self.apply_candidate(c),
            AppRequest::SetGainPan { gain, pan } if model::valid_gain_pan(gain, pan) => {
                if !self.candidate_identity_matches_session {
                    c.graph
                        .record_error("请先应用载入的候选项目，再调整该项目的实时参数");
                    return;
                }
                c.core.update_gain_pan(gain, pan);
                if let Some(prep) = self.desired_graph.as_mut()
                    && let Some((gain_node, pan_node)) = prep.gain_pan_nodes()
                {
                    self.parameters.clear_node_intent(
                        &mut c.control,
                        gain_node,
                        moiren_engine::processor::Gain::LEVEL,
                    );
                    self.parameters.clear_node_intent(
                        &mut c.control,
                        pan_node,
                        moiren_engine::processor::Pan::POSITION,
                    );
                    let _ = c.graph.edit(GraphEdit::SetGain {
                        node: gain_node,
                        gain,
                    });
                    let _ = c.graph.edit(GraphEdit::SetPan {
                        node: pan_node,
                        pan,
                    });
                    if let Some(node) = prep.nodes.iter_mut().find(|n| n.id == gain_node) {
                        node.gain = gain;
                    }
                    if let Some(node) = prep.nodes.iter_mut().find(|n| n.id == pan_node) {
                        node.pan = pan;
                    }
                }
                self.parameters.set(&mut c.control, gain, pan)
            }
            AppRequest::SetGainPan { .. } => {}
        }
        self.cancel_prepare(c);
    }
    fn poll(&mut self, c: &mut ServiceContext, pool: &mut JobPool, budget: RoundBudget) {
        if self.automatic_startup && self.startup_retry_at.is_some_and(|at| Instant::now() >= at) {
            self.startup_retry_at = None;
            self.catalog.request();
        }
        if let Some(job) = self.refused_cleanup.take()
            && let Err((_, job)) = c.try_defer(JobPriority::Cleanup, job)
        {
            self.refused_cleanup = Some(job);
        }
        self.cancel_prepare(c);
        for _ in 0..budget.completions {
            let Some(result) = self.queue().try_recv() else {
                break;
            };
            match result {
                Completion::Prepared {
                    generation,
                    result,
                    credits,
                } => {
                    self.preparing = None;
                    match result {
                        Err(error) => {
                            c.graph.preparation_failed(generation, &error);
                            c.core.start_failed(generation, error);
                            self.cleanup(c, generation, false, Cleanup::SignalsOnly, credits);
                        }
                        Ok(session) if !c.core.may_activate(generation) => {
                            let _ = session.cancel();
                            self.cleanup(c, generation, false, Cleanup::Prepared(session), credits);
                        }
                        Ok(session) => match session.activate() {
                            Ok(()) => {
                                assert!(c.core.activated(generation));
                                self.active = Some(Owner {
                                    generation,
                                    session: session.into_active(),
                                    credits,
                                    recovery: c
                                        .monitor_latency
                                        .as_ref()
                                        .filter(|status| {
                                            status.selected.profile
                                                != MonitorLatencyProfile::Compatible
                                        })
                                        .and(self.recovery_policy)
                                        .map(recovery::OwnerRecovery::new),
                                });
                            }
                            Err(error) => {
                                c.graph.preparation_failed(generation, &error);
                                c.core.start_failed(generation, error);
                                let _ = session.cancel();
                                self.cleanup(
                                    c,
                                    generation,
                                    false,
                                    Cleanup::Prepared(session),
                                    credits,
                                );
                            }
                        },
                    }
                }
                Completion::Reaped {
                    generation,
                    owner,
                    report,
                } => {
                    self.cleaning -= 1;
                    if owner {
                        let report = report.unwrap_or_else(|error| SessionReport {
                            status: SessionEnd::Failed,
                            error: Some(error.to_string()),
                            control_replies: Vec::new(),
                        });
                        self.reaping = Some(Reaping {
                            generation,
                            status: report.status,
                            error: report.error,
                            replies: report.control_replies.into_iter(),
                        });
                    }
                }
                Completion::Catalog(result) => {
                    self.catalog.finished();
                    match result {
                        Ok(catalog) => {
                            c.catalog = Arc::new(catalog);
                            c.catalog_error = None;
                            if self.automatic_startup {
                                if c.core.generation() != SessionGeneration(0) || self.exiting {
                                    self.automatic_startup = false;
                                } else if let Some(spec) = startup::default_session(&c.catalog) {
                                    self.automatic_startup = false;
                                    self.request(c, pool, AppRequest::Start(spec));
                                } else {
                                    // Known divergence: only the legacy driver
                                    // retries automatic startup (1 s cadence)
                                    // until a usable catalog appears; managed
                                    // startup does not retry.
                                    self.startup_retry_at =
                                        Some(Instant::now() + Duration::from_secs(1));
                                }
                            }
                        }
                        Err(error) => {
                            c.catalog_error = Some(error.to_string());
                            if self.automatic_startup {
                                self.startup_retry_at =
                                    Some(Instant::now() + Duration::from_secs(1));
                            }
                        }
                    }
                }
            }
        }
        let mut applied = budget.applied.min(32);
        if let Some(reaping) = self.reaping.as_mut()
            && self
                .parameters
                .reaped(&mut c.control, &mut reaping.replies, &mut applied)
        {
            let reaping = self.reaping.take().expect("reaping owner");
            if reaping.status == SessionEnd::Failed {
                let generation = Self::failure_generation(c, reaping.generation);
                c.core.start_failed(
                    generation,
                    BackendError::new(
                        "session",
                        reaping.error.unwrap_or_else(|| "owner failed".into()),
                    ),
                );
            } else {
                // The completed owner's unaccepted sliders have no remaining
                // target. A replacement generation may already have newer
                // intent, which must survive this older owner's finalization.
                if c.core.generation() == reaping.generation
                    && c.core.owner_generation() == Some(reaping.generation)
                {
                    self.parameters.clear_intent(&mut c.control);
                }
                c.core.owner_completed(reaping.generation);
            }
            c.core.owner_reaped(reaping.generation);
            c.graph.stopped();
        }
        if let Some(owner) = self.active.as_ref() {
            let stale = self.exiting
                || owner.generation != c.core.generation()
                || c.core.desired().is_none();
            let started = owner.session.poll_started();
            // Completion is monotonic. Sample it after the Start observation
            // so a natural completion racing this poll is resolved by its join
            // report rather than prematurely classified as a startup failure.
            let finished = owner.session.is_finished();
            if !stale && !finished && started == StartedState::Running {
                c.core.owner_started(owner.generation);
                c.active_audio_settings = Some(AudioSettings::default());
                c.graph
                    .running(owner.generation, owner.credits.graph_revision);
            }
            if !stale && !finished && started == StartedState::Failed {
                c.graph
                    .preparation_failed(owner.generation, "native owner failed");
                c.core.start_failed(
                    owner.generation,
                    BackendError::new("start", "native owner failed"),
                );
            }
            if stale || finished || started == StartedState::Failed {
                c.active_audio_settings = None;
                c.graph.stopped();
                let owner = self.active.take().expect("observed owner");
                c.core.owner_stopping(owner.generation);
                if let Err(error) = owner.session.request_stop() {
                    c.core
                        .start_failed(Self::failure_generation(c, owner.generation), error);
                }
                self.cleanup(
                    c,
                    owner.generation,
                    true,
                    Cleanup::Active(owner.session),
                    owner.credits,
                );
            }
        }
        self.recover_monitor(c);
        if let Some(owner) = self.active.as_mut() {
            self.parameters
                .poll(owner.session.as_mut(), &mut c.control, applied);
        }
        self.prepare(c, pool);
        self.catalog(pool);
        c.monitor_diagnostics =
            self.active
                .as_ref()
                .filter(|owner| {
                    owner.generation == c.core.generation()
                        && c.core.phase() == SessionPhase::Running
                        && c.core.desired().is_some_and(|spec| {
                            matches!(spec.source, InputSelection::Physical { .. })
                        })
                })
                .and_then(|owner| {
                    owner.session.monitor_observation().map(|observation| {
                        MonitorDiagnosticsSnapshot {
                            generation: owner.generation,
                            observation,
                        }
                    })
                });
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, _: &mut JobPool) {
        self.exiting = true;
        c.active_audio_settings = None;
        c.graph.stopped();
        self.catalog.cancel();
        self.session_stopped(c);
        self.cancel_prepare(c);
    }
    fn is_drained(&self) -> bool {
        !self.automatic_startup
            && self.reaping.is_none()
            && self.parameters.is_drained()
            && self.preparing.is_none()
            && self.pending_prepare.is_none()
            && self.active.is_none()
            && self.cleaning == 0
            && self.refused_cleanup.is_none()
            && !self.catalog.is_running()
            && self
                .completions
                .as_ref()
                .is_none_or(|q| q.outstanding() == 0)
    }
}
