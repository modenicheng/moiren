//! Bounded accepted ledger; only unaccepted slider intent may be replaced.
use super::{ActiveSession, ControlSummary};
use moiren_core::protocol::{
    ApplyAt, ControlReply, ParamValue, ParameterKey, ParameterRequest, ReplyCode,
};
use moiren_engine::processor::{Gain, Pan};
const LIMIT: usize = 64;
pub(super) struct Parameters {
    unsent: [Option<f64>; 2],
    ledger: Vec<ControlReply>,
    next_id: u64,
}
impl Default for Parameters {
    fn default() -> Self {
        Self {
            unsent: [None, None],
            ledger: Vec::with_capacity(LIMIT),
            next_id: 1,
        }
    }
}
impl Parameters {
    pub fn set(&mut self, summary: &mut ControlSummary, gain: f64, pan: f64) {
        self.unsent = [Some(gain), Some(pan)];
        summary.pending_gain_pan = Some((gain, pan));
    }
    pub fn clear_intent(&mut self, summary: &mut ControlSummary) {
        self.unsent = [None, None];
        summary.pending_gain_pan = None;
    }
    pub fn is_drained(&self) -> bool {
        self.ledger.is_empty()
    }
    fn terminal(&mut self, summary: &mut ControlSummary, reply: ControlReply) {
        if reply.code == ReplyCode::Accepted {
            return;
        }
        if let Some(index) = self.ledger.iter().position(|entry| {
            entry.request_id == reply.request_id
                && entry.plan_revision == reply.plan_revision
                && entry.timeline_epoch == reply.timeline_epoch
        }) {
            self.ledger.swap_remove(index);
            summary.last_result = Some(reply);
        }
        summary.accepted_pending = self.ledger.len();
    }
    pub fn poll(
        &mut self,
        session: &mut dyn ActiveSession,
        summary: &mut ControlSummary,
        budget: usize,
    ) {
        let timeline = session.timeline();
        let nodes = session.gain_pan_nodes();
        let Some((control, bindings)) = session.control() else {
            return;
        };
        for _ in 0..budget.min(32) {
            let Some(reply) = control.poll_applied() else {
                break;
            };
            self.terminal(summary, reply);
        }
        let (Some(timeline), Some((gain, pan))) = (timeline, nodes) else {
            return;
        };
        let (revision, epoch) = control.identity();
        if timeline.revision != revision || timeline.epoch != epoch {
            return;
        }
        for (slot, node, parameter) in [(0, gain, Gain::LEVEL), (1, pan, Pan::POSITION)] {
            if self.ledger.len() == LIMIT {
                break;
            }
            let Some(value) = self.unsent[slot] else {
                continue;
            };
            let Some(processor) = bindings.node(node) else {
                continue;
            };
            let Some(next) = self.next_id.checked_add(1) else {
                return;
            };
            let reply = control.submit(
                ParameterRequest {
                    request_id: self.next_id,
                    plan_revision: revision,
                    timeline_epoch: epoch,
                    target: ParameterKey {
                        processor,
                        parameter,
                    },
                    at: ApplyAt::NextBlock,
                    value: ParamValue::Float(value),
                    ramp_frames: 0,
                },
                timeline.frame,
            );
            if reply.code == ReplyCode::QueueFull {
                break;
            }
            self.next_id = next;
            self.unsent[slot] = None;
            if reply.code == ReplyCode::Accepted {
                self.ledger.push(reply);
            }
            summary.last_result = Some(reply);
        }
        summary.accepted_pending = self.ledger.len();
        if self.unsent == [None, None] {
            summary.pending_gain_pan = None;
        }
    }
    /// The owner has conclusively joined before this is called. Normally every
    /// entry is covered by retired RT replies; a panicked output owner destroys
    /// its engine during unwind and cannot supply frame/apply evidence. Those
    /// remaining accepted requests end as StaleRevision, never invented Applied.
    pub fn reaped(
        &mut self,
        summary: &mut ControlSummary,
        replies: &mut std::vec::IntoIter<ControlReply>,
        budget: &mut usize,
    ) -> bool {
        while *budget > 0 {
            if let Some(reply) = replies.next() {
                self.terminal(summary, reply);
            } else if let Some(accepted) = self.ledger.pop() {
                summary.last_result = Some(ControlReply {
                    code: ReplyCode::StaleRevision,
                    ..accepted
                });
            } else {
                break;
            }
            *budget -= 1;
        }
        summary.accepted_pending = self.ledger.len();
        replies.len() == 0 && self.ledger.is_empty()
    }
}
