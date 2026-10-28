//! Ownership-moving fixed pools. Publication never lends ingress to receivers.

use crate::{
    DemandSnapshot, PublicationMeta, PublishError, PublishOutcome, WriteError, WriteUnavailable,
    error::flatten_write_error,
    sequence::Sequence,
    storage::Payload,
    storage::{BlockSlot, PoolWriter, StreamStorage, ValueSlot},
    subscriber::TopicState,
};

/// B5 supplies this typed borrow only after host/scope authority validation.
pub(crate) struct StreamEndpoint<'a, S> {
    writer: &'a mut PoolWriter<S>,
    sequence: &'a mut Sequence,
    state: &'a TopicState,
    demand: DemandSnapshot,
    capacity: usize,
}

#[cfg(test)]
impl<S: Payload> StreamStorage<S> {
    pub(crate) fn endpoint(&mut self) -> StreamEndpoint<'_, S> {
        assert_eq!(
            self.state
                .host_owner
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        let writer = self.cell.test_writer::<S>();
        writer.demand = self.state.demand.snapshot();
        StreamEndpoint::new(writer, &self.state)
    }
}
impl<'a, S: Clone + Send> StreamEndpoint<'a, S> {
    pub(crate) fn new(writer: &'a mut crate::host::slot::Writer<S>, state: &'a TopicState) -> Self {
        let crate::host::slot::Producer::Stream(producer) = &mut writer.producer else {
            unreachable!("validated Stream shape")
        };
        Self {
            writer: producer,
            sequence: &mut writer.sequence,
            state,
            demand: writer.demand,
            capacity: writer.capacity,
        }
    }
}

impl<S> StreamEndpoint<'_, S> {
    fn dropped(&mut self) -> Result<u64, PublishError> {
        let sequence = self.sequence.allocate(&self.state.attempt_sequence)?;
        crate::lifecycle::increment(&self.state.ingress_drops);
        Ok(sequence)
    }
    /// Demand/pool acquisition shared by every stream write entry point.
    fn acquire_slot(&mut self) -> Result<S, WriteError> {
        if !self.demand.active {
            return Err(WriteError::Unavailable(WriteUnavailable::NoDemand));
        }
        let Some(slot) = self.writer.acquire() else {
            let sequence = self.dropped()?;
            return Err(WriteError::Unavailable(WriteUnavailable::Full { sequence }));
        };
        Ok(slot)
    }
    fn acquire(&mut self) -> Result<StreamWrite<'_, S>, WriteError> {
        self.state.check_open()?;
        let slot = self.acquire_slot()?;
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
        self.endpoint.state.check_open()?;
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
            crate::lifecycle::increment(&self.endpoint.state.ingress_drops);
            Ok(PublishOutcome::DroppedFull { sequence })
        }
    }
}

impl<T: Copy + Send + 'static> StreamEndpoint<'_, ValueSlot<T>> {
    pub(crate) fn publish(&mut self, value: T) -> Result<PublishOutcome, PublishError> {
        let mut write = match self.acquire() {
            Ok(write) => write,
            Err(error) => return flatten_write_error(error),
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
    marker: std::marker::PhantomData<std::rc::Rc<()>>,
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
    /// Validation shared with the production `try_write_owned` path so the
    /// test-facing borrow entry point exercises the same checks.
    fn check_write(&self, max_len: usize) -> Result<(), WriteError> {
        self.state.check_open()?;
        if max_len > self.capacity {
            return Err(PublishError::InvalidLength.into());
        }
        Ok(())
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn try_write(
        &mut self,
        max_len: usize,
    ) -> Result<StreamBlockWriteGuard<'_, H, E>, WriteError> {
        self.check_write(max_len)?;
        let write = self.acquire()?;
        Ok(StreamBlockWriteGuard {
            write,
            max_len,
            marker: std::marker::PhantomData,
        })
    }
    /// Test seam: one-call slice publication for private transport tests.
    /// Production writes go through `try_write_owned` (scoped guards) or the
    /// value `publish`; the scope-level `SignalScope::publish_slice` is the
    /// production slice entry point.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn publish_slice(
        &mut self,
        header: H,
        elements: &[E],
    ) -> Result<PublishOutcome, PublishError> {
        let mut write = match self.try_write(elements.len()) {
            Ok(write) => write,
            Err(error) => return flatten_write_error(error),
        };
        *write.header_mut() = header;
        write.elements_mut().copy_from_slice(elements);
        write.commit(elements.len())
    }
}
impl<'a, H: Copy + Send + 'static, E: Copy + Send + 'static> StreamEndpoint<'a, BlockSlot<H, E>> {
    pub(crate) fn try_write_owned(
        mut self,
        max_len: usize,
    ) -> Result<StreamBlockWriteGuard<'a, H, E>, WriteError> {
        self.check_write(max_len)?;
        let slot = self.acquire_slot()?;
        Ok(StreamBlockWriteGuard {
            write: StreamWrite {
                endpoint: self,
                slot: Some(slot),
            },
            max_len,
            marker: std::marker::PhantomData,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchStep {
    Empty,
    Discarded,
    Ready,
}

// Dispatcher charges dequeue/cleanup and each independent receiver copy.
// No operation scans a configured Stream depth, including a depth of ZST slots.
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
        let mut staging = self.staging.take().expect("prepared dispatch staging");
        staging.copy_from(&slot);
        self.reader.recycle(slot);
        self.dispatch = Some(staging);
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
        let Some(lease) = sender.lease.upgrade() else {
            return;
        };
        if !lease.counted || !sender.eligible(meta, &demand) {
            return;
        }
        let Some(mut destination) = sender.writer.acquire() else {
            sender.record_drop(&lease);
            return;
        };
        destination.copy_from(source);
        if !sender.writer.enqueue(destination) {
            sender.record_drop(&lease);
        } else {
            crate::lifecycle::increment(&lease.delivered);
        }
    }
    pub(crate) fn finish_dispatch(&mut self) {
        if let Some(slot) = self.dispatch.take() {
            self.staging = Some(slot);
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
