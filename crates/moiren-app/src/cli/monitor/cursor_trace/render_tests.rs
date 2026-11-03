use super::*;
use moiren_engine::audio_config::SampleFormat;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Default)]
enum Failure {
    #[default]
    None,
    Write,
    PartialWrite,
    Flush,
    Sync,
}

#[derive(Clone, Default)]
struct TestSink {
    bytes: Arc<Mutex<Vec<u8>>>,
    failure: Failure,
}

impl Write for TestSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self.failure {
            Failure::Write => return Err(std::io::Error::other("injected write")),
            Failure::PartialWrite if !self.bytes.lock().unwrap().is_empty() => {
                return Err(std::io::Error::other("injected partial write"));
            }
            Failure::PartialWrite => {
                self.bytes.lock().unwrap().push(bytes[0]);
                return Ok(1);
            }
            _ => {}
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if matches!(self.failure, Failure::Flush) {
            Err(std::io::Error::other("injected flush"))
        } else {
            Ok(())
        }
    }
}

impl ArchiveSink for TestSink {
    fn sync_all(&self) -> std::io::Result<()> {
        if matches!(self.failure, Failure::Sync) {
            Err(std::io::Error::other("injected sync"))
        } else {
            Ok(())
        }
    }
}

fn format() -> RenderEvidenceFormat {
    RenderEvidenceFormat {
        sample_rate: 48_000,
        sample_format: SampleFormat::Pcm24,
        channels: 2,
        container_bits: 32,
        valid_bits: 24,
    }
}

fn config() -> RenderEvidenceConfig {
    RenderEvidenceConfig {
        capacity_frames: 8192,
        max_packet_frames: 8192,
    }
}

