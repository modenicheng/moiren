//! Concrete prepared storage, never zero-initialized arbitrary Copy values.
//!
//! Templates are immutable and separate from transport slots. Future subscribers
//! must clone this template rather than copying any concurrently modified slot.

use crate::{
    PublicationMeta, budget::Charge, dispatcher::Fanout, lifecycle::StateHandle,
    sequence::Sequence, subscriber::SubscriptionLease,
};
use std::sync::{Arc, Mutex, Weak};

pub(crate) struct LatestSender<S: Clone + Send> {
    pub(crate) input: triple_buffer::Input<S>,
    pub(crate) lease: Weak<SubscriptionLease>,
    pub(crate) last_sequence: u64,
    pub(crate) _charge: Charge,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ValueSlot<T> {
    pub(crate) meta: PublicationMeta,
    pub(crate) value: T,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub(crate) struct BlockSlot<H, E> {
    pub(crate) meta: PublicationMeta,
    pub(crate) header: H,
    pub(crate) elements: Box<[E]>,
}

pub(crate) trait Payload: Clone + Send + 'static {
    fn meta(&self) -> PublicationMeta;
    fn set_meta(&mut self, meta: PublicationMeta);
    fn copy_from(&mut self, source: &Self);
    fn element_bytes(&self) -> usize;
    fn element_count(&self) -> usize;
    fn copy_bytes(&self) -> usize;
}
impl<T: Copy + Send + 'static> Payload for ValueSlot<T> {
    fn meta(&self) -> PublicationMeta {
        self.meta
    }
    fn set_meta(&mut self, meta: PublicationMeta) {
        self.meta = meta;
    }
    fn copy_from(&mut self, source: &Self) {
        *self = *source;
    }
    fn element_bytes(&self) -> usize {
        0
    }
    fn element_count(&self) -> usize {
        0
    }
    fn copy_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}
impl<H: Copy + Send + 'static, E: Copy + Send + 'static> Payload for BlockSlot<H, E> {
    fn meta(&self) -> PublicationMeta {
        self.meta
    }
    fn set_meta(&mut self, meta: PublicationMeta) {
        self.meta = meta;
    }
    fn copy_from(&mut self, source: &Self) {
        self.meta = source.meta;
        self.header = source.header;
        // Box ownership never changes: fan-out copies into prepared regions.
        self.elements[..source.meta.len].copy_from_slice(&source.elements[..source.meta.len]);
    }
    fn element_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.elements)
    }
    fn element_count(&self) -> usize {
        self.elements.len()
    }
    fn copy_bytes(&self) -> usize {
        crate::budget::block_copy::<H, E>(self.meta.len)
            .expect("validated payload layout")
            .size()
    }
}

// These owned fields are consumed by the later transport/dispatcher tasks.
#[allow(dead_code)]
pub(crate) struct LatestStorage<S: Clone + Send> {
    pub(crate) template: S,
    pub(crate) dispatch: S,
    pub(crate) input: triple_buffer::Input<S>,
    pub(crate) output: triple_buffer::Output<S>,
    pub(crate) state: StateHandle,
    pub(crate) sequence: Sequence,
    // Fixed business sender directory, allocated as part of the storage box.
    pub(crate) senders: [Option<LatestSender<S>>; 64],
    pub(crate) occupied: u64,
    pub(crate) fanout: Fanout,
}

impl<S: Clone + Send> LatestStorage<S> {
    pub(crate) fn new(template: S) -> Self {
        let (input, output) = triple_buffer::TripleBuffer::new(&template).split();
        Self {
            dispatch: template.clone(),
            template,
            input,
            output,
            state: StateHandle::new(),
            sequence: Sequence::default(),
            senders: std::array::from_fn(|_| None),
            occupied: 0,
            fanout: Fanout::default(),
        }
    }
}

#[allow(dead_code)]
pub(crate) struct StreamStorage<S: Clone + Send> {
    pub(crate) template: S,
    pub(crate) writer: PoolWriter<S>,
    pub(crate) reader: PoolReader<S>,
    pub(crate) dispatch: Option<S>,
    pub(crate) staging: Option<S>,
    pub(crate) state: StateHandle,
    pub(crate) sequence: Sequence,
    pub(crate) senders: [Option<StreamSender<S>>; 64],
    pub(crate) occupied: u64,
    pub(crate) fanout: Fanout,
}

impl<S: Clone + Send> StreamStorage<S> {
    pub(crate) fn new(template: S, depth: usize) -> Self {
        let (mut writer, reader) = pool(&template, depth);
        writer.spare = Some(template.clone());
        Self {
            staging: Some(template.clone()),
            template,
            writer,
            reader,
            dispatch: None,
            state: StateHandle::new(),
            sequence: Sequence::default(),
            senders: std::array::from_fn(|_| None),
            occupied: 0,
            fanout: Fanout::default(),
        }
    }
}

/// A slot has exactly one owner: free ring, writer spare/acquired guard, ready
/// ring, or reader loan. Dispatch copies into separate prepared staging and
/// immediately returns ingress ownership. No mutable pool aliases exist.
/// The ingress owns depth+1 slots; each receiver separately owns depth+1 slots.
pub(crate) struct PoolWriter<S> {
    pub(crate) free: rtrb::Consumer<S>,
    pub(crate) ready: rtrb::Producer<S>,
    pub(crate) spare: Option<S>,
}
pub(crate) struct PoolReader<S> {
    pub(crate) free: rtrb::Producer<S>,
    pub(crate) ready: rtrb::Consumer<S>,
}
#[allow(dead_code)]
pub(crate) struct StreamSender<S> {
    pub(crate) writer: PoolWriter<S>,
    pub(crate) lease: Weak<SubscriptionLease>,
    pub(crate) fence: u64,
    pub(crate) generation: u64,
    pub(crate) drops: u64,
    pub(crate) retirement: Arc<Mutex<Option<PoolReader<S>>>>,
    pub(crate) _charge: Charge,
}

