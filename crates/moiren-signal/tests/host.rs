use moiren_signal::{
    ActivateError, BlockInit, Bus, HostError, PublishError, PublishOutcome, PublisherStatus,
    PumpBudget, UnregisterError,
};

fn pump(bus: &mut Bus) {
    bus.pump(PumpBudget {
        topic_checks: 1024,
        deliveries: 256,
        copy_bytes: 65536,
    });
}

#[test]
fn typed_handle_requires_active_scope() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("status/value", 0.0_f32).unwrap();
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(4).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    assert_eq!(
        host.scope(&op).demand(&publisher.handle()),
        Err(PublishError::InactiveBinding)
    );
    assert_eq!(
        host.scope(&op).publish(&publisher.handle(), 1.0),
        Err(PublishError::InactiveBinding)
    );
    drop(host.activate(update).unwrap());
    host.begin_block();
    assert_eq!(
        host.scope(&op).demand(&publisher.handle()).unwrap(),
        moiren_signal::DemandSnapshot {
            active: true,
            generation: 1
        }
    );
    host.scope(&op).publish(&publisher.handle(), 0.75).unwrap();
    pump(&mut bus);
    assert_eq!(subscriber.read_latest().unwrap().value, 0.75);
    assert_eq!(topic.status().publisher, PublisherStatus::Bound);
}

#[test]
fn candidates_do_not_change_active_authority_and_tables_have_independent_identity() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("value", 0_u32).unwrap();
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(2).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let first = control.prepare_update(&[publisher.erase()]).unwrap();
    let old_op = first.op_bindings(key).unwrap();
    drop(host.activate(first).unwrap());
    host.begin_block();
    let first_meta = host.scope(&old_op).publish(&publisher.handle(), 1).unwrap();
    let second = control.prepare_update(&[publisher.erase()]).unwrap();
    let new_op = second.op_bindings(key).unwrap();
    let cancelled = control.prepare_update(&[publisher.erase()]).unwrap();
    drop(cancelled);
    assert_eq!(
        host.scope(&new_op).publish(&publisher.handle(), 9),
        Err(PublishError::InactiveBinding)
    );
    let retired = host.activate(second).unwrap();
    assert_eq!(
        host.scope(&old_op).publish(&publisher.handle(), 9),
        Err(PublishError::InactiveBinding)
    );
    let second_meta = host.scope(&new_op).publish(&publisher.handle(), 2).unwrap();
    let (PublishOutcome::Published(first_meta), PublishOutcome::Published(second_meta)) =
        (first_meta, second_meta)
    else {
        panic!("expected publications")
    };
    assert_eq!(second_meta.sequence, first_meta.sequence + 1);
    assert_eq!(
        second_meta.activation_generation,
        first_meta.activation_generation
    );
    drop(retired);
    pump(&mut bus);
    assert_eq!(subscriber.read_latest().unwrap().value, 2);
}

