use std::any::Any;
use std::sync::atomic::Ordering;

use crate::{
    PumpBudget, PumpReport,
    lifecycle::{StateHandle, increment},
    storage::{LatestStorage, Payload, StreamStorage},
};

#[derive(Default)]
pub(crate) struct Fanout {
    pub(crate) active: bool,
    pub(crate) recipients: u64,
    retired_last: bool,
    retire_cursor: u32,
    #[cfg(test)]
    after_full_check: Option<std::sync::Arc<DeliveryGate>>,
}

#[cfg(test)]
struct DeliveryGate {
    checked: std::sync::Barrier,
    resume: std::sync::Barrier,
}

impl Fanout {
    fn retired_slot(&mut self, retired: u64) -> usize {
        let index =
            (retired.rotate_right(self.retire_cursor).trailing_zeros() + self.retire_cursor) % 64;
        self.retire_cursor = (index + 1) % 64;
        index as usize
    }
}

pub(crate) trait Dispatch: Any + Send {
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn state(&self) -> &StateHandle;
    fn state_mut(&mut self) -> &mut StateHandle;
    fn pending(&self) -> bool;
    fn step(&mut self, report: &mut PumpReport, budget: PumpBudget) -> bool;
}

fn charge(report: &mut PumpReport, budget: PumpBudget, bytes: usize) -> bool {
    let total = report.copy_bytes.checked_add(bytes);
    if report.deliveries >= budget.deliveries || total.is_none_or(|total| total > budget.copy_bytes)
    {
        report.has_pending = true;
        report.required_copy_bytes = report.required_copy_bytes.max(bytes);
        return false;
    }
    report.deliveries += 1;
    report.copy_bytes = total.expect("checked total");
    true
}

impl<S: Payload> Dispatch for LatestStorage<S> {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn state(&self) -> &StateHandle {
        &self.state
    }
    fn state_mut(&mut self) -> &mut StateHandle {
        &mut self.state
    }
    fn pending(&self) -> bool {
        self.fanout.active
            || self.output.updated()
            || self.output.output_buffer().meta() != self.dispatch.meta()
            || self.state.retired_receivers.load(Ordering::Acquire) & self.occupied != 0
    }
    fn step(&mut self, report: &mut PumpReport, budget: PumpBudget) -> bool {
        if self.fanout.active && self.fanout.recipients == 0 {
            self.fanout.active = false;
        }
        let retired = self.state.retired_receivers.load(Ordering::Acquire) & self.occupied;
        if retired != 0 {
            if !charge(report, budget, 0) {
                return false;
            }
            let index = self.fanout.retired_slot(retired);
            self.senders[index] = None;
            self.occupied &= !(1 << index);
            self.fanout.recipients &= !(1 << index);
            if self.fanout.recipients == 0 {
                self.fanout.active = false;
            }
            self.state
                .retired_receivers
                .fetch_and(!(1 << index), Ordering::Relaxed);
            return true;
        }
        if !self.fanout.active {
            self.output.update();
            let source = self.output.output_buffer();
            if source.meta() == self.dispatch.meta() {
                return false;
            }
            let demand = self.state.demand.snapshot();
            if !demand.active || source.meta().activation_generation != demand.generation {
                if !charge(report, budget, 0) {
                    return false;
                }
                self.dispatch.set_meta(source.meta());
                return true;
            }
            if !charge(report, budget, source.copy_bytes()) {
                return false;
            }
            self.dispatch.copy_from(source);
            self.fanout.recipients = self.occupied;
            self.fanout.active = self.occupied != 0;
            return true;
        }
        let index = self.fanout.recipients.trailing_zeros() as usize;
        let source = &self.dispatch;
        let meta = source.meta();
        let demand = self.state.demand.snapshot();
        let sender = self.senders[index].as_mut().expect("snapshotted sender");
        let lease = sender.lease.upgrade();
        let eligible = lease.is_some()
            && demand.active
            && meta.activation_generation == demand.generation
            && meta.sequence > sender.last_sequence;
        if !charge(
            report,
            budget,
            if eligible { source.copy_bytes() } else { 0 },
        ) {
            return false;
        }
        self.fanout.recipients &= !(1 << index);
        if eligible {
            sender.input.input_buffer_mut().copy_from(source);
            sender.input.publish();
            sender.last_sequence = meta.sequence;
            increment(&lease.expect("eligible lease").delivered);
        }
        if self.fanout.recipients == 0 {
            self.fanout.active = false;
        }
        true
    }
}

