use crate::{resample::StereoResampler, servo::FollowerServo};
use moiren_engine::{
    boundary::{AudioReader, AudioWriter, BoundaryReport, RtAudioSink, RtAudioSource},
    buffer::{AudioBlock, AudioBlockMut},
    host::BackendError,
    processor::ProcessContext,
    sample::ProcessingSample,
};
use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering},
};

use crate::NATIVE_SAMPLE_RATE;
const SRC_HEADROOM_FRAMES: usize = 32;

/// Control-side observation of one virtual boundary's carrier. The realtime
/// side records relaxed snapshots once per block (no allocation, no locks);
/// [`BoundaryProbe::snapshot`] samples the latest values for soak evidence
/// and regressions.
#[derive(Debug, Default)]
pub struct BoundaryProbe {
    level_frames: AtomicU32,
    /// Servo correction applied to this carrier, in thousandths of a ppm.
    /// Positive drains the queue faster.
    applied_milli_ppm: AtomicI32,
    xruns: AtomicU64,
    soft_events: AtomicU64,
}

/// Latest probe sample. `level_frames` is queued content (occupancy, not
/// free space) for both directions.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoundaryProbeSample {
    pub level_frames: u32,
    pub applied_milli_ppm: i32,
    pub xruns: u64,
    pub soft_events: u64,
}

impl BoundaryProbe {
    /// Realtime path: relaxed stores and adds only.
    fn observe(&self, level_frames: usize, applied_ppm: f64, xruns: u64, soft_events: u64) {
        self.level_frames.store(
            u32::try_from(level_frames).unwrap_or(u32::MAX),
            Ordering::Relaxed,
        );
        // Float-to-int casts saturate, so no clamp is needed.
        self.applied_milli_ppm
            .store((applied_ppm * 1000.0) as i32, Ordering::Relaxed);
        self.xruns.fetch_add(xruns, Ordering::Relaxed);
        self.soft_events.fetch_add(soft_events, Ordering::Relaxed);
    }

    /// Control side: sample the latest snapshot.
    pub fn snapshot(&self) -> BoundaryProbeSample {
        BoundaryProbeSample {
            level_frames: self.level_frames.load(Ordering::Relaxed),
            applied_milli_ppm: self.applied_milli_ppm.load(Ordering::Relaxed),
            xruns: self.xruns.load(Ordering::Relaxed),
            soft_events: self.soft_events.load(Ordering::Relaxed),
        }
    }
}

/// Reuses the existing carrier: reserve a worst-case transfer, then center the
/// remaining headroom. This is not an additional audio queue or a latency claim.
struct ClockDrift {
    servo: FollowerServo,
    transfer: usize,
    primed: bool,
}

struct EpochHandshake {
    producer: Arc<AtomicU64>,
    consumer: Arc<AtomicU64>,
}

impl ClockDrift {
    fn new(capacity: usize, transfer: usize) -> Result<Self, BackendError> {
        let headroom = capacity.saturating_sub(transfer) / 2;
        if headroom < SRC_HEADROOM_FRAMES {
            return Err(invalid_layout());
        }
        Ok(Self {
            servo: FollowerServo::new(FollowerServo::start_target(headroom, capacity - transfer)),
            transfer,
            primed: false,
        })
    }
    fn reset(&mut self) {
        self.servo = FollowerServo::new(self.servo.target());
        self.primed = false;
    }
    fn correction(&mut self, available: usize) -> f64 {
        self.servo.observe(available.saturating_sub(self.transfer))
    }
}

fn drift_transfer(frames: usize, rate: u32) -> Result<usize, BackendError> {
    // Covers both +2000 ppm consumption and the inverse -2000 ppm output
    // ratio, plus the existing FIR lookahead, at every accepted Engine rate.
    let native =
        (frames as u128 * u128::from(NATIVE_SAMPLE_RATE) * 1000).div_ceil(u128::from(rate) * 998);
    usize::try_from(native)
        .ok()
        .and_then(|n| n.checked_add(SRC_HEADROOM_FRAMES))
        .ok_or_else(invalid_layout)
}

