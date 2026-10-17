use super::*;
use crate::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};
use moiren_signal::{BlockInit, Bus, OpSignalBindings, PumpBudget, RtHost, Topic};
use std::{cell::Cell, f64::consts::TAU};

thread_local! {
    static CONVERSIONS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Copy)]
struct CountedSample(f64);

impl ProcessingSample for CountedSample {
    const ZERO: Self = Self(0.0);

    fn from_f64(value: f64) -> Self {
        Self(value)
    }

    fn to_f64(self) -> f64 {
        CONVERSIONS.with(|count| count.set(count.get() + 1));
        self.0
    }
}

type SpectrumTopic = Topic<LatestBlock<SpectrumWindow, f32>>;

fn prepare(
    capacity: usize,
) -> (
    Bus,
    SpectrumTopic,
    RtHost,
    OpSignalBindings,
    PreparedPublisher<LatestBlock<SpectrumWindow, f32>>,
) {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_latest_block(
            "spectrum/module",
            BlockInit {
                header: SpectrumWindow {
                    timeline_epoch: 0,
                    frame_start: 0,
                    frame_end: 0,
                    sample_rate: 0.0,
                    fft_size: 0,
                    channels: 0,
                },
                element: 0.0,
                capacity,
            },
        )
        .unwrap();
    let (control, mut host) = bus.create_host(2).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let update = control.prepare_update(&[publisher.erase()]).unwrap();
    let op = update.op_bindings(key).unwrap();
    drop(host.activate(update).unwrap());
    (bus, topic, host, op, publisher)
}

struct Fixture<S: ProcessingSample> {
    bus: Bus,
    topic: SpectrumTopic,
    host: RtHost,
    op: OpSignalBindings,
    analyzer: SignalSpectrumAnalyzer,
    arena: BufferArena<S>,
    io: PreparedIo,
    write_io: PreparedIo,
}

fn context(start: u64, frames: usize) -> ProcessContext {
    ProcessContext {
        timeline_epoch: 9,
        timeline_start: start,
        frames,
        processing_sr: 48_000.0,
    }
}

fn settings(fft_size: usize, hop_frames: usize) -> SpectrumSettings {
    SpectrumSettings {
        fft_size,
        hop_frames,
    }
}

impl<S: ProcessingSample> Fixture<S> {
    fn new(channels: usize, settings: SpectrumSettings) -> Self {
        let (bus, topic, host, op, publisher) = prepare(channels * settings.bins_per_channel());
        let analyzer = SignalSpectrumAnalyzer::new(channels, settings, publisher).unwrap();
        let arena = BufferArena::new(
            &[BufferSlotLayout {
                channels,
                capacity_frames: settings.fft_size * 3,
            }],
            16 * 1024 * 1024,
        )
        .unwrap();
        let slot = arena.slot(0).unwrap();
        let write_io = arena
            .prepare_io(&[PortAccess::Write { port: 0, slot }])
            .unwrap();
        let io = arena
            .prepare_io(&[PortAccess::Read { port: 0, slot }])
            .unwrap();
        RtObserver::<S>::validate_inputs(&analyzer, &io).unwrap();
        Self {
            bus,
            topic,
            host,
            op,
            analyzer,
            arena,
            io,
            write_io,
        }
    }

    fn fill(&mut self, frames: usize, sample: impl Fn(usize, usize) -> f64) {
        self.arena
            .with_io(&self.write_io, 0..frames, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    panic!("expected writable fixture audio");
                };
                let mut output = outputs.get_mut(0).unwrap();
                for channel in 0..output.channel_count() {
                    for (frame, value) in output.channel_mut(channel).iter_mut().enumerate() {
                        *value = S::from_f64(sample(channel, frame));
                    }
                }
            })
            .unwrap();
    }

    fn observe_rt(&mut self, ctx: ProcessContext) {
        self.host.begin_block();
        self.arena
            .with_io(&self.io, 0..ctx.frames, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    panic!("expected read-only fixture audio");
                };
                self.analyzer
                    .observe(&ctx, inputs, self.host.scope(&self.op).into());
            })
            .unwrap();
    }

    fn observe(&mut self, ctx: ProcessContext) {
        self.observe_rt(ctx);
        self.bus.pump(PumpBudget {
            topic_checks: 1024,
            deliveries: 128,
            copy_bytes: 1024 * 1024,
        });
    }
}

fn assert_near(actual: f32, expected: f64) {
    assert!(
        (f64::from(actual) - expected).abs() < 2e-6,
        "actual {actual}, expected {expected}"
    );
}