#[test]
fn replacement_handle_foreign_host_and_duplicate_topic_are_denied() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("value", 0_u32).unwrap();
    let _subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(2).unwrap();
    let (foreign, mut foreign_host) = bus.create_host(2).unwrap();
    let old_key = control.new_binding().unwrap();
    let old = control.prepare(&topic, old_key).unwrap();
    assert!(matches!(
        foreign.prepare(&topic, foreign.new_binding().unwrap()),
        Err(HostError::PublisherConflict)
    ));
    let update = control.prepare_update(&[old.erase()]).unwrap();
    let old_op = update.op_bindings(old_key).unwrap();
    drop(host.activate(update).unwrap());
    let key = control.new_binding().unwrap();
    let new = control.prepare(&topic, key).unwrap();
    assert!(matches!(
        control.prepare_update(&[old.erase(), new.erase()]),
        Err(HostError::PublisherConflict)
    ));
    let update = control.prepare_update(&[new.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    assert_eq!(
        host.scope(&old_op).publish(&new.handle(), 1),
        Err(PublishError::InactiveBinding)
    );
    assert_eq!(
        foreign_host.scope(&op).publish(&new.handle(), 1),
        Err(PublishError::ForeignHost)
    );
    assert_eq!(
        foreign_host.scope(&op).demand(&new.handle()),
        Err(PublishError::ForeignHost)
    );
    drop(host.activate(update).unwrap());
    assert_eq!(
        host.scope(&op).publish(&old.handle(), 1),
        Err(PublishError::InactiveBinding)
    );
}

#[test]
fn transaction_abort_and_stale_or_foreign_failure_keep_complete_update() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_stream_value("stream", 0_u32, 2).unwrap();
    let mut reader = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(2).unwrap();
    let (_, mut foreign) = bus.create_host(2).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let mut update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activation(&mut update).unwrap());
    assert_eq!(
        host.scope(&op).publish(&publisher.handle(), 1),
        Err(PublishError::InactiveBinding)
    );
    let failure = foreign.activate(update).unwrap_err();
    assert_eq!(failure.error, ActivateError::ForeignHost);
    let mut update = failure.update;
    drop(host.activation(&mut update).unwrap().commit());
    host.begin_block();
    host.scope(&op).publish(&publisher.handle(), 7).unwrap();
    let stale = control.prepare_update(&[publisher.erase()]).unwrap();
    let fresh = control.prepare_update(&[publisher.erase()]).unwrap();
    drop(host.activate(fresh).unwrap());
    let failure = host.activate(stale).unwrap_err();
    assert_eq!(failure.error, ActivateError::StaleBasis);
    assert!(failure.update.op_bindings(key).is_ok());
    pump(&mut bus);
    assert_eq!(reader.next().unwrap().value, 7);
}

#[test]
fn stopped_host_rebind_preserves_actual_stream_queue_and_checked_sequence() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_stream_value("stream", 0_u32, 2).unwrap();
    let mut reader = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(2).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    host.begin_block();
    host.scope(&op).publish(&publisher.handle(), 11).unwrap();
    drop(host);
    assert_eq!(topic.status().publisher, PublisherStatus::NoPublisher);
    assert!(matches!(
        control.prepare(&topic, key),
        Err(HostError::Closed)
    ));
    let (next_control, mut next_host) = bus.create_host(2).unwrap();
    let next_key = next_control.new_binding().unwrap();
    let next = next_control.prepare(&topic, next_key).unwrap();
    let update = next_control.prepare_update(&[next.erase()]).unwrap();
    let next_op = update.op_bindings(next_key).unwrap();
    drop(next_host.activate(update).unwrap());
    next_host.begin_block();
    assert!(
        matches!(next_host.scope(&next_op).publish(&next.handle(), 12).unwrap(), PublishOutcome::Published(meta) if meta.sequence == 2 && meta.activation_generation == 1)
    );
    pump(&mut bus);
    assert_eq!(reader.next().unwrap().value, 11);
    assert_eq!(reader.next().unwrap().value, 12);
}

#[test]
fn prepared_active_retired_and_operation_leases_make_unregister_busy() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("lease", 0_u32).unwrap();
    let (control, mut host) = bus.create_host(1).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(publisher);
    drop(host.activate(update).unwrap());
    assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
    let retired = host.activate(control.prepare_update(&[]).unwrap()).unwrap();
    assert_eq!(topic.status().publisher, PublisherStatus::NoPublisher);
    assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
    drop(retired);
    assert_eq!(bus.unregister(&topic), Err(UnregisterError::Busy));
    drop(op);
    bus.unregister(&topic).unwrap();
    assert!(matches!(
        control.prepare(&topic, key),
        Err(HostError::Closed)
    ));
    drop(host);
}

