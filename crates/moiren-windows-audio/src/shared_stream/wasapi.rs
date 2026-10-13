use super::{
    SharedFallbackReason, SharedInitializationApi, SharedInitializationFailure,
    SharedInitializationStage, SharedPeriodCaps, SharedPeriodPolicy, SharedPeriodReport,
    choose_shared_period,
};
use crate::owner::TaskMemory;
use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL},
        Media::Audio::{
            AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient,
            IAudioClient3, WAVEFORMATEX,
        },
    },
    core::{Error, GUID, Interface},
};

#[derive(Debug)]
pub(crate) struct SharedClient3Error {
    pub stage: &'static str,
    pub fallback_reason: Option<SharedFallbackReason>,
    pub error: Error,
}

fn failure(
    stage: SharedInitializationStage,
    error: Error,
    report: &mut SharedPeriodReport,
) -> SharedClient3Error {
    let hresult = error.code();
    report.client3_failure = Some(SharedInitializationFailure {
        stage,
        hresult: hresult.0,
    });
    let fallback_reason = match (stage, hresult) {
        (SharedInitializationStage::InterfaceQuery, E_NOINTERFACE) => {
            Some(SharedFallbackReason::Client3Unavailable)
        }
        (
            SharedInitializationStage::Capabilities,
            E_NOINTERFACE | E_NOTIMPL | AUDCLNT_E_UNSUPPORTED_FORMAT,
        ) => Some(SharedFallbackReason::CapabilityQueryUnsupported),
        _ => None,
    };
    SharedClient3Error {
        stage: match stage {
            SharedInitializationStage::InterfaceQuery => "QueryInterface(IAudioClient3)",
            SharedInitializationStage::Capabilities => "GetSharedModeEnginePeriod",
            SharedInitializationStage::PeriodSelection => "choose_shared_period",
            SharedInitializationStage::Initialization => "InitializeSharedAudioStream",
        },
        fallback_reason,
        error,
    }
}

trait Client3Api {
    fn capabilities(&self) -> Result<SharedPeriodCaps, Error>;
    fn initialize(&self, period: u32) -> Result<(), Error>;
}

struct NativeClient3<'client> {
    client: &'client IAudioClient3,
    format: *const WAVEFORMATEX,
    session_guid: Option<&'client GUID>,
}

impl Client3Api for NativeClient3<'_> {
    fn capabilities(&self) -> Result<SharedPeriodCaps, Error> {
        let mut caps = SharedPeriodCaps {
            default_frames: 0,
            fundamental_frames: 0,
            min_frames: 0,
            max_frames: 0,
        };
        // The owner retains its validated format and configured client through
        // preparation. This query follows SetClientProperties for each stream.
        unsafe {
            self.client.GetSharedModeEnginePeriod(
                self.format,
                &mut caps.default_frames,
                &mut caps.fundamental_frames,
                &mut caps.min_frames,
                &mut caps.max_frames,
            )?;
        }
        Ok(caps)
    }

    fn initialize(&self, period: u32) -> Result<(), Error> {
        // Client3 supports EVENTCALLBACK here; legacy conversion/session flags
        // are deliberately kept on the separate legacy initialization path.
        unsafe {
            self.client.InitializeSharedAudioStream(
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                period,
                self.format,
                self.session_guid.map(std::ptr::from_ref),
            )
        }
    }
}

fn initialize_client3(
    api: &impl Client3Api,
    sample_rate: u32,
    policy: SharedPeriodPolicy,
    report: &mut SharedPeriodReport,
) -> Result<(), SharedClient3Error> {
    let caps = api
        .capabilities()
        .map_err(|error| failure(SharedInitializationStage::Capabilities, error, report))?;
    report.caps = Some(caps);
    let period = choose_shared_period(caps, sample_rate, policy).map_err(|error| {
        failure(
            SharedInitializationStage::PeriodSelection,
            Error::new(E_INVALIDARG, error.to_string()),
            report,
        )
    })?;
    report.requested_period_frames = Some(period);
    // Never retry an attempted Initialize on this client. Configuration and
    // device failures stay visible; compatibility fallback precedes Initialize.
    api.initialize(period)
        .map_err(|error| failure(SharedInitializationStage::Initialization, error, report))?;
    report.initialization_api = Some(SharedInitializationApi::Client3);
    Ok(())
}

pub(crate) fn initialize_with_client3(
    client: &IAudioClient,
    format: *const WAVEFORMATEX,
    sample_rate: u32,
    policy: SharedPeriodPolicy,
    session_guid: Option<&GUID>,
    report: &mut SharedPeriodReport,
) -> Result<(), SharedClient3Error> {
    report.desired_period_ms = policy.desired_period_ms;
    let client3 = client
        .cast::<IAudioClient3>()
        .map_err(|error| failure(SharedInitializationStage::InterfaceQuery, error, report))?;
    initialize_client3(
        &NativeClient3 {
            client: &client3,
            format,
            session_guid,
        },
        sample_rate,
        policy,
        report,
    )
}

pub(crate) fn record_current_period(client: &IAudioClient, report: &mut SharedPeriodReport) {
    let result = client.cast::<IAudioClient3>().and_then(|client3| {
        let mut format = std::ptr::null_mut();
        let mut period = 0;
        let result = unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut format, &mut period) };
        let _format = TaskMemory(format);
        result.map(|()| period)
    });
    match result {
        Ok(period) => report.actual_period_frames = Some(period),
        Err(error) => report.actual_period_hresult = Some(error.code().0),
    }
}

#[cfg(test)]
mod tests;
