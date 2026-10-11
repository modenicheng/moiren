use crate::{
    BlockInit, Bus, BusConfig, PublishOutcome, PumpBudget, SubscriptionOptions,
    storage::{BlockSlot, ValueSlot},
};

fn budget(checks: usize, deliveries: usize, bytes: usize) -> PumpBudget {
    PumpBudget {
        topic_checks: checks,
        deliveries,
        copy_bytes: bytes,
    }
}

#[test]
fn settled_pump_reports_no_pending_work_in_the_same_call() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_latest_value("settle", 0_u32).unwrap();
    let mut reader = bus.subscribe(&topic, Default::default()).unwrap();
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(1)
        .unwrap();
    let report = bus.pump(budget(20, 20, usize::MAX));
    assert!(
        !report.has_pending,
        "all observed work finished within this call"
    );
    assert_eq!(report.deliveries, 2);
    assert_eq!(reader.read_latest().unwrap().value, 1);
}

fn pump_all(bus: &mut Bus) {
    for _ in 0..1000 {
        if !bus.pump(budget(128, 128, usize::MAX)).has_pending {
            return;
        }
    }
    panic!("pump did not settle");
}

#[test]
fn latest_fanout_charges_source_and_each_copy_and_preserves_snapshot() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_latest_value("latest", 0_u32).unwrap();
    let mut first = bus.subscribe(&topic, Default::default()).unwrap();
    let mut second = bus.subscribe(&topic, Default::default()).unwrap();
    let mut third = bus.subscribe(&topic, Default::default()).unwrap();
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(1)
        .unwrap();
    let bytes = std::mem::size_of::<ValueSlot<u32>>();
    let report = bus.pump(budget(2, 2, bytes * 2));
    assert_eq!(
        (report.topic_checks, report.deliveries, report.copy_bytes),
        (2, 2, bytes * 2)
    );
    assert!(report.has_pending);
    assert_eq!(first.read_latest().unwrap().value, 1);
    assert_eq!(second.read_latest(), None);
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(2)
        .unwrap();
    let report = bus.pump(budget(1, 1, bytes));
    assert_eq!(report.copy_bytes, bytes);
    assert_eq!(second.read_latest().unwrap().value, 1);
    let report = bus.pump(budget(1, 1, bytes));
    assert_eq!(report.copy_bytes, bytes);
    assert_eq!(third.read_latest().unwrap().value, 1);
    pump_all(&mut bus);
    assert_eq!(first.read_latest().unwrap().value, 2);
    assert_eq!(second.read_latest().unwrap().value, 2);
    assert_eq!(third.read_latest().unwrap().value, 2);
    assert_eq!(topic.stats().attempts, 2);
    assert_eq!(topic.stats().ingress_drops, 0);
}

#[test]
fn oversized_message_retries_without_starving_a_small_topic() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 2,
        ..Default::default()
    })
    .unwrap();
    let large = bus
        .register_latest_block(
            "large",
            BlockInit {
                header: 0_u32,
                element: 0_u64,
                capacity: 20,
            },
        )
        .unwrap();
    let small = bus.register_stream_value("small", 0_u32, 2).unwrap();
    let mut large_reader = bus.subscribe(&large, Default::default()).unwrap();
    let mut small_reader = bus.subscribe(&small, Default::default()).unwrap();
    let storage = bus
        .latest_storage_mut::<_, BlockSlot<u32, u64>>(&large)
        .unwrap();
    let mut endpoint = storage.endpoint();
    let mut write = endpoint.try_write(20).unwrap();
    *write.header_mut() = 7;
    write.elements_mut().fill(9);
    write.commit(20).unwrap();
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&small)
        .unwrap()
        .endpoint()
        .publish(5)
        .unwrap();
    let small_bytes = std::mem::size_of::<ValueSlot<u32>>();
    let report = bus.pump(budget(12, 12, small_bytes * 2));
    assert!(report.has_pending);
    assert_eq!(
        report.required_copy_bytes,
        crate::budget::block_copy::<u32, u64>(20).unwrap().size()
    );
    assert!(report.copy_bytes <= small_bytes * 2);
    assert_eq!(small_reader.next().unwrap().value, 5);
    assert!(large_reader.read_latest().is_none());
    pump_all(&mut bus);
    let read = large_reader.read_latest().unwrap();
    assert_eq!(*read.header(), 7);
    assert_eq!(read.elements(), [9; 20]);
}

