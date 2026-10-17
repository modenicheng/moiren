use super::*;
use std::cell::{Cell, RefCell};
use windows::{
    Win32::Media::Audio::{AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_INVALID_DEVICE_PERIOD},
    core::HRESULT,
};

mod compatibility;

#[test]
fn compatibility_selects_float_client_without_reinterpreting_native_pcm_or_array_bytes() {
    use super::super::format::{PreparedSharedFormat, test_mix};
    for capture in [false, true] {
        for (rate, channels, bits, float) in [
            (22_050, 2, 8, false),
            (44_100, 2, 16, false),
            (48_000, 2, 24, false),
            (96_000, 2, 32, false),
            (192_000, 2, 32, true),
            (48_000, 2, 64, true),
        ] {
            let format =
                PreparedSharedFormat::from_owned(test_mix(rate, channels, bits, float), capture)
                    .unwrap();
            assert!(format.windows_auto_conversion);
            assert_eq!(format.mix.sample_rate, rate);
            assert_eq!(format.mix.channels, channels);
            assert_eq!(format.mix.container_bits, bits);
            let client = unsafe { format.as_ptr().read_unaligned() };
            assert_eq!({ client.nSamplesPerSec }, 48_000);
            assert_eq!({ client.nChannels }, 2);
            assert_eq!({ client.wBitsPerSample }, 32);
            assert_eq!({ client.wFormatTag }, 3);
            assert_eq!({ client.nBlockAlign }, 8);
        }
    }
}

#[test]
fn compatibility_preserves_native_44100_mono_capture_and_48000_stereo_render() {
    use super::super::format::{PreparedSharedFormat, test_mix};
    for (capture, rate, channels) in [(true, 44_100, 1), (true, 48_000, 2), (false, 48_000, 2)] {
        let memory = test_mix(rate, channels, 32, true);
        let pointer = memory.0;
        let format = PreparedSharedFormat::from_owned(memory, capture).unwrap();
        assert!(!format.windows_auto_conversion);
        assert_eq!(format.as_ptr(), pointer.cast_const());
    }
}

#[test]
fn compatibility_rejects_invalid_mix_layouts_instead_of_using_conversion_to_hide_them() {
    use super::super::format::{PreparedSharedFormat, test_mix};
    for (rate, channels, bits) in [(0, 2, 32), (48_000, 0, 32), (48_000, 2, 20)] {
        assert!(
            PreparedSharedFormat::from_owned(test_mix(rate, channels, bits, false), true).is_err()
        );
    }
    let memory = test_mix(48_000, 2, 32, true);
    unsafe { (*memory.0).nBlockAlign = 4 };
    assert!(PreparedSharedFormat::from_owned(memory, false).is_err());
    let memory = test_mix(48_000, 2, 32, true);
    unsafe { (*memory.0).nAvgBytesPerSec = 1 };
    assert!(PreparedSharedFormat::from_owned(memory, false).is_err());
    let memory = test_mix(48_000, 2, 32, true);
    unsafe {
        (*memory.0).wFormatTag = 0xfffe;
        (*memory.0).cbSize = 0;
    }
    assert!(PreparedSharedFormat::from_owned(memory, true).is_err());
    assert!(PreparedSharedFormat::from_owned(TaskMemory(std::ptr::null_mut()), true).is_err());
}

#[test]
fn converted_periods_are_reported_in_client_frames_not_native_frames() {
    let mut report = SharedPeriodReport::default();
    record_period(960, 96_000, 48_000, &mut report).unwrap();
    assert_eq!(report.actual_period_frames, Some(480));
    assert_eq!(report.engine_period_frames, Some(960));
    assert_eq!(report.engine_sample_rate, Some(96_000));
    record_period(128, 44_100, 48_000, &mut report).unwrap();
    assert_eq!(report.actual_period_frames, Some(140));
    for (frames, engine_rate, client_rate) in [
        (0, 48_000, 48_000),
        (480, 0, 48_000),
        (480, 48_000, 0),
        (u32::MAX, 1, 48_000),
    ] {
        assert!(record_period(frames, engine_rate, client_rate, &mut report).is_err());
    }
}

struct FakeClient3 {
    caps: Cell<SharedPeriodCaps>,
    query_error: Option<HRESULT>,
    initialization_error: Option<HRESULT>,
    retry_error: Option<HRESULT>,
    current_error: Option<HRESULT>,
    current_period: Cell<u32>,
    operations: RefCell<Vec<&'static str>>,
    requested: Cell<Option<u32>>,
}

