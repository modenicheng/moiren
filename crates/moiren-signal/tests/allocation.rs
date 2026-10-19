use moiren_signal::{
    BlockInit, Bus, BusConfig, PublishOutcome, PumpBudget, SubscriptionOptions, WriteError,
};

mod common;

// The counting allocator is shared verbatim with the library's unit-test
// binary: integration tests cannot import `#[cfg(test)]` items, so this file
// textually includes the single owner of the counting semantics.
include!("../src/test_alloc.rs");

fn budget() -> PumpBudget {
    PumpBudget {
        topic_checks: 128,
        deliveries: 128,
        copy_bytes: 262_144,
    }
}

#[test]
fn rt_publication_and_activation_paths_do_not_allocate() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let latest_value = bus.register_latest_value("latest/value", 0_u32).unwrap();
    let latest_block = bus
        .register_latest_block(
            "latest/block",
            BlockInit {
                header: 0_u32,
                element: 0_u32,
                capacity: 2,
            },
        )
        .unwrap();
    let stream_block = bus
        .register_stream_block(
            "stream/block",
            BlockInit {
                header: 0_u32,
                element: 0_u32,
                capacity: 1,
            },
            1,
        )
        .unwrap();
    let mut latest_reader = bus.subscribe(&latest_value, Default::default()).unwrap();
    let mut block_reader = bus.subscribe(&latest_block, Default::default()).unwrap();
    let mut stream_reader = bus
        .subscribe(
            &stream_block,
            SubscriptionOptions {
                stream_depth: Some(1),
            },
        )
        .unwrap();
    let (control, mut host) = bus.create_host(5).unwrap();
    let key = control.new_binding().unwrap();
    let value_publisher = control.prepare(&latest_value, key).unwrap();
    let block_publisher = control.prepare(&latest_block, key).unwrap();
    let stream_publisher = control.prepare(&stream_block, key).unwrap();
    let activation_topic = bus
        .register_latest_value("activation/new-topic", 0_u32)
        .unwrap();
    let _activation_reader = bus
        .subscribe(&activation_topic, Default::default())
        .unwrap();
    let activation_publisher = control.prepare(&activation_topic, key).unwrap();
    let update = control
        .prepare_update(&[
            value_publisher.erase(),
            block_publisher.erase(),
            stream_publisher.erase(),
            activation_publisher.erase(),
        ])
        .unwrap();
    let op = update.op_bindings(key).unwrap();
    let mut activation_retired = None;
    assert_eq!(
        measure(|| {
            activation_retired = Some(host.activate(update).unwrap());
        }),
        Counts::default()
    );
    drop(activation_retired);

    let value_handle = value_publisher.handle();
    let block_handle = block_publisher.handle();
    let stream_handle = stream_publisher.handle();
    assert_eq!(
        measure(|| {
            host.begin_block();
            let mut scope = host.scope(&op);
            assert!(matches!(
                scope.publish(&value_handle, 1),
                Ok(PublishOutcome::Published(_))
            ));
            assert!(matches!(
                scope.publish(&value_handle, 2),
                Ok(PublishOutcome::Published(_))
            ));
            let mut latest_write = scope.try_write(&block_handle, 2).unwrap();
            *latest_write.header_mut() = 7;
            latest_write.elements_mut().fill(9);
            assert!(matches!(
                latest_write.commit(2),
                Ok(PublishOutcome::Published(_))
            ));
            let mut aborted = scope.try_write(&block_handle, 2).unwrap();
            aborted.elements_mut().fill(11);
            drop(aborted);
            let mut stream_write = scope.try_write(&stream_handle, 1).unwrap();
            *stream_write.header_mut() = 13;
            stream_write.elements_mut()[0] = 17;
            assert!(matches!(
                stream_write.commit(1),
                Ok(PublishOutcome::Published(_))
            ));
            assert!(matches!(
                scope.try_write(&stream_handle, 1),
                Err(WriteError::Unavailable(
                    moiren_signal::WriteUnavailable::Full { .. }
                ))
            ));
        }),
        Counts::default()
    );
    common::pump_until_settled(&mut bus, budget());
    assert_eq!(latest_reader.read_latest().unwrap().value, 2);
    assert_eq!(block_reader.read_latest().unwrap().elements(), &[9, 9]);
    assert_eq!(stream_reader.next().unwrap().elements(), &[17]);

    let rejected_topic = bus
        .register_latest_value("activation/rejected-new-topic", 0_u32)
        .unwrap();
    let rejected_publisher = control.prepare(&rejected_topic, key).unwrap();
    let stale = control
        .prepare_update(&[
            value_publisher.erase(),
            block_publisher.erase(),
            rejected_publisher.erase(),
        ])
        .unwrap();
    let replacement = control
        .prepare_update(&[value_publisher.erase(), block_publisher.erase()])
        .unwrap();
    let replacement_op = replacement.op_bindings(key).unwrap();
    let mut retired = None;
    assert_eq!(
        measure(|| {
            retired = Some(host.activate(replacement).unwrap());
        }),
        Counts::default()
    );
    drop(retired);
    let mut failed = None;
    assert_eq!(
        measure(|| {
            failed = Some(host.activate(stale).unwrap_err());
        }),
        Counts::default()
    );
    assert_eq!(
        failed.as_ref().unwrap().error,
        moiren_signal::ActivateError::StaleBasis
    );
    assert_eq!(rejected_topic.stats().attempts, 0);
    drop(failed);

    drop(latest_reader);
    assert_eq!(
        measure(|| {
            host.begin_block();
            let mut scope = host.scope(&replacement_op);
            assert!(!scope.demand(&value_handle).unwrap().active);
            assert_eq!(
                scope.publish(&value_handle, 3),
                Ok(PublishOutcome::NoDemand)
            );
        }),
        Counts::default()
    );
    let mut latest_reader = bus.subscribe(&latest_value, Default::default()).unwrap();
    assert_eq!(
        measure(|| {
            host.begin_block();
            assert!(matches!(
                host.scope(&replacement_op).publish(&value_handle, 4),
                Ok(PublishOutcome::Published(_))
            ));
        }),
        Counts::default()
    );
    common::pump_until_settled(&mut bus, budget());
    assert_eq!(latest_reader.read_latest().unwrap().value, 4);
}
