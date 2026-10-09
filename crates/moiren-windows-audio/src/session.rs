//! Share cancellation without sharing any apartment-bound audio interface.
use std::{
    os::windows::io::{AsRawHandle, OwnedHandle},
    sync::Arc,
};
use windows::Win32::{Foundation::HANDLE, System::Threading::SetEvent};

#[derive(Clone)]
pub struct StopSignal {
    pub(crate) event: Arc<OwnedHandle>,
}
impl StopSignal {
    /// A persistent manual-reset kernel event; linked owners all observe it.
    /// Signaling requires neither COM nor waiting for a worker's final Release.
    pub fn request_stop(&self) -> windows::core::Result<()> {
        unsafe { SetEvent(HANDLE(self.event.as_raw_handle())) }
    }
}
