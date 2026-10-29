//! Process-loopback delivery policy, prepared before native audio admission.
use crate::{
    capture::CaptureError,
    latency::{
        PreparedInputQueue,
        transport::{InputDeliveryTiming, TransportInputPolicy, prepare_transport_input},
    },
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessBridgeOptions {
    pub input: TransportInputPolicy,
    pub delivery: Option<InputDeliveryTiming>,
    pub engine_sample_rate: u32,
    /// Maximum complete consumption burst, including startup render demand.
    pub max_demand_frames: u32,
    /// Requested sample-age enforcement, not nominal queue duration. Process
    /// capture cannot currently qualify sample age, so every Some is refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_limit_100ns: Option<u64>,
    /// Maximum host elapsed time since packet publication started. This does
    /// not establish native sample age or end-to-end presentation latency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_ingress_elapsed_100ns: Option<u64>,
}

impl Default for ProcessBridgeOptions {
    fn default() -> Self {
        Self {
            input: TransportInputPolicy::Compatible,
            delivery: None,
            engine_sample_rate: crate::NATIVE_SAMPLE_RATE,
            max_demand_frames: 256,
            age_limit_100ns: None,
            max_ingress_elapsed_100ns: None,
        }
    }
}

impl ProcessBridgeOptions {
    /// Replace caller estimates with the owner's complete prepared demand.
    /// Graph segmentation does not reduce the reserve needed for one wake.
    pub fn for_consumer(
        self,
        engine_sample_rate: u32,
        max_demand_frames: u32,
    ) -> Result<Self, CaptureError> {
        let effective = Self {
            engine_sample_rate,
            max_demand_frames,
            ..self
        };
        effective.prepare()?;
        Ok(effective)
    }

    pub fn prepare(self) -> Result<PreparedInputQueue, CaptureError> {
        // Never let Compatible silently ignore age intent, or attach a limit
        // to a bridge whose Process packet stamps cannot establish sample age.
        if self.age_limit_100ns.is_some() {
            return Err(CaptureError::ProcessAgeUnsupported);
        }
        if self.max_ingress_elapsed_100ns == Some(0) {
            return Err(crate::clock_bridge::ClockBridgeError::InvalidConfig.into());
        }
        if !moiren_engine::audio_config::SAMPLE_RATES.contains(&self.engine_sample_rate) {
            return Err(crate::clock_bridge::ClockBridgeError::InvalidConfig.into());
        }
        // Windows converts the virtual client's PCM to our fixed 48 kHz
        // bridge format. Its nominal rate is not a physical device period.
        if self
            .delivery
            .is_some_and(|delivery| delivery.sample_rate != crate::NATIVE_SAMPLE_RATE)
        {
            return Err(crate::latency::LatencyBudgetError::InvalidDeliveryBound.into());
        }
        Ok(prepare_transport_input(
            self.input,
            self.delivery,
            self.engine_sample_rate,
            self.max_demand_frames,
            2,
        )?)
    }

    pub(crate) fn clock_config(
        self,
    ) -> Result<crate::clock_bridge::ClockBridgeConfig, CaptureError> {
        let input = self.prepare()?;
        if self.input == TransportInputPolicy::Compatible {
            // The legacy source has no running high-water recovery. Attaching
            // prepared watermarks would change that behavior even if prime
            // and capacity happened to be the same.
            return Ok(crate::clock_bridge::ClockBridgeConfig {
                output_sample_rate: self.engine_sample_rate,
                detect_position_gaps: false,
                max_ingress_elapsed_100ns: self.max_ingress_elapsed_100ns,
                ..Default::default()
            });
        }
        Ok(crate::clock_bridge::ClockBridgeConfig {
            detect_position_gaps: false,
            max_ingress_elapsed_100ns: self.max_ingress_elapsed_100ns,
            ..crate::clock_bridge::ClockBridgeConfig::with_engine_input(
                crate::NATIVE_SAMPLE_RATE,
                self.engine_sample_rate,
                2,
                input,
            )
        })
    }
}