impl<S: Payload> Dispatch for StreamStorage<S> {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn state(&self) -> &StateHandle {
        &self.state
    }
    fn state_mut(&mut self) -> &mut StateHandle {
        &mut self.state
    }
    fn pending(&self) -> bool {
        self.dispatch.is_some()
            || !self.reader.ready.is_empty()
            || self.state.retired_receivers.load(Ordering::Acquire) & self.occupied != 0
    }
    fn step(&mut self, report: &mut PumpReport, budget: PumpBudget) -> bool {
        if self.dispatch.is_some() && self.fanout.recipients == 0 {
            self.finish_dispatch();
        }
        let retired = self.state.retired_receivers.load(Ordering::Acquire) & self.occupied;
        let business_pending = self.dispatch.is_some() || !self.reader.ready.is_empty();
        if retired != 0 && (!self.fanout.retired_last || !business_pending) {
            if !charge(report, budget, 0) {
                return false;
            }
            self.fanout.retired_last = true;
            let index = self.fanout.retired_slot(retired);
            if !self.senders[index]
                .as_mut()
                .expect("retired sender")
                .retire_one()
            {
                return true;
            }
            self.senders[index] = None;
            self.occupied &= !(1 << index);
            self.fanout.recipients &= !(1 << index);
            self.state
                .retired_receivers
                .fetch_and(!(1 << index), Ordering::Relaxed);
            if self.fanout.recipients == 0 {
                self.finish_dispatch();
            }
            return true;
        }
        self.fanout.retired_last = false;
        if self.dispatch.is_none() {
            let Ok(source) = self.reader.ready.peek() else {
                return false;
            };
            let demand = self.state.demand.snapshot();
            let valid = demand.active && source.meta().activation_generation == demand.generation;
            if !charge(report, budget, if valid { source.copy_bytes() } else { 0 }) {
                return false;
            }
            self.begin_dispatch();
            if self.dispatch.is_some() {
                self.fanout.recipients = self.occupied;
                if self.occupied == 0 {
                    self.finish_dispatch();
                }
            }
            return true;
        }
        let index = self.fanout.recipients.trailing_zeros() as usize;
        let source = self.dispatch.as_ref().expect("pending dispatch");
        let meta = source.meta();
        let demand = self.state.demand.snapshot();
        let sender = self.senders[index].as_mut().expect("snapshotted sender");
        let lease = sender.lease.upgrade();
        let eligible = lease.is_some()
            && demand.active
            && meta.activation_generation == demand.generation
            && meta.activation_generation == sender.generation
            && meta.sequence > sender.fence;
        let full = eligible && sender.writer.ready.is_full();
        let bytes = if eligible && !full {
            source.copy_bytes()
        } else {
            0
        };
        #[cfg(test)]
        if let Some(gate) = &self.fanout.after_full_check {
            gate.checked.wait();
            gate.resume.wait();
        }
        if !charge(report, budget, bytes) {
            return false;
        }
        self.fanout.recipients &= !(1 << index);
        if full {
            sender.drops = sender.drops.saturating_add(1);
            increment(&lease.expect("eligible full receiver").overflow_drops);
        } else if eligible {
            self.deliver_to(index);
        }
        if self.fanout.recipients == 0 {
            self.finish_dispatch();
        }
        true
    }
}

#[cfg(test)]
mod tests;
