//! Platform-neutral application intent, bounded work admission and UI mailbox.
//! Native session and ControlPort driving are supplied by `ServiceDriver`.

mod core;
mod jobs;
mod mailbox;
mod model;
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
    pub core: ServiceCore,
    pub catalog: Arc<DeviceCatalog>,
    pub control: ControlSummary,
    pending: VecDeque<(JobPriority, Job, DeferredPermit)>,
    slots: Arc<AtomicUsize>,
}
impl Default for ServiceContext {
    fn default() -> Self {
        Self {
            core: ServiceCore::default(),
            catalog: Arc::new(DeviceCatalog::default()),
            control: ControlSummary::default(),
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
        }
    }
}
pub(crate) fn snapshot(c: &ServiceContext) -> AppSnapshot {
    AppSnapshot {
        generation: c.core.generation(),
        phase: c.core.phase(),
        desired: c.core.desired().cloned(),
        error: c.core.error().map(str::to_owned),
        catalog: c.catalog.clone(),
        control: c.control.clone(),
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
    pub fn spawn_with_driver(mut driver: impl ServiceDriver) -> io::Result<Self> {
        let mut pool = JobPool::new()?;
        let (hand_tx, hand_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("app-service".into())
            .spawn(move || {
                let (handle, mailbox) = ServiceMailbox::new(thread::current());
                if hand_tx.send(handle).is_err() {
                    return;
                }
                let mut c = ServiceContext::default();
                let mut exiting = false;
                loop {
                    if mailbox.exit_requested() && !exiting {
                        exiting = true;
                        c.core.begin_exit();
                        driver.begin_exit(&mut c, &mut pool);
                    }
                    let mut handled = 0;
                    if !exiting {
                        for _ in 0..16 {
                            if mailbox.exit_requested() {
                                break;
                            }
                            let Some(request) = mailbox.try_request() else {
                                break;
                            };
                            driver.request(&mut c, &mut pool, request);
                            handled += 1;
                        }
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
                    mailbox.publish(snapshot(&c));
                    if c.core.phase() == SessionPhase::Exited {
                        break;
                    }
                    if handled == 16 {
                        continue;
                    }
                    if !driver.is_drained()
                        || c.core.owner_generation().is_some()
                        || !pool.is_idle()
                        || !c.pending.is_empty()
                        || exiting
                    {
                        thread::park_timeout(Duration::from_millis(10));
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
