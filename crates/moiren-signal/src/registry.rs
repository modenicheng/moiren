use std::{
    alloc::Layout,
    marker::PhantomData,
    sync::{Arc, atomic::Ordering},
};

use crate::{
    BlockInit, BusConfig, LatestBlock, LatestValue, LookupError, PumpBudget, PumpReport,
    RegisterError, Shape, StreamBlock, StreamValue, SubscribeError, SubscriptionOptions, Topic,
    TopicId, UnregisterError,
    budget::Charge,
    budget::{self, Budget},
    dispatcher::Dispatch,
    lifecycle::DirectoryId,
    storage::Payload,
    storage::{
        LatestSender, LatestStorage, StreamSender, StreamStorage, ValueSlot, block_template, pool,
    },
    subscriber::{LatestReceiver, StreamReceiver, Subscriber, SubscriptionLease},
    types::{Mode, PayloadType},
};

struct Entry {
    id: TopicId,
    name: Box<str>,
    payload_type: PayloadType,
    mode: Mode,
    // Erasure and name lookup occur only on the non-RT preparation side.
    // No publisher/host bypass is exposed by this directory.
    storage: Box<dyn Dispatch>,
    _charge: Charge,
}

/// Non-real-time directory and owner of prepared topic resources.
pub struct Bus {
    config: BusConfig,
    directory: DirectoryId,
    entries: Box<[Option<Entry>]>,
    count: usize,
    next_generation: u64,
    budget: Budget,
    pump_cursor: usize,
}

impl Bus {
    pub fn new(config: BusConfig) -> Result<Self, RegisterError> {
        if config.max_topics == 0
            || !(1..=64).contains(&config.max_subscribers_per_topic)
            || config.default_stream_depth == 0
        {
            return Err(RegisterError::InvalidCapacity);
        }
        config
            .default_stream_depth
            .checked_add(1)
            .and_then(|free_depth| free_depth.checked_mul(2))
            .ok_or(RegisterError::SizeOverflow)?;
        let directory_layout = budget::array::<Option<Entry>>(config.max_topics)?;
        // Count the bus's own inline metadata and every fixed directory slot,
        // including vacant entries. No directory growth is possible later.
        let baseline = budget::add(
            budget::add(Layout::new::<Self>().size(), directory_layout.size())?,
            budget::arc_layout::<std::sync::atomic::AtomicUsize>()?.size(),
        )?;
        let budget = Budget::new(config.byte_budget, baseline)?;
        let mut entries = Vec::with_capacity(config.max_topics);
        entries.resize_with(config.max_topics, || None);
        let entries = entries.into_boxed_slice();
        // All checks/allocations precede the globally unique identity commit.
        let directory = DirectoryId::allocate()?;
        Ok(Self {
            config,
            directory,
            entries,
            count: 0,
            next_generation: 1,
            budget,
            pump_cursor: 0,
        })
    }

    /// Bytes owned by the bus: inline metadata, reserved directory slots and
    /// every registered name, transport, template, queue, dispatch and receiver.
    /// Sizes describe allocation layouts, excluding allocator bookkeeping.
    /// Closed topics, retired pools and loans stay charged through their actual
    /// final release; failed preparation never changes the budget.
    pub fn budget_used(&self) -> usize {
        self.budget.used()
    }

    /// Prepares independent receiver storage before committing business demand.
    /// Latest rejects `stream_depth`; Stream depth counts queued messages and
    /// reserves one additional initialized receiver loan.
    pub fn subscribe<P: Shape>(
        &mut self,
        topic: &Topic<P>,
        options: SubscriptionOptions,
    ) -> Result<Subscriber<P>, SubscribeError> {
        if topic.id.directory != self.directory {
            return Err(SubscribeError::ForeignTopic);
        }
        let state = topic.state.clone();
        let _gate = state.gate.lock().expect("lifecycle gate poisoned");
        if state.demand.is_closed() {
            return Err(SubscribeError::Closed);
        }
        P::subscribe(self, topic, options)
    }

