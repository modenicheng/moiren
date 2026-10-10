use super::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::Thread,
};
const REQUEST_CAPACITY: usize = 64;
#[derive(Clone)]
pub struct AppHandle {
    sender: SyncSender<AppRequest>,
    exit: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Arc<AppSnapshot>>>,
    wake: Thread,
}
impl AppHandle {
    pub fn try_request(&self, request: AppRequest) -> Result<(), DispatchError> {
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
        self.sender.try_send(request).map_err(|e| match e {
            TrySendError::Full(_) => DispatchError::Busy,
            TrySendError::Disconnected(_) => DispatchError::Disconnected,
        })?;
        self.wake.unpark();
        Ok(())
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
    receiver: Receiver<AppRequest>,
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
                exit: exit.clone(),
                snapshot: snapshot.clone(),
                wake,
            },
            Self {
                receiver,
                exit,
                snapshot,
            },
        )
    }
    pub fn exit_requested(&self) -> bool {
        self.exit.load(Ordering::Acquire)
    }
    pub fn try_request(&self) -> Option<AppRequest> {
        self.receiver.try_recv().ok()
    }
    pub(crate) fn publish(&self, value: AppSnapshot) {
        let replacement = Arc::new(value);
        let previous = {
            let mut slot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *slot, replacement)
        };
        drop(previous);
    }
}
