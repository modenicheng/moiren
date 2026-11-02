use crate::stats::{SILENT, TIMESTAMP_ERROR};
use serde::Serialize;
use thiserror::Error;

const BYTES_PER_F32: usize = std::mem::size_of::<f32>();
const MAX_PACKET_HISTORY: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmArtifactConfig {
    pub capacity_frames: usize,
    /// Zero preserves PCM-only behavior; packet storage is prepared before Start.
    pub packet_capacity: usize,
}

impl PcmArtifactConfig {
    pub const fn new(capacity_frames: usize) -> Self {
        Self {
            capacity_frames,
            packet_capacity: 0,
        }
    }

    pub const fn with_packet_capacity(mut self, packets: usize) -> Self {
        self.packet_capacity = packets;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PcmArtifactError {
    #[error("PCM artifact capacity must be nonzero")]
    ZeroCapacity,
    #[error("PCM artifact channel count must be between 1 and 32")]
    InvalidChannels,
    #[error("PCM artifact sample rate must be nonzero")]
    InvalidSampleRate,
    #[error("PCM artifact storage size overflow")]
    StorageOverflow,
    #[error("PCM artifact packet history exceeds its 65536-record preparation budget")]
    PacketHistoryBudget,
    #[error("PCM artifact packet length does not match the configured format")]
    InvalidPacket,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PcmArtifactMetadata {
    pub schema_version: u32,
    pub sample_rate: u32,
    pub channels: usize,
    pub capacity_frames: usize,
    pub packets: u64,
    pub observed_frames: u64,
    pub stored_frames: u64,
    pub dropped_frames: u64,
    pub silent_frames: u64,
    pub overflow_events: u64,
    /// False means gap counters cannot establish device-position continuity.
    pub detect_position_gaps: bool,
    pub gap_events: u64,
    pub gap_frames: u64,
    pub discontinuity_events: u64,
    pub timestamp_error_packets: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_history: Option<PcmPacketHistoryMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PcmPacketHistoryMetadata {
    pub capacity_packets: usize,
    pub stored_packets: u64,
    pub omitted_packets: u64,
    pub omitted_frames: u64,
    pub counters_saturated: bool,
    /// All offered packet records retained, not PCM/timestamp/quality acceptance.
    pub complete: bool,
}

/// An actual GetBuffer offer, before ReleaseBuffer; no native epoch is inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PcmPacketRecord {
    pub packet_sequence: u64,
    pub observed_frame_start: u64,
    pub stored_frame_start: u64,
    pub frames: u32,
    pub stored_frames: u32,
    pub flags: u32,
    pub device_position_frames: u64,
    /// None for the legacy unstamped API; native zero is retained as Some(0).
    pub qpc_100ns: Option<u64>,
}

/// A completed, bounded PCM capture. The byte payload is deliberately kept
/// outside `CaptureReport` serialization; callers decide when to persist it.
#[derive(Debug)]
pub struct PcmArtifact {
    metadata: PcmArtifactMetadata,
    pcm: Vec<u8>,
    packets: Vec<PcmPacketRecord>,
}

impl PcmArtifact {
    pub fn metadata(&self) -> PcmArtifactMetadata {
        self.metadata
    }

    /// Interleaved little-endian f32 samples, limited to `stored_frames`.
    pub fn pcm_bytes(&self) -> &[u8] {
        &self.pcm
    }

    pub fn packet_history(&self) -> &[PcmPacketRecord] {
        &self.packets
    }

    /// Legacy PCM-only decomposition; use `into_parts_with_packet_history` to
    /// retain the separately opted-in packet payload.
    pub fn into_parts(self) -> (PcmArtifactMetadata, Vec<u8>) {
        (self.metadata, self.pcm)
    }

    pub fn into_parts_with_packet_history(
        self,
    ) -> (PcmArtifactMetadata, Vec<u8>, Vec<PcmPacketRecord>) {
        (self.metadata, self.pcm, self.packets)
    }
}

#[derive(Debug)]
pub struct PcmRecorder {
    metadata: PcmArtifactMetadata,
    frame_bytes: usize,
    pcm: Vec<u8>,
    next_position: Option<u64>,
    packets: Vec<PcmPacketRecord>,
}

impl PcmRecorder {
    pub fn new(
        config: PcmArtifactConfig,
        sample_rate: u32,
        channels: usize,
    ) -> Result<Self, PcmArtifactError> {
        Self::new_with_position_gaps(config, sample_rate, channels, true)
    }

    pub(crate) fn new_with_position_gaps(
        config: PcmArtifactConfig,
        sample_rate: u32,
        channels: usize,
        detect_position_gaps: bool,
    ) -> Result<Self, PcmArtifactError> {
        if config.capacity_frames == 0 {
            return Err(PcmArtifactError::ZeroCapacity);
        }
        if config.packet_capacity > MAX_PACKET_HISTORY {
            return Err(PcmArtifactError::PacketHistoryBudget);
        }
        if sample_rate == 0 {
            return Err(PcmArtifactError::InvalidSampleRate);
        }
        if !(1..=32).contains(&channels) {
            return Err(PcmArtifactError::InvalidChannels);
        }
        let frame_bytes = channels
            .checked_mul(BYTES_PER_F32)
            .ok_or(PcmArtifactError::StorageOverflow)?;
        let bytes = config
            .capacity_frames
            .checked_mul(frame_bytes)
            .ok_or(PcmArtifactError::StorageOverflow)?;
        Ok(Self {
            metadata: PcmArtifactMetadata {
                schema_version: if config.packet_capacity == 0 { 2 } else { 3 },
                sample_rate,
                channels,
                capacity_frames: config.capacity_frames,
                detect_position_gaps,
                packet_history: (config.packet_capacity > 0).then_some(PcmPacketHistoryMetadata {
                    capacity_packets: config.packet_capacity,
                    stored_packets: 0,
                    omitted_packets: 0,
                    omitted_frames: 0,
                    counters_saturated: false,
                    complete: true,
                }),
                ..PcmArtifactMetadata::default()
            },
            frame_bytes,
            pcm: vec![0; bytes],
            next_position: None,
            packets: Vec::with_capacity(config.packet_capacity),
        })
    }

    /// Copy one already-borrowed WASAPI packet. This method never grows the
    /// backing allocation; bytes beyond the configured bound are counted.
    pub fn record_packet(
        &mut self,
        bytes: &[u8],
        frames: u32,
        flags: u32,
        device_position_frames: u64,
    ) -> Result<(), PcmArtifactError> {
        self.record_packet_inner(bytes, frames, flags, device_position_frames, None)
    }

    /// The caller supplies the already-returned native QPC, never a new clock read.
    pub fn record_native_packet(
        &mut self,
        bytes: &[u8],
        packet: super::PcmEvidencePacket,
    ) -> Result<(), PcmArtifactError> {
        self.record_packet_inner(
            bytes,
            packet.frames,
            packet.flags,
            packet.device_position_frames,
            Some(packet.qpc_100ns),
        )
    }

    fn record_packet_inner(
        &mut self,
        bytes: &[u8],
        frames: u32,
        flags: u32,
        device_position_frames: u64,
        qpc_100ns: Option<u64>,
    ) -> Result<(), PcmArtifactError> {
        if frames == 0 {
            return Ok(());
        }
        let frames = frames as usize;
        let packet_bytes = frames
            .checked_mul(self.frame_bytes)
            .ok_or(PcmArtifactError::InvalidPacket)?;
        if flags & SILENT == 0 && bytes.len() != packet_bytes {
            return Err(PcmArtifactError::InvalidPacket);
        }
        let observed_frame_start = self.metadata.observed_frames;
        let stored_frame_start = self.metadata.stored_frames;
        self.metadata.packets = self.metadata.packets.saturating_add(1);
        self.metadata.observed_frames = self.metadata.observed_frames.saturating_add(frames as u64);
        if flags & SILENT != 0 {
            self.metadata.silent_frames = self.metadata.silent_frames.saturating_add(frames as u64);
        }
        if flags & crate::stats::DISCONTINUITY != 0 {
            self.metadata.discontinuity_events =
                self.metadata.discontinuity_events.saturating_add(1);
        }
        if flags & TIMESTAMP_ERROR != 0 {
            self.metadata.timestamp_error_packets =
                self.metadata.timestamp_error_packets.saturating_add(1);
            self.next_position = None;
        } else if self.metadata.detect_position_gaps {
            if let Some(expected) = self.next_position
                && expected != device_position_frames
            {
                self.metadata.gap_events = self.metadata.gap_events.saturating_add(1);
                self.metadata.gap_frames = self
                    .metadata
                    .gap_frames
                    .saturating_add(expected.abs_diff(device_position_frames));
            }
            self.next_position = device_position_frames.checked_add(frames as u64);
        }

        let remaining = self
            .metadata
            .capacity_frames
            .saturating_sub(self.metadata.stored_frames as usize);
        let stored = remaining.min(frames);
        if stored < frames {
            self.metadata.dropped_frames = self
                .metadata
                .dropped_frames
                .saturating_add((frames - stored) as u64);
            self.metadata.overflow_events = self.metadata.overflow_events.saturating_add(1);
        }
        if stored > 0 {
            let destination_start = self.metadata.stored_frames as usize * self.frame_bytes;
            let destination_end = destination_start + stored * self.frame_bytes;
            if flags & SILENT != 0 {
                self.pcm[destination_start..destination_end].fill(0);
            } else {
                self.pcm[destination_start..destination_end]
                    .copy_from_slice(&bytes[..stored * self.frame_bytes]);
            }
            self.metadata.stored_frames = self.metadata.stored_frames.saturating_add(stored as u64);
        }
        if let Some(history) = self.metadata.packet_history.as_mut() {
            if self.metadata.packets == u64::MAX || self.metadata.observed_frames == u64::MAX {
                history.counters_saturated = true;
                history.complete = false;
            }
            if self.packets.len() < history.capacity_packets {
                self.packets.push(PcmPacketRecord {
                    packet_sequence: self.metadata.packets,
                    observed_frame_start,
                    stored_frame_start,
                    frames: frames as u32,
                    stored_frames: stored as u32,
                    flags,
                    device_position_frames,
                    qpc_100ns,
                });
                history.stored_packets += 1;
            } else {
                // Saturating loss counters never overwrite the retained prefix,
                // resize storage, block capture, or turn omission into success.
                history.complete = false;
                history.omitted_packets = history.omitted_packets.saturating_add(1);
                history.omitted_frames = history.omitted_frames.saturating_add(frames as u64);
                history.counters_saturated |=
                    history.omitted_packets == u64::MAX || history.omitted_frames == u64::MAX;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> PcmArtifact {
        self.pcm
            .truncate(self.metadata.stored_frames as usize * self.frame_bytes);
        PcmArtifact {
            metadata: self.metadata,
            pcm: self.pcm,
            packets: self.packets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_preallocates_bounds_storage_and_reports_overflow() {
        let mut recorder = PcmRecorder::new(PcmArtifactConfig::new(2), 48_000, 2).unwrap();
        let packet = [
            0.25f32.to_le_bytes(),
            (-0.25f32).to_le_bytes(),
            0.5f32.to_le_bytes(),
            (-0.5f32).to_le_bytes(),
        ]
        .concat();

        recorder.record_packet(&packet, 2, 0, 0).unwrap();
        recorder.record_packet(&packet, 2, 0, 2).unwrap();

        let artifact = recorder.finish();
        assert_eq!(artifact.metadata().stored_frames, 2);
        assert_eq!(artifact.metadata().dropped_frames, 2);
        assert_eq!(artifact.metadata().overflow_events, 1);
        assert_eq!(artifact.metadata().gap_events, 0);
        assert_eq!(artifact.pcm_bytes(), &packet);
    }

    #[test]
    fn recorder_counts_silent_packets_and_position_gaps_without_payload() {
        let mut recorder = PcmRecorder::new(PcmArtifactConfig::new(8), 48_000, 2).unwrap();

        recorder
            .record_packet(&[], 2, crate::stats::SILENT, 0)
            .unwrap();
        recorder
            .record_packet(&[], 2, crate::stats::SILENT, 4)
            .unwrap();

        let artifact = recorder.finish();
        assert_eq!(artifact.metadata().silent_frames, 4);
        assert_eq!(artifact.metadata().gap_events, 1);
        assert_eq!(artifact.metadata().gap_frames, 2);
        assert_eq!(artifact.metadata().stored_frames, 4);
        assert!(artifact.pcm_bytes().iter().all(|&byte| byte == 0));
    }
}
