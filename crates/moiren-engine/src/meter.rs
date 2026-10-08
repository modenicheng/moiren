//! Read-only sample-peak/RMS example, NOT a BS.1770/LUFS or true-peak meter.
//! The integration window delays the display, never the audio signal.
use crate::{
    buffer::{PreparedIo, ReadPorts},
    processor::{ProcessContext, ProcessorError, RtObserver},
    sample::ProcessingSample,
};
use rtrb::{Consumer, Producer, RingBuffer};

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
pub struct LevelMeter {
    tx: Producer<LevelSnapshot>,
    interval_frames: u64,
    frames: u64,
    samples: u64,
    peak: f64,
    squares: f64,
    dropped: u64,
    non_finite: u64,
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
    fn observe(&mut self, ctx: &ProcessContext, inputs: ReadPorts<'_, S>) {
        let input = inputs.get(0).expect("validated observer input");
        for channel in input.channels() {
            for sample in channel {
                let sample = sample.to_f64();
                if !sample.is_finite() {
                    self.non_finite = self.non_finite.saturating_add(1);
                    continue;
                }
                self.peak = self.peak.max(sample.abs());
                self.squares += sample * sample;
                self.samples = self.samples.saturating_add(1);
            }
        }
        self.frames = self.frames.saturating_add(ctx.frames as u64);
        if self.frames >= self.interval_frames {
            let snapshot = LevelSnapshot {
                timeline_epoch: ctx.timeline_epoch,
                end_frame: ctx.timeline_start + ctx.frames as u64,
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
}
