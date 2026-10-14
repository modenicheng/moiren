//! Shared-mode period selection is portable; COM preparation stays on the owner.
use serde::Serialize;
use thiserror::Error;

#[cfg(windows)]
mod wasapi;
#[cfg(windows)]
pub(crate) use wasapi::{initialize_with_client3, record_current_period};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SharedPeriodCaps {
    pub default_frames: u32,
    pub fundamental_frames: u32,
    pub min_frames: u32,
    pub max_frames: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedInitializationApi {
    LegacyClient,
    Client3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedFallbackReason {
    Client3Unavailable,
    CapabilityQueryUnsupported,
    EnginePeriodLocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedInitializationStage {
    InterfaceQuery,
    Capabilities,
    PeriodSelection,
    Initialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SharedInitializationFailure {
    pub stage: SharedInitializationStage,
    pub hresult: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
pub struct SharedPeriodReport {
    pub desired_period_ms: Option<f64>,
    pub requested_period_frames: Option<u32>,
    pub actual_period_frames: Option<u32>,
    pub caps: Option<SharedPeriodCaps>,
    pub initialization_api: Option<SharedInitializationApi>,
    pub fallback_reason: Option<SharedFallbackReason>,
    pub client3_failure: Option<SharedInitializationFailure>,
    pub actual_period_hresult: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SharedPeriodPolicy {
    /// None requests the device default. Positive values are rounded to a legal
    /// period, so the effective request may exceed this desired time.
    pub desired_period_ms: Option<f64>,
}

impl Default for SharedPeriodPolicy {
    fn default() -> Self {
        Self {
            desired_period_ms: Some(5.0),
        }
    }
}

impl SharedPeriodPolicy {
    pub fn validate(self) -> Result<(), SharedPeriodError> {
        if self
            .desired_period_ms
            .is_some_and(|milliseconds| !milliseconds.is_finite() || milliseconds <= 0.0)
        {
            return Err(SharedPeriodError::InvalidDesiredPeriod);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SharedPeriodError {
    #[error("shared period capabilities contain invalid bounds or no legal multiple")]
    InvalidCapabilities,
    #[error("desired shared period is not finite and positive")]
    InvalidDesiredPeriod,
    #[error("desired shared period overflows frame conversion")]
    DesiredPeriodOverflow,
    #[error("sample rate must be nonzero")]
    InvalidSampleRate,
}

pub fn choose_shared_period(
    caps: SharedPeriodCaps,
    sample_rate: u32,
    policy: SharedPeriodPolicy,
) -> Result<u32, SharedPeriodError> {
    policy.validate()?;
    if sample_rate == 0 {
        return Err(SharedPeriodError::InvalidSampleRate);
    }
    if caps.fundamental_frames == 0
        || caps.min_frames == 0
        || caps.min_frames > caps.max_frames
        || !(caps.min_frames..=caps.max_frames).contains(&caps.default_frames)
        || !caps.default_frames.is_multiple_of(caps.fundamental_frames)
    {
        return Err(SharedPeriodError::InvalidCapabilities);
    }
    let fundamental = u64::from(caps.fundamental_frames);
    let minimum = u64::from(caps.min_frames).div_ceil(fundamental) * fundamental;
    let maximum = u64::from(caps.max_frames) / fundamental * fundamental;
    if minimum > maximum {
        return Err(SharedPeriodError::InvalidCapabilities);
    }
    let Some(milliseconds) = policy.desired_period_ms else {
        return Ok(caps.default_frames);
    };
    let desired = (f64::from(sample_rate) * milliseconds / 1000.0).ceil();
    if !desired.is_finite() || desired > f64::from(u32::MAX) {
        return Err(SharedPeriodError::DesiredPeriodOverflow);
    }
    let rounded = (desired as u64).div_ceil(fundamental) * fundamental;
    Ok(rounded.clamp(minimum, maximum) as u32)
}