#[test]
fn newer_late_seed_never_regresses_during_old_fanout_or_slot_reuse() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_latest_value("latest", 0_u32).unwrap();
    let mut first = bus.subscribe(&topic, Default::default()).unwrap();
    let dropped = bus.subscribe(&topic, Default::default()).unwrap();
    let mut last = bus.subscribe(&topic, Default::default()).unwrap();
    let bytes = std::mem::size_of::<ValueSlot<u32>>();
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(1)
        .unwrap();
    bus.pump(budget(2, 2, bytes * 2));
    assert_eq!(first.read_latest().unwrap().value, 1);
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(2)
        .unwrap();
    drop(dropped);
    let mut late = bus.subscribe(&topic, Default::default()).unwrap();
    assert_eq!(late.read_latest().unwrap().value, 2);
    bus.pump(budget(2, 2, bytes * 2));
    assert_eq!(last.read_latest().unwrap().value, 1);
    assert_eq!(late.read_latest(), None);
    pump_all(&mut bus);
    assert_eq!(first.read_latest().unwrap().value, 2);
    assert_eq!(last.read_latest().unwrap().value, 2);
    assert_eq!(late.read_latest(), None);
    assert_eq!(late.stats().delivered, 1);
}

#[test]
fn stream_cleanup_and_fenced_slots_are_paid_and_bounded_at_large_depth() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_stream_value("stream", (), 1000).unwrap();
    let first = bus.subscribe(&topic, Default::default()).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<()>>(&topic).unwrap();
    for _ in 0..1000 {
        storage.endpoint().publish(()).unwrap();
    }
    drop(first);
    let mut late = bus.subscribe(&topic, Default::default()).unwrap();
    let report = bus.pump(budget(1, 1, 0));
    assert_eq!(
        (report.topic_checks, report.deliveries, report.copy_bytes),
        (1, 1, 0)
    );
    assert!(report.has_pending);
    assert_eq!(
        bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
            .unwrap()
            .reader
            .ready
            .slots(),
        999
    );
    assert_eq!(late.next(), None);
    pump_all(&mut bus);
    bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
        .unwrap()
        .endpoint()
        .publish(())
        .unwrap();
    pump_all(&mut bus);
    assert_eq!(late.next().unwrap().meta.sequence, 1001);
}

#[test]
fn round_robin_stream_loss_is_local_and_last_full_is_observable() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 2,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_stream_value("stream", 0_u32, 2).unwrap();
    let other = bus.register_latest_value("other", 0_u32).unwrap();
    let mut slow = bus
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(1),
            },
        )
        .unwrap();
    let mut fast = bus.subscribe(&topic, Default::default()).unwrap();
    let mut observer = bus.subscribe(&other, Default::default()).unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
    for value in [1, 2] {
        storage.endpoint().publish(value).unwrap();
    }
    assert_eq!(
        storage.endpoint().publish(3),
        Ok(PublishOutcome::DroppedFull { sequence: 3 })
    );
    assert_eq!(topic.stats().ingress_drops, 1);
    assert_eq!(topic.stats().attempts, 3);
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&other)
        .unwrap()
        .endpoint()
        .publish(7)
        .unwrap();
    let bytes = std::mem::size_of::<ValueSlot<u32>>();
    for _ in 0..4 {
        bus.pump(budget(1, 1, bytes));
    }
    assert_eq!(observer.read_latest().unwrap().value, 7);
    pump_all(&mut bus);
    assert_eq!(slow.stats().delivered, 1);
    assert_eq!(slow.stats().overflow_drops, 1);
    assert_eq!(fast.stats().delivered, 2);
    assert_eq!(fast.stats().overflow_drops, 0);
    assert_eq!(slow.next().unwrap().value, 1);
    assert_eq!(fast.next().unwrap().value, 1);
    assert_eq!(fast.next().unwrap().value, 2);
}

