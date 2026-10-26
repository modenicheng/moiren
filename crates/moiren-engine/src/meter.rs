//! Read-only sample-peak/RMS example, NOT a BS.1770/LUFS or true-peak meter.
//! The integration window delays the display, never the audio signal.
use crate::{
    buffer::{PreparedIo, ReadPorts},
    processor::{ProcessContext, ProcessorError, RtObserver},
    sample::ProcessingSample,
};
use rtrb::{Consumer, Producer, RingBuffer};

mod polling;
pub mod signal;
pub use polling::MeterPollingTimer;

/// Default explicit Meter-node integration window: 480 frames is 10 ms at
/// 48 kHz. Implicit UI observers use [`ui_window_frames`] instead. Analysis
/// windows delay observations, never audio.
pub const DEFAULT_WINDOW_FRAMES: usize = 480;

/// Implicit UI observers target at least 240 analysis windows per audio second.
/// Derive this off the audio thread from the validated processing rate; explicit
/// Meter-node settings keep their chosen integration interval instead.
pub fn ui_window_frames(processing_sample_rate: f64) -> usize {
    ((processing_sample_rate / 240.0).floor() as usize).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MeterSettings {
    pub window_frames: usize,
}
impl Default for MeterSettings {
    fn default() -> Self {
        Self {
            window_frames: DEFAULT_WINDOW_FRAMES,
        }
    }
}
impl MeterSettings {
    pub fn valid(self) -> bool {
        (1..=1_048_576).contains(&self.window_frames)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LevelSnapshot {
    pub timeline_epoch: u64,
    pub end_frame: u64,
    pub channels: usize,
    pub samples: u64,
    pub peak: f64,
    pub rms: f64,
    pub dropped_snapshots: u64,
    pub non_finite_samples: u64,
}
/// Queue-backed meter: fixed snapshots cross to control through a bounded
/// rtrb queue that is drained on demand, with no signal-bus participation.
/// This is the older variant retained for the queue-based `NodeConfig::Meter`
/// path; its continuity tracking is epoch + next frame only (no activation
/// generation or sample-rate latch, both of which are meaningless without a
/// demand-gated publisher). Demand-gated exact-window measurement lives in
/// [`signal::SignalLevelMeter`]; do not add new behavior here.
pub struct LevelMeter {
    tx: Producer<LevelSnapshot>,
    interval_frames: u64,
    frames: u64,
    samples: u64,
    peak: f64,
    squares: f64,
    dropped: u64,
    non_finite: u64,
    timeline_epoch: u64,
    next_frame: Option<u64>,
}
pub struct MeterReader {
    rx: Consumer<LevelSnapshot>,
}

pub fn level_meter(
    interval_frames: u64,
    capacity: usize,
) -> Result<(LevelMeter, MeterReader), ProcessorError> {
    if interval_frames == 0 || capacity == 0 {
        return Err(ProcessorError::InvalidIo);
    }
    let (tx, rx) = RingBuffer::new(capacity);
    Ok((
        LevelMeter {
            tx,
            interval_frames,
            frames: 0,
            samples: 0,
            peak: 0.0,
            squares: 0.0,
            dropped: 0,
            non_finite: 0,
            timeline_epoch: 0,
            next_frame: None,
        },
        MeterReader { rx },
    ))
}
impl MeterReader {
    /// Drain only the initial queue snapshot; never spin chasing a live producer.
    pub fn latest(&mut self) -> Option<LevelSnapshot> {
        let count = self.rx.slots();
        let mut latest = None;
        for _ in 0..count {
            if let Ok(value) = self.rx.pop() {
                latest = Some(value);
            }
        }
        latest
    }
}
impl<S: ProcessingSample> RtObserver<S> for LevelMeter {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.input_count() == 1 && io.input_channels(0).is_some() {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }
    fn observe(
        &mut self,
        ctx: &ProcessContext,
        inputs: ReadPorts<'_, S>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        let input = inputs.get(0).expect("validated observer input");
        if self.next_frame.is_some()
            && (self.timeline_epoch != ctx.timeline_epoch
                || self.next_frame != Some(ctx.timeline_start))
        {
            self.frames = 0;
            self.samples = 0;
            self.peak = 0.0;
            self.squares = 0.0;
            self.non_finite = 0;
        }
        self.timeline_epoch = ctx.timeline_epoch;
        for frame in 0..ctx.frames {
            for channel in 0..input.channel_count() {
                let sample = input.channel(channel)[frame].to_f64();
                if !sample.is_finite() {
                    self.non_finite = self.non_finite.saturating_add(1);
                    continue;
                }
                self.peak = self.peak.max(sample.abs());
                self.squares += sample * sample;
                self.samples = self.samples.saturating_add(1);
            }
            self.frames = self.frames.saturating_add(1);
            if self.frames == self.interval_frames {
                let snapshot = LevelSnapshot {
                    timeline_epoch: ctx.timeline_epoch,
                    end_frame: ctx.timeline_start + frame as u64 + 1,
                    channels: input.channel_count(),
                    samples: self.samples,
                    peak: self.peak,
                    rms: if self.samples == 0 {
                        0.0
                    } else {
                        (self.squares / self.samples as f64).sqrt()
                    },
                    dropped_snapshots: self.dropped,
                    non_finite_samples: self.non_finite,
                };
                if self.tx.push(snapshot).is_err() {
                    self.dropped = self.dropped.saturating_add(1);
                }
                self.frames = 0;
                self.samples = 0;
                self.peak = 0.0;
                self.squares = 0.0;
                self.non_finite = 0;
            }
        }
        self.next_frame = Some(ctx.timeline_start.saturating_add(ctx.frames as u64));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn implicit_ui_windows_floor_the_rate_with_at_least_one_frame() {
        for (rate, frames) in [
            (1.0, 1),
            (239.0, 1),
            (240.0, 1),
            (44_100.0, 183),
            (48_000.0, 200),
            (96_000.0, 400),
        ] {
            assert_eq!(super::ui_window_frames(rate), frames);
        }
        assert_eq!(super::MeterSettings::default().window_frames, 480);
    }

    use super::*;
    use crate::{
        buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
        processor::ProcessContext,
    };

    #[test]
    fn interval_splits_a_larger_audio_block_and_keeps_exact_window_size() {
        let mut arena = BufferArena::<f64>::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 8,
            }],
            1024,
        )
        .unwrap();
        let slot = arena.slot(0).unwrap();
        let write = arena
            .prepare_io(&[PortAccess::Write { port: 0, slot }])
            .unwrap();
        arena
            .with_io(&write, 0..8, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    panic!()
                };
                outputs.get_mut(0).unwrap().channel_mut(0).fill(1.0);
            })
            .unwrap();
        let read = arena
            .prepare_io(&[PortAccess::Read { port: 0, slot }])
            .unwrap();
        let (mut meter, mut reader) = level_meter(4, 4).unwrap();
        let context = ProcessContext {
            timeline_epoch: 3,
            timeline_start: 10,
            frames: 8,
            processing_sr: 48_000.0,
        };
        arena
            .with_io(&read, 0..8, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    panic!()
                };
                meter.observe(
                    &context,
                    inputs,
                    crate::signal::ProcessSignals::empty(&mut ()),
                );
            })
            .unwrap();

        let first = reader.rx.pop().unwrap();
        let second = reader.rx.pop().unwrap();
        for (snapshot, end_frame) in [(first, 14), (second, 18)] {
            assert_eq!(snapshot.samples, 4);
            assert_eq!(snapshot.end_frame, end_frame);
            assert_eq!(snapshot.peak, 1.0);
            assert_eq!(snapshot.rms, 1.0);
        }
    }

    #[test]
    fn partial_windows_continue_across_contiguous_blocks() {
        let mut arena = BufferArena::<f64>::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 4,
            }],
            1024,
        )
        .unwrap();
        let slot = arena.slot(0).unwrap();
        let write = arena
            .prepare_io(&[PortAccess::Write { port: 0, slot }])
            .unwrap();
        arena
            .with_io(&write, 0..4, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    panic!()
                };
                outputs.get_mut(0).unwrap().channel_mut(0).fill(1.0);
            })
            .unwrap();
        let read = arena
            .prepare_io(&[PortAccess::Read { port: 0, slot }])
            .unwrap();
        let (mut meter, mut reader) = level_meter(4, 4).unwrap();
        let observe = |meter: &mut LevelMeter,
                       arena: &mut BufferArena<f64>,
                       read: &PreparedIo,
                       start: u64,
                       frames: usize| {
            let context = ProcessContext {
                timeline_epoch: 3,
                timeline_start: start,
                frames,
                processing_sr: 48_000.0,
            };
            arena
                .with_io(read, 0..frames, |io| {
                    let ProcessIo::ReadOnly { inputs } = io else {
                        panic!()
                    };
                    meter.observe(
                        &context,
                        inputs,
                        crate::signal::ProcessSignals::empty(&mut ()),
                    );
                })
                .unwrap();
        };

        observe(&mut meter, &mut arena, &read, 10, 3);
        assert!(reader.rx.pop().is_err());
        observe(&mut meter, &mut arena, &read, 13, 3);
        assert_eq!(reader.rx.pop().unwrap().end_frame, 14);
        observe(&mut meter, &mut arena, &read, 16, 2);
        assert_eq!(reader.rx.pop().unwrap().end_frame, 18);
    }

    #[test]
    fn timeline_discontinuity_discards_partial_window_before_resuming() {
        let mut arena = BufferArena::<f64>::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 4,
            }],
            1024,
        )
        .unwrap();
        let slot = arena.slot(0).unwrap();
        let write = arena
            .prepare_io(&[PortAccess::Write { port: 0, slot }])
            .unwrap();
        arena
            .with_io(&write, 0..4, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    panic!()
                };
                outputs.get_mut(0).unwrap().channel_mut(0).fill(1.0);
            })
            .unwrap();
        let read = arena
            .prepare_io(&[PortAccess::Read { port: 0, slot }])
            .unwrap();
        let (mut meter, mut reader) = level_meter(4, 4).unwrap();
        let observe = |meter: &mut LevelMeter,
                       arena: &mut BufferArena<f64>,
                       read: &PreparedIo,
                       epoch: u64,
                       start: u64| {
            let context = ProcessContext {
                timeline_epoch: epoch,
                timeline_start: start,
                frames: 2,
                processing_sr: 48_000.0,
            };
            arena
                .with_io(read, 0..2, |io| {
                    let ProcessIo::ReadOnly { inputs } = io else {
                        panic!()
                    };
                    meter.observe(
                        &context,
                        inputs,
                        crate::signal::ProcessSignals::empty(&mut ()),
                    );
                })
                .unwrap();
        };

        observe(&mut meter, &mut arena, &read, 3, 10);
        observe(&mut meter, &mut arena, &read, 3, 20);
        assert!(reader.rx.pop().is_err());
        observe(&mut meter, &mut arena, &read, 4, 100);
        assert!(reader.rx.pop().is_err());
        observe(&mut meter, &mut arena, &read, 4, 102);
        let snapshot = reader.rx.pop().unwrap();
        assert_eq!(snapshot.timeline_epoch, 4);
        assert_eq!(snapshot.end_frame, 104);
        assert_eq!(snapshot.samples, 4);
        assert!(reader.rx.pop().is_err());
    }
}
