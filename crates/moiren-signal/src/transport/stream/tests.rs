use crate::{
    BlockInit, Bus, PublishError, PublishOutcome, ReadError, SubscriptionOptions, WriteError,
    WriteUnavailable,
    storage::{BlockSlot, ValueSlot},
};
use std::sync::atomic::Ordering;

fn depth(depth: usize) -> SubscriptionOptions {
    SubscriptionOptions {
        stream_depth: Some(depth),
    }
}

#[test]
fn ingress_drops_new_preserves_fifo_and_allocates_one_sequence_per_full_attempt() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_stream_value("stream", 99_u32, 2).unwrap();
    let mut subscriber = bus.subscribe(&topic, depth(8)).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
    for (value, sequence) in [(10, 1), (20, 2)] {
        let PublishOutcome::Published(meta) = storage.endpoint().publish(value).unwrap() else {
            panic!("published")
        };
        assert_eq!(meta.sequence, sequence);
    }
    assert_eq!(
        storage.endpoint().publish(30),
        Ok(PublishOutcome::DroppedFull { sequence: 3 })
    );
    assert_eq!(storage.state.ingress_drops.load(Ordering::Acquire), 1);
    assert!(storage.deliver_next());
    assert!(storage.deliver_next());
    assert!(!storage.deliver_next());
    let first = subscriber.next().unwrap();
    let second = subscriber.next().unwrap();
    assert_eq!((first.value, second.value), (10, 20));
    assert_eq!((first.meta.sequence, second.meta.sequence), (1, 2));
    storage.endpoint().publish(40).unwrap();
    storage.deliver_next();
    let fourth = subscriber.next().unwrap();
    assert_eq!((fourth.value, fourth.meta.sequence), (40, 4));
}

#[test]
fn block_abort_invalid_commit_and_full_keep_slots_and_attempts_conserved() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 99_u32,
                element: 99_u32,
                capacity: 4,
            },
            2,
        )
        .unwrap();
    let mut subscriber = bus.subscribe(&topic, depth(4)).unwrap();
    let storage = bus
        .stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
        .unwrap();
    for _ in 0..100 {
        let mut endpoint = storage.endpoint();
        let mut abort = endpoint.try_write(2).unwrap();
        *abort.header_mut() = 0;
        abort.elements_mut().fill(0);
        drop(abort);
        assert_eq!(
            endpoint.try_write(1).unwrap().commit(2),
            Err(PublishError::InvalidLength)
        );
    }
    assert_eq!(storage.state.attempt_sequence.load(Ordering::Acquire), 0);
    for value in [10, 20] {
        storage
            .endpoint()
            .publish_slice(value, &[value, value + 1])
            .unwrap();
    }
    assert!(matches!(
        storage.endpoint().try_write(1),
        Err(WriteError::Unavailable(WriteUnavailable::Full {
            sequence: 3
        }))
    ));
    assert_eq!(
        storage.endpoint().publish_slice(30, &[30]),
        Ok(PublishOutcome::DroppedFull { sequence: 4 })
    );
    assert_eq!(storage.state.ingress_drops.load(Ordering::Acquire), 2);
    storage.deliver_next();
    storage.deliver_next();
    for (value, sequence) in [(10, 1), (20, 2)] {
        let read = subscriber.next().unwrap();
        assert_eq!(*read.header(), value);
        assert_eq!(read.elements(), [value, value + 1]);
        assert_eq!(read.meta().sequence, sequence);
    }
    storage.endpoint().publish_slice(40, &[]).unwrap();
    storage.deliver_next();
    let read = subscriber.next().unwrap();
    assert!(read.elements().is_empty());
    assert_eq!(read.meta().sequence, 5);
}

