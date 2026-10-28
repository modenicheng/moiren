//! Complete stereo frames cross bounded SPSC rings. Only graph boundaries cast S.
use crate::clock_bridge::RateClock;
use crate::engine_backend::observations::Counters;
use crate::latency::QpcClock;
pub(super) use crate::servo::FollowerServo;
use crate::{native::StereoFrame, resample::StereoResampler};
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

/// Live capture older than this is discarded rather than played as a burst.
const STALE_CAPTURE_100NS: u64 = 2_500_000;
/// Linear fade-in length after a recovery, matching the clock-bridge policy.
const RECOVERY_FADE_FRAMES: usize = 32;

/// Cumulative capture position with the QPC of its latest native read. The
/// single native writer publishes once per wake; graph-side readers snapshot
/// with the same bounded even-sequence protocol as the observation counters.
#[derive(Default)]
pub(super) struct CaptureWriteClock {
    sequence: std::sync::atomic::AtomicU64,
    frames: std::sync::atomic::AtomicU64,
    qpc_100ns: std::sync::atomic::AtomicU64,
}

impl CaptureWriteClock {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn publish(&self, frames: u64, qpc_100ns: u64) {
        self.sequence.fetch_add(1, Ordering::SeqCst);
        self.frames.store(frames, Ordering::Relaxed);
        self.qpc_100ns.store(qpc_100ns, Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    fn snapshot(&self) -> Option<(u64, u64)> {
        let before = self.sequence.load(Ordering::SeqCst);
        if before.is_multiple_of(2) {
            let frames = self.frames.load(Ordering::Relaxed);
            let qpc_100ns = self.qpc_100ns.load(Ordering::Relaxed);
            if self.sequence.load(Ordering::SeqCst) == before {
                return Some((frames, qpc_100ns));
            }
        }
        None
    }
}

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
    fade: usize,
    backlog_watermark: usize,
    clock: Arc<CaptureWriteClock>,
    rate: RateClock,
    last_stamp: Option<(u64, u64)>,
    /// Elapsed time that must pass before a write gap counts as a stall; a
    /// few periods of scheduling jitter must not discard live audio.
    min_gap_elapsed_100ns: u64,
    qpc: Option<QpcClock>,
    native_rate: u32,
    last_correction_ppm: f64,
    counters: Arc<Counters>,
    marker: PhantomData<S>,
}

impl<S: ProcessingSample> RingSource<S> {
    pub fn with_input_queue(mut self, queue: crate::latency::PreparedInputQueue) -> Self {
        self.servo = FollowerServo::new(queue.servo_reference_frames as usize);
        self.backlog_watermark = queue.high_water_frames as usize;
        self
    }

    pub fn new(
        rx: Consumer<StereoFrame>,
        native_rate: u32,
        engine_rate: u32,
        period: usize,
        capacity: usize,
        counters: Arc<Counters>,
        clock: Arc<CaptureWriteClock>,
    ) -> Result<Self, BackendError> {
        let target = FollowerServo::start_target(period, capacity);
        Ok(Self {
            rx,
            src: StereoResampler::new(native_rate, engine_rate, capacity).map_err(unsupported)?,
            servo: FollowerServo::new(target),
            primed: false,
            fade: 0,
            backlog_watermark: (target * 3)
                .min(capacity / 2)
                .max(target + target.div_ceil(2)),
            clock,
            rate: RateClock::default(),
            last_stamp: None,
            min_gap_elapsed_100ns: u64::from(period as u32) * 40_000_000 / u64::from(native_rate),
            qpc: QpcClock::new(),
            native_rate,
            last_correction_ppm: 0.0,
            counters,
            marker: PhantomData,
        })
    }

    /// Combined resampler correction in ppm: capture/QPC feed-forward plus the
    /// occupancy follower. Exposed for tests and probes only.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn correction_ppm(&self) -> f64 {
        self.last_correction_ppm
    }

    /// Feed-forward drift estimate from the native write clock. Stamps must be
    /// strictly increasing; a backwards observation re-anchors from scratch.
    fn observe_rate(&mut self, stamp: Option<(u64, u64)>) -> f64 {
        let Some((frames, qpc_100ns)) = stamp.filter(|&(_, qpc)| qpc != 0) else {
            return 0.0;
        };
        let advanced = self
            .last_stamp
            .is_some_and(|(last_frames, last_qpc)| last_frames < frames && last_qpc < qpc_100ns);
        if !advanced {
            self.rate = RateClock::default();
            self.last_stamp = Some((frames, qpc_100ns));
            return 0.0;
        }
        self.last_stamp = Some((frames, qpc_100ns));
        self.rate
            .observe(frames, qpc_100ns, self.native_rate)
            .map_or(0.0, |ppm| ppm as f64)
    }

