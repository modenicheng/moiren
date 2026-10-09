//! Share cancellation without sharing any apartment-bound audio interface.
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::Arc,
};
use windows::Win32::{
    Foundation::HANDLE,
    System::Threading::{CreateEventW, SetEvent},
};

#[derive(Clone)]
pub struct StopSignal {
    pub(crate) event: Arc<OwnedHandle>,
}
impl StopSignal {
    /// Prepare cancellation before starting an asynchronous activation. This
    /// allows another control thread to cancel even before startup returns.
    pub fn new() -> windows::core::Result<Self> {
        let raw = unsafe { CreateEventW(None, true, false, None) }?;
        // SAFETY: a successful CreateEvent returns a uniquely owned handle.
        Ok(Self {
            event: Arc::new(unsafe { OwnedHandle::from_raw_handle(raw.0) }),
        })
    }
    /// A persistent manual-reset kernel event; linked owners all observe it.
    /// Signaling requires neither COM nor waiting for a worker's final Release.
    pub fn request_stop(&self) -> windows::core::Result<()> {
        unsafe { SetEvent(HANDLE(self.event.as_raw_handle())) }
    }
}