#[test]
fn stream_snapshot_fence_and_replacement_are_each_one_paid_operation() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_stream_value("fences", (), 2).unwrap();
    let mut first = bus.subscribe(&topic, Default::default()).unwrap();
    bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
        .unwrap()
        .endpoint()
        .publish(())
        .unwrap();
    let mut fenced: Vec<_> = (0..63)
        .map(|_| bus.subscribe(&topic, Default::default()).unwrap())
        .collect();
    let bytes = std::mem::size_of::<ValueSlot<()>>();
    bus.pump(budget(2, 2, bytes * 2));
    assert_eq!(first.next().unwrap().meta.sequence, 1);
    let report = bus.pump(budget(1, 1, 0));
    assert_eq!(
        (report.topic_checks, report.deliveries, report.copy_bytes),
        (1, 1, 0)
    );
    assert!(report.has_pending);
    assert_eq!(
        bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
            .unwrap()
            .fanout
            .recipients
            .count_ones(),
        62
    );
    let dropped = fenced.remove(1);
    drop(dropped);
    let mut replacement = bus.subscribe(&topic, Default::default()).unwrap();
    assert_eq!(
        bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
            .unwrap()
            .fanout
            .recipients
            .count_ones(),
        61
    );
    pump_all(&mut bus);
    assert_eq!(replacement.next(), None);
    assert!(
        fenced
            .iter_mut()
            .all(|subscriber| subscriber.next().is_none())
    );
    bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
        .unwrap()
        .endpoint()
        .publish(())
        .unwrap();
    pump_all(&mut bus);
    assert_eq!(replacement.next().unwrap().meta.sequence, 2);
    assert!(
        fenced
            .iter_mut()
            .all(|subscriber| subscriber.next().unwrap().meta.sequence == 2)
    );
}

#[test]
fn stream_required_bytes_retain_ingress_and_source_staging_releases_rt_pool() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 0_u32,
                element: 0_u64,
                capacity: 20,
            },
            1,
        )
        .unwrap();
    let mut first = bus.subscribe(&topic, Default::default()).unwrap();
    let mut second = bus.subscribe(&topic, Default::default()).unwrap();
    bus.stream_storage_mut::<_, BlockSlot<u32, u64>>(&topic)
        .unwrap()
        .endpoint()
        .publish_slice(7, &[8, 9])
        .unwrap();
    let bytes = crate::budget::block_copy::<u32, u64>(2).unwrap().size();
    let report = bus.pump(budget(1, 1, bytes - 1));
    assert_eq!(report.copy_bytes, 0);
    assert_eq!(report.deliveries, 0);
    assert_eq!(report.required_copy_bytes, bytes);
    assert_eq!(
        bus.stream_storage_mut::<_, BlockSlot<u32, u64>>(&topic)
            .unwrap()
            .reader
            .ready
            .slots(),
        1
    );
    let report = bus.pump(budget(1, 1, bytes));
    assert_eq!(report.copy_bytes, bytes);
    bus.stream_storage_mut::<_, BlockSlot<u32, u64>>(&topic)
        .unwrap()
        .endpoint()
        .publish_slice(10, &[11])
        .unwrap();
    assert!(first.next().is_none());
    bus.pump(budget(1, 1, bytes));
    let held = first.next().unwrap();
    assert_eq!((*held.header(), held.elements()), (7, &[8, 9][..]));
    bus.pump(budget(1, 1, bytes));
    assert_eq!(second.next().unwrap().elements(), [8, 9]);
    pump_all(&mut bus);
    assert_eq!(second.next().unwrap().elements(), [11]);
    assert_eq!(held.elements(), [8, 9]);
    drop(held);
    assert_eq!(first.next().unwrap().elements(), [11]);
}

