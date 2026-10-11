//! One movable RT owner; preparation and final reclamation are non-RT.
mod binding;
mod scope;
pub(crate) mod slot;

use crate::{
    ActivateError, HostError, Shape, Topic,
    budget::{self, Budget, Charge},
    lifecycle::{DirectoryId, TopicLease},
};
pub use binding::{
    BindingKey, ErasedBinding, HostId, OpSignalBindings, PreparedPublisher, PublisherHandle,
};
use binding::{BindingOwner, SlotRoot};
pub use scope::{SignalBlockWriteGuard, SignalScope};
use slot::Owner;
use std::{
    alloc::Layout,
    marker::PhantomData,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

static NEXT_HOST: AtomicU64 = AtomicU64::new(1);
type HostSlot = Option<(crate::TopicId, Owner<SlotRoot>)>;

fn identity(counter: &AtomicU64) -> Result<u64, HostError> {
    counter
        .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map_err(|_| HostError::BindingExhausted)
}

pub(crate) struct HostRoot {
    pub(crate) id: HostId,
    directory: DirectoryId,
    capacity: usize,
    stopped: AtomicBool,
    basis: AtomicU64,
    identities: AtomicU64,
    slots: Mutex<Box<[HostSlot]>>,
    budget: Budget,
}

/// Cloneable cleanup/preparation root. Keep one on non-RT until the host and
/// all returned updates have been transferred back for destruction.
#[derive(Clone)]
pub struct HostControl {
    root: Owner<HostRoot>,
}

pub(crate) struct Table {
    identity: u64,
    basis: u64,
    pub(crate) bindings: Box<[ErasedBinding]>,
    root: Owner<HostRoot>,
    _charge: Charge,
}

/// Owning candidate. Prepare/drop on non-RT; activation may move it on RT.
pub struct PreparedHostUpdate {
    table: Option<Table>,
}
impl std::fmt::Debug for PreparedHostUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedHostUpdate")
            .field("prepared", &self.table.is_some())
            .finish()
    }
}

/// Replaced owners must be returned to non-RT reclamation without dropping on RT.
#[must_use = "return retired owners to non-RT cleanup"]
pub struct RetiredHostUpdate {
    _table: Option<Table>,
}
impl std::fmt::Debug for RetiredHostUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetiredHostUpdate")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct ActivateFailure {
    pub error: ActivateError,
    pub update: PreparedHostUpdate,
}

/// Unique Send, non-Clone, non-Sync publisher authority. Drop only after the
/// audio owner stops and transfers this object back to non-RT cleanup.
///
/// ```compile_fail
/// fn duplicate(host: moiren_signal::RtHost) { let _ = host.clone(); }
/// ```
/// ```compile_fail
/// fn require_sync<Value: Sync>() {}
/// require_sync::<moiren_signal::RtHost>();
/// ```
pub struct RtHost {
    pub(crate) active: Option<Table>,
    pub(crate) root: Owner<HostRoot>,
    marker: PhantomData<std::cell::Cell<()>>,
}

