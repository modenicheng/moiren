//! Console control-event bridge: routes terminal Ctrl+C (and the sibling
//! console events Windows delivers to console-attached processes) into the
//! GUI's graceful quit path. Without this hook the default handler terminates
//! the process on the spot, skipping the autosave flush, the window-state
//! save and the service retirement that a normal quit runs.
//!
//! The handler runs on an OS-created thread, so it must not touch gpui or
//! the audio service. It only raises a flag and wakes the future that the UI
//! event loop is awaiting; `cx.quit()` then executes on the main thread and
//! the rest of the shutdown stays on the ordinary quit path.
//!
//! Windows-only: Ctrl+C as a console control event is a Win32 mechanism. A
//! console-less launch (Explorer, shortcut) simply never fires the handler.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use windows::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    SetConsoleCtrlHandler,
};
use windows::core::BOOL;

static REQUESTED: AtomicBool = AtomicBool::new(false);
static WAKER: Mutex<Option<Waker>> = Mutex::new(None);
static INSTALLED: OnceLock<()> = OnceLock::new();

/// Register the console handler. Idempotent; a registration failure is
/// logged and otherwise ignored so a non-interactive launch still runs.
pub fn install_quit_handler() {
    if INSTALLED.set(()).is_err() {
        return;
    }
    unsafe {
        if let Err(error) = SetConsoleCtrlHandler(Some(handler_routine), true) {
            eprintln!("Moiren console Ctrl+C hook unavailable: {error}");
        }
    }
}

/// Console control events the graceful quit covers: Ctrl+C, Ctrl+Break,
/// closing the host console window, and session logoff/shutdown. Returning
/// `TRUE` claims the event (suppressing the default immediate termination;
/// for a closing console it starts the OS grace period our shutdown fits
/// in) and the quit itself proceeds on the UI thread.
unsafe extern "system" fn handler_routine(event: u32) -> BOOL {
    match event {
        CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
        | CTRL_SHUTDOWN_EVENT => {
            request_quit();
            true.into()
        }
        _ => false.into(),
    }
}

/// The one entry the OS handler thread uses. Wakes the registered waker so
/// the UI event loop observes the request without any cross-thread locking
/// beyond this handoff.
fn request_quit() {
    REQUESTED.store(true, Ordering::Release);
    if let Ok(mut waker) = WAKER.lock()
        && let Some(waker) = waker.take()
    {
        waker.wake();
    }
}

/// Resolves once a console control event requested a graceful quit. Awaited
/// inside the gpui app context; resolution happens from the OS handler
/// thread, polling always on the event loop.
pub fn quit_requested() -> impl Future<Output = ()> {
    struct QuitRequested;
    impl Future for QuitRequested {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if REQUESTED.load(Ordering::Acquire) {
                return Poll::Ready(());
            }
            if let Ok(mut waker) = WAKER.lock() {
                *waker = Some(cx.waker().clone());
            }
            // Re-check after publishing the waker: the handler may have fired
            // between the first check and the store, in which case its wake
            // found nothing to wake.
            if REQUESTED.load(Ordering::Acquire) {
                return Poll::Ready(());
            }
            Poll::Pending
        }
    }
    QuitRequested
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::pin;

    #[test]
    fn quit_requested_resolves_once_the_handler_signals() {
        let mut requested = pin!(quit_requested());
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        assert_eq!(requested.as_mut().poll(&mut cx), Poll::Pending);
        // Same entry the OS handler thread drives for every covered event.
        request_quit();
        assert_eq!(requested.as_mut().poll(&mut cx), Poll::Ready(()));
    }
}
