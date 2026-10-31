//! VAD carrier residency is explicit; spare capacity never chooses its target.
use crate::latency::{
    LatencyBudgetError, PreparedInputQueue,
    transport::{InputDeliveryTiming, TransportInputPolicy, prepare_transport_input},
};
use serde::{Deserialize, Serialize};

const FIR_LOOKAHEAD_FRAMES: u32 = 32;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VirtualBufferOptions {
    pub input: TransportInputPolicy,
    pub delivery: Option<InputDeliveryTiming>,
    /// Maximum complete Engine periods repaid at one virtual-clock wake.
    /// None preserves the legacy eight-period ceiling; this is not residency.
    pub recovery_cap_periods: Option<u64>,
}

/// Immutable, preparation-side admission evidence, not measured latency.
/// Declared delivery bounds never masquerade as a negotiated native packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PreparedVirtualBudget {
    pub requested: VirtualBufferOptions,
    pub queue: Option<PreparedInputQueue>,
    pub engine_sample_rate: u32,
    pub engine_block_frames: u32,
    pub engine_burst_frames: u32,
    /// Native 48 kHz transfer including the +/-2000 ppm SRC ratio bound.
    pub native_transfer_frames: u32,
    pub fir_lookahead_frames: u32,
    /// Explicit jitter reserve; None for the unqualified Compatible policy.
    pub wake_allowance_frames: Option<u32>,
    pub recovery_cap_periods: u64,
    /// Unknown until an actual client negotiation is supplied and admitted.
    pub native_packet_frames: Option<u32>,
}

impl PreparedVirtualBudget {
    pub fn admit_native_packet(mut self, frames: u32) -> Result<Self, LatencyBudgetError> {
        self.requested.admit_native_packet(frames)?;
        self.native_packet_frames = Some(frames);
        Ok(self)
    }
}

impl VirtualBufferOptions {
    /// Restrict configurable recovery to the already-supported finite ceiling.
    /// Larger requests must not quietly become eight-period recovery.
    pub fn admitted_recovery_cap_periods(self) -> Result<u64, LatencyBudgetError> {
        let periods = self.recovery_cap_periods.unwrap_or(super::DEBT_CAP_PERIODS);
        if !(1..=super::DEBT_CAP_PERIODS).contains(&periods) {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        Ok(periods)
    }

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
        self.admitted_recovery_cap_periods()?;
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

    /// Admit the *whole* configured repayment burst, never just its first block.
    /// Unsafe target/cap combinations fail instead of implicitly raising target
    /// or retaining the legacy 2048-frame Graph256 repayment under a small cap.
    pub fn prepare_recovery(
        self,
        engine_rate: u32,
        engine_block_frames: u32,
    ) -> Result<PreparedVirtualBudget, LatencyBudgetError> {
        let demand = u64::from(engine_block_frames)
            .checked_mul(self.admitted_recovery_cap_periods()?)
            .and_then(|frames| u32::try_from(frames).ok())
            .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
        self.prepare_budget(engine_rate, engine_block_frames, demand)
    }

    /// The physical master may supply a different complete wake demand. The
    /// recovery ceiling affects only virtual scheduling, not that native burst.
    pub fn prepare_budget(
        self,
        engine_rate: u32,
        engine_block_frames: u32,
        max_demand_frames: u32,
    ) -> Result<PreparedVirtualBudget, LatencyBudgetError> {
        if engine_block_frames == 0 || max_demand_frames < engine_block_frames {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        let mut queue = self.prepare(engine_rate, max_demand_frames)?;
        let transfer = (u64::from(max_demand_frames) * u64::from(crate::NATIVE_SAMPLE_RATE) * 1000)
            .div_ceil(u64::from(engine_rate) * 998);
        let transfer =
            u32::try_from(transfer).map_err(|_| LatencyBudgetError::ArithmeticOverflow)?;
        let wake_allowance_frames = match self.input {
            TransportInputPolicy::Compatible => None,
            TransportInputPolicy::Target { jitter_frames, .. } => Some(jitter_frames),
        };
        if let Some(queue) = &mut queue {
            let delivery = self
                .delivery
                .ok_or(LatencyBudgetError::MissingDeliveryBound)?;
            queue.low_water_frames = transfer
                .checked_add(FIR_LOOKAHEAD_FRAMES)
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            // Each direction also needs a FIR-length residual after its SRC
            // transfer, even when a declared packet/gap happens to be tiny.
            let reserve = delivery
                .max_gap_frames
                .checked_add(wake_allowance_frames.unwrap_or(0))
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?
                .max(FIR_LOOKAHEAD_FRAMES);
            let minimum = queue
                .low_water_frames
                .checked_add(reserve)
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            if queue.prime_frames < minimum {
                return Err(LatencyBudgetError::InsufficientInputBudget);
            }
            queue.modeled_minimum_frames = Some(minimum);
        }
        Ok(PreparedVirtualBudget {
            requested: self,
            queue,
            engine_sample_rate: engine_rate,
            engine_block_frames,
            engine_burst_frames: max_demand_frames,
            native_transfer_frames: transfer,
            fir_lookahead_frames: FIR_LOOKAHEAD_FRAMES,
            wake_allowance_frames,
            recovery_cap_periods: self.admitted_recovery_cap_periods()?,
            native_packet_frames: None,
        })
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