fn frame(offset: u32) -> RenderEvidenceFrame {
    RenderEvidenceFrame {
        bytes: [
            0x91, 0x82, 0x73, 0x64, 0x55, 0x46, 0x37, 0x28, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
        packet_sequence: 1,
        packet_frame_offset: offset,
        packet_frames: 3,
        release_hresult: 1,
    }
}

fn summary() -> RenderEvidenceSummary {
    RenderEvidenceSummary {
        format: format(),
        config: config(),
        attempts: 1,
        accepted_frames: 3,
        rejected_frames: 0,
        stored_frames: 3,
        dropped_frames: 0,
        overflow_packets: 0,
        reader_abandoned_packets: 0,
        invalid_attempts: 0,
        unobserved_packets: 0,
        unobserved_frames: 0,
        pending_abandoned_packets: 0,
        pending_abandoned_frames: 0,
        counters_saturated: false,
        sequence_exhausted: false,
    }
}

fn persistence() -> RenderPersistence<TestSink> {
    RenderPersistence::from_sinks(
        TestSink::default(),
        TestSink::default(),
        format(),
        config(),
        DISK_LIMIT,
    )
    .unwrap()
}

#[test]
fn encoded_frames_preserve_container_padding_and_packet_mapping_across_batches() {
    let mut p = persistence();
    p.persist(&[frame(0)]).unwrap();
    assert_eq!(
        p.mapping.partial_packet().unwrap()["stored_packet_frames"],
        1
    );
    p.persist(&[frame(1), frame(2)]).unwrap();
    assert_eq!(
        *p.samples.bytes.lock().unwrap(),
        frame(0).bytes[..8].repeat(3)
    );
    let line: serde_json::Value = serde_json::from_slice(&p.packets.bytes.lock().unwrap()).unwrap();
    assert_eq!(line["packet_sequence"], 1);
    assert_eq!(line["packet_frames"], 3);
    assert_eq!(line["release_hresult"], 1);
    assert_eq!(line["stored_before_frames"], 0);
    assert!(line.get("bytes").is_none());
    assert_eq!(p.persisted_frames, 3);
    assert!(p.validate_final(&summary(), true).is_ok());
}

#[test]
fn every_encoded_layout_persists_only_meaningful_frame_bytes() {
    for (sample_format, container_bits, valid_bits) in [
        (SampleFormat::Pcm16, 16, 16),
        (SampleFormat::Pcm24, 24, 24),
        (SampleFormat::Pcm24, 32, 24),
        (SampleFormat::Pcm32, 32, 1),
        (SampleFormat::Pcm32, 32, 32),
        (SampleFormat::Float32, 32, 32),
        (SampleFormat::Float64, 64, 64),
    ] {
        for channels in [1, 2] {
            let format = RenderEvidenceFormat {
                sample_format,
                channels,
                container_bits,
                valid_bits,
                ..format()
            };
            let mut p = RenderPersistence::from_sinks(
                TestSink::default(),
                TestSink::default(),
                format,
                config(),
                DISK_LIMIT,
            )
            .unwrap();
            let count = channels * usize::from(container_bits / 8);
            let mut encoded = frame(0);
            encoded.packet_frames = 1;
            encoded.bytes[..count].fill(0xa5);
            encoded.bytes[count..].fill(0);
            p.persist(&[encoded]).unwrap();
            assert_eq!(*p.samples.bytes.lock().unwrap(), vec![0xa5; count]);
        }
    }
}

#[test]
fn invalid_layouts_and_unbounded_configs_are_refused_before_io() {
    for bad in [
        RenderEvidenceFormat {
            sample_format: SampleFormat::Native,
            ..format()
        },
        RenderEvidenceFormat {
            sample_rate: 0,
            ..format()
        },
        RenderEvidenceFormat {
            channels: 0,
            ..format()
        },
        RenderEvidenceFormat {
            channels: 3,
            ..format()
        },
        RenderEvidenceFormat {
            valid_bits: 25,
            ..format()
        },
        RenderEvidenceFormat {
            container_bits: 16,
            ..format()
        },
        RenderEvidenceFormat {
            sample_format: SampleFormat::Float64,
            container_bits: 64,
            valid_bits: 32,
            ..format()
        },
    ] {
        assert!(
            RenderPersistence::from_sinks(
                TestSink::default(),
                TestSink::default(),
                bad,
                config(),
                DISK_LIMIT
            )
            .is_err()
        );
    }
    for bad in [
        RenderEvidenceConfig {
            capacity_frames: 0,
            ..config()
        },
        RenderEvidenceConfig {
            max_packet_frames: 0,
            ..config()
        },
        RenderEvidenceConfig {
            capacity_frames: 1,
            max_packet_frames: 2,
        },
        RenderEvidenceConfig {
            capacity_frames: 65_537,
            max_packet_frames: 65_537,
        },
        RenderEvidenceConfig {
            capacity_frames: usize::MAX,
            max_packet_frames: 1,
        },
    ] {
        assert!(
            RenderPersistence::from_sinks(
                TestSink::default(),
                TestSink::default(),
                format(),
                bad,
                DISK_LIMIT
            )
            .is_err()
        );
    }
}

#[test]
fn rejected_hresult_is_preserved_and_storage_reconciles_without_quality_pass() {
    let mut p = persistence();
    let rejected: Vec<_> = (0..3)
        .map(|offset| RenderEvidenceFrame {
            release_hresult: i32::MIN,
            ..frame(offset)
        })
        .collect();
    p.persist(&rejected).unwrap();
    let mut final_summary = summary();
    final_summary.accepted_frames = 0;
    final_summary.rejected_frames = 3;
    assert!(p.validate_final(&final_summary, true).is_ok());
    let line: serde_json::Value = serde_json::from_slice(&p.packets.bytes.lock().unwrap()).unwrap();
    assert_eq!(line["release_hresult"], i32::MIN);
    assert_eq!(p.mapping.rejected_frames, 3);
    assert_eq!(p.mapping.accepted_frames, 0);
}

#[test]
fn strict_sequence_offsets_metadata_and_unused_payload_are_transactional() {
    for bad in [
        RenderEvidenceFrame {
            packet_sequence: 0,
            ..frame(0)
        },
        RenderEvidenceFrame {
            packet_sequence: 2,
            ..frame(0)
        },
        RenderEvidenceFrame {
            packet_frame_offset: 1,
            ..frame(0)
        },
        RenderEvidenceFrame {
            packet_frames: 0,
            ..frame(0)
        },
        RenderEvidenceFrame {
            packet_frames: 8193,
            ..frame(0)
        },
        RenderEvidenceFrame {
            bytes: [7; 16],
            ..frame(0)
        },
    ] {
        let mut p = persistence();
        assert!(p.persist(&[bad]).is_err());
        assert_eq!(p.persisted_frames, 0);
        assert!(p.samples.bytes.lock().unwrap().is_empty());
        assert!(p.error.is_some());
    }
    for bad in [
        RenderEvidenceFrame {
            packet_sequence: 2,
            ..frame(1)
        },
        RenderEvidenceFrame {
            packet_frames: 2,
            ..frame(1)
        },
        RenderEvidenceFrame {
            release_hresult: 0,
            ..frame(1)
        },
        frame(2),
    ] {
        let mut p = persistence();
        assert!(p.persist(&[frame(0), bad]).is_err());
        assert!(p.samples.bytes.lock().unwrap().is_empty());
        assert_eq!(p.mapping.stored_frames, 0);
    }
    let mut p = persistence();
    p.persist(&[frame(0)]).unwrap();
    assert!(p.persist(&[frame(2)]).is_err());
    assert_eq!(p.mapping.offset, 1);
    assert_eq!(p.persisted_frames, 1);
}

#[test]
fn packet_receipts_use_stored_offsets_and_do_not_reindex_rejected_packets() {
    let mut p = persistence();
    p.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    p.persist(&[RenderEvidenceFrame {
        packet_sequence: 2,
        packet_frames: 1,
        release_hresult: -1,
        ..frame(0)
    }])
    .unwrap();
    let text = String::from_utf8(p.packets.bytes.lock().unwrap().clone()).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["stored_before_frames"], 3);
    assert_eq!(rows[1]["release_hresult"], -1);
    let final_summary = RenderEvidenceSummary {
        attempts: 2,
        rejected_frames: 1,
        stored_frames: 4,
        ..summary()
    };
    assert!(p.validate_final(&final_summary, true).is_ok());
}