    /// Backlog, stall and gap recovery. A sustained queue far above the servo
    /// reference is discarded once with a fade-in; a stalled or gapped native
    /// writer invalidates the whole backlog so resumed audio stays live.
    fn recover(&mut self, stamp: Option<(u64, u64)>) {
        if !self.primed {
            return;
        }
        let queued = self.rx.slots() + self.src.queued_frames();
        if queued > self.backlog_watermark {
            self.discard_backlog(queued - self.servo.target());
            return;
        }
        let Some((frames, qpc_100ns)) = stamp.filter(|&(_, qpc)| qpc != 0) else {
            return;
        };
        let now = self.qpc.as_ref().and_then(|clock| clock.now());
        if queued > self.servo.target()
            && now.is_some_and(|now| now.saturating_sub(qpc_100ns) > STALE_CAPTURE_100NS)
        {
            self.discard_backlog(queued - self.servo.target());
            return;
        }
        if let Some((last_frames, last_qpc)) = self.last_stamp
            && last_frames < frames
            && last_qpc < qpc_100ns
        {
            let elapsed = qpc_100ns - last_qpc;
            if elapsed >= self.min_gap_elapsed_100ns
                && u128::from(frames - last_frames) * 20_000_000
                    < u128::from(elapsed) * u128::from(self.native_rate)
            {
                // The writer delivered under half its nominal frames across a
                // multi-period interval: the queued audio spans a device gap.
                self.discard_backlog(queued.saturating_sub(self.servo.target()));
            }
        }
    }