fn staging(
    frames: usize,
    byte_budget: usize,
    carrier_bytes: usize,
    src_bytes: usize,
) -> Result<Vec<f32>, BackendError> {
    let samples = frames.checked_mul(2).ok_or_else(invalid_layout)?;
    let bytes = samples
        .checked_mul(size_of::<f32>())
        .ok_or_else(invalid_layout)?;
    if frames == 0
        || carrier_bytes
            .checked_add(bytes)
            .and_then(|total| total.checked_add(src_bytes))
            .is_none_or(|total| total > byte_budget)
        || bytes > isize::MAX as usize
    {
        return Err(invalid_layout());
    }
    Ok(vec![0.0; samples])
}

fn invalid_layout() -> BackendError {
    BackendError::Unsupported(
        "virtual boundary requires mono/stereo and bounded carrier staging".into(),
    )
}

fn validate_carrier(capacity: usize, frames: usize) -> Result<usize, BackendError> {
    let bytes = capacity
        .checked_mul(2 * size_of::<f32>())
        .ok_or_else(invalid_layout)?;
    if capacity < frames || bytes > isize::MAX as usize {
        return Err(invalid_layout());
    }
    Ok(bytes)
}

fn src_input_frames(
    output_frames: usize,
    input_rate: u32,
    output_rate: u32,
) -> Result<usize, BackendError> {
    let scaled = (output_frames as u128)
        .checked_mul(u128::from(input_rate))
        .ok_or_else(invalid_layout)?;
    let frames = scaled.div_ceil(u128::from(output_rate));
    usize::try_from(frames)
        .ok()
        .and_then(|frames| frames.checked_add(SRC_HEADROOM_FRAMES))
        .ok_or_else(invalid_layout)
}

fn src_capacity(input_frames: usize) -> Result<usize, BackendError> {
    input_frames
        .checked_add(SRC_HEADROOM_FRAMES)
        .filter(|&capacity| capacity <= 1_048_576)
        .map(|capacity| capacity.max(64))
        .ok_or_else(invalid_layout)
}

fn src_storage(capacity: Option<usize>) -> Result<usize, BackendError> {
    capacity.map_or(Ok(0), |capacity| {
        StereoResampler::storage_bytes(capacity).ok_or_else(invalid_layout)
    })
}

fn resampler(
    input_rate: u32,
    output_rate: u32,
    capacity: Option<usize>,
) -> Result<Option<StereoResampler>, BackendError> {
    capacity
        .map(|capacity| {
            StereoResampler::new(input_rate, output_rate, capacity).map_err(|_| invalid_layout())
        })
        .transpose()
}

fn missing_output_frames(missing_input: usize, input_rate: u32, output_rate: u32) -> usize {
    ((missing_input as u128 * u128::from(output_rate)).div_ceil(u128::from(input_rate)))
        .min(usize::MAX as u128) as usize
}

fn short(frames: usize) -> BoundaryReport {
    BoundaryReport {
        transferred_frames: 0,
        discontinuity: frames > 0,
        xruns: u64::from(frames > 0),
    }
}

/// Mono reads carrier slot 0 exactly; slot 1 is neither duplicated nor summed.
pub struct VirtualPlaybackSource {
    carrier: AudioReader<f32>,
    channels: usize,
    scratch: Vec<f32>,
    resampler: Option<StereoResampler>,
    engine_sample_rate: u32,
    max_block_frames: usize,
    reset_epoch: Option<Arc<AtomicU64>>,
    observed_reset: u64,
    byte_budget: usize,
    drift: Option<ClockDrift>,
    handshake: Option<EpochHandshake>,
    probe: Option<Arc<BoundaryProbe>>,
    /// Frames of exact-zero recovery edge remaining after a prime (re)arm.
    resume_mute: usize,
}