#[test]
fn all_io_failures_keep_committed_frontiers_and_refuse_retry() {
    for failure in [
        Failure::Write,
        Failure::PartialWrite,
        Failure::Flush,
        Failure::Sync,
    ] {
        for fail_samples in [false, true] {
            let broken = TestSink {
                failure,
                ..TestSink::default()
            };
            let good = TestSink::default();
            let (samples, packets) = if fail_samples {
                (broken, good)
            } else {
                (good, broken)
            };
            let mut p =
                RenderPersistence::from_sinks(samples, packets, format(), config(), DISK_LIMIT)
                    .unwrap();
            assert!(p.persist(&[frame(0), frame(1), frame(2)]).is_err());
            let saved = (
                p.samples.bytes.lock().unwrap().len(),
                p.packets.bytes.lock().unwrap().len(),
            );
            let original_error = p.error.clone();
            assert_eq!(p.persisted_frames, 0);
            assert_eq!(p.persisted_packets, 0);
            assert_eq!(p.mapping.stored_frames, 0);
            assert!(p.persist(&[frame(0), frame(1), frame(2)]).is_err());
            assert_eq!(p.error, original_error);
            assert_eq!(
                saved,
                (
                    p.samples.bytes.lock().unwrap().len(),
                    p.packets.bytes.lock().unwrap().len()
                )
            );
        }
    }
}