#[test]
fn multi_topic_operation_and_both_block_shapes_use_the_real_endpoints() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus
        .register_latest_block(
            "latest",
            BlockInit {
                header: 0_u32,
                element: 0_u32,
                capacity: 3,
            },
        )
        .unwrap();
    let stream = bus
        .register_stream_block(
            "stream",
            BlockInit {
                header: 0_u32,
                element: 0_u32,
                capacity: 3,
            },
            2,
        )
        .unwrap();
    let mut latest_reader = bus.subscribe(&latest, Default::default()).unwrap();
    let mut stream_reader = bus.subscribe(&stream, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(2).unwrap();
    let key = control.new_binding().unwrap();
    let latest_publisher = control.prepare(&latest, key).unwrap();
    let stream_publisher = control.prepare(&stream, key).unwrap();
    let update = control
        .prepare_update(&[latest_publisher.erase(), stream_publisher.erase()])
        .unwrap();
    assert!(matches!(
        update.op_bindings(control.new_binding().unwrap()),
        Err(HostError::UnknownBinding)
    ));
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    host.begin_block();
    {
        let mut scope = host.scope(&op);
        assert!(scope.demand(&latest_publisher.handle()).unwrap().active);
        assert!(scope.demand(&stream_publisher.handle()).unwrap().active);
        let mut write: moiren_signal::BlockWriteGuard<'_, u32, u32> =
            scope.try_write(&latest_publisher.handle(), 2).unwrap();
        *write.header_mut() = 5;
        write.elements_mut().copy_from_slice(&[6, 7]);
        write.commit(2).unwrap();
        let write: moiren_signal::StreamBlockWriteGuard<'_, u32, u32> =
            scope.try_write(&stream_publisher.handle(), 1).unwrap();
        drop(write);
        scope
            .publish_slice(&stream_publisher.handle(), 8, &[9, 10])
            .unwrap();
    }
    pump(&mut bus);
    let latest_read = latest_reader.read_latest().unwrap();
    assert_eq!(
        (*latest_read.header(), latest_read.elements()),
        (5, &[6, 7][..])
    );
    let stream_read = stream_reader.next().unwrap();
    assert_eq!(
        (*stream_read.header(), stream_read.elements()),
        (8, &[9, 10][..])
    );
}

#[test]
fn capacity_and_foreign_topic_preparation_are_atomic() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let mut foreign_bus = Bus::new(Default::default()).unwrap();
    assert!(matches!(
        bus.create_host(0),
        Err(HostError::InvalidCapacity)
    ));
    let topic = bus.register_latest_value("one", 0_u32).unwrap();
    let another = bus.register_latest_value("two", 0_u32).unwrap();
    let foreign = foreign_bus.register_latest_value("foreign", 0_u32).unwrap();
    let (control, _host) = bus.create_host(1).unwrap();
    let key = control.new_binding().unwrap();
    assert!(matches!(
        control.prepare(&foreign, key),
        Err(HostError::ForeignTopic)
    ));
    let publisher = control.prepare(&topic, key).unwrap();
    assert!(matches!(
        control.prepare(&another, key),
        Err(HostError::HostCapacity)
    ));
    assert_eq!(another.status().publisher, PublisherStatus::NoPublisher);
    drop(publisher);
}

