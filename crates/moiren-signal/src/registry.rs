use std::{alloc::Layout, any::Any, marker::PhantomData};

use crate::{
    BlockInit, BusConfig, LatestBlock, LatestValue, LookupError, RegisterError, Shape, StreamBlock,
    StreamValue, Topic, TopicId,
    budget::{self, Budget},
    lifecycle::DirectoryId,
    storage::{LatestStorage, StreamStorage, ValueSlot, block_template},
    types::{Mode, PayloadType},
};

struct Entry {
    id: TopicId,
    name: Box<str>,
    payload_type: PayloadType,
    mode: Mode,
    // Erasure and name lookup occur only on the non-RT preparation side.
    // No publisher/host bypass is exposed by this directory.
    #[allow(dead_code)]
    storage: Box<dyn Any + Send>,
}

/// Non-real-time directory and owner of prepared topic resources.
pub struct Bus {
    config: BusConfig,
    directory: DirectoryId,
    entries: Box<[Option<Entry>]>,
    count: usize,
    next_generation: u64,
    budget: Budget,
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
            .checked_mul(2)
            .ok_or(RegisterError::SizeOverflow)?;
        let directory_layout = budget::array::<Option<Entry>>(config.max_topics)?;
        // Count the bus's own inline metadata and every fixed directory slot,
        // including vacant entries. No directory growth is possible later.
        let baseline = budget::add(Layout::new::<Self>().size(), directory_layout.size())?;
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
        })
    }

    /// Bytes owned by the bus: inline metadata, reserved directory slots and
    /// every registered name, transport, template, queue and dispatch slot.
    /// Sizes describe allocation layouts, excluding allocator bookkeeping.
    pub fn budget_used(&self) -> usize {
        self.budget.used
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
        depth.checked_mul(2).ok_or(RegisterError::SizeOverflow)?;
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
        let next_budget = self.budget.check(bytes)?;
        Ok(Staged {
            next_generation,
            next_budget,
        })
    }

    fn commit<P: Shape, S: Any + Send>(
        &mut self,
        name: &str,
        staged: Staged,
        storage: S,
    ) -> Result<Topic<P>, RegisterError> {
        // Storage and name ownership remain staged through the last allocation.
        // A panicking factory or allocation never commits a name/slot/generation.
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
        });
        self.next_generation = staged.next_generation;
        self.budget.used = staged.next_budget;
        self.count += 1;
        Ok(Topic {
            id,
            marker: PhantomData,
        })
    }
}

struct Staged {
    next_generation: u64,
    next_budget: usize,
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
