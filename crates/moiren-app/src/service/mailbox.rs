use super::*;
use std::{
    cell::Cell,
    cell::RefCell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
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
#[derive(Clone)]
pub struct AppHandle {
    sender: SyncSender<QueuedRequest>,
    enqueue_id: Arc<Mutex<u64>>,
    exit: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Arc<AppSnapshot>>>,
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
}
/// Requests and exit are independent so a full queue cannot suppress shutdown.
pub struct ServiceMailbox {
    receiver: Receiver<QueuedRequest>,
    pending: RefCell<Option<QueuedRequest>>,
    consumed_request_id: Cell<RequestId>,
    exit: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Arc<AppSnapshot>>>,
}
impl ServiceMailbox {
    pub fn new(wake: Thread) -> (AppHandle, Self) {
        Self::with_snapshot(wake, snapshot(&ServiceContext::default()))
    }
    pub(crate) fn with_snapshot(wake: Thread, initial: AppSnapshot) -> (AppHandle, Self) {
        let (sender, receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
        let exit = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Arc::new(initial)));
        (
            AppHandle {
                sender,
                enqueue_id: Arc::new(Mutex::new(0)),
                exit: exit.clone(),
                snapshot: snapshot.clone(),
                wake,
            },
            Self {
                receiver,
                pending: RefCell::new(None),
                consumed_request_id: Cell::new(RequestId::default()),
                exit,
                snapshot,
            },
        )
    }
    pub fn exit_requested(&self) -> bool {
        self.exit.load(Ordering::Acquire)
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
        let replacement = Arc::new(value);
        let previous = {
            let mut slot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *slot, replacement)
        };
        drop(previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
