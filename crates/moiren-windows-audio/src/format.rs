//! Shared-mode stream data types: endpoint mix/client descriptors, period
//! reports and their serde shapes. Conversion policy lives with the callers.
use serde::Serialize;

/// Endpoint mix and client formats can differ on the Windows conversion path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SharedFormatInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub container_bits: u16,
    pub valid_bits: Option<u16>,
    pub channel_mask: Option<u32>,
    pub floating_point: bool,
}

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
    WindowsFormatConversion,
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
    pub mix_format: Option<SharedFormatInfo>,
    pub client_format: Option<SharedFormatInfo>,
    pub windows_auto_conversion: bool,
    pub desired_period_ms: Option<f64>,
    pub requested_period_frames: Option<u32>,
    /// Period normalized to the client format's sample rate, rounded upward.
    pub actual_period_frames: Option<u32>,
    /// Raw engine period, before normalization into client-rate frames.
    pub engine_period_frames: Option<u32>,
    pub engine_sample_rate: Option<u32>,
    pub caps: Option<SharedPeriodCaps>,
    pub initialization_api: Option<SharedInitializationApi>,
    pub fallback_reason: Option<SharedFallbackReason>,
    pub client3_failure: Option<SharedInitializationFailure>,
    pub actual_period_hresult: Option<i32>,
}
