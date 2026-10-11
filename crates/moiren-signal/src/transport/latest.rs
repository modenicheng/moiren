use crate::{
    DemandSnapshot, PublicationMeta, PublishError, PublishOutcome, WriteError, WriteUnavailable,
    storage::{BlockSlot, LatestStorage, ValueSlot},
    subscriber::TopicState,
};
use std::sync::Arc;

pub(crate) trait LatestPayload: Clone + Send + 'static {
    fn meta(&self) -> PublicationMeta;
    fn copy_from(&mut self, source: &Self);
    fn element_bytes(&self) -> usize;
}
impl<T: Copy + Send + 'static> LatestPayload for ValueSlot<T> {
    fn meta(&self) -> PublicationMeta {
        self.meta
    }
    fn copy_from(&mut self, source: &Self) {
        *self = *source;
    }
    fn element_bytes(&self) -> usize {
        0
    }
}
impl<H: Copy + Send + 'static, E: Copy + Send + 'static> LatestPayload for BlockSlot<H, E> {
    fn meta(&self) -> PublicationMeta {
        self.meta
    }
    fn copy_from(&mut self, source: &Self) {
        self.meta = source.meta;
        self.header = source.header;
        // Box ownership never changes: fan-out copies into prepared regions.
        self.elements.copy_from_slice(&source.elements);
    }
    fn element_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.elements)
    }
}

/// Private typed writer borrow. B5 will obtain this only after scope validation.
pub(crate) struct LatestEndpoint<'a, S: Clone + Send> {
    input: &'a mut triple_buffer::Input<S>,
    sequence: &'a mut crate::sequence::Sequence,
    state: &'a Arc<TopicState>,
    demand: DemandSnapshot,
}
impl<S: Clone + Send> LatestStorage<S> {
    // B5 validates host/binding authority before acquiring this private borrow.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn endpoint(&mut self) -> LatestEndpoint<'_, S> {
        LatestEndpoint {
            input: &mut self.input,
            sequence: &mut self.sequence,
            state: &self.state,
            demand: self.state.demand.snapshot(),
        }
    }
}
impl<S: Clone + Send> LatestEndpoint<'_, S> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn demand(&self) -> DemandSnapshot {
        self.demand
    }
    fn check_open(&self) -> Result<(), PublishError> {
        if self.state.demand.is_closed() {
            Err(PublishError::Closed)
        } else {
            Ok(())
        }
    }
}
impl<T: Copy + Send> LatestEndpoint<'_, ValueSlot<T>> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn publish(&mut self, value: T) -> Result<PublishOutcome, PublishError> {
        self.check_open()?;
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
        self.endpoint.check_open()?;
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
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn try_write(
        &mut self,
        max_len: usize,
    ) -> Result<BlockWriteGuard<'_, H, E>, WriteError> {
        self.check_open()?;
        if max_len > self.input.input_buffer_mut().elements.len() {
            return Err(PublishError::InvalidLength.into());
        }
        if !self.demand.active {
            return Err(WriteError::Unavailable(WriteUnavailable::NoDemand));
        }
        Ok(BlockWriteGuard {
            endpoint: LatestEndpoint {
                input: self.input,
                sequence: self.sequence,
                state: self.state,
                demand: self.demand,
            },
            max_len,
        })
    }
}

impl<S: LatestPayload> LatestStorage<S> {
    /// One bounded transport delivery step; the public budgeted pump is B4.
    #[cfg_attr(not(test), allow(dead_code))]
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

    mod allocator {
        use std::{
            alloc::{GlobalAlloc, Layout, System},
            cell::Cell,
        };
        #[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
        pub(super) struct Counts {
            alloc: usize,
            realloc: usize,
            free: usize,
        }
        thread_local! { static PROBE: Cell<Option<Counts>> = const { Cell::new(None) }; }
        pub(super) struct Allocator;
        fn record(kind: usize) {
            let _ = PROBE.try_with(|state| {
                if let Some(mut counts) = state.get() {
                    match kind {
                        0 => counts.alloc += 1,
                        1 => counts.realloc += 1,
                        _ => counts.free += 1,
                    }
                    state.set(Some(counts));
                }
            });
        }
        // SAFETY: All operations delegate the original pointer/layout unchanged
        // to System. Preinitialized TLS counters do not allocate recursively.
        unsafe impl GlobalAlloc for Allocator {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                record(0);
                // SAFETY: Forward the caller's valid allocation layout.
                unsafe { System.alloc(layout) }
            }
            unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
                record(1);
                // SAFETY: Forward the original allocation and requested size.
                unsafe { System.realloc(pointer, layout, size) }
            }
            unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
                record(2);
                // SAFETY: Forward the original allocation and layout.
                unsafe { System.dealloc(pointer, layout) };
            }
        }
        pub(super) fn measure(operation: impl FnOnce()) -> Counts {
            struct Stop;
            impl Drop for Stop {
                fn drop(&mut self) {
                    PROBE.with(|state| state.set(None));
                }
            }
            PROBE.with(|state| state.set(Some(Counts::default())));
            let stop = Stop;
            operation();
            let counts = PROBE.with(|state| state.get().unwrap());
            drop(stop);
            counts
        }
    }

    #[global_allocator]
    static ALLOCATOR: allocator::Allocator = allocator::Allocator;

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
        let state = Arc::clone(&storage.state);
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
        let state = Arc::clone(&storage.state);
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
