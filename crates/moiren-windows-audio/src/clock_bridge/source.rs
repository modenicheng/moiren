//! Continuous linear interpolation and a bounded fill servo on the render owner.
use super::{ClockBridgeConfig, ClockBridgeError, Frame, OUTPUT_SAMPLE_RATE, telemetry::Counters};
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource},
    buffer::AudioBlockMut,
    processor::ProcessContext,
};
use rtrb::Consumer;
use std::sync::{Arc, atomic::Ordering::Relaxed};

const RECOVERY_FADE_FRAMES: usize = 32;

pub struct ClockSource {
    rx: Consumer<Frame>,
    config: ClockBridgeConfig,
    counters: Arc<Counters>,
    current: Option<Frame>,
    next: Option<Frame>,
    phase: f64,
    filtered_fill: f64,
    integral: f64,
    fade_remaining: usize,
}
impl ClockSource {
    pub(super) fn new(
        rx: Consumer<Frame>,
        config: ClockBridgeConfig,
        counters: Arc<Counters>,
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
            fade_remaining: 0,
        }
    }
    pub fn read_interleaved(
        &mut self,
        output: &mut [f32],
    ) -> Result<BoundaryReport, ClockBridgeError> {
        if !output.len().is_multiple_of(2) {
            return Err(ClockBridgeError::InvalidOutput);
        }
        output.fill(0.0);
        Ok(self.read_to(output.len() / 2, |frame, pair| {
            output[frame * 2..frame * 2 + 2].copy_from_slice(&pair)
        }))
    }
    fn reset(&mut self) {
        self.current = None;
        self.next = None;
        self.phase = 0.0;
        self.filtered_fill = self.config.servo_reference() as f64;
        self.integral = 0.0;
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
    fn ratio(&mut self, fill: usize, frames: usize) -> f64 {
        // Scale updates by elapsed output time, rather than by callback count:
        // parameter events can divide one Engine block into many segments.
        let dt = frames as f64 / f64::from(OUTPUT_SAMPLE_RATE);
        self.filtered_fill += dt / (0.25 + dt) * (fill as f64 - self.filtered_fill);
        let error_seconds = (self.filtered_fill - self.config.servo_reference() as f64)
            / f64::from(self.config.input_sample_rate);
        let limit = self.config.max_correction_ppm / 1_000_000.0;
        let candidate = (self.integral + 0.02 * error_seconds * dt).clamp(-limit, limit);
        let raw = 0.2 * error_seconds + candidate;
        // Integrate only when unsaturated, or when the error pulls back from a
        // saturated limit. Startup/event backlog is not evidence of clock drift.
        if (-limit..=limit).contains(&raw)
            || (raw > limit && error_seconds < 0.0)
            || (raw < -limit && error_seconds > 0.0)
        {
            self.integral = candidate;
        }
        let correction = (0.2 * error_seconds + self.integral).clamp(-limit, limit);
        self.counters
            .correction_bits
            .store((correction * 1_000_000.0).to_bits(), Relaxed);
        f64::from(self.config.input_sample_rate) / f64::from(OUTPUT_SAMPLE_RATE)
            * (1.0 + correction)
    }
    fn read_to(&mut self, frames: usize, mut set: impl FnMut(usize, [f32; 2])) -> BoundaryReport {
        if frames == 0 {
            return BoundaryReport::default();
        }
        let mut fill = self.rx.slots()
            + usize::from(self.current.is_some())
            + usize::from(self.next.is_some());
        let mut report = BoundaryReport::default();
        let discarded = self.recover_backlog(fill);
        if discarded > 0 {
            fill -= discarded;
            report.discontinuity = true;
        }
        self.counters.fill_frames.store(fill, Relaxed);
        if self.current.is_none() {
            let target = if self.rx.is_abandoned() {
                2
            } else {
                self.config.target_fill_frames
            };
            if fill < target {
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
        let step = self.ratio(fill, frames);
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
        self.read_to(output.frames(), |frame, pair| {
            output.channel_mut(0)[frame] = pair[0];
            output.channel_mut(1)[frame] = pair[1];
        })
    }
}
