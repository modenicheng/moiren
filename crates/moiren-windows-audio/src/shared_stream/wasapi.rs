use super::format::PreparedSharedFormat;
use super::{
    SharedFallbackReason, SharedInitializationApi, SharedInitializationFailure,
    SharedInitializationStage, SharedPeriodCaps, SharedPeriodPolicy, SharedPeriodReport,
    choose_shared_period,
};
use crate::negotiation::{
    EnginePeriodicityPolicy, LegacySharedInit, client3_rejects_format, initialize_legacy_shared,
    periodicity_locked_retry, shared_engine_periods,
};
use crate::owner::TaskMemory;
use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_POINTER},
        Media::Audio::{
            AUDCLNT_E_ENGINE_FORMAT_LOCKED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient,
            IAudioClient3, WAVEFORMATEX,
        },
    },
    core::{Error, GUID, Interface},
};

/// D4: this stack's PERIODICITY_LOCKED policy — retry exactly once with the
/// engine's current period; the professional stack is exact-or-fail (see
/// `crate::negotiation::EnginePeriodicityPolicy`).
const PERIODICITY_POLICY: EnginePeriodicityPolicy = EnginePeriodicityPolicy::EnginePeriodRetry;

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
    let fallback_reason = match stage {
        // The interface query falls back on a missing Client3 alone; the
        // kernel rejection matrix covers the capability-query set.
        SharedInitializationStage::InterfaceQuery if hresult == E_NOINTERFACE => {
            Some(SharedFallbackReason::Client3Unavailable)
        }
        SharedInitializationStage::Capabilities if client3_rejects_format(hresult) => {
            Some(SharedFallbackReason::CapabilityQueryUnsupported)
        }
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
    fn current_period(&self) -> Result<u32, Error> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

struct NativeClient3<'client> {
    client: &'client IAudioClient3,
    format: *const WAVEFORMATEX,
    session_guid: Option<&'client GUID>,
}

impl Client3Api for NativeClient3<'_> {
    fn capabilities(&self) -> Result<SharedPeriodCaps, Error> {
        // The owner retains its validated format and configured client through
        // preparation. This query follows SetClientProperties for each stream.
        // SAFETY: the owner-held descriptor is complete for this query.
        let periods = unsafe { shared_engine_periods(self.client, self.format)? };
        Ok(SharedPeriodCaps {
            default_frames: periods.default_frames,
            fundamental_frames: periods.fundamental_frames,
            min_frames: periods.min_frames,
            max_frames: periods.max_frames,
        })
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

    fn current_period(&self) -> Result<u32, Error> {
        let mut format = std::ptr::null_mut();
        let mut period = 0;
        let queried = unsafe {
            self.client
                .GetCurrentSharedModeEnginePeriod(&mut format, &mut period)
        };
        let _format = TaskMemory(format);
        queried?;
        if format.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        // Both descriptors are owner-held COM allocations. A period-only
        // renegotiation must not silently reinterpret a different engine format.
        let same_format = unsafe {
            let expected_size = size_of::<WAVEFORMATEX>() + usize::from((*self.format).cbSize);
            let current_size = size_of::<WAVEFORMATEX>() + usize::from((*format).cbSize);
            expected_size == current_size
                && std::slice::from_raw_parts(self.format.cast::<u8>(), expected_size)
                    == std::slice::from_raw_parts(format.cast::<u8>(), current_size)
        };
        if !same_format {
            return Err(Error::from_hresult(AUDCLNT_E_ENGINE_FORMAT_LOCKED));
        }
        Ok(period)
    }
}

fn initialize_client3(
    api: &impl Client3Api,
    sample_rate: u32,
    policy: SharedPeriodPolicy,
    locked_policy: EnginePeriodicityPolicy,
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
    if let Err(error) = api.initialize(period) {
        let initial_failure = failure(SharedInitializationStage::Initialization, error, report);
        // Only PERIODICITY_LOCKED may retry, and only under this stack's D4
        // policy; every other initialization failure stays terminal.
        if !periodicity_locked_retry(locked_policy, initial_failure.error.code()) {
            return Err(initial_failure);
        }
        let current = match api.current_period() {
            Ok(current) => current,
            Err(error) => {
                report.actual_period_hresult = Some(error.code().0);
                return Err(initial_failure);
            }
        };
        if current == period
            || current < caps.min_frames
            || current > caps.max_frames
            || !current.is_multiple_of(caps.fundamental_frames)
        {
            return Err(initial_failure);
        }
        // PERIODICITY_LOCKED did not initialize a stream. Retry exactly once
        // with the matching format/current engine period; other failures remain terminal.
        api.initialize(current)
            .map_err(|error| failure(SharedInitializationStage::Initialization, error, report))?;
        report.actual_period_frames = Some(current);
        report.fallback_reason = Some(SharedFallbackReason::EnginePeriodLocked);
    }
    report.initialization_api = Some(SharedInitializationApi::Client3);
    Ok(())
}