// Deliberately direct O(N^2) DFT: no production FFT, window or scaling helpers.
fn reference_amplitudes(samples: &[f64]) -> Vec<f64> {
    let n = samples.len();
    let window: Vec<_> = (0..n)
        .map(|frame| 0.5 - 0.5 * (TAU * frame as f64 / n as f64).cos())
        .collect();
    let coherent_sum: f64 = window.iter().sum();
    (0..=n / 2)
        .map(|bin| {
            let mut re = 0.0;
            let mut im = 0.0;
            for (frame, (&sample, &weight)) in samples.iter().zip(&window).enumerate() {
                let angle = TAU * bin as f64 * frame as f64 / n as f64;
                re += sample * weight * angle.cos();
                im -= sample * weight * angle.sin();
            }
            let sidedness = if bin == 0 || bin == n / 2 { 1.0 } else { 2.0 };
            re.hypot(im) * sidedness / coherent_sum
        })
        .collect()
}

#[test]
fn settings_defaults_bounds_and_bin_count_are_explicit() {
    assert_eq!(SpectrumSettings::default(), settings(1024, 512));
    for fft_size in 0..=16_385 {
        let config = settings(fft_size, 1);
        assert_eq!(
            config.valid(),
            (16..=8192).contains(&fft_size) && fft_size.is_power_of_two()
        );
        assert_eq!(config.bins_per_channel(), fft_size / 2 + 1);
    }
    for fft_size in [16, 32, 1024, 8192] {
        assert!(settings(fft_size, 1).valid());
        assert!(settings(fft_size, fft_size).valid());
        assert!(!settings(fft_size, 0).valid());
        assert!(!settings(fft_size, fft_size + 1).valid());
    }
    assert!(!settings(usize::MAX, 1).valid());
}

#[test]
fn settings_serde_defaults_and_unknown_fields_follow_the_public_contract() {
    use serde::{Deserialize, de::value::MapDeserializer};

    fn require_serialize<T: serde::Serialize>() {}
    require_serialize::<SpectrumSettings>();
    let decode = |entries: Vec<(&str, usize)>| {
        SpectrumSettings::deserialize(MapDeserializer::<_, serde::de::value::Error>::new(
            entries.into_iter(),
        ))
    };
    assert_eq!(decode(vec![]).unwrap(), SpectrumSettings::default());
    assert_eq!(decode(vec![("fft_size", 64)]).unwrap(), settings(64, 512));
    assert_eq!(
        decode(vec![("hop_frames", 16)]).unwrap(),
        settings(1024, 16)
    );
    assert_eq!(
        decode(vec![("fft_size", 64), ("hop_frames", 32)]).unwrap(),
        settings(64, 32)
    );
    assert!(decode(vec![("fft_szie", 64)]).is_err());
}

#[test]
fn invalid_channels_settings_and_publisher_capacity_are_rejected() {
    for (channels, config, capacity, expected) in [
        (0, settings(16, 8), 9, SpectrumError::InvalidChannels),
        (
            usize::MAX,
            settings(16, 8),
            9,
            SpectrumError::InvalidChannels,
        ),
        (1, settings(8, 4), 5, SpectrumError::InvalidSettings),
        (1, settings(17, 8), 9, SpectrumError::InvalidSettings),
        (1, settings(16_384, 8), 9, SpectrumError::InvalidSettings),
        (1, settings(16, 0), 9, SpectrumError::InvalidSettings),
        (1, settings(16, 17), 9, SpectrumError::InvalidSettings),
        (2, settings(16, 8), 17, SpectrumError::CapacityMismatch),
        (2, settings(16, 8), 19, SpectrumError::CapacityMismatch),
    ] {
        let (_bus, _topic, _host, _op, publisher) = prepare(capacity);
        assert_eq!(
            SignalSpectrumAnalyzer::new(channels, config, publisher).err(),
            Some(expected)
        );
    }
}

