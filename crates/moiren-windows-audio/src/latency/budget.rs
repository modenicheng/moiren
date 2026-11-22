//! Preparation-only admission budgets. Device timing and jitter assumptions are
//! explicit inputs; these bounds do not establish measured end-to-end latency.
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NativeStreamTiming {
    pub sample_rate: u32,
    pub period_frames: Option<u32>,
    pub capacity_frames: u32,
}

impl NativeStreamTiming {
    pub(crate) fn validate(self) -> Result<(), LatencyBudgetError> {
        if self.sample_rate == 0
            || self.capacity_frames == 0
            || self.period_frames.is_some_and(|period| period == 0)
        {
            return Err(LatencyBudgetError::InvalidTiming);
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderQueuePolicy {
    #[default]
    FullCapacity,
    Target {
        frames: u32,
        wake_margin_frames: u32,
        callback_budget_frames: u32,
    },
}

impl RenderQueuePolicy {
    pub fn validate(self) -> Result<(), LatencyBudgetError> {
        if let Self::Target {
            frames,
            callback_budget_frames,
            ..
        } = self
            && (frames == 0 || callback_budget_frames == 0)
        {
            return Err(LatencyBudgetError::InvalidRenderTarget);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PreparedRenderQueue {
    pub capacity_frames: u32,
    pub target_frames: u32,
    /// None preserves legacy full-capacity behavior without claiming an
    /// admitted deadline budget. Custom targets require an actual period.
    pub deadline_budget_frames: Option<u32>,
}

impl PreparedRenderQueue {
    pub fn demand(self, padding_frames: u32) -> Result<u32, LatencyBudgetError> {
        target_demand(self.capacity_frames, padding_frames, self.target_frames)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PreparedInputQueue {
    pub capacity_frames: u32,
    pub prime_frames: u32,
    pub servo_reference_frames: u32,
    pub low_water_frames: u32,
    pub high_water_frames: u32,
    pub modeled_minimum_frames: Option<u32>,
}

impl PreparedInputQueue {
    /// Legacy live-capture residency, without a negotiated delivery guarantee.
    pub const fn compatible() -> Self {
        Self {
            capacity_frames: 8192,
            prime_frames: 2048,
            servo_reference_frames: 2048,
            low_water_frames: 2,
            high_water_frames: 8191,
            modeled_minimum_frames: None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyBudgetPolicy {
    #[default]
    Compatible,
    Custom {
        input_capacity_frames: u32,
        input_target_frames: u32,
        input_jitter_frames: u32,
        render: RenderQueuePolicy,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PreparedLatencyBudget {
    pub input_timing: NativeStreamTiming,
    pub output_timing: NativeStreamTiming,
    pub input: PreparedInputQueue,
    pub render: PreparedRenderQueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LatencyBudgetError {
    #[error("native stream timing requires a nonzero rate, capacity and optional period")]
    InvalidTiming,
    #[error("custom queue budgets require actual device periods")]
    MissingActualPeriod,
    #[error("virtual input targets require an explicit packet delivery bound")]
    MissingDeliveryBound,
    #[error("input delivery bounds, consumer rate and demand must be nonzero")]
    InvalidDeliveryBound,
    #[error("render target must fit capacity and have a positive callback budget")]
    InvalidRenderTarget,
    #[error("render target cannot cover period, wake margin and callback budget")]
    InsufficientRenderBudget,
    #[error("input target must be at least two frames and below its high water and capacity")]
    InvalidInputTarget,
    #[error("input target cannot cover render demand, capture gap, jitter and lookahead")]
    InsufficientInputBudget,
    #[error("latency budget frame conversion or addition overflowed")]
    ArithmeticOverflow,
    #[error("padding exceeds capacity")]
    InvalidPadding,
}

pub fn target_demand(
    capacity_frames: u32,
    padding_frames: u32,
    target_frames: u32,
) -> Result<u32, LatencyBudgetError> {
    if capacity_frames == 0 || target_frames == 0 || target_frames > capacity_frames {
        return Err(LatencyBudgetError::InvalidRenderTarget);
    }
    if padding_frames > capacity_frames {
        return Err(LatencyBudgetError::InvalidPadding);
    }
    Ok(target_frames.saturating_sub(padding_frames))
}

pub fn prepare_render_queue(
    timing: NativeStreamTiming,
    policy: RenderQueuePolicy,
) -> Result<PreparedRenderQueue, LatencyBudgetError> {
    timing.validate()?;
    policy.validate()?;
    let (target_frames, deadline_budget_frames) = match policy {
        RenderQueuePolicy::FullCapacity => (timing.capacity_frames, None),
        RenderQueuePolicy::Target {
            frames,
            wake_margin_frames,
            callback_budget_frames,
        } => {
            if frames > timing.capacity_frames {
                return Err(LatencyBudgetError::InvalidRenderTarget);
            }
            let deadline = timing
                .period_frames
                .ok_or(LatencyBudgetError::MissingActualPeriod)?
                .checked_add(wake_margin_frames)
                .and_then(|frames| frames.checked_add(callback_budget_frames))
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            if frames < deadline {
                return Err(LatencyBudgetError::InsufficientRenderBudget);
            }
            (frames, Some(deadline))
        }
    };
    Ok(PreparedRenderQueue {
        capacity_frames: timing.capacity_frames,
        target_frames,
        deadline_budget_frames,
    })
}

pub fn prepare_latency_budget(
    input_timing: NativeStreamTiming,
    output_timing: NativeStreamTiming,
    policy: LatencyBudgetPolicy,
) -> Result<PreparedLatencyBudget, LatencyBudgetError> {
    input_timing.validate()?;
    output_timing.validate()?;
    let (input, render) = match policy {
        LatencyBudgetPolicy::Compatible => (
            PreparedInputQueue::compatible(),
            prepare_render_queue(output_timing, RenderQueuePolicy::FullCapacity)?,
        ),
        LatencyBudgetPolicy::Custom {
            input_capacity_frames,
            input_target_frames,
            input_jitter_frames,
            render,
        } => {
            let input_period = input_timing
                .period_frames
                .ok_or(LatencyBudgetError::MissingActualPeriod)?;
            output_timing
                .period_frames
                .ok_or(LatencyBudgetError::MissingActualPeriod)?;
            let render = prepare_render_queue(output_timing, render)?;
            // Bound startup and late-wake demand by the render target. Capture
            // gaps are modeled as one actual period plus the declared jitter.
            let converted = (u64::from(render.target_frames) * u64::from(input_timing.sample_rate))
                .div_ceil(u64::from(output_timing.sample_rate));
            let low_water = u32::try_from(converted)
                .ok()
                .and_then(|frames| frames.checked_add(2))
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            let minimum = low_water
                .checked_add(input_period)
                .and_then(|frames| frames.checked_add(input_jitter_frames))
                .ok_or(LatencyBudgetError::ArithmeticOverflow)?;
            let high_water = input_capacity_frames
                .checked_sub(input_period)
                .ok_or(LatencyBudgetError::InvalidInputTarget)?;
            if input_target_frames < 2 || input_target_frames >= high_water {
                return Err(LatencyBudgetError::InvalidInputTarget);
            }
            if input_target_frames < minimum {
                return Err(LatencyBudgetError::InsufficientInputBudget);
            }
            (
                PreparedInputQueue {
                    capacity_frames: input_capacity_frames,
                    prime_frames: input_target_frames,
                    servo_reference_frames: input_target_frames,
                    low_water_frames: low_water,
                    high_water_frames: high_water,
                    modeled_minimum_frames: Some(minimum),
                },
                render,
            )
        }
    };
    Ok(PreparedLatencyBudget {
        input_timing,
        output_timing,
        input,
        render,
    })
}