#[test]
fn failed_second_batch_preserves_the_durable_prefix_and_original_error() {
    let mut p = persistence();
    p.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    let prefix_bytes = p.persisted_bytes;
    let sample_prefix = p.samples.bytes.lock().unwrap().clone();
    let packet_prefix = p.packets.bytes.lock().unwrap().clone();
    p.samples.failure = Failure::Write;
    let next = RenderEvidenceFrame {
        packet_sequence: 2,
        packet_frames: 1,
        ..frame(0)
    };
    assert!(p.persist(&[next]).is_err());
    assert_eq!(p.persisted_frames, 3);
    assert_eq!(p.persisted_packets, 1);
    assert_eq!(p.persisted_bytes, prefix_bytes);
    assert_eq!(p.mapping.complete_packets, 1);
    assert_eq!(*p.samples.bytes.lock().unwrap(), sample_prefix);
    assert_eq!(*p.packets.bytes.lock().unwrap(), packet_prefix);
    let error = p.error.clone();
    p.samples.failure = Failure::None;
    assert!(p.persist(&[next]).is_err());
    assert_eq!(p.error, error);
    assert_eq!(*p.samples.bytes.lock().unwrap(), sample_prefix);
}

#[test]
fn packet_counter_overflow_is_refused_before_io_and_is_sticky() {
    let mut p = persistence();
    p.persisted_packets = u64::MAX;
    assert!(p.persist(&[frame(0)]).is_err());
    assert!(p.samples.bytes.lock().unwrap().is_empty());
    assert!(p.packets.bytes.lock().unwrap().is_empty());
    assert_eq!(p.mapping.stored_frames, 0);
    assert!(p.error.is_some());
}

#[test]
fn disk_cap_counts_packet_metadata_and_refuses_before_any_bytes_change() {
    let mut p = persistence();
    p.limit = 24;
    assert!(p.persist(&[frame(0), frame(1), frame(2)]).is_err());
    assert!(p.samples.bytes.lock().unwrap().is_empty());
    assert!(p.packets.bytes.lock().unwrap().is_empty());
    assert_eq!(p.persisted_bytes, 0);
    let mut p = persistence();
    p.persisted_bytes = u64::MAX;
    assert!(p.persist(&[frame(0)]).is_err());
}

#[test]
fn exact_disk_boundary_accepts_bytes_and_reserves_both_owner_receipts() {
    let mut reference = persistence();
    reference.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    let mut p = persistence();
    p.limit = reference.persisted_bytes;
    p.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    let next = RenderEvidenceFrame {
        packet_sequence: 2,
        packet_frames: 1,
        ..frame(0)
    };
    assert!(p.persist(&[next]).is_err());
    assert_eq!(p.persisted_bytes, p.limit);
    let directory = TemporaryDirectory::new();
    let a = archive(&directory.0, vec![]);
    let stream_bytes = fs::metadata(a.directory.join("stream.json")).unwrap().len();
    assert_eq!(
        a.persistence.limit + stream_bytes + RECEIPT_LIMIT,
        DISK_LIMIT
    );
}

#[test]
fn oversized_batches_are_refused_and_partial_packets_cannot_close() {
    let mut p = persistence();
    assert!(p.persist(&vec![frame(0); DRAIN_LIMIT + 1]).is_err());
    let mut p = persistence();
    p.persist(&[frame(0)]).unwrap();
    assert!(p.validate_final(&summary(), true).is_err());
    assert_eq!(p.mapping.partial_packet().unwrap()["packet_frames"], 3);
}

