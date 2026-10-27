//! Continuous linear interpolation and a bounded fill servo on the render owner.
use super::{ClockBridgeConfig, ClockBridgeError, Frame, telemetry::Counters};
use crate::latency::clock::QpcClock;
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource},
    buffer::AudioBlockMut,
    processor::ProcessContext,
};
use rtrb::Consumer;
use std::sync::{Arc, atomic::Ordering::Relaxed};

const RECOVERY_FADE_FRAMES: usize = 32;
const SERVO_QUANTUM_FRAMES: usize = 32;

pub struct ClockSource {
    rx: Consumer<Frame>,
    config: ClockBridgeConfig,
    counters: Arc<Counters>,
    current: Option<Frame>,
    next: Option<Frame>,
    phase: f64,
    filtered_fill: f64,
    integral: f64,
    step: f64,
    servo_frames_remaining: usize,
    low_water_guard: bool,
    fade_remaining: usize,
    last_age_recovery_written_frames: Option<u64>,
    age_clock: Option<QpcClock>,
}
impl ClockSource {
    /// Non-consuming owner-local fill, with approximate scalar counters.
    /// Concurrent producer writes can advance immediately after this sample.
    pub fn snapshot(&self) -> super::BridgeSnapshot {
        let mut snapshot = self.counters.snapshot();
        snapshot.fill_frames = self.rx.slots()
            + usize::from(self.current.is_some())
            + usize::from(self.next.is_some());
        snapshot
    }

    /// Estimate the age of the oldest pending native frame without advancing
    /// interpolation or changing telemetry. The observation must use the same
    /// QPC/100 ns domain as CapturePacket; callback arrival time is not a stamp.
    /// Empty, unqualified, future or overflowing times return None.
    pub fn oldest_frame_age(&self, now_qpc_100ns: u64) -> Option<super::QueuedFrameAge> {
        self.oldest_frame_stamp()?
            .estimate_age(self.config.input_sample_rate, now_qpc_100ns)
    }

    pub fn oldest_frame_age_now(&self) -> Option<super::QueuedFrameAge> {
        self.age_clock
            .as_ref()?
            .now()
            .and_then(|now| self.oldest_frame_age(now))
    }

    /// Read a coherent owner-local correlation without consuming audio. None
    /// means empty/unvalidated timing, including virtual positionless sources.
    /// A returned stamp does not imply enough lookahead to render the frame.
    pub fn oldest_frame_stamp(&self) -> Option<super::QueuedFrameStamp> {
        // Phase advancement is deferred until the next output iteration. At
        // Pending frames still belong to the same bounded native FIFO.
        let frame = if self.current.is_none() || self.phase >= 2.0 {
            self.rx.peek().ok()
        } else if self.phase >= 1.0 {
            self.next.as_ref()
        } else {
            self.current.as_ref()
        }?;
        let stamp = frame.packet_stamp?;
        Some(super::QueuedFrameStamp {
            generation: frame.generation,
            packet_position_frames: stamp.position_frames,
            packet_qpc_100ns: stamp.qpc_100ns.get(),
            packet_accepted_frames: stamp.accepted_frames,
            packet_offset_frames: frame.packet_offset_frames,
            rate_correction_ppm: stamp.rate_correction_ppm,
        })
    }

    pub(super) fn new(
        rx: Consumer<Frame>,
        config: ClockBridgeConfig,
        counters: Arc<Counters>,
        age_clock: Option<QpcClock>,
    ) -> Self {
        Self {
            rx,
            config,
            counters,
            current: None,
            next: None,
            phase: 0.0,
            filtered_fill: config.servo_reference() as f64,
            integral: 0.0,
            step: f64::from(config.input_sample_rate) / f64::from(config.output_sample_rate),
            servo_frames_remaining: 0,
            low_water_guard: false,
            fade_remaining: 0,
            last_age_recovery_written_frames: None,
            age_clock,
        }
    }
    pub fn read_interleaved(
        &mut self,
        output: &mut [f32],
    ) -> Result<BoundaryReport, ClockBridgeError> {
        self.read_interleaved_at_qpc(output, None)
    }