impl VirtualPlaybackSource {
    pub fn new(
        carrier: AudioReader<f32>,
        channels: usize,
        frames: usize,
        engine_sample_rate: u32,
        budget: usize,
    ) -> Result<Self, BackendError> {
        if !(1..=2).contains(&channels)
            || carrier.channel_count() != 2
            || frames == 0
            || !(8_000..=384_000).contains(&engine_sample_rate)
        {
            return Err(invalid_layout());
        }
        let conversion = engine_sample_rate != NATIVE_SAMPLE_RATE;
        let input_frames = if conversion {
            src_input_frames(frames, NATIVE_SAMPLE_RATE, engine_sample_rate)?
        } else {
            frames
        };
        let carrier_bytes = validate_carrier(carrier.capacity_frames(), input_frames)?;
        let capacity = if conversion {
            Some(src_capacity(input_frames)?)
        } else {
            None
        };
        // Admission covers carrier, staging and FIR storage before allocating
        // either the staging buffer or the filter's kernels.
        let scratch = staging(input_frames, budget, carrier_bytes, src_storage(capacity)?)?;
        let src = resampler(NATIVE_SAMPLE_RATE, engine_sample_rate, capacity)?;
        Ok(Self {
            carrier,
            channels,
            scratch,
            resampler: src,
            engine_sample_rate,
            max_block_frames: frames,
            reset_epoch: None,
            observed_reset: 0,
            byte_budget: budget,
            drift: None,
            handshake: None,
            probe: None,
            resume_mute: 0,
        })
    }

    /// Attach the control-side carrier probe; snapshots land once per block.
    pub fn with_probe(mut self, probe: Arc<BoundaryProbe>) -> Self {
        self.probe = Some(probe);
        self
    }

    /// Off-RT opt-in for independently paced native and Engine callbacks.
    /// Uses the existing carrier's headroom and the bounded follower servo;
    /// playback content may prime, but Engine/owner activation never waits.
    /// Unlike `new`, this prepares a FIR even at equal 48 kHz rates.
    ///
    /// The reference is `(carrier_capacity - worst_transfer) / 2` native
    /// frames, plus one transfer at the observation point. A provider carrier
    /// of 4800 frames therefore holds roughly 50 ms, plus FIR lookahead; this
    /// safety reference is NOT a negotiated latency-profile budget. Admission
    /// against actual native packet/jitter timing remains a provider concern.
    pub fn with_clock_drift(mut self) -> Result<Self, BackendError> {
        let transfer = drift_transfer(self.max_block_frames, self.engine_sample_rate)?;
        let drift = ClockDrift::new(self.carrier.capacity_frames(), transfer)?;
        let carrier_bytes = validate_carrier(self.carrier.capacity_frames(), transfer)?;
        let capacity = Some(src_capacity(transfer)?);
        let scratch = staging(
            transfer,
            self.byte_budget,
            carrier_bytes,
            src_storage(capacity)?,
        )?;
        let src = resampler(NATIVE_SAMPLE_RATE, self.engine_sample_rate, capacity)?;
        self.scratch = scratch;
        self.resampler = src;
        self.drift = Some(drift);
        Ok(self)
    }

    pub fn with_reset_epoch(mut self, reset_epoch: Arc<AtomicU64>) -> Self {
        self.observed_reset = self.handshake.as_ref().map_or_else(
            || reset_epoch.load(Ordering::Acquire),
            |handshake| handshake.consumer.load(Ordering::Acquire),
        );
        self.reset_epoch = Some(reset_epoch);
        self
    }

    /// Attach the provider's requested epoch and both quiescence fences off RT.
    /// A reset requested during preparation still requires the full handshake.
    pub fn with_reset_fence(
        self,
        reset_epoch: Arc<AtomicU64>,
        producer_quiesced_epoch: Arc<AtomicU64>,
        consumer_quiesced_epoch: Arc<AtomicU64>,
    ) -> Self {
        self.with_reset_epoch(reset_epoch)
            .with_epoch_handshake(producer_quiesced_epoch, consumer_quiesced_epoch)
    }

    /// Pair with the provider's two-phase reset. Only a quiesced producer can
    /// authorize the consumer to discard old frames; no waiting occurs on RT.
    pub fn with_epoch_handshake(
        mut self,
        producer_quiesced_epoch: Arc<AtomicU64>,
        consumer_quiesced_epoch: Arc<AtomicU64>,
    ) -> Self {
        self.observed_reset = consumer_quiesced_epoch.load(Ordering::Acquire);
        self.handshake = Some(EpochHandshake {
            producer: producer_quiesced_epoch,
            consumer: consumer_quiesced_epoch,
        });
        self
    }

