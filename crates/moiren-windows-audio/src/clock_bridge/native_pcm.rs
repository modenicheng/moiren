//! Optional native/offered PCM, indexed before accepted-ring tail drops.
//! Prepared SPSC storage only; no disk, locks, allocation or retries on capture.
use super::CapturePacket;
use crate::stats::SILENT;
use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

const MAX_STORAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativePcmConfig {
    pub capacity_frames: usize,
}

impl NativePcmConfig {
    pub fn validate(self) -> Result<(), &'static str> {
        self.storage_bytes().map(|_| ())
    }

    pub(super) fn storage_bytes(self) -> Result<usize, &'static str> {
        self.capacity_frames
            .checked_mul(size_of::<NativePcmFrame>())
            .filter(|&bytes| bytes > 0 && bytes <= MAX_STORAGE_BYTES)
            .ok_or("native PCM capacity must fit the nonzero 8 MiB preparation budget")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NativePcmFrame {
    /// Unmodified native f32 bits, including nonfinite values and signed zero.
    /// SILENT has no readable payload; mono leaves the second slot zero.
    pub sample_bits: [u32; 2],
    pub packet_sequence: u64,
    pub offered_before_frames: u64,
    pub accepted_before_frames: u64,
    pub packet_frames: u32,
    pub packet_accepted_frames: u32,
    pub packet_frame_offset: u32,
    pub flags: u32,
    pub generation: u64,
    pub device_position_frames: u64,
    pub qpc_100ns: u64,
}

impl NativePcmFrame {
    /// The exact sample conversion before native ring publication. The caller
    /// must first exclude offsets >= packet_accepted_frames from the SRC index.
    pub fn accepted_samples(self, channels: usize) -> Option<[f32; 2]> {
        if !(1..=2).contains(&channels) || self.packet_frame_offset >= self.packet_accepted_frames {
            return None;
        }
        let mut samples = self.sample_bits.map(|bits| {
            let value = f32::from_bits(bits);
            if value.is_finite() { value } else { 0.0 }
        });
        if self.flags & SILENT != 0 {
            samples = [0.0; 2];
        } else if channels == 1 {
            samples[1] = samples[0];
        }
        Some(samples)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct NativePcmSummary {
    pub capacity_frames: usize,
    pub sample_rate: u32,
    pub channels: usize,
    pub packets: u64,
    pub offered_frames: u64,
    pub accepted_frames: u64,
    pub stored_frames: u64,
    pub lost_frames: u64,
    pub overflow_packets: u64,
    pub reader_abandoned_packets: u64,
    pub saturated: bool,
    pub writer_finished: bool,
}

struct Published {
    packets: AtomicU64,
    offered: AtomicU64,
    accepted: AtomicU64,
    stored: AtomicU64,
    lost: AtomicU64,
    overflow: AtomicU64,
    abandoned: AtomicU64,
    saturated: AtomicBool,
    finished: AtomicBool,
}

pub(crate) struct NativePcmWriter {
    tx: Producer<NativePcmFrame>,
    published: Arc<Published>,
    summary: NativePcmSummary,
}

impl std::fmt::Debug for NativePcmWriter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativePcmWriter")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

/// Native teardown may close diagnostics on the capture thread, but storage
/// destruction belongs to the control joiner, even if the reader failed early.
#[cfg(windows)]
pub(crate) struct NativePcmIngressGuard<'a> {
    ingress: super::CaptureIngress,
    retired: &'a mut Option<NativePcmWriter>,
}

#[cfg(windows)]
impl<'a> NativePcmIngressGuard<'a> {
    pub(crate) fn new(
        ingress: super::CaptureIngress,
        retired: &'a mut Option<NativePcmWriter>,
    ) -> Self {
        // This invariant is checked before native activation, not from Drop
        // where a panic would be especially dangerous during owner unwind.
        assert!(retired.is_none(), "one native stream per capture report");
        Self { ingress, retired }
    }
}

#[cfg(windows)]
impl std::ops::Deref for NativePcmIngressGuard<'_> {
    type Target = super::CaptureIngress;
    fn deref(&self) -> &Self::Target {
        &self.ingress
    }
}

