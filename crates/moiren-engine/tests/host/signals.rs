use super::support::*;
use moiren_core::graph::{NodeKind, SendParams};
use moiren_engine::{
    host::*, meter::MeterSettings, processor::CompressorSettings, recorder::RecorderSettings,
    spectrum::SpectrumSettings,
};
use std::time::{Duration, Instant};

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
