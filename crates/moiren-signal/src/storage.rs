//! Concrete prepared storage, never zero-initialized arbitrary Copy values.
//!
//! Templates are immutable and separate from transport slots. Future subscribers
//! must clone this template rather than copying any concurrently modified slot.

use crate::PublicationMeta;

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

// These owned fields are consumed by the later transport/dispatcher tasks.
#[allow(dead_code)]
pub(crate) struct LatestStorage<S: Clone + Send> {
    pub(crate) template: S,
    pub(crate) dispatch: S,
    pub(crate) input: triple_buffer::Input<S>,
    pub(crate) output: triple_buffer::Output<S>,
}

impl<S: Clone + Send> LatestStorage<S> {
    pub(crate) fn new(template: S) -> Self {
        let (input, output) = triple_buffer::TripleBuffer::new(&template).split();
        Self {
            dispatch: template.clone(),
            template,
            input,
            output,
        }
    }
}

#[allow(dead_code)]
pub(crate) struct StreamStorage<S: Clone + Send> {
    pub(crate) template: S,
    pub(crate) dispatch: S,
    pub(crate) slots: Box<[S]>,
    // SPSC roles: only the dispatcher returns slots to free_producer;
    // only RT consumes free slots and pushes completed ready indices.
    pub(crate) free_producer: rtrb::Producer<usize>,
    pub(crate) free_consumer: rtrb::Consumer<usize>,
    pub(crate) ready_producer: rtrb::Producer<usize>,
    pub(crate) ready_consumer: rtrb::Consumer<usize>,
    // RT restores this exclusive spare after abort/Full, never pushing to the
    // dispatcher's free_producer. Together, the pool owns depth+1 slots.
    pub(crate) writer_spare: usize,
}

impl<S: Clone + Send> StreamStorage<S> {
    pub(crate) fn new(template: S, depth: usize) -> Self {
        let mut slots = Vec::with_capacity(depth + 1);
        for _ in 0..=depth {
            slots.push(template.clone());
        }
        let slots = slots.into_boxed_slice();
        let (mut free_producer, free_consumer) = rtrb::RingBuffer::new(depth);
        let (ready_producer, ready_consumer) = rtrb::RingBuffer::new(depth);
        for index in 1..=depth {
            free_producer
                .push(index)
                .expect("checked free queue capacity");
        }
        Self {
            dispatch: template.clone(),
            template,
            slots,
            free_producer,
            free_consumer,
            ready_producer,
            ready_consumer,
            writer_spare: 0,
        }
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
        assert!(storage.ready_consumer.pop().is_err());
        assert_eq!(storage.writer_spare, 0);
        assert_eq!(storage.slots.len(), 9);
        for slot in storage
            .slots
            .iter()
            .chain([&storage.template, &storage.dispatch])
        {
            assert_eq!(slot.meta, PublicationMeta::INITIAL);
            assert_eq!(slot.header.get(), 5);
            assert!(slot.elements.iter().all(|element| element.get() == 9));
        }
        for index in 1..=8 {
            assert_eq!(storage.free_consumer.pop().unwrap(), index);
        }
        assert!(storage.free_consumer.pop().is_err());
    }
}
