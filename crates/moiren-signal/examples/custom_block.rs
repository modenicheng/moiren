use moiren_signal::{BlockInit, Bus, BusConfig, PumpBudget, SubscriptionOptions};

#[derive(Debug, Clone, Copy)]
struct FrameHeader {
    frame_start: u64,
    sample_rate: u32,
}

fn main() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let state = bus
        .register_latest_value("example/latest", 0.0_f32)
        .unwrap();
    let blocks = bus
        .register_stream_block(
            "example/frames",
            BlockInit {
                header: FrameHeader {
                    frame_start: 0,
                    sample_rate: 48_000,
                },
                element: 0.0_f32,
                capacity: 2,
            },
            8,
        )
        .unwrap();
    let mut state_reader = bus
        .subscribe(&state, SubscriptionOptions::default())
        .unwrap();
    let mut block_reader = bus
        .subscribe(&blocks, SubscriptionOptions::default())
        .unwrap();
    let (control, mut host) = bus.create_host(4).unwrap();
    let key = control.new_binding().unwrap();
    let state_publisher = control.prepare(&state, key).unwrap();
    let block_publisher = control.prepare(&blocks, key).unwrap();
    let update = control
        .prepare_update(&[state_publisher.erase(), block_publisher.erase()])
        .unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    let state_handle = state_publisher.handle();
    let block_handle = block_publisher.handle();
    let producer = std::thread::spawn(move || {
        for block in 0..4_u64 {
            host.begin_block();
            let mut scope = host.scope(&op);
            scope.publish(&state_handle, block as f32).unwrap();
            let mut write = scope.try_write(&block_handle, 2).unwrap();
            *write.header_mut() = FrameHeader {
                frame_start: block * 2,
                sample_rate: 48_000,
            };
            write
                .elements_mut()
                .copy_from_slice(&[block as f32, block as f32 + 0.5]);
            write.commit(2).unwrap();
        }
        (host, op)
    });
    let (host, op) = producer.join().unwrap();
    for _ in 0..8 {
        let report = bus.pump(PumpBudget {
            topic_checks: 128,
            deliveries: 128,
            copy_bytes: 262_144,
        });
        if !report.has_pending {
            break;
        }
    }
    let latest = state_reader.read_latest().unwrap();
    println!("latest={} sequence={}", latest.value, latest.meta.sequence);
    let mut output = [0.0_f32; 2];
    let mut received = 0;
    while let Some(message) = block_reader.copy_next(&mut output).unwrap() {
        received += 1;
        println!(
            "frame={} sr={} sequence={} values={:?}",
            message.header.frame_start,
            message.header.sample_rate,
            message.meta.sequence,
            &output[..message.len]
        );
    }
    assert_eq!(received, 4);
    assert_eq!(latest.value, 3.0);
    println!(
        "topic={:?} subscriber={:?}",
        blocks.stats(),
        block_reader.stats()
    );
    drop((host, op, state_publisher, block_publisher));
}
