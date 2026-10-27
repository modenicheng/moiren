use moiren_app::{
    monitor::{MonitorConfig, prepare_monitor_with_signals},
    service::{LevelTopic, SignalPreparation},
    tone::{ToneConfig, prepare_tone_with_signals},
};
use moiren_engine::{
    boundary::{AudioReader, ConstantSource},
    meter::signal::{AudioWindow, ChannelLevel},
    runtime::Engine,
};
use moiren_signal::{BlockInit, Bus, BusConfig, PublisherStatus, PumpBudget, Subscriber, Topic};

const UI_WINDOW_FRAMES: usize = 200;
const LEVEL_STREAM_DEPTH: usize = 16;

fn register(bus: &mut Bus, name: &str) -> Topic<LevelTopic> {
    bus.register_stream_block(
        name,
        BlockInit {
            header: AudioWindow {
                timeline_epoch: 0,
                frame_start: 0,
                frame_end: 0,
                sample_rate: 0.0,
            },
            element: ChannelLevel {
                peak: 0.0,
                rms: 0.0,
            },
            capacity: 2,
        },
        LEVEL_STREAM_DEPTH,
    )
    .unwrap()
}

/// Drains the stream FIFO and returns the newest window's sequence, header,
/// and levels.
fn newest_window(
    subscriber: &mut Subscriber<LevelTopic>,
) -> Option<(u64, AudioWindow, Vec<ChannelLevel>)> {
    let mut newest = None;
    while let Some(window) = subscriber.next() {
        newest = Some((
            window.meta().sequence,
            *window.header(),
            window.elements().to_vec(),
        ));
    }
    newest
}

fn render(engine: &mut Engine<f32>, output: &mut AudioReader<f32>, blocks: usize) {
    for _ in 0..blocks {
        engine.render(256).unwrap();
        assert_eq!(
            output
                .read_interleaved(&mut [0.0; 512])
                .unwrap()
                .transferred_frames,
            256
        );
    }
}

fn render_frames(engine: &mut Engine<f32>, output: &mut AudioReader<f32>, frames: usize) {
    engine.render(frames).unwrap();
    let mut samples = vec![0.0; frames * 2];
    assert_eq!(
        output
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        frames
    );
}

fn pump(bus: &mut Bus) {
    bus.pump(PumpBudget {
        topic_checks: 32,
        deliveries: 64,
        copy_bytes: 256 * 1024,
    });
}

#[test]
fn tone_publishes_only_complete_demanded_windows_and_rebind_keeps_sequence() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 2,
        ..Default::default()
    })
    .unwrap();
    let topic = register(&mut bus, "tone/output/level");
    let mut preparation = SignalPreparation::new(&mut bus, None, topic.clone()).unwrap();
    let config = ToneConfig {
        gain: 0.01,
        pan: 1.0,
        ..ToneConfig::default()
    };
    let mut tone = prepare_tone_with_signals(config, &mut preparation).unwrap();
    // No measurements may publish without demand, even after multiple windows.
    render(&mut tone.compiled.engine, &mut tone.output, 6);
    assert_eq!(topic.stats().attempts, 0);
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    // Demand starts a fresh 200-frame window at this audio position. Split it
    // across unequal callbacks to verify the exact publication boundary.
    render_frames(
        &mut tone.compiled.engine,
        &mut tone.output,
        UI_WINDOW_FRAMES - 1,
    );
    pump(&mut bus);
    assert!(subscriber.next().is_none());
    render_frames(&mut tone.compiled.engine, &mut tone.output, 1);
    assert_eq!(topic.stats().attempts, 1, "complete window must publish");
    pump(&mut bus);
    let first_sequence = {
        let window = subscriber.next().unwrap();
        assert_eq!(
            window.header().frame_end - window.header().frame_start,
            UI_WINDOW_FRAMES as u64
        );
        assert_eq!(window.header().sample_rate, 48_000.0);
        assert_eq!(
            window.elements()[0],
            ChannelLevel {
                peak: 0.0,
                rms: 0.0
            }
        );
        // This short window does not contain an integer number of sine cycles;
        // compare the actual finite interval rather than the infinite-wave RMS.
        let mut peak = 0.0_f64;
        let mut squares = 0.0;
        for frame in window.header().frame_start..window.header().frame_end {
            let sample = (std::f64::consts::TAU * config.frequency_hz * frame as f64 / 48_000.0)
                .sin()
                * config.gain;
            peak = peak.max(sample.abs());
            squares += sample * sample;
        }
        let rms = (squares / UI_WINDOW_FRAMES as f64).sqrt();
        assert!((f64::from(window.elements()[1].peak) - peak).abs() < 0.0000001);
        assert!((f64::from(window.elements()[1].rms) - rms).abs() < 0.0000001);
        window.meta().sequence
    };
    drop(subscriber);
    render(&mut tone.compiled.engine, &mut tone.output, 6);
    assert_eq!(topic.stats().attempts, first_sequence);
    drop(tone);
    drop(preparation);
    assert_eq!(topic.status().publisher, PublisherStatus::NoPublisher);
    let mut preparation = SignalPreparation::new(&mut bus, None, topic.clone()).unwrap();
    let mut tone = prepare_tone_with_signals(config, &mut preparation).unwrap();
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    pump(&mut bus);
    assert!(subscriber.next().is_none());
    // One complete window publishes; the sequence continues from before
    // the rebind instead of resetting.
    render_frames(
        &mut tone.compiled.engine,
        &mut tone.output,
        UI_WINDOW_FRAMES,
    );
    pump(&mut bus);
    assert_eq!(
        subscriber.next().unwrap().meta().sequence,
        first_sequence + 1
    );
}

