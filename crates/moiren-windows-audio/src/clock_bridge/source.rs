//! Continuous linear interpolation and a bounded fill servo on the render owner.
use super::{ClockBridgeConfig, ClockBridgeError, Frame, telemetry::Counters};
use crate::latency::QpcClock;
use crate::latency::queue::{QueueBoundary, QueueHistoryWriter, QueueReadSample};
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
    queue_events: QueueHistoryWriter,
    removed_frames: u64,
    reset_discarded_frames: u64,
    reset_count: u64,
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
    acquired_ring_remaining: Option<usize>,
    ingress_read_now: Option<u64>,
    ingress_clock_high_water_100ns: Option<u64>,
    ingress_fade_pending: bool,
}
impl ClockSource {
    /// Immutable queue policy actually attached during capture preparation.
    /// This describes FIFO residency, not a negotiated native device period.
    pub fn prepared_input(&self) -> Option<crate::latency::PreparedInputQueue> {
        self.config.prepared_input
    }

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
        queue_events: QueueHistoryWriter,
    ) -> Self {
        Self {
            queue_events,
            removed_frames: 0,
            reset_discarded_frames: 0,
            reset_count: 0,
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
            acquired_ring_remaining: None,
            ingress_read_now: None,
            ingress_clock_high_water_100ns: None,
            ingress_fade_pending: false,
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
            self.counters.oldest_frame_age.publish(None);
            return Err(ClockBridgeError::InvalidOutput);
        }
        output.fill(0.0);
        Ok(
            self.read_to(output.len() / 2, now_qpc_100ns, |frame, pair| {
                output[frame * 2..frame * 2 + 2].copy_from_slice(&pair)
            }),
        )
    }

    /// Bracket an actual consumer read with a host clock, never a source
    /// sample timestamp. The prepared native path uses the same mechanism;
    /// this injectable variant allows deterministic envelope verification.
    /// Ingress elapsed enforcement adds one observation after acquiring the
    /// finite input prefix, for three observations per nonempty read.
    pub fn read_interleaved_with_publication_clock(
        &mut self,
        output: &mut [f32],
        mut clock: impl FnMut() -> Option<u64>,
    ) -> Result<BoundaryReport, ClockBridgeError> {
        if !output.len().is_multiple_of(2) {
            self.counters.oldest_frame_age.publish(None);
            return Err(ClockBridgeError::InvalidOutput);
        }
        output.fill(0.0);
        Ok(self.read_to_with_clock(
            output.len() / 2,
            None,
            |frame, pair| output[frame * 2..frame * 2 + 2].copy_from_slice(&pair),
            &mut clock,
        ))
    }
    fn add_removed(&mut self, frames: usize) {
        if let Some(total) = self.removed_frames.checked_add(frames as u64) {
            self.removed_frames = total;
        } else {
            self.queue_events.mark_saturated();
        }
    }
    fn pop_frame(&mut self) -> Option<Frame> {
        if self.acquired_ring_remaining == Some(0) {
            return None;
        }
        let frame = self.rx.pop().ok()?;
        if let Some(remaining) = self.acquired_ring_remaining.as_mut() {
            *remaining -= 1;
        }
        self.add_removed(1);
        Some(frame)
    }
    fn discard_ring(&mut self, frames: usize) {
        if let Some(remaining) = self.acquired_ring_remaining.as_mut() {
            assert!(frames <= *remaining, "discard stays in the acquired prefix");
            *remaining -= frames;
        }
        self.rx
            .read_chunk(frames)
            .expect("capture discard snapshot")
            .commit_all();
        self.add_removed(frames);
    }
    fn queue_boundary(&self, ring_frames: usize) -> QueueBoundary {
        QueueBoundary {
            removed_frames: self.removed_frames,
            ring_frames: ring_frames as u64,
            held_frames: u64::from(self.current.is_some()) + u64::from(self.next.is_some()),
        }
    }
    fn reset(&mut self, discard_held: bool) {
        let held = u64::from(self.current.is_some()) + u64::from(self.next.is_some());
        if let Some(total) = self.reset_count.checked_add(1) {
            self.reset_count = total;
        } else {
            self.queue_events.mark_saturated();
        }
        if discard_held {
            if let Some(total) = self.reset_discarded_frames.checked_add(held) {
                self.reset_discarded_frames = total;
            } else {
                self.queue_events.mark_saturated();
            }
        }
        self.counters.oldest_frame_age.publish(None);
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
    fn ingress_frame_fresh(&self, frame: Frame) -> bool {
        let Some(limit) = self.config.max_ingress_elapsed_100ns else {
            return true;
        };
        self.ingress_read_now
            .zip(frame.ingress_started_100ns)
            .and_then(|(now, started)| now.checked_sub(started.get()))
            .is_some_and(|elapsed| elapsed <= limit)
    }

    fn observe_ingress_clock(&mut self, observation: Option<u64>) -> bool {
        let Some(now) = observation.filter(|&now| now != 0) else {
            return false;
        };
        let monotonic = self
            .ingress_clock_high_water_100ns
            .is_none_or(|previous| now >= previous);
        // Even a rejected bracket can contain a later known time. Retain every
        // endpoint so a subsequent regression cannot make old input fresh.
        self.ingress_clock_high_water_100ns = self.ingress_clock_high_water_100ns.max(Some(now));
        monotonic
    }

    fn discard_ingress_prefix(&mut self, sample: &mut QueueReadSample) {
        let ring = self.acquired_ring_remaining.unwrap_or(0);
        let held = usize::from(self.current.is_some()) + usize::from(self.next.is_some());
        let discarded = ring + held;
        if discarded == 0 {
            return;
        }
        self.discard_ring(ring);
        // Loss is attributed here, not again as reset-held discards.
        self.reset(false);
        self.ingress_fade_pending = true;
        sample.ingress_elapsed_discarded_frames += discarded as u64;
        self.counters
            .ingress_elapsed_discarded_frames
            .fetch_add(discarded as u64, Relaxed);
        self.counters
            .ingress_elapsed_recoveries
            .fetch_add(1, Relaxed);
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
            self.discard_ring(ring_discarded);
        }
        self.reset(false);
        self.current = retained.or_else(|| self.pop_frame());
        self.next = self.pop_frame();
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
            self.discard_ring(ring_discarded);
        }
        let retained = (desired == 1).then_some(self.next).flatten();
        self.reset(false);
        self.current = retained.or_else(|| self.pop_frame());
        self.next = self.pop_frame();
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
        set: impl FnMut(usize, [f32; 2]),
    ) -> BoundaryReport {
        let clock = self.age_clock;
        self.read_to_with_clock(frames, now_qpc_100ns, set, &mut || {
            clock.as_ref().and_then(QpcClock::now)
        })
    }

    fn read_to_with_clock(
        &mut self,
        frames: usize,
        now_qpc_100ns: Option<u64>,
        set: impl FnMut(usize, [f32; 2]),
        clock: &mut impl FnMut() -> Option<u64>,
    ) -> BoundaryReport {
        if frames == 0 {
            self.counters.oldest_frame_age.publish(None);
            return BoundaryReport::default();
        }
        let now_qpc_100ns = now_qpc_100ns.or_else(&mut *clock);
        let before = self.queue_boundary(self.rx.slots());
        if self.config.max_ingress_elapsed_100ns.is_some() {
            self.acquired_ring_remaining = Some(before.ring_frames as usize);
            let before_monotonic = self.observe_ingress_clock(now_qpc_100ns);
            // The pre-read stamp precedes tail acquisition. Observe once after
            // acquisition so a concurrent newer publication cannot appear fresh
            // using a clock observation that actually preceded its arrival.
            let after_acquire = clock().filter(|&now| now != 0);
            let after_monotonic = self.observe_ingress_clock(after_acquire);
            self.ingress_read_now = after_acquire.filter(|_| before_monotonic && after_monotonic);
        }
        let mut sample = QueueReadSample {
            requested_frames: frames as u64,
            before,
            qpc_before_100ns: now_qpc_100ns,
            ..Default::default()
        };
        let resets = self.reset_count;
        let reset_discarded = self.reset_discarded_frames;
        let report = self.read_inventory(frames, now_qpc_100ns, set, &mut sample);
        self.acquired_ring_remaining = None;
        self.ingress_read_now = None;
        sample.after = self.queue_boundary(self.rx.slots());
        sample.qpc_after_100ns = clock();
        if self.config.max_ingress_elapsed_100ns.is_some() {
            self.observe_ingress_clock(sample.qpc_after_100ns);
        }
        sample.output_frames = report.transferred_frames as u64;
        sample.resets = self.reset_count - resets;
        sample.reset_discarded_frames = self.reset_discarded_frames - reset_discarded;
        self.queue_events.record(sample);
        report
    }
    fn read_inventory(
        &mut self,
        frames: usize,
        now_qpc_100ns: Option<u64>,
        mut set: impl FnMut(usize, [f32; 2]),
        sample: &mut QueueReadSample,
    ) -> BoundaryReport {
        // Sample the actual oldest pending correlation before any recovery,
        // priming trim or transfer. Never substitute the latest packet stamp
        // or fill/rate. Every read replaces prior success with known/unknown.
        self.counters
            .oldest_frame_age
            .publish(now_qpc_100ns.and_then(|now| self.oldest_frame_age(now)));
        let mut fill = (sample.before.ring_frames + sample.before.held_frames) as usize;
        let mut report = BoundaryReport::default();
        if self.config.max_ingress_elapsed_100ns.is_some() {
            let fresh = self
                .current
                .is_none_or(|frame| self.ingress_frame_fresh(frame))
                && self
                    .next
                    .is_none_or(|frame| self.ingress_frame_fresh(frame))
                && (sample.before.ring_frames == 0
                    || self
                        .rx
                        .peek()
                        .is_ok_and(|frame| self.ingress_frame_fresh(*frame)));
            if !fresh {
                self.discard_ingress_prefix(sample);
                self.counters.fill_frames.store(0, Relaxed);
                return BoundaryReport {
                    discontinuity: true,
                    xruns: 1,
                    ..Default::default()
                };
            }
        }
        let discarded = self.recover_backlog(fill);
        let age_discarded = (discarded == 0)
            .then_some(now_qpc_100ns)
            .flatten()
            .map(|now| self.recover_age(fill - discarded, now))
            .unwrap_or(0);
        let recovered_age = age_discarded > 0;
        sample.backlog_discarded_frames = discarded as u64;
        sample.age_discarded_frames = age_discarded as u64;
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
                self.discard_ring(discarded);
                sample.prime_discarded_frames = discarded as u64;
                self.counters
                    .prime_discarded_frames
                    .fetch_add(discarded as u64, Relaxed);
                fill -= discarded;
                self.counters.fill_frames.store(fill, Relaxed);
            }
            self.current = self.pop_frame();
            self.next = self.pop_frame();
            if self.ingress_fade_pending {
                self.fade_remaining = RECOVERY_FADE_FRAMES;
                self.ingress_fade_pending = false;
            }
        }
        self.counters.min_fill_frames.fetch_min(fill, Relaxed);
        self.counters.max_fill_frames.fetch_max(fill, Relaxed);
        let mut guarded_frames = 0u64;
        for frame in 0..frames {
            while self.phase >= 1.0 {
                self.phase -= 1.0;
                self.current = self.next.take();
                self.next = self.pop_frame();
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
                sample.underrun_frames = (frames - frame) as u64;
                report.discontinuity = true;
                report.xruns = 1;
                self.reset(true);
                break;
            };
            if !self.ingress_frame_fresh(a) || !self.ingress_frame_fresh(b) {
                self.discard_ingress_prefix(sample);
                report.discontinuity = true;
                report.xruns = 1;
                break;
            }
            if a.generation != b.generation {
                // Keep the new generation's remaining frames in the ring, but
                // never interpolate across a missing packet or overflow tail.
                report.discontinuity = true;
                report.xruns = 1;
                self.reset(true);
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
            self.counters.oldest_frame_age.publish(None);
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