fn assert_channel_tones<S: ProcessingSample>() {
    let mut fixture = Fixture::<S>::new(3, settings(64, 32));
    let mut subscriber = fixture
        .bus
        .subscribe(&fixture.topic, Default::default())
        .unwrap();
    fixture.fill(64, |channel, frame| match channel {
        0 => 0.8 * (TAU * 7.0 * frame as f64 / 64.0).sin(),
        1 => 0.25 * (TAU * 19.0 * frame as f64 / 64.0).cos(),
        _ => 0.0,
    });
    fixture.observe(context(100, 64));
    let read = subscriber.read_latest().unwrap();
    assert_eq!(
        *read.header(),
        SpectrumWindow {
            timeline_epoch: 9,
            frame_start: 100,
            frame_end: 164,
            sample_rate: 48_000.0,
            fft_size: 64,
            channels: 3,
        }
    );
    assert_eq!(read.elements().len(), 3 * 33);
    for (channel, bins) in read.elements().as_chunks::<33>().0.iter().enumerate() {
        if channel == 2 {
            assert!(bins.iter().all(|&value| value == 0.0));
            continue;
        }
        let peak = bins
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .unwrap()
            .0;
        assert_eq!(peak, if channel == 0 { 7 } else { 19 });
        assert_near(bins[peak], if channel == 0 { 0.8 } else { 0.25 });
        assert_near(bins[if channel == 0 { 19 } else { 7 }], 0.0);
    }
}

#[test]
fn channel_major_peak_bins_and_amplitudes_are_correct_in_both_precisions() {
    assert_channel_tones::<f32>();
    assert_channel_tones::<f64>();
}

#[test]
fn dc_and_nyquist_are_not_doubled() {
    let mut fixture = Fixture::<f64>::new(2, settings(32, 32));
    let mut subscriber = fixture
        .bus
        .subscribe(&fixture.topic, Default::default())
        .unwrap();
    fixture.fill(32, |channel, frame| {
        if channel == 0 || frame % 2 == 0 {
            0.6
        } else {
            -0.6
        }
    });
    fixture.observe(context(0, 32));
    let read = subscriber.read_latest().unwrap();
    assert_near(read.elements()[0], 0.6);
    assert_near(read.elements()[16], 0.0);
    assert_near(read.elements()[17], 0.0);
    assert_near(read.elements()[17 + 16], 0.6);
}

#[test]
fn every_fft_bin_matches_independent_dft() {
    for fft_size in [16, 32, 64, 128, 256] {
        let mut fixture = Fixture::<f64>::new(2, settings(fft_size, fft_size));
        let mut subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        let sample = |channel, frame| {
            let phase = frame as f64;
            let sign = if channel == 0 { 1.0 } else { -1.0 };
            sign * (0.13
                + 0.7 * (phase * 0.937).sin()
                + 0.2 * (phase * 0.123).cos()
                + ((frame * 37 + channel * 11) % 19) as f64 / 57.0)
        };
        fixture.fill(fft_size, sample);
        fixture.observe(context(0, fft_size));
        let read = subscriber.read_latest().unwrap();
        for channel in 0..2 {
            let samples: Vec<_> = (0..fft_size).map(|frame| sample(channel, frame)).collect();
            let expected = reference_amplitudes(&samples);
            for (&actual, expected) in read.elements()
                [channel * expected.len()..(channel + 1) * expected.len()]
                .iter()
                .zip(expected)
            {
                assert_near(actual, expected);
            }
        }
    }
}

#[test]
fn overlapping_windows_keep_exact_source_intervals_and_ring_order() {
    for hop in [1, 5, 8, 16] {
        let mut fixture = Fixture::<f64>::new(1, settings(16, hop));
        let mut subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        let sample = |frame: usize| (frame as f64 * 0.713).sin() + frame as f64 / 64.0;
        let mut start = 41;
        let mut consumed = 0;
        let mut published = 0;
        for frames in [3, 13, 1, 7, 9, 15] {
            fixture.fill(frames, |_, frame| sample(consumed + frame));
            fixture.observe(context(start, frames));
            consumed += frames;
            start += frames as u64;
            let count = if consumed < 16 {
                0
            } else {
                1 + (consumed - 16) / hop
            };
            assert_eq!(fixture.topic.stats().attempts, count as u64);
            if count > published {
                let window_start = (count - 1) * hop;
                let read = subscriber.read_latest().unwrap();
                assert_eq!(read.header().frame_start, 41 + window_start as u64);
                assert_eq!(read.header().frame_end, 41 + window_start as u64 + 16);
                let samples: Vec<_> = (window_start..window_start + 16).map(sample).collect();
                for (&actual, expected) in
                    read.elements().iter().zip(reference_amplitudes(&samples))
                {
                    assert_near(actual, expected);
                }
            } else {
                assert!(subscriber.read_latest().is_none());
            }
            published = count;
        }
    }
}

