use std::{
    alloc::Layout,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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
        LatestSender, LatestStorage, StreamSender, StreamStorage, ValueSlot, block_template,
        detach_sender, pool,
    },
    subscriber::{LatestReceiver, StreamReceiver, Subscriber, SubscriptionLease},
    types::{Mode, PayloadType},
};

/// Slots whose receiver signalled post-deallocation completion while its
/// sender is still physically installed. Arc strong_count == 0 only means
/// destruction has begun, not that the lease fields and receiver box have
/// finished releasing allocations, so `completed` must be the Acquire load
/// pairing with the receiver Drop's post-deallocation Release signal.
fn retired_slot_snapshot<S>(senders: &[Option<S>; 64], completed: u64) -> [bool; 64] {
    std::array::from_fn(|index| senders[index].is_some() && completed & (1 << index) != 0)
}

/// First slot a replacement subscriber may adopt: physically free, or
/// retired and therefore reclaimable by this preparation.
fn next_sender_slot<S>(
    senders: &[Option<S>; 64],
    retired_slots: &[bool; 64],
) -> Result<usize, SubscribeError> {
    senders
        .iter()
        .enumerate()
        .position(|(index, sender)| sender.is_none() || retired_slots[index])
        .ok_or(SubscribeError::SubscriberLimit)
}

