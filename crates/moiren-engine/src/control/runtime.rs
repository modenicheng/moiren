use super::{FloatRamp, ParamSpec};
use moiren_core::protocol::{ControlReply, ParamValue, ReplyCode};
use rtrb::{Consumer, Producer};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy)]
pub(super) enum ParamState {
    Float(FloatRamp),
    Discrete(ParamValue),
}
#[derive(Debug, Clone, Copy)]
pub(super) struct ScheduledEvent {
    pub(super) slot: usize,
    pub(super) request_id: u64,
    pub(super) at: u64,
    pub(super) value: ParamValue,
    pub(super) ramp_frames: u32,
}

pub struct ParameterRuntime {
    pub(super) specs: Arc<[ParamSpec]>,
    pub(super) states: Box<[ParamState]>,
    pub(super) rx: Consumer<ScheduledEvent>,
    pub(super) replies: Producer<ControlReply>,
    pub(super) revision: u64,
    pub(super) epoch: u64,
    pub(super) retired: Arc<AtomicBool>,
}

impl ParameterRuntime {
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
