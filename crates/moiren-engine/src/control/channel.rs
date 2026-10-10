use super::runtime::{ParamState, ScheduledEvent};
use super::{ControlError, FloatRamp, ParamSpec, ParameterRuntime};
use moiren_core::protocol::{ApplyAt, ControlReply, ParamValue, ParameterRequest, ReplyCode};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub struct ControlPort {
    specs: Arc<[ParamSpec]>,
    tx: Producer<ScheduledEvent>,
    replies: Consumer<ControlReply>,
    revision: u64,
    epoch: u64,
    last_frame: u64,
    horizon_frames: u64,
    retired: Arc<AtomicBool>,
}
/// Construct on a non-RT thread. Both endpoints must be destroyed after RT stops.
pub fn parameter_channel(
    specs: Vec<ParamSpec>,
    revision: u64,
    epoch: u64,
    capacity: usize,
    horizon_frames: u64,
) -> Result<(ControlPort, ParameterRuntime), ControlError> {
    if capacity == 0 || horizon_frames == 0 || specs.iter().any(|s| !s.accepts(s.initial, 0)) {
        return Err(ControlError::InvalidConfiguration);
    }
    for (i, spec) in specs.iter().enumerate() {
        if specs[..i].iter().any(|s| s.key == spec.key) {
            return Err(ControlError::DuplicateParameter);
        }
    }
    let states = specs
        .iter()
        .map(|s| match s.initial {
            ParamValue::Float(v) => ParamState::Float(FloatRamp::constant(v)),
            value => ParamState::Discrete(value),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let specs: Arc<[ParamSpec]> = specs.into();
    let (tx, rx) = RingBuffer::new(capacity);
    let (reply_tx, reply_rx) = RingBuffer::new(capacity);
    let retired = Arc::new(AtomicBool::new(false));
    Ok((
        ControlPort {
            specs: Arc::clone(&specs),
            tx,
            replies: reply_rx,
            revision,
            epoch,
            last_frame: 0,
            horizon_frames,
            retired: Arc::clone(&retired),
        },
        ParameterRuntime {
            specs,
            states,
            rx,
            replies: reply_tx,
            revision,
            epoch,
            retired,
        },
    ))
}

impl ControlPort {
    /// `observed_frame` is the worker's latest epoch-matched RT timeline snapshot.
    /// FIFO is strictly time ordered; rejected submissions consume no queue slot.
    /// Long automation must remain in the control scheduler until within horizon.
    pub fn submit(&mut self, request: ParameterRequest, observed_frame: u64) -> ControlReply {
        let code = self
            .enqueue(request, observed_frame)
            .err()
            .unwrap_or(ReplyCode::Accepted);
        ControlReply {
            request_id: request.request_id,
            plan_revision: self.revision,
            timeline_epoch: self.epoch,
            code,
            effective_frame: 0,
        }
    }
    fn enqueue(&mut self, request: ParameterRequest, observed: u64) -> Result<(), ReplyCode> {
        if self.retired.load(Ordering::Acquire) || self.tx.is_abandoned() {
            return Err(ReplyCode::StaleRevision);
        }
        if request.plan_revision != self.revision {
            return Err(ReplyCode::StaleRevision);
        }
        if request.timeline_epoch != self.epoch {
            return Err(ReplyCode::StaleEpoch);
        }
        let slot = self
            .specs
            .iter()
            .position(|s| s.key == request.target)
            .ok_or(ReplyCode::UnknownParameter)?;
        if !self.specs[slot].accepts(request.value, request.ramp_frames) {
            return Err(ReplyCode::InvalidValue);
        }
        let at = match request.at {
            ApplyAt::NextBlock => observed,
            ApplyAt::Frame(frame) => frame,
        };
        if at == u64::MAX || at > observed.saturating_add(self.horizon_frames) {
            return Err(ReplyCode::InvalidTime);
        }
        if at < self.last_frame {
            return Err(ReplyCode::OutOfOrder);
        }
        self.tx
            .push(ScheduledEvent {
                slot,
                request_id: request.request_id,
                at,
                value: request.value,
                ramp_frames: request.ramp_frames,
            })
            .map_err(|_| ReplyCode::QueueFull)?;
        self.last_frame = at;
        Ok(())
    }
    pub fn poll_applied(&mut self) -> Option<ControlReply> {
        self.replies.pop().ok()
    }
}