    /// Test/control-side variant that supplies a QPC observation for the
    /// optional live age policy. Native integration supplies this clock from
    /// its prepared owner clock; the ordinary FIFO API remains unchanged.
    pub fn read_interleaved_at_qpc(
        &mut self,
        output: &mut [f32],
        now_qpc_100ns: Option<u64>,
    ) -> Result<BoundaryReport, ClockBridgeError> {
        if !output.len().is_multiple_of(2) {
            return Err(ClockBridgeError::InvalidOutput);
        }
        output.fill(0.0);
        Ok(
            self.read_to(output.len() / 2, now_qpc_100ns, |frame, pair| {
                output[frame * 2..frame * 2 + 2].copy_from_slice(&pair)
            }),
        )
    }
    fn reset(&mut self) {
        self.current = None;
        self.next = None;
        self.phase = 0.0;
        self.filtered_fill = self.config.servo_reference() as f64;
        self.integral = 0.0;
        self.step =
            f64::from(self.config.input_sample_rate) / f64::from(self.config.output_sample_rate);
        self.servo_frames_remaining = 0;
        self.low_water_guard = false;
        self.fade_remaining = 0;
        self.counters
            .correction_bits
            .store(0.0f64.to_bits(), Relaxed);
        self.counters.resets.fetch_add(1, Relaxed);
    }
    fn recover_backlog(&mut self, fill: usize) -> usize {
        let Some(input) = self.config.prepared_input else {
            return 0;
        };
        if !self.config.trim_on_prime
            || self.current.is_none()
            || fill < input.high_water_frames as usize
        {
            return 0;
        }
        let discarded = fill - self.config.servo_reference();
        let held = usize::from(self.current.is_some()) + usize::from(self.next.is_some());
        // One discarded frame removes current only; next is already outside
        // the ring and must become the first retained interpolation frame.
        let retained = if discarded == 1 { self.next } else { None };
        let ring_discarded = discarded.saturating_sub(held);
        if ring_discarded > 0 {
            // Only this consumer advances the read cursor. The discard is
            // bounded by the initial snapshot, even if capture keeps writing.
            self.rx
                .read_chunk(ring_discarded)
                .expect("running capture backlog snapshot")
                .commit_all();
        }
        self.reset();
        self.current = retained.or_else(|| self.rx.pop().ok());
        self.next = self.rx.pop().ok();
        self.fade_remaining = RECOVERY_FADE_FRAMES;
        self.counters
            .backlog_discarded_frames
            .fetch_add(discarded as u64, Relaxed);
        self.counters.backlog_recoveries.fetch_add(1, Relaxed);
        discarded
    }
    fn recover_age(&mut self, fill: usize, now_qpc_100ns: u64) -> usize {
        let Some(limit) = self.config.age_limit_100ns else {
            return 0;
        };
        if !self.config.trim_on_prime || self.current.is_none() || fill <= 2 {
            return 0;
        }
        let written_frames = self.counters.written_frames.load(Relaxed);
        if self
            .last_age_recovery_written_frames
            .is_some_and(|previous| previous == written_frames)
        {
            return 0;
        }
        let Some(age) = self.oldest_frame_age(now_qpc_100ns) else {
            return 0;
        };
        if age.estimated_age_100ns <= limit {
            return 0;
        }
        let rate = u128::from(self.config.input_sample_rate);
        let excess = u128::from(age.estimated_age_100ns - limit);
        let desired = usize::try_from((excess * rate).div_ceil(10_000_000))
            .unwrap_or(fill - 2)
            .clamp(1, fill - 2);
        let held = usize::from(self.current.is_some()) + usize::from(self.next.is_some());
        let ring_discarded = desired.saturating_sub(held);
        if ring_discarded > 0 {
            self.rx
                .read_chunk(ring_discarded)
                .expect("running capture age snapshot")
                .commit_all();
        }
        let retained = (desired == 1).then_some(self.next).flatten();
        self.reset();
        self.current = retained.or_else(|| self.rx.pop().ok());
        self.next = self.rx.pop().ok();
        self.fade_remaining = RECOVERY_FADE_FRAMES;
        self.last_age_recovery_written_frames = Some(written_frames);
        self.counters
            .age_discarded_frames
            .fetch_add(desired as u64, Relaxed);
        self.counters.age_recoveries.fetch_add(1, Relaxed);
        desired
    }
    fn ratio(&mut self, fill: usize, rate_correction_ppm: Option<i64>) -> f64 {
        // A fixed output-frame cadence samples the same FIFO trajectory even
        // when parameter events divide one Engine block into many segments.
        let dt = SERVO_QUANTUM_FRAMES as f64 / f64::from(self.config.output_sample_rate);
        self.filtered_fill += dt / (0.25 + dt) * (fill as f64 - self.filtered_fill);
        let error_seconds = (self.filtered_fill - self.config.servo_reference() as f64)
            / f64::from(self.config.input_sample_rate);
        let limit = self.config.max_correction_ppm / 1_000_000.0;
        // Only a prepared live input enables capture/QPC feed-forward. The
        // FIFO servo still corrects render-clock drift and queue phase bias.
        let feed_forward = if self.config.prepared_input.is_some() && self.config.trim_on_prime {
            rate_correction_ppm.map_or(0.0, |ppm| ppm as f64 / 1_000_000.0)
        } else {
            0.0
        };
        self.low_water_guard = self
            .config
            .prepared_input
            .is_some_and(|input| fill <= input.low_water_frames as usize && limit > 0.0);
        let candidate = (self.integral + 0.02 * error_seconds * dt).clamp(-limit, limit);
        let raw = feed_forward + 0.2 * error_seconds + candidate;
        // Integrate only when unsaturated, or when the error pulls back from a
        // saturated limit. Startup/event backlog is not evidence of clock drift.
        if !self.low_water_guard
            && ((-limit..=limit).contains(&raw)
                || (raw > limit && error_seconds < 0.0)
                || (raw < -limit && error_seconds > 0.0))
        {
            self.integral = candidate;
        }
        // A stale filtered backlog must not accelerate a depleted queue.
        // This temporary guard stays inside the admitted rate correction
        // limit and freezes integral learning; real starvation still reports.
        let correction = if self.low_water_guard {
            -limit
        } else {
            (feed_forward + 0.2 * error_seconds + self.integral).clamp(-limit, limit)
        };
        self.counters
            .correction_bits
            .store((correction * 1_000_000.0).to_bits(), Relaxed);
        f64::from(self.config.input_sample_rate) / f64::from(self.config.output_sample_rate)
            * (1.0 + correction)
    }
    fn read_to(
        &mut self,
        frames: usize,
        now_qpc_100ns: Option<u64>,
        mut set: impl FnMut(usize, [f32; 2]),
    ) -> BoundaryReport {
        if frames == 0 {
            return BoundaryReport::default();
        }
        let now_qpc_100ns =
            now_qpc_100ns.or_else(|| self.age_clock.as_ref().and_then(QpcClock::now));
        let mut fill = self.rx.slots()
            + usize::from(self.current.is_some())
            + usize::from(self.next.is_some());
        let mut report = BoundaryReport::default();
        let discarded = self.recover_backlog(fill);
        let age_discarded = (discarded == 0)
            .then_some(now_qpc_100ns)
            .flatten()
            .map(|now| self.recover_age(fill - discarded, now))
            .unwrap_or(0);
        let recovered_age = age_discarded > 0;
        if discarded + age_discarded > 0 {
            fill -= discarded + age_discarded;
            report.discontinuity = true;
        }
        self.counters.fill_frames.store(fill, Relaxed);
        if self.current.is_none() {
            let target = if self.rx.is_abandoned() {
                2
            } else {
                self.config.target_fill_frames
            };
            if !recovered_age && fill < target {
                self.counters
                    .priming_frames
                    .fetch_add(frames as u64, Relaxed);
                return BoundaryReport::default();
            }
            if self.config.trim_on_prime && fill > self.config.target_fill_frames {
                // Startup/Graph/device preparation can accumulate old audio.
                // Trim exactly one initial snapshot; never chase the producer.
                let discarded = fill - self.config.target_fill_frames;
                self.rx
                    .read_chunk(discarded)
                    .expect("initial capture backlog")
                    .commit_all();
                self.counters
                    .prime_discarded_frames
                    .fetch_add(discarded as u64, Relaxed);
                fill -= discarded;
                self.counters.fill_frames.store(fill, Relaxed);
            }
            self.current = self.rx.pop().ok();
            self.next = self.rx.pop().ok();
        }
        self.counters.min_fill_frames.fetch_min(fill, Relaxed);
        self.counters.max_fill_frames.fetch_max(fill, Relaxed);
        let mut guarded_frames = 0u64;
        for frame in 0..frames {
            while self.phase >= 1.0 {
                self.phase -= 1.0;
                self.current = self.next.take();
                self.next = self.rx.pop().ok();
            }
            let (Some(a), Some(b)) = (self.current, self.next) else {
                self.counters.last_underrun_at_output_frame.store(
                    self.counters.output_frames.load(Relaxed) + frame as u64,
                    Relaxed,
                );
                self.counters
                    .last_underrun_producer_finished
                    .store(self.counters.producer_finished.load(Relaxed), Relaxed);
                self.counters
                    .underrun_frames
                    .fetch_add((frames - frame) as u64, Relaxed);
                report.discontinuity = true;
                report.xruns = 1;
                self.reset();
                break;
            };
            if a.generation != b.generation {
                // Keep the new generation's remaining frames in the ring, but
                // never interpolate across a missing packet or overflow tail.
                report.discontinuity = true;
                report.xruns = 1;
                self.reset();
                break;
            }
            if self.servo_frames_remaining == 0 {
                self.step = self.ratio(
                    self.rx.slots() + 2,
                    a.packet_stamp.and_then(|stamp| stamp.rate_correction_ppm),
                );
                self.servo_frames_remaining = SERVO_QUANTUM_FRAMES;
            }
            let step = self.step;
            self.servo_frames_remaining -= 1;
            guarded_frames += u64::from(self.low_water_guard);
            let gain = if self.fade_remaining > 0 {
                let gain = (RECOVERY_FADE_FRAMES - self.fade_remaining + 1) as f64
                    / RECOVERY_FADE_FRAMES as f64;
                self.fade_remaining -= 1;
                gain
            } else {
                1.0
            };
            let pair = std::array::from_fn(|ch| {
                // Use f64 intermediate so finite f32 headroom cannot overflow
                // the subtraction used by interpolation.
                ((f64::from(a.samples[ch]) * (1.0 - self.phase)
                    + f64::from(b.samples[ch]) * self.phase)
                    * gain) as f32
            });
            set(frame, pair);
            report.transferred_frames += 1;
            self.phase += step;
        }
        self.counters
            .output_frames
            .fetch_add(report.transferred_frames as u64, Relaxed);
        if guarded_frames > 0 {
            self.counters
                .low_water_guard_frames
                .fetch_add(guarded_frames, Relaxed);
        }
        report
    }
}
impl RtAudioSource<f32> for ClockSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, f32>) -> BoundaryReport {
        output.clear();
        if output.channel_count() != 2 {
            return BoundaryReport {
                discontinuity: true,
                xruns: 1,
                ..BoundaryReport::default()
            };
        }
        self.read_to(output.frames(), None, |frame, pair| {
            output.channel_mut(0)[frame] = pair[0];
            output.channel_mut(1)[frame] = pair[1];
        })
    }
}
