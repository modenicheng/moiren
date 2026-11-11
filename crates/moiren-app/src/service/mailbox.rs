use super::*;
use std::{
    cell::Cell,
    cell::RefCell,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    task::{Context as TaskContext, Poll, Waker},
    thread::Thread,
};
const REQUEST_CAPACITY: usize = 64;
struct QueuedRequest {
    id: RequestId,
    payload: QueuedPayload,
}
enum QueuedPayload {
    Application(Box<AppRequest>),
    Subscription(signals::SubscriptionRequest),
}
/// Publish notification for the UI's control pump (design 22 D7). The
/// service bumps the generation after every snapshot it actually swaps in
/// and wakes whoever waits, so an edit's echo reaches the UI as soon as the
/// service has applied it, not on the pump's next timer tick.
#[derive(Default)]
struct PublishSignal {
    generation: AtomicU64,
    wakers: Mutex<Vec<Waker>>,
}
impl PublishSignal {
    /// Service side, after the snapshot swap: a reader that sees the new
    /// generation also sees the new snapshot. The wakers are taken, not
    /// cloned, so waking costs no allocation on the service thread.
    fn notify(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        let wakers = std::mem::take(&mut *self.wakers.lock().unwrap_or_else(|e| e.into_inner()));
        for waker in wakers {
            waker.wake();
        }
    }
}
/// Resolves once the service has published past `seen`.
struct PublishedAfter {
    signal: Arc<PublishSignal>,
    seen: u64,
}
impl Future for PublishedAfter {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        let published = || self.signal.generation.load(Ordering::Acquire) != self.seen;
        if published() {
            return Poll::Ready(());
        }
        {
            let mut wakers = self.signal.wakers.lock().unwrap_or_else(|e| e.into_inner());
            if !wakers.iter().any(|waker| waker.will_wake(cx.waker())) {
                wakers.push(cx.waker().clone());
            }
        }
        // Check again after registering: a publish between the first check
        // and the registration took the waker list before this waker joined.
        if published() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
#[derive(Clone)]
pub struct AppHandle {
    sender: SyncSender<QueuedRequest>,
    enqueue_id: Arc<Mutex<u64>>,
    exit: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Arc<AppSnapshot>>>,
    publish: Arc<PublishSignal>,
    wake: Thread,
}
impl AppHandle {
    pub fn try_request(&self, request: AppRequest) -> Result<(), DispatchError> {
        self.try_request_tracked(request).map(|_| ())
    }
    /// Serialize ID assignment with successful enqueue, using only try_lock on
    /// the non-RT caller. Concurrent producers therefore cannot acknowledge a
    /// lower-ID request that is still behind a higher-ID request in the queue.
    pub fn try_request_tracked(&self, request: AppRequest) -> Result<RequestId, DispatchError> {
        if self.exit.load(Ordering::Acquire) {
            return Err(DispatchError::Exiting);
        }
        match &request {
            AppRequest::LoadProject(document) => document
                .validate()
                .map_err(|_| DispatchError::InvalidConfig)?,
            AppRequest::SetExportAuthorizations(values) => {
                super::validate_export_authorizations(values)
                    .map_err(|_| DispatchError::InvalidConfig)?
            }
            AppRequest::Start(spec) => spec.validate()?,
            AppRequest::SetAudioSettings(settings) => settings
                .validate()
                .map_err(|_| DispatchError::InvalidConfig)?,
            AppRequest::SetGainPan { gain, pan } if !model::valid_gain_pan(*gain, *pan) => {
                return Err(DispatchError::InvalidConfig);
            }
            _ => {}
        }
        self.try_enqueue(QueuedPayload::Application(Box::new(request)))
    }
    pub fn subscribe_topic<P: moiren_signal::Shape>(
        &self,
        topic: moiren_signal::Topic<P>,
        options: moiren_signal::SubscriptionOptions,
    ) -> Result<SubscriptionTicket<P>, DispatchError> {
        let (ticket, request) = signals::subscription_request(topic, options);
        self.try_enqueue(QueuedPayload::Subscription(request))?;
        Ok(ticket)
    }
    fn try_enqueue(&self, payload: QueuedPayload) -> Result<RequestId, DispatchError> {
        if self.exit.load(Ordering::Acquire) {
            return Err(DispatchError::Exiting);
        }
        let mut sequence = self
            .enqueue_id
            .try_lock()
            .map_err(|_| DispatchError::Busy)?;
        let id = RequestId(
            sequence
                .checked_add(1)
                .ok_or(DispatchError::RequestIdExhausted)?,
        );
        self.sender
            .try_send(QueuedRequest { id, payload })
            .map_err(|e| match e {
                TrySendError::Full(_) => DispatchError::Busy,
                TrySendError::Disconnected(_) => DispatchError::Disconnected,
            })?;
        *sequence = id.0;
        drop(sequence);
        self.wake.unpark();
        Ok(id)
    }
    pub fn request_exit(&self) {
        self.exit.store(true, Ordering::Release);
        self.wake.unpark();
    }
    pub fn try_snapshot(&self) -> Option<Arc<AppSnapshot>> {
        self.snapshot
            .try_lock()
            .ok()
            .map(|value| Arc::clone(&value))
    }
    /// How many snapshots the service has published. A pump reads it before
    /// draining the snapshot and then waits with [`Self::published_after`],
    /// so a publish that lands during the drain cuts the next wait short.
    pub fn publish_generation(&self) -> u64 {
        self.publish.generation.load(Ordering::Acquire)
    }
    /// Resolves once the service publishes past generation `seen` — at once
    /// when it already has. The UI's control pump waits on this next to its
    /// timer.
    pub fn published_after(&self, seen: u64) -> impl Future<Output = ()> + Unpin + 'static {
        PublishedAfter {
            signal: Arc::clone(&self.publish),
            seen,
        }
    }
}
/// Requests and exit are independent so a full queue cannot suppress shutdown.
pub struct ServiceMailbox {
    receiver: Receiver<QueuedRequest>,
    pending: RefCell<Option<QueuedRequest>>,
    consumed_request_id: Cell<RequestId>,
    exit: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Arc<AppSnapshot>>>,
    /// Publish-side dedup state; only the service thread calls [`publish`].
    last_published: RefCell<Option<Arc<AppSnapshot>>>,
    publish_signal: Arc<PublishSignal>,
}
impl ServiceMailbox {
    pub fn new(wake: Thread) -> (AppHandle, Self) {
        Self::with_snapshot(wake, snapshot(&ServiceContext::default()))
    }
    pub(crate) fn with_snapshot(wake: Thread, initial: AppSnapshot) -> (AppHandle, Self) {
        let (sender, receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
        let exit = Arc::new(AtomicBool::new(false));
        let initial = Arc::new(initial);
        let snapshot = Arc::new(Mutex::new(Arc::clone(&initial)));
        let publish_signal = Arc::new(PublishSignal::default());
        (
            AppHandle {
                sender,
                enqueue_id: Arc::new(Mutex::new(0)),
                exit: exit.clone(),
                snapshot: snapshot.clone(),
                publish: publish_signal.clone(),
                wake,
            },
            Self {
                receiver,
                pending: RefCell::new(None),
                consumed_request_id: Cell::new(RequestId::default()),
                exit,
                snapshot,
                last_published: RefCell::new(Some(initial)),
                publish_signal,
            },
        )
    }
    pub fn exit_requested(&self) -> bool {
        self.exit.load(Ordering::Acquire)
    }
    /// The snapshot the publish dedup compares against. The service loop
    /// reads it to skip rebuilds whose content cannot have changed.
    pub(crate) fn last_published(&self) -> Arc<AppSnapshot> {
        self.last_published
            .borrow()
            .as_ref()
            .expect("the publisher always holds a snapshot")
            .clone()
    }
    pub fn try_request(&self) -> Option<AppRequest> {
        let queued = self.next_payload()?;
        match queued.payload {
            QueuedPayload::Application(request) => {
                self.consumed_request_id.set(queued.id);
                Some(*request)
            }
            QueuedPayload::Subscription(payload) => {
                *self.pending.borrow_mut() = Some(QueuedRequest {
                    id: queued.id,
                    payload: QueuedPayload::Subscription(payload),
                });
                None
            }
        }
    }
    pub(crate) fn try_dispatch(
        &self,
        signals: &mut signals::SignalService,
    ) -> Option<Option<AppRequest>> {
        let queued = self.next_payload()?;
        self.consumed_request_id.set(queued.id);
        match queued.payload {
            QueuedPayload::Application(request) => Some(Some(*request)),
            QueuedPayload::Subscription(request) => {
                signals.execute(request);
                Some(None)
            }
        }
    }
    fn next_payload(&self) -> Option<QueuedRequest> {
        self.pending
            .borrow_mut()
            .take()
            .or_else(|| self.receiver.try_recv().ok())
    }
    pub(crate) fn publish(&self, mut value: AppSnapshot) {
        // The service publishes only after handling its drained round. A UI
        // holding an older phase snapshot cannot observe the newer receipt.
        value.consumed_request_id = self.consumed_request_id.get();
        // An unchanged snapshot keeps the previous Arc so the UI's pointer
        // identity poll sees no change and skips its notify; only real events
        // (requests, phase transitions, quantized clock ticks, tick-batched
        // observation refreshes) swap the Arc.
        if self
            .last_published
            .borrow()
            .as_ref()
            .is_some_and(|previous| previous.as_ref() == &value)
        {
            return;
        }
        let replacement = Arc::new(value);
        *self.last_published.borrow_mut() = Some(Arc::clone(&replacement));
        let previous = {
            let mut slot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *slot, replacement)
        };
        self.publish_signal.notify();
        drop(previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts wakes, for driving [`PublishedAfter`] by hand.
    #[derive(Default)]
    struct CountingWaker(std::sync::atomic::AtomicUsize);
    impl std::task::Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn poll_once(future: &mut (impl Future<Output = ()> + Unpin), waker: &Waker) -> Poll<()> {
        Pin::new(future).poll(&mut TaskContext::from_waker(waker))
    }

    #[test]
    fn a_publish_wakes_the_pump_waiting_on_it() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        let counter = Arc::new(CountingWaker::default());
        let waker = Waker::from(counter.clone());
        let seen = handle.publish_generation();
        let mut wait = handle.published_after(seen);
        assert!(poll_once(&mut wait, &waker).is_pending());
        let mut changed = (*handle.try_snapshot().unwrap()).clone();
        changed.phase = SessionPhase::Running;
        mailbox.publish(changed);
        assert_eq!(
            counter.0.load(Ordering::SeqCst),
            1,
            "the publish wakes the waiter"
        );
        assert!(poll_once(&mut wait, &waker).is_ready());
        assert_eq!(handle.publish_generation(), seen + 1);
        // A wait created after the publish, from a generation read before
        // it, resolves without waiting: the pump cannot miss a publish that
        // landed while it drained.
        assert!(poll_once(&mut handle.published_after(seen), &waker).is_ready());
    }

    #[test]
    fn an_unchanged_publish_wakes_nobody() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        let counter = Arc::new(CountingWaker::default());
        let waker = Waker::from(counter.clone());
        let seen = handle.publish_generation();
        let mut wait = handle.published_after(seen);
        assert!(poll_once(&mut wait, &waker).is_pending());
        // Polling again does not register the same waker twice.
        assert!(poll_once(&mut wait, &waker).is_pending());
        mailbox.publish((*handle.try_snapshot().unwrap()).clone());
        assert_eq!(
            counter.0.load(Ordering::SeqCst),
            0,
            "a deduplicated publish is no news"
        );
        assert!(poll_once(&mut wait, &waker).is_pending());
        let mut changed = (*handle.try_snapshot().unwrap()).clone();
        changed.phase = SessionPhase::Running;
        mailbox.publish(changed);
        assert_eq!(counter.0.load(Ordering::SeqCst), 1, "one waker, one wake");
    }

