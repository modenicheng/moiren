//! Ordered PCM pumping: four playback reads, one just-in-time microphone write.
//! IO is injectable so completion ordering and resets are tested without a driver.
use super::*;
use crate::vad::{PcmPacketStamp, SAMPLE_RATE};
use std::time::Instant;

pub(super) trait PcmIo {
    fn wait_activity(&mut self, ticket: PcmTicket, timeout: Duration) -> Result<(), ClientError>;
    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError>;
    fn read(&mut self) -> Result<PcmTicket, ClientError>;
    fn write(&mut self, packet: &[u8]) -> Result<PcmTicket, ClientError>;
}
impl PcmIo for PcmOwner {
    fn wait_activity(&mut self, ticket: PcmTicket, timeout: Duration) -> Result<(), ClientError> {
        self.wait_activity(ticket, timeout)
    }
    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        self.try_complete(ticket)
    }
    fn read(&mut self) -> Result<PcmTicket, ClientError> {
        self.submit_read()
    }
    fn write(&mut self, packet: &[u8]) -> Result<PcmTicket, ClientError> {
        self.submit_write(packet)
    }
}

#[derive(Default)]
pub(super) struct PacketOrder(Option<(u64, PcmPacketStamp)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacketProgress {
    First,
    Contiguous,
    Gap { missing_slots: u64 },
    Stale,
}

impl PacketOrder {
    fn accept(&mut self, epoch: u64, stamp: PcmPacketStamp) -> PacketProgress {
        let progress = match self.0.filter(|(last_epoch, _)| *last_epoch == epoch) {
            None => PacketProgress::First,
            Some((_, last)) if stamp.sequence <= last.sequence || stamp.qpc < last.qpc => {
                return PacketProgress::Stale;
            }
            Some((_, last)) => {
                // Strictly increasing sequence makes both subtractions safe,
                // including MAX. Never infer the unobserved initial prefix.
                let missing_slots = stamp.sequence - last.sequence - 1;
                if missing_slots == 0 {
                    PacketProgress::Contiguous
                } else {
                    PacketProgress::Gap { missing_slots }
                }
            }
        };
        self.0 = Some((epoch, stamp));
        progress
    }
}

pub(super) struct MicrophonePump {
    order: PacketOrder,
    half_period: Duration,
    period: Duration,
    next_submit: Option<Instant>,
    last_phase: Option<Instant>,
    observation_gap: Duration,
    last_observed: Option<Instant>,
    /// Earliest instant the first fresh submission may go out while the
    /// carrier has not yet reached `reprime_frames` (see `arm_reprime`).
    reprime_deadline: Option<Instant>,
    reprime_frames: usize,
    reprime_wait: Duration,
}

impl MicrophonePump {
    pub(super) fn prepared(policy: crate::virtual_owner::PreparedMicrophoneReprime) -> Self {
        let mut pump = Self::new(policy.packet_frames);
        pump.reprime_frames = policy.required_frames as usize;
        pump.reprime_wait = Duration::new(
            policy.max_wait_100ns / 10_000_000,
            ((policy.max_wait_100ns % 10_000_000) * 100) as u32,
        );
        pump
    }

    pub(super) fn new(packet_frames: u32) -> Self {
        Self {
            order: PacketOrder::default(),
            half_period: Duration::from_nanos(
                (u64::from(packet_frames) * 1_000_000_000).div_ceil(u64::from(SAMPLE_RATE) * 2),
            ),
            period: Duration::from_nanos(
                (u64::from(packet_frames) * 1_000_000_000).div_ceil(u64::from(SAMPLE_RATE)),
            ),
            next_submit: None,
            last_phase: None,
            observation_gap: Duration::from_nanos(
                (u64::from(packet_frames) * 1_000_000_000).div_ceil(u64::from(SAMPLE_RATE)),
            ) + POLL_INTERVAL,
            last_observed: None,
            reprime_deadline: None,
            reprime_frames: usize::try_from(packet_frames).unwrap_or(0) * 2,
            reprime_wait: Duration::from_nanos(
                (u64::from(packet_frames) * 1_000_000_000).div_ceil(u64::from(SAMPLE_RATE)),
            ) * 5
                / 2,
        }
    }

    /// After a completed hard reset the carrier was discarded at level zero
    /// while production roughly equals device-clock consumption: the level
    /// cannot rebuild on servo authority alone (+/-2000 ppm is ~96 frames/s
    /// against a 480-frame packet cadence), so every packet would carry a
    /// silent tail for ~20 s. Hold the first fresh submission for the prepared
    /// whole-Engine-wake margin, with one finite deadline. Missing production
    /// still falls back to counted silence; native drops remain counted too.
    /// This is recovery readiness, not a physical latency guarantee. Fresh
    /// streams keep their immediate first submission and native phase.
    pub(super) fn arm_reprime(&mut self, now: Instant) -> Result<(), BackendError> {
        self.reprime_deadline =
            Some(now.checked_add(self.reprime_wait).ok_or_else(|| {
                BackendError::Failed("microphone re-prime deadline overflow".into())
            })?);
        Ok(())
    }

    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.next_submit
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    fn current(&mut self, state: &SessionState, epoch: u64) -> bool {
        if state.cancel.load(Ordering::Acquire) || !epoch_ready(state, epoch) {
            self.order = PacketOrder::default();
            self.next_submit = None;
            self.last_phase = None;
            self.last_observed = None;
            self.reprime_deadline = None;
            return false;
        }
        true
    }

    /// Only a gap strictly greater than one native period plus POLL_INTERVAL
    /// degrades pumping, and it degrades softly: the phase anchor is stale,
    /// not the carrier content — in-order graph output behind an unchanged
    /// epoch. The next successful collection re-arms the phase from its own
    /// timestamp exactly like a dropped completion; no epoch is bumped and
    /// no data is discarded (the fenced discard stays reserved for real
    /// epoch events). The first observation has no history; this is not a
    /// source sample-age guarantee or a timeout on missing PCM/completions.
    fn observe(&mut self, state: &SessionState, epoch: u64, now: Instant) -> bool {
        if !self.current(state, epoch) {
            return false;
        }
        if self
            .last_observed
            .is_some_and(|last| now.saturating_duration_since(last) > self.observation_gap)
        {
            self.order = PacketOrder::default();
            self.next_submit = None;
            self.last_phase = None;
            log::debug!(target: "moiren::audio::vad", "event=microphone_observation_gap_soft_reset");
        }
        self.last_observed = Some(now);
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PumpState {
    Current,
    Reset,
}

/// Retire every old native request before releasing the producer fence. The
/// boundary then discards the ring and acknowledges it before fresh publication.
pub(super) fn quiesce_playback(
    state: &SessionState,
    observed: &mut u64,
    retire: impl FnOnce(),
) -> bool {
    let epoch = state.reset_epoch.load(Ordering::Acquire);
    if epoch == *observed {
        return true;
    }
    state.telemetry.epoch_reset(epoch);
    retire();
    state
        .producer_quiesced_epoch
        .store(epoch, Ordering::Release);
    if state.consumer_quiesced_epoch.load(Ordering::Acquire) != epoch
        || state.reset_epoch.load(Ordering::Acquire) != epoch
    {
        return false;
    }
    *observed = epoch;
    true
}

/// Call only after retiring the old PcmOwner and recovering its carrier reader.
/// The graph producer stays stopped between its release and our acknowledgment,
/// so this discard includes even an old write that completed after the reset.
pub(super) fn quiesce_microphone(
    state: &SessionState,
    observed: &mut u64,
    reader: &mut AudioReader<f32>,
) -> bool {
    let epoch = state.reset_epoch.load(Ordering::Acquire);
    if epoch == *observed {
        return true;
    }
    state.telemetry.epoch_reset(epoch);
    if state.producer_quiesced_epoch.load(Ordering::Acquire) != epoch {
        return false;
    }
    reader.discard();
    state
        .consumer_quiesced_epoch
        .store(epoch, Ordering::Release);
    if state.reset_epoch.load(Ordering::Acquire) != epoch {
        return false;
    }
    *observed = epoch;
    true
}

pub(super) fn playback<C: PcmIo>(
    owner: &mut C,
    bridge: &mut PcmPlaybackBridge,
    tickets: &mut Vec<PcmTicket>,
    order: &mut PacketOrder,
    state: &SessionState,
    epoch: u64,
) -> Result<PumpState, BackendError> {
    if !epoch_ready(state, epoch) {
        return Ok(PumpState::Reset);
    }
    while let Some(ticket) = tickets.first().copied() {
        if state.reset_epoch.load(Ordering::Acquire) != epoch {
            return Ok(PumpState::Reset);
        }
        // A kernel quota completion is terminal for the ticket but transient
        // for the owner: drop it and refill on a later pass. Propagating it
        // would fail the whole session while a retired owner's cancelled
        // requests still occupy the endpoint quota (kernel audit 2026-10-24).
        let completion = match owner.complete(ticket) {
            Ok(completion) => completion,
            Err(ClientError::KernelPcmQuotaFull) => {
                log::debug!(
                    target: "moiren::audio::vad",
                    "event=pcm_kernel_quota_deferred endpoint=playback stage=complete"
                );
                tickets.remove(0);
                break;
            }
            Err(error) => return Err(client_error(error)),
        };
        match completion {
            Some(PcmCompletion::Render { stamp, pcm }) => {
                if state.reset_epoch.load(Ordering::Acquire) != epoch {
                    return Ok(PumpState::Reset);
                }
                #[cfg(test)]
                tests::before_playback_publish();
                if !epoch_ready(state, epoch) {
                    return Ok(PumpState::Reset);
                }
                state.telemetry.completion(bridge.packet_frames());
                let accepted = match order.accept(epoch, stamp) {
                    PacketProgress::Stale => {
                        state.telemetry.stale_packet();
                        false
                    }
                    PacketProgress::Gap { missing_slots } => {
                        // READ stamps describe native playback slots. A hole
                        // does not prove non-silent PCM loss and is not a reset.
                        state.telemetry.sequence_gap(missing_slots);
                        true
                    }
                    PacketProgress::First | PacketProgress::Contiguous => true,
                };
                if accepted && !bridge.decode_packet(pcm)? {
                    // Only the consumer can discard this SPSC carrier. Ask its
                    // prepared boundary to drop the stale queue before replay.
                    state.telemetry.carrier_drop();
                    state.reset_epoch.fetch_add(1, Ordering::AcqRel);
                    return Ok(PumpState::Reset);
                }
                tickets.remove(0);
            }
            Some(_) => {
                return Err(BackendError::Failed(
                    "playback returned capture data".into(),
                ));
            }
            None => break,
        }
    }
    while tickets.len() < super::super::REQUEST_SLOTS {
        if state.reset_epoch.load(Ordering::Acquire) != epoch {
            return Ok(PumpState::Reset);
        }
        state
            .telemetry
            .submission_attempt(bridge.packet_frames(), 0);
        match owner.read() {
            Ok(ticket) => {
                state.telemetry.submission_accepted();
                tickets.push(ticket);
            }
            Err(ClientError::KernelPcmQuotaFull) => {
                state.telemetry.quota_drop();
                log::debug!(
                    target: "moiren::audio::vad",
                    "event=pcm_kernel_quota_deferred endpoint=playback stage=read"
                );
                break;
            }
            Err(error) => return Err(client_error(error)),
        }
    }
    Ok(PumpState::Current)
}

pub(super) fn microphone<C: PcmIo>(
    owner: &mut C,
    bridge: &mut PcmMicrophoneBridge,
    tickets: &mut Vec<PcmTicket>,
    pump: &mut MicrophonePump,
    state: &SessionState,
    epoch: u64,
    mut now: impl FnMut() -> Instant,
) -> Result<PumpState, BackendError> {
    if !pump.observe(state, epoch, now()) {
        return Ok(PumpState::Reset);
    }
    while let Some(ticket) = tickets.first().copied() {
        if !pump.observe(state, epoch, now()) {
            return Ok(PumpState::Reset);
        }
        // Quota completions are terminal for the ticket, transient for the
        // owner (see `playback`); drop the ticket and let the fill path
        // resubmit once the retired owner's requests stop occupying quota.
        let completion = match owner.complete(ticket) {
            Ok(completion) => completion,
            Err(ClientError::KernelPcmQuotaFull) => {
                state.telemetry.quota_drop();
                log::debug!(
                    target: "moiren::audio::vad",
                    "event=pcm_kernel_quota_deferred endpoint=microphone stage=complete"
                );
                tickets.remove(0);
                break;
            }
            Err(error) => return Err(client_error(error)),
        };
        let (stamp, dropped) = match completion {
            Some(PcmCompletion::Capture { stamp, bytes }) if bytes == bridge.packet_bytes() => {
                state.telemetry.completion(bridge.packet_frames());
                (stamp, false)
            }
            Some(PcmCompletion::CaptureDropped { stamp }) => {
                state.telemetry.completion(bridge.packet_frames());
                state.telemetry.driver_drop();
                log::debug!(
                    target: "moiren::audio::vad",
                    "event=microphone_request_expired epoch={epoch} stamp={stamp:?}",
                );
                (stamp, true)
            }
            Some(_) => {
                return Err(BackendError::Failed(
                    "microphone completion shape changed".into(),
                ));
            }
            None => break,
        };
        let collected_at = now();
        if !pump.observe(state, epoch, collected_at) {
            return Ok(PumpState::Reset);
        }
        // WRITE completion echoes a host submission sequence, not the native
        // packet cursor. Keep order validation, never report native slot loss.
        if pump.order.accept(epoch, stamp) == PacketProgress::Stale {
            return Err(BackendError::Failed(
                "microphone completion sequence regressed".into(),
            ));
        }
        tickets.remove(0);
        // Only the first completion establishes the halfway phase. Later
        // collection can be delayed by scheduling or control IO; using it as
        // a fresh origin would push a future WRITE past the native deadline.
        // A successful but late collection submits one freshly encoded packet
        // immediately, never a burst. Expiry cannot establish a native phase.
        let cadence = (!dropped)
            .then_some(pump.last_phase)
            .flatten()
            .and_then(|phase| phase.checked_add(pump.period))
            .map(|phase| phase.max(collected_at));
        pump.next_submit = Some(match cadence {
            Some(phase) => phase,
            None => collected_at
                .checked_add(pump.half_period)
                .ok_or_else(|| BackendError::Failed("microphone phase deadline overflow".into()))?,
        });
    }
    if tickets.is_empty()
        && pump
            .remaining(now())
            .is_none_or(|remaining| remaining.is_zero())
    {
        if !pump.observe(state, epoch, now()) {
            return Ok(PumpState::Reset);
        }
        // Bounded re-prime after a hard reset: hold the first fresh
        // submission until the carrier holds the prepared recovery margin or the
        // deadline expires; wait_for_microphone yields in 1 ms slices.
        let reprime_hold = match pump.reprime_deadline {
            Some(deadline) => bridge.available_frames() < pump.reprime_frames && now() < deadline,
            None => false,
        };
        if reprime_hold {
            return Ok(PumpState::Current);
        }
        pump.reprime_deadline = None;
        // Do not consume the carrier or encode until the phase is due. The
        // unchanged PcmOwner samples its real QPC only when submitting this PCM.
        let packet_frames = bridge.packet_frames();
        let (packet, silence_frames) = bridge.encode_packet_with_metrics()?;
        state
            .telemetry
            .submission_attempt(packet_frames, silence_frames);
        if !pump.observe(state, epoch, now()) {
            state.telemetry.carrier_drop();
            return Ok(PumpState::Reset);
        }
        let submitted_at = now();
        match owner.write(packet) {
            Ok(ticket) => {
                state.telemetry.submission_accepted();
                tickets.push(ticket);
                pump.last_phase = pump
                    .next_submit
                    .take()
                    .filter(|&phase| submitted_at.saturating_duration_since(phase) < pump.period);
            }
            // The packet was already consumed from the carrier: the kernel
            // quota rejection is a bounded drop (the driver's own backpressure
            // semantics), never a session fault. The phase stays due, so the
            // next pass encodes the next packet immediately.
            Err(ClientError::KernelPcmQuotaFull) => {
                state.telemetry.quota_drop();
                log::debug!(
                    target: "moiren::audio::vad",
                    "event=pcm_kernel_quota_dropped endpoint=microphone stage=write"
                );
            }
            Err(error) => return Err(client_error(error)),
        }
    }
    Ok(PumpState::Current)
}

pub(super) fn wait_for_microphone<C: PcmIo>(
    owner: &mut C,
    tickets: &[PcmTicket],
    pump: &mut MicrophonePump,
    state: &SessionState,
    epoch: u64,
    now: Instant,
    sleep: impl FnOnce(Duration),
) -> Result<(), BackendError> {
    if !pump.current(state, epoch) {
        return Ok(());
    }
    if let Some(&ticket) = tickets.first() {
        owner
            .wait_activity(
                ticket,
                pump.remaining(now)
                    .unwrap_or(POLL_INTERVAL)
                    .min(POLL_INTERVAL),
            )
            .map_err(client_error)?;
    } else if let Some(remaining) = pump
        .remaining(now)
        .filter(|remaining| !remaining.is_zero())
        .or_else(|| {
            pump.reprime_deadline
                .map(|deadline| deadline.saturating_duration_since(now))
        })
    {
        // This is the non-RT owner. Yield between control polls without a
        // fixed 10 ms overshoot or spinning while no native request is pending.
        sleep(remaining.min(Duration::from_millis(1)));
    }
    Ok(())
}

/// Native events pace the IO worker, not Engine render. Waiting for the first
/// submitted ticket preserves ordered collection even if later slots complete.
pub(super) fn wait_for_activity<C: PcmIo>(
    owner: &mut C,
    tickets: &[PcmTicket],
    state: &SessionState,
    epoch: u64,
) -> Result<(), BackendError> {
    if state.cancel.load(Ordering::Acquire) || !epoch_ready(state, epoch) {
        return Ok(());
    }
    if let Some(&ticket) = tickets.first() {
        owner
            .wait_activity(ticket, POLL_INTERVAL)
            .map_err(client_error)?;
    }
    Ok(())
}

fn epoch_ready(state: &SessionState, epoch: u64) -> bool {
    state.reset_epoch.load(Ordering::Acquire) == epoch
        && state.producer_quiesced_epoch.load(Ordering::Acquire) == epoch
        && state.consumer_quiesced_epoch.load(Ordering::Acquire) == epoch
}

#[cfg(test)]
mod tests;
