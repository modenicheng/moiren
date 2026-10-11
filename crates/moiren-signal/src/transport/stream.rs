//! Ownership-moving fixed pools. Publication never lends ingress to receivers.

use crate::{
    DemandSnapshot, PublicationMeta, PublishError, PublishOutcome, WriteError, WriteUnavailable,
    sequence::Sequence,
    storage::Payload,
    storage::{BlockSlot, PoolWriter, StreamStorage, ValueSlot},
    subscriber::TopicState,
};
use std::sync::atomic::Ordering;

/// B5 supplies this typed borrow only after host/scope authority validation.
pub(crate) struct StreamEndpoint<'a, S> {
    writer: &'a mut PoolWriter<S>,
    sequence: &'a mut Sequence,
    state: &'a TopicState,
    demand: DemandSnapshot,
    capacity: usize,
}

impl<S: Payload> StreamStorage<S> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn endpoint(&mut self) -> StreamEndpoint<'_, S> {
        StreamEndpoint {
            writer: &mut self.writer,
            sequence: &mut self.sequence,
            state: &self.state,
            demand: self.state.demand.snapshot(),
            capacity: self.template.element_count(),
        }
    }
}

impl<S> StreamEndpoint<'_, S> {
    fn check_open(&self) -> Result<(), PublishError> {
        if self.state.demand.is_closed() {
            Err(PublishError::Closed)
        } else {
            Ok(())
        }
    }
    fn dropped(&mut self) -> Result<u64, PublishError> {
        let sequence = self.sequence.allocate(&self.state.attempt_sequence)?;
        self.state.ingress_drops.fetch_add(1, Ordering::Relaxed);
        Ok(sequence)
    }
    fn acquire(&mut self) -> Result<StreamWrite<'_, S>, WriteError> {
        self.check_open()?;
        if !self.demand.active {
            return Err(WriteError::Unavailable(WriteUnavailable::NoDemand));
        }
        let Some(slot) = self.writer.acquire() else {
            let sequence = self.dropped()?;
            return Err(WriteError::Unavailable(WriteUnavailable::Full { sequence }));
        };
        Ok(StreamWrite {
            endpoint: StreamEndpoint {
                writer: self.writer,
                sequence: self.sequence,
                state: self.state,
                demand: self.demand,
                capacity: self.capacity,
            },
            slot: Some(slot),
        })
    }
}

struct StreamWrite<'a, S> {
    endpoint: StreamEndpoint<'a, S>,
    slot: Option<S>,
}
impl<S> Drop for StreamWrite<'_, S> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            self.endpoint.writer.restore(slot);
        }
    }
}
impl<S: Payload> StreamWrite<'_, S> {
    fn slot_mut(&mut self) -> &mut S {
        self.slot.as_mut().expect("acquired owner")
    }
    fn commit(mut self, len: usize) -> Result<PublishOutcome, PublishError> {
        self.endpoint.check_open()?;
        let sequence = self
            .endpoint
            .sequence
            .allocate(&self.endpoint.state.attempt_sequence)?;
        let meta = PublicationMeta {
            sequence,
            activation_generation: self.endpoint.demand.generation,
            len,
        };
        self.slot_mut().set_meta(meta);
        // Commit's Release sequence allocation occurs before ready enqueue.
        // An abort or any early error keeps the slot in the local RT spare.
        let slot = self.slot.take().expect("acquired owner");
        if self.endpoint.writer.enqueue(slot) {
            Ok(PublishOutcome::Published(meta))
        } else {
            // Single producer means capacity cannot shrink after acquire. Still
            // retain the owner if enqueue fails, never destructing it on RT.
            self.endpoint
                .state
                .ingress_drops
                .fetch_add(1, Ordering::Relaxed);
            Ok(PublishOutcome::DroppedFull { sequence })
        }
    }
}

impl<T: Copy + Send + 'static> StreamEndpoint<'_, ValueSlot<T>> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn publish(&mut self, value: T) -> Result<PublishOutcome, PublishError> {
        let mut write = match self.acquire() {
            Ok(write) => write,
            Err(WriteError::Unavailable(WriteUnavailable::NoDemand)) => {
                return Ok(PublishOutcome::NoDemand);
            }
            Err(WriteError::Unavailable(WriteUnavailable::Full { sequence })) => {
                return Ok(PublishOutcome::DroppedFull { sequence });
            }
            Err(WriteError::Invalid(error)) => return Err(error),
        };
        write.slot_mut().value = value;
        write.commit(1)
    }
}