#[test]
fn held_loan_and_full_receiver_leave_other_depths_and_ingress_progressing() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 99_u32,
                element: 99_u32,
                capacity: 3,
            },
            2,
        )
        .unwrap();
    let mut slow = bus.subscribe(&topic, depth(1)).unwrap();
    let mut medium = bus.subscribe(&topic, depth(2)).unwrap();
    let mut fast = bus.subscribe(&topic, depth(4)).unwrap();
    let storage = bus
        .stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
        .unwrap();
    storage.endpoint().publish_slice(1, &[1, 2, 3]).unwrap();
    storage.deliver_next();
    let held = slow.next().unwrap();
    assert_eq!(medium.next().unwrap().elements(), [1, 2, 3]);
    assert_eq!(fast.next().unwrap().elements(), [1, 2, 3]);
    for value in 2..20 {
        storage.endpoint().publish_slice(value, &[value]).unwrap();
        storage.deliver_next();
        assert_eq!(*fast.next().unwrap().header(), value);
        assert_eq!(held.elements(), [1, 2, 3]);
    }
    assert_eq!(storage.senders[0].as_ref().unwrap().drops, 17);
    assert_eq!(storage.senders[1].as_ref().unwrap().drops, 16);
    assert_eq!(storage.senders[2].as_ref().unwrap().drops, 0);
    drop(held);
    assert_eq!(*slow.next().unwrap().header(), 2);
    assert!(slow.next().is_none());
    assert_eq!(*medium.next().unwrap().header(), 2);
    assert_eq!(*medium.next().unwrap().header(), 3);
    assert!(medium.next().is_none());
    assert_eq!(storage.state.ingress_drops.load(Ordering::Acquire), 0);
}

#[test]
fn copy_next_too_small_does_not_consume_header_elements_or_meta() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 99_u32,
                element: 99_u32,
                capacity: 4,
            },
            2,
        )
        .unwrap();
    let mut subscriber = bus.subscribe(&topic, depth(2)).unwrap();
    let storage = bus
        .stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
        .unwrap();
    storage.endpoint().publish_slice(7, &[8, 9, 10]).unwrap();
    storage.deliver_next();
    let mut small = [100; 2];
    for _ in 0..3 {
        assert_eq!(
            subscriber.copy_next(&mut small),
            Err(ReadError::DestinationTooSmall { required: 3 })
        );
        assert_eq!(small, [100; 2]);
    }
    let read = subscriber.next().unwrap();
    assert_eq!(
        (*read.header(), read.elements(), read.meta().sequence),
        (7, &[8, 9, 10][..], 1)
    );
    drop(read);
    assert!(subscriber.next().is_none());
    storage.endpoint().publish_slice(11, &[12, 13]).unwrap();
    storage.deliver_next();
    let mut output = [100; 4];
    let copied = subscriber.copy_next(&mut output).unwrap().unwrap();
    assert_eq!(
        (copied.header, copied.len, copied.meta.sequence),
        (11, 2, 2)
    );
    assert_eq!(output, [12, 13, 100, 100]);
    assert_eq!(subscriber.copy_next(&mut output), Ok(None));
}

#[test]
fn subscription_fence_filters_committed_ingress_even_before_dispatch() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_stream_value("stream", 99_u32, 2).unwrap();
    let mut existing = bus.subscribe(&topic, depth(4)).unwrap();
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(1)
        .unwrap();
    let mut late = bus.subscribe(&topic, depth(4)).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
    storage.deliver_next();
    assert_eq!(existing.next().unwrap().value, 1);
    assert_eq!(late.next(), None);
    storage.endpoint().publish(2).unwrap();
    storage.deliver_next();
    assert_eq!(existing.next().unwrap().value, 2);
    assert_eq!(late.next().unwrap().value, 2);
}

#[test]
fn cleanup_discards_one_stale_ingress_per_step_and_preserves_current_epoch() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_stream_value("stream", 99_u32, 3).unwrap();
    let first = bus.subscribe(&topic, depth(3)).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
    storage.endpoint().publish(1).unwrap();
    storage.endpoint().publish(2).unwrap();
    drop(first);
    let mut current = bus.subscribe(&topic, depth(3)).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
    storage.endpoint().publish(3).unwrap();
    assert_eq!(storage.begin_dispatch(), super::DispatchStep::Discarded);
    assert_eq!(storage.reader.ready.slots(), 2);
    assert_eq!(storage.begin_dispatch(), super::DispatchStep::Discarded);
    assert_eq!(storage.reader.ready.slots(), 1);
    assert_eq!(storage.begin_dispatch(), super::DispatchStep::Ready);
    for index in 0..64 {
        storage.deliver_to(index);
    }
    storage.finish_dispatch();
    assert_eq!(current.next().unwrap().value, 3);
    assert_eq!(storage.begin_dispatch(), super::DispatchStep::Empty);
}