#[test]
fn epoch_gap_rewind_and_sample_rate_changes_discard_partial_history() {
    for already_published in [false, true] {
        for discontinuity in [
            ProcessContext {
                timeline_epoch: 10,
                ..context(12, 8)
            },
            context(19, 8),
            context(0, 8),
            ProcessContext {
                processing_sr: 96_000.0,
                ..context(12, 8)
            },
        ] {
            let mut fixture = Fixture::<f64>::new(1, settings(16, 4));
            let mut subscriber = fixture
                .bus
                .subscribe(&fixture.topic, Default::default())
                .unwrap();
            fixture.fill(32, |_, _| 1.0);
            if already_published {
                fixture.observe(context(0, 16));
                drop(subscriber.read_latest().unwrap());
            } else {
                fixture.observe(context(0, 12));
            }
            fixture.fill(8, |_, _| 0.25);
            fixture.observe(discontinuity);
            assert!(subscriber.read_latest().is_none());
            fixture.observe(ProcessContext {
                timeline_start: discontinuity.timeline_start + 8,
                ..discontinuity
            });
            let read = subscriber.read_latest().unwrap();
            assert_eq!(read.header().timeline_epoch, discontinuity.timeline_epoch);
            assert_eq!(read.header().frame_start, discontinuity.timeline_start);
            assert_eq!(read.header().frame_end, discontinuity.timeline_start + 16);
            assert_eq!(read.header().sample_rate, discontinuity.processing_sr);
            assert_near(read.elements()[0], 0.25);
        }
    }
}

#[test]
fn inactive_demand_performs_no_sample_conversion_or_publication() {
    let mut fixture = Fixture::<CountedSample>::new(2, settings(16, 8));
    fixture.fill(32, |_, _| 1.0);
    CONVERSIONS.with(|count| count.set(0));
    fixture.observe(context(0, 32));
    assert_eq!(CONVERSIONS.with(Cell::get), 0);
    assert_eq!(fixture.topic.stats().attempts, 0);
    let subscriber = fixture
        .bus
        .subscribe(&fixture.topic, Default::default())
        .unwrap();
    fixture.observe(context(32, 8));
    assert_eq!(CONVERSIONS.with(Cell::get), 16);
    drop(subscriber);
    fixture.observe(context(40, 32));
    assert_eq!(CONVERSIONS.with(Cell::get), 16);
    assert_eq!(fixture.topic.stats().attempts, 0);
}

#[test]
fn demand_generation_change_discards_history_even_without_inactive_observe() {
    for observe_inactive in [false, true] {
        let mut fixture = Fixture::<f64>::new(1, settings(16, 4));
        fixture.fill(32, |_, _| 1.0);
        let subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        fixture.observe(context(0, 12));
        drop(subscriber);
        let mut start = 12;
        if observe_inactive {
            fixture.observe(context(start, 4));
            start += 4;
        }
        let mut subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        fixture.fill(8, |_, _| 0.4);
        fixture.observe(context(start, 8));
        assert!(subscriber.read_latest().is_none());
        fixture.observe(context(start + 8, 8));
        let read = subscriber.read_latest().unwrap();
        assert_eq!(read.header().frame_start, start);
        assert_eq!(read.header().frame_end, start + 16);
        assert_near(read.elements()[0], 0.4);
    }
}

#[test]
fn prepared_observer_requires_exactly_one_input_with_configured_channels() {
    let fixture = Fixture::<f64>::new(2, settings(16, 8));
    let slot = fixture.arena.slot(0).unwrap();
    for ports in [
        vec![],
        vec![PortAccess::Read { port: 1, slot }],
        vec![
            PortAccess::Read { port: 0, slot },
            PortAccess::Read { port: 1, slot },
        ],
    ] {
        let io = fixture.arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtObserver::<f64>::validate_inputs(&fixture.analyzer, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    let arena = BufferArena::<f64>::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    assert_eq!(
        RtObserver::<f64>::validate_inputs(&fixture.analyzer, &io),
        Err(ProcessorError::InvalidIo)
    );
    assert_eq!(
        RtObserver::<f64>::signal_bindings(&fixture.analyzer).len(),
        1
    );
    assert_eq!(
        RtObserver::<f64>::signal_bindings(&fixture.analyzer)[0]
            .schema()
            .2,
        18
    );
}

#[test]
fn maximum_fft_size_is_prepared_and_observed() {
    let mut fixture = Fixture::<f32>::new(1, settings(8192, 8192));
    let mut subscriber = fixture
        .bus
        .subscribe(&fixture.topic, Default::default())
        .unwrap();
    fixture.fill(8192, |_, _| 0.3);
    fixture.observe(context(0, 8192));
    let read = subscriber.read_latest().unwrap();
    assert_eq!(read.elements().len(), 4097);
    assert_near(read.elements()[0], 0.3);
}
