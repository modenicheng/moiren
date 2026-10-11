use crate::{
    LatestBlock, LatestValue, PublicationMeta, PublisherStatus, ReadError, Shape, StreamBlock,
    StreamValue, TopicLifecycle, TopicStatus,
    demand::Demand,
    storage::Payload,
    storage::{BlockSlot, PoolReader, ValueSlot},
};
use std::{
    any::Any,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub(crate) struct TopicState {
    pub(crate) demand: Demand,
    pub(crate) attempt_sequence: AtomicU64,
    pub(crate) ingress_drops: AtomicU64,
    pub(crate) publisher_bound: AtomicBool,
    // Sender slots may be refunded only after their receiver box is freed.
    // This bitmap lives in the still-charged topic allocation, not the lease
    // whose final Arc drop can still be running when strong_count reaches zero.
    pub(crate) retired_receivers: AtomicU64,
}
impl TopicState {
    pub(crate) fn new() -> Self {
        Self {
            demand: Demand::new(),
            attempt_sequence: AtomicU64::new(0),
            ingress_drops: AtomicU64::new(0),
            publisher_bound: AtomicBool::new(false),
            retired_receivers: AtomicU64::new(0),
        }
    }
}

pub(crate) struct SubscriptionLease {
    pub(crate) state: Arc<TopicState>,
    pub(crate) counted: bool,
    pub(crate) sender_slot: usize,
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
    receiver: Option<Box<dyn Receiver>>,
    marker: PhantomData<fn() -> P>,
}

pub(crate) trait Receiver: Any + Send {
    fn retirement(&self) -> (Arc<TopicState>, usize);
    fn state(&self) -> &TopicState;
}

impl<P: Shape> Drop for Subscriber<P> {
    fn drop(&mut self) {
        if let Some(receiver) = self.receiver.take() {
            let (state, slot) = receiver.retirement();
            // Field Drop releases transport handles, loan and lease; the
            // receiver box is also budgeted. Signal completion after its free.
            drop(receiver);
            state
                .retired_receivers
                .fetch_or(1 << slot, Ordering::Release);
        }
    }
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

impl<S: Clone + Send + 'static> Receiver for LatestReceiver<S> {
    fn retirement(&self) -> (Arc<TopicState>, usize) {
        (Arc::clone(&self.lease.state), self.lease.sender_slot)
    }
    fn state(&self) -> &TopicState {
        &self.lease.state
    }
}

pub(crate) struct StreamReceiver<S> {
    pub(crate) reader: PoolReader<S>,
    // Always resident: swapping a dequeued owner with this loan returns the old
    // owner to free without allocating. A read guard borrows only this slot.
    pub(crate) loan: S,
    pub(crate) lease: Arc<SubscriptionLease>,
    pub(crate) fence: u64,
    pub(crate) generation: u64,
}
impl<S: Send + 'static> Receiver for StreamReceiver<S> {
    fn retirement(&self) -> (Arc<TopicState>, usize) {
        (Arc::clone(&self.lease.state), self.lease.sender_slot)
    }
    fn state(&self) -> &TopicState {
        &self.lease.state
    }
}