    #[test]
    fn identical_publish_keeps_arc_and_real_change_swaps_it() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        let initial = handle.try_snapshot().unwrap();
        // Same content (including consumed id): the UI's Arc::ptr_eq poll
        // must not observe a change, so the previous Arc is retained.
        mailbox.publish((*initial).clone());
        assert!(Arc::ptr_eq(&initial, &handle.try_snapshot().unwrap()));
        let mut changed = (*initial).clone();
        changed.phase = SessionPhase::Running;
        mailbox.publish(changed);
        let updated = handle.try_snapshot().unwrap();
        assert!(!Arc::ptr_eq(&initial, &updated));
        assert_eq!(updated.phase, SessionPhase::Running);
        // The swapped Arc is itself stable across unchanged republishes.
        mailbox.publish((*updated).clone());
        assert!(Arc::ptr_eq(&updated, &handle.try_snapshot().unwrap()));
    }
    #[test]
    fn invalid_audio_settings_do_not_enter_queue_or_consume_request_id() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        let mut settings = AudioSettings::default();
        for invalid_rate in [0, 32_000, 192_000] {
            settings.processing.sample_rate = invalid_rate;
            assert_eq!(
                handle.try_request_tracked(AppRequest::SetAudioSettings(settings.clone())),
                Err(DispatchError::InvalidConfig)
            );
            assert_eq!(mailbox.try_request(), None);
        }
        settings.processing.sample_rate = 48_000;
        settings.output.buffer_frames = Some(65);
        assert_eq!(
            handle.try_request(AppRequest::SetAudioSettings(settings)),
            Err(DispatchError::InvalidConfig)
        );
        assert_eq!(
            handle.try_request_tracked(AppRequest::SetAudioSettings(AudioSettings::default())),
            Ok(RequestId(1))
        );
    }

    #[test]
    fn lock_and_queue_busy_do_not_consume_ids_and_retry_keeps_enqueue_order() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        let guard = handle.enqueue_id.lock().unwrap();
        assert_eq!(
            handle.try_request_tracked(AppRequest::Stop),
            Err(DispatchError::Busy)
        );
        drop(guard);
        for expected in 1..=REQUEST_CAPACITY as u64 {
            assert_eq!(
                handle.try_request_tracked(AppRequest::RefreshCatalog),
                Ok(RequestId(expected))
            );
        }
        assert_eq!(
            handle.try_request_tracked(AppRequest::Stop),
            Err(DispatchError::Busy)
        );
        assert_eq!(mailbox.try_request(), Some(AppRequest::RefreshCatalog));
        assert_eq!(
            handle.try_request_tracked(AppRequest::Stop),
            Ok(RequestId(65))
        );
        for _ in 1..REQUEST_CAPACITY {
            mailbox.try_request().unwrap();
        }
        mailbox.publish((*handle.try_snapshot().unwrap()).clone());
        assert_eq!(
            handle.try_snapshot().unwrap().consumed_request_id,
            RequestId(64)
        );
        assert_eq!(mailbox.try_request(), Some(AppRequest::Stop));
        let before_publish = handle.try_snapshot().unwrap();
        assert_eq!(before_publish.consumed_request_id, RequestId(64));
        mailbox.publish((*before_publish).clone());
        assert_eq!(
            handle.try_snapshot().unwrap().consumed_request_id,
            RequestId(65)
        );
        handle.request_exit();
        assert_eq!(
            handle.try_request_tracked(AppRequest::Stop),
            Err(DispatchError::Exiting)
        );
    }
    #[test]
    fn exhausted_request_ids_never_wrap_or_enqueue() {
        let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
        *handle.enqueue_id.lock().unwrap() = u64::MAX;
        assert_eq!(
            handle.try_request_tracked(AppRequest::Stop),
            Err(DispatchError::RequestIdExhausted)
        );
        assert_eq!(mailbox.try_request(), None);
        handle.request_exit();
        assert!(mailbox.exit_requested());
    }

    #[test]
    fn compatibility_reader_preserves_typed_subscription_for_dispatch() {
        let mut context = ServiceContext::default();
        let (handle, mailbox) =
            ServiceMailbox::with_snapshot(std::thread::current(), snapshot(&context));
        let topic = handle
            .try_snapshot()
            .unwrap()
            .signal_topics
            .tone_output
            .clone();
        let mut ticket = handle.subscribe_topic(topic, Default::default()).unwrap();
        assert_eq!(mailbox.try_request(), None);
        assert_eq!(
            handle.try_snapshot().unwrap().consumed_request_id,
            RequestId(0)
        );
        assert_eq!(mailbox.try_dispatch(&mut context.signals), Some(None));
        assert!(ticket.try_take().unwrap().is_some());
        mailbox.publish(snapshot(&context));
        assert_eq!(
            handle.try_snapshot().unwrap().consumed_request_id,
            RequestId(1)
        );
    }
}