    pub fn has_business_demand(&self) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.storage.state().demand.snapshot().active)
    }

    /// Closes under the preparation/lease gate. Busy changes nothing; successful
    /// closure frees the directory slot while old subscribers keep their loans.
    pub fn unregister<P: Shape>(&mut self, topic: &Topic<P>) -> Result<(), UnregisterError> {
        if topic.id.directory != self.directory {
            return Err(UnregisterError::ForeignTopic);
        }
        let entry = self
            .entries
            .get(topic.id.slot)
            .and_then(Option::as_ref)
            .filter(|entry| entry.id == topic.id)
            .ok_or(UnregisterError::Closed)?;
        entry.storage.state().close()?;
        let removed = self.entries[topic.id.slot].take();
        self.count -= 1;
        drop(removed);
        Ok(())
    }

    /// Checks one directory slot and performs at most one charged operation
    /// per round-robin visit. This method and reclamation are non-RT.
    pub fn pump(&mut self, budget: PumpBudget) -> PumpReport {
        let mut report = PumpReport::default();
        let mut idle = 0;
        let mut quiet_checks = 0;
        while report.topic_checks < budget.topic_checks && idle < self.entries.len() {
            let slot = self.pump_cursor;
            self.pump_cursor = (slot + 1) % self.entries.len();
            report.topic_checks += 1;
            let progressed = if let Some(entry) = &mut self.entries[slot] {
                let progressed = entry.storage.step(&mut report, budget);
                quiet_checks = if entry.storage.pending() {
                    0
                } else {
                    quiet_checks + 1
                };
                progressed
            } else {
                quiet_checks += 1;
                false
            };
            idle = if progressed { 0 } else { idle + 1 };
        }
        report.has_pending |= quiet_checks < self.entries.len() && self.count != 0;
        report
    }

    pub(crate) fn latest_storage_mut<P: Shape, S: Payload>(
        &mut self,
        topic: &Topic<P>,
    ) -> Result<&mut LatestStorage<S>, SubscribeError> {
        if topic.id.directory != self.directory {
            return Err(SubscribeError::ForeignTopic);
        }
        let entry = self
            .entries
            .get_mut(topic.id.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.id == topic.id)
            .ok_or(SubscribeError::Closed)?;
        entry
            .storage
            .as_any_mut()
            .downcast_mut::<LatestStorage<S>>()
            .ok_or(SubscribeError::ForeignTopic)
    }

    pub(crate) fn subscribe_latest<P: Shape, S: Payload>(
        &mut self,
        topic: &Topic<P>,
        options: SubscriptionOptions,
    ) -> Result<Subscriber<P>, SubscribeError> {
        if options.stream_depth.is_some() {
            return Err(SubscribeError::InvalidOptions);
        }
        let limit = self.config.max_subscribers_per_topic;
        let storage = self.latest_storage_mut::<P, S>(topic)?;
        storage.state.demand.check_add(limit)?;
        // Arc strong_count == 0 means destruction has begun, not that the
        // lease fields and receiver box have finished releasing allocations.
        // Acquire pairs with Subscriber's post-deallocation completion signal.
        let completed = storage.state.retired_receivers.load(Ordering::Acquire);
        let retired_slots: [bool; 64] = std::array::from_fn(|index| {
            storage.senders[index].is_some() && completed & (1 << index) != 0
        });
        // Demand can decrease before a concurrent receiver's deallocation
        // completes. Its sender still occupies one of the fixed physical slots.
        let sender_slot = storage
            .senders
            .iter()
            .enumerate()
            .position(|(index, sender)| sender.is_none() || retired_slots[index])
            .ok_or(SubscribeError::SubscriberLimit)?;
        let bytes = budget::latest_subscriber::<S>(storage.template.element_bytes())
            .map_err(|_| SubscribeError::BudgetExceeded)?;
        // The replacement is allocated while retired senders are still staged.
        // Bound this peak too, rather than lending their credit before release.
        self.budget
            .check(bytes)
            .map_err(|_| SubscribeError::BudgetExceeded)?;
        // Retired resources are only reclaimed after all fallible preparation;
        // failed subscriptions preserve both demand and the charged budget.
        let charge = self.budget.reserve(bytes);
        let storage = self.latest_storage_mut::<P, S>(topic)?;
        let (mut input, output) = triple_buffer::TripleBuffer::new(&storage.template).split();
        let lease = Arc::new(SubscriptionLease {
            state: storage.state.clone(),
            counted: false,
            sender_slot,
            delivered: std::sync::atomic::AtomicU64::new(0),
            overflow_drops: std::sync::atomic::AtomicU64::new(0),
        });
        let mut receiver = Box::new(LatestReceiver::new(output, lease));
        let demand = storage.state.demand.add(limit)?;
        Arc::get_mut(&mut receiver.lease)
            .expect("staged sole lease owner")
            .counted = true;
        storage.output.update();
        let current = storage.output.output_buffer();
        let mut seeded_sequence = 0;
        if current.meta().sequence != 0 && current.meta().activation_generation == demand.generation
        {
            input.input_buffer_mut().copy_from(current);
            input.publish();
            seeded_sequence = current.meta().sequence;
            crate::lifecycle::increment(&receiver.lease.delivered);
        }
        for (index, (slot, retire)) in storage.senders.iter_mut().zip(retired_slots).enumerate() {
            // Drop may race preparation on another subscriber thread. Reclaim
            // exactly the slots whose credit was included in staged accounting.
            if retire {
                *slot = None;
                storage.occupied &= !(1 << index);
                storage.fanout.recipients &= !(1 << index);
            }
        }
        // Only the Bus reuses slots, after observing the old completion. Clear
        // that bit before handing its identity to the replacement receiver.
        storage
            .state
            .retired_receivers
            .fetch_and(!(1 << sender_slot), Ordering::Relaxed);
        storage.senders[sender_slot] = Some(LatestSender {
            input,
            lease: Arc::downgrade(&receiver.lease),
            last_sequence: seeded_sequence,
            _charge: charge.clone(),
        });
        storage.occupied |= 1 << sender_slot;
        Ok(Subscriber::new(receiver, charge))
    }

    pub(crate) fn stream_storage_mut<P: Shape, S: Payload>(
        &mut self,
        topic: &Topic<P>,
    ) -> Result<&mut StreamStorage<S>, SubscribeError> {
        if topic.id.directory != self.directory {
            return Err(SubscribeError::ForeignTopic);
        }
        let entry = self
            .entries
            .get_mut(topic.id.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.id == topic.id)
            .ok_or(SubscribeError::Closed)?;
        entry
            .storage
            .as_any_mut()
            .downcast_mut::<StreamStorage<S>>()
            .ok_or(SubscribeError::ForeignTopic)
    }

    pub(crate) fn subscribe_stream<P: Shape, S: Payload>(
        &mut self,
        topic: &Topic<P>,
        options: SubscriptionOptions,
    ) -> Result<Subscriber<P>, SubscribeError> {
        let depth = options
            .stream_depth
            .unwrap_or(self.config.default_stream_depth);
        if depth == 0
            || depth
                .checked_add(1)
                .and_then(|slots| slots.checked_mul(2))
                .is_none()
        {
            return Err(SubscribeError::InvalidCapacity);
        }
        let limit = self.config.max_subscribers_per_topic;
        let storage = self.stream_storage_mut::<P, S>(topic)?;
        storage.state.demand.check_add(limit)?;
        storage
            .template
            .element_count()
            .checked_mul(depth + 1)
            .ok_or(SubscribeError::InvalidCapacity)?;
        let completed = storage.state.retired_receivers.load(Ordering::Acquire);
        let retired_slots: [bool; 64] = std::array::from_fn(|index| {
            storage.senders[index].is_some() && completed & (1 << index) != 0
        });
        let sender_slot = storage
            .senders
            .iter()
            .enumerate()
            .position(|(index, sender)| sender.is_none() || retired_slots[index])
            .ok_or(SubscribeError::SubscriberLimit)?;
        let bytes = budget::stream_subscriber::<S>(depth, storage.template.element_bytes())
            .map_err(|_| SubscribeError::BudgetExceeded)?;
        // Bound preparation's peak; retained sender/ring/payload owners cannot
        // lend their charge to a replacement until actual non-RT destruction.
        self.budget
            .check(bytes)
            .map_err(|_| SubscribeError::BudgetExceeded)?;
        let charge = self.budget.reserve(bytes);
        let storage = self.stream_storage_mut::<P, S>(topic)?;
        let (writer, reader) = pool(&storage.template, depth);
        let retirement = Arc::new(std::sync::Mutex::new(None));
        let lease = Arc::new(SubscriptionLease {
            state: storage.state.clone(),
            counted: false,
            sender_slot,
            delivered: std::sync::atomic::AtomicU64::new(0),
            overflow_drops: std::sync::atomic::AtomicU64::new(0),
        });
        let mut receiver = Box::new(StreamReceiver {
            reader: Some(reader),
            loan: storage.template.clone(),
            lease,
            fence: 0,
            generation: 0,
            retirement: Arc::clone(&retirement),
        });
        // The commit boundary is sequence allocation, not pump dequeue time.
        receiver.fence = crate::sequence::subscription_fence(&storage.state.attempt_sequence);
        let demand = storage.state.demand.add(limit)?;
        receiver.generation = demand.generation;
        Arc::get_mut(&mut receiver.lease)
            .expect("staged sole lease owner")
            .counted = true;
        for (index, (slot, retire)) in storage.senders.iter_mut().zip(retired_slots).enumerate() {
            if retire {
                *slot = None;
                storage.occupied &= !(1 << index);
                storage.fanout.recipients &= !(1 << index);
            }
        }
        storage
            .state
            .retired_receivers
            .fetch_and(!(1 << sender_slot), Ordering::Relaxed);
        storage.senders[sender_slot] = Some(StreamSender {
            writer,
            lease: Arc::downgrade(&receiver.lease),
            fence: receiver.fence,
            generation: receiver.generation,
            drops: 0,
            retirement,
            _charge: charge.clone(),
        });
        storage.occupied |= 1 << sender_slot;
        Ok(Subscriber::new(receiver, charge))
    }

    pub fn resolve<P: Shape>(&self, name: &str) -> Result<Topic<P>, LookupError> {
        let entry = self
            .entries
            .iter()
            .flatten()
            .find(|entry| entry.name.as_ref() == name)
            .ok_or(LookupError::NotFound)?;
        if entry.payload_type != P::payload_type() {
            return Err(LookupError::TypeMismatch);
        }
        if entry.mode != Mode::of::<P>() {
            return Err(LookupError::ModeMismatch);
        }
        Ok(Topic {
            id: entry.id,
            state: entry.storage.state().clone(),
            marker: PhantomData,
        })
    }

    pub fn register_latest_value<T: Copy + Send + Sync + 'static>(
        &mut self,
        name: &str,
        initial: T,
    ) -> Result<Topic<LatestValue<T>>, RegisterError> {
        let layout = budget::value_copy::<T>();
        let bytes = budget::latest::<ValueSlot<T>>(0)?;
        let staged = self.prepare(name, layout, bytes)?;
        let template = ValueSlot {
            meta: crate::PublicationMeta::INITIAL,
            value: initial,
        };
        self.commit(name, staged, LatestStorage::new(template))
    }

    pub fn register_stream_value<T: Copy + Send + Sync + 'static>(
        &mut self,
        name: &str,
        initial: T,
        depth: usize,
    ) -> Result<Topic<StreamValue<T>>, RegisterError> {
        Self::check_depth(depth)?;
        let layout = budget::value_copy::<T>();
        let bytes = budget::stream::<ValueSlot<T>>(depth, 0)?;
        let staged = self.prepare(name, layout, bytes)?;
        let template = ValueSlot {
            meta: crate::PublicationMeta::INITIAL,
            value: initial,
        };
        self.commit(name, staged, StreamStorage::new(template, depth))
    }

    pub fn register_latest_block<
        H: Copy + Send + Sync + 'static,
        E: Copy + Send + Sync + 'static,
    >(
        &mut self,
        name: &str,
        initial: BlockInit<H, E>,
    ) -> Result<Topic<LatestBlock<H, E>>, RegisterError> {
        self.register_latest_block_with(
            name,
            initial.capacity,
            || initial.header,
            || initial.element,
        )
    }

    pub fn register_stream_block<
        H: Copy + Send + Sync + 'static,
        E: Copy + Send + Sync + 'static,
    >(
        &mut self,
        name: &str,
        initial: BlockInit<H, E>,
        depth: usize,
    ) -> Result<Topic<StreamBlock<H, E>>, RegisterError> {
        self.register_stream_block_with(
            name,
            initial.capacity,
            || initial.header,
            || initial.element,
            depth,
        )
    }

    /// Calls the header factory once and the element factory `capacity` times
    /// to build an immutable template. All transport slots copy valid template
    /// contents; initialization never publishes a message.
    pub fn register_latest_block_with<
        H: Copy + Send + Sync + 'static,
        E: Copy + Send + Sync + 'static,
    >(
        &mut self,
        name: &str,
        capacity: usize,
        header_factory: impl FnMut() -> H,
        element_factory: impl FnMut() -> E,
    ) -> Result<Topic<LatestBlock<H, E>>, RegisterError> {
        Self::check_capacity(capacity)?;
        let layout = budget::block_copy::<H, E>(capacity)?;
        let bytes = budget::latest_block::<H, E>(capacity)?;
        let staged = self.prepare(name, layout, bytes)?;
        let template = block_template(capacity, header_factory, element_factory);
        self.commit(name, staged, LatestStorage::new(template))
    }

    /// Builds the immutable template as in `register_latest_block_with`, then
    /// prepares `depth` queued slots and an exclusive writer spare.
    pub fn register_stream_block_with<
        H: Copy + Send + Sync + 'static,
        E: Copy + Send + Sync + 'static,
    >(
        &mut self,
        name: &str,
        capacity: usize,
        header_factory: impl FnMut() -> H,
        element_factory: impl FnMut() -> E,
        depth: usize,
    ) -> Result<Topic<StreamBlock<H, E>>, RegisterError> {
        Self::check_capacity(capacity)?;
        Self::check_depth(depth)?;
        let layout = budget::block_copy::<H, E>(capacity)?;
        let bytes = budget::stream_block::<H, E>(capacity, depth)?;
        let staged = self.prepare(name, layout, bytes)?;
        let template = block_template(capacity, header_factory, element_factory);
        self.commit(name, staged, StreamStorage::new(template, depth))
    }

    fn check_capacity(capacity: usize) -> Result<(), RegisterError> {
        if capacity == 0 {
            return Err(RegisterError::InvalidCapacity);
        }
        Ok(())
    }

    fn check_depth(depth: usize) -> Result<(), RegisterError> {
        Self::check_capacity(depth)?;
        // The free owner ring has depth+1 slots and rtrb doubles its indices.
        depth
            .checked_add(1)
            .and_then(|free_depth| free_depth.checked_mul(2))
            .ok_or(RegisterError::SizeOverflow)?;
        Ok(())
    }

    fn prepare(
        &self,
        name: &str,
        payload_layout: Layout,
        bytes: usize,
    ) -> Result<Staged, RegisterError> {
        if name.is_empty() || name.len() > 256 || name.contains('\0') {
            return Err(RegisterError::InvalidName);
        }
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| entry.name.as_ref() == name)
        {
            return Err(RegisterError::DuplicateName);
        }
        if payload_layout.size() > self.config.max_payload_bytes {
            return Err(RegisterError::PayloadTooLarge);
        }
        if self.count == self.entries.len() {
            return Err(RegisterError::TopicLimit);
        }
        let next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(RegisterError::GenerationExhausted)?;
        let bytes = budget::add(bytes, budget::array::<u8>(name.len())?.size())?;
        self.budget.check(bytes)?;
        Ok(Staged {
            next_generation,
            bytes,
        })
    }

    fn commit<P: Shape, S: Dispatch>(
        &mut self,
        name: &str,
        staged: Staged,
        mut storage: S,
    ) -> Result<Topic<P>, RegisterError> {
        // Storage and name ownership remain staged through the last allocation.
        // A panicking factory or allocation never commits a name/slot/generation.
        let state_bytes = budget::state_bytes()?;
        storage
            .state_mut()
            .attach_charge(self.budget.reserve(state_bytes));
        let charge = self.budget.reserve(staged.bytes - state_bytes);
        let state = storage.state().clone();
        let storage = Box::new(storage);
        let name = Box::<str>::from(name);
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .expect("prepared vacant slot");
        let id = TopicId {
            directory: self.directory,
            slot,
            generation: self.next_generation,
        };
        self.entries[slot] = Some(Entry {
            id,
            name,
            payload_type: P::payload_type(),
            mode: Mode::of::<P>(),
            storage,
            _charge: charge,
        });
        self.next_generation = staged.next_generation;
        self.count += 1;
        Ok(Topic {
            id,
            state,
            marker: PhantomData,
        })
    }
}

struct Staged {
    next_generation: u64,
    bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_registration_preserves_slot_and_generation() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let before = bus.budget_used();
        bus.next_generation = u64::MAX;
        assert!(matches!(
            bus.register_latest_value("retry", 7_u32),
            Err(RegisterError::GenerationExhausted)
        ));
        assert_eq!(bus.budget_used(), before);
        assert_eq!(bus.count, 0);
        assert!(bus.entries.iter().all(Option::is_none));
        bus.next_generation = 4;
        let topic = bus.register_latest_value("retry", 7_u32).unwrap();
        assert_eq!((topic.id.slot, topic.id.generation), (0, 4));
        assert_eq!(bus.next_generation, 5);
        let before = bus.budget_used();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = bus.register_latest_block_with(
                    "factory-retry",
                    3,
                    || 1_u32,
                    || -> u32 { panic!("partial template") },
                );
            }))
            .is_err()
        );
        assert_eq!(bus.budget_used(), before);
        assert_eq!((bus.count, bus.next_generation), (1, 5));
        assert!(bus.entries[1].is_none());
        let next = bus.register_latest_value("factory-retry", 9_u32).unwrap();
        assert_eq!((next.id.slot, next.id.generation), (1, 5));
    }
}
