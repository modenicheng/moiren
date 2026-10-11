use super::*;
use std::{
    cell::Cell,
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
    request: AppRequest,
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
            AppRequest::Start(spec) => spec.validate()?,
            AppRequest::SetGainPan { gain, pan } if !model::valid_gain_pan(*gain, *pan) => {
                return Err(DispatchError::InvalidConfig);
            }
            _ => {}
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
            .try_send(QueuedRequest { id, request })
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
    consumed_request_id: Cell<RequestId>,
    exit: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Arc<AppSnapshot>>>,
}
impl ServiceMailbox {
    pub fn new(wake: Thread) -> (AppHandle, Self) {
        let (sender, receiver) = mpsc::sync_channel(REQUEST_CAPACITY);
        let exit = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Arc::new(snapshot(&ServiceContext::default()))));
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
        let queued = self.receiver.try_recv().ok()?;
        self.consumed_request_id.set(queued.id);
        Some(queued.request)
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
}