impl<S> StreamSender<S> {
    /// Queue endpoints are handed back after the receiver box releases its
    /// loan, so each step frees one owner and final destructors see empty rings.
    pub(crate) fn retire_one(&mut self) -> bool {
        let mut retired = self.retirement.lock().expect("retirement handoff poisoned");
        let reader = retired.as_mut().expect("completed receiver handoff");
        if let Ok(slot) = reader.ready.pop() {
            drop(slot);
            return false;
        }
        if let Ok(slot) = self.writer.free.pop() {
            drop(slot);
            return false;
        }
        if let Some(slot) = self.writer.spare.take() {
            drop(slot);
            return false;
        }
        true
    }
}

/// Preparation only. The remaining slot is placed in the ingress writer spare
/// or the subscriber loan by the caller. The free ring needs depth+1 capacity:
/// a dispatcher can return every owner before RT acquires its next spare.
pub(crate) fn pool<S: Clone>(template: &S, depth: usize) -> (PoolWriter<S>, PoolReader<S>) {
    let (mut free, free_consumer) = rtrb::RingBuffer::new(depth + 1);
    let (ready, ready_consumer) = rtrb::RingBuffer::new(depth);
    for _ in 0..depth {
        assert!(
            free.push(template.clone()).is_ok(),
            "prepared free capacity"
        );
    }
    (
        PoolWriter {
            free: free_consumer,
            ready,
            spare: None,
        },
        PoolReader {
            free,
            ready: ready_consumer,
        },
    )
}

impl<S> PoolWriter<S> {
    pub(crate) fn acquire(&mut self) -> Option<S> {
        if self.ready.is_full() {
            return None;
        }
        self.spare.take().or_else(|| self.free.pop().ok())
    }
    pub(crate) fn restore(&mut self, slot: S) {
        // Abort never uses the other thread's producer and cannot fail into a
        // payload destructor on RT. Acquire emptied this exclusive local spare.
        debug_assert!(self.spare.is_none());
        self.spare = Some(slot);
    }
    pub(crate) fn enqueue(&mut self, slot: S) -> bool {
        match self.ready.push(slot) {
            Ok(()) => true,
            Err(rtrb::PushError::Full(slot)) => {
                self.restore(slot);
                false
            }
        }
    }
}
impl<S> PoolReader<S> {
    pub(crate) fn recycle(&mut self, slot: S) {
        // A reader can return only a slot removed from the ready ring or its
        // own loan. At most depth+1 owners exist, so this ring always has room.
        assert!(self.free.push(slot).is_ok(), "conserved free slot capacity");
    }
}

pub(crate) fn block_template<H, E>(
    capacity: usize,
    mut header: impl FnMut() -> H,
    mut element: impl FnMut() -> E,
) -> BlockSlot<H, E> {
    let header = header();
    let mut elements = Vec::with_capacity(capacity);
    for _ in 0..capacity {
        elements.push(element());
    }
    let elements = elements.into_boxed_slice();
    BlockSlot {
        meta: PublicationMeta::INITIAL,
        header,
        elements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    #[test]
    fn latest_valid_template_initializes_all_slots_without_a_message() {
        let mut next = 0;
        let template = block_template(
            4,
            || NonZeroU32::new(7).unwrap(),
            || {
                next += 1;
                NonZeroU32::new(next).unwrap()
            },
        );
        let mut storage = LatestStorage::new(template);
        for slot in [
            &storage.template,
            &storage.dispatch,
            storage.input.input_buffer_mut(),
            storage.output.output_buffer(),
        ] {
            assert_eq!(slot.meta, PublicationMeta::INITIAL);
            assert_eq!(slot.header.get(), 7);
            assert_eq!(
                slot.elements
                    .iter()
                    .map(|element| element.get())
                    .collect::<Vec<_>>(),
                [1, 2, 3, 4]
            );
        }
        // Exchange through all three initialized slots; there was no initial publish.
        assert!(!storage.output.update());
        for _ in 0..3 {
            storage.input.publish();
            assert!(storage.output.update());
            assert_eq!(storage.output.output_buffer().meta.sequence, 0);
            assert_eq!(storage.input.input_buffer_mut().elements[3].get(), 4);
        }
    }

    #[test]
    fn stream_slots_are_initialized_and_ready_queue_starts_empty() {
        let template = block_template(
            3,
            || NonZeroU32::new(5).unwrap(),
            || NonZeroU32::new(9).unwrap(),
        );
        let mut storage = StreamStorage::new(template, 8);
        assert!(storage.reader.ready.pop().is_err());
        assert_eq!(storage.writer.free.slots(), 8);
        for slot in [&storage.template, storage.writer.spare.as_ref().unwrap()] {
            assert_eq!(slot.meta, PublicationMeta::INITIAL);
            assert_eq!(slot.header.get(), 5);
            assert!(slot.elements.iter().all(|element| element.get() == 9));
        }
        for _ in 1..=8 {
            let slot = storage.writer.free.pop().unwrap();
            assert_eq!(slot.meta, PublicationMeta::INITIAL);
            assert_eq!(slot.header.get(), 5);
            assert!(slot.elements.iter().all(|element| element.get() == 9));
        }
        assert!(storage.writer.free.pop().is_err());
    }
}