impl HostControl {
    pub(crate) fn create(
        directory: DirectoryId,
        budget: &Budget,
        capacity: usize,
    ) -> Result<(Self, RtHost), HostError> {
        if capacity == 0 {
            return Err(HostError::InvalidCapacity);
        }
        let bytes = budget::array::<HostSlot>(capacity)
            .and_then(|slots| budget::add(slots.size(), budget::arc_layout::<HostRoot>()?.size()))
            .and_then(|bytes| budget::add(bytes, budget::reservation_bytes()?))
            .map_err(|_| HostError::BudgetExceeded)?;
        let charge = budget
            .try_reserve(bytes)
            .map_err(|_| HostError::BudgetExceeded)?;
        let id = HostId(identity(&NEXT_HOST)?);
        let root = Owner::new(
            HostRoot {
                id,
                directory,
                capacity,
                stopped: AtomicBool::new(false),
                basis: AtomicU64::new(0),
                identities: AtomicU64::new(1),
                slots: Mutex::new((0..capacity).map(|_| None).collect()),
                budget: budget.clone(),
            },
            charge,
        );
        Ok((
            Self { root: root.clone() },
            RtHost {
                root,
                active: None,
                marker: PhantomData,
            },
        ))
    }
    pub fn new_binding(&self) -> Result<BindingKey, HostError> {
        if self.root.stopped.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        Ok(BindingKey {
            host: self.root.id,
            value: identity(&self.root.identities)?,
        })
    }
    pub fn prepare<P: Shape>(
        &self,
        topic: &Topic<P>,
        key: BindingKey,
    ) -> Result<PreparedPublisher<P>, HostError> {
        if topic.id.directory != self.root.directory {
            return Err(HostError::ForeignTopic);
        }
        if key.host != self.root.id {
            return Err(HostError::UnknownBinding);
        }
        let mut slots = self
            .root
            .slots
            .lock()
            .expect("host preparation gate poisoned");
        if self.root.stopped.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        let mut leases = topic.state.gate.lock().expect("lifecycle gate poisoned");
        if topic.state.demand.is_closed() {
            return Err(HostError::Closed);
        }
        let owner = topic.state.host_owner.load(Ordering::Acquire);
        if owner != 0 && owner != self.root.id.0 {
            return Err(HostError::PublisherConflict);
        }
        let index = slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|(id, _)| *id == topic.id))
            .or_else(|| slots.iter().position(Option::is_none))
            .or_else(|| {
                slots.iter().position(|slot| {
                    slot.as_ref()
                        .is_some_and(|(_, slot)| slot.state.demand.is_closed())
                })
            })
            .ok_or(HostError::HostCapacity)?;
        let same_topic = slots[index].as_ref().is_some_and(|(id, _)| *id == topic.id);
        let bytes = budget::arc_layout::<BindingOwner>()
            .and_then(|layout| budget::add(layout.size(), budget::reservation_bytes()?))
            .map_err(|_| HostError::BudgetExceeded)?;
        let (charge, slot_charge) = if !same_topic {
            let slot_bytes = budget::arc_layout::<SlotRoot>()
                .and_then(|layout| budget::add(layout.size(), budget::reservation_bytes()?))
                .map_err(|_| HostError::BudgetExceeded)?;
            let (charge, slot_charge) = self
                .root
                .budget
                .try_reserve_pair(bytes, slot_bytes)
                .map_err(|_| HostError::BudgetExceeded)?;
            (charge, Some(slot_charge))
        } else {
            (
                self.root
                    .budget
                    .try_reserve(bytes)
                    .map_err(|_| HostError::BudgetExceeded)?,
                None,
            )
        };
        let identity = identity(&self.root.identities)?;
        leases.checked_add(1).ok_or(HostError::HostCapacity)?;
        let slot = match &slots[index] {
            Some((id, slot)) if *id == topic.id => slot.clone(),
            _ => {
                let cell = topic
                    .state
                    .cell
                    .lock()
                    .expect("publisher root poisoned")
                    .as_ref()
                    .expect("registered producer root")
                    .clone();
                let slot = Owner::new(
                    SlotRoot {
                        state: topic.state.clone(),
                        cell,
                    },
                    slot_charge.expect("reserved new slot"),
                );
                if let Some((_, old)) = &slots[index] {
                    // Closed is permanent and required every plan lease to
                    // finish. No active/retired RT table can refer to this slot;
                    // reclaim its claim under its gate, then free only off RT.
                    let old_leases = old.state.gate.lock().expect("lifecycle gate poisoned");
                    debug_assert_eq!(*old_leases, 0);
                    old.state.host_owner.store(0, Ordering::Release);
                }
                slots[index] = Some((topic.id, slot.clone()));
                topic
                    .state
                    .host_owner
                    .store(self.root.id.0, Ordering::Release);
                slot
            }
        };
        let (pointer, refresh) = P::bind_cell(&slot.cell);
        let lease = TopicLease::acquire_locked(&topic.state, &mut leases)
            .expect("validated lifecycle lease");
        Ok(PreparedPublisher {
            binding: ErasedBinding {
                owner: Owner::new(
                    BindingOwner {
                        host: self.root.id,
                        key,
                        identity,
                        topic: topic.id,
                        capacity: slot.cell.capacity(),
                        pointer,
                        refresh,
                        slot,
                        _lease: lease,
                    },
                    charge,
                ),
            },
            marker: PhantomData,
        })
    }
    pub fn prepare_update(
        &self,
        bindings: &[ErasedBinding],
    ) -> Result<PreparedHostUpdate, HostError> {
        let _gate = self
            .root
            .slots
            .lock()
            .expect("host preparation gate poisoned");
        if self.root.stopped.load(Ordering::Acquire) {
            return Err(HostError::Closed);
        }
        if bindings.len() > self.root.capacity {
            return Err(HostError::HostCapacity);
        }
        for (index, binding) in bindings.iter().enumerate() {
            if binding.owner.host != self.root.id {
                return Err(HostError::PublisherConflict);
            }
            if binding.owner.slot.state.demand.is_closed() {
                return Err(HostError::Closed);
            }
            if bindings[..index]
                .iter()
                .any(|other| other.owner.topic == binding.owner.topic)
            {
                return Err(HostError::PublisherConflict);
            }
        }
        let charge = table_charge(
            &self.root.budget,
            bindings.len(),
            Layout::new::<Table>().size(),
        )?;
        let table = Table {
            identity: identity(&self.root.identities)?,
            basis: self.root.basis.load(Ordering::Acquire),
            bindings: bindings.to_vec().into_boxed_slice(),
            root: self.root.clone(),
            _charge: charge,
        };
        Ok(PreparedHostUpdate { table: Some(table) })
    }
}

