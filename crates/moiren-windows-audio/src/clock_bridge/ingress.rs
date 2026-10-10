//! Native packet decoding and generation publication; no driver pointer escapes.
use super::{ClockBridgeConfig, ClockBridgeError, Frame, telemetry::Counters};
use crate::stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR};
use rtrb::Producer;
use std::sync::{
    Arc,
    atomic::Ordering::{Relaxed, Release},
};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct CapturePacket {
    pub frames: usize,
    pub flags: u32,
    pub device_position_frames: u64,
    pub qpc_100ns: u64,
}
pub struct CaptureIngress {
    tx: Producer<Frame>,
    config: ClockBridgeConfig,
    generation: u64,
    next_position: Option<u64>,
    counters: Arc<Counters>,
    last_packet_at: Option<Instant>,
}
impl CaptureIngress {
    pub(super) fn new(
        tx: Producer<Frame>,
        config: ClockBridgeConfig,
        counters: Arc<Counters>,
    ) -> Self {
        Self {
            tx,
            config,
            generation: 1,
            next_position: None,
            counters,
            last_packet_at: None,
        }
    }
    pub fn input_channels(&self) -> usize {
        self.config.input_channels
    }

    /// Retire publication before blocking native Stop/COM cleanup. The ring
    /// stays alive for finite consumers, but no new packets may follow finish.
    pub fn finish(&mut self) {
        self.counters.producer_finished.store(true, Release);
    }

    /// SILENT packets may have no readable data. Overflow accepts a whole-frame
    /// prefix and drops the new tail; the next packet starts a fresh generation.
    pub fn push_packet(
        &mut self,
        bytes: &[u8],
        packet: CapturePacket,
    ) -> Result<usize, ClockBridgeError> {
        if self.counters.producer_finished.load(Relaxed) {
            return Err(ClockBridgeError::ProducerFinished);
        }
        let silent = packet.flags & SILENT != 0;
        let width = self.config.input_channels * 4;
        let expected = packet
            .frames
            .checked_mul(width)
            .ok_or(ClockBridgeError::InvalidPacket)?;
        if !silent && bytes.len() != expected {
            return Err(ClockBridgeError::InvalidPacket);
        }
        if packet.frames == 0 {
            return Ok(0);
        }
        // Scalar diagnostics stay on the capture owner. Normal render demand
        // uses no clock reads; only its first shortfall samples publication age.
        let now = Instant::now();
        self.counters.last_packet_elapsed_us.store(
            now.duration_since(self.counters.prepared_at)
                .as_micros()
                .min(u64::MAX as u128) as u64,
            Relaxed,
        );
        let index = self.counters.packets.fetch_add(1, Relaxed);
        if index == 0 {
            self.counters
                .first_packet_frames
                .store(packet.frames, Relaxed);
        }
        self.counters
            .max_packet_frames
            .fetch_max(packet.frames, Relaxed);
        if let Some(previous) = self.last_packet_at {
            let interval = now
                .duration_since(previous)
                .as_micros()
                .min(u64::MAX as u128) as u64;
            self.counters
                .max_packet_interval_us
                .fetch_max(interval, Relaxed);
            // Keep cold startup separate from later scheduling stalls/pauses.
            if index < 16 {
                let previous_max = self
                    .counters
                    .max_startup_packet_interval_us
                    .fetch_max(interval, Relaxed);
                if interval > previous_max {
                    self.counters
                        .max_startup_gap_after_packet
                        .store(index, Relaxed);
                    self.counters
                        .max_startup_gap_at_output_frame
                        .store(self.counters.output_frames.load(Relaxed), Relaxed);
                    self.counters.max_startup_gap_qpc_interval_us.store(
                        packet
                            .qpc_100ns
                            .saturating_sub(self.counters.last_qpc_100ns.load(Relaxed))
                            / 10,
                        Relaxed,
                    );
                }
            }
        }
        self.last_packet_at = Some(now);
        let invalid_time = packet.flags & TIMESTAMP_ERROR != 0 || packet.qpc_100ns == 0;
        let gap = self.config.detect_position_gaps
            && !invalid_time
            && self
                .next_position
                .is_some_and(|p| p != packet.device_position_frames);
        if gap || packet.flags & DISCONTINUITY != 0 {
            self.generation = self.generation.wrapping_add(1);
            self.counters.discontinuities.fetch_add(1, Relaxed);
        }
        self.next_position = if invalid_time {
            None
        } else {
            packet
                .device_position_frames
                .checked_add(packet.frames as u64)
        };
        if invalid_time {
            self.counters.timestamp_errors.fetch_add(1, Relaxed);
        } else {
            self.counters
                .last_device_position_frames
                .store(packet.device_position_frames, Relaxed);
            self.counters
                .last_qpc_100ns
                .store(packet.qpc_100ns, Relaxed);
        }
        self.counters
            .captured_frames
            .fetch_add(packet.frames as u64, Relaxed);
        if silent {
            self.counters
                .silent_frames
                .fetch_add(packet.frames as u64, Relaxed);
        }
        let accepted = if self.tx.is_abandoned() {
            0
        } else {
            packet.frames.min(self.tx.slots())
        };
        let mut nonfinite = 0u64;
        if accepted > 0 {
            let generation = self.generation;
            let channels = self.config.input_channels;
            let chunk = self
                .tx
                .write_chunk_uninit(accepted)
                .expect("reserved capture frames");
            chunk.fill_from_iter((0..accepted).map(|frame| {
                let mut samples = [0.0; 2];
                if !silent {
                    for (ch, sample) in samples.iter_mut().enumerate().take(channels) {
                        let offset = frame * width + ch * 4;
                        let value = f32::from_le_bytes(
                            bytes[offset..offset + 4]
                                .try_into()
                                .expect("validated f32 width"),
                        );
                        if value.is_finite() {
                            *sample = value;
                        } else {
                            nonfinite += 1;
                        }
                    }
                    if channels == 1 {
                        samples[1] = samples[0];
                    }
                }
                Frame {
                    samples,
                    generation,
                }
            }));
        }
        self.counters
            .nonfinite_samples
            .fetch_add(nonfinite, Relaxed);
        self.counters
            .written_frames
            .fetch_add(accepted as u64, Relaxed);
        self.counters
            .dropped_frames
            .fetch_add((packet.frames - accepted) as u64, Relaxed);
        if accepted < packet.frames {
            self.generation = self.generation.wrapping_add(1);
        }
        Ok(accepted)
    }
}
impl Drop for CaptureIngress {
    fn drop(&mut self) {
        self.finish();
    }
}
