//! The stereo f64 carrier is independent of the Engine's processing precision.
//! Implementations are created, used, stopped and destroyed on one native owner.
use std::time::Duration;

use crate::latency::{
    NativeStreamTiming, PreparedRenderQueue, RenderQueuePolicy, render::RenderPaddingObserver,
    shared_period::SharedPeriodReport,
};
use moiren_engine::{audio_config::SampleFormat, host::BackendError};

pub type StereoFrame = [f64; 2];

/// Native capture flags, not proof that non-silent signal was lost. Unsupported
/// adapters and an unavailable publication expose None, never fabricated zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct NativeCaptureQuality {
    pub discontinuities: u64,
    pub timestamp_errors: u64,
    pub silent_frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStreamInfo {
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub buffer_frames: u32,
    pub channels: usize,
}

/// Explicit Exclusive minimum-period request evidence. This records native
/// selection only; it is not a delivery-period or latency qualification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ExclusivePeriodEvidence {
    pub qualification: &'static str,
    pub requested_minimum: bool,
    pub default_hns: Option<i64>,
    pub minimum_hns: Option<i64>,
    pub query_hresult: Option<i32>,
    /// Exact first Initialize argument; absent when preparation refused it.
    pub initial_request_hns: Option<i64>,
    /// Exact last attempted Initialize argument, not a planned alignment retry.
    pub final_request_hns: Option<i64>,
    /// Alignment recovery entered; only retry_initialize_hresult establishes
    /// that the second Initialize returned (a fresh Activate can fail first).
    pub alignment_retry: bool,
    pub alignment_query_hresult: Option<i32>,
    pub aligned_buffer_frames: Option<u32>,
    pub first_initialize_hresult: Option<i32>,
    pub retry_initialize_hresult: Option<i32>,
    /// Raw successful GetBufferSize result, still subject to capacity checks.
    pub actual_buffer_frames: Option<u32>,
    pub actual_buffer_hresult: Option<i32>,
}

#[cfg(any(windows, test))]
impl ExclusivePeriodEvidence {
    pub(crate) fn record_query(&mut self, default_hns: i64, minimum_hns: i64) {
        self.default_hns = Some(default_hns);
        self.minimum_hns = Some(minimum_hns);
    }

    pub(crate) fn record_query_hresult(&mut self, hresult: i32) {
        self.query_hresult = Some(hresult);
    }

    pub(crate) fn record_alignment(&mut self, frames: u32) {
        self.alignment_retry = true;
        self.aligned_buffer_frames = Some(frames);
    }

    pub(crate) fn begin_alignment(&mut self) {
        self.alignment_retry = true;
    }

    pub(crate) fn record_alignment_hresult(&mut self, hresult: i32) {
        self.alignment_query_hresult = Some(hresult);
    }

    pub(crate) fn record_actual(&mut self, frames: u32) {
        self.actual_buffer_frames = Some(frames);
    }

    pub(crate) fn record_actual_hresult(&mut self, hresult: i32) {
        self.actual_buffer_hresult = Some(hresult);
    }
}

pub type ExclusivePeriodEvidenceHandle = std::sync::Arc<std::sync::Mutex<ExclusivePeriodEvidence>>;

/// Preparation-only call boundary: record the exact duration passed to the
/// native initializer, not a separately rounded estimate. No lock spans COM.
#[cfg(any(windows, test))]
pub(crate) fn observe_exclusive_initialize<T, E>(
    handle: Option<&ExclusivePeriodEvidenceHandle>,
    duration_hns: i64,
    retry: bool,
    initialize: impl FnOnce(i64) -> Result<T, E>,
    error_hresult: impl FnOnce(&E) -> i32,
) -> Result<T, E> {
    if let Some(handle) = handle {
        let mut evidence = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !retry {
            evidence.initial_request_hns = Some(duration_hns);
        }
        evidence.final_request_hns = Some(duration_hns);
    }
    let result = initialize(duration_hns);
    if let Some(handle) = handle {
        let hresult = result.as_ref().err().map_or(0, error_hresult);
        let mut evidence = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if retry {
            evidence.retry_initialize_hresult = Some(hresult);
        } else {
            evidence.first_initialize_hresult = Some(hresult);
        }
    }
    result
}

