//! Direct IASIO hosting without an SDK or a C++ build dependency.
//!
//! The backend selects channels 0/1 (or mono channel 0) in each enabled direction.
//! ASIO owns the hardware sample format; `info()` reports it for the caller's
//! requested-format validation. Mixed effective formats are rejected.
//!
//! Only 64-bit Windows can activate drivers. The stream is apartment-bound and
//! must be opened, started, stopped and dropped on the native owner thread.
//! One process-wide lease covers driver activation through final COM Release.

use moiren_engine::host::BackendError;

mod capabilities;

pub use capabilities::{
    AsioBufferCapabilities, AsioCapabilities, AsioChannel, AsioLatencies, AsioRateSupport,
    AsioSampleType, query_capabilities,
};

#[cfg(any(all(windows, target_pointer_width = "64"), test))]
mod abi;
#[cfg(any(all(windows, target_pointer_width = "64"), test))]
mod buffer;
#[cfg(all(windows, target_pointer_width = "64"))]
mod callback;
#[cfg(all(windows, target_pointer_width = "64"))]
mod driver;
#[cfg(all(windows, target_pointer_width = "64"))]
mod registry;
#[cfg(any(all(windows, target_pointer_width = "64"), test))]
mod sample;
#[cfg(all(windows, target_pointer_width = "64"))]
mod stream;

#[cfg(all(windows, target_pointer_width = "64"))]
pub use stream::AsioStream;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AsioDriver {
    /// Canonical registry CLSID, including braces; never an enumeration index.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsioOptions {
    pub driver_id: String,
    pub sample_rate: u32,
    /// Zero selects the driver's preferred size; nonzero requests are exact.
    pub buffer_frames: u32,
    pub input: bool,
    pub output: bool,
}

/// Enumerate registered drivers in the native 64-bit registry view.
/// An empty list means that no ASIO driver is installed.
pub fn list_drivers() -> Result<Vec<AsioDriver>, BackendError> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        registry::list()
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        Err(unsupported_platform())
    }
}

#[cfg(not(all(windows, target_pointer_width = "64")))]
fn unsupported_platform() -> BackendError {
    BackendError::Unsupported("direct ASIO requires a 64-bit Windows process and driver".into())
}

#[cfg(not(all(windows, target_pointer_width = "64")))]
pub struct AsioStream {
    _private: (),
}

#[cfg(not(all(windows, target_pointer_width = "64")))]
impl AsioStream {
    pub fn open(_options: AsioOptions) -> Result<Self, BackendError> {
        Err(unsupported_platform())
    }
}

// The uninhabited-in-practice fallback keeps dispatch portable. `open` always
// returns Unsupported, so an inactive Windows backend cannot masquerade as IO.
#[cfg(not(all(windows, target_pointer_width = "64")))]
impl super::native::NativeStream for AsioStream {
    fn info(&self) -> super::native::NativeStreamInfo {
        unreachable!("ASIO cannot open on this platform")
    }
    fn start(&mut self) -> Result<(), BackendError> {
        Err(unsupported_platform())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        Err(unsupported_platform())
    }
    fn wait(&mut self, _timeout: std::time::Duration) -> Result<bool, BackendError> {
        Err(unsupported_platform())
    }
    fn read(&mut self, _output: &mut [[f64; 2]]) -> Result<usize, BackendError> {
        Err(unsupported_platform())
    }
    fn write(&mut self, _input: &[[f64; 2]]) -> Result<usize, BackendError> {
        Err(unsupported_platform())
    }
}
