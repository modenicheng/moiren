//! Preparation-only input budgets for transports without a physical period.
//!
//! Delivery bounds are assumptions to qualify against real packet telemetry.
//! They do not establish device timing or measured end-to-end latency.
use super::{LatencyBudgetError, PreparedInputQueue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryTimingSource {
    NativePacket,
    Empirical,
    /// An explicit experiment assumption that has not yet been measured.
    Declared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDeliveryTiming {
    pub sample_rate: u32,
    /// Largest single publication; capacity reserves this much burst space.
    pub max_packet_frames: u32,
    /// Longest delivery gap expressed in native input frames.
    pub max_gap_frames: u32,
    pub source: DeliveryTimingSource,
}

impl InputDeliveryTiming {
    fn validate(self) -> Result<(), LatencyBudgetError> {
        if self.sample_rate == 0 || self.max_packet_frames == 0 || self.max_gap_frames == 0 {
            return Err(LatencyBudgetError::InvalidDeliveryBound);
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransportInputPolicy {
    #[default]
    Compatible,
    Target {
        capacity_frames: u32,
        target_frames: u32,
        jitter_frames: u32,
    },
}

/// Preparation-time reserve in input-rate frames, not measured latency or a
/// promise about future packet delivery. Shared by admission and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TransportInputBudget {
    pub input_sample_rate: u32,
    pub demand_frames: u32,
    pub delivery_gap_frames: u32,
    pub jitter_frames: u32,
    pub lookahead_frames: u32,
    pub minimum_frames: u32,
}

pub fn transport_input_budget(
    delivery: InputDeliveryTiming,
    consumer_sample_rate: u32,
    max_consumer_demand_frames: u32,
    lookahead_frames: u32,
    jitter_frames: u32,
) -> Result<TransportInputBudget, LatencyBudgetError> {
    delivery.validate()?;
    if consumer_sample_rate == 0 || max_consumer_demand_frames == 0 {
        return Err(LatencyBudgetError::InvalidDeliveryBound);
    }
    let demand = (u64::from(max_consumer_demand_frames) * u64::from(delivery.sample_rate))
        .div_ceil(u64::from(consumer_sample_rate));
    let demand_frames =
        u32::try_from(demand).map_err(|_| LatencyBudgetError::ArithmeticOverflow)?;
    // The interpolation pair is required even without extra FIR lookahead.
    let lookahead_frames = lookahead_frames.max(2);
    let minimum_frames = demand_frames
        .checked_add(lookahead_frames)
        .and_then(|frames| frames.checked_add(delivery.max_gap_frames))
        .and_then(|frames| frames.checked_add(jitter_frames))
        .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
    Ok(TransportInputBudget {
        input_sample_rate: delivery.sample_rate,
        demand_frames,
        delivery_gap_frames: delivery.max_gap_frames,
        jitter_frames,
        lookahead_frames,
        minimum_frames,
    })
}

/// Resolve normal residency independently of spare ring capacity.
///
/// Consumer demand is the maximum complete startup or callback burst, not the
/// Engine segment size. Otherwise splitting one native demand into small graph
/// calls would incorrectly authorize a smaller input reserve.
pub fn prepare_transport_input(
    policy: TransportInputPolicy,
    delivery: Option<InputDeliveryTiming>,
    consumer_sample_rate: u32,
    max_consumer_demand_frames: u32,
    lookahead_frames: u32,
) -> Result<PreparedInputQueue, LatencyBudgetError> {
    if consumer_sample_rate == 0 || max_consumer_demand_frames == 0 {
        return Err(LatencyBudgetError::InvalidDeliveryBound);
    }
    if let Some(delivery) = delivery {
        delivery.validate()?;
    }
    let TransportInputPolicy::Target {
        capacity_frames,
        target_frames,
        jitter_frames,
    } = policy
    else {
        return Ok(PreparedInputQueue::compatible());
    };
    let delivery = delivery.ok_or(LatencyBudgetError::MissingDeliveryBound)?;
    let budget = transport_input_budget(
        delivery,
        consumer_sample_rate,
        max_consumer_demand_frames,
        lookahead_frames,
        jitter_frames,
    )?;
    let low_water = budget.demand_frames + budget.lookahead_frames;
    let minimum = budget.minimum_frames;
    let high_water = capacity_frames
        .checked_sub(delivery.max_packet_frames)
        .ok_or(LatencyBudgetError::InvalidInputTarget)?;
    if target_frames < 2 || target_frames >= high_water {
        return Err(LatencyBudgetError::InvalidInputTarget);
    }
    if target_frames < minimum {
        return Err(LatencyBudgetError::InsufficientInputBudget);
    }
    Ok(PreparedInputQueue {
        capacity_frames,
        prime_frames: target_frames,
        servo_reference_frames: target_frames,
        low_water_frames: low_water,
        high_water_frames: high_water,
        modeled_minimum_frames: Some(minimum),
    })
}
