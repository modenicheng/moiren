//! Complete stereo frames cross bounded SPSC rings. Only graph boundaries cast S.
use super::{native::StereoFrame, observations::Counters, resample::StereoResampler};
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSink, RtAudioSource},
    buffer::{AudioBlock, AudioBlockMut},
    host::BackendError,
    processor::ProcessContext,
    sample::ProcessingSample,
};
use rtrb::{Consumer, Producer};
use std::{
    marker::PhantomData,
    sync::{Arc, atomic::Ordering},
};

pub(super) struct RingSink<S: ProcessingSample> {
    pub tx: Producer<StereoFrame>,
    pub counters: Arc<Counters>,
    pub marker: PhantomData<S>,
}
impl<S: ProcessingSample> RtAudioSink<S> for RingSink<S> {
    fn channel_count(&self) -> usize {
        2
    }
    fn write(&mut self, _: &ProcessContext, input: AudioBlock<'_, S>) -> BoundaryReport {
        let count = if self.tx.is_abandoned() {
            0
        } else {
            input.frames().min(self.tx.slots())
        };
        for i in 0..count {
            let frame = [input.channel(0)[i].to_f64(), input.channel(1)[i].to_f64()];
            let _ = self.tx.push(frame);
        }
        let dropped = input.frames() - count;
        self.counters
            .dropped
            .fetch_add(dropped as u64, Ordering::Relaxed);
        report(count, input.frames())
    }
}

pub(super) struct RingSource<S: ProcessingSample> {
    rx: Consumer<StereoFrame>,
    src: StereoResampler,
    target: usize,
    primed: bool,
    correction: f64,
    counters: Arc<Counters>,
    marker: PhantomData<S>,
}

impl<S: ProcessingSample> RingSource<S> {
    pub fn new(
        rx: Consumer<StereoFrame>,
        native_rate: u32,
        engine_rate: u32,
        period: usize,
        capacity: usize,
        counters: Arc<Counters>,
    ) -> Result<Self, BackendError> {
        Ok(Self {
            rx,
            src: StereoResampler::new(native_rate, engine_rate, capacity).map_err(unsupported)?,
            target: (period * 2).max(64).min(capacity / 2),
            primed: false,
            correction: 0.0,
            counters,
            marker: PhantomData,
        })
    }
}
impl<S: ProcessingSample> RtAudioSource<S> for RingSource<S> {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        let queued = self.rx.slots() + self.src.queued_frames();
        if !self.primed && queued >= self.target {
            self.primed = true;
        }
        let desired = ((queued as f64 - self.target as f64) / self.target as f64 * 500.0)
            .clamp(-2000.0, 2000.0);
        self.correction += (desired - self.correction) * 0.01;
        self.src.set_correction_ppm(self.correction);
        let mut transferred = 0;
        for i in 0..output.frames() {
            let frame = if self.primed {
                refill(&mut self.src, &mut self.rx);
                self.src.next_frame()
            } else {
                None
            };
            let frame = match frame {
                Some(frame) => {
                    transferred += 1;
                    frame
                }
                None => {
                    self.primed = false;
                    [0.0; 2]
                }
            };
            output.channel_mut(0)[i] = S::from_f64(frame[0]);
            output.channel_mut(1)[i] = S::from_f64(frame[1]);
        }
        self.counters
            .silent
            .fetch_add((output.frames() - transferred) as u64, Ordering::Relaxed);
        report(transferred, output.frames())
    }
}

pub(super) fn refill(src: &mut StereoResampler, rx: &mut Consumer<StereoFrame>) {
    let count = src.free_frames().min(rx.slots());
    for _ in 0..count {
        if let Ok(frame) = rx.pop() {
            src.push_frame(frame);
        }
    }
}

pub(super) fn report(transferred_frames: usize, requested: usize) -> BoundaryReport {
    BoundaryReport {
        transferred_frames,
        discontinuity: transferred_frames < requested,
        xruns: u64::from(transferred_frames < requested),
    }
}

pub(super) fn unsupported(error: impl std::fmt::Display) -> BackendError {
    BackendError::Unsupported(error.to_string())
}

pub(super) struct Tone {
    frequency: f64,
    phase: f64,
}
impl Tone {
    pub fn new(name: &str, processing_rate: u32) -> Result<Self, BackendError> {
        let frequency = name
            .strip_prefix("moiren:tone:")
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0 && *v < f64::from(processing_rate) * 0.5)
            .ok_or_else(|| unsupported("invalid software tone selector"))?;
        Ok(Self {
            frequency,
            phase: 0.0,
        })
    }
}
impl<S: ProcessingSample> RtAudioSource<S> for Tone {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        let step = self.frequency / ctx.processing_sr;
        for i in 0..output.frames() {
            let value = S::from_f64((self.phase * std::f64::consts::TAU).sin() * 0.1);
            output.channel_mut(0)[i] = value;
            output.channel_mut(1)[i] = value;
            self.phase = (self.phase + step).fract();
        }
        report(output.frames(), output.frames())
    }
}

#[cfg(windows)]
pub(super) struct ProcessSource<S: ProcessingSample> {
    pub source: crate::clock_bridge::ClockSource,
    pub src: StereoResampler,
    pub scratch: Box<[f32]>,
    pub marker: PhantomData<S>,
}
#[cfg(windows)]
impl<S: ProcessingSample> RtAudioSource<S> for ProcessSource<S> {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        let mut discontinuity = false;
        let mut xruns = 0;
        let mut transferred_frames = 0;
        for i in 0..output.frames() {
            let frame = if let Some(frame) = self.src.next_frame() {
                Some(frame)
            } else {
                // This source already follows the master at 48 kHz. Only rate
                // adaptation remains; applying another drift servo would fight it.
                let count = (self.scratch.len() / 2).min(self.src.free_frames());
                let accepted = match self.source.read_interleaved(&mut self.scratch[..count * 2]) {
                    Ok(r) => {
                        discontinuity |= r.discontinuity;
                        xruns += r.xruns;
                        r.transferred_frames.min(count)
                    }
                    Err(_) => {
                        discontinuity = true;
                        xruns += 1;
                        0
                    }
                };
                for frame in self.scratch[..accepted * 2].as_chunks::<2>().0 {
                    self.src.push_frame([frame[0] as f64, frame[1] as f64]);
                }
                self.src.next_frame()
            };
            let frame = match frame {
                Some(frame) => {
                    transferred_frames += 1;
                    frame
                }
                None => {
                    discontinuity = true;
                    [0.0; 2]
                }
            };
            output.channel_mut(0)[i] = S::from_f64(frame[0]);
            output.channel_mut(1)[i] = S::from_f64(frame[1]);
        }
        if transferred_frames < output.frames() {
            xruns = xruns.max(1);
        }
        BoundaryReport {
            transferred_frames,
            discontinuity,
            xruns,
        }
    }
}