/// Exclusive owner of a prepared ingress slot; Drop aborts without an attempt.
#[must_use = "a block write is only published by an explicit commit"]
pub struct StreamBlockWriteGuard<'a, H: Copy + Send + 'static, E: Copy + Send + 'static> {
    write: StreamWrite<'a, BlockSlot<H, E>>,
    max_len: usize,
}
impl<H: Copy + Send + 'static, E: Copy + Send + 'static> StreamBlockWriteGuard<'_, H, E> {
    pub fn header_mut(&mut self) -> &mut H {
        &mut self.write.slot_mut().header
    }
    pub fn elements_mut(&mut self) -> &mut [E] {
        &mut self.write.slot_mut().elements[..self.max_len]
    }
    pub fn commit(self, len: usize) -> Result<PublishOutcome, PublishError> {
        if len > self.max_len {
            return Err(PublishError::InvalidLength);
        }
        self.write.commit(len)
    }
}
impl<H: Copy + Send + 'static, E: Copy + Send + 'static> StreamEndpoint<'_, BlockSlot<H, E>> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn try_write(
        &mut self,
        max_len: usize,
    ) -> Result<StreamBlockWriteGuard<'_, H, E>, WriteError> {
        self.check_open()?;
        if max_len > self.capacity {
            return Err(PublishError::InvalidLength.into());
        }
        let write = self.acquire()?;
        Ok(StreamBlockWriteGuard { write, max_len })
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn publish_slice(
        &mut self,
        header: H,
        elements: &[E],
    ) -> Result<PublishOutcome, PublishError> {
        let mut write = match self.try_write(elements.len()) {
            Ok(write) => write,
            Err(WriteError::Unavailable(WriteUnavailable::NoDemand)) => {
                return Ok(PublishOutcome::NoDemand);
            }
            Err(WriteError::Unavailable(WriteUnavailable::Full { sequence })) => {
                return Ok(PublishOutcome::DroppedFull { sequence });
            }
            Err(WriteError::Invalid(error)) => return Err(error),
        };
        *write.header_mut() = header;
        write.elements_mut().copy_from_slice(elements);
        write.commit(elements.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchStep {
    Empty,
    Discarded,
    Ready,
}

// B4 can charge dequeue/cleanup, each receiver copy, and owner return separately.
// No operation scans a configured Stream depth, including a depth of ZST slots.
#[cfg_attr(not(test), allow(dead_code))]
impl<S: Payload> StreamStorage<S> {
    pub(crate) fn begin_dispatch(&mut self) -> DispatchStep {
        assert!(self.dispatch.is_none(), "finish the held dispatch first");
        let Ok(slot) = self.reader.ready.pop() else {
            return DispatchStep::Empty;
        };
        let demand = self.state.demand.snapshot();
        if !demand.active || slot.meta().activation_generation != demand.generation {
            self.reader.recycle(slot);
            return DispatchStep::Discarded;
        }
        self.dispatch = Some(slot);
        DispatchStep::Ready
    }
    pub(crate) fn deliver_to(&mut self, index: usize) {
        let Some(source) = self.dispatch.as_ref() else {
            return;
        };
        let Some(sender) = self.senders.get_mut(index).and_then(Option::as_mut) else {
            return;
        };
        let demand = self.state.demand.snapshot();
        let meta = source.meta();
        if sender.lease.strong_count() == 0
            || !demand.active
            || meta.activation_generation != demand.generation
            || meta.activation_generation != sender.generation
            || meta.sequence <= sender.fence
        {
            return;
        }
        let Some(mut destination) = sender.writer.acquire() else {
            sender.drops = sender.drops.saturating_add(1);
            return;
        };
        destination.copy_from(source);
        if !sender.writer.enqueue(destination) {
            sender.drops = sender.drops.saturating_add(1);
        }
    }
    pub(crate) fn finish_dispatch(&mut self) {
        if let Some(slot) = self.dispatch.take() {
            self.reader.recycle(slot);
        }
    }
    #[cfg(test)]
    fn deliver_next(&mut self) -> bool {
        match self.begin_dispatch() {
            DispatchStep::Empty => false,
            DispatchStep::Discarded => true,
            DispatchStep::Ready => {
                for index in 0..64 {
                    self.deliver_to(index);
                }
                self.finish_dispatch();
                true
            }
        }
    }
}

#[cfg(test)]
mod tests;