#[test]
fn final_summary_requires_exact_closure_reconciliation_and_no_evidence_loss() {
    let mut p = persistence();
    p.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    assert!(p.validate_final(&summary(), false).is_err());
    let changes: &[fn(&mut RenderEvidenceSummary)] = &[
        |s| s.attempts = 0,
        |s| s.attempts = 2,
        |s| s.accepted_frames = 4,
        |s| s.rejected_frames = 1,
        |s| s.stored_frames = 2,
        |s| s.dropped_frames = 1,
        |s| s.overflow_packets = 1,
        |s| s.reader_abandoned_packets = 1,
        |s| s.invalid_attempts = 1,
        |s| s.unobserved_packets = 1,
        |s| s.unobserved_frames = 1,
        |s| s.pending_abandoned_packets = 1,
        |s| s.pending_abandoned_frames = 1,
        |s| s.counters_saturated = true,
        |s| s.sequence_exhausted = true,
        |s| s.format.sample_rate = 44_100,
        |s| s.config.capacity_frames += 1,
        |s| s.accepted_frames = u64::MAX,
    ];
    for change in changes {
        let mut final_summary = summary();
        change(&mut final_summary);
        assert!(
            p.validate_final(&final_summary, true).is_err(),
            "{final_summary:?}"
        );
    }
}

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moiren-render-archive-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct TestReader {
    frames: VecDeque<RenderEvidenceFrame>,
    summary: Option<RenderEvidenceSummary>,
    closed: bool,
    endless: bool,
    pops: usize,
}
impl EvidenceReader for TestReader {
    fn pop(&mut self) -> Option<RenderEvidenceFrame> {
        self.pops += 1;
        if self.endless {
            Some(RenderEvidenceFrame {
                packet_sequence: self.pops as u64,
                packet_frames: 1,
                ..frame(0)
            })
        } else {
            self.frames.pop_front()
        }
    }
    fn is_finished(&self) -> bool {
        self.closed && self.frames.is_empty() && !self.endless
    }
    fn final_summary(&self) -> Option<RenderEvidenceSummary> {
        self.summary
    }
}

fn archive(directory: &Path, frames: Vec<RenderEvidenceFrame>) -> Archive<TestReader> {
    let (directory, persistence) = prepare(directory, &serde_json::json!({ "owner_generation": 7, "node": 2, "kind": "render", "format": format(), "config": config() }), format(), config()).unwrap();
    Archive {
        directory,
        persistence,
        reader: TestReader {
            frames: frames.into(),
            summary: Some(summary()),
            closed: true,
            endless: false,
            pops: 0,
        },
        pending: Vec::with_capacity(DRAIN_LIMIT),
        last_flush: Instant::now(),
        finalized: false,
        complete: false,
        finish_error: None,
    }
}

fn receipt(directory: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(directory.join("native-render/final.json")).unwrap()).unwrap()
}

#[test]
fn create_new_layout_never_overwrites_existing_owner_evidence() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    assert!(!a.writer_finished());
    a.finish().unwrap();
    assert!(a.writer_finished());
    assert!(a.persistence_complete());
    let before = fs::read(directory.0.join("native-render/samples.bin")).unwrap();
    assert!(prepare(&directory.0, &serde_json::json!({}), format(), config()).is_err());
    assert_eq!(
        fs::read(directory.0.join("native-render/samples.bin")).unwrap(),
        before
    );
    let saved = receipt(&directory.0);
    assert_eq!(saved["persistence_complete"], true);
    assert_eq!(saved["qualified"], false);
    let stream: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.0.join("native-render/stream.json")).unwrap())
            .unwrap();
    assert_eq!(stream["origin"]["owner_generation"], 7);
    assert_eq!(stream["frame_bytes"], 8);
    a.finish().unwrap();
}

#[test]
fn closure_requires_join_summary_and_never_drains_a_live_producer() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    a.reader.summary = None;
    assert!(!a.writer_finished());
    assert!(a.finish().is_err());
    assert_eq!(a.reader.pops, 0);
    assert_eq!(receipt(&directory.0)["summary"], serde_json::Value::Null);
    assert!(!a.persistence_complete());
    assert!(a.finish().is_err());
}