impl FakeClient3 {
    fn new() -> Self {
        Self {
            caps: Cell::new(SharedPeriodCaps {
                default_frames: 480,
                fundamental_frames: 48,
                min_frames: 96,
                max_frames: 480,
            }),
            query_error: None,
            initialization_error: None,
            retry_error: None,
            current_error: None,
            current_period: Cell::new(480),
            operations: RefCell::new(Vec::new()),
            requested: Cell::new(None),
        }
    }
}

impl Client3Api for FakeClient3 {
    fn capabilities(&self) -> Result<SharedPeriodCaps, Error> {
        self.operations.borrow_mut().push("capabilities");
        self.query_error.map_or_else(
            || Ok(self.caps.get()),
            |code| Err(Error::from_hresult(code)),
        )
    }

    fn initialize(&self, period: u32) -> Result<(), Error> {
        self.operations.borrow_mut().push("initialize");
        self.requested.set(Some(period));
        if self.initialization_error
            == Some(windows::Win32::Media::Audio::AUDCLNT_E_ENGINE_PERIODICITY_LOCKED)
            && self
                .operations
                .borrow()
                .iter()
                .filter(|operation| **operation == "initialize")
                .count()
                > 1
        {
            return self
                .retry_error
                .map_or(Ok(()), |code| Err(Error::from_hresult(code)));
        }
        self.initialization_error
            .map_or(Ok(()), |code| Err(Error::from_hresult(code)))
    }
    fn current_period(&self) -> Result<u32, Error> {
        self.operations.borrow_mut().push("current_period");
        self.current_error
            .map_or(Ok(self.current_period.get()), |code| {
                Err(Error::from_hresult(code))
            })
    }
}

#[test]
fn period_locked_negotiates_the_existing_engine_period_once() {
    let mut api = FakeClient3::new();
    api.initialization_error =
        Some(windows::Win32::Media::Audio::AUDCLNT_E_ENGINE_PERIODICITY_LOCKED);
    let mut report = SharedPeriodReport::default();
    initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report).unwrap();
    assert_eq!(
        *api.operations.borrow(),
        ["capabilities", "initialize", "current_period", "initialize"]
    );
    assert_eq!(report.requested_period_frames, Some(240));
    assert_eq!(report.actual_period_frames, Some(480));
    assert_eq!(api.requested.get(), Some(480));
    assert_eq!(
        report.initialization_api,
        Some(SharedInitializationApi::Client3)
    );
}

#[test]
fn period_locked_never_retries_an_invalid_or_unchanged_current_period() {
    for period in [0, 240, 481, 500] {
        let mut api = FakeClient3::new();
        api.initialization_error =
            Some(windows::Win32::Media::Audio::AUDCLNT_E_ENGINE_PERIODICITY_LOCKED);
        api.current_period.set(period);
        let mut report = SharedPeriodReport::default();
        assert!(
            initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report).is_err()
        );
        assert_eq!(
            *api.operations.borrow(),
            ["capabilities", "initialize", "current_period"]
        );
        assert!(report.initialization_api.is_none());
    }
}

#[test]
fn period_locked_retry_failure_preserves_the_actual_device_error() {
    let mut api = FakeClient3::new();
    api.initialization_error =
        Some(windows::Win32::Media::Audio::AUDCLNT_E_ENGINE_PERIODICITY_LOCKED);
    api.retry_error = Some(AUDCLNT_E_DEVICE_INVALIDATED);
    let mut report = SharedPeriodReport::default();
    let failure =
        initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report).unwrap_err();
    assert_eq!(failure.error.code(), AUDCLNT_E_DEVICE_INVALIDATED);
    assert!(report.initialization_api.is_none());
    assert!(report.fallback_reason.is_none());
}

#[test]
fn locked_period_query_or_format_failure_never_retries_initialization() {
    for query_error in [
        E_NOTIMPL,
        windows::Win32::Media::Audio::AUDCLNT_E_ENGINE_FORMAT_LOCKED,
    ] {
        let mut api = FakeClient3::new();
        api.initialization_error =
            Some(windows::Win32::Media::Audio::AUDCLNT_E_ENGINE_PERIODICITY_LOCKED);
        api.current_error = Some(query_error);
        let mut report = SharedPeriodReport::default();
        let failure = initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report)
            .unwrap_err();
        assert_eq!(
            failure.error.code(),
            windows::Win32::Media::Audio::AUDCLNT_E_ENGINE_PERIODICITY_LOCKED
        );
        assert_eq!(report.actual_period_hresult, Some(query_error.0));
        assert_eq!(
            *api.operations.borrow(),
            ["capabilities", "initialize", "current_period"]
        );
        assert!(report.initialization_api.is_none());
    }
}

