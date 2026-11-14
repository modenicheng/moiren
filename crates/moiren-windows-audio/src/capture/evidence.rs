//! Prepared bounded evidence storage. The control joiner owns reclamation.

use crate::stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use thiserror::Error;

const MAX_STORAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct PcmEvidenceConfig {
    pub capacity_frames: usize,
    /// Only enable for device positions expressed in the same frame units as
    /// the PCM packets. Process Loopback commonly has no such position clock.
    pub detect_position_gaps: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PcmEvidencePacket {
    pub frames: u32,
    pub flags: u32,
    pub device_position_frames: u64,
    pub qpc_100ns: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PcmEvidenceFrame {
    /// Original sample bits; mono uses slot zero and leaves slot one at zero.
    pub samples: [f32; 2],
    pub packet_sequence: u64,
    pub packet_frame_offset: u32,
    pub packet_frames: u32,
    pub flags: u32,
    pub device_position_frames: u64,
    /// Raw WASAPI packet QPC value in 100 ns units, not physical E2E timing.
    pub qpc_100ns: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct PcmEvidenceSummary {
    pub sample_rate: u32,
    pub channels: usize,
    pub capacity_frames: usize,
    pub detect_position_gaps: bool,
    pub packets: u64,
    pub observed_frames: u64,
    pub stored_frames: u64,
    pub dropped_frames: u64,
    pub silent_frames: u64,
    pub overflow_packets: u64,
    pub reader_abandoned_packets: u64,
    pub gap_events: u64,
    pub gap_frames: u64,
    pub discontinuity_events: u64,
    pub timestamp_error_packets: u64,
    pub position_overflow_packets: u64,
    pub counters_saturated: bool,
    pub sequence_exhausted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PcmEvidenceError {
    #[error("evidence tap requires nonzero capacity/rate and mono or stereo")]
    InvalidConfig,
    #[error("evidence tap storage exceeds the 8 MiB preparation budget")]
    StorageBudget,
    #[error("evidence packet payload does not match its complete frame count")]
    InvalidPacket,
    #[error("evidence packet sequence is exhausted")]
    SequenceExhausted,
    #[error("evidence tap is closed")]
    Closed,
}

#[derive(Debug)]
pub struct PcmEvidenceWriter {
    tx: Producer<PcmEvidenceFrame>,
    closed: Arc<AtomicBool>,
    config: PcmEvidenceConfig,
    summary: PcmEvidenceSummary,
    next_position: Option<u64>,
}

#[derive(Debug)]
pub struct PcmEvidenceReader {
    rx: Consumer<PcmEvidenceFrame>,
    closed: Arc<AtomicBool>,
}

pub fn pcm_evidence_stream(
    config: PcmEvidenceConfig,
    sample_rate: u32,
    channels: usize,
) -> Result<(PcmEvidenceWriter, PcmEvidenceReader), PcmEvidenceError> {
    if config.capacity_frames == 0 || sample_rate == 0 || !(1..=2).contains(&channels) {
        return Err(PcmEvidenceError::InvalidConfig);
    }
    let bytes = config
        .capacity_frames
        .checked_mul(size_of::<PcmEvidenceFrame>())
        .ok_or(PcmEvidenceError::StorageBudget)?;
    if bytes > MAX_STORAGE_BYTES {
        return Err(PcmEvidenceError::StorageBudget);
    }
    let (tx, rx) = RingBuffer::new(config.capacity_frames);
    let closed = Arc::new(AtomicBool::new(false));
    Ok((
        PcmEvidenceWriter {
            tx,
            closed: closed.clone(),
            config,
            summary: PcmEvidenceSummary {
                sample_rate,
                channels,
                capacity_frames: config.capacity_frames,
                detect_position_gaps: config.detect_position_gaps,
                ..Default::default()
            },
            next_position: None,
        },
        PcmEvidenceReader { rx, closed },
    ))
}

impl PcmEvidenceWriter {
    /// Copy without allocation or waiting. Insufficient capacity drops the
    /// whole packet; a concurrent reader may observe an admitted packet prefix.
    /// Dropping this owner can free storage: retain it through the native join.
    pub fn record_packet(
        &mut self,
        bytes: &[u8],
        packet: PcmEvidencePacket,
    ) -> Result<usize, PcmEvidenceError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(PcmEvidenceError::Closed);
        }
        if packet.frames == 0 {
            return Ok(0);
        }
        let width = self.summary.channels * 4;
        let frames = packet.frames as usize;
        let required = frames
            .checked_mul(width)
            .ok_or(PcmEvidenceError::InvalidPacket)?;
        let silent = packet.flags & SILENT != 0;
        if !silent && bytes.len() != required {
            return Err(PcmEvidenceError::InvalidPacket);
        }
        let Some(sequence) = self.summary.packets.checked_add(1) else {
            self.summary.sequence_exhausted = true;
            return Err(PcmEvidenceError::SequenceExhausted);
        };
        self.summary.packets = sequence;
        add(
            &mut self.summary.observed_frames,
            u64::from(packet.frames),
            &mut self.summary.counters_saturated,
        );
        if silent {
            add(
                &mut self.summary.silent_frames,
                u64::from(packet.frames),
                &mut self.summary.counters_saturated,
            );
        }
        if packet.flags & DISCONTINUITY != 0 {
            add(
                &mut self.summary.discontinuity_events,
                1,
                &mut self.summary.counters_saturated,
            );
        }
        if packet.flags & TIMESTAMP_ERROR != 0 || packet.qpc_100ns == 0 {
            add(
                &mut self.summary.timestamp_error_packets,
                1,
                &mut self.summary.counters_saturated,
            );
            self.next_position = None;
        } else if self.config.detect_position_gaps {
            if let Some(expected) = self.next_position
                && expected != packet.device_position_frames
            {
                add(
                    &mut self.summary.gap_events,
                    1,
                    &mut self.summary.counters_saturated,
                );
                add(
                    &mut self.summary.gap_frames,
                    expected.abs_diff(packet.device_position_frames),
                    &mut self.summary.counters_saturated,
                );
            }
            self.next_position = packet
                .device_position_frames
                .checked_add(u64::from(packet.frames));
            if self.next_position.is_none() {
                add(
                    &mut self.summary.position_overflow_packets,
                    1,
                    &mut self.summary.counters_saturated,
                );
            }
        }
        let abandoned = self.tx.is_abandoned();
        if abandoned || self.tx.slots() < frames {
            add(
                &mut self.summary.dropped_frames,
                u64::from(packet.frames),
                &mut self.summary.counters_saturated,
            );
            if abandoned {
                add(
                    &mut self.summary.reader_abandoned_packets,
                    1,
                    &mut self.summary.counters_saturated,
                );
            } else {
                add(
                    &mut self.summary.overflow_packets,
                    1,
                    &mut self.summary.counters_saturated,
                );
            }
            return Ok(0);
        }
        // Only this producer consumes free slots; the reader can only add more.
        // The capacity check therefore reserves room for the complete packet.
        for offset in 0..frames {
            let mut samples = [0.0; 2];
            if !silent {
                for (channel, sample) in samples.iter_mut().enumerate().take(self.summary.channels)
                {
                    let start = offset * width + channel * 4;
                    *sample = f32::from_le_bytes(
                        bytes[start..start + 4]
                            .try_into()
                            .expect("validated packet width"),
                    );
                }
            }
            self.tx
                .push(PcmEvidenceFrame {
                    samples,
                    packet_sequence: sequence,
                    packet_frame_offset: offset as u32,
                    packet_frames: packet.frames,
                    flags: packet.flags,
                    device_position_frames: packet.device_position_frames,
                    qpc_100ns: packet.qpc_100ns,
                })
                .expect("single producer reserved the complete packet");
        }
        add(
            &mut self.summary.stored_frames,
            u64::from(packet.frames),
            &mut self.summary.counters_saturated,
        );
        Ok(frames)
    }

    pub fn snapshot(&self) -> PcmEvidenceSummary {
        self.summary
    }

    pub fn close(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}

fn add(counter: &mut u64, amount: u64, saturated: &mut bool) {
    match counter.checked_add(amount) {
        Some(value) => *counter = value,
        None => {
            *counter = u64::MAX;
            *saturated = true;
        }
    }
}

impl Drop for PcmEvidenceWriter {
    fn drop(&mut self) {
        self.close();
    }
}

impl PcmEvidenceReader {
    pub fn pop(&mut self) -> Option<PcmEvidenceFrame> {
        self.rx.pop().ok()
    }

    pub fn is_finished(&self) -> bool {
        self.closed.load(Ordering::Acquire) && self.rx.slots() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_packet_sequence_never_wraps_or_publishes_an_aba_frame() {
        let (mut writer, mut reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 1,
                detect_position_gaps: false,
            },
            48_000,
            1,
        )
        .unwrap();
        writer.summary.packets = u64::MAX;
        let packet = PcmEvidencePacket {
            frames: 1,
            flags: SILENT,
            device_position_frames: 0,
            qpc_100ns: 1,
        };
        assert_eq!(
            writer.record_packet(&[], packet),
            Err(PcmEvidenceError::SequenceExhausted)
        );
        assert!(writer.snapshot().sequence_exhausted);
        assert!(reader.pop().is_none());
    }

    #[test]
    fn evidence_counter_saturation_is_visible_and_position_overflow_is_not_a_wrap() {
        let (mut writer, _reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 1,
                detect_position_gaps: true,
            },
            48_000,
            1,
        )
        .unwrap();
        writer.summary.observed_frames = u64::MAX;
        writer
            .record_packet(
                &[],
                PcmEvidencePacket {
                    frames: 1,
                    flags: SILENT,
                    device_position_frames: u64::MAX,
                    qpc_100ns: 1,
                },
            )
            .unwrap();
        assert!(writer.snapshot().counters_saturated);
        assert_eq!(writer.snapshot().position_overflow_packets, 1);
        assert_eq!(writer.next_position, None);
    }
}
