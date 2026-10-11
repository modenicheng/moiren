use crate::{
    LatestBlock, LatestValue, PublicationMeta, Shape,
    demand::Demand,
    storage::{BlockSlot, ValueSlot},
};
use std::{
    any::Any,
    marker::PhantomData,
    sync::{Arc, atomic::AtomicU64},
};

pub(crate) struct TopicState {
    pub(crate) demand: Demand,
    pub(crate) attempt_sequence: AtomicU64,
}
impl TopicState {
    pub(crate) fn new() -> Self {
        Self {
            demand: Demand::new(),
            attempt_sequence: AtomicU64::new(0),
        }
    }
}

pub(crate) struct SubscriptionLease {
    pub(crate) state: Arc<TopicState>,
    pub(crate) counted: bool,
}
impl Drop for SubscriptionLease {
    fn drop(&mut self) {
        if self.counted {
            self.state.demand.remove();
        }
    }
}

/// Independently owned receiver. Reading and dropping are non-RT operations.
pub struct Subscriber<P: Shape> {
    receiver: Box<dyn Any + Send>,
    marker: PhantomData<fn() -> P>,
}

// The complete receiver metadata lives in the budgeted allocation, rather than
// leaving its demand ownership and read fence as uncharged caller storage.
pub(crate) struct LatestReceiver<S: Clone + Send> {
    pub(crate) output: triple_buffer::Output<S>,
    pub(crate) lease: Arc<SubscriptionLease>,
    last_sequence: u64,
}
impl<S: Clone + Send> LatestReceiver<S> {
    pub(crate) fn new(output: triple_buffer::Output<S>, lease: Arc<SubscriptionLease>) -> Self {
        Self {
            output,
            lease,
            last_sequence: 0,
        }
    }
}

impl<P: Shape> Subscriber<P> {
    pub(crate) fn new(receiver: Box<dyn Any + Send>) -> Self {
        Self {
            receiver,
            marker: PhantomData,
        }
    }
    fn latest<S: Clone + Send + 'static>(
        &mut self,
        metadata: impl Fn(&S) -> PublicationMeta,
    ) -> Option<&S> {
        let receiver = self
            .receiver
            .downcast_mut::<LatestReceiver<S>>()
            .expect("sealed subscriber shape");
        let demand = receiver.lease.state.demand.snapshot();
        if !demand.active {
            return None;
        }
        let output = &mut receiver.output;
        output.update();
        let slot = output.output_buffer();
        let meta = metadata(slot);
        if meta.sequence == 0
            || meta.sequence == receiver.last_sequence
            || meta.activation_generation != demand.generation
        {
            return None;
        }
        receiver.last_sequence = meta.sequence;
        Some(slot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueMessage<T> {
    pub meta: PublicationMeta,
    pub value: T,
}

impl<T: Copy + Send + Sync + 'static> Subscriber<LatestValue<T>> {
    pub fn read_latest(&mut self) -> Option<ValueMessage<T>> {
        self.latest::<ValueSlot<T>>(|slot| slot.meta)
            .map(|slot| ValueMessage {
                meta: slot.meta,
                value: slot.value,
            })
    }
}

/// Borrows only this receiver's output slot, leaving every other receiver free.
pub struct BlockReadGuard<'a, H, E> {
    slot: &'a BlockSlot<H, E>,
}
impl<H, E> BlockReadGuard<'_, H, E> {
    pub fn meta(&self) -> PublicationMeta {
        self.slot.meta
    }
    pub fn header(&self) -> &H {
        &self.slot.header
    }
    pub fn elements(&self) -> &[E] {
        &self.slot.elements[..self.slot.meta.len]
    }
}
impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static>
    Subscriber<LatestBlock<H, E>>
{
    pub fn read_latest(&mut self) -> Option<BlockReadGuard<'_, H, E>> {
        self.latest::<BlockSlot<H, E>>(|slot| slot.meta)
            .map(|slot| BlockReadGuard { slot })
    }
}
