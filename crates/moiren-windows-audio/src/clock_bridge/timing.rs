//! Packet correlation travels in the same SPSC slot as its accepted audio frame.
use super::CapturePacket;
use crate::stats::{DISCONTINUITY, TIMESTAMP_ERROR};
use std::num::NonZeroU64;

/// Correlation for the oldest pending native frame, not the latest received
/// packet. The QPC stamp belongs to the packet's first frame; the offset stays
/// in native frames. This is neither an audio-age nor a presentation estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedFrameStamp {
    pub generation: u64,
    pub packet_position_frames: u64,
    pub packet_qpc_100ns: u64,
    pub packet_accepted_frames: u64,
    pub packet_offset_frames: u64,
}

#[derive(Clone, Copy)]
pub(super) struct PacketStamp {
    pub position_frames: u64,
    pub qpc_100ns: NonZeroU64,
    pub accepted_frames: u64,
}

#[derive(Clone, Copy)]
struct Anchor {
    position_frames: u64,
    qpc_100ns: NonZeroU64,
    frames: u64,
    generation: u64,
}

#[derive(Default)]
pub(super) struct PacketTiming {
    previous: Option<Anchor>,
}

impl PacketTiming {
    pub fn accept(
        &mut self,
        packet: CapturePacket,
        accepted_frames: usize,
        generation: u64,
        sample_rate: u32,
        position_clock: bool,
    ) -> Option<PacketStamp> {
        let previous = self.previous.take();
        if !position_clock || accepted_frames == 0 || packet.flags & TIMESTAMP_ERROR != 0 {
            return None;
        }
        let qpc_100ns = NonZeroU64::new(packet.qpc_100ns)?;
        packet
            .device_position_frames
            .checked_add(packet.frames as u64)?;
        let current = Anchor {
            position_frames: packet.device_position_frames,
            qpc_100ns,
            frames: packet.frames as u64,
            generation,
        };
        if packet.flags & DISCONTINUITY != 0
            || previous.is_none_or(|anchor| anchor.generation != generation)
        {
            self.previous = Some(current);
            return None;
        }
        let previous = previous.expect("same-generation clock anchor");
        if previous.position_frames.checked_add(previous.frames) != Some(current.position_frames) {
            return None;
        }
        let elapsed = current
            .qpc_100ns
            .get()
            .checked_sub(previous.qpc_100ns.get())?;
        if elapsed == 0 {
            return None;
        }
        let nominal = u128::from(previous.frames) * 10_000_000;
        let observed = u128::from(elapsed) * u128::from(sample_rate);
        // A broad 1% packet-slope gate rejects implausible correlations while
        // allowing 100 ns quantization, even for one-frame packets. It does not
        // estimate drift or change the configured SRC correction limit.
        if nominal.abs_diff(observed) > nominal / 100 {
            return None;
        }
        self.previous = Some(current);
        Some(PacketStamp {
            position_frames: current.position_frames,
            qpc_100ns,
            accepted_frames: accepted_frames as u64,
        })
    }
}
