//! High-resolution one-shot waits; no timeBeginPeriod, priority escalation or
//! spinning. The disposable Mixer view owns both the worker and cancellation.
use super::{Cadence, Instant};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::Arc,
    thread::JoinHandle,
};
use windows::{
    Win32::{
        Foundation::{HANDLE, HWND, WAIT_OBJECT_0},
        Graphics::Gdi::{RDW_INVALIDATE, RedrawWindow},
        System::Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateWaitableTimerExW, INFINITE,
            SetEvent, SetWaitableTimerEx, TIMER_ALL_ACCESS, WaitForMultipleObjects,
        },
    },
    core::PCWSTR,
};

pub(super) struct Wake {
    stop: Arc<OwnedHandle>,
    worker: Option<JoinHandle<()>>,
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

impl Wake {
    pub fn cancel(&self) {
        // SAFETY: the event is owned here and retained by the worker. This
        // interrupts the wait independently of the next frame deadline.
        let _ = unsafe { SetEvent(raw(&self.stop)) };
    }

    pub fn start() -> anyhow::Result<(Self, async_channel::Receiver<()>)> {
        // SAFETY: unnamed kernel handles, immediately transferred into RAII
        // ownership. Only the worker waits; drop signals before joining it.
        let stop = Arc::new(unsafe {
            OwnedHandle::from_raw_handle(CreateEventW(None, true, false, PCWSTR::null())?.0)
        });
        let timer = unsafe {
            OwnedHandle::from_raw_handle(
                CreateWaitableTimerExW(
                    None,
                    PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS.0,
                )?
                .0,
            )
        };
        let (sender, receiver) = async_channel::bounded(1);
        let cancel = stop.clone();
        let worker = std::thread::Builder::new()
            .name("mixer-frame-clock".into())
            .spawn(move || {
                let mut cadence = Cadence::new(Instant::now());
                loop {
                    let remaining = cadence.next.saturating_duration_since(Instant::now());
                    let due = -(remaining
                        .as_nanos()
                        .div_ceil(100)
                        .max(1)
                        .min(i64::MAX as u128) as i64);
                    // SAFETY: handles stay open until this worker has returned;
                    // due is a relative 100ns interval, no APC callback is used.
                    let result = unsafe {
                        if SetWaitableTimerEx(raw(&timer), &due, 0, None, None, None, 0).is_err() {
                            break;
                        }
                        WaitForMultipleObjects(&[raw(&cancel), raw(&timer)], false, INFINITE)
                    };
                    if result.0 != WAIT_OBJECT_0.0 + 1 {
                        break;
                    }
                    if cadence.take(Instant::now()) {
                        match sender.try_send(()) {
                            Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
                            Err(async_channel::TrySendError::Closed(())) => break,
                        }
                    }
                }
            })?;
        Ok((
            Self {
                stop,
                worker: Some(worker),
            },
            receiver,
        ))
    }
}

impl Drop for Wake {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn has_window(window: &gpui_kit::Window) -> bool {
    HasWindowHandle::window_handle(window)
        .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Win32(_)))
}

pub(super) fn request_paint(window: &gpui_kit::Window) {
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        // SAFETY: this is the current window's live HWND. Invalidating is
        // asynchronous, so no nested GPUI borrow or forced cache refresh.
        unsafe {
            let _ = RedrawWindow(
                Some(HWND(handle.hwnd.get() as *mut _)),
                None,
                None,
                RDW_INVALIDATE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn native_demand_is_bounded_and_drop_cancels_the_waiter() {
        let (wake, receiver) = Wake::start().expect("high-resolution timer");
        let deadline = Instant::now() + Duration::from_secs(2);
        while receiver.is_empty() {
            assert!(Instant::now() < deadline, "native timer never woke");
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(receiver.len(), 1, "late consumers must coalesce demand");
        drop(wake);
        assert!(receiver.is_closed(), "drop must join the stopped worker");
    }
}