#[test]
fn closed_loan_remains_readable_and_storage_stays_charged() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 99_u32,
                element: 99_u32,
                capacity: 4,
            },
            2,
        )
        .unwrap();
    let mut subscriber = bus.subscribe(&topic, depth(2)).unwrap();
    let storage = bus
        .stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
        .unwrap();
    storage.endpoint().publish_slice(7, &[8, 9]).unwrap();
    storage.deliver_next();
    let state = storage.state.clone();
    let before = bus.budget_used();
    let guard = subscriber.next().unwrap();
    state.demand.close();
    assert_eq!((*guard.header(), guard.elements()), (7, &[8, 9][..]));
    assert_eq!(bus.budget_used(), before);
    drop(guard);
    assert_eq!(subscriber.status().lifecycle, crate::TopicLifecycle::Closed);
    assert!(subscriber.next().is_none());
    drop(subscriber);
    assert_eq!(bus.budget_used(), before);
}

#[test]
fn barriers_order_subscription_fence_against_commit_allocation_before_enqueue() {
    use std::sync::{Arc, Barrier};
    for subscribe_before_commit in [false, true] {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus.register_stream_value("fence", 99_u32, 2).unwrap();
        let mut existing = bus.subscribe(&topic, depth(4)).unwrap();
        let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
        let state = storage.state.clone();
        let gate = Arc::new(crate::sequence::CommitGate::new());
        storage
            .cell
            .test_writer::<ValueSlot<u32>>()
            .sequence
            .pause_after_allocate(Arc::clone(&gate));
        let (control, mut host) = bus.create_host(1).unwrap();
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&topic, key).unwrap();
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        let op = update.op_bindings(key).unwrap();
        let handle = publisher.handle();
        drop(host.activate(update).unwrap());
        let start = Barrier::new(2);
        let (mut late, _host) = std::thread::scope(|scope| {
            let start_ref = &start;
            let op_ref = &op;
            let worker = scope.spawn(move || {
                start_ref.wait();
                host.begin_block();
                assert!(matches!(
                    host.scope(op_ref).publish(&handle, 7),
                    Ok(PublishOutcome::Published(_))
                ));
                host
            });
            let late = if subscribe_before_commit {
                let late = bus.subscribe(&topic, depth(4)).unwrap();
                start.wait();
                gate.allocated.wait();
                late
            } else {
                start.wait();
                gate.allocated.wait();
                // Sequence has been Release-stored; ready enqueue is blocked.
                assert_eq!(state.attempt_sequence.load(Ordering::Acquire), 1);
                bus.subscribe(&topic, depth(4)).unwrap()
            };
            gate.resume.wait();
            (late, worker.join().unwrap())
        });
        let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
        assert!(storage.deliver_next());
        assert_eq!(existing.next().unwrap().value, 7);
        assert_eq!(
            late.next().map(|message| message.value),
            subscribe_before_commit.then_some(7)
        );
    }
}