#[cfg(windows)]
impl std::ops::DerefMut for NativePcmIngressGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ingress
    }
}

#[cfg(windows)]
impl Drop for NativePcmIngressGuard<'_> {
    fn drop(&mut self) {
        if let Some(writer) = self.ingress.native_pcm.take() {
            writer.close();
            *self.retired = Some(writer);
        }
    }
}

/// Single control-side reader. Retain it through the native owner join so
/// final ring reclamation stays off streaming. Snapshot scalars may be skewed
/// while writing; only a finished writer certifies the final aggregate tally.
pub struct NativePcmReader {
    rx: Consumer<NativePcmFrame>,
    published: Arc<Published>,
    config: NativePcmSummary,
}

pub(super) fn stream(
    config: NativePcmConfig,
    sample_rate: u32,
    channels: usize,
) -> (NativePcmWriter, NativePcmReader) {
    let (tx, rx) = RingBuffer::new(config.capacity_frames);
    let published = Arc::new(Published {
        packets: AtomicU64::new(0),
        offered: AtomicU64::new(0),
        accepted: AtomicU64::new(0),
        stored: AtomicU64::new(0),
        lost: AtomicU64::new(0),
        overflow: AtomicU64::new(0),
        abandoned: AtomicU64::new(0),
        saturated: AtomicBool::new(false),
        finished: AtomicBool::new(false),
    });
    let summary = NativePcmSummary {
        capacity_frames: config.capacity_frames,
        sample_rate,
        channels,
        packets: 0,
        offered_frames: 0,
        accepted_frames: 0,
        stored_frames: 0,
        lost_frames: 0,
        overflow_packets: 0,
        reader_abandoned_packets: 0,
        saturated: false,
        writer_finished: false,
    };
    (
        NativePcmWriter {
            tx,
            published: published.clone(),
            summary,
        },
        NativePcmReader {
            rx,
            published,
            config: summary,
        },
    )
}

impl NativePcmWriter {
    fn close(&self) {
        self.published.finished.store(true, Ordering::Release);
    }
    pub(super) fn mark_saturated(&self) {
        self.published.saturated.store(true, Ordering::Relaxed);
    }