    fn synchronize_epoch(&mut self) -> bool {
        let Some(epoch) = &self.reset_epoch else {
            return true;
        };
        let current = epoch.load(Ordering::Acquire);
        if current == self.observed_reset {
            return true;
        }
        if let Some(handshake) = &self.handshake
            && handshake.producer.load(Ordering::Acquire) != current
        {
            return false;
        }
        self.carrier.discard();
        if let Some(src) = &mut self.resampler {
            src.reset();
        }
        if let Some(drift) = &mut self.drift {
            drift.reset();
        }
        if epoch.load(Ordering::Acquire) != current {
            return false;
        }
        if let Some(handshake) = &self.handshake {
            handshake.consumer.store(current, Ordering::Release);
        }
        self.observed_reset = current;
        true
    }

    fn epoch_current(&self) -> bool {
        self.reset_epoch
            .as_ref()
            .is_none_or(|epoch| epoch.load(Ordering::Acquire) == self.observed_reset)
    }
}

impl<S: ProcessingSample> RtAudioSource<S> for VirtualPlaybackSource {
    fn channel_count(&self) -> usize {
        self.channels
    }

    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        if !self.synchronize_epoch() {
            output.clear();
            return short(output.frames());
        }
        if output.channel_count() != self.channels || output.frames() > self.max_block_frames {
            output.clear();
            return short(output.frames());
        }
        if output.frames() == 0 {
            return BoundaryReport::default();
        }
        if let Some(drift) = &mut self.drift {
            let available = self.carrier.available_frames();
            if !drift.primed {
                if available < drift.transfer + drift.servo.target() {
                    output.clear();
                    return short(output.frames());
                }
                drift.primed = true;
                // The filter's group delay smears the first frames after any
                // resumption into mixed-amplitude edges. Exact zero for the
                // first FIR length is the honest recovery edge: counted as
                // silence downstream, never as corrupted content.
                self.resume_mute = SRC_HEADROOM_FRAMES;
            }
            self.resampler
                .as_mut()
                .expect("drift FIR prepared")
                .set_correction_ppm(drift.correction(available));
        }
        let required_input = if let Some(src) = &self.resampler {
            match src.input_frames_needed(output.frames()) {
                Some(frames) if frames <= self.scratch.len() / 2 => frames,
                _ => {
                    output.clear();
                    return short(output.frames());
                }
            }
        } else {
            output.frames()
        };
        let scratch = &mut self.scratch[..required_input * 2];
        let report = self
            .carrier
            .read_interleaved(scratch)
            .expect("complete carrier frames");
        if let Some(src) = &mut self.resampler {
            let transferred = report.transferred_frames.min(required_input);
            // Silence advances the same timeline as real PCM. Freezing the
            // filter on starvation would replay its history after an idle gap.
            for frame in scratch.as_chunks::<2>().0 {
                if !src.push_frame([f64::from(frame[0]), f64::from(frame[1])]) {
                    src.reset();
                    output.clear();
                    return short(output.frames());
                }
            }
            let mut produced = 0;
            let mut muted = 0_usize;
            while produced < output.frames() {
                let Some(frame) = src.next_frame() else {
                    break;
                };
                if self.resume_mute > 0 {
                    self.resume_mute -= 1;
                    muted += 1;
                    output.channel_mut(0)[produced] = S::ZERO;
                    if self.channels == 2 {
                        output.channel_mut(1)[produced] = S::ZERO;
                    }
                } else {
                    output.channel_mut(0)[produced] = S::from_f64(frame[0]);
                    if self.channels == 2 {
                        output.channel_mut(1)[produced] = S::from_f64(frame[1]);
                    }
                }
                produced += 1;
            }
            if produced < output.frames() {
                for channel in 0..self.channels {
                    output.channel_mut(channel)[produced..].fill(S::ZERO);
                }
            }
            let missing = missing_output_frames(
                required_input - transferred,
                NATIVE_SAMPLE_RATE,
                self.engine_sample_rate,
            );
            // A starved block is an honest, counted xrun. The silence-fill
            // above already advanced the FIR across the gap, and the servo's
            // settled correction is exactly the state that must survive to
            // recenter the level. A fully starved block additionally re-arms
            // only the prime gate: consumption pauses while the native side
            // keeps producing at full rate, so the level rebuilds to
            // transfer+target in well under a ring time instead of riding
            // marginal for ~20 s of servo-only recovery - but, unlike the
            // old reset, no carrier content is discarded, no epoch moves,
            // and the servo and FIR state survive the rebuild.
            if transferred == 0
                && let Some(drift) = &mut self.drift
            {
                drift.primed = false;
            }
            #[cfg(test)]
            tests::before_transfer_return();
            if !self.epoch_current() {
                output.clear();
                return short(output.frames());
            }
            if let Some(probe) = &self.probe {
                probe.observe(
                    self.carrier.available_frames(),
                    self.drift
                        .as_ref()
                        .map_or(0.0, |drift| drift.servo.correction()),
                    report.xruns + u64::from(produced < output.frames()),
                    u64::from(transferred < required_input) + muted as u64,
                );
            }
            return BoundaryReport {
                transferred_frames: produced.saturating_sub(missing),
                discontinuity: report.discontinuity || produced < output.frames(),
                xruns: report.xruns + u64::from(produced < output.frames()),
            };
        }
        for channel in 0..self.channels {
            for (frame, sample) in output.channel_mut(channel).iter_mut().enumerate() {
                *sample = S::from_f64(f64::from(scratch[frame * 2 + channel]));
            }
        }
        #[cfg(test)]
        tests::before_transfer_return();
        if !self.epoch_current() {
            output.clear();
            return short(output.frames());
        }
        if let Some(probe) = &self.probe {
            probe.observe(self.carrier.available_frames(), 0.0, report.xruns, 0);
        }
        report
    }
}

