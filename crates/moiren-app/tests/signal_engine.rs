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
use moiren_signal::{BlockInit, Bus, BusConfig, PublisherStatus, PumpBudget, Topic};

fn register(bus: &mut Bus, name: &str) -> Topic<LevelTopic> {
    bus.register_latest_block(
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
    )
    .unwrap()
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
    render(&mut tone.compiled.engine, &mut tone.output, 6);
    assert_eq!(topic.stats().attempts, 0);
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    render(&mut tone.compiled.engine, &mut tone.output, 5);
    pump(&mut bus);
    assert!(subscriber.read_latest().is_none());
    render(&mut tone.compiled.engine, &mut tone.output, 1);
    assert_eq!(topic.stats().attempts, 1, "complete window must publish");
    pump(&mut bus);
    let first_sequence = {
        let window = subscriber.read_latest().unwrap();
        assert_eq!(
            window.header().frame_end - window.header().frame_start,
            1536
        );
        assert_eq!(window.header().sample_rate, 48_000.0);
        assert_eq!(
            window.elements()[0],
            ChannelLevel {
                peak: 0.0,
                rms: 0.0
            }
        );
        assert!((window.elements()[1].peak - 0.01).abs() < 0.00001);
        assert!((window.elements()[1].rms - 0.01 / 2.0_f32.sqrt()).abs() < 0.0001);
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
    assert!(subscriber.read_latest().is_none());
    render(&mut tone.compiled.engine, &mut tone.output, 6);
    pump(&mut bus);
    assert_eq!(
        subscriber.read_latest().unwrap().meta().sequence,
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
    let source = input_subscription.read_latest().unwrap();
    let destination = output_subscription.read_latest().unwrap();
    assert_eq!(source.header(), destination.header());
    assert_eq!(
        source.elements(),
        &[ChannelLevel {
            peak: 0.25,
            rms: 0.25
        }; 2]
    );
    assert_eq!(
        destination.elements(),
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
        render(&mut compiled.engine, &mut output, 6);
        pump(&mut bus);
        {
            let window = subscriber.read_latest().unwrap();
            assert_eq!(window.header().frame_start, 0);
            assert_eq!(window.header().frame_end, 1536);
            assert_ne!(window.header().timeline_epoch, 0);
            if let Some(epoch) = previous_epoch {
                assert_ne!(window.header().timeline_epoch, epoch);
            }
            assert!(window.meta().sequence > previous_sequence);
            previous_epoch = Some(window.header().timeline_epoch);
            previous_sequence = window.meta().sequence;
        }
        drop(compiled);
        drop(preparation);
        assert_eq!(topic.status().publisher, PublisherStatus::NoPublisher);
        assert_eq!(topic.business_subscribers(), 1);
    }
}
