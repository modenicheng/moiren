use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use moiren_signal::{BlockInit, Bus, BusConfig, PublishOutcome, PumpBudget, SubscriptionOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    sequence_echo: u64,
    inverse_echo: u64,
}

fn pump_budget() -> PumpBudget {
    PumpBudget {
        topic_checks: 256,
        deliveries: 256,
        copy_bytes: 262_144,
    }
}

#[test]
fn producer_pump_and_independent_subscribers_preserve_pairs_with_held_guard() {
    const MESSAGES: u64 = 24;
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus
        .register_stream_block(
            "frames",
            BlockInit {
                header: Frame {
                    sequence_echo: 0,
                    inverse_echo: 0,
                },
                element: 0_u64,
                capacity: 1,
            },
            32,
        )
        .unwrap();
    let value_topic = bus.register_latest_value("latest", 0_u64).unwrap();
    let mut values = bus.subscribe(&value_topic, Default::default()).unwrap();
    let first = bus
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(32),
            },
        )
        .unwrap();
    let second = bus
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(32),
            },
        )
        .unwrap();
    let (control, mut host) = bus.create_host(2).unwrap();
    let key = control.new_binding().unwrap();
    let frame_publisher = control.prepare(&topic, key).unwrap();
    let value_publisher = control.prepare(&value_topic, key).unwrap();
    let update = control
        .prepare_update(&[frame_publisher.erase(), value_publisher.erase()])
        .unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    let frame_handle = frame_publisher.handle();
    let value_handle = value_publisher.handle();
    let start = Arc::new(Barrier::new(4));
    let done = Arc::new(AtomicBool::new(false));
    let (producer_tx, producer_rx) = mpsc::sync_channel(1);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let (pump_tx, pump_rx) = mpsc::sync_channel(1);
    let producer_start = start.clone();
    let producer = thread::spawn(move || {
        producer_start.wait();
        for sequence in 1..=MESSAGES {
            host.begin_block();
            let mut scope = host.scope(&op);
            let frame = Frame {
                sequence_echo: sequence,
                inverse_echo: u64::MAX - sequence,
            };
            assert!(matches!(
                scope.publish(&value_handle, sequence),
                Ok(PublishOutcome::Published(_))
            ));
            let mut write = scope.try_write(&frame_handle, 1).unwrap();
            *write.header_mut() = frame;
            write.elements_mut()[0] = sequence;
            assert!(matches!(write.commit(1), Ok(PublishOutcome::Published(_))));
        }
        done_tx.send(()).unwrap();
        producer_tx.send((host, op)).unwrap();
    });
    let pump_start = start.clone();
    let pump_done = done.clone();
    let pump = thread::spawn(move || {
        let mut bus = bus;
        pump_start.wait();
        let mut complete = false;
        for _ in 0..256 {
            complete |= done_rx.try_recv().is_ok();
            let report = bus.pump(pump_budget());
            if complete && !report.has_pending {
                break;
            }
            thread::yield_now();
        }
        pump_done.store(true, Ordering::Release);
        let returned = producer_rx.recv().unwrap();
        pump_tx.send((bus, returned)).unwrap();
    });
    let hold_gate = Arc::new(Barrier::new(2));
    let release_gate = Arc::new(Barrier::new(2));
    let first_done = done.clone();
    let first_start = start.clone();
    let first_hold = hold_gate.clone();
    let first_release = release_gate.clone();
    let first_reader = thread::spawn(move || {
        first_start.wait();
        let mut first = first;
        let mut seen = Vec::new();
        let mut held = false;
        loop {
            if let Some(guard) = first.next() {
                let frame = *guard.header();
                assert_eq!(frame.sequence_echo ^ frame.inverse_echo, u64::MAX);
                if !held {
                    held = true;
                    first_hold.wait();
                    first_release.wait();
                }
                seen.push(frame.sequence_echo);
            } else if first_done.load(Ordering::Acquire) {
                break;
            } else {
                thread::yield_now();
            }
        }
        (seen, first.stats())
    });
    let second_done = done.clone();
    let second_start = start.clone();
    let second_hold = hold_gate.clone();
    let second_release = release_gate.clone();
    let second_reader = thread::spawn(move || {
        second_start.wait();
        let mut second = second;
        second_hold.wait();
        let mut seen = Vec::new();
        loop {
            if let Some(guard) = second.next() {
                let frame = *guard.header();
                assert_eq!(frame.sequence_echo ^ frame.inverse_echo, u64::MAX);
                seen.push(frame.sequence_echo);
            } else if second_done.load(Ordering::Acquire) {
                break;
            } else {
                thread::yield_now();
            }
        }
        second_release.wait();
        (seen, second.stats())
    });
    producer.join().unwrap();
    pump.join().unwrap();
    let (_bus, (returned_host, returned_op)) = pump_rx.recv().unwrap();
    let (first_seen, first_stats) = first_reader.join().unwrap();
    let (second_seen, second_stats) = second_reader.join().unwrap();
    drop((returned_host, returned_op, control));
    assert_eq!(first_seen, second_seen);
    assert_eq!(first_seen.len() as u64, MESSAGES);
    assert!(first_seen.windows(2).all(|window| window[0] < window[1]));
    assert_eq!(topic.stats().attempts, MESSAGES);
    assert_eq!(first_stats.overflow_drops, 0);
    assert_eq!(first_stats.delivered, MESSAGES);
    assert_eq!(second_stats.overflow_drops, 0);
    assert_eq!(second_stats.delivered, MESSAGES);
    assert_eq!(values.read_latest().unwrap().value, MESSAGES);
    assert_eq!(topic.stats().ingress_drops, 0);
}
