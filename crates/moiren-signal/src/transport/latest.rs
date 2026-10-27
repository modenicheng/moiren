use crate::{
    DemandSnapshot, PublicationMeta, PublishError, PublishOutcome, WriteError, WriteUnavailable,
    storage::{BlockSlot, LatestStorage, Payload, ValueSlot},
    subscriber::TopicState,
};

/// Private typed writer borrow. B5 will obtain this only after scope validation.
pub(crate) struct LatestEndpoint<'a, S: Clone + Send> {
    input: &'a mut triple_buffer::Input<S>,
    sequence: &'a mut crate::sequence::Sequence,
    state: &'a TopicState,
    demand: DemandSnapshot,
}
#[cfg(test)]
impl<S: Clone + Send + 'static> LatestStorage<S> {
    // B5 validates host/binding authority before acquiring this private borrow.
    pub(crate) fn endpoint(&mut self) -> LatestEndpoint<'_, S> {
        assert_eq!(
            self.state
                .host_owner
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        let writer = self.cell.test_writer::<S>();
        writer.demand = self.state.demand.snapshot();
        LatestEndpoint::new(writer, &self.state)
    }
}
impl<'a, S: Clone + Send> LatestEndpoint<'a, S> {
    pub(crate) fn new(writer: &'a mut crate::host::slot::Writer<S>, state: &'a TopicState) -> Self {
        let crate::host::slot::Producer::Latest(input) = &mut writer.producer else {
            unreachable!("validated Latest shape")
        };
        Self {
            input,
            sequence: &mut writer.sequence,
            state,
            demand: writer.demand,
        }
    }
}
impl<S: Clone + Send> LatestEndpoint<'_, S> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn demand(&self) -> DemandSnapshot {
        self.demand
    }
}
impl<T: Copy + Send> LatestEndpoint<'_, ValueSlot<T>> {
    pub(crate) fn publish(&mut self, value: T) -> Result<PublishOutcome, PublishError> {
        self.state.check_open()?;
        if !self.demand.active {
            return Ok(PublishOutcome::NoDemand);
        }
        let sequence = self.sequence.allocate(&self.state.attempt_sequence)?;
        let meta = PublicationMeta {
            sequence,
            activation_generation: self.demand.generation,
            len: 1,
        };
        let slot = self.input.input_buffer_mut();
        slot.value = value;
        slot.meta = meta;
        self.input.publish();
        Ok(PublishOutcome::Published(meta))
    }
}

/// Exclusive prepared input borrow. Dropping it aborts without publication.
#[must_use = "a block write is only published by an explicit commit"]
pub struct BlockWriteGuard<'a, H: Copy + Send, E: Copy + Send> {
    endpoint: LatestEndpoint<'a, BlockSlot<H, E>>,
    max_len: usize,
    marker: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl<H: Copy + Send, E: Copy + Send> Drop for BlockWriteGuard<'_, H, E> {
    fn drop(&mut self) {
        // Upstream write guards publish on Drop. We borrow input_buffer_mut()
        // directly, so abort only ends this borrow and preserves the last message.
    }
}
impl<H: Copy + Send, E: Copy + Send> BlockWriteGuard<'_, H, E> {
    pub fn header_mut(&mut self) -> &mut H {
        &mut self.endpoint.input.input_buffer_mut().header
    }
    pub fn elements_mut(&mut self) -> &mut [E] {
        &mut self.endpoint.input.input_buffer_mut().elements[..self.max_len]
    }
    pub fn commit(self, len: usize) -> Result<PublishOutcome, PublishError> {
        if len > self.max_len {
            return Err(PublishError::InvalidLength);
        }
        self.endpoint.state.check_open()?;
        if !self.endpoint.demand.active {
            return Ok(PublishOutcome::NoDemand);
        }
        let sequence = self
            .endpoint
            .sequence
            .allocate(&self.endpoint.state.attempt_sequence)?;
        let meta = PublicationMeta {
            sequence,
            activation_generation: self.endpoint.demand.generation,
            len,
        };
        self.endpoint.input.input_buffer_mut().meta = meta;
        self.endpoint.input.publish();
        Ok(PublishOutcome::Published(meta))
    }
}
impl<H: Copy + Send, E: Copy + Send> LatestEndpoint<'_, BlockSlot<H, E>> {
    /// Validation shared with the production `try_write_owned` path so the
    /// test-facing borrow entry point exercises the same checks.
    fn check_write(&mut self, max_len: usize) -> Result<(), WriteError> {
        self.state.check_open()?;
        if max_len > self.input.input_buffer_mut().elements.len() {
            return Err(PublishError::InvalidLength.into());
        }
        if !self.demand.active {
            return Err(WriteError::Unavailable(WriteUnavailable::NoDemand));
        }
        Ok(())
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn try_write(
        &mut self,
        max_len: usize,
    ) -> Result<BlockWriteGuard<'_, H, E>, WriteError> {
        self.check_write(max_len)?;
        Ok(BlockWriteGuard {
            endpoint: LatestEndpoint {
                input: self.input,
                sequence: self.sequence,
                state: self.state,
                demand: self.demand,
            },
            max_len,
            marker: std::marker::PhantomData,
        })
    }
}
impl<'a, H: Copy + Send, E: Copy + Send> LatestEndpoint<'a, BlockSlot<H, E>> {
    pub(crate) fn try_write_owned(
        mut self,
        max_len: usize,
    ) -> Result<BlockWriteGuard<'a, H, E>, WriteError> {
        self.check_write(max_len)?;
        Ok(BlockWriteGuard {
            endpoint: self,
            max_len,
            marker: std::marker::PhantomData,
        })
    }
}

