use std::{
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::RegisterError;
use crate::{UnregisterError, budget::Charge, subscriber::TopicState};

#[derive(Clone)]
/// No weak handles escape. Moving the final state out of its Arc first lets
/// its reservation outlive the actual state allocation's deallocator.
pub(crate) struct StateHandle(Option<Arc<TopicState>>);

impl std::fmt::Debug for StateHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TopicState")
            .field("status", &self.status())
            .finish()
    }
}
impl StateHandle {
    pub(crate) fn new() -> Self {
        Self(Some(Arc::new(TopicState::new())))
    }
    pub(crate) fn attach_charge(&mut self, charge: Charge) {
        Arc::get_mut(self.0.as_mut().expect("live state"))
            .expect("unshared staged state")
            .charge = Some(charge);
    }
    pub(crate) fn close(&self) -> Result<(), UnregisterError> {
        let leases = self.gate.lock().expect("lifecycle gate poisoned");
        if self.demand.is_closed() {
            return Err(UnregisterError::Closed);
        }
        if *leases != 0 || self.publisher_bound.load(Ordering::Acquire) {
            return Err(UnregisterError::Busy);
        }
        self.demand.close();
        self.cell.lock().expect("publisher root poisoned").take();
        Ok(())
    }
    pub(crate) fn set_bound_leased(&self, bound: bool) {
        // Preparation acquires the complete plan lease under gate, and every
        // active/retired/table owner retains it. close() tests that lease under
        // the same gate, so an RT Bound transition cannot race successful close.
        // No RT mutex is needed: only the unique host writes Bound, and final
        // lease release and stopped-host ownership changes occur under gate.
        self.publisher_bound.store(bound, Ordering::Release);
    }
}
impl Deref for StateHandle {
    type Target = TopicState;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("live state")
    }
}
impl Drop for StateHandle {
    fn drop(&mut self) {
        if let Some(state) = Arc::into_inner(self.0.take().expect("live state")) {
            drop(state);
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
/// Non-RT lifecycle ownership for prepared, active and retired resources.
/// Binding changes must use the same state gate before changing Bound status.
pub(crate) struct TopicLease {
    state: StateHandle,
}
impl TopicLease {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn acquire(state: &StateHandle) -> Result<Self, UnregisterError> {
        let mut leases = state.gate.lock().expect("lifecycle gate poisoned");
        Self::acquire_locked(state, &mut leases)
    }
    pub(crate) fn acquire_locked(
        state: &StateHandle,
        leases: &mut usize,
    ) -> Result<Self, UnregisterError> {
        if state.demand.is_closed() {
            return Err(UnregisterError::Closed);
        }
        *leases = leases.checked_add(1).ok_or(UnregisterError::Busy)?;
        Ok(Self {
            state: state.clone(),
        })
    }
}
impl Drop for TopicLease {
    fn drop(&mut self) {
        *self.state.gate.lock().expect("lifecycle gate poisoned") -= 1;
    }
}

/// Each counter has one writer: the unique publisher or the serialized Bus.
pub(crate) fn increment(counter: &AtomicU64) {
    counter.store(
        counter.load(Ordering::Relaxed).saturating_add(1),
        Ordering::Relaxed,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DirectoryId(u64);

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

impl DirectoryId {
    pub(crate) fn allocate() -> Result<Self, RegisterError> {
        checked_identity(&NEXT_DIRECTORY).map(Self)
    }
}

fn checked_identity(counter: &AtomicU64) -> Result<u64, RegisterError> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current
            .checked_add(1)
            .ok_or(RegisterError::GenerationExhausted)?;
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(current),
            Err(observed) => current = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, LatestValue, PublisherStatus, TopicLifecycle};

    #[test]
    fn bound_and_each_live_lease_make_unregister_busy_without_mutation() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_latest_value("busy", 7_u32).unwrap();
        let subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let before = bus.budget_used();
        {
            let _gate = topic.state.gate.lock().unwrap();
            topic.state.publisher_bound.store(true, Ordering::Release);
        }
        assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
        assert_eq!(bus.budget_used(), before);
        assert!(bus.has_business_demand());
        assert_eq!(subscriber.status().lifecycle, TopicLifecycle::Registered);
        assert_eq!(subscriber.status().publisher, PublisherStatus::Bound);
        assert_eq!(bus.resolve::<LatestValue<u32>>("busy").unwrap(), topic);
        {
            let _gate = topic.state.gate.lock().unwrap();
            topic.state.publisher_bound.store(false, Ordering::Release);
        }
        let leases: Vec<_> = (0..3)
            .map(|_| TopicLease::acquire(&topic.state).unwrap())
            .collect();
        for lease in leases {
            assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
            assert_eq!(bus.budget_used(), before);
            drop(lease);
        }
        bus.unregister(&topic).unwrap();
        assert_eq!(subscriber.status().lifecycle, TopicLifecycle::Closed);
        assert!(matches!(
            TopicLease::acquire(&topic.state),
            Err(UnregisterError::Closed)
        ));
        drop(subscriber);
        assert!(!topic.state.demand.snapshot().active);
    }

    #[test]
    fn lease_acquisition_and_unregister_share_the_gate_in_both_orders() {
        use std::sync::Barrier;
        for lease_first in [true, false] {
            let mut bus = Bus::new(Default::default()).unwrap();
            let topic = bus.register_latest_value("race", 7_u32).unwrap();
            let state = topic.state.clone();
            let acquired = Barrier::new(2);
            let release = Barrier::new(2);
            std::thread::scope(|scope| {
                if lease_first {
                    let worker = scope.spawn(|| {
                        let lease = TopicLease::acquire(&state).unwrap();
                        acquired.wait();
                        release.wait();
                        drop(lease);
                    });
                    acquired.wait();
                    assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
                    release.wait();
                    worker.join().unwrap();
                    bus.unregister(&topic).unwrap();
                } else {
                    bus.unregister(&topic).unwrap();
                    scope
                        .spawn(|| {
                            assert!(matches!(
                                TopicLease::acquire(&state),
                                Err(UnregisterError::Closed)
                            ))
                        })
                        .join()
                        .unwrap();
                }
            });
        }
    }
    #[test]
    fn exhausted_identity_never_wraps() {
        let counter = AtomicU64::new(u64::MAX);
        assert_eq!(
            checked_identity(&counter),
            Err(RegisterError::GenerationExhausted)
        );
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}