#[test]
fn actual_unregister_preserves_old_guards_and_separates_closed_from_empty() {
    for stream in [false, true] {
        let mut bus = Bus::new(BusConfig {
            max_topics: 1,
            ..Default::default()
        })
        .unwrap();
        let initial = BlockInit {
            header: 0_u32,
            element: 0_u32,
            capacity: 3,
        };
        if stream {
            let topic = bus.register_stream_block("guard", initial, 2).unwrap();
            let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
            bus.stream_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
                .unwrap()
                .endpoint()
                .publish_slice(7, &[8, 9])
                .unwrap();
            pump_all(&mut bus);
            let held = subscriber.next().unwrap();
            let before = bus.budget_used();
            bus.unregister(&topic).unwrap();
            assert!(bus.budget_used() < before);
            assert_eq!((*held.header(), held.elements()), (7, &[8, 9][..]));
            let new = bus.register_stream_block("guard", initial, 2).unwrap();
            assert_ne!(topic.id(), new.id());
            assert_eq!(held.meta().sequence, 1);
            drop(held);
            assert!(subscriber.next().is_none());
            assert_eq!(subscriber.status().lifecycle, crate::TopicLifecycle::Closed);
        } else {
            let topic = bus.register_latest_block("guard", initial).unwrap();
            let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
            let storage = bus
                .latest_storage_mut::<_, BlockSlot<u32, u32>>(&topic)
                .unwrap();
            let mut endpoint = storage.endpoint();
            let mut write = endpoint.try_write(2).unwrap();
            *write.header_mut() = 7;
            write.elements_mut().copy_from_slice(&[8, 9]);
            write.commit(2).unwrap();
            pump_all(&mut bus);
            let held = subscriber.read_latest().unwrap();
            bus.unregister(&topic).unwrap();
            assert_eq!((*held.header(), held.elements()), (7, &[8, 9][..]));
            let new = bus.register_latest_block("guard", initial).unwrap();
            assert_ne!(topic.id(), new.id());
            drop(held);
            assert!(subscriber.read_latest().is_none());
            assert_eq!(subscriber.status().lifecycle, crate::TopicLifecycle::Closed);
        }
    }
}

#[test]
fn no_publisher_preserves_snapshots_fifo_and_sequence_continuity() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 2,
        ..Default::default()
    })
    .unwrap();
    let latest = bus.register_latest_value("latest", 0_u32).unwrap();
    let stream = bus.register_stream_value("stream", 0_u32, 2).unwrap();
    let mut latest_reader = bus.subscribe(&latest, Default::default()).unwrap();
    let mut stream_reader = bus.subscribe(&stream, Default::default()).unwrap();
    latest
        .state
        .publisher_bound
        .store(true, std::sync::atomic::Ordering::Release);
    stream
        .state
        .publisher_bound
        .store(true, std::sync::atomic::Ordering::Release);
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&latest)
        .unwrap()
        .endpoint()
        .publish(7)
        .unwrap();
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&stream)
        .unwrap()
        .endpoint()
        .publish(8)
        .unwrap();
    latest
        .state
        .publisher_bound
        .store(false, std::sync::atomic::Ordering::Release);
    stream
        .state
        .publisher_bound
        .store(false, std::sync::atomic::Ordering::Release);
    pump_all(&mut bus);
    assert_eq!(
        latest_reader.status().publisher,
        crate::PublisherStatus::NoPublisher
    );
    assert_eq!(
        stream_reader.status().publisher,
        crate::PublisherStatus::NoPublisher
    );
    assert_eq!(latest_reader.read_latest().unwrap().value, 7);
    assert_eq!(stream_reader.next().unwrap().value, 8);
    let mut late = bus.subscribe(&latest, Default::default()).unwrap();
    assert_eq!(late.read_latest().unwrap().value, 7);
    latest
        .state
        .publisher_bound
        .store(true, std::sync::atomic::Ordering::Release);
    stream
        .state
        .publisher_bound
        .store(true, std::sync::atomic::Ordering::Release);
    bus.latest_storage_mut::<_, ValueSlot<u32>>(&latest)
        .unwrap()
        .endpoint()
        .publish(9)
        .unwrap();
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&stream)
        .unwrap()
        .endpoint()
        .publish(10)
        .unwrap();
    pump_all(&mut bus);
    assert_eq!(latest_reader.read_latest().unwrap().meta.sequence, 2);
    assert_eq!(stream_reader.next().unwrap().meta.sequence, 2);
}