    fn discard_backlog(&mut self, excess: usize) {
        let discard = excess.min(self.rx.slots());
        if discard == 0 {
            return;
        }
        if let Ok(chunk) = self.rx.read_chunk(discard) {
            chunk.commit_all();
        }
        self.counters
            .dropped
            .fetch_add(discard as u64, Ordering::Relaxed);
        self.fade = RECOVERY_FADE_FRAMES;
    }
}
impl<S: ProcessingSample> RtAudioSource<S> for RingSource<S> {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        let stamp = self.clock.snapshot();
        let queued = self.rx.slots() + self.src.queued_frames();
        if !self.primed && queued >= self.servo.target() {
            self.primed = true;
        }
        self.recover(stamp);
        let feedforward = self.observe_rate(stamp);
        let feedback = self
            .servo
            .observe(self.rx.slots() + self.src.queued_frames());
        self.last_correction_ppm = feedforward + feedback;
        self.src.set_correction_ppm(self.last_correction_ppm);
        let mut transferred = 0;
        for i in 0..output.frames() {
            let frame = if self.primed {
                // Keep the backlog in the ring: the resampler only holds what
                // the rest of this block still needs (lookahead included), so
                // recovery discards from the ring stay meaningful.
                let remaining = output.frames() - i;
                if let Some(needed) = self.src.input_frames_needed(remaining)
                    && needed > 0
                {
                    refill_bounded(&mut self.src, &mut self.rx, needed);
                }
                self.src.next_frame()
            } else {
                None
            };
            let frame = match frame {
                Some(frame) => {
                    transferred += 1;
                    if self.fade > 0 {
                        let gain = (RECOVERY_FADE_FRAMES - self.fade + 1) as f64
                            / RECOVERY_FADE_FRAMES as f64;
                        self.fade -= 1;
                        [frame[0] * gain, frame[1] * gain]
                    } else {
                        frame
                    }
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

/// Moves at most `max` frames so bulk backlogs stay ring-resident.
fn refill_bounded(src: &mut StereoResampler, rx: &mut Consumer<StereoFrame>, max: usize) {
    let count = src.free_frames().min(rx.slots()).min(max);
    for _ in 0..count {
        if let Ok(frame) = rx.pop() {
            src.push_frame(frame);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo};

    const RATE: u32 = 48_000;

    fn source(capacity: usize) -> (RingSource<f32>, Producer<StereoFrame>) {
        let (tx, rx) = rtrb::RingBuffer::new(capacity);
        let clock = Arc::new(CaptureWriteClock::new());
        let source =
            RingSource::<f32>::new(rx, RATE, RATE, 256, capacity, Arc::default(), clock.clone())
                .unwrap();
        (source, tx)
    }

    fn read(source: &mut RingSource<f32>, frames: usize) -> Vec<f32> {
        let mut arena = BufferArena::<f32>::new(
            &[BufferSlotLayout {
                channels: 2,
                capacity_frames: frames,
            }],
            4096,
        )
        .unwrap();
        let io = arena
            .prepare_io(&[PortAccess::Write {
                port: 0,
                slot: arena.slot(0).unwrap(),
            }])
            .unwrap();
        let ctx = ProcessContext {
            timeline_epoch: 1,
            timeline_start: 0,
            frames,
            processing_sr: f64::from(RATE),
        };
        let mut samples = Vec::new();
        arena
            .with_io(&io, 0..frames, |io| {
                if let ProcessIo::Separate { mut outputs, .. } = io {
                    let _ = source.read(&ctx, outputs.get_mut(0).unwrap());
                    samples.extend_from_slice(outputs.get_mut(0).unwrap().channel(0));
                }
            })
            .unwrap();
        samples
    }

    #[test]
    fn backlog_recovery_discards_to_the_servo_reference_and_fades_in() {
        // target = 2*256 = 512; watermark = 1536.
        let (mut source, mut tx) = source(8192);
        for _ in 0..3000 {
            tx.push([1.0, -1.0]).unwrap();
        }
        let samples = read(&mut source, 64);
        let dropped = source.counters.dropped.load(Ordering::Relaxed);
        assert_eq!(dropped, 3000 - 512);
        assert!(
            samples[0] < 0.1,
            "recovery starts from a fade, got {}",
            samples[0]
        );
        // The 32-frame fade finishes within the following block; the FIR's own
        // priming latency must not leave the output attenuated forever.
        let samples = read(&mut source, 64);
        assert!(
            samples.iter().any(|value| (value - 1.0).abs() < 1e-9),
            "fade must reach unity within one block, got {samples:?}"
        );
    }

    #[test]
    fn capture_gap_discards_the_backlog_across_the_discontinuity() {
        let (mut source, mut tx) = source(8192);
        // Stamps must be plausible absolute QPC values for the stall check.
        let base = QpcClock::new().and_then(|clock| clock.now()).unwrap_or(1);
        // Prime with a live stamp sequence.
        source.clock.publish(480, base);
        for _ in 0..1024 {
            tx.push([0.5, 0.5]).unwrap();
        }
        let _ = read(&mut source, 64);
        assert_eq!(source.counters.dropped.load(Ordering::Relaxed), 0);
        // 480 frames across 100 ms is under half the nominal 4800: a gap.
        source.clock.publish(960, base + 1_000_000);
        let _ = read(&mut source, 64);
        // The queue above the 512 servo target spans the device gap and is
        // invalidated wholesale.
        assert!(source.counters.dropped.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn write_clock_snapshots_reject_torn_reads() {
        let clock = CaptureWriteClock::new();
        assert_eq!(clock.snapshot(), Some((0, 0)));
        clock.publish(100, 555);
        assert_eq!(clock.snapshot(), Some((100, 555)));
    }

    #[test]
    fn rate_feed_forward_engages_after_agreeing_windows() {
        let (mut source, mut tx) = source(8192);
        // Nominal +1000 ppm: each 100 ms advances 4805 frames. The
        // queue is kept at the 512 target so the occupancy follower stays
        // near zero and the measured correction is the feed-forward term.
        let mut frames = 0u64;
        let base = QpcClock::new().and_then(|clock| clock.now()).unwrap_or(1);
        let mut qpc = base;
        // Anchor first while priming the queue.
        source.clock.publish(frames, qpc);
        for _ in 0..576 {
            tx.push([0.25, -0.25]).unwrap();
        }
        let _ = read(&mut source, 64);
        for _ in 1..=24u64 {
            frames += 4_805;
            qpc += 1_000_000;
            source.clock.publish(frames, qpc);
            for _ in 0..64 {
                tx.push([0.25, -0.25]).unwrap();
            }
            let _ = read(&mut source, 64);
        }
        assert!(
            source.correction_ppm() > 500.0,
            "feed-forward must dominate after agreeing windows, got {}",
            source.correction_ppm()
        );
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