#[cfg(test)]
mod period_tests {
    use super::{ExclusivePeriodEvidence, observe_exclusive_initialize};
    use std::sync::{Arc, Mutex};

    fn evidence() -> ExclusivePeriodEvidence {
        ExclusivePeriodEvidence {
            qualification: "unqualified-request-only",
            requested_minimum: true,
            default_hns: None,
            minimum_hns: None,
            query_hresult: None,
            initial_request_hns: None,
            final_request_hns: None,
            alignment_retry: false,
            alignment_query_hresult: None,
            aligned_buffer_frames: None,
            first_initialize_hresult: None,
            retry_initialize_hresult: None,
            actual_buffer_frames: None,
            actual_buffer_hresult: None,
        }
    }

    #[test]
    fn period_receipt_separates_first_and_retry_initialize_results() {
        let handle = Arc::new(Mutex::new(evidence()));
        {
            let mut receipt = handle.lock().unwrap();
            receipt.record_query_hresult(0);
            receipt.record_query(100_000, 30_000);
        }
        assert_eq!(
            observe_exclusive_initialize(
                Some(&handle),
                30_000,
                false,
                |_| Err::<(), _>(-1),
                |code| *code
            ),
            Err(-1)
        );
        {
            let mut receipt = handle.lock().unwrap();
            receipt.begin_alignment();
            receipt.record_alignment_hresult(0);
            receipt.record_alignment(144);
        }
        observe_exclusive_initialize(
            Some(&handle),
            30_000,
            true,
            |_| Ok::<_, i32>(()),
            |code| *code,
        )
        .unwrap();
        let mut receipt = handle.lock().unwrap();
        receipt.record_actual_hresult(0);
        receipt.record_actual(144);

        assert_eq!(receipt.query_hresult, Some(0));
        assert_eq!(receipt.first_initialize_hresult, Some(-1));
        assert_eq!(receipt.retry_initialize_hresult, Some(0));
        assert_eq!(receipt.aligned_buffer_frames, Some(144));
        assert_eq!(receipt.actual_buffer_frames, Some(144));
    }

    #[test]
    fn period_query_does_not_claim_an_initialize_attempt() {
        let mut receipt = evidence();
        receipt.record_query_hresult(0);
        // A successful query can still return an invalid minimum.
        receipt.record_query(100_000, 0);
        assert_eq!(receipt.minimum_hns, Some(0));
        assert_eq!(receipt.initial_request_hns, None);
        assert_eq!(receipt.final_request_hns, None);
        assert_eq!(receipt.first_initialize_hresult, None);
    }

    #[test]
    fn period_alignment_preparation_does_not_claim_an_unexecuted_retry() {
        let handle = Arc::new(Mutex::new(evidence()));
        handle.lock().unwrap().record_query(100_000, 30_000);
        assert_eq!(
            observe_exclusive_initialize(
                Some(&handle),
                30_000,
                false,
                |_| Err::<(), _>(-1),
                |code| *code
            ),
            Err(-1)
        );
        let mut receipt = handle.lock().unwrap();
        receipt.begin_alignment();
        receipt.record_alignment_hresult(0);
        receipt.record_alignment(256);
        // Fresh Activate may fail here, before any second Initialize call.
        assert_eq!(receipt.aligned_buffer_frames, Some(256));
        assert_eq!(receipt.final_request_hns, Some(30_000));
        assert_eq!(receipt.retry_initialize_hresult, None);
    }

