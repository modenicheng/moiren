//! Encoded render evidence only; successful storage is never audio qualification.
use super::write_new;
use anyhow::{Context, Result, ensure};
use moiren_engine::audio_config::SampleFormat;
use moiren_windows_audio::{
    engine_backend::NativeRenderEvidenceReader,
    render_evidence::{
        RenderEvidenceConfig, RenderEvidenceFormat, RenderEvidenceFrame, RenderEvidenceSummary,
    },
};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DISK_LIMIT: u64 = 512 * 1024 * 1024;
const RECEIPT_LIMIT: u64 = 32 * 1024;
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

fn frame_bytes(format: RenderEvidenceFormat, config: RenderEvidenceConfig) -> Result<usize> {
    let valid = match format.sample_format {
        SampleFormat::Pcm16 => format.container_bits == 16 && format.valid_bits == 16,
        SampleFormat::Pcm24 => [24, 32].contains(&format.container_bits) && format.valid_bits == 24,
        SampleFormat::Pcm32 => format.container_bits == 32 && (1..=32).contains(&format.valid_bits),
        SampleFormat::Float32 => format.container_bits == 32 && format.valid_bits == 32,
        SampleFormat::Float64 => format.container_bits == 64 && format.valid_bits == 64,
        SampleFormat::Native => false,
    };
    ensure!(
        valid && format.sample_rate > 0 && (1..=2).contains(&format.channels),
        "native render encoded layout invalid"
    );
    ensure!(
        config.capacity_frames > 0
            && (1..=65_536).contains(&config.max_packet_frames)
            && config.max_packet_frames <= config.capacity_frames,
        "native render evidence capacity invalid"
    );
    let bytes = format.channels * usize::from(format.container_bits / 8);
    let storage = config
        .capacity_frames
        .checked_mul(size_of::<RenderEvidenceFrame>())
        .and_then(|ring| {
            config
                .max_packet_frames
                .checked_mul(bytes)
                .and_then(|staging| ring.checked_add(staging))
        })
        .context("native render evidence capacity exhausted")?;
    ensure!(
        storage <= 8 * 1024 * 1024,
        "native render preparation budget invalid"
    );
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct PacketMetadata {
    packet_sequence: u64,
    packet_frames: u32,
    release_hresult: i32,
    stored_before_frames: u64,
}

impl PacketMetadata {
    fn new(frame: &RenderEvidenceFrame, stored_before_frames: u64) -> Self {
        Self {
            packet_sequence: frame.packet_sequence,
            packet_frames: frame.packet_frames,
            release_hresult: frame.release_hresult,
            stored_before_frames,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Mapping {
    packet: Option<PacketMetadata>,
    offset: u32,
    complete_packets: u64,
    stored_frames: u64,
    accepted_frames: u64,
    rejected_frames: u64,
}

impl Mapping {
    fn accept(
        &mut self,
        frame: &RenderEvidenceFrame,
        bytes: usize,
        max_packet_frames: usize,
    ) -> Result<Option<PacketMetadata>> {
        ensure!(
            frame.bytes[bytes..].iter().all(|&byte| byte == 0),
            "native render unused payload is not zero"
        );
        let mut metadata = None;
        if self.packet.is_none() {
            ensure!(
                frame.packet_sequence
                    == self
                        .complete_packets
                        .checked_add(1)
                        .context("render sequence exhausted")?
                    && frame.packet_frame_offset == 0
                    && frame.packet_frames > 0
                    && frame.packet_frames as usize <= max_packet_frames,
                "native render packet sequence/size invalid"
            );
            let packet = PacketMetadata::new(frame, self.stored_frames);
            self.packet = Some(packet);
            metadata = Some(packet);
        }
        let packet = self.packet.context("native render packet missing")?;
        ensure!(
            frame.packet_frame_offset == self.offset
                && PacketMetadata::new(frame, packet.stored_before_frames) == packet,
            "native render packet offset/metadata changed"
        );
        self.stored_frames = self
            .stored_frames
            .checked_add(1)
            .context("render stored frontier exhausted")?;
        self.offset += 1;
        if self.offset == packet.packet_frames {
            let outcome = if packet.release_hresult >= 0 {
                &mut self.accepted_frames
            } else {
                &mut self.rejected_frames
            };
            *outcome = outcome
                .checked_add(u64::from(packet.packet_frames))
                .context("render outcome frontier exhausted")?;
            self.complete_packets = packet.packet_sequence;
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
                "release_hresult": packet.release_hresult,
                "stored_before_frames": packet.stored_before_frames,
                "stored_packet_frames": self.offset,
            })
        })
    }
}

struct RenderPersistence<S: ArchiveSink> {
    // No BufWriter: after an IO error, drop must not retry unaccounted writes.
    samples: S,
    packets: S,
    format: RenderEvidenceFormat,
    config: RenderEvidenceConfig,
    frame_bytes: usize,
    limit: u64,
    mapping: Mapping,
    persisted_frames: u64,
    persisted_packets: u64,
    persisted_bytes: u64,
    error: Option<String>,
}

impl<S: ArchiveSink> RenderPersistence<S> {
    fn from_sinks(
        samples: S,
        packets: S,
        format: RenderEvidenceFormat,
        config: RenderEvidenceConfig,
        limit: u64,
    ) -> Result<Self> {
        Ok(Self {
            frame_bytes: frame_bytes(format, config)?,
            samples,
            packets,
            format,
            config,
            limit,
            mapping: Mapping::default(),
            persisted_frames: 0,
            persisted_packets: 0,
            persisted_bytes: 0,
            error: None,
        })
    }

    fn persist(&mut self, frames: &[RenderEvidenceFrame]) -> Result<()> {
        ensure!(
            self.error.is_none(),
            "native render persistence already refused"
        );
        let result = self.persist_batch(frames);
        if let Err(error) = &result {
            self.error = Some(format!("{error:#}"));
        }
        result
    }

    fn persist_batch(&mut self, frames: &[RenderEvidenceFrame]) -> Result<()> {
        ensure!(
            frames.len() <= DRAIN_LIMIT,
            "native render batch exceeds bounded drain"
        );
        if frames.is_empty() {
            return Ok(());
        }
        // Validate a bounded transaction before IO. A partial write remains a
        // failed physical tail, not a committed mapping or permission to retry.
        let mut mapping = self.mapping;
        let mut metadata = Vec::new();
        let mut samples = Vec::with_capacity(frames.len() * self.frame_bytes);
        let mut packets = 0;
        for frame in frames {
            if let Some(packet) =
                mapping.accept(frame, self.frame_bytes, self.config.max_packet_frames)?
            {
                serde_json::to_writer(&mut metadata, &packet)?;
                metadata.push(b'\n');
                packets += 1;
            }
            samples.extend_from_slice(&frame.bytes[..self.frame_bytes]);
        }
        let bytes = self
            .persisted_bytes
            .checked_add(samples.len() as u64)
            .and_then(|bytes| bytes.checked_add(metadata.len() as u64))
            .context("render disk counter exhausted")?;
        ensure!(
            bytes <= self.limit,
            "native render 512 MiB per-owner disk cap reached"
        );
        let persisted_packets = self
            .persisted_packets
            .checked_add(packets)
            .context("render packet counter exhausted")?;
        self.samples
            .write_all(&samples)
            .context("write render samples")?;
        self.packets
            .write_all(&metadata)
            .context("write render packets")?;
        self.samples.flush().context("flush render samples")?;
        self.packets.flush().context("flush render packets")?;
        self.samples.sync_all().context("sync render samples")?;
        self.packets.sync_all().context("sync render packets")?;
        self.mapping = mapping;
        self.persisted_frames = mapping.stored_frames;
        self.persisted_packets = persisted_packets;
        self.persisted_bytes = bytes;
        Ok(())
    }

    fn validate_final(&self, summary: &RenderEvidenceSummary, drained: bool) -> Result<()> {
        ensure!(
            self.error.is_none(),
            "native render persistence IO/mapping/cap refusal"
        );
        ensure!(drained, "native render writer not closed and drained");
        ensure!(
            summary.format == self.format && summary.config == self.config,
            "native render origin/final layout mismatch"
        );
        ensure!(
            summary.dropped_frames == 0
                && summary.overflow_packets == 0
                && summary.reader_abandoned_packets == 0
                && summary.invalid_attempts == 0
                && summary.unobserved_packets == 0
                && summary.unobserved_frames == 0
                && summary.pending_abandoned_packets == 0
                && summary.pending_abandoned_frames == 0
                && !summary.counters_saturated
                && !summary.sequence_exhausted,
            "native render evidence loss/unobserved/abandoned/saturation"
        );
        ensure!(
            self.mapping.packet.is_none(),
            "native render partial packet at closure"
        );
        ensure!(
            summary.attempts > 0
                && summary.attempts == self.mapping.complete_packets
                && summary.attempts == self.persisted_packets
                && summary.accepted_frames == self.mapping.accepted_frames
                && summary.rejected_frames == self.mapping.rejected_frames
                && summary.stored_frames == self.persisted_frames
                && summary.accepted_frames.checked_add(summary.rejected_frames)
                    == Some(summary.stored_frames),
            "native render final attempts/outcomes/stored correspondence mismatch"
        );
        Ok(())
    }
}

trait EvidenceReader {
    fn pop(&mut self) -> Option<RenderEvidenceFrame>;
    fn is_finished(&self) -> bool;
    fn final_summary(&self) -> Option<RenderEvidenceSummary>;
}

impl EvidenceReader for NativeRenderEvidenceReader {
    fn pop(&mut self) -> Option<RenderEvidenceFrame> {
        self.pop()
    }
    fn is_finished(&self) -> bool {
        self.is_finished()
    }
    fn final_summary(&self) -> Option<RenderEvidenceSummary> {
        self.final_summary()
    }
}

fn prepare(
    owner: &Path,
    origin: &impl Serialize,
    format: RenderEvidenceFormat,
    config: RenderEvidenceConfig,
) -> Result<(PathBuf, RenderPersistence<File>)> {
    let bytes = frame_bytes(format, config)?;
    let stream = serde_json::json!({
        "schema_version": 1, "origin": origin, "frame_bytes": bytes,
        "disk_limit_bytes": DISK_LIMIT, "qualified": false,
        "scope": "exact encoded native render attempts and ReleaseBuffer HRESULTs; not playback, quality or physical latency",
    });
    let stream_bytes = serde_json::to_vec(&stream)?.len() as u64;
    ensure!(
        stream_bytes <= RECEIPT_LIMIT,
        "native render origin receipt too large"
    );
    let directory = owner.join("native-render");
    fs::create_dir(&directory).context("create new native render directory")?;
    write_new(&directory.join("stream.json"), &stream)?;
    let create = |name| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
    };
    let persistence = RenderPersistence::from_sinks(
        create("samples.bin")?,
        create("packets.jsonl")?,
        format,
        config,
        DISK_LIMIT - stream_bytes - RECEIPT_LIMIT,
    )?;
    Ok((directory, persistence))
}