impl<P: Shape> Subscriber<P> {
    pub(crate) fn new(receiver: Box<dyn Receiver>) -> Self {
        Self {
            receiver: Some(receiver),
            marker: PhantomData,
        }
    }
    pub fn status(&self) -> TopicStatus {
        let state = self
            .receiver
            .as_ref()
            .expect("live subscriber receiver")
            .state();
        TopicStatus {
            lifecycle: if state.demand.is_closed() {
                TopicLifecycle::Closed
            } else {
                TopicLifecycle::Registered
            },
            publisher: if state.publisher_bound.load(Ordering::Acquire) {
                PublisherStatus::Bound
            } else {
                PublisherStatus::NoPublisher
            },
        }
    }
    fn latest<S: Clone + Send + 'static>(
        &mut self,
        metadata: impl Fn(&S) -> PublicationMeta,
    ) -> Option<&S> {
        self.latest_checked(metadata, None)
            .expect("no destination limit")
    }
    fn latest_checked<S: Clone + Send + 'static>(
        &mut self,
        metadata: impl Fn(&S) -> PublicationMeta,
        capacity: Option<usize>,
    ) -> Result<Option<&S>, ReadError> {
        let receiver: &mut dyn Any = self
            .receiver
            .as_mut()
            .expect("live subscriber receiver")
            .as_mut();
        let receiver = receiver
            .downcast_mut::<LatestReceiver<S>>()
            .expect("sealed subscriber shape");
        let demand = receiver.lease.state.demand.snapshot();
        if !demand.active {
            return Ok(None);
        }
        let output = &mut receiver.output;
        output.update();
        let slot = output.output_buffer();
        let meta = metadata(slot);
        if meta.sequence == 0
            || meta.sequence == receiver.last_sequence
            || meta.activation_generation != demand.generation
        {
            return Ok(None);
        }
        check_destination(meta, capacity)?;
        receiver.last_sequence = meta.sequence;
        Ok(Some(slot))
    }
    fn stream_next<S: Payload>(
        &mut self,
        capacity: Option<usize>,
    ) -> Result<Option<&S>, ReadError> {
        let receiver: &mut dyn Any = self
            .receiver
            .as_mut()
            .expect("live subscriber receiver")
            .as_mut();
        let receiver = receiver
            .downcast_mut::<StreamReceiver<S>>()
            .expect("sealed subscriber shape");
        let demand = receiver.lease.state.demand.snapshot();
        if !demand.active {
            return Ok(None);
        }
        let Ok(slot) = receiver.reader.ready.peek() else {
            return Ok(None);
        };
        let meta = slot.meta();
        if meta.sequence <= receiver.fence
            || meta.activation_generation != receiver.generation
            || meta.activation_generation != demand.generation
        {
            // One cleanup step only; do not scan a configured depth in a read.
            let stale = receiver.reader.ready.pop().expect("peeked ready slot");
            receiver.reader.recycle(stale);
            return Ok(None);
        }
        check_destination(meta, capacity)?;
        let slot = receiver.reader.ready.pop().expect("peeked ready slot");
        let previous = std::mem::replace(&mut receiver.loan, slot);
        receiver.reader.recycle(previous);
        Ok(Some(&receiver.loan))
    }
}

fn check_destination(meta: PublicationMeta, capacity: Option<usize>) -> Result<(), ReadError> {
    if capacity.is_some_and(|capacity| capacity < meta.len) {
        return Err(ReadError::DestinationTooSmall { required: meta.len });
    }
    Ok(())
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

impl<T: Copy + Send + Sync + 'static> Subscriber<StreamValue<T>> {
    /// Consumes the oldest queued message. Capacity is independent per reader.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<ValueMessage<T>> {
        self.stream_next::<ValueSlot<T>>(None)
            .expect("no destination limit")
            .map(|slot| ValueMessage {
                meta: slot.meta,
                value: slot.value,
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockMessage<H> {
    pub meta: PublicationMeta,
    pub header: H,
    pub len: usize,
}

fn copy_block<H: Copy, E: Copy>(slot: &BlockSlot<H, E>, output: &mut [E]) -> BlockMessage<H> {
    output[..slot.meta.len].copy_from_slice(&slot.elements[..slot.meta.len]);
    BlockMessage {
        meta: slot.meta,
        header: slot.header,
        len: slot.meta.len,
    }
}

/// Borrows only this receiver's output slot, leaving every other receiver free.
pub struct BlockReadGuard<'a, H, E> {
    slot: &'a BlockSlot<H, E>,
}
impl<H, E> Drop for BlockReadGuard<'_, H, E> {
    fn drop(&mut self) {
        // Ending the exclusive Subscriber borrow releases read permission. The
        // loan remains initialized and budgeted until its next swap or cleanup.
    }
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
    /// Too-small destinations leave the publication available for another read.
    pub fn copy_latest(&mut self, output: &mut [E]) -> Result<Option<BlockMessage<H>>, ReadError> {
        self.latest_checked::<BlockSlot<H, E>>(|slot| slot.meta, Some(output.len()))
            .map(|slot| slot.map(|slot| copy_block(slot, output)))
    }
}

impl<H: Copy + Send + Sync + 'static, E: Copy + Send + Sync + 'static>
    Subscriber<StreamBlock<H, E>>
{
    /// Borrows the extra receiver loan; queued depth remains fully available.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<BlockReadGuard<'_, H, E>> {
        self.stream_next::<BlockSlot<H, E>>(None)
            .expect("no destination limit")
            .map(|slot| BlockReadGuard { slot })
    }
    /// Checks capacity before removing the FIFO head or changing the loan.
    pub fn copy_next(&mut self, output: &mut [E]) -> Result<Option<BlockMessage<H>>, ReadError> {
        self.stream_next::<BlockSlot<H, E>>(Some(output.len()))
            .map(|slot| slot.map(|slot| copy_block(slot, output)))
    }
}