#[test]
fn monitor_observes_source_and_final_pan_output_without_changing_audio() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 2,
        ..Default::default()
    })
    .unwrap();
    let input = register(&mut bus, "monitor/input/output/level");
    let output = register(&mut bus, "monitor/output/level");
    let mut input_subscription = bus.subscribe(&input, Default::default()).unwrap();
    let mut output_subscription = bus.subscribe(&output, Default::default()).unwrap();
    let mut preparation = SignalPreparation::new(&mut bus, Some(input), output).unwrap();
    let mut monitor = prepare_monitor_with_signals(
        ConstantSource {
            channels: 2,
            value: 0.25,
        },
        MonitorConfig {
            gain: 0.4,
            pan: 1.0,
            ..MonitorConfig::default()
        },
        &mut preparation,
    )
    .unwrap();
    for _ in 0..6 {
        monitor.compiled.engine.render(256).unwrap();
        let mut samples = [0.0; 512];
        monitor.output.read_interleaved(&mut samples).unwrap();
        assert!(
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .all(|pair| *pair == [0.0, 0.1])
        );
    }
    pump(&mut bus);
    let source = newest_window(&mut input_subscription).unwrap();
    let destination = newest_window(&mut output_subscription).unwrap();
    assert_eq!(source.1, destination.1);
    assert_eq!(
        source.2,
        &[ChannelLevel {
            peak: 0.25,
            rms: 0.25
        }; 2]
    );
    assert_eq!(
        destination.2,
        &[
            ChannelLevel {
                peak: 0.0,
                rms: 0.0
            },
            ChannelLevel {
                peak: 0.1,
                rms: 0.1
            },
        ]
    );
}

#[test]
fn retained_subscriber_distinguishes_restarted_tone_and_monitor_timelines() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = register(&mut bus, "session/output/level");
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let mut previous_epoch = None;
    let mut previous_sequence = 0;
    for session_index in 0..4 {
        let mut preparation = SignalPreparation::new(&mut bus, None, topic.clone()).unwrap();
        let (mut compiled, mut output) = if session_index < 2 {
            let tone = prepare_tone_with_signals(ToneConfig::default(), &mut preparation).unwrap();
            (tone.compiled, tone.output)
        } else {
            let monitor = prepare_monitor_with_signals(
                ConstantSource {
                    channels: 2,
                    value: 0.25,
                },
                MonitorConfig::default(),
                &mut preparation,
            )
            .unwrap();
            (monitor.compiled, monitor.output)
        };
        // Two callbacks complete two UI windows. The stream hands both to the
        // reader; the newest is the second, and the remaining 112 frames are
        // still a partial third window.
        render(&mut compiled.engine, &mut output, 2);
        pump(&mut bus);
        {
            let (sequence, header, _) = newest_window(&mut subscriber).unwrap();
            assert_eq!(header.frame_start, UI_WINDOW_FRAMES as u64);
            assert_eq!(header.frame_end, 2 * UI_WINDOW_FRAMES as u64);
            assert_ne!(header.timeline_epoch, 0);
            if let Some(epoch) = previous_epoch {
                assert_ne!(header.timeline_epoch, epoch);
            }
            assert!(sequence > previous_sequence);
            previous_epoch = Some(header.timeline_epoch);
            previous_sequence = sequence;
        }
        drop(compiled);
        drop(preparation);
        assert_eq!(topic.status().publisher, PublisherStatus::NoPublisher);
        assert_eq!(topic.business_subscribers(), 1);
    }
}