/// A mono microphone uses carrier slot 0 and initializes slot 1 to exact zero.
pub struct VirtualMicrophoneSink {
    carrier: AudioWriter<f32>,
    channels: usize,
    scratch: Vec<f32>,
    resampler: Option<StereoResampler>,
    engine_sample_rate: u32,
    max_block_frames: usize,
    reset_epoch: Option<Arc<AtomicU64>>,
    observed_reset: u64,
    byte_budget: usize,
    drift: Option<ClockDrift>,
    handshake: Option<EpochHandshake>,
    probe: Option<Arc<BoundaryProbe>>,
}

impl VirtualMicrophoneSink {
    pub fn new(
        carrier: AudioWriter<f32>,
        channels: usize,
        frames: usize,
        engine_sample_rate: u32,
        budget: usize,
    ) -> Result<Self, BackendError> {
        if !(1..=2).contains(&channels)
            || carrier.channel_count() != 2
            || frames == 0
            || !(8_000..=384_000).contains(&engine_sample_rate)
        {
            return Err(invalid_layout());
        }
        let conversion = engine_sample_rate != NATIVE_SAMPLE_RATE;
        let output_frames = if conversion {
            src_input_frames(frames, NATIVE_SAMPLE_RATE, engine_sample_rate)?
        } else {
            frames
        };
        let carrier_bytes = validate_carrier(carrier.capacity_frames(), output_frames)?;
        let capacity = if conversion {
            Some(src_capacity(frames)?)
        } else {
            None
        };
        let scratch = staging(output_frames, budget, carrier_bytes, src_storage(capacity)?)?;
        let src = resampler(engine_sample_rate, NATIVE_SAMPLE_RATE, capacity)?;
        Ok(Self {
            carrier,
            channels,
            scratch,
            resampler: src,
            engine_sample_rate,
            max_block_frames: frames,
            reset_epoch: None,
            observed_reset: 0,
            byte_budget: budget,
            drift: None,
            handshake: None,
            probe: None,
        })
    }

    /// Attach the control-side carrier probe; snapshots land once per block.
    pub fn with_probe(mut self, probe: Arc<BoundaryProbe>) -> Self {
        self.probe = Some(probe);
        self
    }

