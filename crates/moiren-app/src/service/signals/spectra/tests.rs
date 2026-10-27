use super::*;

#[test]
fn spectrum_idle_subscriber_does_not_scan_the_maximum_directory_each_tick() {
    let mut service = SignalService::new().unwrap();
    let _reader = service
        .bus
        .subscribe(&service.topics.tone_output, Default::default())
        .unwrap();
    let now = Instant::now();
    for tick in 0..4 {
        let report = service
            .pump_if_due(now + Duration::from_millis(tick))
            .unwrap();
        assert_eq!(report.deliveries, 0);
        assert_eq!(report.copy_bytes, 0);
        assert!(
            report.topic_checks <= 131 * 4,
            "idle work exceeded the previous bounded pump: {report:?}"
        );
    }
}

#[test]
fn spectrum_maximum_payload_delivers_fairly_beside_a_busy_small_topic() {
    let mut service = SignalService::new().unwrap();
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let settings = SpectrumSettings {
        fft_size: 8192,
        hop_frames: 8192,
    };
    let small = service
        .bus
        .register_latest_value("busy/small", 0_u32)
        .unwrap();
    let mut small_reader = service.bus.subscribe(&small, Default::default()).unwrap();
    let mut bindings = Vec::new();
    let mut readers = Vec::new();
    for _ in 0..2 {
        let node = graph
            .create_node(moiren_core::graph::NodeKind::SpectrumAnalyzer, 256)
            .unwrap();
        let binding = service.register_spectrum(node, settings, 256, 1).unwrap();
        readers.push(
            service
                .bus
                .subscribe(&binding.topic, Default::default())
                .unwrap(),
        );
        bindings.push(binding);
    }
    let (control, mut host) = service.bus.create_host(3).unwrap();
    let small_key = control.new_binding().unwrap();
    let small_writer = control.prepare(&small, small_key).unwrap();
    let mut keys = Vec::new();
    let mut writers = Vec::new();
    for binding in &bindings {
        let key = control.new_binding().unwrap();
        writers.push(control.prepare(&binding.topic, key).unwrap());
        keys.push(key);
    }
    let mut erased = vec![small_writer.erase()];
    erased.extend(writers.iter().map(|writer| writer.erase()));
    let update = control.prepare_update(&erased).unwrap();
    let small_op = update.op_bindings(small_key).unwrap();
    let operations: Vec<_> = keys
        .into_iter()
        .map(|key| update.op_bindings(key).unwrap())
        .collect();
    drop(host.activate(update).unwrap());
    let now = Instant::now();
    let mut received = [false; 2];
    for tick in 0..40 {
        host.begin_block();
        host.scope(&small_op)
            .publish(&small_writer.handle(), tick)
            .unwrap();
        for (index, writer) in writers.iter().enumerate() {
            let mut scope = host.scope(&operations[index]);
            let mut write = scope
                .try_write(&writer.handle(), writer.capacity())
                .unwrap();
            write.elements_mut().fill(index as f32 + 0.5);
            write.commit(writer.capacity()).unwrap();
        }
        let report = service
            .pump_if_due(now + Duration::from_millis(u64::from(tick)))
            .unwrap();
        assert!(report.copy_bytes <= MAX_SPECTRUM_COPY_BYTES + SIGNAL_BASE_COPY_BYTES);
        assert!(report.topic_checks <= SIGNAL_TOPICS * 4);
        for (index, reader) in readers.iter_mut().enumerate() {
            if let Some(window) = reader.read_latest() {
                assert_eq!(window.elements().len(), 256 * (8192 / 2 + 1));
                assert_eq!(window.elements()[0], index as f32 + 0.5);
                assert_eq!(window.elements().last(), Some(&(index as f32 + 0.5)));
                received[index] = true;
            }
        }
        let _ = small_reader.read_latest();
        if received.iter().all(|received| *received) {
            return;
        }
    }
    panic!("maximum spectra starved: {received:?}");
}

#[test]
fn spectrum_latest_read_consumes_sequence_instead_of_replaying_retained_storage() {
    let mut service = SignalService::new().unwrap();
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::SpectrumAnalyzer, 1)
        .unwrap();
    let binding = service
        .register_spectrum(node, SpectrumSettings::default(), 1, 1)
        .unwrap();
    let mut reader = service
        .bus
        .subscribe(&binding.topic, Default::default())
        .unwrap();
    let (control, mut host) = service.create_spectrum_host().unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&binding.topic, key).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let operation = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    host.begin_block();
    let mut scope = host.scope(&operation);
    let mut write = scope
        .try_write(&publisher.handle(), publisher.capacity())
        .unwrap();
    write.elements_mut().fill(0.25);
    write.commit(publisher.capacity()).unwrap();
    let now = Instant::now();
    let mut received = false;
    for tick in 0..40 {
        service
            .pump_if_due(now + Duration::from_millis(tick))
            .unwrap();
        if let Some(window) = reader.read_latest() {
            assert!(!received, "a retained publication was replayed");
            assert_eq!(window.meta().sequence, 1);
            received = true;
        }
        assert!(reader.read_latest().is_none());
    }
    assert!(received);
}

#[test]
fn spectrum_registration_closes_old_subscribers_and_never_seeds_replacement() {
    let mut service = SignalService::new().unwrap();
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph
        .create_node(moiren_core::graph::NodeKind::SpectrumAnalyzer, 2)
        .unwrap();
    let old = service
        .register_spectrum(node, SpectrumSettings::default(), 2, 4)
        .unwrap();
    let mut old_reader = service
        .bus
        .subscribe(&old.topic, Default::default())
        .unwrap();
    service.unregister_spectrum(node).unwrap();
    assert_eq!(
        old.topic.status().lifecycle,
        moiren_signal::TopicLifecycle::Closed
    );
    let new = service
        .register_spectrum(node, SpectrumSettings::default(), 2, 5)
        .unwrap();
    let mut new_reader = service
        .bus
        .subscribe(&new.topic, Default::default())
        .unwrap();
    assert_ne!(old.topic.id(), new.topic.id());
    assert!(old_reader.read_latest().is_none());
    assert!(new_reader.read_latest().is_none());
}
