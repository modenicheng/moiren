//! Generation checks and resource handoff; no blocking session destruction here.
use super::*;
use std::{sync::Arc, thread};

struct SessionCredits {
    deferred: DeferredPermit,
    completion: CompletionPermit<Completion>,
}
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
}
impl Cleanup {
    fn join(self) -> Result<SessionReport, BackendError> {
        match self {
            Self::Prepared(s) => s.join(),
            Self::Active(s) => s.join(),
        }
    }
}
pub struct BackendDriver {
    backend: Arc<dyn BackendAdapter>,
    completions: Option<CompletionQueue<Completion>>,
    preparing: Option<(SessionGeneration, Cancellation)>,
    active: Option<Owner>,
    cleaning: usize,
    catalog_running: bool,
    refresh: bool,
    exiting: bool,
    refused_cleanup: Option<Job>,
    parameters: parameters::Parameters,
    reaping: Option<Reaping>,
}
impl BackendDriver {
    pub fn new(backend: Arc<dyn BackendAdapter>) -> Self {
        Self {
            backend,
            completions: None,
            preparing: None,
            active: None,
            cleaning: 0,
            catalog_running: false,
            refresh: true,
            exiting: false,
            refused_cleanup: None,
            parameters: parameters::Parameters::default(),
            reaping: None,
        }
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
    fn cleanup(
        &mut self,
        c: &mut ServiceContext,
        generation: SessionGeneration,
        owner: bool,
        session: Cleanup,
        credits: SessionCredits,
    ) {
        self.cleaning += 1;
        let job: Job = Box::new(move || {
            // If an adapter panics, captured ownership still unwinds on this
            // worker and the completion terminalizes its accepted ledger.
            let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.join()))
                .unwrap_or_else(|_| Err(BackendError::new("join", "session cleanup panicked")));
            credits.completion.send_or_dispose(
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
        if let Err((_, job)) = c.defer_reserved_cleanup(credits.deferred, job) {
            // Even an invalid reservation must not destroy the owned closure
            // here. Keep it until ordinary deferred storage becomes available.
            assert!(self.refused_cleanup.is_none());
            self.refused_cleanup = Some(job);
        }
    }
    fn prepare(&mut self, c: &mut ServiceContext, pool: &mut JobPool) {
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
        let job: Job = Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                backend.prepare(spec, token)
            }))
            .unwrap_or_else(|_| Err(BackendError::new("prepare", "backend panicked")));
            completion.send_or_dispose(
                Completion::Prepared {
                    generation,
                    result,
                    credits: SessionCredits {
                        deferred,
                        completion: cleanup_completion,
                    },
                },
                |result| {
                    if let Completion::Prepared {
                        result: Ok(session),
                        ..
                    } = result
                    {
                        let _ = session.cancel();
                        let _ = session.join();
                    }
                },
            );
        });
        match pool.try_submit_owned(JobPriority::Prepare, job) {
            Ok(()) => self.preparing = Some((generation, cancellation)),
            Err((_, job)) => drop(job), // Contains only DTOs and credits, no session.
        }
    }
    fn catalog(&mut self, pool: &mut JobPool) {
        if self.exiting || !self.refresh || self.catalog_running {
            return;
        }
        let Ok(credit) = self.queue().port().try_reserve() else {
            return;
        };
        let backend = self.backend.clone();
        let job: Job = Box::new(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.catalog()))
                    .unwrap_or_else(|_| Err(BackendError::new("catalog", "backend panicked")));
            credit.send_or_dispose(Completion::Catalog(result), drop);
        });
        match pool.try_submit_owned(JobPriority::Prepare, job) {
            Ok(()) => {
                self.catalog_running = true;
                self.refresh = false;
            }
            Err((_, job)) => drop(job),
        }
    }
}
impl ServiceDriver for BackendDriver {
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        match request {
            AppRequest::Start(spec) => {
                if c.core.start(spec).is_ok() {
                    self.parameters.clear_intent(&mut c.control);
                }
            }
            AppRequest::Stop => {
                if c.core.stop().is_ok() {
                    self.parameters.clear_intent(&mut c.control);
                }
            }
            AppRequest::RefreshCatalog => self.refresh = true,
            AppRequest::SetGainPan { gain, pan } if model::valid_gain_pan(gain, pan) => {
                self.parameters.set(&mut c.control, gain, pan)
            }
            AppRequest::SetGainPan { .. } => {}
        }
        self.cancel_prepare(c);
    }
    fn poll(&mut self, c: &mut ServiceContext, pool: &mut JobPool, budget: RoundBudget) {
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
                            c.core.start_failed(generation, error);
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
                                });
                            }
                            Err(error) => {
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
                    self.catalog_running = false;
                    if let Ok(catalog) = result {
                        c.catalog = Arc::new(catalog);
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
                c.core.start_failed(
                    reaping.generation,
                    BackendError::new(
                        "session",
                        reaping.error.unwrap_or_else(|| "owner failed".into()),
                    ),
                );
            } else {
                c.core.owner_completed(reaping.generation);
            }
            c.core.owner_reaped(reaping.generation);
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
            }
            if !stale && !finished && started == StartedState::Failed {
                c.core.start_failed(
                    owner.generation,
                    BackendError::new("start", "native owner failed"),
                );
            }
            if stale || finished || started == StartedState::Failed {
                let owner = self.active.take().expect("observed owner");
                c.core.owner_stopping(owner.generation);
                if let Err(error) = owner.session.request_stop() {
                    c.core.start_failed(owner.generation, error);
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
        if let Some(owner) = self.active.as_mut() {
            self.parameters
                .poll(owner.session.as_mut(), &mut c.control, applied);
        }
        self.prepare(c, pool);
        self.catalog(pool);
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, _: &mut JobPool) {
        self.exiting = true;
        self.refresh = false;
        self.parameters.clear_intent(&mut c.control);
        self.cancel_prepare(c);
    }
    fn is_drained(&self) -> bool {
        self.reaping.is_none()
            && self.parameters.is_drained()
            && self.preparing.is_none()
            && self.active.is_none()
            && self.cleaning == 0
            && self.refused_cleanup.is_none()
            && !self.catalog_running
            && self
                .completions
                .as_ref()
                .is_none_or(|q| q.outstanding() == 0)
    }
}