    /// Off-RT packet-phase/drift adaptation without an additional carrier.
    /// The free-space servo has the inverse sign: more free space means the
    /// FIR must produce faster, not consume Engine input faster.
    /// Uses the same ring-derived, unqualified residency policy as
    /// [`VirtualPlaybackSource::with_clock_drift`], with no extra audio queue.
    pub fn with_clock_drift(mut self) -> Result<Self, BackendError> {
        let transfer = drift_transfer(self.max_block_frames, self.engine_sample_rate)?;
        let drift = ClockDrift::new(self.carrier.capacity_frames(), transfer)?;
        let carrier_bytes = validate_carrier(self.carrier.capacity_frames(), transfer)?;
        let capacity = Some(src_capacity(self.max_block_frames)?);
        let scratch = staging(
            transfer,
            self.byte_budget,
            carrier_bytes,
            src_storage(capacity)?,
        )?;
        let src = resampler(self.engine_sample_rate, NATIVE_SAMPLE_RATE, capacity)?;
        self.scratch = scratch;
        self.resampler = src;
        self.drift = Some(drift);
        Ok(self)
    }

    pub fn with_reset_epoch(mut self, reset_epoch: Arc<AtomicU64>) -> Self {
        self.observed_reset = self.handshake.as_ref().map_or_else(
            || reset_epoch.load(Ordering::Acquire),
            |handshake| handshake.consumer.load(Ordering::Acquire),
        );
        self.reset_epoch = Some(reset_epoch);
        self
    }

    /// Attach the provider's requested epoch and both quiescence fences off RT.
    /// Graph publication pauses until the native consumer acknowledges reset.
    pub fn with_reset_fence(
        self,
        reset_epoch: Arc<AtomicU64>,
        producer_quiesced_epoch: Arc<AtomicU64>,
        consumer_quiesced_epoch: Arc<AtomicU64>,
    ) -> Self {
        self.with_reset_epoch(reset_epoch)
            .with_epoch_handshake(producer_quiesced_epoch, consumer_quiesced_epoch)
    }

    /// The graph producer stops before releasing its fence. Native may then
    /// discard its carrier and acknowledge; graph publication resumes later.
    pub fn with_epoch_handshake(
        mut self,
        producer_quiesced_epoch: Arc<AtomicU64>,
        consumer_quiesced_epoch: Arc<AtomicU64>,
    ) -> Self {
        self.observed_reset = consumer_quiesced_epoch.load(Ordering::Acquire);
        self.handshake = Some(EpochHandshake {
            producer: producer_quiesced_epoch,
            consumer: consumer_quiesced_epoch,
        });
        self
    }

    fn synchronize_epoch(&mut self) -> bool {
        let Some(epoch) = &self.reset_epoch else {
            return true;
        };
        let current = epoch.load(Ordering::Acquire);
        if current == self.observed_reset {
            return true;
        }
        if let Some(src) = &mut self.resampler {
            src.reset();
        }
        if let Some(drift) = &mut self.drift {
            drift.reset();
        }
        if let Some(handshake) = &self.handshake {
            handshake.producer.store(current, Ordering::Release);
            if handshake.consumer.load(Ordering::Acquire) != current {
                return false;
            }
        }
        if epoch.load(Ordering::Acquire) != current {
            return false;
        }
        self.observed_reset = current;
        true
    }

    fn finish_transfer(&mut self, report: BoundaryReport, frames: usize) -> BoundaryReport {
        #[cfg(test)]
        tests::before_transfer_return();
        if self
            .reset_epoch
            .as_ref()
            .is_some_and(|epoch| epoch.load(Ordering::Acquire) != self.observed_reset)
        {
            // A publication may have crossed the request. Releasing producer
            // quiescence only after it completed makes the worker's discard
            // cover that last old write, before any fresh PCM submission.
            self.synchronize_epoch();
            return short(frames);
        }
        report
    }
}

impl<S: ProcessingSample> RtAudioSink<S> for VirtualMicrophoneSink {
    fn channel_count(&self) -> usize {
        self.channels
    }

