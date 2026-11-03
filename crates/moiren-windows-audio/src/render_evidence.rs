//! Opt-in, bounded copies of encoded render packets and their release outcomes.
//!
//! This portable foundation does not acquire native buffers or install a render
//! hook. Call `stage_packet` while the native buffer lease is valid, release that
//! buffer, then call `finish_packet` with the exact release HRESULT. No native
//! pointer is retained, and finishing reads only the prepared staging storage.
//!
//! Create the stream off the audio thread. Retain BOTH endpoints through native
//! shutdown/join and reclaim them on the control side. In particular, the writer
//! owns its staging allocation: keeping the reader alive does NOT make dropping
//! the writer allocation-free. A future native hook must explicitly take the
//! writer out of its stream before that stream is destroyed on an audio thread.
//! `close` only publishes closure; it neither frees storage nor joins anything.
//! Release acceptance is not proof of playback, audio quality, or physical latency.

use moiren_engine::audio_config::SampleFormat;
use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;
use std::{
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use thiserror::Error;

const MAX_STORAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKET_FRAMES: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderEvidenceConfig {
    pub capacity_frames: usize,
    pub max_packet_frames: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderEvidenceFormat {
    pub sample_rate: u32,
    pub sample_format: SampleFormat,
    pub channels: usize,
    pub container_bits: u16,
    pub valid_bits: u16,
}

impl RenderEvidenceFormat {
    fn frame_bytes(self) -> Result<usize, RenderEvidenceError> {
        // Match SampleCodec::with_layout without depending on its native/test
        // cfg: this evidence container must also compile in portable libraries.
        let valid = match self.sample_format {
            SampleFormat::Pcm16 => self.container_bits == 16 && self.valid_bits == 16,
            SampleFormat::Pcm24 => [24, 32].contains(&self.container_bits) && self.valid_bits == 24,
            SampleFormat::Pcm32 => self.container_bits == 32 && (1..=32).contains(&self.valid_bits),
            SampleFormat::Float32 => self.container_bits == 32 && self.valid_bits == 32,
            SampleFormat::Float64 => self.container_bits == 64 && self.valid_bits == 64,
            SampleFormat::Native => false,
        };
        if !valid || !(1..=2).contains(&self.channels) {
            return Err(RenderEvidenceError::InvalidLayout);
        }
        Ok(self.channels * usize::from(self.container_bits / 8))
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RenderEvidenceFrame {
    /// Exact encoded frame, including container padding. Unused bytes are zero.
    pub bytes: [u8; 16],
    /// Starts at one; evidence loss leaves visible gaps, never reused identities.
    pub packet_sequence: u64,
    pub packet_frame_offset: u32,
    pub packet_frames: u32,
    /// Exact ReleaseBuffer result; nonnegative means accepted, not played.
    pub release_hresult: i32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RenderEvidenceSummary {
    pub format: RenderEvidenceFormat,
    pub config: RenderEvidenceConfig,
    /// Completed staged release attempts, including evidence-only drops.
    pub attempts: u64,
    pub accepted_frames: u64,
    pub rejected_frames: u64,
    /// Cumulative admitted evidence frames, not the current ring occupancy.
    pub stored_frames: u64,
    /// Released frames whose evidence was dropped; pending closure is separate.
    pub dropped_frames: u64,
    pub overflow_packets: u64,
    pub reader_abandoned_packets: u64,
    pub invalid_attempts: u64,
    /// Staged packets closed without a known release result. Never accepted.
    pub pending_abandoned_packets: u64,
    pub pending_abandoned_frames: u64,
    pub counters_saturated: bool,
    pub sequence_exhausted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RenderEvidenceError {
    #[error("render evidence requires nonzero rate/caps and a packet cap <= capacity and 65536")]
    InvalidConfig,
    #[error("unsupported mono/stereo encoded sample layout")]
    InvalidLayout,
    #[error("render evidence ring and staging exceed the 8 MiB preparation budget")]
    StorageBudget,
    #[error("render evidence requires a complete nonempty packet within the prepared cap")]
    InvalidPacket,
    #[error("a render evidence packet is already staged")]
    PacketPending,
    #[error("no render evidence packet is staged")]
    NoPendingPacket,
    #[error("render evidence is closed")]
    Closed,
    #[error("render evidence packet sequence is exhausted")]
    SequenceExhausted,
}

/// Single producer. Dropping this value can free staging even if the reader lives.
/// Return/retain it for control-side reclamation after the native thread joins.
#[derive(Debug)]
pub struct RenderEvidenceWriter {
    tx: Producer<RenderEvidenceFrame>,
    closed: Arc<AtomicBool>,
    staging: Box<[u8]>,
    staged_frames: Option<usize>,
    frame_bytes: usize,
    summary: RenderEvidenceSummary,
}

/// Single consumer. Keep it alive through native join; reclaim off the audio thread.
#[derive(Debug)]
pub struct RenderEvidenceReader {
    rx: Consumer<RenderEvidenceFrame>,
    closed: Arc<AtomicBool>,
}

/// Prepare all variable-size storage off the audio thread. The 8 MiB payload
/// budget covers `capacity * size_of::<RenderEvidenceFrame>()` plus staging;
/// fixed endpoint/atomic metadata and allocator bookkeeping are not payload.
pub fn render_evidence_stream(
    config: RenderEvidenceConfig,
    format: RenderEvidenceFormat,
) -> Result<(RenderEvidenceWriter, RenderEvidenceReader), RenderEvidenceError> {
    if format.sample_rate == 0
        || config.capacity_frames == 0
        || config.max_packet_frames == 0
        || config.max_packet_frames > config.capacity_frames
        || config.max_packet_frames > MAX_PACKET_FRAMES
    {
        return Err(RenderEvidenceError::InvalidConfig);
    }
    let frame_bytes = format.frame_bytes()?;
    let staging_bytes = config
        .max_packet_frames
        .checked_mul(frame_bytes)
        .ok_or(RenderEvidenceError::StorageBudget)?;
    let total_bytes = config
        .capacity_frames
        .checked_mul(size_of::<RenderEvidenceFrame>())
        .and_then(|ring_bytes| ring_bytes.checked_add(staging_bytes))
        .ok_or(RenderEvidenceError::StorageBudget)?;
    if total_bytes > MAX_STORAGE_BYTES {
        return Err(RenderEvidenceError::StorageBudget);
    }
    let staging = vec![0; staging_bytes].into_boxed_slice();
    let (tx, rx) = RingBuffer::new(config.capacity_frames);
    let closed = Arc::new(AtomicBool::new(false));
    Ok((
        RenderEvidenceWriter {
            tx,
            closed: Arc::clone(&closed),
            staging,
            staged_frames: None,
            frame_bytes,
            summary: RenderEvidenceSummary {
                format,
                config,
                attempts: 0,
                accepted_frames: 0,
                rejected_frames: 0,
                stored_frames: 0,
                dropped_frames: 0,
                overflow_packets: 0,
                reader_abandoned_packets: 0,
                invalid_attempts: 0,
                pending_abandoned_packets: 0,
                pending_abandoned_frames: 0,
                counters_saturated: false,
                sequence_exhausted: false,
            },
        },
        RenderEvidenceReader { rx, closed },
    ))
}

impl RenderEvidenceWriter {
    /// Copy exact encoded bytes while the caller's native lease is valid.
    /// No allocation, waiting, native ownership, or publication occurs here.
    pub fn stage_packet(&mut self, bytes: &[u8], frames: usize) -> Result<(), RenderEvidenceError> {
        if self.closed.load(Ordering::Relaxed) {
            return self.invalid(RenderEvidenceError::Closed);
        }
        if self.staged_frames.is_some() {
            return self.invalid(RenderEvidenceError::PacketPending);
        }
        if frames == 0
            || frames > self.summary.config.max_packet_frames
            || frames.checked_mul(self.frame_bytes) != Some(bytes.len())
        {
            return self.invalid(RenderEvidenceError::InvalidPacket);
        }
        if self.summary.attempts == u64::MAX {
            self.summary.sequence_exhausted = true;
            self.summary.counters_saturated = true;
            return self.invalid(RenderEvidenceError::SequenceExhausted);
        }
        self.staging[..bytes.len()].copy_from_slice(bytes);
        self.staged_frames = Some(frames);
        Ok(())
    }

    /// Record the exact release result using only the owned staging copy.
    /// Returns evidence frames stored (zero for whole-packet evidence loss),
    /// NOT audio frames accepted. Negative results are retained as attempted
    /// packets too. Evidence failure must never gate or retry native audio.
    pub fn finish_packet(&mut self, hr: i32) -> Result<usize, RenderEvidenceError> {
        if self.closed.load(Ordering::Relaxed) {
            return self.invalid(RenderEvidenceError::Closed);
        }
        let Some(frames) = self.staged_frames.take() else {
            return self.invalid(RenderEvidenceError::NoPendingPacket);
        };
        // stage_packet reserved a unique sequence without exposing bytes. Only
        // this writer mutates attempts, so it cannot change while staged.
        let sequence = self.summary.attempts + 1;
        self.summary.attempts = sequence;
        let count = frames as u64;
        let outcome = if hr >= 0 {
            &mut self.summary.accepted_frames
        } else {
            &mut self.summary.rejected_frames
        };
        add(outcome, count, &mut self.summary.counters_saturated);

        let abandoned = self.tx.is_abandoned();
        if abandoned || self.tx.slots() < frames {
            add(
                &mut self.summary.dropped_frames,
                count,
                &mut self.summary.counters_saturated,
            );
            let packets = if abandoned {
                &mut self.summary.reader_abandoned_packets
            } else {
                &mut self.summary.overflow_packets
            };
            add(packets, 1, &mut self.summary.counters_saturated);
            return Ok(0);
        }
        // Only this producer consumes free slots; the reader only adds room.
        // Admission therefore reserves an entire packet without locks. A
        // concurrent reader may consume its prefix as we publish the frames.
        for offset in 0..frames {
            let start = offset * self.frame_bytes;
            let mut bytes = [0; 16];
            bytes[..self.frame_bytes]
                .copy_from_slice(&self.staging[start..start + self.frame_bytes]);
            self.tx
                .push(RenderEvidenceFrame {
                    bytes,
                    packet_sequence: sequence,
                    packet_frame_offset: offset as u32,
                    packet_frames: frames as u32,
                    release_hresult: hr,
                })
                .expect("single producer reserved complete render evidence packet");
        }
        add(
            &mut self.summary.stored_frames,
            count,
            &mut self.summary.counters_saturated,
        );
        Ok(frames)
    }

    pub fn snapshot(&self) -> RenderEvidenceSummary {
        self.summary
    }

    /// Idempotent and allocation/deallocation-free. Unknown pending release
    /// outcomes are counted separately, never fabricated as accepted/rejected.
    pub fn close(&mut self) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        if let Some(frames) = self.staged_frames.take() {
            add(
                &mut self.summary.pending_abandoned_packets,
                1,
                &mut self.summary.counters_saturated,
            );
            add(
                &mut self.summary.pending_abandoned_frames,
                frames as u64,
                &mut self.summary.counters_saturated,
            );
            add(
                &mut self.summary.invalid_attempts,
                1,
                &mut self.summary.counters_saturated,
            );
        }
        self.closed.store(true, Ordering::Release);
    }

    fn invalid<T>(&mut self, error: RenderEvidenceError) -> Result<T, RenderEvidenceError> {
        add(
            &mut self.summary.invalid_attempts,
            1,
            &mut self.summary.counters_saturated,
        );
        Err(error)
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

impl Drop for RenderEvidenceWriter {
    fn drop(&mut self) {
        self.close();
    }
}

impl RenderEvidenceReader {
    pub fn pop(&mut self) -> Option<RenderEvidenceFrame> {
        self.rx.pop().ok()
    }

    pub fn is_finished(&self) -> bool {
        self.closed.load(Ordering::Acquire) && self.rx.slots() == 0
    }
}

#[cfg(test)]
mod tests;