pub(crate) fn table_charge(
    budget: &Budget,
    count: usize,
    metadata: usize,
) -> Result<Charge, HostError> {
    let bytes = budget::array::<ErasedBinding>(count)
        .and_then(|layout| budget::add(layout.size(), metadata))
        .and_then(|bytes| budget::add(bytes, budget::reservation_bytes()?))
        .map_err(|_| HostError::BudgetExceeded)?;
    budget
        .try_reserve(bytes)
        .map_err(|_| HostError::BudgetExceeded)
}

impl PreparedHostUpdate {
    pub fn op_bindings(&self, key: BindingKey) -> Result<OpSignalBindings, HostError> {
        let table = self.table.as_ref().ok_or(HostError::Closed)?;
        let count = table
            .bindings
            .iter()
            .filter(|binding| binding.owner.key == key)
            .count();
        if key.host != table.root.id || count == 0 {
            return Err(HostError::UnknownBinding);
        }
        let charge = table_charge(
            &table.root.budget,
            count,
            Layout::new::<OpSignalBindings>().size(),
        )?;
        let mut bindings = Vec::with_capacity(count);
        bindings.extend(
            table
                .bindings
                .iter()
                .filter(|binding| binding.owner.key == key)
                .cloned(),
        );
        Ok(OpSignalBindings {
            table: table.identity,
            host: table.root.id,
            key,
            bindings: bindings.into_boxed_slice(),
            _root: table.root.clone(),
            _charge: charge,
        })
    }
}