#[test]
fn queries_current_capabilities_before_initializing_each_stream() {
    let api = FakeClient3::new();
    let mut report = SharedPeriodReport::default();
    initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report).unwrap();
    assert_eq!(*api.operations.borrow(), ["capabilities", "initialize"]);
    assert_eq!(api.requested.get(), Some(240));
    assert_eq!(
        report.initialization_api,
        Some(SharedInitializationApi::Client3)
    );
    assert!(report.client3_failure.is_none());
    api.caps.set(SharedPeriodCaps {
        min_frames: 336,
        ..api.caps.get()
    });
    let mut second_report = SharedPeriodReport::default();
    initialize_client3(
        &api,
        48_000,
        SharedPeriodPolicy::default(),
        &mut second_report,
    )
    .unwrap();
    assert_eq!(second_report.caps.unwrap().min_frames, 336);
    assert_eq!(second_report.requested_period_frames, Some(336));
    assert_eq!(api.operations.borrow().len(), 4);
}

#[test]
fn only_missing_interfaces_and_unsupported_queries_allow_legacy_fallback() {
    for (stage, hresult, expected) in [
        (
            SharedInitializationStage::InterfaceQuery,
            E_NOINTERFACE,
            Some(SharedFallbackReason::Client3Unavailable),
        ),
        (
            SharedInitializationStage::Capabilities,
            E_NOTIMPL,
            Some(SharedFallbackReason::CapabilityQueryUnsupported),
        ),
        (
            SharedInitializationStage::Capabilities,
            AUDCLNT_E_UNSUPPORTED_FORMAT,
            Some(SharedFallbackReason::CapabilityQueryUnsupported),
        ),
        (
            SharedInitializationStage::Capabilities,
            AUDCLNT_E_DEVICE_INVALIDATED,
            None,
        ),
        (SharedInitializationStage::Capabilities, E_INVALIDARG, None),
        (
            SharedInitializationStage::Initialization,
            AUDCLNT_E_INVALID_DEVICE_PERIOD,
            None,
        ),
        (
            SharedInitializationStage::Initialization,
            AUDCLNT_E_UNSUPPORTED_FORMAT,
            None,
        ),
    ] {
        let mut report = SharedPeriodReport::default();
        let error = failure(stage, Error::from_hresult(hresult), &mut report);
        assert_eq!(error.fallback_reason, expected);
        assert_eq!(
            report.client3_failure,
            Some(SharedInitializationFailure {
                stage,
                hresult: hresult.0
            })
        );
        assert!(report.initialization_api.is_none());
        assert!(report.fallback_reason.is_none());
    }
}

#[test]
fn query_and_policy_failures_never_attempt_initialization() {
    let mut api = FakeClient3::new();
    api.query_error = Some(E_NOTIMPL);
    let mut report = SharedPeriodReport::default();
    let error =
        initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report).unwrap_err();
    assert_eq!(
        error.fallback_reason,
        Some(SharedFallbackReason::CapabilityQueryUnsupported)
    );
    assert_eq!(*api.operations.borrow(), ["capabilities"]);
    assert!(report.requested_period_frames.is_none());
    let api = FakeClient3::new();
    api.caps.set(SharedPeriodCaps {
        fundamental_frames: 0,
        ..api.caps.get()
    });
    let mut report = SharedPeriodReport::default();
    let error =
        initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report).unwrap_err();
    assert!(error.fallback_reason.is_none());
    assert_eq!(
        report.client3_failure.unwrap().stage,
        SharedInitializationStage::PeriodSelection
    );
    assert!(report.caps.is_some());
    assert_eq!(*api.operations.borrow(), ["capabilities"]);
}

#[test]
fn initialization_failure_preserves_request_without_retrying_or_claiming_success() {
    let mut api = FakeClient3::new();
    api.initialization_error = Some(AUDCLNT_E_INVALID_DEVICE_PERIOD);
    let mut report = SharedPeriodReport::default();
    let error =
        initialize_client3(&api, 48_000, SharedPeriodPolicy::default(), &mut report).unwrap_err();
    assert!(error.fallback_reason.is_none());
    assert_eq!(*api.operations.borrow(), ["capabilities", "initialize"]);
    assert_eq!(report.requested_period_frames, Some(240));
    assert_eq!(
        report.client3_failure.unwrap().hresult,
        AUDCLNT_E_INVALID_DEVICE_PERIOD.0
    );
    assert!(report.actual_period_frames.is_none());
    assert!(report.initialization_api.is_none());
}
