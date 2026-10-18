//! The stereo f64 carrier is independent of the Engine's processing precision.
//! Implementations are created, used, stopped and destroyed on one native owner.
use std::time::Duration;

use moiren_engine::{audio_config::SampleFormat, host::BackendError};

pub type StereoFrame = [f64; 2];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStreamInfo {
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub buffer_frames: u32,
    pub channels: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeStreamStats {
    pub wait_wakes: u64,
    pub wait_timeouts: u64,
    pub write_calls: u64,
    pub written_frames: u64,
    pub zero_writes: u64,
    pub callbacks: u64,
    pub capture_dropped_frames: u64,
    /// Frames zero-filled by a native callback, including startup starvation.
    pub render_silent_frames: u64,
}

/// Driver-reported hardware frames, not measured end-to-end latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeLatency {
    pub input_frames: u32,
    pub output_frames: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRecoveryRequest {
    Reset,
    Resync,
    SampleRateChanged,
    BufferSizeChanged,
}

/// No Send bound: COM/driver objects must remain on their opening thread.
/// Transfers accept a prefix and never wait for queue space. A zero transfer is
/// a normal backpressure/empty indication. wait is the sole blocking operation.
/// Duplex streams expose capture and render from the same native session.
pub trait NativeStream {
    fn info(&self) -> NativeStreamInfo;
    fn start(&mut self) -> Result<(), BackendError>;
    fn stop(&mut self) -> Result<(), BackendError>;
    fn wait(&mut self, timeout: Duration) -> Result<bool, BackendError>;
    fn read(&mut self, output: &mut [StereoFrame]) -> Result<usize, BackendError>;
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError>;
    fn stats(&self) -> NativeStreamStats {
        NativeStreamStats::default()
    }
    fn driver_latency(&self) -> Option<NativeLatency> {
        None
    }
    fn recovery_request(&self) -> Option<NativeRecoveryRequest> {
        None
    }
    /// Owner-only final release after stop. ASIO must surface dispose failures
    /// before a resource event can authorize opening a replacement session.
    fn release(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
}

#[cfg(any(windows, test))]
pub(super) fn canonical_asio_driver_id(selector: &str) -> Result<String, BackendError> {
    let id = selector.strip_prefix("asio:").ok_or_else(|| {
        BackendError::Unsupported(
            "ASIO selector must use the asio:{CLSID} service identity form".into(),
        )
    })?;
    let id = id
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(id);
    if id.len() != 36
        || !id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(BackendError::Unsupported(
            "ASIO selector must contain a driver CLSID, optionally prefixed asio:".into(),
        ));
    }
    Ok(format!("{{{}}}", id.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_asio_selectors_become_canonical_driver_ids() {
        let id = "{01234567-89AB-CDEF-0123-456789ABCDEF}";
        assert_eq!(canonical_asio_driver_id(&format!("asio:{id}")).unwrap(), id);
        assert_eq!(
            canonical_asio_driver_id(&format!("asio:{}", id.to_ascii_lowercase())).unwrap(),
            id
        );
        assert!(canonical_asio_driver_id("01234567-89ab-cdef-0123-456789abcdef").is_err());
        assert!(
            canonical_asio_driver_id("{0.0.0.00000000}.{01234567-89AB-CDEF-0123-456789ABCDEF}")
                .is_err()
        );
        assert!(canonical_asio_driver_id("asio:driver name").is_err());
        assert!(canonical_asio_driver_id("asio:{01234567-89AB-CDEF-0123-456789ABCDEF}\0").is_err());
    }
}