/// Validation holds both exclusive borrows. Dropping it leaves every owner in
/// place; committing only moves preallocated tables and returns replaced owners.
///
/// ```compile_fail
/// fn send(transaction: moiren_signal::ActivationTransaction<'_>) {
///     std::thread::scope(|threads| { threads.spawn(move || drop(transaction)); });
/// }
/// ```
/// ```compile_fail
/// fn duplicate(transaction: moiren_signal::ActivationTransaction<'_>) { let _ = transaction.clone(); }
/// ```
pub struct ActivationTransaction<'a> {
    host: &'a mut RtHost,
    update: &'a mut PreparedHostUpdate,
    marker: PhantomData<std::rc::Rc<()>>,
}
impl Drop for ActivationTransaction<'_> {
    fn drop(&mut self) {}
}
impl ActivationTransaction<'_> {
    pub fn commit(self) -> RetiredHostUpdate {
        let table = self.update.table.take().expect("validated prepared table");
        if let Some(active) = &self.host.active {
            for binding in &active.bindings {
                binding.owner.slot.state.set_bound_leased(false);
            }
        }
        for binding in &table.bindings {
            binding.owner.slot.state.set_bound_leased(true);
        }
        self.host
            .root
            .basis
            .store(table.identity, Ordering::Release);
        RetiredHostUpdate {
            _table: self.host.active.replace(table),
        }
    }
}
impl RtHost {
    pub fn activation<'a>(
        &'a mut self,
        update: &'a mut PreparedHostUpdate,
    ) -> Result<ActivationTransaction<'a>, ActivateError> {
        let table = update.table.as_ref().ok_or(ActivateError::StaleBasis)?;
        if table.root.id != self.root.id {
            return Err(ActivateError::ForeignHost);
        }
        if self.root.stopped.load(Ordering::Acquire) {
            return Err(ActivateError::Closed);
        }
        if table.basis != self.root.basis.load(Ordering::Acquire) {
            return Err(ActivateError::StaleBasis);
        }
        if table.bindings.len() > self.root.capacity {
            return Err(ActivateError::HostCapacity);
        }
        for binding in &table.bindings {
            if binding.owner.slot.state.demand.is_closed() {
                return Err(ActivateError::Closed);
            }
            if binding.owner.slot.state.host_owner.load(Ordering::Acquire) != self.root.id.0 {
                return Err(ActivateError::PublisherConflict);
            }
        }
        Ok(ActivationTransaction {
            host: self,
            update,
            marker: PhantomData,
        })
    }
    pub fn activate(
        &mut self,
        mut update: PreparedHostUpdate,
    ) -> Result<RetiredHostUpdate, ActivateFailure> {
        let result = match self.activation(&mut update) {
            Ok(transaction) => Ok(transaction.commit()),
            Err(error) => Err(error),
        };
        result.map_err(|error| ActivateFailure { error, update })
    }
    pub fn begin_block(&mut self) {
        slot::refresh(self);
    }
    pub fn scope<'a>(&'a mut self, bindings: &'a OpSignalBindings) -> SignalScope<'a> {
        SignalScope {
            host: self,
            bindings,
            marker: PhantomData,
        }
    }
}
impl Drop for RtHost {
    fn drop(&mut self) {
        let slots = self
            .root
            .slots
            .lock()
            .expect("host preparation gate poisoned");
        self.root.stopped.store(true, Ordering::Release);
        for (_, slot) in slots.iter().flatten() {
            let _gate = slot.state.gate.lock().expect("lifecycle gate poisoned");
            slot.state.publisher_bound.store(false, Ordering::Release);
            slot.state.host_owner.store(0, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, BusConfig, PublishOutcome, UnregisterError, test_alloc as allocator};

    #[test]
    fn failed_new_slot_preparation_allocates_nothing_and_keeps_all_ownership() {
        let mut measure = Bus::new(Default::default()).unwrap();
        measure.register_latest_value("value", 0_u32).unwrap();
        let (_control, _host) = measure.create_host(1).unwrap();
        let owner_bytes = budget::arc_layout::<BindingOwner>().unwrap().size()
            + budget::reservation_bytes().unwrap();
        let mut bus = Bus::new(BusConfig {
            byte_budget: measure.budget_used() + owner_bytes,
            ..Default::default()
        })
        .unwrap();
        let topic = bus.register_latest_value("value", 0_u32).unwrap();
        let (control, _host) = bus.create_host(1).unwrap();
        let key = control.new_binding().unwrap();
        let before = bus.budget_used();
        assert_eq!(
            allocator::measure(|| {
                assert!(matches!(
                    control.prepare(&topic, key),
                    Err(HostError::BudgetExceeded)
                ));
            }),
            allocator::Counts::default()
        );
        assert_eq!(bus.budget_used(), before);
        assert_eq!(topic.state.host_owner.load(Ordering::Acquire), 0);
        assert_eq!(*topic.state.gate.lock().unwrap(), 0);
        bus.unregister(&topic).unwrap();
    }

    #[test]
    fn stable_actual_cell_and_sequence_survive_updates_and_stopped_rebind() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_latest_value("value", 0_u32).unwrap();
        let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, mut host) = bus.create_host(1).unwrap();
        let key = control.new_binding().unwrap();
        let first = control.prepare(&topic, key).unwrap();
        let pointer = first.handle().pointer;
        let update = control.prepare_update(&[first.erase()]).unwrap();
        let op = update.op_bindings(key).unwrap();
        drop(host.activate(update).unwrap());
        host.begin_block();
        host.scope(&op).publish(&first.handle(), 1).unwrap();
        let next_key = control.new_binding().unwrap();
        let next = control.prepare(&topic, next_key).unwrap();
        assert!(pointer == next.handle().pointer);
        let update = control.prepare_update(&[next.erase()]).unwrap();
        let next_op = update.op_bindings(next_key).unwrap();
        drop(host.activate(update).unwrap());
        assert!(
            matches!(host.scope(&next_op).publish(&next.handle(), 2).unwrap(), PublishOutcome::Published(meta) if meta.sequence == 2)
        );
        bus.pump(crate::PumpBudget {
            topic_checks: 1024,
            deliveries: 32,
            copy_bytes: 4096,
        });
        assert_eq!(subscriber.read_latest().unwrap().value, 2);
        drop(host);
        let (rebound_control, mut rebound) = bus.create_host(1).unwrap();
        let rebound_key = rebound_control.new_binding().unwrap();
        let rebound_publisher = rebound_control.prepare(&topic, rebound_key).unwrap();
        assert!(pointer == rebound_publisher.handle().pointer);
        let update = rebound_control
            .prepare_update(&[rebound_publisher.erase()])
            .unwrap();
        let rebound_op = update.op_bindings(rebound_key).unwrap();
        drop(rebound.activate(update).unwrap());
        let mut late = bus.subscribe(&topic, Default::default()).unwrap();
        assert_eq!(late.read_latest().unwrap().value, 2);
        assert!(
            matches!(rebound.scope(&rebound_op).publish(&rebound_publisher.handle(), 3).unwrap(), PublishOutcome::Published(meta) if meta.sequence == 3 && meta.activation_generation == 1)
        );
    }

