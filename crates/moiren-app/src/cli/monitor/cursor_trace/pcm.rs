//! Loss-intolerant diagnostic persistence; native audio never waits for this IO.
use super::write_new;
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::clock_bridge::native_pcm::{
    NativePcmFrame, NativePcmReader, NativePcmSummary,
};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DISK_LIMIT: u64 = 512 * 1024 * 1024;
const DRAIN_LIMIT: usize = 4096;
const FLUSH_FRAMES: usize = 2048;
const FLUSH_INTERVAL: Duration = Duration::from_millis(50);

trait ArchiveSink: Write {
    fn sync_all(&self) -> std::io::Result<()>;
}

impl ArchiveSink for File {
    fn sync_all(&self) -> std::io::Result<()> {
        File::sync_all(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct PacketMetadata {
    packet_sequence: u64,
    offered_before_frames: u64,
    accepted_before_frames: u64,
    packet_frames: u32,
    packet_accepted_frames: u32,
    flags: u32,
    generation: u64,
    device_position_frames: u64,
    qpc_100ns: u64,
    stored_before_frames: u64,
}

impl PacketMetadata {
    fn new(frame: NativePcmFrame, stored_before_frames: u64) -> Self {
        Self {
            packet_sequence: frame.packet_sequence,
            offered_before_frames: frame.offered_before_frames,
            accepted_before_frames: frame.accepted_before_frames,
            packet_frames: frame.packet_frames,
            packet_accepted_frames: frame.packet_accepted_frames,
            flags: frame.flags,
            generation: frame.generation,
            device_position_frames: frame.device_position_frames,
            qpc_100ns: frame.qpc_100ns,
            stored_before_frames,
        }
    }
}

#[derive(Clone, Default)]
struct Mapping {
    packet: Option<PacketMetadata>,
    offset: u32,
    complete_packets: u64,
    offered_frames: u64,
    accepted_frames: u64,
    stored_frames: u64,
    generation: u64,
}

impl Mapping {
    fn accept(&mut self, frame: NativePcmFrame, channels: usize) -> Result<Option<PacketMetadata>> {
        ensure!((1..=2).contains(&channels), "native PCM channels invalid");
        ensure!(
            channels != 1 || frame.sample_bits[1] == 0,
            "native PCM mono padding is not zero"
        );
        ensure!(
            frame.flags & moiren_windows_audio::stats::SILENT == 0 || frame.sample_bits == [0; 2],
            "native PCM SILENT payload is not zero"
        );
        let mut metadata = None;
        if self.packet.is_none() {
            ensure!(
                frame.packet_sequence
                    == self
                        .complete_packets
                        .checked_add(1)
                        .context("PCM sequence exhausted")?
                    && frame.packet_frame_offset == 0
                    && frame.packet_frames > 0
                    && frame.packet_accepted_frames <= frame.packet_frames
                    && frame.offered_before_frames == self.offered_frames
                    && frame.accepted_before_frames == self.accepted_frames
                    && frame.generation > 0
                    && frame.generation >= self.generation,
                "native PCM packet sequence/frontier/generation invalid"
            );
            let packet = PacketMetadata::new(frame, self.stored_frames);
            self.packet = Some(packet);
            self.offset = 0;
            metadata = Some(packet);
        }
        let packet = self.packet.expect("packet established");
        ensure!(
            frame.packet_frame_offset == self.offset
                && PacketMetadata::new(frame, packet.stored_before_frames) == packet,
            "native PCM packet offset/metadata changed"
        );
        self.stored_frames = self
            .stored_frames
            .checked_add(1)
            .context("PCM stored frontier exhausted")?;
        self.offset += 1;
        if self.offset == packet.packet_frames {
            self.offered_frames = self
                .offered_frames
                .checked_add(u64::from(packet.packet_frames))
                .context("PCM offered frontier exhausted")?;
            self.accepted_frames = self
                .accepted_frames
                .checked_add(u64::from(packet.packet_accepted_frames))
                .context("PCM accepted frontier exhausted")?;
            self.complete_packets = packet.packet_sequence;
            self.generation = packet.generation;
            self.packet = None;
            self.offset = 0;
        }
        Ok(metadata)
    }

    fn partial_packet(&self) -> Option<serde_json::Value> {
        self.packet.map(|packet| {
            serde_json::json!({
                "packet_sequence": packet.packet_sequence,
                "packet_frames": packet.packet_frames,
                "stored_packet_frames": self.offset,
            })
        })
    }
}

struct PcmPersistence<S: ArchiveSink> {
    samples: BufWriter<S>,
    packets: BufWriter<S>,
    channels: usize,
    limit: u64,
    mapping: Mapping,
    persisted_frames: u64,
    persisted_packets: u64,
    persisted_bytes: u64,
    error: Option<String>,
}

impl<S: ArchiveSink> PcmPersistence<S> {
    fn from_sinks(samples: S, packets: S, channels: usize, limit: u64) -> Self {
        Self {
            samples: BufWriter::new(samples),
            packets: BufWriter::new(packets),
            channels,
            limit,
            mapping: Mapping::default(),
            persisted_frames: 0,
            persisted_packets: 0,
            persisted_bytes: 0,
            error: None,
        }
    }

    fn persist(&mut self, frames: &[NativePcmFrame]) -> Result<()> {
        ensure!(
            self.error.is_none(),
            "native PCM persistence already refused"
        );
        let result = self.persist_batch(frames);
        if let Err(error) = &result {
            self.error = Some(format!("{error:#}"));
        }
        result
    }

    fn persist_batch(&mut self, frames: &[NativePcmFrame]) -> Result<()> {
        ensure!(
            frames.len() <= DRAIN_LIMIT,
            "native PCM batch exceeds bounded drain"
        );
        if frames.is_empty() {
            return Ok(());
        }
        // Validate the whole bounded batch before either file changes. Keep
        // state/counts uncommitted until both files flush and sync successfully.
        let mut mapping = self.mapping.clone();
        let mut metadata = Vec::new();
        let mut packets = 0;
        for &frame in frames {
            if let Some(packet) = mapping.accept(frame, self.channels)? {
                serde_json::to_writer(&mut metadata, &packet)?;
                metadata.push(b'\n');
                packets += 1;
            }
        }
        let added_bytes = (frames.len() as u64) * 8 + metadata.len() as u64;
        let bytes = self
            .persisted_bytes
            .checked_add(added_bytes)
            .context("PCM disk counter exhausted")?;
        ensure!(
            bytes <= self.limit,
            "native PCM 512 MiB per-owner disk cap reached"
        );
        for frame in frames {
            for sample in frame.sample_bits {
                self.samples.write_all(&sample.to_le_bytes())?;
            }
        }
        self.packets.write_all(&metadata)?;
        self.samples.flush()?;
        self.packets.flush()?;
        self.samples.get_ref().sync_all()?;
        self.packets.get_ref().sync_all()?;
        self.mapping = mapping;
        self.persisted_bytes = bytes;
        self.persisted_frames += frames.len() as u64;
        self.persisted_packets += packets;
        Ok(())
    }

    fn validate_final(&self, summary: NativePcmSummary, drained: bool) -> Result<()> {
        ensure!(
            self.error.is_none(),
            "native PCM persistence IO/mapping/cap refusal"
        );
        ensure!(
            summary.writer_finished && drained,
            "native PCM writer not closed and drained"
        );
        ensure!(
            summary.lost_frames == 0
                && summary.overflow_packets == 0
                && summary.reader_abandoned_packets == 0
                && !summary.saturated,
            "native PCM producer evidence loss/saturation"
        );
        ensure!(
            self.mapping.packet.is_none(),
            "native PCM partial packet at closure"
        );
        ensure!(
            summary.sample_rate > 0
                && summary.channels == self.channels
                && summary.packets > 0
                && summary.packets == self.mapping.complete_packets
                && summary.packets == self.persisted_packets
                && summary.offered_frames == self.mapping.offered_frames
                && summary.accepted_frames == self.mapping.accepted_frames
                && summary.stored_frames == self.persisted_frames
                && summary.offered_frames == self.persisted_frames,
            "native PCM final summary/frame/packet correspondence mismatch"
        );
        Ok(())
    }
}

pub(super) struct PcmArchive {
    directory: PathBuf,
    reader: NativePcmReader,
    persistence: PcmPersistence<File>,
    pending: Vec<NativePcmFrame>,
    last_flush: Instant,
    finalized: bool,
    complete: bool,
}

impl PcmArchive {
    pub(super) fn new(owner_directory: &Path, reader: NativePcmReader) -> Result<Self> {
        let directory = owner_directory.join("native-pcm");
        fs::create_dir(&directory)?;
        let summary = reader.snapshot();
        write_new(
            &directory.join("stream.json"),
            &serde_json::json!({
                "schema_version": 1, "sample_format": "f32le-bits", "frame_bytes": 8,
                "channels": summary.channels, "sample_rate": summary.sample_rate,
                "disk_limit_bytes": DISK_LIMIT, "summary": summary, "qualified": false,
                "scope": "native offered PCM; accepted prefix indexed by packet metadata; not physical latency",
            }),
        )?;
        let create = |name| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(name))
        };
        let persistence = PcmPersistence::from_sinks(
            create("samples.f32le")?,
            create("packets.jsonl")?,
            summary.channels,
            DISK_LIMIT,
        );
        Ok(Self {
            directory,
            reader,
            persistence,
            pending: Vec::with_capacity(DRAIN_LIMIT),
            last_flush: Instant::now(),
            finalized: false,
            complete: false,
        })
    }

    pub(super) fn writer_finished(&self) -> bool {
        self.reader.snapshot().writer_finished
    }

    pub(super) fn poll(&mut self) {
        if self.finalized || self.persistence.error.is_some() {
            return;
        }
        for _ in self.pending.len()..DRAIN_LIMIT {
            let Some(frame) = self.reader.pop() else {
                break;
            };
            self.pending.push(frame);
        }
        if self.pending.len() >= FLUSH_FRAMES || self.last_flush.elapsed() >= FLUSH_INTERVAL {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.persistence.error.is_none() && self.persistence.persist(&self.pending).is_ok() {
            self.pending.clear();
        }
        self.last_flush = Instant::now();
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        if self.finalized {
            return Ok(());
        }
        // A final drain is bounded by prepared storage, never by an unjoined
        // live producer. Open writers refuse rather than extending the run.
        let summary = self.reader.snapshot();
        if summary.writer_finished {
            for _ in 0..=summary.capacity_frames.div_ceil(DRAIN_LIMIT) {
                self.poll();
                self.flush();
                if self.reader.is_finished() || self.persistence.error.is_some() {
                    break;
                }
            }
        } else {
            self.flush();
        }
        let summary = self.reader.snapshot();
        let validation = self
            .persistence
            .validate_final(summary, self.reader.is_finished());
        let error = self
            .persistence
            .error
            .clone()
            .or_else(|| validation.as_ref().err().map(|error| format!("{error:#}")));
        write_new(
            &self.directory.join("final.json"),
            &serde_json::json!({
                "schema_version": 1, "summary": summary,
                "persisted_frames": self.persistence.persisted_frames,
                "persisted_packets": self.persistence.persisted_packets,
                "persisted_bytes": self.persistence.persisted_bytes,
                "partial_packet": self.persistence.mapping.partial_packet(),
                "pending_frames": self.pending.len(), "error": error,
                "persistence_complete": validation.is_ok(), "qualified": false,
            }),
        )?;
        self.complete = validation.is_ok();
        self.finalized = true;
        Ok(())
    }

    pub(super) fn persistence_complete(&self) -> bool {
        self.finalized && self.complete
    }
}

#[cfg(test)]
#[path = "pcm_tests.rs"]
mod tests;