    #[test]
    fn period_alignment_duration_matches_the_initialize_rounding() {
        let handle = Arc::new(Mutex::new(evidence()));
        let result = observe_exclusive_initialize(
            Some(&handle),
            53_333,
            true,
            |actual| {
                assert_eq!(actual, 53_333);
                // Metadata is visible before the call, without holding its mutex.
                let receipt = handle.try_lock().unwrap();
                assert_eq!(receipt.final_request_hns, Some(actual));
                assert_eq!(receipt.retry_initialize_hresult, None);
                Ok::<_, i32>(())
            },
            |code| *code,
        );
        assert_eq!(result, Ok(()));
        assert_eq!(handle.lock().unwrap().final_request_hns, Some(53_333));
    }

    #[test]
    fn period_failure_evidence_retains_errors_without_inventing_values() {
        let mut receipt = evidence();
        receipt.record_query_hresult(-2);
        assert_eq!(receipt.query_hresult, Some(-2));
        assert_eq!(receipt.minimum_hns, None);
        assert_eq!(receipt.initial_request_hns, None);
        receipt.begin_alignment();
        receipt.record_alignment_hresult(-3);
        receipt.record_actual_hresult(-4);
        assert_eq!(receipt.alignment_query_hresult, Some(-3));
        assert_eq!(receipt.aligned_buffer_frames, None);
        assert_eq!(receipt.actual_buffer_hresult, Some(-4));
        assert_eq!(receipt.actual_buffer_frames, None);
    }

    #[test]
    fn period_call_boundary_preserves_retry_errors_and_default_call() {
        let handle = Arc::new(Mutex::new(evidence()));
        assert_eq!(
            observe_exclusive_initialize(
                Some(&handle),
                30_000,
                false,
                |_| Ok::<_, i32>(()),
                |code| *code
            ),
            Ok(())
        );
        assert_eq!(
            observe_exclusive_initialize(
                Some(&handle),
                53_333,
                true,
                |_| Err::<(), _>(-7),
                |code| *code
            ),
            Err(-7)
        );
        let receipt = handle.lock().unwrap();
        assert_eq!(receipt.initial_request_hns, Some(30_000));
        assert_eq!(receipt.final_request_hns, Some(53_333));
        assert_eq!(receipt.first_initialize_hresult, Some(0));
        assert_eq!(receipt.retry_initialize_hresult, Some(-7));
        assert_eq!(
            observe_exclusive_initialize(None, 100_000, false, Ok::<_, i32>, |code| *code),
            Ok(100_000)
        );
    }

