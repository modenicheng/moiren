//! Single-use control-to-owner handoff before activation. Stop always wins.
use crate::StopSignal;
use std::{
    os::windows::io::{AsRawHandle, OwnedHandle},
    sync::{Arc, Mutex, mpsc},
};
use windows::Win32::{
    Foundation::{E_ABORT, HANDLE, WAIT_OBJECT_0},
    System::Threading::{INFINITE, WaitForMultipleObjects},
};

pub(crate) struct Sender<T> {
    channel: mpsc::SyncSender<T>,
    event: StopSignal,
    closed: Arc<Mutex<bool>>,
}
pub(crate) struct Receiver<T> {
    channel: mpsc::Receiver<T>,
    event: Arc<OwnedHandle>,
    closed: Arc<Mutex<bool>>,
}
pub(crate) struct OwnerStop(pub(crate) Arc<OwnedHandle>);
impl Drop for OwnerStop {
    fn drop(&mut self) {
        let _ =
            unsafe { windows::Win32::System::Threading::SetEvent(HANDLE(self.0.as_raw_handle())) };
    }
}
pub(crate) fn channel<T>() -> windows::core::Result<(Sender<T>, Receiver<T>)> {
    let event = StopSignal::new()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let closed = Arc::new(Mutex::new(false));
    Ok((
        Sender {
            channel: sender,
            event: event.clone(),
            closed: closed.clone(),
        },
        Receiver {
            channel: receiver,
            event: event.event,
            closed,
        },
    ))
}
impl<T> Sender<T> {
    pub(crate) fn send(self, value: T) -> windows::core::Result<()> {
        let closed = self
            .closed
            .lock()
            .map_err(|_| windows::core::Error::from_hresult(E_ABORT))?;
        if *closed {
            return Err(windows::core::Error::from_hresult(E_ABORT));
        }
        self.channel
            .try_send(value)
            .map_err(|_| windows::core::Error::from_hresult(E_ABORT))?;
        self.event.request_stop()
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let _ = self.event.request_stop();
    }
}
impl<T> Receiver<T> {
    /// Closing and draining atomically prevents a late sender from leaving an
    /// Engine in the channel to be destroyed on the native owner thread.
    pub(crate) fn close_and_take(&self) -> Option<T> {
        let mut closed = self
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *closed = true;
        self.channel.try_recv().ok()
    }
    pub(crate) fn receive(&self, stop: &OwnedHandle) -> windows::core::Result<Option<T>> {
        let handles = [
            HANDLE(stop.as_raw_handle()),
            HANDLE(self.event.as_raw_handle()),
        ];
        match unsafe { WaitForMultipleObjects(&handles, false, INFINITE) } {
            event if event == WAIT_OBJECT_0 => Ok(None),
            event if event.0 == WAIT_OBJECT_0.0 + 1 => self
                .channel
                .try_recv()
                .map(Some)
                .map_err(|_| windows::core::Error::from_hresult(E_ABORT)),
            _ => Err(windows::core::Error::from_thread()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn abandoned_sender_signals_mailbox_without_peer_cancellation() {
        let (sender, receiver) = channel::<usize>().unwrap();
        drop(sender);
        assert_eq!(
            unsafe {
                windows::Win32::System::Threading::WaitForSingleObject(
                    HANDLE(receiver.event.as_raw_handle()),
                    1000,
                )
            },
            WAIT_OBJECT_0
        );
        let stop = StopSignal::new().unwrap();
        assert_eq!(receiver.receive(&stop.event).unwrap_err().code(), E_ABORT);
    }
    #[test]
    fn queued_handoff_loses_to_stop_and_stop_wakes_empty_mailbox() {
        for queued in [false, true] {
            let stop = StopSignal::new().unwrap();
            let (sender, receiver) = channel().unwrap();
            if queued {
                sender.send(42).unwrap();
            }
            stop.request_stop().unwrap();
            assert_eq!(receiver.receive(&stop.event).unwrap(), None);
        }
    }
    #[test]
    fn single_use_handoff_returns_the_owned_value() {
        let stop = StopSignal::new().unwrap();
        let (sender, receiver) = channel().unwrap();
        sender.send(vec![1, 2, 3]).unwrap();
        assert_eq!(receiver.receive(&stop.event).unwrap(), Some(vec![1, 2, 3]));
    }
    #[test]
    fn closing_recovers_queued_ownership_and_rejects_late_handoff() {
        let (sender, receiver) = channel().unwrap();
        sender.send(vec![7]).unwrap();
        assert_eq!(receiver.close_and_take(), Some(vec![7]));
        let (sender, receiver) = channel().unwrap();
        assert_eq!(receiver.close_and_take(), None);
        assert!(sender.send(vec![8]).is_err());
    }
}