impl<S: Payload> LatestStorage<S> {
    /// Complete delivery probe for private transport tests only.
    #[cfg(test)]
    pub(crate) fn deliver_latest(&mut self) {
        self.output.update();
        // A late subscriber may already have updated the consumer-owned slot.
        // Its seed must not consume the pending delivery to existing receivers.
        if self.output.output_buffer().meta() == self.dispatch.meta() {
            return;
        }
        self.dispatch.copy_from(self.output.output_buffer());
        let demand = self.state.demand.snapshot();
        if !demand.active || self.dispatch.meta().activation_generation != demand.generation {
            return;
        }
        for sender in self.senders.iter_mut().flatten() {
            if sender.lease.strong_count() == 0 {
                continue;
            }
            sender.input.input_buffer_mut().copy_from(&self.dispatch);
            sender.input.publish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, BusConfig};

    use crate::test_alloc as allocator;

    #[test]
    fn copy_latest_too_small_preserves_the_pending_publication() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "copy",
                crate::BlockInit {
                    header: 7_u32,
                    element: 8_u32,
                    capacity: 3,
                },
            )
            .unwrap();
        let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let storage = bus
            .latest_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
            .unwrap();
        let mut endpoint = storage.endpoint();
        let mut write = endpoint.try_write(3).unwrap();
        *write.header_mut() = 10;
        write.elements_mut().copy_from_slice(&[11, 12, 13]);
        write.commit(3).unwrap();
        storage.deliver_latest();
        assert_eq!(
            subscriber.copy_latest(&mut [0; 2]),
            Err(crate::ReadError::DestinationTooSmall { required: 3 })
        );
        assert_eq!(
            subscriber.copy_latest(&mut []),
            Err(crate::ReadError::DestinationTooSmall { required: 3 })
        );
        let mut output = [0; 4];
        let copied = subscriber.copy_latest(&mut output).unwrap().unwrap();
        assert_eq!(
            (copied.header, copied.len, copied.meta.sequence),
            (10, 3, 1)
        );
        assert_eq!(output, [11, 12, 13, 0]);
        assert_eq!(subscriber.copy_latest(&mut output), Ok(None));
    }

    #[test]
    fn value_supersession_and_independent_receivers() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus.register_latest_value("latest", 99_u32).unwrap();
        let mut slow = bus.subscribe(&topic, Default::default()).unwrap();
        let mut fast = bus.subscribe(&topic, Default::default()).unwrap();
        assert_eq!(slow.read_latest(), None);
        let storage = bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
        {
            let mut endpoint = storage.endpoint();
            assert!(endpoint.demand().active);
            for value in [1, 2, 3] {
                endpoint.publish(value).unwrap();
            }
        }
        storage.deliver_latest();
        assert_eq!(slow.read_latest().unwrap().value, 3);
        assert_eq!(fast.read_latest().unwrap().value, 3);
        assert_eq!(slow.read_latest(), None);
    }

    #[test]
    fn aborted_and_invalid_block_commits_preserve_the_complete_publication() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "block",
                crate::BlockInit {
                    header: 99_u32,
                    element: 99_u32,
                    capacity: 4,
                },
            )
            .unwrap();
        let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let storage = bus
            .latest_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
            .unwrap();
        {
            let mut endpoint = storage.endpoint();
            let mut write = endpoint.try_write(2).unwrap();
            assert_eq!(write.elements_mut().len(), 2);
            *write.header_mut() = 7;
            write.elements_mut().copy_from_slice(&[8, 9]);
            assert!(matches!(
                write.commit(2).unwrap(),
                PublishOutcome::Published(_)
            ));
            let mut abort = endpoint.try_write(2).unwrap();
            *abort.header_mut() = 0;
            abort.elements_mut().copy_from_slice(&[1, 2]);
            drop(abort);
            let invalid = endpoint.try_write(1).unwrap();
            assert_eq!(invalid.commit(2), Err(PublishError::InvalidLength));
        }
        storage.deliver_latest();
        let read = subscriber.read_latest().unwrap();
        assert_eq!(*read.header(), 7);
        assert_eq!(read.elements(), [8, 9]);
        assert_eq!(read.meta().sequence, 1);
        assert_eq!(
            storage
                .state
                .attempt_sequence
                .load(std::sync::atomic::Ordering::Acquire),
            1
        );
    }

    #[test]
    fn late_subscription_gets_a_complete_snapshot_without_consuming_existing_delivery() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus.register_latest_value("latest", 99_u32).unwrap();
        let mut existing = bus.subscribe(&topic, Default::default()).unwrap();
        bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
            .unwrap()
            .endpoint()
            .publish(7)
            .unwrap();
        let mut late = bus.subscribe(&topic, Default::default()).unwrap();
        assert_eq!(late.read_latest().unwrap().value, 7);
        bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
            .unwrap()
            .deliver_latest();
        assert_eq!(existing.read_latest().unwrap().value, 7);
        assert_eq!(late.read_latest(), None);
    }

    #[test]
    fn reactivation_between_blocks_rejects_old_cached_publications() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus.register_latest_value("latest", 99_u32).unwrap();
        let mut first = bus.subscribe(&topic, Default::default()).unwrap();
        let storage = bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
        let initial = storage.endpoint().demand();
        storage.endpoint().publish(7).unwrap();
        storage.deliver_latest();
        assert_eq!(first.read_latest().unwrap().value, 7);
        drop(first);
        let mut next = bus.subscribe(&topic, Default::default()).unwrap();
        assert_eq!(next.read_latest(), None);
        let storage = bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
        let next_demand = storage.endpoint().demand();
        assert_eq!(next_demand.generation, initial.generation + 1);
        assert!(next_demand.active);
        storage.endpoint().publish(8).unwrap();
        storage.deliver_latest();
        assert_eq!(next.read_latest().unwrap().value, 8);
        let persistent = bus.subscribe(&topic, Default::default()).unwrap();
        drop(next);
        let replacement = bus.subscribe(&topic, Default::default()).unwrap();
        assert_eq!(
            bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
                .unwrap()
                .endpoint()
                .demand()
                .generation,
            next_demand.generation
        );
        drop((persistent, replacement));
    }

    #[test]
    fn held_block_guard_stays_coherent_while_ingress_and_another_receiver_continue() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "block",
                crate::BlockInit {
                    header: 99_u32,
                    element: 99_u32,
                    capacity: 2,
                },
            )
            .unwrap();
        let mut slow = bus.subscribe(&topic, Default::default()).unwrap();
        let mut fast = bus.subscribe(&topic, Default::default()).unwrap();
        let storage = bus
            .latest_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
            .unwrap();
        {
            let mut endpoint = storage.endpoint();
            let mut write = endpoint.try_write(2).unwrap();
            *write.header_mut() = 1;
            write.elements_mut().copy_from_slice(&[1, 2]);
            write.commit(2).unwrap();
        }
        storage.deliver_latest();
        let held = slow.read_latest().unwrap();
        for value in 3..20 {
            let mut endpoint = storage.endpoint();
            let mut write = endpoint.try_write(1).unwrap();
            *write.header_mut() = value;
            write.elements_mut()[0] = value;
            write.commit(1).unwrap();
            storage.deliver_latest();
            assert_eq!(fast.read_latest().unwrap().elements(), [value]);
            assert_eq!(held.elements(), [1, 2]);
            assert_eq!(*held.header(), 1);
        }
    }

    #[test]
    fn no_demand_and_invalid_lengths_do_not_consume_sequence() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let value = bus.register_latest_value("value", 99_u32).unwrap();
        let storage = bus.latest_storage_mut::<_, ValueSlot<u32>>(&value).unwrap();
        assert_eq!(storage.endpoint().publish(5), Ok(PublishOutcome::NoDemand));
        assert_eq!(
            storage
                .state
                .attempt_sequence
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        let block = bus
            .register_latest_block(
                "block",
                crate::BlockInit {
                    header: 99_u32,
                    element: 99_u32,
                    capacity: 2,
                },
            )
            .unwrap();
        let storage = bus
            .latest_storage_mut::<_, BlockSlot<u32, u32>>(&block)
            .unwrap();
        assert!(matches!(
            storage.endpoint().try_write(2),
            Err(WriteError::Unavailable(WriteUnavailable::NoDemand))
        ));
        assert!(matches!(
            storage.endpoint().try_write(3),
            Err(WriteError::Invalid(PublishError::InvalidLength))
        ));
        let subscriber = bus.subscribe(&block, Default::default()).unwrap();
        let storage = bus
            .latest_storage_mut::<_, BlockSlot<u32, u32>>(&block)
            .unwrap();
        assert!(matches!(
            storage.endpoint().try_write(3),
            Err(WriteError::Invalid(PublishError::InvalidLength))
        ));
        assert_eq!(
            storage
                .state
                .attempt_sequence
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        storage.state.demand.close();
        assert!(matches!(
            storage.endpoint().try_write(1),
            Err(WriteError::Invalid(PublishError::Closed))
        ));
        drop(subscriber);
        assert!(!storage.state.demand.snapshot().active);
    }

    #[test]
    fn a_cached_block_keeps_its_epoch_and_receivers_filter_stale_data() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus.register_latest_value("latest", 99_u32).unwrap();
        let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let storage = bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
        let state = storage.state.clone();
        {
            let mut endpoint = storage.endpoint();
            let cached = endpoint.demand();
            // Model the encoded zero-to-positive transition during one outer block;
            // B5 supplies independent Bus/host ownership for real concurrent edits.
            state.demand.remove();
            let current = state.demand.add(64).unwrap();
            assert_eq!(current.generation, cached.generation + 1);
            let PublishOutcome::Published(meta) = endpoint.publish(7).unwrap() else {
                panic!("active cached block")
            };
            assert_eq!(meta.activation_generation, cached.generation);
        }
        storage.deliver_latest();
        assert_eq!(subscriber.read_latest(), None);
        storage.endpoint().publish(8).unwrap();
        storage.deliver_latest();
        assert_eq!(subscriber.read_latest().unwrap().value, 8);
    }

    #[test]
    fn publication_commit_abort_and_validation_have_no_allocator_activity() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let value_topic = bus.register_latest_value("value", 99_u32).unwrap();
        let _value_subscriber = bus.subscribe(&value_topic, Default::default()).unwrap();
        let storage = bus
            .latest_storage_mut::<_, ValueSlot<u32>>(&value_topic)
            .unwrap();
        let counts = allocator::measure(|| {
            for value in 0..1000 {
                storage.endpoint().publish(value).unwrap();
            }
        });
        assert_eq!(counts, allocator::Counts::default());
        let topic = bus
            .register_latest_block(
                "block",
                crate::BlockInit {
                    header: 99_u32,
                    element: 99_u32,
                    capacity: 64,
                },
            )
            .unwrap();
        let _subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let storage = bus
            .latest_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
            .unwrap();
        let counts = allocator::measure(|| {
            for value in 0..1000 {
                let mut endpoint = storage.endpoint();
                {
                    let mut write = endpoint.try_write(64).unwrap();
                    *write.header_mut() = value;
                    write.elements_mut().fill(value);
                    write.commit(64).unwrap();
                }
                let abort = endpoint.try_write(64).unwrap();
                drop(abort);
                let invalid = endpoint.try_write(1).unwrap();
                assert_eq!(invalid.commit(2), Err(PublishError::InvalidLength));
                assert!(matches!(
                    endpoint.try_write(65),
                    Err(WriteError::Invalid(PublishError::InvalidLength))
                ));
            }
        });
        assert_eq!(counts, allocator::Counts::default());
    }

    #[test]
    fn closing_an_acquired_block_aborts_commit_without_consuming_sequence() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "block",
                crate::BlockInit {
                    header: 99_u32,
                    element: 99_u32,
                    capacity: 2,
                },
            )
            .unwrap();
        let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let storage = bus
            .latest_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
            .unwrap();
        let state = storage.state.clone();
        {
            let mut endpoint = storage.endpoint();
            let mut write = endpoint.try_write(2).unwrap();
            write.elements_mut().fill(1);
            state.demand.close();
            assert_eq!(write.commit(2), Err(PublishError::Closed));
        }
        storage.deliver_latest();
        assert_eq!(
            state
                .attempt_sequence
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        assert!(subscriber.read_latest().is_none());
        let before = bus.budget_used();
        assert!(matches!(
            bus.subscribe(&topic, Default::default()),
            Err(crate::SubscribeError::Closed)
        ));
        assert_eq!(bus.budget_used(), before);
        drop(subscriber);
    }
}