fn initialize_with_client3(
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
        PERIODICITY_POLICY,
        report,
    )
}

pub(crate) fn initialize_shared(
    client: &IAudioClient,
    format: &PreparedSharedFormat,
    policy: SharedPeriodPolicy,
    session_guid: &GUID,
    report: &mut SharedPeriodReport,
) -> Result<(), SharedClient3Error> {
    report.mix_format = Some(format.mix);
    report.client_format = Some(format.client());
    report.windows_auto_conversion = format.windows_auto_conversion;
    report.desired_period_ms = policy.desired_period_ms;
    let rate = format.client().sample_rate;
    let auto_convert = format.windows_auto_conversion;
    if auto_convert {
        // Client3 does not accept conversion flags. Keep the native low-latency
        // path separate rather than silently changing its format or retry rules.
        report.fallback_reason = Some(SharedFallbackReason::WindowsFormatConversion);
    } else {
        match initialize_with_client3(
            client,
            format.as_ptr(),
            rate,
            policy,
            Some(session_guid),
            report,
        ) {
            Ok(()) => {
                record_current_period(client, rate, report);
                return Ok(());
            }
            Err(error) => match error.fallback_reason {
                Some(reason) => report.fallback_reason = Some(reason),
                None => return Err(error),
            },
        }
    }
    // IsFormatSupported does not account for AUTOCONVERTPCM; Initialize is
    // authoritative. Never swallow invalidation, permissions or device-in-use.
    // D4: this stack opts out of persisted session state and requests Windows
    // conversion only when the client format differs from the endpoint mix;
    // the professional stack passes its own combination.
    // SAFETY: the descriptor is the owner-held allocation behind `format`.
    unsafe {
        initialize_legacy_shared(
            client,
            format.as_ptr(),
            Some(session_guid),
            LegacySharedInit {
                auto_convert,
                nopersist: true,
            },
        )
    }
    .map_err(|error| SharedClient3Error {
        stage: "Initialize(Shared compatibility)",
        fallback_reason: None,
        error,
    })?;
    report.initialization_api = Some(SharedInitializationApi::LegacyClient);
    record_current_period(client, rate, report);
    Ok(())
}

fn record_period(
    frames: u32,
    engine_rate: u32,
    client_rate: u32,
    report: &mut SharedPeriodReport,
) -> Result<(), Error> {
    if frames == 0 || engine_rate == 0 || client_rate == 0 {
        return Err(E_INVALIDARG.into());
    }
    let normalized = (u64::from(frames) * u64::from(client_rate)).div_ceil(u64::from(engine_rate));
    let normalized = u32::try_from(normalized).map_err(|_| Error::from_hresult(E_INVALIDARG))?;
    report.engine_period_frames = Some(frames);
    report.engine_sample_rate = Some(engine_rate);
    report.actual_period_frames = Some(normalized);
    Ok(())
}

fn record_current_period(client: &IAudioClient, client_rate: u32, report: &mut SharedPeriodReport) {
    let result = client.cast::<IAudioClient3>().and_then(|client3| {
        let mut format = std::ptr::null_mut();
        let mut period = 0;
        let result = unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut format, &mut period) };
        let _format = TaskMemory(format);
        result?;
        if format.is_null() {
            return Err(E_POINTER.into());
        }
        // This query returns engine-rate frames, not client-rate frames when
        // AUTOCONVERTPCM is active. Round up to avoid understating queue budgets.
        let base = unsafe { format.read_unaligned() };
        record_period(period, base.nSamplesPerSec, client_rate, report)
    });
    match result {
        Ok(()) => {}
        Err(error) => report.actual_period_hresult = Some(error.code().0),
    }
}

#[cfg(test)]
mod tests;
