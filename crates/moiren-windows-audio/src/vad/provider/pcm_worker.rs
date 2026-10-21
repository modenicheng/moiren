//! Ordered PCM pumping: four playback reads, one just-in-time microphone write.
//! IO is injectable so completion ordering and resets are tested without a driver.
use super::*;
use crate::vad::{PcmPacketStamp, SAMPLE_RATE};
use std::time::Instant;

pub(super) trait PcmIo {
    fn wait_activity(&mut self, ticket: PcmTicket, timeout: Duration) -> Result<(), BackendError>;
    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, BackendError>;
    fn read(&mut self) -> Result<PcmTicket, BackendError>;
    fn write(&mut self, packet: &[u8]) -> Result<PcmTicket, BackendError>;
}
impl PcmIo for PcmOwner {
    fn wait_activity(&mut self, ticket: PcmTicket, timeout: Duration) -> Result<(), BackendError> {
        self.wait_activity(ticket, timeout).map_err(client_error)
    }
    fn complete(&mut self, ticket: PcmTicket) -> Result<Option<PcmCompletion<'_>>, BackendError> {
        self.try_complete(ticket).map_err(client_error)
    }
    fn read(&mut self) -> Result<PcmTicket, BackendError> {
        self.submit_read().map_err(client_error)
    }
    fn write(&mut self, packet: &[u8]) -> Result<PcmTicket, BackendError> {
        self.submit_write(packet).map_err(client_error)
    }
}

#[derive(Default)]
pub(super) struct PacketOrder(Option<PcmPacketStamp>);
impl PacketOrder {
    fn accept(&mut self, stamp: PcmPacketStamp) -> bool {
        if self
            .0
            .is_some_and(|last| stamp.sequence <= last.sequence || stamp.qpc < last.qpc)
        {
            return false;
        }
        self.0 = Some(stamp);
        true
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
}

impl MicrophonePump {
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
        }
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
            return false;
        }
        true
    }

    /// Only a gap strictly greater than one native period plus POLL_INTERVAL
    /// resets data. The first observation has no history; this is not a source
    /// sample-age guarantee or a timeout on missing PCM/completions.
    fn observe(&mut self, state: &SessionState, epoch: u64, now: Instant) -> bool {
        if !self.current(state, epoch) {
            return false;
        }
        if self
            .last_observed
            .is_some_and(|last| now.saturating_duration_since(last) > self.observation_gap)
        {
            // One native period plus the bounded IO wait is a timeline gap,
            // not a device fault. Fence and discard even shallow old PCM before
            // collection can rearm its phase; do not revoke native admission.
            let _ = state.reset_epoch.compare_exchange(
                epoch,
                epoch.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return self.current(state, epoch);
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
        match owner.complete(ticket)? {
            Some(PcmCompletion::Render { stamp, pcm }) => {
                if state.reset_epoch.load(Ordering::Acquire) != epoch {
                    return Ok(PumpState::Reset);
                }
                #[cfg(test)]
                tests::before_playback_publish();
                if order.accept(stamp) && !bridge.decode_packet(pcm)? {
                    // Only the consumer can discard this SPSC carrier. Ask its
                    // prepared boundary to drop the stale queue before replay.
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
        tickets.push(owner.read()?);
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
        let (stamp, dropped) = match owner.complete(ticket)? {
            Some(PcmCompletion::Capture { stamp, bytes }) if bytes == bridge.packet_bytes() => {
                (stamp, false)
            }
            Some(PcmCompletion::CaptureDropped { stamp }) => {
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
        if !pump.order.accept(stamp) {
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
        // Do not consume the carrier or encode until the phase is due. The
        // unchanged PcmOwner samples its real QPC only when submitting this PCM.
        let packet = bridge.encode_packet()?;
        if !pump.observe(state, epoch, now()) {
            return Ok(PumpState::Reset);
        }
        let submitted_at = now();
        tickets.push(owner.write(packet)?);
        pump.last_phase = pump
            .next_submit
            .take()
            .filter(|&phase| submitted_at.saturating_duration_since(phase) < pump.period);
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
        owner.wait_activity(
            ticket,
            pump.remaining(now)
                .unwrap_or(POLL_INTERVAL)
                .min(POLL_INTERVAL),
        )?;
    } else if let Some(remaining) = pump.remaining(now)
        && !remaining.is_zero()
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
        owner.wait_activity(ticket, POLL_INTERVAL)?;
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
