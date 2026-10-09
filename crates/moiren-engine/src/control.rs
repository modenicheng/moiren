//! IPC worker -> one control owner -> bounded SPSC -> RT table -> applied replies.
//! Blocking I/O/decoding/coalescing belongs to the worker, never these RT methods.
use moiren_core::protocol::{
    ApplyAt, ControlReply, ParamValue, ParameterId, ParameterKey, ParameterRequest, ProcessorId,
    ReplyCode,
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamDomain {
    Float { min: f64, max: f64 },
    Int { min: i64, max: i64 },
    Bool,
    Enum { variants: u32 },
}
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub key: ParameterKey,
    pub domain: ParamDomain,
    pub initial: ParamValue,
}
impl ParamSpec {
    fn accepts(&self, value: ParamValue, ramp: u32) -> bool {
        match (self.domain, value) {
            (ParamDomain::Float { min, max }, ParamValue::Float(v)) => {
                min.is_finite()
                    && max.is_finite()
                    && min <= max
                    && v.is_finite()
                    && v >= min
                    && v <= max
            }
            (ParamDomain::Int { min, max }, ParamValue::Int(v)) => {
                ramp == 0 && min <= v && v <= max
            }
            (ParamDomain::Bool, ParamValue::Bool(_)) => ramp == 0,
            (ParamDomain::Enum { variants }, ParamValue::Enum(v)) => ramp == 0 && v < variants,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ControlError {
    #[error("zero capacity/horizon or an initial value outside its domain")]
    InvalidConfiguration,
    #[error("two parameters declare the same processor/parameter key")]
    DuplicateParameter,
}

#[derive(Debug, Clone, Copy)]
pub struct FloatRamp {
    from: f64,
    target: f64,
    elapsed: u32,
    duration: u32,
}
impl FloatRamp {
    pub fn constant(value: f64) -> Self {
        Self {
            from: value,
            target: value,
            elapsed: 0,
            duration: 0,
        }
    }
    fn at_progress(self, progress: u64) -> f64 {
        if self.duration == 0 || progress >= u64::from(self.duration) {
            return self.target;
        }
        let t = progress as f64 / f64::from(self.duration);
        self.from * (1.0 - t) + self.target * t
    }
    /// The first sample advances by one ramp step; sample N reaches the target.
    pub fn sample(self, offset: usize) -> f64 {
        self.at_progress(
            u64::from(self.elapsed)
                .saturating_add(offset as u64)
                .saturating_add(1),
        )
    }
    fn retarget(&mut self, target: f64, duration: u32) {
        self.from = self.at_progress(u64::from(self.elapsed));
        self.target = target;
        self.duration = duration;
        self.elapsed = 0;
    }
    fn advance(&mut self, frames: usize) {
        self.elapsed = (u64::from(self.elapsed).saturating_add(frames as u64))
            .min(u64::from(self.duration)) as u32;
    }
}
#[derive(Debug, Clone, Copy)]
enum ParamState {
    Float(FloatRamp),
    Discrete(ParamValue),
}
#[derive(Debug, Clone, Copy)]
struct ScheduledEvent {
    slot: usize,
    request_id: u64,
    at: u64,
    value: ParamValue,
    ramp_frames: u32,
}

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
pub struct ParameterRuntime {
    specs: Arc<[ParamSpec]>,
    states: Box<[ParamState]>,
    rx: Consumer<ScheduledEvent>,
    replies: Producer<ControlReply>,
    revision: u64,
    epoch: u64,
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

pub(crate) struct ParameterBindings {
    specs: Arc<[ParamSpec]>,
    slots: Box<[(ParameterId, usize)]>,
}
#[derive(Clone, Copy)]
pub struct ProcessParameters<'a> {
    states: &'a [ParamState],
    slots: &'a [(ParameterId, usize)],
    specs: &'a [ParamSpec],
}
impl ProcessParameters<'_> {
    /// Schema metadata for non-RT processor preparation. Binding the correct
    /// type alone does not guarantee that later automation stays in DSP range.
    pub fn domain(&self, id: ParameterId) -> Option<ParamDomain> {
        let (_, slot) = self.slots.iter().find(|(key, _)| *key == id)?;
        Some(self.specs[*slot].domain)
    }
    pub fn float(&self, id: ParameterId) -> Option<FloatRamp> {
        let (_, slot) = self.slots.iter().find(|(key, _)| *key == id)?;
        match self.states[*slot] {
            ParamState::Float(ramp) => Some(ramp),
            _ => None,
        }
    }
    pub fn discrete(&self, id: ParameterId) -> Option<ParamValue> {
        let (_, slot) = self.slots.iter().find(|(key, _)| *key == id)?;
        match self.states[*slot] {
            ParamState::Discrete(value) => Some(value),
            _ => None,
        }
    }
}
impl ParameterRuntime {
    /// Stop-side, non-RT reclamation after the engine has returned to control.
    /// Prevent new submissions and reject queued requests without rendering.
    /// Drain `ControlPort::poll_applied` and retry while the return value is
    /// nonzero: a full reply queue must never silently discard accepted work.
    pub fn retire_and_reject_pending(&mut self) -> usize {
        self.retire();
        self.reject_pending()
    }
    pub(crate) fn schema(&self) -> Arc<[ParamSpec]> {
        Arc::clone(&self.specs)
    }
    pub(crate) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }
    pub(crate) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }
    pub(crate) fn copy_states_from(&mut self, source: &Self, slots: &[(usize, usize)]) {
        for &(old, new) in slots {
            self.states[new] = source.states[old];
        }
    }
    /// Non-RT reclamation. Keep the retired runtime alive and drain the old
    /// ControlPort if reply backpressure leaves requests to reject.
    pub(crate) fn reject_pending(&mut self) -> usize {
        while !self.rx.is_empty() {
            if !self.replies.is_abandoned() && self.replies.is_full() {
                break;
            }
            let event = self.rx.pop().expect("single retired consumer");
            if !self.replies.is_abandoned() {
                self.replies
                    .push(ControlReply {
                        request_id: event.request_id,
                        plan_revision: self.revision,
                        timeline_epoch: self.epoch,
                        code: ReplyCode::StaleRevision,
                        effective_frame: 0,
                    })
                    .expect("single producer reserved reply capacity");
            }
        }
        self.rx.slots()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub(crate) fn bindings(&self, id: ProcessorId) -> ParameterBindings {
        let slots = self
            .specs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.key.processor == id)
            .map(|(slot, s)| (s.key.parameter, slot))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        ParameterBindings {
            specs: Arc::clone(&self.specs),
            slots,
        }
    }
    pub(crate) fn owns(&self, bindings: &ParameterBindings) -> bool {
        Arc::ptr_eq(&self.specs, &bindings.specs)
    }
    pub(crate) fn view<'a>(&'a self, bindings: &'a ParameterBindings) -> ProcessParameters<'a> {
        ProcessParameters {
            states: &self.states,
            slots: &bindings.slots,
            specs: &self.specs,
        }
    }
    pub(crate) fn queued(&mut self) -> usize {
        self.rx.slots()
    }
    /// Bounded work, including late events. Full reply queue defers controls,
    /// NOT audio; RT never applies an event it cannot acknowledge.
    pub(crate) fn next_segment_end(&mut self, now: u64, end: u64, remaining: &mut usize) -> u64 {
        while *remaining > 0 {
            if self.replies.is_full() {
                return end;
            }
            let Ok(event) = self.rx.peek().copied() else {
                return end;
            };
            if event.at > now {
                return event.at.min(end);
            }
            let event = self.rx.pop().expect("peeked event has one consumer");
            match (&mut self.states[event.slot], event.value) {
                (ParamState::Float(ramp), ParamValue::Float(target)) => {
                    ramp.retarget(target, event.ramp_frames)
                }
                (state, value) => *state = ParamState::Discrete(value),
            }
            let reply = ControlReply {
                request_id: event.request_id,
                plan_revision: self.revision,
                timeline_epoch: self.epoch,
                code: if event.at < now {
                    ReplyCode::AppliedLate
                } else {
                    ReplyCode::Applied
                },
                effective_frame: now,
            };
            self.replies
                .push(reply)
                .expect("single producer reserved reply capacity");
            *remaining -= 1;
        }
        end
    }
    pub(crate) fn advance(&mut self, frames: usize) {
        for state in &mut self.states {
            if let ParamState::Float(ramp) = state {
                ramp.advance(frames);
            }
        }
    }
}