    fn write(&mut self, _: &ProcessContext, input: AudioBlock<'_, S>) -> BoundaryReport {
        if !self.synchronize_epoch() {
            return short(input.frames());
        }
        if input.channel_count() != self.channels || input.frames() > self.max_block_frames {
            return short(input.frames());
        }
        if input.frames() == 0 {
            return BoundaryReport::default();
        }
        if let Some(drift) = &mut self.drift {
            self.resampler
                .as_mut()
                .expect("drift FIR prepared")
                .set_correction_ppm(-drift.correction(self.carrier.available_frames()));
        }
        if let Some(src) = &mut self.resampler {
            if input.frames() > 0 && self.carrier.available_frames() == 0 {
                // Full-carrier overflow drops the current audio (drop-new) and
                // re-primes the FIR, but no longer requests a boundary reset:
                // the queued frames are still in-order graph output, the
                // worker drains them at its own cadence, and a fenced discard
                // here converted bounded jitter into ~50 ms of guaranteed
                // silence plus a full worker re-handshake.
                if let Some(probe) = &self.probe {
                    probe.observe(
                        self.carrier.capacity_frames(),
                        -self
                            .drift
                            .as_ref()
                            .map_or(0.0, |drift| drift.servo.correction()),
                        1,
                        1,
                    );
                }
                src.reset();
                if let Some(drift) = &mut self.drift {
                    drift.reset();
                }
                return self.finish_transfer(short(input.frames()), input.frames());
            }
            for frame in 0..input.frames() {
                let left = input.channel(0)[frame].to_f64();
                let right = if self.channels == 2 {
                    input.channel(1)[frame].to_f64()
                } else {
                    0.0
                };
                if !src.push_frame([left, right]) {
                    src.reset();
                    return short(input.frames());
                }
            }
            // Drain FIR output even when the consumer is behind. Backpressure
            // drops current audio instead of retaining it for a later client.
            let capacity = self.scratch.len() / 2;
            let mut produced = 0;
            while produced < capacity {
                let Some(frame) = src.next_frame() else {
                    break;
                };
                self.scratch[produced * 2] = frame[0] as f32;
                self.scratch[produced * 2 + 1] = frame[1] as f32;
                produced += 1;
            }
            let report = self
                .carrier
                .write_interleaved(&self.scratch[..produced * 2])
                .expect("complete carrier frames");
            let complete = report.transferred_frames == produced;
            if !complete {
                src.reset();
                if let Some(drift) = &mut self.drift {
                    drift.reset();
                }
            }
            let missing = missing_output_frames(
                produced - report.transferred_frames,
                NATIVE_SAMPLE_RATE,
                self.engine_sample_rate,
            );
            let report = BoundaryReport {
                transferred_frames: input.frames().saturating_sub(missing),
                discontinuity: report.discontinuity,
                xruns: report.xruns,
            };
            if let Some(probe) = &self.probe {
                probe.observe(
                    self.carrier.capacity_frames() - self.carrier.available_frames(),
                    -self
                        .drift
                        .as_ref()
                        .map_or(0.0, |drift| drift.servo.correction()),
                    report.xruns,
                    u64::from(!complete),
                );
            }
            return self.finish_transfer(report, input.frames());
        }
        if input.frames() > self.scratch.len() / 2 {
            return short(input.frames());
        }
        let scratch = &mut self.scratch[..input.frames() * 2];
        for (frame, carrier) in scratch.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            carrier[0] = input.channel(0)[frame].to_f64() as f32;
            carrier[1] = if self.channels == 2 {
                input.channel(1)[frame].to_f64() as f32
            } else {
                0.0
            };
        }
        let report = self
            .carrier
            .write_interleaved(scratch)
            .expect("complete carrier frames");
        if let Some(probe) = &self.probe {
            probe.observe(
                self.carrier.capacity_frames() - self.carrier.available_frames(),
                0.0,
                report.xruns,
                u64::from(report.transferred_frames < input.frames()),
            );
        }
        self.finish_transfer(report, input.frames())
    }
}

#[cfg(test)]
mod tests;