#[test]
fn stream_publication_full_abort_and_validation_have_no_allocator_activity() {
    use crate::test_alloc as allocator;
    let mut bus = Bus::new(Default::default()).unwrap();
    let value = bus.register_stream_value("value", 99_u32, 2).unwrap();
    let mut subscriber = bus.subscribe(&value, depth(2)).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&value).unwrap();
    for _ in 0..100 {
        assert_eq!(
            allocator::measure(|| {
                let mut endpoint = storage.endpoint();
                assert!(matches!(
                    endpoint.publish(1),
                    Ok(PublishOutcome::Published(_))
                ));
                assert!(matches!(
                    endpoint.publish(2),
                    Ok(PublishOutcome::Published(_))
                ));
                assert!(matches!(
                    endpoint.publish(3),
                    Ok(PublishOutcome::DroppedFull { .. })
                ));
            }),
            allocator::Counts::default()
        );
        storage.deliver_next();
        storage.deliver_next();
        subscriber.next();
        subscriber.next();
    }
    let topic = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 99_u32,
                element: 99_u32,
                capacity: 4,
            },
            2,
        )
        .unwrap();
    let mut subscriber = bus.subscribe(&topic, depth(2)).unwrap();
    let storage = bus
        .stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
        .unwrap();
    for _ in 0..100 {
        assert_eq!(
            allocator::measure(|| {
                let mut endpoint = storage.endpoint();
                let mut abort = endpoint.try_write(4).unwrap();
                abort.elements_mut().fill(0);
                drop(abort);
                assert_eq!(
                    endpoint.try_write(1).unwrap().commit(2),
                    Err(PublishError::InvalidLength)
                );
                assert!(matches!(
                    endpoint.try_write(5),
                    Err(WriteError::Invalid(PublishError::InvalidLength))
                ));
                endpoint.publish_slice(1, &[1, 2, 3, 4]).unwrap();
                endpoint.publish_slice(2, &[]).unwrap();
                assert!(matches!(
                    endpoint.try_write(1),
                    Err(WriteError::Unavailable(WriteUnavailable::Full { .. }))
                ));
                assert!(matches!(
                    endpoint.publish_slice(3, &[3]),
                    Ok(PublishOutcome::DroppedFull { .. })
                ));
            }),
            allocator::Counts::default()
        );
        storage.deliver_next();
        storage.deliver_next();
        subscriber.next();
        subscriber.next();
    }
}

#[test]
fn no_demand_closed_and_sequence_exhaustion_abort_without_payload_free() {
    use crate::test_alloc as allocator;
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 99_u32,
                element: 99_u32,
                capacity: 4,
            },
            2,
        )
        .unwrap();
    let storage = bus
        .stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
        .unwrap();
    assert_eq!(
        allocator::measure(|| {
            assert_eq!(
                storage.endpoint().publish_slice(1, &[1]),
                Ok(PublishOutcome::NoDemand)
            );
            assert!(matches!(
                storage.endpoint().try_write(1),
                Err(WriteError::Unavailable(WriteUnavailable::NoDemand))
            ));
        }),
        allocator::Counts::default()
    );
    assert_eq!(storage.state.attempt_sequence.load(Ordering::Acquire), 0);
    let _subscriber = bus.subscribe(&topic, depth(2)).unwrap();
    let storage = bus
        .stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
        .unwrap();
    storage.cell.test_writer::<BlockSlot<u32, u32>>().sequence =
        crate::sequence::Sequence::at(u64::MAX);
    assert_eq!(
        allocator::measure(|| {
            assert_eq!(
                storage.endpoint().publish_slice(1, &[1]),
                Err(PublishError::SequenceExhausted)
            );
        }),
        allocator::Counts::default()
    );
    storage.cell.test_writer::<BlockSlot<u32, u32>>().sequence = Default::default();
    let state = storage.state.clone();
    assert_eq!(
        allocator::measure(|| {
            let mut endpoint = storage.endpoint();
            let write = endpoint.try_write(2).unwrap();
            state.demand.close();
            assert_eq!(write.commit(2), Err(PublishError::Closed));
            assert_eq!(endpoint.publish_slice(1, &[1]), Err(PublishError::Closed));
        }),
        allocator::Counts::default()
    );
    assert_eq!(state.attempt_sequence.load(Ordering::Acquire), 0);
    assert_eq!(state.ingress_drops.load(Ordering::Acquire), 0);
}

#[test]
fn subscriber_status_tracks_binding_independently_of_data_and_close() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_stream_value("status", 99_u32, 2).unwrap();
    let mut subscriber = bus.subscribe(&topic, depth(1)).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
    storage.state.publisher_bound.store(true, Ordering::Release);
    assert_eq!(subscriber.status().publisher, crate::PublisherStatus::Bound);
    assert_eq!(subscriber.next(), None);
    storage.endpoint().publish(7).unwrap();
    storage.deliver_next();
    assert_eq!(subscriber.next().unwrap().value, 7);
    assert_eq!(subscriber.status().publisher, crate::PublisherStatus::Bound);
    storage
        .state
        .publisher_bound
        .store(false, Ordering::Release);
    assert_eq!(
        subscriber.status().publisher,
        crate::PublisherStatus::NoPublisher
    );
    storage.state.demand.close();
    assert_eq!(subscriber.status().lifecycle, crate::TopicLifecycle::Closed);
    assert_eq!(subscriber.next(), None);
}
