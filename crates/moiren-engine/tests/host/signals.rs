use super::support::*;
use moiren_core::graph::{NodeKind, SendParams};
use moiren_engine::{
    host::*,
    meter::MeterSettings,
    processor::CompressorSettings,
    rack::{RackRoute, RackSettings},
    recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn spectrum_project() -> (EngineProject, moiren_core::graph::NodeId) {
    let (mut project, input, gain, output) = project();
    project.graph.remove_node(gain).unwrap();
    project.nodes.remove(&gain);
    let spectrum = project
        .graph
        .create_node(NodeKind::SpectrumAnalyzer, 1)
        .unwrap();
    project.nodes.insert(
        spectrum,
        NodeConfig::SpectrumAnalyzer(SpectrumSettings {
            fft_size: 16,
            hop_frames: 16,
        }),
    );
    let source = project.graph.get_node(input).unwrap().outputs()[0].id();
    let destination = project.graph.get_node(output).unwrap().inputs()[0].id();
    let spectrum_input = project.graph.get_node(spectrum).unwrap().inputs()[0].id();
    let spectrum_output = project.graph.get_node(spectrum).unwrap().outputs()[0].id();
    project
        .graph
        .connect(source, spectrum_input, SendParams::default())
        .unwrap();
    project
        .graph
        .connect(spectrum_output, destination, SendParams::default())
        .unwrap();
    (project, spectrum)
}

fn wait_spectrum(subscription: &mut ManagedSpectrumSubscription, fft_size: usize) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(block) = subscription.subscriber.read_latest() {
            assert_eq!(block.header().fft_size, fft_size);
            assert_eq!(block.header().channels, 1);
            assert_eq!(block.elements().len(), fft_size / 2 + 1);
            return;
        }
        assert!(Instant::now() < deadline, "spectrum did not publish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_level(subscription: &mut ManagedLevelSubscription, channels: usize, peak: f32) {
    // A 1536-frame window needs 192 fake 8-frame callbacks. Windows may round
    // each 1 ms callback wait to a timer tick, exceeding the ordinary 3 s bound.
    let deadline = Instant::now() + TIMEOUT.max(Duration::from_secs(10));
    loop {
        if let Some(block) = subscription.subscriber.read_latest() {
            assert_eq!(block.elements().len(), channels);
            assert!(
                block
                    .elements()
                    .iter()
                    .all(|level| (level.peak - peak).abs() < 1e-5)
            );
            return;
        }
        assert!(Instant::now() < deadline, "level did not publish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn levels_follow_rack_port_identity_and_width_only_after_activation() {
    // Equal widths still require different topics when the observed port changes.
    for primary_channels in [2, 1] {
        let (mut project, input, gain, output) = project();
        project.graph.remove_node(gain).unwrap();
        project.nodes.remove(&gain);
        let rack = project
            .graph
            .create_rack(&[2, 1], &[primary_channels, 1])
            .unwrap();
        project.nodes.insert(
            rack,
            NodeConfig::Rack(RackSettings {
                routes: vec![RackRoute {
                    input_port: 1,
                    output_port: 1,
                    slots: vec![],
                }],
                ..Default::default()
            }),
        );
        let source = project.graph.get_node(input).unwrap().outputs()[0].id();
        let destination = project.graph.get_node(output).unwrap().inputs()[0].id();
        let node = project.graph.get_node(rack).unwrap();
        let rack_input = node.inputs()[1].id();
        let unused_output = node.outputs()[0].id();
        let rack_output = node.outputs()[1].id();
        project
            .graph
            .connect(source, rack_input, SendParams::default())
            .unwrap();
        project
            .graph
            .connect(rack_output, destination, SendParams::default())
            .unwrap();
        let backend = FakeBackend::default();
        let state = backend.0.clone();
        let config = HostConfig {
            signal_bindings: 4,
            signals: moiren_signal::BusConfig {
                max_topics: 4,
                ..Default::default()
            },
            ..Default::default()
        };
        let host = EngineHost::<f64>::open(config, project, backend).unwrap();
        let observer = host.observer();
        let initial = wait(&observer, |snapshot| snapshot.active_revision == Some(1));
        let mut old = observer.subscribe_level(rack).unwrap();
        wait_level(&mut old, primary_channels, 0.0);
        state.pause_audio.store(true, Ordering::SeqCst);
        let edit = host
            .control()
            .edit(move |project| {
                project.graph.remove_output_port(rack, unused_output)?;
                let NodeConfig::Rack(settings) = project.nodes.get_mut(&rack).unwrap() else {
                    unreachable!()
                };
                settings.routes[0].output_port = 0;
                Ok(())
            })
            .unwrap();
        wait(&observer, |snapshot| {
            matches!(
                snapshot.update,
                UpdateState::Pending(_) | UpdateState::Blocked { .. }
            )
        });
        assert!(
            matches!(observer.snapshot().update, UpdateState::Pending(_)),
            "{:?}",
            observer.snapshot()
        );
        let during = observer.subscribe_level(rack).unwrap();
        assert_eq!(
            during.subscriber.status().publisher,
            moiren_signal::PublisherStatus::Bound
        );
        assert_eq!(
            old.subscriber.status().lifecycle,
            moiren_signal::TopicLifecycle::Registered
        );
        state.pause_audio.store(false, Ordering::SeqCst);
        assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
        assert_eq!(
            observer.snapshot().owner_generation,
            initial.owner_generation
        );
        let mut active = observer.subscribe_level(rack).unwrap();
        wait_level(&mut active, 1, 1.0);
        wait(&observer, |_| {
            old.subscriber.status().lifecycle == moiren_signal::TopicLifecycle::Closed
        });
        assert_eq!(
            during.subscriber.status().lifecycle,
            moiren_signal::TopicLifecycle::Closed
        );
        drop(host);
        assert_eq!(
            active.subscriber.status().lifecycle,
            moiren_signal::TopicLifecycle::Closed
        );
        assert!(observer.subscribe_level(rack).is_err());
    }
}

#[test]
fn cancelled_spectrum_resize_preserves_active_subscription_and_reclaims_candidate() {
    let (project, spectrum) = spectrum_project();
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let config = HostConfig {
        signal_bindings: 5,
        signals: moiren_signal::BusConfig {
            max_topics: 5,
            ..Default::default()
        },
        ..Default::default()
    };
    let host = EngineHost::<f64>::open(config, project, backend).unwrap();
    let observer = host.observer();
    wait(&observer, |snapshot| snapshot.active_revision == Some(1));
    let mut active = observer.subscribe_spectrum(spectrum).unwrap();
    wait_spectrum(&mut active, 16);
    state.pause_audio.store(true, Ordering::SeqCst);
    let cancelled = host
        .control()
        .edit(move |project| {
            project.nodes.insert(
                spectrum,
                NodeConfig::SpectrumAnalyzer(SpectrumSettings {
                    fft_size: 32,
                    hop_frames: 32,
                }),
            );
            Ok(())
        })
        .unwrap();
    wait(&observer, |snapshot| {
        matches!(snapshot.update, UpdateState::Pending(_))
    });
    let replacement = host
        .control()
        .edit(move |project| {
            project.nodes.insert(
                spectrum,
                NodeConfig::SpectrumAnalyzer(SpectrumSettings {
                    fft_size: 16,
                    hop_frames: 16,
                }),
            );
            Ok(())
        })
        .unwrap();
    wait(&observer, |snapshot| snapshot.desired_revision == 3);
    assert!(matches!(
        cancelled.outcome(),
        EditOutcome::Superseded { .. }
    ));
    state.pause_audio.store(false, Ordering::SeqCst);
    assert!(matches!(
        replacement.wait(TIMEOUT),
        EditOutcome::Activated { .. }
    ));
    assert_eq!(
        active.subscriber.status().lifecycle,
        moiren_signal::TopicLifecycle::Registered
    );
    wait_spectrum(&mut active, 16);
    // A third shape needs the cancelled candidate's topic and host slot back.
    let resize = host
        .control()
        .edit(move |project| {
            project.nodes.insert(
                spectrum,
                NodeConfig::SpectrumAnalyzer(SpectrumSettings {
                    fft_size: 64,
                    hop_frames: 64,
                }),
            );
            Ok(())
        })
        .unwrap();
    assert!(
        matches!(resize.wait(TIMEOUT), EditOutcome::Activated { .. }),
        "{:?}",
        observer.snapshot()
    );
    let mut next = observer.subscribe_spectrum(spectrum).unwrap();
    wait_spectrum(&mut next, 64);
}

#[test]
fn spectrum_resize_switches_topics_only_on_activation_and_reclaims_old_shapes() {
    let (project, spectrum) = spectrum_project();
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    // Three level topics plus the active and candidate spectrum topics.
    let config = HostConfig {
        signal_bindings: 5,
        signals: moiren_signal::BusConfig {
            max_topics: 5,
            ..Default::default()
        },
        ..Default::default()
    };
    let host = EngineHost::<f64>::open(config, project, backend).unwrap();
    let observer = host.observer();
    let initial = wait(&observer, |snapshot| snapshot.active_revision == Some(1));
    let mut subscription = observer.subscribe_spectrum(spectrum).unwrap();
    wait_spectrum(&mut subscription, 16);
    for fft_size in [32, 64, 16] {
        state.pause_audio.store(true, Ordering::SeqCst);
        let edit = host
            .control()
            .edit(move |project| {
                project.nodes.insert(
                    spectrum,
                    NodeConfig::SpectrumAnalyzer(SpectrumSettings {
                        fft_size,
                        hop_frames: fft_size,
                    }),
                );
                Ok(())
            })
            .unwrap();
        wait(&observer, |snapshot| {
            matches!(snapshot.update, UpdateState::Pending(_))
                || matches!(snapshot.update, UpdateState::Blocked { .. })
        });
        assert!(
            matches!(observer.snapshot().update, UpdateState::Pending(_)),
            "{:?}",
            observer.snapshot()
        );
        let candidate_subscription = observer.subscribe_spectrum(spectrum).unwrap();
        assert_eq!(
            candidate_subscription.subscriber.status().publisher,
            moiren_signal::PublisherStatus::Bound
        );
        assert_eq!(
            subscription.subscriber.status().lifecycle,
            moiren_signal::TopicLifecycle::Registered
        );
        state.pause_audio.store(false, Ordering::SeqCst);
        assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
        assert_eq!(
            observer.snapshot().owner_generation,
            initial.owner_generation
        );
        let mut next = observer.subscribe_spectrum(spectrum).unwrap();
        wait_spectrum(&mut next, fft_size);
        wait(&observer, |_| {
            subscription.subscriber.status().lifecycle == moiren_signal::TopicLifecycle::Closed
        });
        assert_eq!(
            candidate_subscription.subscriber.status().lifecycle,
            moiren_signal::TopicLifecycle::Closed
        );
        subscription = next;
    }
    drop(host);
    assert_eq!(
        subscription.subscriber.status().lifecycle,
        moiren_signal::TopicLifecycle::Closed
    );
    assert!(observer.subscribe_spectrum(spectrum).is_err());
}

#[test]
fn unrelated_updates_preserve_recorder_state_and_unread_queue() {
    for recording in [false, true] {
        let (mut project, input, gain, output) = project();
        project.graph.remove_node(gain).unwrap();
        project.nodes.remove(&gain);
        let recorder = project.graph.create_node(NodeKind::Recorder, 1).unwrap();
        project.nodes.insert(
            recorder,
            NodeConfig::Recorder(RecorderSettings {
                capacity_frames: 64,
            }),
        );
        let source = project.graph.get_node(input).unwrap().outputs()[0].id();
        let destination = project.graph.get_node(output).unwrap().inputs()[0].id();
        let recorder_input = project.graph.get_node(recorder).unwrap().inputs()[0].id();
        let recorder_output = project.graph.get_node(recorder).unwrap().outputs()[0].id();
        project
            .graph
            .connect(source, recorder_input, SendParams::default())
            .unwrap();
        project
            .graph
            .connect(recorder_output, destination, SendParams::default())
            .unwrap();
        let host = EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default())
            .unwrap();
        let observer = host.observer();
        let initial = wait(&observer, |snapshot| {
            snapshot.execution == ExecutionState::Running
        });
        let mut reader = observer.recorder_reader(recorder).unwrap();
        wait(&observer, |_| reader.queued_frames() == Some(64));
        if !recording {
            assert!(reader.stop());
        }
        let mut prefix = [0.0; 8];
        assert_eq!(reader.read_interleaved(&mut prefix), Some(8));
        assert_eq!(prefix, [1.0; 8]);
        let before = reader.stats().unwrap();
        let queued = reader.queued_frames().unwrap();
        let edit = host
            .control()
            .edit(|project| {
                let unrelated = project.graph.create_node(NodeKind::Gain, 1)?;
                project
                    .nodes
                    .insert(unrelated, NodeConfig::Gain { initial: 0.5 });
                Ok(())
            })
            .unwrap();
        assert!(matches!(edit.wait(TIMEOUT), EditOutcome::Activated { .. }));
        assert_eq!(
            observer.snapshot().owner_generation,
            initial.owner_generation
        );
        assert_eq!(reader.is_recording(), Some(recording));
        let after = reader.stats().unwrap();
        assert_eq!(after.read_frames, before.read_frames);
        assert!(after.accepted_frames >= before.accepted_frames);
        assert!(reader.queued_frames().unwrap() >= queued);
        if !recording {
            assert_eq!(reader.queued_frames(), Some(queued));
            // A callback that observed Recording before stop may finish once.
            assert!(after.accepted_frames <= before.accepted_frames + 8);
        }
        let mut tail = [0.0; 64];
        let frames = reader.read_interleaved(&mut tail).unwrap();
        assert!(frames >= queued);
        assert!(tail[..frames].iter().all(|sample| *sample == 1.0));
        assert!(reader.start());
        wait(&observer, |_| {
            reader.queued_frames().is_some_and(|frames| frames > 0)
        });
        drop(host);
        assert_eq!(reader.is_recording(), None);
        assert!(!reader.start());
    }
}

#[test]
fn signals_attach_automatically_and_retained_subscriber_does_not_hold_owner_alive() {
    let (mut project, input, gain, output) = project();
    project.graph.remove_node(gain).unwrap();
    project.nodes.remove(&gain);
    let compressor = project.graph.create_node(NodeKind::Compressor, 1).unwrap();
    project.nodes.insert(
        compressor,
        NodeConfig::Compressor(CompressorSettings::default()),
    );
    let source = project.graph.get_node(input).unwrap().outputs()[0].id();
    let destination = project.graph.get_node(output).unwrap().inputs()[0].id();
    let compressor_input = project.graph.get_node(compressor).unwrap().inputs()[0].id();
    let compressor_output = project.graph.get_node(compressor).unwrap().outputs()[0].id();
    project
        .graph
        .connect(source, compressor_input, SendParams::default())
        .unwrap();
    project
        .graph
        .connect(compressor_output, destination, SendParams::default())
        .unwrap();
    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    let snapshot = wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let mut subscription = observer.subscribe_compressor(compressor).unwrap();
    assert_eq!(subscription.owner_generation, snapshot.owner_generation);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if subscription.subscriber.read_latest().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
    assert_eq!(
        subscription.subscriber.status().publisher,
        moiren_signal::PublisherStatus::NoPublisher
    );
}

#[test]
fn managed_measurements_keep_readers_and_publish_typed_spectrum_blocks() {
    let (mut project, input, gain, output) = project();
    project.graph.remove_node(gain).unwrap();
    project.nodes.remove(&gain);

    let meter = project.graph.create_node(NodeKind::Meter, 1).unwrap();
    let spectrum = project
        .graph
        .create_node(NodeKind::SpectrumAnalyzer, 1)
        .unwrap();
    let recorder = project.graph.create_node(NodeKind::Recorder, 1).unwrap();
    project
        .nodes
        .insert(meter, NodeConfig::Meter(MeterSettings { window_frames: 4 }));
    project.nodes.insert(
        spectrum,
        NodeConfig::SpectrumAnalyzer(SpectrumSettings {
            fft_size: 16,
            hop_frames: 16,
        }),
    );
    project.nodes.insert(
        recorder,
        NodeConfig::Recorder(RecorderSettings {
            capacity_frames: 64,
        }),
    );

    let source = project.graph.get_node(input).unwrap().outputs()[0].id();
    let meter_input = project.graph.get_node(meter).unwrap().inputs()[0].id();
    let meter_output = project.graph.get_node(meter).unwrap().outputs()[0].id();
    let spectrum_input = project.graph.get_node(spectrum).unwrap().inputs()[0].id();
    let spectrum_output = project.graph.get_node(spectrum).unwrap().outputs()[0].id();
    let recorder_input = project.graph.get_node(recorder).unwrap().inputs()[0].id();
    let recorder_output = project.graph.get_node(recorder).unwrap().outputs()[0].id();
    let destination = project.graph.get_node(output).unwrap().inputs()[0].id();
    for (source, destination) in [
        (source, meter_input),
        (meter_output, spectrum_input),
        (spectrum_output, recorder_input),
        (recorder_output, destination),
    ] {
        project
            .graph
            .connect(source, destination, SendParams::default())
            .unwrap();
    }

    let host =
        EngineHost::<f64>::open(HostConfig::default(), project, FakeBackend::default()).unwrap();
    let observer = host.observer();
    let snapshot = wait(&observer, |snapshot| {
        snapshot.execution == ExecutionState::Running
    });
    let mut meter_reader = observer.meter_reader(meter).unwrap();
    let recorder_reader = observer.recorder_reader(recorder).unwrap();
    let mut level = observer.subscribe_level(meter).unwrap();
    let mut spectrum = observer.subscribe_spectrum(spectrum).unwrap();
    assert_eq!(meter_reader.owner_generation(), snapshot.owner_generation);
    assert_eq!(
        recorder_reader.owner_generation(),
        snapshot.owner_generation
    );

    let deadline = Instant::now() + TIMEOUT;
    let mut meter_snapshot = None;
    let mut level_seen = false;
    let mut spectrum_seen = false;
    while meter_snapshot.is_none()
        || !level_seen
        || !spectrum_seen
        || recorder_reader.queued_frames() == Some(0)
    {
        meter_snapshot = meter_reader.latest().or(meter_snapshot);
        if let Some(block) = level.subscriber.read_latest() {
            assert_eq!(block.header().frame_end - block.header().frame_start, 4);
            level_seen = true;
        }
        if let Some(block) = spectrum.subscriber.read_latest() {
            assert_eq!(block.header().fft_size, 16);
            spectrum_seen = true;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(meter_snapshot.unwrap().samples, 4);
    assert!(
        recorder_reader
            .queued_frames()
            .is_some_and(|frames| frames > 0)
    );
}
