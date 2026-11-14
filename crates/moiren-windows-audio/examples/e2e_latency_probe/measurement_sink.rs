//! Bounded non-RT raw evidence storage; not physical-latency qualification.

use anyhow::{Context, Result, ensure};
use moiren_windows_audio::capture::{PcmEvidenceFrame, PcmEvidenceReader};
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_PCM_BYTES: u64 = 3 * 1024 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PACKETS: u64 = 1_000_000;
const MAX_RECORD_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SinkLimits {
    pub(crate) max_frames: u64,
    pub(crate) max_pcm_bytes: u64,
    pub(crate) max_metadata_bytes: u64,
    pub(crate) max_packets: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SinkReport {
    pub(crate) schema_version: u32,
    pub(crate) sample_rate: u32,
    pub(crate) channels: usize,
    /// Accepted complete PCM frames, not a durability guarantee after an IO failure.
    pub(crate) written_frames: u64,
    /// Bytes accepted by the output wrapper; a failed flush may persist fewer.
    pub(crate) written_pcm_bytes: u64,
    pub(crate) written_metadata_bytes: u64,
    /// Only retained as complete if both final flushes succeed (not fsync).
    pub(crate) complete_packets: u64,
    pub(crate) sequence_gaps: u64,
    pub(crate) missing_packets: u64,
    pub(crate) incomplete_packets: u64,
    pub(crate) nonfinite_samples: u64,
    pub(crate) counters_saturated: bool,
    pub(crate) cancelled: bool,
    pub(crate) failure: Option<String>,
}

pub(crate) struct EvidenceSink {
    worker: Option<JoinHandle<SinkReport>>,
    cancelled: Arc<AtomicBool>,
}

impl EvidenceSink {
    pub(crate) fn create(
        reader: PcmEvidenceReader,
        sample_rate: u32,
        channels: usize,
        pcm_path: &Path,
        packet_path: &Path,
        limits: SinkLimits,
    ) -> Result<Self> {
        limits.validate(sample_rate, channels)?;
        ensure!(
            pcm_path != packet_path,
            "PCM and metadata output paths must differ"
        );
        let pcm_identity = output_identity(pcm_path)?;
        let metadata_identity = output_identity(packet_path)?;
        #[cfg(windows)]
        let identical = pcm_identity
            .as_os_str()
            .eq_ignore_ascii_case(metadata_identity.as_os_str());
        #[cfg(not(windows))]
        let identical = pcm_identity == metadata_identity;
        ensure!(!identical, "PCM and metadata output paths must differ");
        let pcm = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(pcm_path)
            .with_context(|| format!("create new PCM evidence {}", pcm_path.display()))?;
        // Preserve the first artifact even if opening the second output fails.
        let metadata = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(packet_path)
            .with_context(|| format!("create new packet evidence {}", packet_path.display()))?;
        // Fixed buffers amortize per-frame writes without duration-sized storage.
        Self::from_sinks(
            reader,
            sample_rate,
            channels,
            BufWriter::with_capacity(64 * 1024, pcm),
            BufWriter::with_capacity(16 * 1024, metadata),
            limits,
        )
    }

    fn from_sinks(
        mut reader: PcmEvidenceReader,
        sample_rate: u32,
        channels: usize,
        pcm: impl Write + Send + 'static,
        packets: impl Write + Send + 'static,
        limits: SinkLimits,
    ) -> Result<Self> {
        limits.validate(sample_rate, channels)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        let worker = thread::Builder::new()
            .name("moiren-measurement-sink".into())
            .spawn(move || {
                let mut output = PacketWriter::new(pcm, packets, sample_rate, channels, limits);
                let failure = loop {
                    if cancellation.load(Ordering::Acquire) {
                        break None;
                    }
                    if let Some(frame) = reader.pop() {
                        if let Err(error) = output.accept(frame) {
                            break Some(format!("{error:#}"));
                        }
                    } else if reader.is_finished() {
                        break None;
                    } else {
                        // Poll only on this non-RT worker; no queue or duration-sized allocation.
                        thread::sleep(Duration::from_millis(1));
                    }
                };
                output.complete(cancellation.load(Ordering::Acquire), failure)
            })
            .context("spawn measurement evidence worker")?;
        Ok(Self {
            worker: Some(worker),
            cancelled,
        })
    }

    pub(crate) fn check(&self) -> Result<()> {
        ensure!(
            !self.worker.as_ref().is_none_or(JoinHandle::is_finished),
            "measurement evidence worker stopped before collection finished"
        );
        Ok(())
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// The caller must stop and join native capture before joining this worker.
    /// Without cancellation, all remaining frames are drained through writer EOF.
    pub(crate) fn finish(&mut self) -> Result<SinkReport> {
        let mut report = self
            .worker
            .take()
            .context("measurement evidence worker owner missing")?
            .join()
            .map_err(|_| anyhow::anyhow!("measurement evidence worker panicked"))?;
        // Cancellation can arrive while the worker is blocked in its final flush.
        report.cancelled |= self.cancelled.load(Ordering::Acquire);
        Ok(report)
    }
}

impl Drop for EvidenceSink {
    fn drop(&mut self) {
        // Cancellation cannot interrupt disk IO; ownership must survive until join.
        self.cancel();
        if self.worker.is_some() {
            let _ = self.finish();
        }
    }
}

impl SinkLimits {
    fn validate(self, sample_rate: u32, channels: usize) -> Result<()> {
        ensure!(
            sample_rate > 0 && (1..=2).contains(&channels),
            "evidence requires nonzero rate and mono/stereo"
        );
        ensure!(
            self.max_frames > 0
                && self.max_pcm_bytes > 0
                && self.max_metadata_bytes > 0
                && self.max_packets > 0,
            "evidence caps must all be nonzero"
        );
        ensure!(
            self.max_pcm_bytes <= MAX_PCM_BYTES,
            "PCM hard limit is 3 GiB"
        );
        ensure!(
            self.max_metadata_bytes <= MAX_METADATA_BYTES,
            "metadata hard limit is 256 MiB"
        );
        ensure!(
            self.max_packets <= MAX_PACKETS,
            "packet hard limit is 1,000,000"
        );
        self.max_frames
            .checked_mul(channels as u64 * 4)
            .context("frame cap byte arithmetic overflow")?;
        Ok(())
    }
}

fn output_identity(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("evidence output requires a file name")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent
        .canonicalize()
        .context("resolve evidence output directory")?
        .join(name))
}

struct PacketWriter<P: Write, M: Write> {
    pcm: P,
    metadata: M,
    report: SinkReport,
    limits: SinkLimits,
    active: Option<ActivePacket>,
    last_sequence: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
struct PacketMetadata {
    packet_sequence: u64,
    flags: u32,
    device_position_frames: u64,
    qpc_100ns: u64,
    frames: u32,
    stored_start_frame: u64,
}

struct ActivePacket {
    metadata: PacketMetadata,
    next_offset: u32,
    record: BoundedRecord,
}

struct BoundedRecord {
    bytes: [u8; MAX_RECORD_BYTES],
    len: usize,
}

impl Write for BoundedRecord {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .len
            .checked_add(bytes.len())
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| io::Error::other("packet metadata exceeds 1024-byte record cap"))?;
        self.bytes[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<P: Write, M: Write> PacketWriter<P, M> {
    fn new(pcm: P, metadata: M, sample_rate: u32, channels: usize, limits: SinkLimits) -> Self {
        Self {
            pcm,
            metadata,
            limits,
            active: None,
            last_sequence: 0,
            report: SinkReport {
                schema_version: 1,
                sample_rate,
                channels,
                written_frames: 0,
                written_pcm_bytes: 0,
                written_metadata_bytes: 0,
                complete_packets: 0,
                sequence_gaps: 0,
                missing_packets: 0,
                incomplete_packets: 0,
                nonfinite_samples: 0,
                counters_saturated: false,
                cancelled: false,
                failure: None,
            },
        }
    }

    fn accept(&mut self, frame: PcmEvidenceFrame) -> Result<()> {
        if self.active.is_none() {
            ensure!(
                frame.packet_frames > 0,
                "packet must declare nonzero frames"
            );
            ensure!(
                frame.packet_frame_offset == 0,
                "packet must begin at offset zero"
            );
            ensure!(
                frame.packet_sequence > self.last_sequence,
                "packet sequence wrapped or reordered"
            );
            let metadata = PacketMetadata {
                packet_sequence: frame.packet_sequence,
                flags: frame.flags,
                device_position_frames: frame.device_position_frames,
                qpc_100ns: frame.qpc_100ns,
                frames: frame.packet_frames,
                stored_start_frame: self.report.written_frames,
            };
            let mut record = BoundedRecord {
                bytes: [0; MAX_RECORD_BYTES],
                len: 0,
            };
            serde_json::to_writer(&mut record, &metadata).context("encode packet metadata")?;
            record.write_all(b"\n")?;
            let frames = u64::from(frame.packet_frames);
            let pcm_bytes = frames
                .checked_mul(self.report.channels as u64 * 4)
                .context("packet byte arithmetic overflow")?;
            // Reserve a whole packet before its first byte. No duration-sized PCM
            // is retained, and a metadata cap cannot strand its PCM deliberately.
            ensure_budget(
                self.report.written_frames,
                frames,
                self.limits.max_frames,
                "frame cap",
            )?;
            ensure_budget(
                self.report.written_pcm_bytes,
                pcm_bytes,
                self.limits.max_pcm_bytes,
                "PCM byte cap",
            )?;
            ensure_budget(
                self.report.written_metadata_bytes,
                record.len as u64,
                self.limits.max_metadata_bytes,
                "metadata byte cap",
            )?;
            ensure_budget(
                self.report.complete_packets,
                1,
                self.limits.max_packets,
                "packet cap",
            )?;
            let missing = frame.packet_sequence - self.last_sequence - 1;
            if missing != 0 {
                add(
                    &mut self.report.sequence_gaps,
                    1,
                    &mut self.report.counters_saturated,
                );
                add(
                    &mut self.report.missing_packets,
                    missing,
                    &mut self.report.counters_saturated,
                );
            }
            self.active = Some(ActivePacket {
                metadata,
                next_offset: 0,
                record,
            });
        }
        let active = self.active.as_mut().expect("packet admitted above");
        let identity = active.metadata;
        ensure!(
            frame.packet_sequence == identity.packet_sequence
                && frame.packet_frames == identity.frames
                && frame.flags == identity.flags
                && frame.device_position_frames == identity.device_position_frames
                && frame.qpc_100ns == identity.qpc_100ns,
            "packet metadata identity changed mid-packet"
        );
        ensure!(
            frame.packet_frame_offset == active.next_offset,
            "nonconsecutive packet frame offset"
        );
        let mut bytes = [0u8; 8];
        for (sample, destination) in frame
            .samples
            .iter()
            .zip(bytes.as_chunks_mut::<4>().0.iter_mut())
            .take(self.report.channels)
        {
            destination.copy_from_slice(&sample.to_bits().to_le_bytes());
        }
        let width = self.report.channels * 4;
        let before = self.report.written_pcm_bytes;
        let result = write_counted(
            &mut self.pcm,
            &bytes[..width],
            &mut self.report.written_pcm_bytes,
        );
        self.report.written_frames = self.report.written_pcm_bytes / width as u64;
        let samples_written = ((self.report.written_pcm_bytes - before) / 4) as usize;
        let nonfinite = frame
            .samples
            .iter()
            .take(samples_written)
            .filter(|sample| !sample.is_finite())
            .count() as u64;
        add(
            &mut self.report.nonfinite_samples,
            nonfinite,
            &mut self.report.counters_saturated,
        );
        result.context("write raw PCM evidence")?;
        active.next_offset = active
            .next_offset
            .checked_add(1)
            .context("packet offset overflow")?;
        if active.next_offset == active.metadata.frames {
            // These completions are provisional until both final flushes succeed.
            // Never flush per frame/packet: that defeats the bounded file buffers.
            write_counted(
                &mut self.metadata,
                &active.record.bytes[..active.record.len],
                &mut self.report.written_metadata_bytes,
            )
            .context("write packet metadata evidence")?;
            add(
                &mut self.report.complete_packets,
                1,
                &mut self.report.counters_saturated,
            );
            self.last_sequence = active.metadata.packet_sequence;
            self.active = None;
        }
        Ok(())
    }

    fn complete(mut self, cancelled: bool, failure: Option<String>) -> SinkReport {
        self.report.cancelled = cancelled;
        self.report.failure = failure;
        if self.active.is_some() {
            add(
                &mut self.report.incomplete_packets,
                1,
                &mut self.report.counters_saturated,
            );
            append_failure(
                &mut self.report.failure,
                "incomplete packet at evidence stop",
            );
        }
        // Attempt both flushes even after another IO/protocol failure; do not
        // discard a secondary cleanup error or delete already-written artifacts.
        let mut flush_failed = false;
        if let Err(error) = self.pcm.flush() {
            flush_failed = true;
            append_failure(
                &mut self.report.failure,
                &format!("final PCM flush: {error}"),
            );
        }
        if let Err(error) = self.metadata.flush() {
            flush_failed = true;
            append_failure(
                &mut self.report.failure,
                &format!("final metadata flush: {error}"),
            );
        }
        if flush_failed {
            // Buffered outputs cannot identify which earlier packets persisted.
            // Preserve accepted byte totals but conservatively qualify none.
            add(
                &mut self.report.incomplete_packets,
                self.report.complete_packets,
                &mut self.report.counters_saturated,
            );
            self.report.complete_packets = 0;
        }
        self.report
    }
}

fn ensure_budget(written: u64, amount: u64, cap: u64, label: &str) -> Result<()> {
    ensure!(
        written.checked_add(amount).is_some_and(|end| end <= cap),
        "{label} exceeded"
    );
    Ok(())
}

fn write_counted(sink: &mut impl Write, mut bytes: &[u8], written: &mut u64) -> io::Result<()> {
    while !bytes.is_empty() {
        // Count successful short writes too: write_all alone hides a partial
        // frame/JSON suffix when the following write fails.
        match sink.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) if count <= bytes.len() => {
                *written = written
                    .checked_add(count as u64)
                    .ok_or_else(|| io::Error::other("written byte counter overflow"))?;
                bytes = &bytes[count..];
            }
            Ok(_) => return Err(io::Error::other("sink returned invalid write length")),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
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

fn append_failure(failure: &mut Option<String>, message: &str) {
    if let Some(failure) = failure {
        failure.push_str("; ");
        failure.push_str(message);
    } else {
        *failure = Some(message.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_windows_audio::capture::{
        PcmEvidenceConfig, PcmEvidencePacket, PcmEvidenceWriter, pcm_evidence_stream,
    };
    use std::{
        io,
        sync::Mutex,
        thread,
        time::{Duration, Instant},
    };

    #[derive(Clone, Default)]
    struct Bytes(Arc<Mutex<Vec<u8>>>);

    impl Bytes {
        fn snapshot(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Write for Bytes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn limits() -> SinkLimits {
        SinkLimits {
            max_frames: 10_000,
            max_pcm_bytes: 80_000,
            max_metadata_bytes: 100_000,
            max_packets: 1_000,
        }
    }

    fn stream(capacity_frames: usize, channels: usize) -> (PcmEvidenceWriter, PcmEvidenceReader) {
        pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames,
                detect_position_gaps: false,
            },
            48_000,
            channels,
        )
        .unwrap()
    }

    fn packet(frames: u32, position: u64) -> PcmEvidencePacket {
        PcmEvidencePacket {
            frames,
            flags: 0x8000_0000,
            device_position_frames: position,
            qpc_100ns: 42 + position,
        }
    }

    fn wait_for(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate() {
            assert!(
                Instant::now() < deadline,
                "worker did not make bounded-time test progress"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn streams_raw_mono_and_stereo_bits_and_metadata_through_ring_wrap() {
        for channels in [1, 2] {
            let (mut native, reader) = stream(3, channels);
            let pcm = Bytes::default();
            let metadata = Bytes::default();
            let mut sink = EvidenceSink::from_sinks(
                reader,
                48_000,
                channels,
                pcm.clone(),
                metadata.clone(),
                limits(),
            )
            .unwrap();
            let mut expected = Vec::new();
            for index in 0..40u64 {
                let bits = [0x8000_0000u32, 0x7fc0_1234, 0xff80_0000, 0x3e80_0000];
                let bytes: Vec<_> = bits[..channels * 2]
                    .iter()
                    .flat_map(|bits| bits.to_le_bytes())
                    .collect();
                assert_eq!(
                    native.record_packet(&bytes, packet(2, index * 2)).unwrap(),
                    2
                );
                expected.extend_from_slice(&bytes);
                wait_for(|| {
                    metadata
                        .snapshot()
                        .iter()
                        .filter(|&&byte| byte == b'\n')
                        .count()
                        == index as usize + 1
                });
            }
            native.close();
            let report = sink.finish().unwrap();
            assert_eq!(pcm.snapshot(), expected);
            assert_eq!(report.schema_version, 1);
            assert_eq!(report.sample_rate, 48_000);
            assert_eq!(report.channels, channels);
            assert_eq!(report.written_frames, 80);
            assert_eq!(report.written_pcm_bytes, expected.len() as u64);
            assert_eq!(
                report.written_metadata_bytes,
                metadata.snapshot().len() as u64
            );
            assert_eq!(report.complete_packets, 40);
            assert_eq!(
                report.nonfinite_samples,
                if channels == 1 { 40 } else { 80 }
            );
            assert!(!report.counters_saturated);
            assert!(!report.cancelled);
            assert!(report.failure.is_none(), "{report:?}");
            let records: Vec<serde_json::Value> = metadata
                .snapshot()
                .split(|&byte| byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| {
                    assert!(line.len() < 1024);
                    serde_json::from_slice(line).unwrap()
                })
                .collect();
            for (index, record) in records.iter().enumerate() {
                assert_eq!(record["packet_sequence"], index as u64 + 1);
                assert_eq!(record["frames"], 2);
                assert_eq!(record["flags"], 0x8000_0000u32);
                assert_eq!(record["device_position_frames"], index as u64 * 2);
                assert_eq!(record["qpc_100ns"], 42 + index as u64 * 2);
                assert_eq!(record["stored_start_frame"], index as u64 * 2);
            }
            let serialized = serde_json::to_value(&report).unwrap();
            assert_eq!(serialized["complete_packets"], 40);
            assert!(sink.check().is_err());
            assert!(sink.finish().is_err(), "a joined owner cannot join twice");
        }
    }

    #[test]
    fn records_leading_and_internal_whole_packet_loss_without_padding() {
        let (mut native, reader) = stream(2, 1);
        assert_eq!(native.record_packet(&[0; 12], packet(3, 0)).unwrap(), 0);
        assert_eq!(native.record_packet(&[0; 12], packet(3, 3)).unwrap(), 0);
        assert_eq!(native.record_packet(&[0; 4], packet(1, 6)).unwrap(), 1);
        let pcm = Bytes::default();
        let metadata = Bytes::default();
        let mut sink =
            EvidenceSink::from_sinks(reader, 48_000, 1, pcm.clone(), metadata.clone(), limits())
                .unwrap();
        wait_for(|| !metadata.snapshot().is_empty());
        assert_eq!(native.record_packet(&[0; 12], packet(3, 7)).unwrap(), 0);
        assert_eq!(native.record_packet(&[0; 4], packet(1, 10)).unwrap(), 1);
        native.close();
        let report = sink.finish().unwrap();
        assert_eq!(report.sequence_gaps, 2);
        assert_eq!(report.missing_packets, 3);
        assert_eq!(report.complete_packets, 2);
        assert_eq!(report.written_frames, 2);
        assert_eq!(pcm.snapshot().len(), 8);
        assert!(
            report.failure.is_none(),
            "gaps are evidence, not concealed or padded: {report:?}"
        );
        let records: Vec<serde_json::Value> = metadata
            .snapshot()
            .split(|&byte| byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(records[0]["packet_sequence"], 3);
        assert_eq!(records[1]["packet_sequence"], 5);
        assert_eq!(records[1]["stored_start_frame"], 1);
    }

    #[test]
    fn caps_reserve_complete_packet_before_any_over_budget_write() {
        for (caps, expected_frames, expected_packets, error) in [
            (
                SinkLimits {
                    max_frames: 1,
                    ..limits()
                },
                0,
                0,
                "frame cap",
            ),
            (
                SinkLimits {
                    max_pcm_bytes: 7,
                    ..limits()
                },
                0,
                0,
                "PCM byte cap",
            ),
            (
                SinkLimits {
                    max_metadata_bytes: 1,
                    ..limits()
                },
                0,
                0,
                "metadata byte cap",
            ),
            (
                SinkLimits {
                    max_packets: 1,
                    ..limits()
                },
                2,
                1,
                "packet cap",
            ),
        ] {
            let (mut native, reader) = stream(4, 1);
            native.record_packet(&[0; 8], packet(2, 0)).unwrap();
            native.record_packet(&[0; 8], packet(2, 2)).unwrap();
            native.close();
            let pcm = Bytes::default();
            let metadata = Bytes::default();
            let mut sink =
                EvidenceSink::from_sinks(reader, 48_000, 1, pcm.clone(), metadata.clone(), caps)
                    .unwrap();
            wait_for(|| sink.check().is_err());
            let report = sink.finish().unwrap();
            assert_eq!(report.written_frames, expected_frames);
            assert_eq!(report.complete_packets, expected_packets);
            assert!(
                report.failure.as_ref().unwrap().contains(error),
                "{report:?}"
            );
            assert!(report.written_pcm_bytes <= caps.max_pcm_bytes);
            assert!(report.written_metadata_bytes <= caps.max_metadata_bytes);
            assert_eq!(pcm.snapshot().len() as u64, report.written_pcm_bytes);
            assert_eq!(
                metadata.snapshot().len() as u64,
                report.written_metadata_bytes
            );
        }
    }

    #[test]
    fn invalid_admission_creates_no_files_or_overwrites_and_keeps_partial_creation() {
        use std::{
            fs,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "moiren-sink-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pcm = root.join("samples.f32le");
        let metadata = root.join("packets.jsonl");
        for (rate, channels, caps) in [
            (0, 1, limits()),
            (48_000, 0, limits()),
            (48_000, 3, limits()),
            (
                48_000,
                1,
                SinkLimits {
                    max_frames: 0,
                    ..limits()
                },
            ),
            (
                48_000,
                1,
                SinkLimits {
                    max_pcm_bytes: 0,
                    ..limits()
                },
            ),
            (
                48_000,
                1,
                SinkLimits {
                    max_metadata_bytes: 0,
                    ..limits()
                },
            ),
            (
                48_000,
                1,
                SinkLimits {
                    max_packets: 0,
                    ..limits()
                },
            ),
            (
                48_000,
                1,
                SinkLimits {
                    max_pcm_bytes: 3 * 1024 * 1024 * 1024 + 1,
                    ..limits()
                },
            ),
            (
                48_000,
                1,
                SinkLimits {
                    max_metadata_bytes: 256 * 1024 * 1024 + 1,
                    ..limits()
                },
            ),
            (
                48_000,
                1,
                SinkLimits {
                    max_packets: 1_000_001,
                    ..limits()
                },
            ),
            (
                48_000,
                2,
                SinkLimits {
                    max_frames: u64::MAX,
                    ..limits()
                },
            ),
        ] {
            let (_native, reader) = stream(1, 1);
            assert!(EvidenceSink::create(reader, rate, channels, &pcm, &metadata, caps).is_err());
            assert!(!pcm.exists() && !metadata.exists());
        }
        for collision in [pcm.clone(), root.join(".").join("samples.f32le")] {
            let (_native, reader) = stream(1, 1);
            assert!(EvidenceSink::create(reader, 48_000, 1, &pcm, &collision, limits()).is_err());
            assert!(
                !pcm.exists(),
                "identical paths must fail before the first create"
            );
        }
        #[cfg(windows)]
        {
            let (_native, reader) = stream(1, 1);
            assert!(
                EvidenceSink::create(
                    reader,
                    48_000,
                    1,
                    &pcm,
                    &root.join("SAMPLES.F32LE"),
                    limits()
                )
                .is_err()
            );
            assert!(!pcm.exists());
        }
        fs::write(&metadata, b"existing evidence").unwrap();
        let (_native, reader) = stream(1, 1);
        assert!(EvidenceSink::create(reader, 48_000, 1, &pcm, &metadata, limits()).is_err());
        assert_eq!(fs::read(&metadata).unwrap(), b"existing evidence");
        assert!(
            pcm.exists(),
            "a failed second create must not delete the first artifact"
        );
        fs::write(&pcm, b"partial PCM").unwrap();
        let (_native, reader) = stream(1, 1);
        assert!(
            EvidenceSink::create(reader, 48_000, 1, &pcm, &root.join("new.jsonl"), limits())
                .is_err()
        );
        assert_eq!(fs::read(&pcm).unwrap(), b"partial PCM");
        assert!(!root.join("new.jsonl").exists());
        // This unique test-owned directory, not failed production evidence, is reclaimed.
        fs::remove_dir_all(root).unwrap();
    }

    struct FaultSink {
        bytes: Bytes,
        remaining: usize,
        fail_flush: bool,
    }

    impl Write for FaultSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::other("injected disk full"));
            }
            let count = bytes.len().min(self.remaining);
            self.remaining -= count;
            self.bytes.write(&bytes[..count])
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("injected flush failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn partial_pcm_metadata_and_flush_failures_return_partial_reports() {
        for (pcm_remaining, metadata_remaining, pcm_flush, metadata_flush) in [
            (3, usize::MAX, false, false),
            (usize::MAX, 5, false, false),
            (usize::MAX, usize::MAX, true, false),
            (usize::MAX, usize::MAX, false, true),
        ] {
            let (mut native, reader) = stream(2, 2);
            native.record_packet(&[0; 16], packet(2, 0)).unwrap();
            native.close();
            let pcm = Bytes::default();
            let metadata = Bytes::default();
            let mut sink = EvidenceSink::from_sinks(
                reader,
                48_000,
                2,
                FaultSink {
                    bytes: pcm.clone(),
                    remaining: pcm_remaining,
                    fail_flush: pcm_flush,
                },
                FaultSink {
                    bytes: metadata.clone(),
                    remaining: metadata_remaining,
                    fail_flush: metadata_flush,
                },
                limits(),
            )
            .unwrap();
            let report = sink.finish().unwrap();
            assert_eq!(
                report.complete_packets, 0,
                "never qualify a partially persisted packet"
            );
            assert_eq!(report.incomplete_packets, 1);
            assert!(
                report.failure.as_ref().unwrap().contains("injected"),
                "{report:?}"
            );
            assert_eq!(report.written_pcm_bytes as usize, pcm.snapshot().len());
            assert_eq!(
                report.written_metadata_bytes as usize,
                metadata.snapshot().len()
            );
            assert_eq!(report.written_frames as usize, pcm.snapshot().len() / 8);
            assert!(!pcm.snapshot().is_empty(), "retain partial PCM");
        }
    }

    struct BlockedSink {
        entered: Option<std::sync::mpsc::SyncSender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        dropped: Arc<AtomicBool>,
    }

    impl Write for BlockedSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered.send(()).unwrap();
                self.release.recv().unwrap();
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for BlockedSink {
        fn drop(&mut self) {
            self.dropped
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    #[test]
    fn cancel_and_health_do_not_join_blocked_io_and_finish_owns_join_after_native_stop() {
        use std::sync::{atomic::Ordering, mpsc};
        let (mut native, reader) = stream(2, 1);
        native.record_packet(&[0; 8], packet(2, 0)).unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (stopped_tx, stopped_rx) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicBool::new(false));
        let mut sink = EvidenceSink::from_sinks(
            reader,
            48_000,
            1,
            BlockedSink {
                entered: Some(entered_tx),
                release: release_rx,
                dropped: dropped.clone(),
            },
            Bytes::default(),
            limits(),
        )
        .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let controller = thread::spawn(move || {
            assert!(sink.check().is_ok(), "blocked IO is not a stopped worker");
            sink.cancel();
            native.close();
            drop(native);
            stopped_tx.send(()).unwrap();
            sink.finish().unwrap()
        });
        let independent_stop = stopped_rx.recv_timeout(Duration::from_secs(2));
        let detached = dropped.load(Ordering::Acquire);
        release_tx.send(()).unwrap();
        let report = controller.join().unwrap();
        assert!(
            independent_stop.is_ok(),
            "cancel/health delayed native shutdown"
        );
        assert!(!detached);
        assert!(dropped.load(Ordering::Acquire));
        assert!(report.cancelled);
        assert_eq!(report.incomplete_packets, 1);
        assert_eq!(report.complete_packets, 0);
        assert_eq!(report.written_frames, 1);
        assert!(report.failure.as_ref().unwrap().contains("incomplete"));
    }

    #[test]
    fn drop_cancels_and_joins_blocked_worker_instead_of_detaching() {
        use std::sync::{atomic::Ordering, mpsc};
        let (mut native, reader) = stream(2, 1);
        native.record_packet(&[0; 8], packet(2, 0)).unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicBool::new(false));
        let sink = EvidenceSink::from_sinks(
            reader,
            48_000,
            1,
            BlockedSink {
                entered: Some(entered_tx),
                release: release_rx,
                dropped: dropped.clone(),
            },
            Bytes::default(),
            limits(),
        )
        .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        native.close();
        drop(native);
        let cancellation = sink.cancelled.clone();
        let owner = thread::spawn(move || {
            drop(sink);
            done_tx.send(()).unwrap();
        });
        wait_for(|| cancellation.load(Ordering::Acquire));
        let detached = done_rx.try_recv().is_ok();
        release_tx.send(()).unwrap();
        owner.join().unwrap();
        assert!(!detached, "Drop returned without joining blocked IO");
        assert!(dropped.load(Ordering::Acquire));
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn clean_eof_stops_health_and_worker_panic_is_an_owner_error() {
        let (mut native, reader) = stream(1, 1);
        let mut sink = EvidenceSink::from_sinks(
            reader,
            48_000,
            1,
            Bytes::default(),
            Bytes::default(),
            limits(),
        )
        .unwrap();
        assert!(sink.check().is_ok());
        native.close();
        wait_for(|| sink.check().is_err());
        let report = sink.finish().unwrap();
        assert!(report.failure.is_none());
        assert!(!report.cancelled);
        assert_eq!(report.written_frames, 0);

        struct PanicSink;
        impl Write for PanicSink {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                panic!("injected worker panic")
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (mut native, reader) = stream(1, 1);
        native.record_packet(&[0; 4], packet(1, 0)).unwrap();
        native.close();
        let mut sink =
            EvidenceSink::from_sinks(reader, 48_000, 1, PanicSink, Bytes::default(), limits())
                .unwrap();
        assert!(sink.finish().unwrap_err().to_string().contains("panicked"));
    }

    fn observed_frames() -> Vec<PcmEvidenceFrame> {
        let (mut native, mut reader) = stream(4, 2);
        native.record_packet(&[0; 16], packet(2, 0)).unwrap();
        native.record_packet(&[0; 16], packet(2, 2)).unwrap();
        native.close();
        std::iter::from_fn(|| reader.pop()).collect()
    }

    fn assemble(frames: &[PcmEvidenceFrame], channels: usize) -> (SinkReport, Vec<u8>, Vec<u8>) {
        let pcm = Bytes::default();
        let metadata = Bytes::default();
        let mut assembler =
            PacketWriter::new(pcm.clone(), metadata.clone(), 48_000, channels, limits());
        let failure = frames.iter().find_map(|&frame| {
            assembler
                .accept(frame)
                .err()
                .map(|error| format!("{error:#}"))
        });
        (
            assembler.complete(false, failure),
            pcm.snapshot(),
            metadata.snapshot(),
        )
    }

    #[test]
    fn offset_mismatch_zero_frames_and_incomplete_eof_retain_only_observed_pcm() {
        let frames = observed_frames();
        let (report, pcm, metadata) = assemble(&frames[..1], 2);
        assert_eq!(report.written_frames, 1);
        assert_eq!(report.incomplete_packets, 1);
        assert_eq!(pcm.len(), 8);
        assert!(metadata.is_empty());
        assert!(report.failure.as_ref().unwrap().contains("incomplete"));
        let (report, pcm, metadata) = assemble(&frames[1..2], 2);
        assert!(report.failure.as_ref().unwrap().contains("offset"));
        assert!(pcm.is_empty() && metadata.is_empty());
        let mut wrong = frames[1];
        wrong.packet_frame_offset = 0;
        let (report, pcm, metadata) = assemble(&[frames[0], wrong], 2);
        assert!(report.failure.as_ref().unwrap().contains("offset"));
        assert_eq!(report.incomplete_packets, 1);
        assert_eq!(pcm.len(), 8);
        assert!(metadata.is_empty());
        let mut zero = frames[0];
        zero.packet_frames = 0;
        let (report, pcm, metadata) = assemble(&[zero], 2);
        assert!(report.failure.is_some());
        assert!(pcm.is_empty() && metadata.is_empty());
    }

    #[test]
    fn rejects_every_mid_packet_identity_change_and_sequence_wrap_or_reorder() {
        let frames = observed_frames();
        for field in 0..5 {
            let mut wrong = frames[1];
            match field {
                0 => wrong.packet_sequence += 1,
                1 => wrong.packet_frames += 1,
                2 => wrong.flags ^= 1,
                3 => wrong.device_position_frames += 1,
                _ => wrong.qpc_100ns += 1,
            }
            let (report, pcm, metadata) = assemble(&[frames[0], wrong], 2);
            assert!(
                report.failure.is_some(),
                "identity field {field} escaped validation"
            );
            assert_eq!(report.incomplete_packets, 1);
            assert_eq!(report.complete_packets, 0);
            assert_eq!(pcm.len(), 8);
            assert!(metadata.is_empty());
        }
        for sequence in [0, 1] {
            let mut reordered = frames[2];
            reordered.packet_sequence = sequence;
            let (report, pcm, _) = assemble(&[frames[0], frames[1], reordered], 2);
            assert!(report.failure.as_ref().unwrap().contains("sequence"));
            assert_eq!(report.complete_packets, 1);
            assert_eq!(pcm.len(), 16);
        }
        let mut end = frames[0];
        end.packet_frames = 1;
        end.packet_sequence = u64::MAX;
        let mut wrapped = end;
        wrapped.packet_sequence = 0;
        let (report, pcm, _) = assemble(&[end, wrapped], 2);
        assert!(report.failure.as_ref().unwrap().contains("sequence"));
        assert_eq!(report.complete_packets, 1);
        assert_eq!(report.missing_packets, u64::MAX - 1);
        assert_eq!(pcm.len(), 8);
    }

    #[test]
    fn mono_ignores_raw_second_slot_and_nonfinite_payload_is_not_sanitized() {
        let mut frame = observed_frames()[0];
        frame.packet_frames = 1;
        frame.samples = [f32::from_bits(0x7fa0_1234), f32::NEG_INFINITY];
        let (report, pcm, _) = assemble(&[frame], 1);
        assert_eq!(report.nonfinite_samples, 1);
        assert_eq!(pcm, 0x7fa0_1234u32.to_le_bytes());
        assert!(report.failure.is_none());
    }

    #[test]
    fn cancellation_during_final_flush_is_retained_in_finish_report() {
        use std::sync::mpsc;
        struct FlushGate {
            entered: Option<mpsc::SyncSender<()>>,
            release: mpsc::Receiver<()>,
        }
        impl Write for FlushGate {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                if let Some(entered) = self.entered.take() {
                    entered.send(()).unwrap();
                    self.release.recv().unwrap();
                }
                Ok(())
            }
        }
        let (mut native, reader) = stream(1, 1);
        native.close();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let mut sink = EvidenceSink::from_sinks(
            reader,
            48_000,
            1,
            FlushGate {
                entered: Some(entered_tx),
                release: release_rx,
            },
            Bytes::default(),
            limits(),
        )
        .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        sink.cancel();
        release_tx.send(()).unwrap();
        let report = sink.finish().unwrap();
        assert!(
            report.cancelled,
            "cancellation during final IO must not disappear"
        );
        assert!(report.failure.is_none());
    }

    #[test]
    fn exact_caps_allow_complete_files_and_leave_unobserved_trailing_loss_to_native_summary() {
        use std::{fs, sync::atomic::AtomicU64};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "moiren-sink-exact-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pcm = root.join("samples.f32le");
        let metadata = root.join("packets.jsonl");
        let (mut native, reader) = stream(1, 2);
        let raw = [0x8000_0000u32.to_le_bytes(), 0x7fc0_1234u32.to_le_bytes()].concat();
        native.record_packet(&raw, packet(1, 0)).unwrap();
        assert_eq!(native.record_packet(&[0; 16], packet(2, 1)).unwrap(), 0);
        assert_eq!(native.snapshot().dropped_frames, 2);
        native.close();
        let mut sink = EvidenceSink::create(
            reader,
            48_000,
            2,
            &pcm,
            &metadata,
            SinkLimits {
                max_frames: 1,
                max_pcm_bytes: 8,
                max_metadata_bytes: 1024,
                max_packets: 1,
            },
        )
        .unwrap();
        let report = sink.finish().unwrap();
        assert_eq!(fs::read(&pcm).unwrap(), raw);
        assert_eq!(
            fs::read(&metadata)
                .unwrap()
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count(),
            1
        );
        assert_eq!(report.written_frames, 1);
        assert_eq!(report.complete_packets, 1);
        assert_eq!(
            report.missing_packets, 0,
            "only the native summary sees a trailing drop"
        );
        assert!(report.failure.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_record_and_checked_budgets_have_inclusive_limits_and_visible_saturation() {
        let mut record = BoundedRecord {
            bytes: [0; MAX_RECORD_BYTES],
            len: 0,
        };
        record.write_all(&[b'x'; MAX_RECORD_BYTES]).unwrap();
        assert!(record.write_all(b"\n").is_err());
        assert_eq!(record.len, MAX_RECORD_BYTES);
        assert!(ensure_budget(5, 5, 10, "test").is_ok());
        assert!(ensure_budget(5, 6, 10, "test").is_err());
        assert!(ensure_budget(u64::MAX, 1, u64::MAX, "test").is_err());
        let mut counter = u64::MAX - 1;
        let mut saturated = false;
        add(&mut counter, 1, &mut saturated);
        assert_eq!(counter, u64::MAX);
        assert!(!saturated);
        add(&mut counter, 1, &mut saturated);
        assert_eq!(counter, u64::MAX);
        assert!(saturated);
    }

    #[test]
    fn defers_packet_flush_until_finish_and_invalidates_completions_on_flush_failure() {
        #[derive(Clone, Default)]
        struct CountFlush(Arc<std::sync::atomic::AtomicU64>);
        impl Write for CountFlush {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
        let pcm = CountFlush::default();
        let metadata = CountFlush::default();
        let mut output = PacketWriter::new(pcm.clone(), metadata.clone(), 48_000, 2, limits());
        for frame in observed_frames() {
            output.accept(frame).unwrap();
        }
        assert_eq!(
            pcm.0.load(Ordering::Relaxed),
            0,
            "do not turn buffering into a per-packet syscall"
        );
        assert_eq!(metadata.0.load(Ordering::Relaxed), 0);
        assert!(output.complete(false, None).failure.is_none());
        assert_eq!(pcm.0.load(Ordering::Relaxed), 1);
        assert_eq!(metadata.0.load(Ordering::Relaxed), 1);

        let bytes = Bytes::default();
        let pcm = std::io::BufWriter::with_capacity(
            64 * 1024,
            FaultSink {
                bytes: bytes.clone(),
                remaining: 3,
                fail_flush: false,
            },
        );
        let mut output = PacketWriter::new(pcm, Bytes::default(), 48_000, 2, limits());
        for frame in observed_frames() {
            output.accept(frame).unwrap();
        }
        assert!(
            bytes.snapshot().is_empty(),
            "fixed buffering keeps tiny writes in memory until final IO"
        );
        let report = output.complete(false, None);
        assert_eq!(
            report.written_pcm_bytes, 32,
            "buffered counts represent accepted bytes, not durable bytes"
        );
        assert_eq!(bytes.snapshot().len(), 3);
        assert_eq!(
            report.complete_packets, 0,
            "failed final flush invalidates provisional packet completion"
        );
        assert_eq!(report.incomplete_packets, 2);
        assert!(report.failure.as_ref().unwrap().contains("final PCM flush"));
    }
}
