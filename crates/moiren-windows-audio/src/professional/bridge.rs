//! Complete stereo frames cross bounded SPSC rings. Only graph boundaries cast S.
pub(super) use super::servo::FollowerServo;
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
    servo: FollowerServo,
    primed: bool,
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
            servo: FollowerServo::new(FollowerServo::start_target(period, capacity)),
            primed: false,
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
        if !self.primed && queued >= self.servo.target() {
            self.primed = true;
        }
        self.src.set_correction_ppm(self.servo.observe(queued));
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
        for start in (0..output.frames()).step_by(self.scratch.len() / 2) {
            // ClockSource already adapts to Engine rate and follows its master.
            // Read only this invocation's demand: a second FIR and fixed-size
            // prefetch hide queued content from the one drift controller.
            let count = (output.frames() - start).min(self.scratch.len() / 2);
            match self.source.read_interleaved(&mut self.scratch[..count * 2]) {
                Ok(r) => {
                    discontinuity |= r.discontinuity;
                    xruns += r.xruns;
                    transferred_frames += r.transferred_frames.min(count);
                }
                Err(_) => {
                    discontinuity = true;
                    xruns += 1;
                    self.scratch[..count * 2].fill(0.0);
                }
            }
            for i in 0..count {
                output.channel_mut(0)[start + i] = S::from_f64(f64::from(self.scratch[i * 2]));
                output.channel_mut(1)[start + i] = S::from_f64(f64::from(self.scratch[i * 2 + 1]));
            }
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

#[cfg(all(test, windows))]
mod process_tests {
    use super::*;
    use crate::clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge};

    #[test]
    fn process_read_does_not_prefetch_an_extra_graph_block_or_filter_twice() {
        let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig {
            target_fill_frames: 320,
            trim_on_prime: false,
            max_correction_ppm: 0.0,
            detect_position_gaps: false,
            ..Default::default()
        })
        .unwrap();
        let pcm: Vec<u8> = (0..1024)
            .flat_map(|frame| {
                let value = frame as f32 / 4096.0;
                [value, -value].into_iter().flat_map(f32::to_le_bytes)
            })
            .collect();
        ingress
            .push_packet(
                &pcm,
                CapturePacket {
                    frames: 1024,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 1,
                },
            )
            .unwrap();
        let source = ProcessSource::<f64> {
            source,
            scratch: vec![0.0; 512].into_boxed_slice(),
            marker: PhantomData,
        };
        let mut graph = moiren_core::graph::LogicalGraph::new();
        let input = graph
            .create_node(moiren_core::graph::NodeKind::Source, 2)
            .unwrap();
        let output = graph
            .create_node(moiren_core::graph::NodeKind::Sink, 2)
            .unwrap();
        graph
            .connect(
                graph.get_node(input).unwrap().outputs()[0].id(),
                graph.get_node(output).unwrap().inputs()[0].id(),
                Default::default(),
            )
            .unwrap();
        let (writer, mut reader) =
            moiren_engine::boundary::audio_bridge::<f64>(2, 256, 4096).unwrap();
        let mut bindings = moiren_engine::compiler::NodeBindings::new();
        bindings.bind_source(input, source).unwrap();
        bindings.bind_sink(output, writer).unwrap();
        let mut processing = moiren_engine::host::EngineProject::default().processing;
        processing.engine.max_block_frames = 64;
        let mut compiled =
            moiren_engine::compiler::compile(&graph, bindings, processing.compile_config(1, 1))
                .unwrap();
        compiled.engine.render(64).unwrap();
        let mut samples = [0.0f64; 128];
        assert_eq!(
            reader
                .read_interleaved(&mut samples)
                .unwrap()
                .transferred_frames,
            64
        );
        assert_eq!(observer.snapshot().output_frames, 64);
        for frame in 0..64 {
            assert_eq!(samples[frame * 2], frame as f64 / 4096.0);
            assert_eq!(samples[frame * 2 + 1], -(frame as f64 / 4096.0));
        }
    }
}
