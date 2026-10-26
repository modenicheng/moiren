//! VAD carrier residency is explicit; spare capacity never chooses its target.
use crate::latency::{
    LatencyBudgetError, PreparedInputQueue,
    transport::{InputDeliveryTiming, TransportInputPolicy, prepare_transport_input},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VirtualBufferOptions {
    pub input: TransportInputPolicy,
    pub delivery: Option<InputDeliveryTiming>,
}

impl VirtualBufferOptions {
    pub fn validate(self) -> Result<(), LatencyBudgetError> {
        self.prepare(crate::NATIVE_SAMPLE_RATE, 1)?;
        Ok(())
    }

    /// Resolve against the full Engine consumption burst before activation.
    /// Native packet shape can arrive later when an external client opens.
    pub fn prepare(
        self,
        engine_rate: u32,
        max_demand_frames: u32,
    ) -> Result<Option<PreparedInputQueue>, LatencyBudgetError> {
        if self
            .delivery
            .is_some_and(|d| d.sample_rate != crate::NATIVE_SAMPLE_RATE)
        {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        let queue = prepare_transport_input(
            self.input,
            self.delivery,
            engine_rate,
            max_demand_frames,
            32,
        )?;
        if let (
            TransportInputPolicy::Target {
                capacity_frames,
                target_frames,
                ..
            },
            Some(delivery),
        ) = (self.input, self.delivery)
        {
            let transfer =
                (u64::from(max_demand_frames) * u64::from(crate::NATIVE_SAMPLE_RATE) * 1000)
                    .div_ceil(u64::from(engine_rate) * 998)
                    .checked_add(32)
                    .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            let required = u64::from(target_frames)
                .checked_add(u64::from(delivery.max_packet_frames))
                .and_then(|value| value.checked_add(transfer))
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            if required > u64::from(capacity_frames) {
                return Err(LatencyBudgetError::InsufficientInputBudget);
            }
        }
        Ok((self.input != TransportInputPolicy::Compatible).then_some(queue))
    }

    /// A later client negotiation must fit the bound used for preparation.
    /// Recheck every stream/shape generation before creating native PCM slots.
    pub fn admit_native_packet(self, frames: u32) -> Result<(), LatencyBudgetError> {
        if frames == 0 {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        if self.input == TransportInputPolicy::Compatible {
            return Ok(());
        }
        let delivery = self
            .delivery
            .ok_or(LatencyBudgetError::MissingDeliveryBound)?;
        if frames > delivery.max_packet_frames || frames > delivery.max_gap_frames {
            return Err(LatencyBudgetError::InsufficientInputBudget);
        }
        Ok(())
    }
}