#[test]
fn storage_can_be_complete_with_rejected_submit_but_finish_explicitly_fails() {
    let directory = TemporaryDirectory::new();
    let frames = (0..3)
        .map(|offset| RenderEvidenceFrame {
            release_hresult: -42,
            ..frame(offset)
        })
        .collect();
    let mut a = archive(&directory.0, frames);
    a.reader.summary = Some(RenderEvidenceSummary {
        accepted_frames: 0,
        rejected_frames: 3,
        ..summary()
    });
    assert!(a.finish().unwrap_err().to_string().contains("rejected"));
    assert!(a.persistence_complete());
    assert!(a.finish().is_err());
    let saved = receipt(&directory.0);
    assert_eq!(saved["persistence_complete"], true);
    assert_eq!(saved["native_submit_failed"], true);
    assert_eq!(saved["qualified"], false);
}

#[test]
fn failed_payload_remains_pending_and_failed_receipt_does_not_clear_it() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    a.persistence.limit = 1;
    assert!(a.finish().is_err());
    assert_eq!(a.pending.len(), 3);
    assert_eq!(a.pending[0].bytes, frame(0).bytes);
    let error = a.persistence.error.clone();
    let saved = receipt(&directory.0);
    assert_eq!(saved["pending_frames"], 3);
    assert_eq!(saved["persistence_complete"], false);
    assert!(saved["error"].is_string());
    a.poll();
    assert_eq!(a.pending.len(), 3);
    assert_eq!(a.persistence.error, error);
}

#[test]
fn final_drain_crosses_batch_boundaries_but_is_finite_for_a_broken_reader() {
    let directory = TemporaryDirectory::new();
    let frames: Vec<_> = (0..5000)
        .map(|offset| RenderEvidenceFrame {
            packet_frames: 5000,
            ..frame(offset)
        })
        .collect();
    let mut a = archive(&directory.0, frames);
    a.reader.summary = Some(RenderEvidenceSummary {
        accepted_frames: 5000,
        stored_frames: 5000,
        ..summary()
    });
    a.finish().unwrap();
    assert_eq!(a.persistence.persisted_frames, 5000);
    assert_eq!(a.persistence.persisted_packets, 1);
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![]);
    a.reader.endless = true;
    assert!(a.finish().is_err());
    assert_eq!(a.reader.pops, config().capacity_frames);
    assert_eq!(
        a.persistence.persisted_frames,
        config().capacity_frames as u64
    );
    assert!(a.pending.len() <= DRAIN_LIMIT);
}

#[test]
fn final_drain_completes_an_already_pending_packet_without_exceeding_capacity() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    assert_eq!(a.drain(1), 1);
    assert_eq!(a.pending.len(), 1);
    a.finish().unwrap();
    assert_eq!(a.persistence.persisted_frames, 3);
    assert_eq!(a.persistence.persisted_packets, 1);
    assert!(a.pending.is_empty());
    assert!(a.persistence_complete());
}

#[test]
fn polling_and_pending_are_bounded_and_do_not_claim_unjoined_completion() {
    let directory = TemporaryDirectory::new();
    let frames = (0..8192)
        .map(|offset| RenderEvidenceFrame {
            packet_frames: 8192,
            ..frame(offset)
        })
        .collect();
    let mut a = archive(&directory.0, frames);
    a.reader.summary = None;
    a.poll();
    assert_eq!(a.reader.pops, DRAIN_LIMIT);
    assert_eq!(a.persistence.persisted_frames, DRAIN_LIMIT as u64);
    assert!(a.pending.len() <= DRAIN_LIMIT);
    assert!(!a.writer_finished());
}

#[test]
fn partial_packet_and_final_receipt_collision_are_reported_without_overwrite() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0)]);
    assert!(a.finish().is_err());
    assert_eq!(
        receipt(&directory.0)["partial_packet"]["stored_packet_frames"],
        1
    );
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    let path = directory.0.join("native-render/final.json");
    fs::write(&path, b"original receipt").unwrap();
    assert!(a.finish().is_err());
    assert_eq!(fs::read(path).unwrap(), b"original receipt");
    assert!(!a.persistence_complete());
    assert!(a.finish().is_err());
}
