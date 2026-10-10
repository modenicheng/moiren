use moiren_app::service::*;
use std::{
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};
#[test]
fn full_mailbox_cannot_lose_exit() {
    let (h, m) = ServiceMailbox::new(thread::current());
    for _ in 0..64 {
        h.try_request(AppRequest::RefreshCatalog).unwrap();
    }
    assert_eq!(h.try_request(AppRequest::Stop), Err(DispatchError::Busy));
    h.request_exit();
    assert!(m.exit_requested());
    assert_eq!(h.try_request(AppRequest::Stop), Err(DispatchError::Exiting));
}
#[test]
fn cleanup_is_reserved_while_prepare_is_blocked() {
    let mut pool = JobPool::new().unwrap();
    let gate = Arc::new(Barrier::new(2));
    let block = gate.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    pool.try_submit(
        JobPriority::Prepare,
        Box::new(move || {
            entered_tx.send(()).unwrap();
            block.wait();
        }),
    )
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    pool.try_submit(
        JobPriority::Cleanup,
        Box::new(move || done_tx.send(()).unwrap()),
    )
    .unwrap();
    done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    gate.wait();
}
struct DropProbe(mpsc::Sender<thread::ThreadId>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.send(thread::current().id()).unwrap();
    }
}
#[test]
fn refused_owned_jobs_are_returned_without_drop() {
    let mut pool = JobPool::new().unwrap();
    let gate = Arc::new(Barrier::new(2));
    let block = gate.clone();
    pool.try_submit(
        JobPriority::Cleanup,
        Box::new(move || {
            block.wait();
        }),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let owned = DropProbe(tx);
    let job: Job = Box::new(move || drop(owned));
    let (error, job) = pool
        .try_submit_owned(JobPriority::Cleanup, job)
        .err()
        .unwrap();
    assert_eq!(error, DispatchError::Busy);
    assert!(rx.try_recv().is_err());
    gate.wait();
    let (ui_id, worker_rx) = {
        let (tx, rx) = mpsc::channel();
        let t = thread::spawn(move || {
            tx.send(thread::current().id()).unwrap();
            job();
        });
        t.join().unwrap();
        (rx.recv().unwrap(), rx)
    };
    drop(worker_rx);
    assert_eq!(rx.recv().unwrap(), ui_id);
    assert_ne!(ui_id, thread::current().id());
}
#[test]
fn completion_credits_bound_and_failed_handoff_disposes_on_worker() {
    let queue = CompletionQueue::<DropProbe>::new(thread::current());
    let port = queue.port();
    let mut permits = Vec::new();
    for _ in 0..8 {
        permits.push(port.try_reserve().unwrap());
    }
    assert!(matches!(port.try_reserve(), Err(DispatchError::Busy)));
    assert_eq!(queue.outstanding(), 8);
    let permit = permits.pop().unwrap();
    drop(queue);
    let (tx, rx) = mpsc::channel();
    let (worker_tx, worker_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        worker_tx.send(thread::current().id()).unwrap();
        permit.send_or_dispose(DropProbe(tx), drop);
    });
    worker.join().unwrap();
    assert_eq!(rx.recv().unwrap(), worker_rx.recv().unwrap());
    drop(permits);
    assert!(port.try_reserve().is_ok());
}
fn spec() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone { frequency_hz: 440. },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 0.05,
        pan: 0.,
        max_block_frames: 256,
    }
}
struct AckDriver {
    ack: mpsc::Sender<SessionGeneration>,
}
impl ServiceDriver for AckDriver {
    fn is_drained(&self) -> bool {
        true
    }
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, r: AppRequest) {
        if let AppRequest::Start(s) = r {
            let g = c.core.start(s).unwrap();
            self.ack.send(g).unwrap();
        }
    }
}
#[test]
fn unread_snapshots_do_not_stall_generation_and_exit() {
    let (tx, rx) = mpsc::channel();
    let runtime = ServiceRuntime::spawn_with_driver(AckDriver { ack: tx }).unwrap();
    let h = runtime.handle();
    for _ in 0..3 {
        h.try_request(AppRequest::Start(spec())).unwrap();
    }
    for n in 1..=3 {
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            SessionGeneration(n)
        );
    }
    h.request_exit();
    runtime.join().unwrap();
    assert_eq!(h.try_snapshot().unwrap().phase, SessionPhase::Exited);
}
#[test]
fn deferred_jobs_have_fixed_capacity_and_preserve_refusals() {
    let mut c = ServiceContext::default();
    for _ in 0..MAX_PENDING_JOBS {
        c.try_defer(JobPriority::Cleanup, Box::new(|| {}))
            .unwrap_or_else(|_| panic!("space"));
    }
    let (tx, rx) = mpsc::channel();
    let probe = DropProbe(tx);
    let (_, job) = c
        .try_defer(JobPriority::Cleanup, Box::new(move || drop(probe)))
        .err()
        .unwrap();
    assert!(rx.try_recv().is_err());
    thread::spawn(job).join().unwrap();
    assert_ne!(rx.recv().unwrap(), thread::current().id());
}
#[test]
fn all_sixty_four_queued_requests_are_drained_in_bounded_rounds() {
    let (tx, rx) = mpsc::channel();
    let runtime = ServiceRuntime::spawn_with_driver(AckDriver { ack: tx }).unwrap();
    let h = runtime.handle();
    for _ in 0..64 {
        h.try_request(AppRequest::Start(spec())).unwrap();
    }
    for n in 1..=64 {
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            SessionGeneration(n)
        );
    }
    h.request_exit();
    runtime.join().unwrap();
}
struct BlockedAckDriver {
    gate: Arc<Barrier>,
    entered: mpsc::Sender<()>,
    ack: mpsc::Sender<SessionGeneration>,
}
impl ServiceDriver for BlockedAckDriver {
    fn is_drained(&self) -> bool {
        true
    }
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, r: AppRequest) {
        match r {
            AppRequest::RefreshCatalog => {
                self.entered.send(()).unwrap();
                self.gate.wait();
            }
            AppRequest::Start(s) => {
                self.ack.send(c.core.start(s).unwrap()).unwrap();
            }
            _ => {}
        }
    }
}
#[test]
fn bounded_rounds_continue_until_backlog_is_empty() {
    let gate = Arc::new(Barrier::new(2));
    let (tx, rx) = mpsc::channel();
    let (etx, erx) = mpsc::channel();
    let runtime = ServiceRuntime::spawn_with_driver(BlockedAckDriver {
        gate: gate.clone(),
        entered: etx,
        ack: tx,
    })
    .unwrap();
    let h = runtime.handle();
    h.try_request(AppRequest::RefreshCatalog).unwrap();
    erx.recv_timeout(Duration::from_secs(2)).unwrap();
    for _ in 0..64 {
        h.try_request(AppRequest::Start(spec())).unwrap();
    }
    gate.wait();
    let mut observed = 0;
    while observed < 64 {
        if rx.recv_timeout(Duration::from_secs(1)).is_err() {
            break;
        }
        observed += 1;
    }
    h.request_exit();
    runtime.join().unwrap();
    assert_eq!(observed, 64);
}
struct OwnedPrepared {
    probe: DropProbe,
    slot: DeferredPermit,
}
struct DisposalDriver {
    queue: Option<CompletionQueue<JobResult<Result<OwnedPrepared, OwnedPrepared>>>>,
    gate: Arc<Barrier>,
    started: mpsc::Sender<SessionGeneration>,
    dropped: mpsc::Sender<thread::ThreadId>,
    worker: mpsc::Sender<thread::ThreadId>,
    fail: bool,
    exit_seen: Option<mpsc::Sender<()>>,
}
impl ServiceDriver for DisposalDriver {
    fn request(&mut self, c: &mut ServiceContext, pool: &mut JobPool, r: AppRequest) {
        if let AppRequest::Start(s) = r {
            let generation = c.core.start(s).unwrap();
            if self.queue.is_none() {
                let queue = CompletionQueue::new(thread::current());
                let permit = queue.port().try_reserve().unwrap();
                let slot = c.try_reserve_cleanup().unwrap();
                self.queue = Some(queue);
                let gate = self.gate.clone();
                let dropped = self.dropped.clone();
                let worker = self.worker.clone();
                let fail = self.fail;
                pool.try_submit_owned(
                    JobPriority::Prepare,
                    Box::new(move || {
                        worker.send(thread::current().id()).unwrap();
                        gate.wait();
                        let probe = OwnedPrepared {
                            probe: DropProbe(dropped),
                            slot,
                        };
                        permit.send_or_dispose(
                            JobResult {
                                generation,
                                value: if fail { Err(probe) } else { Ok(probe) },
                            },
                            drop,
                        );
                    }),
                )
                .unwrap_or_else(|_| panic!("prepare admitted"));
            }
            self.started.send(generation).unwrap();
        }
    }
    fn poll(&mut self, c: &mut ServiceContext, _: &mut JobPool, b: RoundBudget) {
        if let Some(queue) = &self.queue {
            for _ in 0..b.completions {
                let Some(result) = queue.try_recv() else {
                    break;
                };
                if result.value.is_err() {
                    c.core.failed(result.generation, "prepare failed".into());
                }
                assert!(!c.core.may_activate(result.generation));
                let owned = match result.value {
                    Ok(owned) | Err(owned) => owned,
                };
                c.defer_reserved_cleanup(owned.slot, Box::new(move || drop(owned.probe)))
                    .unwrap_or_else(|_| panic!("reserved cleanup storage"));
            }
        }
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, _: &mut JobPool) {
        assert_eq!(c.core.phase(), SessionPhase::Exiting);
        if let Some(ack) = self.exit_seen.take() {
            ack.send(()).unwrap();
        }
    }
    fn is_drained(&self) -> bool {
        self.queue.as_ref().is_none_or(|q| q.outstanding() == 0)
    }
}
fn disposed_stale_or_failed(fail: bool) {
    let gate = Arc::new(Barrier::new(2));
    let (stx, srx) = mpsc::channel();
    let (dtx, drx) = mpsc::channel();
    let (wtx, wrx) = mpsc::channel();
    let runtime = ServiceRuntime::spawn_with_driver(DisposalDriver {
        queue: None,
        gate: gate.clone(),
        started: stx,
        dropped: dtx,
        worker: wtx,
        fail,
        exit_seen: None,
    })
    .unwrap();
    let h = runtime.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    assert_eq!(
        srx.recv_timeout(Duration::from_secs(2)).unwrap(),
        SessionGeneration(1)
    );
    let prepare_thread = wrx.recv_timeout(Duration::from_secs(2)).unwrap();
    h.try_request(AppRequest::Start(spec())).unwrap();
    assert_eq!(
        srx.recv_timeout(Duration::from_secs(2)).unwrap(),
        SessionGeneration(2)
    );
    gate.wait();
    let disposal_thread = drx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_ne!(disposal_thread, thread::current().id());
    assert_ne!(disposal_thread, prepare_thread);
    h.request_exit();
    runtime.join().unwrap();
    let snapshot = h.try_snapshot().unwrap();
    assert_eq!(snapshot.generation, SessionGeneration(2));
    assert_eq!(snapshot.error, None);
    assert_eq!(snapshot.phase, SessionPhase::Exited);
}
#[test]
fn stale_results_are_disposed_on_cleanup_worker() {
    disposed_stale_or_failed(false);
}
#[test]
fn failed_stale_results_are_disposed_on_cleanup_worker() {
    disposed_stale_or_failed(true);
}
#[test]
fn reserved_cleanup_survives_full_service_storage() {
    let mut c = ServiceContext::default();
    let reserved = c.try_reserve_cleanup().unwrap();
    for _ in 0..15 {
        c.try_defer(JobPriority::Prepare, Box::new(|| {}))
            .unwrap_or_else(|_| panic!("space"));
    }
    assert!(matches!(c.try_reserve_cleanup(), Err(DispatchError::Busy)));
    c.defer_reserved_cleanup(reserved, Box::new(|| {}))
        .unwrap_or_else(|_| panic!("reserved space"));
    assert_eq!(c.pending_jobs(), 16);
}
#[test]
fn exit_waits_for_blocked_prepare_result_and_cleanup() {
    let gate = Arc::new(Barrier::new(2));
    let (stx, srx) = mpsc::channel();
    let (dtx, drx) = mpsc::channel();
    let (wtx, wrx) = mpsc::channel();
    let (etx, erx) = mpsc::channel();
    let runtime = ServiceRuntime::spawn_with_driver(DisposalDriver {
        queue: None,
        gate: gate.clone(),
        started: stx,
        dropped: dtx,
        worker: wtx,
        fail: false,
        exit_seen: Some(etx),
    })
    .unwrap();
    let h = runtime.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    srx.recv_timeout(Duration::from_secs(2)).unwrap();
    wrx.recv_timeout(Duration::from_secs(2)).unwrap();
    h.request_exit();
    erx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(drx.try_recv().is_err());
    gate.wait();
    assert_ne!(
        drx.recv_timeout(Duration::from_secs(2)).unwrap(),
        thread::current().id()
    );
    runtime.join().unwrap();
    assert_eq!(h.try_snapshot().unwrap().phase, SessionPhase::Exited);
}
struct ExitDuringStartDriver {
    gate: Arc<Barrier>,
    entered: mpsc::Sender<SessionGeneration>,
    first_poll: Option<mpsc::Sender<(SessionPhase, bool, usize)>>,
    generation: Option<SessionGeneration>,
    exit_count: Arc<std::sync::atomic::AtomicUsize>,
    resource: Option<DropProbe>,
    cleaned_tx: mpsc::Sender<()>,
    cleaned_rx: mpsc::Receiver<()>,
    drained: bool,
}
impl ServiceDriver for ExitDuringStartDriver {
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        if let AppRequest::Start(spec) = request {
            let generation = c.core.start(spec).unwrap();
            self.generation = Some(generation);
            self.entered.send(generation).unwrap();
            self.gate.wait();
        }
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, pool: &mut JobPool) {
        assert_eq!(c.core.phase(), SessionPhase::Exiting);
        self.exit_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let resource = self.resource.take().unwrap();
        let cleaned = self.cleaned_tx.clone();
        pool.try_submit_owned(
            JobPriority::Cleanup,
            Box::new(move || {
                drop(resource);
                cleaned.send(()).unwrap();
            }),
        )
        .unwrap_or_else(|_| panic!("cleanup admitted"));
    }
    fn poll(&mut self, c: &mut ServiceContext, _: &mut JobPool, _: RoundBudget) {
        if let Some(generation) = self.generation
            && let Some(first_poll) = self.first_poll.take()
        {
            first_poll
                .send((
                    c.core.phase(),
                    c.core.may_activate(generation),
                    self.exit_count.load(std::sync::atomic::Ordering::SeqCst),
                ))
                .unwrap();
        }
        if self.cleaned_rx.try_recv().is_ok() {
            self.drained = true;
        }
    }
    fn is_drained(&self) -> bool {
        self.drained
    }
}
#[test]
fn exit_during_request_revokes_activation_before_first_poll() {
    let gate = Arc::new(Barrier::new(2));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (poll_tx, poll_rx) = mpsc::channel();
    let (drop_tx, drop_rx) = mpsc::channel();
    let (cleaned_tx, cleaned_rx) = mpsc::channel();
    let exit_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runtime = ServiceRuntime::spawn_with_driver(ExitDuringStartDriver {
        gate: gate.clone(),
        entered: entered_tx,
        first_poll: Some(poll_tx),
        generation: None,
        exit_count: exit_count.clone(),
        resource: Some(DropProbe(drop_tx)),
        cleaned_tx,
        cleaned_rx,
        drained: false,
    })
    .unwrap();
    let h = runtime.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    assert_eq!(
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        SessionGeneration(1)
    );
    h.request_exit();
    gate.wait();
    let first_poll = poll_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let drop_thread = drop_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    runtime.join().unwrap();
    assert_eq!(first_poll, (SessionPhase::Exiting, false, 1));
    assert_eq!(exit_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_ne!(drop_thread, thread::current().id());
    assert_eq!(h.try_snapshot().unwrap().phase, SessionPhase::Exited);
}