struct Archive<R: EvidenceReader> {
    directory: PathBuf,
    reader: R,
    persistence: RenderPersistence<File>,
    pending: Vec<RenderEvidenceFrame>,
    last_flush: Instant,
    finalized: bool,
    complete: bool,
    finish_error: Option<String>,
}

impl<R: EvidenceReader> Archive<R> {
    fn writer_finished(&self) -> bool {
        self.reader.final_summary().is_some() && self.reader.is_finished()
    }

    fn poll(&mut self) {
        if self.finalized || self.persistence.error.is_some() {
            return;
        }
        self.drain(DRAIN_LIMIT - self.pending.len());
        if self.pending.len() >= FLUSH_FRAMES || self.last_flush.elapsed() >= FLUSH_INTERVAL {
            self.flush();
        }
    }

    fn drain(&mut self, limit: usize) -> usize {
        let mut drained = 0;
        for _ in 0..limit.min(DRAIN_LIMIT - self.pending.len()) {
            let Some(frame) = self.reader.pop() else {
                break;
            };
            self.pending.push(frame);
            drained += 1;
        }
        drained
    }

    fn flush(&mut self) {
        if self.persistence.error.is_none() && self.persistence.persist(&self.pending).is_ok() {
            self.pending.clear();
        }
        self.last_flush = Instant::now();
    }

