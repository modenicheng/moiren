use super::*;
use std::cell::{Cell, RefCell};
use windows::{
    Win32::Media::Audio::{AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_INVALID_DEVICE_PERIOD},
    core::HRESULT,
};

struct FakeClient3 {
    caps: Cell<SharedPeriodCaps>,
    query_error: Option<HRESULT>,
    initialization_error: Option<HRESULT>,
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
        self.initialization_error
            .map_or(Ok(()), |code| Err(Error::from_hresult(code)))
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