    /// Called only after ingress validates the entire packet and fixes the
    /// accepted prefix/generation. Missing tap capacity drops a whole packet,
    /// never its audio. Later packet frontiers preserve that evidence hole.
    pub(super) fn record(
        &mut self,
        bytes: &[u8],
        packet: CapturePacket,
        accepted: usize,
        generation: u64,
    ) {
        if self.published.saturated.load(Ordering::Relaxed) {
            return;
        }
        let s = &mut self.summary;
        let before_offered = s.offered_frames;
        let before_accepted = s.accepted_frames;
        let (Some(sequence), Some(offered), Some(published), Ok(frames), Ok(accepted_frames)) = (
            s.packets.checked_add(1),
            s.offered_frames.checked_add(packet.frames as u64),
            s.accepted_frames.checked_add(accepted as u64),
            u32::try_from(packet.frames),
            u32::try_from(accepted),
        ) else {
            self.mark_saturated();
            return;
        };
        if generation == 0 {
            self.mark_saturated();
            return;
        }
        s.packets = sequence;
        s.offered_frames = offered;
        s.accepted_frames = published;
        let abandoned = self.tx.is_abandoned();
        if abandoned || self.tx.slots() < packet.frames {
            // These disjoint totals are bounded by the checked offered total.
            s.lost_frames += u64::from(frames);
            if abandoned {
                s.reader_abandoned_packets += 1;
            } else {
                s.overflow_packets += 1;
            }
        } else {
            let channels = s.channels;
            let silent = packet.flags & SILENT != 0;
            self.tx
                .write_chunk_uninit(packet.frames)
                .expect("reserved native PCM frames")
                .fill_from_iter((0..packet.frames).map(|offset| {
                    let mut sample_bits = [0; 2];
                    if !silent {
                        for (channel, bits) in sample_bits.iter_mut().enumerate().take(channels) {
                            let start = (offset * channels + channel) * 4;
                            *bits = u32::from_le_bytes(
                                bytes[start..start + 4]
                                    .try_into()
                                    .expect("validated native PCM width"),
                            );
                        }
                    }
                    NativePcmFrame {
                        sample_bits,
                        packet_sequence: sequence,
                        offered_before_frames: before_offered,
                        accepted_before_frames: before_accepted,
                        packet_frames: frames,
                        packet_accepted_frames: accepted_frames,
                        packet_frame_offset: offset as u32,
                        flags: packet.flags,
                        generation,
                        device_position_frames: packet.device_position_frames,
                        qpc_100ns: packet.qpc_100ns,
                    }
                }));
            s.stored_frames += u64::from(frames);
        }
        let p = &self.published;
        for (field, value) in [
            (&p.packets, s.packets),
            (&p.offered, s.offered_frames),
            (&p.accepted, s.accepted_frames),
            (&p.stored, s.stored_frames),
            (&p.lost, s.lost_frames),
            (&p.overflow, s.overflow_packets),
            (&p.abandoned, s.reader_abandoned_packets),
        ] {
            field.store(value, Ordering::Relaxed);
        }
    }
}

impl Drop for NativePcmWriter {
    fn drop(&mut self) {
        self.close();
    }
}

impl NativePcmReader {
    pub fn pop(&mut self) -> Option<NativePcmFrame> {
        self.rx.pop().ok()
    }

    pub fn is_finished(&self) -> bool {
        self.published.finished.load(Ordering::Acquire) && self.rx.is_empty()
    }

    pub fn snapshot(&self) -> NativePcmSummary {
        let p = &self.published;
        // Acquire closure before reading final counters, not after them.
        let writer_finished = p.finished.load(Ordering::Acquire);
        NativePcmSummary {
            packets: p.packets.load(Ordering::Relaxed),
            offered_frames: p.offered.load(Ordering::Relaxed),
            accepted_frames: p.accepted.load(Ordering::Relaxed),
            stored_frames: p.stored.load(Ordering::Relaxed),
            lost_frames: p.lost.load(Ordering::Relaxed),
            overflow_packets: p.overflow.load(Ordering::Relaxed),
            reader_abandoned_packets: p.abandoned.load(Ordering::Relaxed),
            saturated: p.saturated.load(Ordering::Relaxed),
            writer_finished,
            ..self.config
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_pcm_frontier_and_sequence_exhaustion_are_sticky_refusals() {
        for field in 0..3 {
            let (mut writer, mut reader) =
                stream(NativePcmConfig { capacity_frames: 1 }, 48_000, 2);
            match field {
                0 => writer.summary.packets = u64::MAX,
                1 => writer.summary.offered_frames = u64::MAX,
                _ => writer.summary.accepted_frames = u64::MAX,
            }
            let packet = CapturePacket {
                frames: 1,
                flags: SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            };
            crate::test_allocation::assert_allocates_nothing(|| writer.record(&[], packet, 1, 1));
            assert!(reader.snapshot().saturated);
            assert!(reader.pop().is_none());
            writer.summary.packets = 0;
            writer.summary.offered_frames = 0;
            writer.summary.accepted_frames = 0;
            writer.record(&[], packet, 1, 1);
            assert!(
                reader.pop().is_none(),
                "a later valid packet cannot heal exhausted evidence"
            );
            drop(writer);
            assert!(reader.is_finished());
        }
    }
}