    fn finish(&mut self) -> Result<()> {
        if self.finalized {
            return self.finish_result();
        }
        // Only a control-side joined summary permits a final drain. A finite
        // frame budget, not a live producer/empty retry loop, bounds shutdown.
        let summary = self.reader.final_summary();
        if summary.is_some() {
            let mut remaining = self.persistence.config.capacity_frames;
            self.flush();
            while remaining > 0 && self.persistence.error.is_none() {
                let drained = self.drain(remaining);
                remaining -= drained;
                self.flush();
                if drained == 0 || self.reader.is_finished() {
                    break;
                }
            }
        } else {
            self.flush();
        }
        let validation = summary
            .as_ref()
            .context("native render final summary unavailable before control-side join")
            .and_then(|summary| {
                self.persistence
                    .validate_final(summary, self.reader.is_finished())
            });
        let native_submit_failed = summary
            .as_ref()
            .is_some_and(|summary| summary.rejected_frames > 0);
        self.finish_error = self
            .persistence
            .error
            .clone()
            .or_else(|| validation.as_ref().err().map(|error| format!("{error:#}")))
            .or_else(|| {
                native_submit_failed.then(|| {
                    "native render submit rejected; evidence preserved, not a quality pass".into()
                })
            });
        let receipt = serde_json::json!({
            "schema_version": 1, "summary": summary,
            "persisted_frames": self.persistence.persisted_frames,
            "persisted_packets": self.persistence.persisted_packets,
            "persisted_bytes": self.persistence.persisted_bytes,
            "partial_packet": self.persistence.mapping.partial_packet(),
            "pending_frames": self.pending.len(),
            "error": self.finish_error, "native_submit_failed": native_submit_failed,
            "persistence_complete": validation.is_ok(), "qualified": false,
        });
        // Space for this receipt was reserved before any payload write.
        ensure!(
            serde_json::to_vec(&receipt)?.len() as u64 <= RECEIPT_LIMIT,
            "native render final receipt too large"
        );
        if let Err(error) = write_new(&self.directory.join("final.json"), &receipt) {
            let previous = self.finish_error.take().unwrap_or_default();
            self.finish_error = Some(format!(
                "{previous}; native render final receipt: {error:#}"
            ));
            self.finalized = true;
            return self.finish_result();
        }
        self.complete = validation.is_ok();
        self.finalized = true;
        self.finish_result()
    }

    fn finish_result(&self) -> Result<()> {
        match &self.finish_error {
            Some(error) => Err(anyhow::anyhow!("{error}")),
            None => Ok(()),
        }
    }

    fn persistence_complete(&self) -> bool {
        self.finalized && self.complete
    }
}

pub(super) struct RenderArchive(Archive<NativeRenderEvidenceReader>);

impl RenderArchive {
    pub(super) fn new(owner_directory: &Path, reader: NativeRenderEvidenceReader) -> Result<Self> {
        let origin = reader.origin();
        let (directory, persistence) =
            prepare(owner_directory, origin, origin.format, origin.config)?;
        Ok(Self(Archive {
            directory,
            reader,
            persistence,
            pending: Vec::with_capacity(DRAIN_LIMIT),
            last_flush: Instant::now(),
            finalized: false,
            complete: false,
            finish_error: None,
        }))
    }

    pub(super) fn poll(&mut self) {
        self.0.poll();
    }
    pub(super) fn writer_finished(&self) -> bool {
        self.0.writer_finished()
    }
    pub(super) fn finish(&mut self) -> Result<()> {
        self.0.finish()
    }
    pub(super) fn persistence_complete(&self) -> bool {
        self.0.persistence_complete()
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