#[test]
fn function_pointer_payloads_keep_exact_public_publication_types() {
    type Callback = fn(&u32) -> u32;
    fn read(value: &u32) -> u32 {
        *value
    }
    fn copy_handle<P: moiren_signal::Shape>(
        handle: moiren_signal::PublisherHandle<P>,
    ) -> moiren_signal::PublisherHandle<P> {
        fn require_traits<Handle: Copy + Clone + Send + Sync>() {}
        require_traits::<moiren_signal::PublisherHandle<P>>();
        handle
    }

    let callback: Callback = read;
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest_value = bus.register_latest_value("latest/value", callback).unwrap();
    let stream_value = bus
        .register_stream_value("stream/value", callback, 2)
        .unwrap();
    let init = BlockInit {
        header: callback,
        element: callback,
        capacity: 1,
    };
    let latest_block = bus.register_latest_block("latest/block", init).unwrap();
    let stream_block = bus.register_stream_block("stream/block", init, 2).unwrap();
    let mut latest_value_reader = bus.subscribe(&latest_value, Default::default()).unwrap();
    let mut stream_value_reader = bus.subscribe(&stream_value, Default::default()).unwrap();
    let mut latest_block_reader = bus.subscribe(&latest_block, Default::default()).unwrap();
    let mut stream_block_reader = bus.subscribe(&stream_block, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(4).unwrap();
    let key = control.new_binding().unwrap();
    let latest_value_publisher = control.prepare(&latest_value.clone(), key).unwrap();
    let stream_value_publisher = control.prepare(&stream_value.clone(), key).unwrap();
    let latest_block_publisher = control.prepare(&latest_block.clone(), key).unwrap();
    let stream_block_publisher = control.prepare(&stream_block.clone(), key).unwrap();
    let update = control
        .prepare_update(&[
            latest_value_publisher.erase(),
            stream_value_publisher.erase(),
            latest_block_publisher.erase(),
            stream_block_publisher.erase(),
        ])
        .unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    host.begin_block();
    {
        let mut scope = host.scope(&op);
        scope
            .publish(&copy_handle(latest_value_publisher.handle()), callback)
            .unwrap();
        scope
            .publish(&copy_handle(stream_value_publisher.handle()), callback)
            .unwrap();
        let mut write: moiren_signal::BlockWriteGuard<'_, Callback, Callback> = scope
            .try_write(&copy_handle(latest_block_publisher.handle()), 1)
            .unwrap();
        *write.header_mut() = callback;
        write.elements_mut()[0] = callback;
        write.commit(1).unwrap();
        scope
            .publish_slice(
                &copy_handle(stream_block_publisher.handle()),
                callback,
                &[callback],
            )
            .unwrap();
    }
    pump(&mut bus);
    let value = 42;
    assert_eq!(
        (latest_value_reader.read_latest().unwrap().value)(&value),
        value
    );
    assert_eq!((stream_value_reader.next().unwrap().value)(&value), value);
    let latest_read = latest_block_reader.read_latest().unwrap();
    assert_eq!((latest_read.header())(&value), value);
    assert_eq!((latest_read.elements()[0])(&value), value);
    let stream_read = stream_block_reader.next().unwrap();
    assert_eq!((stream_read.header())(&value), value);
    assert_eq!((stream_read.elements()[0])(&value), value);
}

#[test]
fn compatible_activation_preserves_cached_demand_until_the_next_block() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("epoch", 0_u32).unwrap();
    let subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, mut host) = bus.create_host(1).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    drop(host.activate(update).unwrap());
    host.begin_block();
    drop(subscriber);
    let mut next_reader = bus.subscribe(&topic, Default::default()).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    assert!(
        matches!(host.scope(&op).publish(&publisher.handle(), 1).unwrap(), PublishOutcome::Published(meta) if meta.activation_generation == 1)
    );
    assert_eq!(
        host.scope(&op)
            .demand(&publisher.handle())
            .unwrap()
            .generation,
        1
    );
    pump(&mut bus);
    assert!(next_reader.read_latest().is_none());
    host.begin_block();
    assert!(
        matches!(host.scope(&op).publish(&publisher.handle(), 2).unwrap(), PublishOutcome::Published(meta) if meta.sequence == 2 && meta.activation_generation == 2)
    );
    assert_eq!(
        host.scope(&op)
            .demand(&publisher.handle())
            .unwrap()
            .generation,
        2
    );
    pump(&mut bus);
    assert_eq!(next_reader.read_latest().unwrap().value, 2);
}

#[test]
fn closed_retained_slot_is_reclaimed_off_rt_for_new_registration() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("reuse", 0_u32).unwrap();
    let (control, mut host) = bus.create_host(1).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let old_handle = publisher.handle();
    drop(publisher);
    bus.unregister(&topic).unwrap();
    let next_topic = bus.register_latest_value("reuse", 0_u32).unwrap();
    assert_ne!(topic.id(), next_topic.id());
    let mut reader = bus.subscribe(&next_topic, Default::default()).unwrap();
    let publisher = control.prepare(&next_topic, key).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    host.begin_block();
    assert_eq!(
        host.scope(&op).publish(&old_handle, 1),
        Err(PublishError::InactiveBinding)
    );
    host.scope(&op).publish(&publisher.handle(), 2).unwrap();
    pump(&mut bus);
    assert_eq!(reader.read_latest().unwrap().value, 2);
    assert!(matches!(
        control.prepare(&topic, key),
        Err(HostError::Closed)
    ));
}