#[test]
fn subscriber_and_ingress_statistics_saturate_independently() {
    use std::sync::atomic::Ordering;
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_stream_value("stats", 0_u32, 1).unwrap();
    let mut reader = bus
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(1),
            },
        )
        .unwrap();
    let storage = bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic).unwrap();
    storage
        .state
        .ingress_drops
        .store(u64::MAX, Ordering::Relaxed);
    let lease = storage.senders[0]
        .as_ref()
        .unwrap()
        .lease
        .upgrade()
        .unwrap();
    lease.delivered.store(u64::MAX, Ordering::Relaxed);
    lease.overflow_drops.store(u64::MAX, Ordering::Relaxed);
    storage.endpoint().publish(1).unwrap();
    storage.endpoint().publish(2).unwrap();
    pump_all(&mut bus);
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(3)
        .unwrap();
    pump_all(&mut bus);
    assert_eq!(
        topic.stats(),
        crate::TopicStats {
            attempts: 3,
            ingress_drops: u64::MAX
        }
    );
    assert_eq!(
        reader.stats(),
        crate::SubscriberStats {
            delivered: u64::MAX,
            overflow_drops: u64::MAX
        }
    );
    assert_eq!(reader.next().unwrap().value, 1);
}

#[test]
fn budget_failure_adds_no_demand_and_no_work_budget_changes_no_cursor() {
    let mut probe = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    probe.register_stream_value("tight", (), 2).unwrap();
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        byte_budget: probe.budget_used(),
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_stream_value("tight", (), 2).unwrap();
    let before = bus.budget_used();
    assert!(matches!(
        bus.subscribe(&topic, Default::default()),
        Err(crate::SubscribeError::BudgetExceeded)
    ));
    assert!(!bus.has_business_demand());
    assert_eq!(bus.budget_used(), before);
    assert_eq!(
        bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
            .unwrap()
            .endpoint()
            .publish(()),
        Ok(PublishOutcome::NoDemand)
    );
    assert_eq!(topic.stats().attempts, 0);
}

#[test]
fn large_retired_pool_does_not_delay_other_recipients_of_the_same_fanout() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus
        .register_stream_value("retirement-fairness", (), 2)
        .unwrap();
    let dropped = bus
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(1000),
            },
        )
        .unwrap();
    let mut live = bus.subscribe(&topic, Default::default()).unwrap();
    bus.stream_storage_mut::<_, ValueSlot<()>>(&topic)
        .unwrap()
        .endpoint()
        .publish(())
        .unwrap();
    let bytes = std::mem::size_of::<ValueSlot<()>>();
    bus.pump(budget(1, 1, bytes));
    drop(dropped);
    for _ in 0..4 {
        bus.pump(budget(1, 1, bytes));
    }
    assert_eq!(live.next().unwrap().meta.sequence, 1);
    assert_eq!(live.stats().delivered, 1);
}

#[test]
fn receiver_draining_after_full_observation_cannot_cause_an_uncharged_copy() {
    use std::sync::{Arc, Barrier};
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_stream_value("copy-race", 0_u32, 2).unwrap();
    let mut reader = bus
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(1),
            },
        )
        .unwrap();
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(1)
        .unwrap();
    pump_all(&mut bus);
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .endpoint()
        .publish(2)
        .unwrap();
    let bytes = std::mem::size_of::<ValueSlot<u32>>();
    bus.pump(budget(1, 1, bytes));
    let gate = Arc::new(super::DeliveryGate {
        checked: Barrier::new(2),
        resume: Barrier::new(2),
    });
    bus.stream_storage_mut::<_, ValueSlot<u32>>(&topic)
        .unwrap()
        .fanout
        .after_full_check = Some(Arc::clone(&gate));
    let mut reader = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            gate.checked.wait();
            assert_eq!(reader.next().unwrap().value, 1);
            gate.resume.wait();
            reader
        });
        let report = bus.pump(budget(1, 1, 0));
        assert_eq!((report.deliveries, report.copy_bytes), (1, 0));
        worker.join().unwrap()
    });
    assert_eq!(
        reader.stats().delivered,
        1,
        "zero copy budget cannot deliver into a concurrently emptied queue"
    );
    assert_eq!(reader.stats().overflow_drops, 1);
    assert_eq!(reader.next(), None);
}