    #[cfg(windows)]
    #[test]
    fn period_retry_uses_existing_256_frame_duration_conversion() {
        let handle = Arc::new(Mutex::new(evidence()));
        let actual = crate::negotiation::exclusive_frames_duration(256, 48_000);
        observe_exclusive_initialize(
            Some(&handle),
            actual,
            true,
            |passed| {
                assert_eq!(passed, 53_333);
                Ok::<_, i32>(())
            },
            |code| *code,
        )
        .unwrap();
        assert_eq!(handle.lock().unwrap().final_request_hns, Some(53_333));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct NativeStreamStats {
    pub wait_wakes: u64,
    pub wait_timeouts: u64,
    pub write_calls: u64,
    pub written_frames: u64,
    pub zero_writes: u64,
    pub callbacks: u64,
    pub capture_dropped_frames: u64,
    /// True only when the adapter actually observes all three capture flags.
    pub capture_quality_available: bool,
    pub capture_discontinuities: u64,
    pub capture_timestamp_errors: u64,
    pub capture_silent_frames: u64,
    /// Frames zero-filled by a native callback, including startup starvation.
    pub render_silent_frames: u64,
}

/// Driver-reported hardware frames, not measured end-to-end latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeLatency {
    pub input_frames: u32,
    pub output_frames: u32,
}

/// An optional native clock observation, distinct from client-rate frames.
/// Normalize position only with the observed frequency, never a sample-rate
/// default. Failed/low-accuracy position calls retain their raw HRESULT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct NativeDeviceClockObservation {
    pub frequency_hz: Option<u64>,
    pub frequency_hresult: Option<i32>,
    pub sample: crate::latency::DeviceClockObservation,
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
    /// Prepared device timing, in client-rate frames. Capacity alone does not
    /// establish the delivery period of an adapter or callback transport.
    fn prepared_timing(&self) -> NativeStreamTiming {
        let info = self.info();
        NativeStreamTiming {
            sample_rate: info.sample_rate,
            period_frames: None,
            capacity_frames: info.buffer_frames,
        }
    }
    fn shared_period(&self) -> Option<SharedPeriodReport> {
        None
    }
    fn render_queue(&self) -> Option<PreparedRenderQueue> {
        None
    }
    fn render_padding_observer(&self) -> Option<RenderPaddingObserver> {
        None
    }
    /// One bounded owner-local sample of an already prepared clock service.
    /// Missing service support is None; sampling failures remain diagnostic
    /// HRESULTs rather than failures of native audio processing.
    fn device_clock_observation(&self) -> Option<NativeDeviceClockObservation> {
        None
    }
    /// Install a bounded raw-PCM tap before Start. Errors return the prepared
    /// writer so its backing storage can be reclaimed on the control side.
    fn attach_capture_evidence(
        &mut self,
        writer: crate::capture::PcmEvidenceWriter,
    ) -> Result<(), Box<(BackendError, crate::capture::PcmEvidenceWriter)>> {
        Err(Box::new((
            BackendError::Unsupported("native capture evidence is not supported".into()),
            writer,
        )))
    }
    /// Transfer a closed evidence writer to the control-side joiner.
    fn take_capture_evidence(&mut self) -> Option<crate::capture::PcmEvidenceWriter> {
        None
    }
    /// Called during owner preparation, before Pump allocation and Start.
    /// Adapters must opt in to native queue admission rather than silently
    /// accepting a target they cannot enforce.
    fn configure_render_queue(&mut self, policy: RenderQueuePolicy) -> Result<(), BackendError> {
        match policy {
            RenderQueuePolicy::FullCapacity => Ok(()),
            RenderQueuePolicy::Target { .. } => Err(BackendError::Unsupported(
                "this native adapter does not support a target render queue".into(),
            )),
        }
    }
    /// Maximum frames to produce now; querying this must not wait for space.
    fn render_demand(&mut self) -> Result<usize, BackendError> {
        Ok(self.info().buffer_frames as usize)
    }
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

    struct UnobservedAdapter;
    impl NativeStream for UnobservedAdapter {
        fn info(&self) -> NativeStreamInfo {
            NativeStreamInfo {
                sample_rate: 48_000,
                format: SampleFormat::Float32,
                buffer_frames: 512,
                channels: 2,
            }
        }
        fn start(&mut self) -> Result<(), BackendError> {
            unreachable!()
        }
        fn stop(&mut self) -> Result<(), BackendError> {
            unreachable!()
        }
        fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
            unreachable!()
        }
        fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
            unreachable!()
        }
        fn write(&mut self, _: &[StereoFrame]) -> Result<usize, BackendError> {
            unreachable!()
        }
    }

    #[test]
    fn default_native_adapter_exposes_capacity_without_claiming_a_period_or_target_support() {
        let mut stream = UnobservedAdapter;
        assert_eq!(
            stream.prepared_timing(),
            NativeStreamTiming {
                sample_rate: 48_000,
                period_frames: None,
                capacity_frames: 512,
            }
        );
        assert!(stream.shared_period().is_none());
        assert!(stream.render_queue().is_none());
        assert!(stream.render_padding_observer().is_none());
        assert!(stream.device_clock_observation().is_none());
        assert_eq!(stream.render_demand().unwrap(), 512);
        stream
            .configure_render_queue(RenderQueuePolicy::FullCapacity)
            .unwrap();
        assert!(matches!(
            stream.configure_render_queue(RenderQueuePolicy::Target {
                frames: 256,
                wake_margin_frames: 0,
                callback_budget_frames: 1,
            }),
            Err(BackendError::Unsupported(_))
        ));
    }

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
