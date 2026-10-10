use super::DispatchError;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle, Thread},
};
pub type Job = Box<dyn FnOnce() + Send>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobPriority {
    Prepare,
    Cleanup,
}
struct Lane {
    sender: Option<SyncSender<Job>>,
    busy: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Lane {
    fn new(name: &str) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let busy = Arc::new(AtomicBool::new(false));
        let flag = busy.clone();
        let thread = thread::Builder::new().name(name.into()).spawn(move || {
            while let Ok(job) = receiver.recv() {
                // Unwinding disposes job captures on this worker and releases
                // completion reservations; keep the lane usable afterward.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                flag.store(false, Ordering::Release);
            }
        })?;
        Ok(Self {
            sender: Some(sender),
            busy,
            thread: Some(thread),
        })
    }
    fn submit(&self, job: Job) -> Result<(), (DispatchError, Job)> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err((DispatchError::Busy, job));
        }
        let Some(sender) = &self.sender else {
            self.busy.store(false, Ordering::Release);
            return Err((DispatchError::Disconnected, job));
        };
        match sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.busy.store(false, Ordering::Release);
                match e {
                    TrySendError::Full(job) => Err((DispatchError::Busy, job)),
                    TrySendError::Disconnected(job) => Err((DispatchError::Disconnected, job)),
                }
            }
        }
    }
}
/// Two fixed lanes reserve an entire worker for cleanup even when prepare blocks.
/// Each lane admits only one queued or executing job. Never put owned sessions
/// into the convenience API: use `try_submit_owned` and retain refusals instead.
pub struct JobPool {
    prepare: Lane,
    cleanup: Lane,
}
impl JobPool {
    pub fn new() -> io::Result<Self> {
        let prepare = Lane::new("app-prepare")?;
        match Lane::new("app-cleanup") {
            Ok(cleanup) => Ok(Self { prepare, cleanup }),
            Err(e) => {
                drop(prepare.sender);
                if let Some(t) = prepare.thread {
                    let _ = t.join();
                }
                Err(e)
            }
        }
    }
    pub fn try_submit(&mut self, priority: JobPriority, job: Job) -> Result<(), DispatchError> {
        self.try_submit_owned(priority, job)
            .map_err(|(error, _)| error)
    }
    pub fn try_submit_owned(
        &mut self,
        priority: JobPriority,
        job: Job,
    ) -> Result<(), (DispatchError, Job)> {
        match priority {
            JobPriority::Prepare => &self.prepare,
            JobPriority::Cleanup => &self.cleanup,
        }
        .submit(job)
    }
    pub fn is_idle(&self) -> bool {
        !self.prepare.busy.load(Ordering::Acquire) && !self.cleanup.busy.load(Ordering::Acquire)
    }
    /// Close and join fixed workers after all submitted jobs have drained.
    pub fn shutdown(&mut self) {
        self.prepare.sender.take();
        self.cleanup.sender.take();
        for lane in [&mut self.prepare, &mut self.cleanup] {
            if let Some(t) = lane.thread.take() {
                let _ = t.join();
            }
        }
    }
    pub fn reap_finished(&mut self) {
        for lane in [&mut self.prepare, &mut self.cleanup] {
            if lane.thread.as_ref().is_some_and(|t| t.is_finished())
                && let Some(t) = lane.thread.take()
            {
                let _ = t.join();
            }
        }
    }
}
impl Drop for JobPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}
/// Reserve completion storage before dispatching an owned prepare/cleanup job.
/// Reservations plus queued results share the fixed eight-slot budget.
pub struct CompletionQueue<T> {
    receiver: Receiver<T>,
    port: CompletionPort<T>,
}
pub struct CompletionPort<T> {
    sender: SyncSender<T>,
    credits: Arc<AtomicUsize>,
    wake: Thread,
}
impl<T> Clone for CompletionPort<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            credits: self.credits.clone(),
            wake: self.wake.clone(),
        }
    }
}
pub struct CompletionPermit<T> {
    port: CompletionPort<T>,
    used: bool,
}
impl<T> CompletionQueue<T> {
    pub fn new(wake: Thread) -> Self {
        let (sender, receiver) = mpsc::sync_channel(8);
        Self {
            receiver,
            port: CompletionPort {
                sender,
                credits: Arc::new(AtomicUsize::new(8)),
                wake,
            },
        }
    }
    pub fn port(&self) -> CompletionPort<T> {
        self.port.clone()
    }
    pub fn try_recv(&self) -> Option<T> {
        let item = self.receiver.try_recv().ok()?;
        self.port.credits.fetch_add(1, Ordering::Release);
        Some(item)
    }
    pub fn outstanding(&self) -> usize {
        8 - self.port.credits.load(Ordering::Acquire)
    }
}
impl<T> CompletionPort<T> {
    pub fn try_reserve(&self) -> Result<CompletionPermit<T>, DispatchError> {
        self.credits
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .map_err(|_| DispatchError::Busy)?;
        Ok(CompletionPermit {
            port: self.clone(),
            used: false,
        })
    }
}
impl<T> CompletionPermit<T> {
    /// On receiver failure ownership returns to the worker, which must dispose it.
    pub fn send(mut self, item: T) -> Result<(), T> {
        match self.port.sender.try_send(item) {
            Ok(()) => {
                self.used = true;
                self.port.wake.unpark();
                Ok(())
            }
            Err(TrySendError::Disconnected(item) | TrySendError::Full(item)) => Err(item),
        }
    }
    pub fn send_or_dispose(self, item: T, dispose: impl FnOnce(T)) {
        if let Err(item) = self.send(item) {
            dispose(item);
        }
    }
}
impl<T> Drop for CompletionPermit<T> {
    fn drop(&mut self) {
        if !self.used {
            self.port.credits.fetch_add(1, Ordering::Release);
            self.port.wake.unpark();
        }
    }
}
/// Every backend completion carries the generation that produced its owned value.
#[derive(Debug)]
pub struct JobResult<T> {
    pub generation: super::SessionGeneration,
    pub value: T,
}