    #[test]
    fn cleanup_root_outlives_bus_rt_and_retired_owners_and_actual_cell_frees_on_cleanup_thread() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let baseline = bus.budget_used();
        let topic = bus.register_stream_value("value", 0_u32, 2).unwrap();
        let subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, mut host) = bus.create_host(1).unwrap();
        let cleanup = control.clone();
        let ledger = cleanup.root.budget.clone();
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&topic, key).unwrap();
        let handle = publisher.handle();
        let address = handle.pointer.address();
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        let op = update.op_bindings(key).unwrap();
        drop(host.activate(update).unwrap());
        let mut update = control.prepare_update(&[publisher.erase()]).unwrap();
        let next_op = update.op_bindings(key).unwrap();
        drop(bus);
        drop(control);
        let (host, retired, op, next_op, counts, rt_thread) = std::thread::spawn(move || {
            let rt_thread = std::thread::current().id();
            let mut retired = None;
            let counts = allocator::measure(|| {
                host.begin_block();
                assert!(host.scope(&op).demand(&handle).unwrap().active);
                assert!(matches!(host.scope(&op).publish(&handle, 1), Ok(PublishOutcome::Published(meta)) if meta.sequence == 1));
                drop(host.activation(&mut update).unwrap());
                assert!(matches!(host.scope(&op).publish(&handle, 2), Ok(PublishOutcome::Published(meta)) if meta.sequence == 2));
                retired = Some(host.activation(&mut update).unwrap().commit());
                assert!(matches!(host.scope(&next_op).publish(&handle, 3), Ok(PublishOutcome::DroppedFull { sequence: 3 })));
            });
            (host, retired, op, next_op, counts, rt_thread)
        }).join().unwrap();
        assert_eq!(counts, allocator::Counts::default());
        let (frees, cleanup_thread) = std::thread::spawn(move || {
            let cleanup_thread = std::thread::current().id();
            let frees = allocator::frees(|| {
                drop((
                    host, retired, op, next_op, publisher, subscriber, topic, cleanup,
                ))
            });
            (frees, cleanup_thread)
        })
        .join()
        .unwrap();
        assert_ne!(rt_thread, cleanup_thread);
        assert!(
            frees.events[..frees.count].contains(&Some((address, cleanup_thread))),
            "actual producer cell box must free on cleanup thread"
        );
        assert!(
            frees.count > 10,
            "producer/state/transport/lease/reservation owners really freed"
        );
        assert!(
            frees.events[..frees.count]
                .iter()
                .all(|event| event.unwrap().1 == cleanup_thread)
        );
        assert_eq!(
            ledger.used(),
            baseline,
            "all producer, state, lease, table, slot and host charges must refund after real cleanup"
        );
    }

    #[test]
    fn all_host_identity_allocations_are_checked_without_wrapping() {
        let counter = AtomicU64::new(u64::MAX);
        assert_eq!(identity(&counter), Err(HostError::BindingExhausted));
        assert_eq!(counter.load(Ordering::Acquire), u64::MAX);
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_latest_value("exhaustion", 0_u32).unwrap();
        let (control, _host) = bus.create_host(1).unwrap();
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&topic, key).unwrap();
        control.root.identities.store(u64::MAX, Ordering::Release);
        assert_eq!(control.new_binding(), Err(HostError::BindingExhausted));
        assert!(matches!(
            control.prepare(&topic, key),
            Err(HostError::BindingExhausted)
        ));
        assert!(matches!(
            control.prepare_update(&[publisher.erase()]),
            Err(HostError::BindingExhausted)
        ));
        assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
    }

    #[test]
    fn new_topic_activation_and_owned_failure_move_complete_preallocated_owners_without_free() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_latest_value("existing", 0_u32).unwrap();
        let _subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, mut host) = bus.create_host(2).unwrap();
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&topic, key).unwrap();
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        drop(host.activate(update).unwrap());
        host.begin_block();
        let new_topic = bus.register_stream_value("new", 0_u32, 2).unwrap();
        let _new_subscriber = bus.subscribe(&new_topic, Default::default()).unwrap();
        let new_publisher = control.prepare(&new_topic, key).unwrap();
        let pointer = new_publisher.handle().pointer;
        let stale = control
            .prepare_update(&[publisher.erase(), new_publisher.erase()])
            .unwrap();
        let update = control
            .prepare_update(&[publisher.erase(), new_publisher.erase()])
            .unwrap();
        let op = update.op_bindings(key).unwrap();
        let mut retired = None;
        let mut failed = None;
        assert_eq!(
            allocator::measure(|| {
                retired = Some(host.activate(update).unwrap());
                host.begin_block();
                host.scope(&op).publish(&new_publisher.handle(), 5).unwrap();
                failed = Some(host.activate(stale).unwrap_err());
            }),
            allocator::Counts::default()
        );
        assert!(pointer == new_publisher.handle().pointer);
        let failure = failed.unwrap();
        assert_eq!(failure.error, ActivateError::StaleBasis);
        assert!(failure.update.op_bindings(key).is_ok());
        drop(retired);
    }
}
