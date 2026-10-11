//! Demand-gated sample peak/RMS over exact, contiguous source-frame windows.
use crate::{
    buffer::{PreparedIo, ReadPorts},
    processor::{ProcessContext, ProcessorError, RtObserver},
    sample::ProcessingSample,
    signal::ProcessSignals,
};
use moiren_signal::{ErasedBinding, LatestBlock, PreparedPublisher, PublisherHandle};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioWindow {
    pub timeline_epoch: u64,
    pub frame_start: u64,
    pub frame_end: u64,
    pub sample_rate: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelLevel {
    pub peak: f32,
    pub rms: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MeterError {
    #[error("channel count must be nonzero")]
    InvalidChannels,
    #[error("window frame count must be nonzero")]
    InvalidWindow,
    #[error("topic capacity must equal the channel count")]
    CapacityMismatch,
}

/// Read-only per-channel measurement with all accumulation storage prepared off RT.
pub struct SignalLevelMeter {
    channels: usize,
    window_frames: usize,
    accumulated_frames: usize,
    frame_start: u64,
    generation: u64,
    timeline_epoch: u64,
    next_frame: u64,
    sample_rate: f64,
    peaks: Box<[f64]>,
    squares: Box<[f64]>,
    publisher: PublisherHandle<LatestBlock<AudioWindow, ChannelLevel>>,
    bindings: Box<[ErasedBinding]>,
}

impl SignalLevelMeter {
    /// Requires a nonzero window and a topic with exactly one element per channel.
    pub fn new(
        channels: usize,
        window_frames: usize,
        publisher: PreparedPublisher<LatestBlock<AudioWindow, ChannelLevel>>,
    ) -> Result<Self, MeterError> {
        if channels == 0 {
            return Err(MeterError::InvalidChannels);
        }
        if window_frames == 0 {
            return Err(MeterError::InvalidWindow);
        }
        if publisher.capacity() != channels {
            return Err(MeterError::CapacityMismatch);
        }
        Ok(Self {
            channels,
            window_frames,
            accumulated_frames: 0,
            frame_start: 0,
            generation: 0,
            timeline_epoch: 0,
            next_frame: 0,
            sample_rate: 0.0,
            peaks: vec![0.0; channels].into_boxed_slice(),
            squares: vec![0.0; channels].into_boxed_slice(),
            publisher: publisher.handle(),
            bindings: vec![publisher.erase()].into_boxed_slice(),
        })
    }

    fn reset_window(&mut self) {
        self.accumulated_frames = 0;
        self.peaks.fill(0.0);
        self.squares.fill(0.0);
    }
}

impl<S: ProcessingSample> RtObserver<S> for SignalLevelMeter {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() == 1 && io.input_channels(0) == Some(self.channels) {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }

    fn signal_bindings(&self) -> &[ErasedBinding] {
        &self.bindings
    }

    fn observe(
        &mut self,
        ctx: &ProcessContext,
        inputs: ReadPorts<'_, S>,
        mut signals: ProcessSignals<'_>,
    ) {
        let Ok(demand) = signals.demand(&self.publisher) else {
            return;
        };
        if !demand.active {
            return;
        }
        if self.generation != demand.generation
            || self.timeline_epoch != ctx.timeline_epoch
            || self.next_frame != ctx.timeline_start
            || self.sample_rate != ctx.processing_sr
        {
            self.reset_window();
        }
        self.generation = demand.generation;
        self.timeline_epoch = ctx.timeline_epoch;
        self.sample_rate = ctx.processing_sr;
        let input = inputs.get(0).expect("validated signal meter input");
        for frame in 0..ctx.frames {
            if self.accumulated_frames == 0 {
                self.frame_start = ctx.timeline_start + frame as u64;
            }
            for channel in 0..self.channels {
                let sample = input.channel(channel)[frame].to_f64();
                self.peaks[channel] = self.peaks[channel].max(sample.abs());
                self.squares[channel] += sample * sample;
            }
            self.accumulated_frames += 1;
            if self.accumulated_frames == self.window_frames {
                if let Ok(mut write) = signals.try_write(&self.publisher, self.channels) {
                    *write.header_mut() = AudioWindow {
                        timeline_epoch: ctx.timeline_epoch,
                        frame_start: self.frame_start,
                        frame_end: ctx.timeline_start + frame as u64 + 1,
                        sample_rate: ctx.processing_sr,
                    };
                    for (channel, level) in write.elements_mut().iter_mut().enumerate() {
                        *level = ChannelLevel {
                            peak: self.peaks[channel] as f32,
                            rms: (self.squares[channel] / self.window_frames as f64).sqrt() as f32,
                        };
                    }
                    let _ = write.commit(self.channels);
                }
                self.reset_window();
            }
        }
        self.next_frame = ctx.timeline_start + ctx.frames as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{BufferArena, BufferSlotLayout, PortAccess, PreparedIo, ProcessIo};
    use moiren_signal::{BlockInit, Bus, OpSignalBindings, PumpBudget, RtHost, Topic};
    use std::cell::Cell;

    thread_local! { static CONVERSIONS: Cell<usize> = const { Cell::new(0) }; }

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

    struct Fixture {
        bus: Bus,
        topic: Topic<LatestBlock<AudioWindow, ChannelLevel>>,
        host: RtHost,
        op: OpSignalBindings,
        meter: SignalLevelMeter,
        arena: BufferArena<CountedSample>,
        io: PreparedIo,
    }

    fn context(start: u64, frames: usize) -> ProcessContext {
        ProcessContext {
            timeline_epoch: 9,
            timeline_start: start,
            frames,
            processing_sr: 44_100.0,
        }
    }

    impl Fixture {
        fn new(window_frames: usize) -> Self {
            let mut bus = Bus::new(Default::default()).unwrap();
            let topic = bus
                .register_latest_block(
                    "meter/module",
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
                .unwrap();
            let (control, mut host) = bus.create_host(2).unwrap();
            let key = control.new_binding().unwrap();
            let publisher = control.prepare(&topic, key).unwrap();
            let update = control.prepare_update(&[publisher.erase()]).unwrap();
            let op = update.op_bindings(key).unwrap();
            drop(host.activate(update).unwrap());
            let meter = SignalLevelMeter::new(2, window_frames, publisher).unwrap();
            let mut arena = BufferArena::new(
                &[BufferSlotLayout {
                    channels: 2,
                    capacity_frames: 16,
                }],
                4096,
            )
            .unwrap();
            let slot = arena.slot(0).unwrap();
            let write = arena
                .prepare_io(&[PortAccess::Write { port: 0, slot }])
                .unwrap();
            arena
                .with_io(&write, 0..16, |io| {
                    let ProcessIo::Separate { mut outputs, .. } = io else {
                        panic!()
                    };
                    let mut block = outputs.get_mut(0).unwrap();
                    block.channel_mut(0).fill(CountedSample(1.0));
                    block.channel_mut(1).fill(CountedSample(0.5));
                })
                .unwrap();
            let io = arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .unwrap();
            Self {
                bus,
                topic,
                host,
                op,
                meter,
                arena,
                io,
            }
        }

        fn observe(&mut self, ctx: ProcessContext) {
            self.host.begin_block();
            self.arena
                .with_io(&self.io, 0..ctx.frames, |io| {
                    let ProcessIo::ReadOnly { inputs } = io else {
                        panic!()
                    };
                    self.meter
                        .observe(&ctx, inputs, self.host.scope(&self.op).into());
                })
                .unwrap();
            self.bus.pump(PumpBudget {
                topic_checks: 1024,
                deliveries: 128,
                copy_bytes: 65536,
            });
        }
    }

    #[test]
    fn inactive_demand_performs_zero_conversions_and_preserves_partial_state() {
        let mut fixture = Fixture::new(4);
        CONVERSIONS.with(|count| count.set(0));
        fixture.observe(context(0, 2));
        assert_eq!(CONVERSIONS.with(Cell::get), 0);
        assert_eq!(fixture.meter.accumulated_frames, 0);
        let subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        fixture.observe(context(2, 2));
        assert_eq!(CONVERSIONS.with(Cell::get), 4);
        assert_eq!(fixture.meter.accumulated_frames, 2);
        drop(subscriber);
        fixture.observe(context(4, 2));
        assert_eq!(CONVERSIONS.with(Cell::get), 4);
        assert_eq!(fixture.meter.accumulated_frames, 2);
        assert_eq!(&*fixture.meter.peaks, &[1.0, 0.5]);
        assert_eq!(&*fixture.meter.squares, &[2.0, 0.5]);
        assert_eq!(fixture.topic.stats().attempts, 0);
    }

    #[test]
    fn changed_activation_generation_discards_partial_window_even_without_inactive_render() {
        let mut fixture = Fixture::new(4);
        let subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        fixture.observe(context(0, 2));
        let generation = fixture.meter.generation;
        drop(subscriber);
        let mut subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        fixture.observe(context(2, 2));
        assert!(fixture.meter.generation > generation);
        assert!(subscriber.read_latest().is_none());
        fixture.observe(context(4, 2));
        let read = subscriber.read_latest().unwrap();
        assert_eq!(read.header().frame_start, 2);
        assert_eq!(read.header().frame_end, 6);
    }

    #[test]
    fn epoch_frame_gap_rewind_and_rate_changes_restart_only_measurement() {
        for discontinuity in [
            ProcessContext {
                timeline_epoch: 10,
                ..context(2, 2)
            },
            context(7, 2),
            context(0, 2),
            ProcessContext {
                processing_sr: 96_000.0,
                ..context(2, 2)
            },
        ] {
            let mut fixture = Fixture::new(4);
            let mut subscriber = fixture
                .bus
                .subscribe(&fixture.topic, Default::default())
                .unwrap();
            fixture.observe(context(0, 2));
            fixture.observe(discontinuity);
            assert!(subscriber.read_latest().is_none());
            fixture.observe(ProcessContext {
                timeline_start: discontinuity.timeline_start + 2,
                ..discontinuity
            });
            let read = subscriber.read_latest().unwrap();
            assert_eq!(
                *read.header(),
                AudioWindow {
                    timeline_epoch: discontinuity.timeline_epoch,
                    frame_start: discontinuity.timeline_start,
                    frame_end: discontinuity.timeline_start + 4,
                    sample_rate: discontinuity.processing_sr,
                }
            );
        }
    }

    #[test]
    fn multiple_exact_windows_publish_and_contiguous_partial_tail_is_retained() {
        let mut fixture = Fixture::new(4);
        let mut subscriber = fixture
            .bus
            .subscribe(&fixture.topic, Default::default())
            .unwrap();
        fixture.observe(context(11, 10));
        assert_eq!(fixture.topic.stats().attempts, 2);
        assert_eq!(fixture.meter.accumulated_frames, 2);
        let read = subscriber.read_latest().unwrap();
        assert_eq!(read.header().frame_start, 15);
        assert_eq!(read.header().frame_end, 19);
        drop(read);
        fixture.observe(context(21, 2));
        assert_eq!(fixture.topic.stats().attempts, 3);
        let read = subscriber.read_latest().unwrap();
        assert_eq!(
            *read.header(),
            AudioWindow {
                timeline_epoch: 9,
                frame_start: 19,
                frame_end: 23,
                sample_rate: 44_100.0
            }
        );
        assert_eq!(
            read.elements(),
            &[
                ChannelLevel {
                    peak: 1.0,
                    rms: 1.0
                },
                ChannelLevel {
                    peak: 0.5,
                    rms: 0.5
                }
            ]
        );
    }

    #[test]
    fn prepared_observer_rejects_wrong_channels_or_input_number() {
        let fixture = Fixture::new(4);
        let arena = BufferArena::<f64>::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 4,
            }],
            4096,
        )
        .unwrap();
        for ports in [
            vec![],
            vec![PortAccess::Read {
                port: 1,
                slot: fixture.arena.slot(0).unwrap(),
            }],
        ] {
            let io = fixture.arena.prepare_io(&ports).unwrap();
            assert_eq!(
                RtObserver::<f64>::validate_inputs(&fixture.meter, &io),
                Err(ProcessorError::InvalidIo)
            );
        }
        let io = arena
            .prepare_io(&[PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            }])
            .unwrap();
        assert_eq!(
            RtObserver::<f64>::validate_inputs(&fixture.meter, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
}
