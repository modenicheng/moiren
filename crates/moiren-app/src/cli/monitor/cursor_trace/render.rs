//! Encoded render evidence only; successful storage is never audio qualification.
use super::write_new;
use anyhow::{Context, Result, ensure};
use moiren_engine::audio_config::SampleFormat;
use moiren_windows_audio::{
    engine_backend::NativeRenderEvidenceReader,
    render_evidence::{
        RENDER_EVENT_CAPACITY, RenderEvent, RenderEvidenceConfig, RenderEvidenceFormat,
        RenderEvidenceFrame, RenderEvidenceSummary, RenderTimelineSummary,
        render_timeline_storage_bytes,
    },
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DISK_LIMIT: u64 = 512 * 1024 * 1024;
const RECEIPT_LIMIT: u64 = 32 * 1024;
const DRAIN_LIMIT: usize = 4096;
const FLUSH_FRAMES: usize = 2048;
const FLUSH_INTERVAL: Duration = Duration::from_millis(50);
const SIDECAR_LINE_LIMIT: usize = 4096;

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
        .and_then(|storage| storage.checked_add(render_timeline_storage_bytes()))
        .context("native render evidence capacity exhausted")?;
    ensure!(
        storage <= 8 * 1024 * 1024,
        "native render preparation budget invalid"
    );
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    reserved_bytes: u64,
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
            reserved_bytes: 0,
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
        // A failed physical tail still consumes its reserved disk budget while
        // the independent timeline continues. It is never a committed frontier.
        self.reserved_bytes = bytes;
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

#[derive(Clone, Copy, Default)]
struct TimelineMapping {
    events: u64,
    last_sequence: u64,
    missing_events: u64,
    unavailable_clocks: u64,
    regressed_clocks: u64,
    last_qpc: Option<u64>,
}

impl TimelineMapping {
    fn accept(&mut self, event: &RenderEvent) -> Result<()> {
        ensure!(
            event.sequence > self.last_sequence,
            "native render timeline sequence repeated/regressed"
        );
        self.missing_events = self
            .missing_events
            .checked_add(event.sequence - self.last_sequence - 1)
            .context("render timeline missing counter exhausted")?;
        self.events = self
            .events
            .checked_add(1)
            .context("render timeline counter exhausted")?;
        self.last_sequence = event.sequence;
        let before = event.sample.qpc_before_100ns;
        let after = event.sample.qpc_after_100ns;
        if before.is_none() || after.is_none() {
            self.unavailable_clocks = self
                .unavailable_clocks
                .checked_add(1)
                .context("render timeline unavailable counter exhausted")?;
        }
        if before.zip(after).is_some_and(|(a, b)| b < a)
            || self.last_qpc.zip(before).is_some_and(|(a, b)| b < a)
        {
            self.regressed_clocks = self
                .regressed_clocks
                .checked_add(1)
                .context("render timeline regression counter exhausted")?;
        }
        self.last_qpc = after;
        Ok(())
    }
}

struct TimelinePersistence<S: ArchiveSink> {
    sink: Option<S>,
    mapping: TimelineMapping,
    persisted_events: u64,
    persisted_bytes: u64,
    reserved_bytes: u64,
    limit: u64,
    error: Option<String>,
}

impl<S: ArchiveSink> TimelinePersistence<S> {
    fn from_sink(sink: std::io::Result<S>, limit: u64) -> Self {
        let (sink, error) = match sink {
            Ok(sink) => (Some(sink), None),
            Err(error) => (None, Some(format!("create render timeline: {error}"))),
        };
        Self {
            sink,
            mapping: TimelineMapping::default(),
            persisted_events: 0,
            persisted_bytes: 0,
            reserved_bytes: 0,
            limit,
            error,
        }
    }

    fn persist(&mut self, events: &[RenderEvent]) -> Result<()> {
        ensure!(
            self.error.is_none(),
            "native render timeline persistence already refused"
        );
        let result = self.persist_batch(events);
        if let Err(error) = &result {
            self.error = Some(format!("{error:#}"));
        }
        result
    }

    fn persist_batch(&mut self, events: &[RenderEvent]) -> Result<()> {
        ensure!(
            events.len() <= DRAIN_LIMIT,
            "native render timeline batch exceeds bounded drain"
        );
        if events.is_empty() {
            return Ok(());
        }
        let mut mapping = self.mapping;
        let mut encoded = Vec::new();
        for event in events {
            mapping.accept(event)?;
            // Preserve raw unknown/regressed clocks and packet identities. Gaps
            // remain visible; they refuse completeness, not later event storage.
            serde_json::to_writer(&mut encoded, event)?;
            encoded.push(b'\n');
        }
        let bytes = self
            .persisted_bytes
            .checked_add(encoded.len() as u64)
            .context("render timeline disk counter exhausted")?;
        ensure!(
            bytes <= self.limit,
            "native render timeline per-owner disk cap reached"
        );
        self.reserved_bytes = bytes;
        let sink = self
            .sink
            .as_mut()
            .context("native render timeline file unavailable")?;
        sink.write_all(&encoded).context("write render timeline")?;
        sink.flush().context("flush render timeline")?;
        sink.sync_all().context("sync render timeline")?;
        self.mapping = mapping;
        self.persisted_events = mapping.events;
        self.persisted_bytes = bytes;
        Ok(())
    }

    fn validate_final(&self, summary: &RenderTimelineSummary, drained: bool) -> Result<()> {
        ensure!(
            self.error.is_none(),
            "native render timeline IO/sequence/cap refusal"
        );
        ensure!(
            drained,
            "native render timeline writer not closed and drained"
        );
        ensure!(
            summary.capacity_events == RENDER_EVENT_CAPACITY,
            "native render timeline capacity mismatch"
        );
        ensure!(
            summary.dropped_events == 0
                && summary.reader_abandoned_events == 0
                && !summary.saturated
                && self.mapping.missing_events == 0,
            "native render timeline loss/abandoned/saturation/sequence gap"
        );
        ensure!(
            summary.events == self.persisted_events
                && summary.stored_events == self.persisted_events
                && summary.events == self.mapping.last_sequence
                && summary.unavailable_clocks == self.mapping.unavailable_clocks
                && summary.regressed_clocks == self.mapping.regressed_clocks,
            "native render timeline final tally mismatch"
        );
        Ok(())
    }
}

#[derive(Deserialize)]
struct TimelineRecord {
    sequence: u64,
    packet_sequence: Option<u64>,
    accepted_frames: u64,
    sample: TimelineSample,
}

#[derive(Deserialize)]
struct TimelineSample {
    kind: String,
    requested_frames: u64,
    submitted_frames: u32,
    release_hresult: Option<i32>,
    failed: bool,
}

fn next_sidecar_line<T: DeserializeOwned>(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
) -> Result<Option<T>> {
    line.clear();
    // Limit before allocation/parse, including malformed files without newlines.
    let bytes = reader
        .take(SIDECAR_LINE_LIMIT as u64 + 1)
        .read_until(b'\n', line)
        .context("read render sidecar line")?;
    if bytes == 0 {
        return Ok(None);
    }
    ensure!(
        bytes <= SIDECAR_LINE_LIMIT && line.last() == Some(&b'\n'),
        "native render sidecar line exceeds 4 KiB or is unterminated"
    );
    serde_json::from_slice(line)
        .context("parse render sidecar line")
        .map(Some)
}

fn sidecar_reader(path: &Path, expected_bytes: u64) -> Result<BufReader<std::io::Take<File>>> {
    let file = File::open(path).context("open render sidecar for crosscheck")?;
    let bytes = file.metadata().context("stat render sidecar")?.len();
    ensure!(
        bytes == expected_bytes && bytes <= DISK_LIMIT,
        "native render sidecar persisted size/cap mismatch"
    );
    // The finite read also prevents an unexpectedly growing file from extending
    // control-side finalization beyond the already committed payload length.
    Ok(BufReader::new(file.take(bytes)))
}

fn crosscheck_timeline(
    directory: &Path,
    audio: &RenderPersistence<File>,
    timeline: &TimelinePersistence<File>,
    summary: &RenderEvidenceSummary,
) -> Result<()> {
    let sample_bytes = audio
        .persisted_frames
        .checked_mul(audio.frame_bytes as u64)
        .context("render sample byte counter exhausted")?;
    let packet_bytes = audio
        .persisted_bytes
        .checked_sub(sample_bytes)
        .context("render packet byte counter invalid")?;
    let mut packets = sidecar_reader(&directory.join("packets.jsonl"), packet_bytes)?;
    let mut events = sidecar_reader(&directory.join("timeline.jsonl"), timeline.persisted_bytes)?;
    let mut line = Vec::with_capacity(SIDECAR_LINE_LIMIT + 1);
    let mut sequence = 0_u64;
    let mut packet_sequence = 0_u64;
    let mut stored_frames = 0_u64;
    let mut accepted_frames = 0_u64;
    let mut rejected_frames = 0_u64;
    while let Some(event) = next_sidecar_line::<TimelineRecord>(&mut events, &mut line)? {
        sequence = sequence
            .checked_add(1)
            .context("render event sequence exhausted")?;
        ensure!(
            event.sequence == sequence,
            "native render crosscheck event sequence mismatch"
        );
        let sample = &event.sample;
        if sample.kind == "write" {
            ensure!(
                u64::from(sample.submitted_frames) <= sample.requested_frames,
                "native render crosscheck submitted frames exceed request"
            );
            if let Some(id) = event.packet_sequence {
                let packet = next_sidecar_line::<PacketMetadata>(&mut packets, &mut line)?
                    .context("native render crosscheck Write has no PCM packet")?;
                packet_sequence = packet_sequence
                    .checked_add(1)
                    .context("render packet sequence exhausted")?;
                ensure!(
                    id == packet_sequence
                        && packet.packet_sequence == id
                        && packet.packet_frames > 0
                        && packet.packet_frames as usize <= audio.config.max_packet_frames
                        && packet.stored_before_frames == stored_frames
                        && packet.packet_frames == sample.submitted_frames
                        && sample.release_hresult == Some(packet.release_hresult)
                        && sample.failed == (packet.release_hresult < 0),
                    "native render crosscheck Write/PCM packet identity/size/HRESULT mismatch"
                );
                stored_frames = stored_frames
                    .checked_add(u64::from(packet.packet_frames))
                    .context("render stored frontier exhausted")?;
                let frontier = if packet.release_hresult >= 0 {
                    &mut accepted_frames
                } else {
                    &mut rejected_frames
                };
                *frontier = frontier
                    .checked_add(u64::from(packet.packet_frames))
                    .context("render outcome frontier exhausted")?;
            } else {
                // Missing staging/release witnesses cannot certify completeness.
                // A zero write or pre-reservation error is valid without a packet.
                ensure!(
                    sample.submitted_frames == 0 && sample.release_hresult.is_none(),
                    "native render crosscheck unassociated Write has a release"
                );
            }
        } else {
            ensure!(
                matches!(sample.kind.as_str(), "start" | "stop" | "wait")
                    && event.packet_sequence.is_none()
                    && sample.submitted_frames == 0
                    && sample.release_hresult.is_none(),
                "native render crosscheck non-Write carries packet/release metadata"
            );
        }
        ensure!(
            event.accepted_frames == accepted_frames,
            "native render crosscheck accepted frontier mismatch"
        );
    }
    ensure!(
        next_sidecar_line::<PacketMetadata>(&mut packets, &mut line)?.is_none(),
        "native render crosscheck PCM packet missing Write event"
    );
    ensure!(
        sequence == timeline.persisted_events
            && packet_sequence == audio.persisted_packets
            && packet_sequence == summary.attempts
            && stored_frames == audio.persisted_frames
            && stored_frames == summary.stored_frames
            && accepted_frames == summary.accepted_frames
            && rejected_frames == summary.rejected_frames,
        "native render crosscheck final packet/frontier tally mismatch"
    );
    Ok(())
}

trait EvidenceReader {
    fn pop(&mut self) -> Option<RenderEvidenceFrame>;
    fn is_finished(&self) -> bool;
    fn final_summary(&self) -> Option<RenderEvidenceSummary>;
    fn pop_event(&mut self) -> Option<RenderEvent> {
        None
    }
    fn events_finished(&self) -> bool {
        true
    }
}

impl EvidenceReader for NativeRenderEvidenceReader {
    fn pop_event(&mut self) -> Option<RenderEvent> {
        self.pop_event()
    }
    fn events_finished(&self) -> bool {
        self.events_finished()
    }
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
) -> Result<(PathBuf, RenderPersistence<File>, TimelinePersistence<File>)> {
    let bytes = frame_bytes(format, config)?;
    let stream = serde_json::json!({
        "schema_version": 1, "origin": origin, "frame_bytes": bytes,
        "disk_limit_bytes": DISK_LIMIT, "qualified": false,
        "timeline_capacity_events": RENDER_EVENT_CAPACITY,
        "timeline_scope": "raw native service/submit timing and padding; not underrun, playback or physical latency qualification",
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
    // Timeline creation is independently sticky: never discard the successfully
    // opened audio archive merely because the diagnostic sidecar is unavailable.
    let timeline = TimelinePersistence::from_sink(
        create("timeline.jsonl"),
        DISK_LIMIT - stream_bytes - RECEIPT_LIMIT,
    );
    Ok((directory, persistence, timeline))
}

struct Archive<R: EvidenceReader> {
    directory: PathBuf,
    reader: R,
    persistence: RenderPersistence<File>,
    timeline: TimelinePersistence<File>,
    pending: Vec<RenderEvidenceFrame>,
    pending_events: Vec<RenderEvent>,
    last_flush: Instant,
    finalized: bool,
    complete: bool,
    finish_error: Option<String>,
}

impl<R: EvidenceReader> Archive<R> {
    fn writer_finished(&self) -> bool {
        self.reader.final_summary().is_some()
            && self.reader.is_finished()
            // A sticky diagnostic failure stops draining that ring; it must not
            // prevent a joined owner's independent audio/final receipt closure.
            && (self.timeline.error.is_some() || self.reader.events_finished())
    }

    fn poll(&mut self) {
        if self.finalized {
            return;
        }
        if self.persistence.error.is_none() {
            self.drain(DRAIN_LIMIT - self.pending.len());
        }
        if self.timeline.error.is_none() {
            self.drain_events(DRAIN_LIMIT - self.pending_events.len());
        }
        if self.pending.len() >= FLUSH_FRAMES
            || self.pending_events.len() >= FLUSH_FRAMES
            || self.last_flush.elapsed() >= FLUSH_INTERVAL
        {
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
        self.flush_audio();
        self.flush_events();
        self.last_flush = Instant::now();
    }

    fn flush_audio(&mut self) {
        let limit = self.persistence.limit;
        self.persistence.limit = limit.saturating_sub(self.timeline.reserved_bytes);
        if self.persistence.error.is_none() && self.persistence.persist(&self.pending).is_ok() {
            self.pending.clear();
        }
        self.persistence.limit = limit;
    }

    fn drain_events(&mut self, limit: usize) -> usize {
        let mut drained = 0;
        for _ in 0..limit.min(DRAIN_LIMIT - self.pending_events.len()) {
            let Some(event) = self.reader.pop_event() else {
                break;
            };
            self.pending_events.push(event);
            drained += 1;
        }
        drained
    }

    fn flush_events(&mut self) {
        let limit = self.timeline.limit;
        self.timeline.limit = limit.saturating_sub(self.persistence.reserved_bytes);
        if self.timeline.error.is_none() && self.timeline.persist(&self.pending_events).is_ok() {
            self.pending_events.clear();
        }
        self.timeline.limit = limit;
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
            self.flush_audio();
            while remaining > 0 && self.persistence.error.is_none() {
                let drained = self.drain(remaining);
                remaining -= drained;
                self.flush_audio();
                if drained == 0 || self.reader.is_finished() {
                    break;
                }
            }
            let mut remaining = RENDER_EVENT_CAPACITY;
            self.flush_events();
            while remaining > 0 && self.timeline.error.is_none() {
                let drained = self.drain_events(remaining);
                remaining -= drained;
                self.flush_events();
                if drained == 0 || self.reader.events_finished() {
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
        let timeline_validation = summary
            .as_ref()
            .context("native render timeline final summary unavailable before control-side join")
            .and_then(|summary| {
                self.timeline
                    .validate_final(&summary.timeline, self.reader.events_finished())
                    .and_then(|()| {
                        crosscheck_timeline(
                            &self.directory,
                            &self.persistence,
                            &self.timeline,
                            summary,
                        )
                    })
            });
        let timeline_error = self.timeline.error.clone().or_else(|| {
            timeline_validation
                .as_ref()
                .err()
                .map(|error| format!("{error:#}"))
        });
        let audio_error = self
            .persistence
            .error
            .clone()
            .or_else(|| validation.as_ref().err().map(|error| format!("{error:#}")))
            .or_else(|| {
                native_submit_failed.then(|| {
                    "native render submit rejected; evidence preserved, not a quality pass".into()
                })
            });
        self.finish_error = match (audio_error, &timeline_error) {
            (Some(audio), Some(timeline)) => Some(format!("{audio}; timeline: {timeline}")),
            (Some(audio), None) => Some(audio),
            (None, Some(timeline)) => Some(format!("native render timeline: {timeline}")),
            (None, None) => None,
        };
        let complete = validation.is_ok() && timeline_validation.is_ok();
        let receipt = serde_json::json!({
            "schema_version": 1, "summary": summary,
            "persisted_frames": self.persistence.persisted_frames,
            "persisted_packets": self.persistence.persisted_packets,
            "persisted_bytes": self.persistence.persisted_bytes,
            "partial_packet": self.persistence.mapping.partial_packet(),
            "pending_frames": self.pending.len(),
            "error": self.finish_error, "native_submit_failed": native_submit_failed,
            "audio_persistence_complete": validation.is_ok(),
            "timeline": {
                "persisted_events": self.timeline.persisted_events,
                "persisted_bytes": self.timeline.persisted_bytes,
                "reserved_bytes": self.timeline.reserved_bytes,
                "last_sequence": self.timeline.mapping.last_sequence,
                "missing_events": self.timeline.mapping.missing_events,
                "unavailable_clocks": self.timeline.mapping.unavailable_clocks,
                "regressed_clocks": self.timeline.mapping.regressed_clocks,
                "pending_events": self.pending_events.len(),
                "pending_first_sequence": self.pending_events.first().map(|event| event.sequence),
                "pending_last_sequence": self.pending_events.last().map(|event| event.sequence),
                "writer_closed_and_drained": self.reader.events_finished(),
                "error": timeline_error,
                "persistence_complete": timeline_validation.is_ok(),
            },
            "persistence_complete": complete, "qualified": false,
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
        self.complete = complete;
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
        let (directory, persistence, timeline) =
            prepare(owner_directory, origin, origin.format, origin.config)?;
        Ok(Self(Archive {
            directory,
            reader,
            persistence,
            timeline,
            pending: Vec::with_capacity(DRAIN_LIMIT),
            pending_events: Vec::with_capacity(DRAIN_LIMIT),
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