/// Reclaims the staged-retired physical slots, then clears the adopted
/// slot's completion bit. This function is the single authority for the
/// ordering invariant shared by `subscribe_latest` and `subscribe_stream`:
/// every fallible step — `demand.check_add`, slot selection and
/// `Bus::reserve_subscription_bytes` — must complete before this call, so a
/// reservation always precedes reclamation and a failed subscription leaves
/// retired slots staged and the budget untouched; after this call the
/// replacement sender must be installed and `occupied` extended immediately,
/// with no fallible step in between.
///
/// Mirrors `storage::retire_sender`'s field order: each retired slot is
/// detached from senders/occupied/fanout.recipients first. The adopted
/// `sender_slot`'s completion bit is cleared with Relaxed after all
/// reclaims and strictly before the replacement sender is installed, so no
/// in-progress receiver Drop can set a stale flag for the replacement
/// (tests/registry.rs: `retiring_receiver_keeps_its_sender_slot_until_drop_completes`).
/// Bits of merely reclaimed slots stay set; `occupied` no longer selects
/// them and their sender is `None`, so no later reader observes them.
fn reclaim_retired_and_clear_adopted<S>(
    senders: &mut [Option<S>; 64],
    occupied: &mut u64,
    recipients: &mut u64,
    retired_receivers: &AtomicU64,
    retired_slots: [bool; 64],
    sender_slot: usize,
) {
    // Drop may race preparation on another subscriber thread. Reclaim
    // exactly the slots whose credit was included in staged accounting.
    for (index, retire) in retired_slots.into_iter().enumerate() {
        if retire {
            detach_sender(senders, occupied, recipients, index);
        }
    }
    // Only the Bus reuses slots, after observing the old completion. Clear
    // that bit before handing its identity to the replacement receiver.
    retired_receivers.fetch_and(!(1 << sender_slot), Ordering::Relaxed);
}

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
    pub fn create_host(
        &mut self,
        max_bindings: usize,
    ) -> Result<(crate::HostControl, crate::RtHost), crate::HostError> {
        crate::host::HostControl::create(self.directory, &self.budget, max_bindings)
    }
    pub fn new(config: BusConfig) -> Result<Self, RegisterError> {
        if config.max_topics == 0
            || !(1..=64).contains(&config.max_subscribers_per_topic)
            || config.default_stream_depth == 0
        {
            return Err(RegisterError::InvalidCapacity);
        }
        budget::ring_index_bound(config.default_stream_depth)?;
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

    /// Directory lookup shared by both transport accessors: validates the
    /// bus identity and current generation, then returns the erased storage.
    fn entry_storage_mut<P: Shape>(
        &mut self,
        topic: &Topic<P>,
    ) -> Result<&mut dyn Dispatch, SubscribeError> {
        if topic.id.directory != self.directory {
            return Err(SubscribeError::ForeignTopic);
        }
        self.entries
            .get_mut(topic.id.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.id == topic.id)
            .map(|entry| &mut *entry.storage)
            .ok_or(SubscribeError::Closed)
    }

    pub(crate) fn latest_storage_mut<P: Shape, S: Payload>(
        &mut self,
        topic: &Topic<P>,
    ) -> Result<&mut LatestStorage<S>, SubscribeError> {
        self.entry_storage_mut(topic)?
            .as_any_mut()
            .downcast_mut::<LatestStorage<S>>()
            .ok_or(SubscribeError::ForeignTopic)
    }

    /// Checks headroom and reserves one subscription's bytes as a single
    /// charged pair, strictly before any fallible business preparation or
    /// reclamation. A replacement is allocated while retired senders are still
    /// staged, so this bounds the peak rather than lending retired credit
    /// before its actual non-RT release; retained sender/ring/payload owners
    /// keep their charge until real destruction. A failed check or reservation
    /// changes neither demand nor the budget.
    fn reserve_subscription_bytes(&self, bytes: usize) -> Result<Charge, SubscribeError> {
        self.budget
            .check(bytes)
            .map_err(|_| SubscribeError::BudgetExceeded)?;
        self.budget
            .try_reserve(bytes)
            .map_err(|_| SubscribeError::BudgetExceeded)
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
        let completed = storage.state.retired_receivers.load(Ordering::Acquire);
        let retired_slots = retired_slot_snapshot(&storage.senders, completed);
        // Demand can decrease before a concurrent receiver's deallocation
        // completes. Its sender still occupies one of the fixed physical slots.
        let sender_slot = next_sender_slot(&storage.senders, &retired_slots)?;
        let bytes = budget::latest_subscriber::<S>(storage.template.element_bytes())
            .map_err(|_| SubscribeError::BudgetExceeded)?;
        let charge = self.reserve_subscription_bytes(bytes)?;
        let storage = self.latest_storage_mut::<P, S>(topic)?;
        let (mut input, output) = triple_buffer::TripleBuffer::new(&storage.template).split();
        let lease = SubscriptionLease::staged(&storage.state, sender_slot);
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
        reclaim_retired_and_clear_adopted(
            &mut storage.senders,
            &mut storage.occupied,
            &mut storage.fanout.recipients,
            &storage.state.retired_receivers,
            retired_slots,
            sender_slot,
        );
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
        self.entry_storage_mut(topic)?
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
        if depth == 0 || budget::ring_index_bound(depth).is_err() {
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
        let retired_slots = retired_slot_snapshot(&storage.senders, completed);
        let sender_slot = next_sender_slot(&storage.senders, &retired_slots)?;
        let bytes = budget::stream_subscriber::<S>(depth, storage.template.element_bytes())
            .map_err(|_| SubscribeError::BudgetExceeded)?;
        let charge = self.reserve_subscription_bytes(bytes)?;
        let storage = self.stream_storage_mut::<P, S>(topic)?;
        let (writer, reader) = pool(&storage.template, depth);
        let retirement = Arc::new(std::sync::Mutex::new(None));
        let lease = SubscriptionLease::staged(&storage.state, sender_slot);
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
        reclaim_retired_and_clear_adopted(
            &mut storage.senders,
            &mut storage.occupied,
            &mut storage.fanout.recipients,
            &storage.state.retired_receivers,
            retired_slots,
            sender_slot,
        );
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
        budget::ring_index_bound(depth)?;
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
        let state_bytes = budget::state_bytes()?;
        let (state_charge, charge) = self
            .budget
            .try_reserve_pair(state_bytes, bytes - state_bytes)?;
        Ok(Staged {
            next_generation,
            state_charge,
            charge,
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
        storage.state_mut().attach_charge(staged.state_charge);
        let charge = staged.charge;
        storage.cell_mut().attach_charge(charge.clone());
        let cell = storage.cell_mut().clone();
        *storage
            .state()
            .cell
            .lock()
            .expect("publisher root poisoned") = Some(cell);
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
    state_charge: Charge,
    charge: Charge,
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
