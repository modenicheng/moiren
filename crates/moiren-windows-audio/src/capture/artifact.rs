use crate::stats::{SILENT, TIMESTAMP_ERROR};
use serde::Serialize;
use thiserror::Error;

const BYTES_PER_F32: usize = std::mem::size_of::<f32>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmArtifactConfig {
    pub capacity_frames: usize,
}

impl PcmArtifactConfig {
    pub const fn new(capacity_frames: usize) -> Self {
        Self { capacity_frames }
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
    pub gap_events: u64,
    pub gap_frames: u64,
    pub discontinuity_events: u64,
}

/// A completed, bounded PCM capture. The byte payload is deliberately kept
/// outside `CaptureReport` serialization; callers decide when to persist it.
#[derive(Debug)]
pub struct PcmArtifact {
    metadata: PcmArtifactMetadata,
    pcm: Vec<u8>,
}

impl PcmArtifact {
    pub fn metadata(&self) -> PcmArtifactMetadata {
        self.metadata
    }

    /// Interleaved little-endian f32 samples, limited to `stored_frames`.
    pub fn pcm_bytes(&self) -> &[u8] {
        &self.pcm
    }

    pub fn into_parts(self) -> (PcmArtifactMetadata, Vec<u8>) {
        (self.metadata, self.pcm)
    }
}

#[derive(Debug)]
pub struct PcmRecorder {
    metadata: PcmArtifactMetadata,
    frame_bytes: usize,
    pcm: Vec<u8>,
    next_position: Option<u64>,
}

impl PcmRecorder {
    pub fn new(
        config: PcmArtifactConfig,
        sample_rate: u32,
        channels: usize,
    ) -> Result<Self, PcmArtifactError> {
        if config.capacity_frames == 0 {
            return Err(PcmArtifactError::ZeroCapacity);
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
                schema_version: 1,
                sample_rate,
                channels,
                capacity_frames: config.capacity_frames,
                ..PcmArtifactMetadata::default()
            },
            frame_bytes,
            pcm: vec![0; bytes],
            next_position: None,
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
            self.next_position = None;
        } else {
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
        Ok(())
    }

    pub fn finish(mut self) -> PcmArtifact {
        self.pcm
            .truncate(self.metadata.stored_frames as usize * self.frame_bytes);
        PcmArtifact {
            metadata: self.metadata,
            pcm: self.pcm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_preallocates_bounds_storage_and_reports_overflow() {
        let mut recorder =
            PcmRecorder::new(PcmArtifactConfig { capacity_frames: 2 }, 48_000, 2).unwrap();
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
        let mut recorder =
            PcmRecorder::new(PcmArtifactConfig { capacity_frames: 8 }, 48_000, 2).unwrap();

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
