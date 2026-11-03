use super::*;
use moiren_engine::audio_config::SampleFormat;
use moiren_windows_audio::render_evidence::{
    RENDER_EVENT_CAPACITY, RenderEvent, RenderEventKind, RenderEventSample, RenderTimelineSummary,
    render_timeline_storage_bytes,
};
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
        timeline: RenderTimelineSummary {
            capacity_events: RENDER_EVENT_CAPACITY,
            ..Default::default()
        },
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
    events: VecDeque<RenderEvent>,
    events_closed: bool,
    endless_events: bool,
    event_pops: usize,
}
impl EvidenceReader for TestReader {
    fn pop_event(&mut self) -> Option<RenderEvent> {
        self.event_pops += 1;
        if self.endless_events {
            Some(event(self.event_pops as u64))
        } else {
            self.events.pop_front()
        }
    }
    fn events_finished(&self) -> bool {
        self.events_closed && self.events.is_empty() && !self.endless_events
    }
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
    let (directory, persistence, timeline) = prepare(directory, &serde_json::json!({ "owner_generation": 7, "node": 2, "kind": "render", "format": format(), "config": config() }), format(), config()).unwrap();
    let mut accepted = 0;
    let events = frames
        .iter()
        .filter(|frame| frame.packet_frame_offset == 0)
        .enumerate()
        .map(|(index, frame)| {
            if frame.release_hresult >= 0 {
                accepted += u64::from(frame.packet_frames);
            }
            write_event(
                index as u64 + 1,
                frame.packet_sequence,
                frame.packet_frames,
                frame.release_hresult,
                accepted,
            )
        })
        .collect();
    let mut archive = Archive {
        directory,
        persistence,
        timeline,
        pending_events: Vec::with_capacity(DRAIN_LIMIT),
        reader: TestReader {
            frames: frames.into(),
            summary: Some(summary()),
            closed: true,
            endless: false,
            pops: 0,
            events: VecDeque::new(),
            events_closed: true,
            endless_events: false,
            event_pops: 0,
        },
        pending: Vec::with_capacity(DRAIN_LIMIT),
        last_flush: Instant::now(),
        finalized: false,
        complete: false,
        finish_error: None,
    };
    set_events(&mut archive, events);
    archive
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
        timeline: a.reader.summary.unwrap().timeline,
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
        timeline: a.reader.summary.unwrap().timeline,
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

fn event(sequence: u64) -> RenderEvent {
    RenderEvent {
        sequence,
        packet_sequence: None,
        accepted_frames: 0,
        sample: RenderEventSample {
            kind: RenderEventKind::Wait,
            qpc_before_100ns: Some(sequence * 100),
            qpc_after_100ns: Some(sequence * 100 + 50),
            ready: Some(false),
            ..Default::default()
        },
    }
}

fn set_events(a: &mut Archive<TestReader>, events: Vec<RenderEvent>) {
    let count = events.len() as u64;
    a.reader.events = events.into();
    a.reader.summary.as_mut().unwrap().timeline = RenderTimelineSummary {
        capacity_events: RENDER_EVENT_CAPACITY,
        events: count,
        stored_events: count,
        ..Default::default()
    };
}

fn timeline_rows(directory: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(directory.join("native-render/timeline.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn timeline_preserves_unknown_regressed_clocks_and_packet_metadata_without_repair() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    let events = vec![
        RenderEvent {
            sample: RenderEventSample {
                kind: RenderEventKind::Start,
                qpc_before_100ns: None,
                qpc_after_100ns: None,
                ..Default::default()
            },
            ..event(1)
        },
        RenderEvent {
            packet_sequence: Some(1),
            accepted_frames: 3,
            sample: RenderEventSample {
                kind: RenderEventKind::Write,
                qpc_before_100ns: Some(500),
                qpc_after_100ns: Some(490),
                padding_frames: Some(0),
                requested_frames: 64,
                submitted_frames: 3,
                release_hresult: Some(1),
                ..Default::default()
            },
            ..event(2)
        },
        RenderEvent {
            accepted_frames: 3,
            sample: RenderEventSample {
                kind: RenderEventKind::Stop,
                failed: true,
                ..event(3).sample
            },
            ..event(3)
        },
    ];
    set_events(&mut a, events.clone());
    let tally = &mut a.reader.summary.as_mut().unwrap().timeline;
    tally.unavailable_clocks = 1;
    tally.regressed_clocks = 2;
    a.finish().unwrap();
    assert_eq!(
        timeline_rows(&directory.0),
        events
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect::<Vec<_>>()
    );
    let saved = receipt(&directory.0);
    assert_eq!(saved["timeline"]["persisted_events"], 3);
    assert_eq!(saved["timeline"]["last_sequence"], 3);
    assert_eq!(saved["timeline"]["persistence_complete"], true);
    assert_eq!(saved["summary"]["timeline"]["unavailable_clocks"], 1);
    assert_eq!(saved["summary"]["timeline"]["regressed_clocks"], 2);
    assert_eq!(saved["audio_persistence_complete"], true);
    assert_eq!(saved["qualified"], false);
    assert_eq!(
        fs::read(directory.0.join("native-render/samples.bin")).unwrap(),
        frame(0).bytes[..8].repeat(3)
    );
}

#[test]
fn timeline_overflow_preserves_later_sequences_and_explicitly_refuses_completion() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    set_events(
        &mut a,
        vec![
            write_event(1, 1, 3, 1, 3),
            RenderEvent {
                accepted_frames: 3,
                ..event(3)
            },
        ],
    );
    let tally = &mut a.reader.summary.as_mut().unwrap().timeline;
    tally.events = 3;
    tally.dropped_events = 1;
    assert!(a.finish().is_err());
    let rows = timeline_rows(&directory.0);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["sequence"], 3);
    let saved = receipt(&directory.0);
    assert_eq!(saved["timeline"]["missing_events"], 1);
    assert_eq!(saved["timeline"]["persistence_complete"], false);
    assert!(saved["timeline"]["error"].is_string());
    assert_eq!(saved["audio_persistence_complete"], true);
    assert_eq!(saved["persistence_complete"], false);
    assert!(!a.persistence_complete());
}

#[test]
fn timeline_final_tallies_and_closure_must_match_independently_of_audio() {
    let changes: &[fn(&mut RenderTimelineSummary)] = &[
        |s| s.capacity_events = 0,
        |s| s.events += 1,
        |s| s.stored_events += 1,
        |s| s.dropped_events = 1,
        |s| s.reader_abandoned_events = 1,
        |s| s.unavailable_clocks = 1,
        |s| s.regressed_clocks = 1,
        |s| s.saturated = true,
    ];
    for change in changes {
        let directory = TemporaryDirectory::new();
        let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
        set_events(&mut a, vec![write_event(1, 1, 3, 1, 3)]);
        change(&mut a.reader.summary.as_mut().unwrap().timeline);
        assert!(a.finish().is_err());
        let saved = receipt(&directory.0);
        assert_eq!(saved["audio_persistence_complete"], true);
        assert_eq!(saved["timeline"]["persistence_complete"], false);
    }
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    set_events(&mut a, vec![write_event(1, 1, 3, 1, 3)]);
    a.reader.events_closed = false;
    assert!(a.finish().is_err());
    assert!(!a.writer_finished());
    assert_eq!(receipt(&directory.0)["audio_persistence_complete"], true);
}

#[test]
fn timeline_requires_join_and_final_drain_is_capacity_bounded() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    set_events(&mut a, vec![write_event(1, 1, 3, 1, 3)]);
    a.reader.summary = None;
    assert!(a.finish().is_err());
    assert_eq!(a.reader.event_pops, 0);
    assert!(timeline_rows(&directory.0).is_empty());

    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    a.reader.endless_events = true;
    assert!(a.finish().is_err());
    assert_eq!(a.reader.event_pops, RENDER_EVENT_CAPACITY);
    assert_eq!(timeline_rows(&directory.0).len(), RENDER_EVENT_CAPACITY);
    assert!(a.pending_events.len() <= DRAIN_LIMIT);
    assert_eq!(receipt(&directory.0)["audio_persistence_complete"], true);
}

#[test]
fn timeline_poll_is_bounded_and_audio_failure_does_not_skip_timeline() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    a.reader.endless_events = true;
    a.reader.summary = None;
    a.poll();
    assert_eq!(a.reader.event_pops, DRAIN_LIMIT);
    assert_eq!(timeline_rows(&directory.0).len(), DRAIN_LIMIT);
    assert!(!a.writer_finished());

    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    set_events(&mut a, vec![write_event(1, 1, 3, 1, 3)]);
    a.persistence.limit = 1;
    assert!(a.finish().is_err());
    assert_eq!(timeline_rows(&directory.0).len(), 1);
    assert_eq!(
        receipt(&directory.0)["timeline"]["persistence_complete"],
        false
    );
}

#[test]
fn timeline_io_failure_is_sticky_and_keeps_original_audio_evidence() {
    for failure in [
        Failure::Write,
        Failure::PartialWrite,
        Failure::Flush,
        Failure::Sync,
    ] {
        let sink = TestSink {
            failure,
            ..Default::default()
        };
        let mut p = TimelinePersistence::from_sink(Ok(sink), DISK_LIMIT);
        assert!(p.persist(&[event(1)]).is_err());
        let bytes = p.sink.as_ref().unwrap().bytes.lock().unwrap().clone();
        let error = p.error.clone();
        assert_eq!(p.persisted_events, 0);
        assert!(p.persist(&[event(1)]).is_err());
        assert_eq!(*p.sink.as_ref().unwrap().bytes.lock().unwrap(), bytes);
        assert_eq!(p.error, error);
    }
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    set_events(&mut a, vec![event(1)]);
    a.timeline.sink = Some(File::open(directory.0.join("native-render/timeline.jsonl")).unwrap());
    assert!(a.finish().is_err());
    assert_eq!(a.persistence.persisted_frames, 3);
    assert_eq!(a.pending_events, vec![event(1)]);
    let saved = receipt(&directory.0);
    assert_eq!(saved["audio_persistence_complete"], true);
    assert_eq!(saved["timeline"]["pending_events"], 1);
    assert_eq!(saved["persistence_complete"], false);
    assert!(saved["timeline"]["error"].is_string());
    assert!(a.finish().is_err());
}

#[test]
fn timeline_initialization_failure_and_disk_cap_are_explicit_and_independent() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    a.timeline =
        TimelinePersistence::from_sink(Err(std::io::Error::other("injected create")), DISK_LIMIT);
    assert!(a.finish().is_err());
    let saved = receipt(&directory.0);
    assert_eq!(saved["audio_persistence_complete"], true);
    assert!(
        saved["timeline"]["error"]
            .as_str()
            .unwrap()
            .contains("injected create")
    );
    let mut p = TimelinePersistence::from_sink(Ok(TestSink::default()), 1);
    assert!(p.persist(&[event(1)]).is_err());
    assert!(p.sink.as_ref().unwrap().bytes.lock().unwrap().is_empty());
    assert_eq!(p.persisted_events, 0);
}

#[test]
fn timeline_storage_is_included_in_the_existing_preparation_budget() {
    let bytes = format().channels * usize::from(format().container_bits / 8);
    let capacity_frames = (8 * 1024 * 1024 - bytes) / size_of::<RenderEvidenceFrame>();
    let config = RenderEvidenceConfig {
        capacity_frames,
        max_packet_frames: 1,
    };
    assert!(capacity_frames * size_of::<RenderEvidenceFrame>() + bytes <= 8 * 1024 * 1024);
    assert!(
        capacity_frames * size_of::<RenderEvidenceFrame>()
            + bytes
            + render_timeline_storage_bytes()
            > 8 * 1024 * 1024
    );
    assert!(frame_bytes(format(), config).is_err());
}

#[test]
fn timeline_sequence_validation_is_transactional_across_batches_and_sticky() {
    for bad in [
        vec![event(0)],
        vec![event(2), event(1)],
        vec![event(2), event(2)],
    ] {
        let mut p = TimelinePersistence::from_sink(Ok(TestSink::default()), DISK_LIMIT);
        assert!(p.persist(&bad).is_err());
        assert_eq!(p.persisted_events, 0);
        assert!(p.sink.as_ref().unwrap().bytes.lock().unwrap().is_empty());
    }
    let mut p = TimelinePersistence::from_sink(Ok(TestSink::default()), DISK_LIMIT);
    p.persist(&[event(1)]).unwrap();
    let prefix = p.sink.as_ref().unwrap().bytes.lock().unwrap().clone();
    assert!(p.persist(&[event(1)]).is_err());
    let error = p.error.clone();
    assert!(p.persist(&[event(2)]).is_err());
    assert_eq!(p.error, error);
    assert_eq!(p.persisted_events, 1);
    assert_eq!(*p.sink.as_ref().unwrap().bytes.lock().unwrap(), prefix);
}

#[test]
fn timeline_clock_tallies_and_finish_cross_an_already_pending_batch() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    let mut second = RenderEvent {
        accepted_frames: 3,
        ..event(2)
    };
    second.sample.qpc_before_100ns = Some(149);
    second.sample.qpc_after_100ns = None;
    set_events(
        &mut a,
        vec![
            write_event(1, 1, 3, 1, 3),
            second,
            RenderEvent {
                accepted_frames: 3,
                ..event(3)
            },
        ],
    );
    let tally = &mut a.reader.summary.as_mut().unwrap().timeline;
    tally.unavailable_clocks = 1;
    tally.regressed_clocks = 1;
    assert_eq!(a.drain_events(1), 1);
    a.flush_events();
    assert!(!a.writer_finished());
    assert_eq!(a.drain_events(1), 1);
    a.finish().unwrap();
    assert_eq!(timeline_rows(&directory.0).len(), 3);
    assert_eq!(a.timeline.persisted_events, 3);
    assert!(a.pending_events.is_empty());
    assert!(a.writer_finished());
    assert!(a.persistence_complete());
}

#[test]
fn timeline_and_audio_share_one_disk_budget_including_failed_physical_tails() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    let events = vec![write_event(1, 1, 3, 1, 3)];
    let timeline_bytes = serde_json::to_vec(&events[0]).unwrap().len() as u64 + 1;
    let mut audio = persistence();
    audio.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    let shared_limit = audio.persisted_bytes + timeline_bytes;
    a.persistence.limit = shared_limit;
    a.timeline.limit = shared_limit;
    set_events(&mut a, events);
    a.finish().unwrap();
    assert_eq!(
        a.persistence.persisted_bytes + a.timeline.persisted_bytes,
        shared_limit
    );

    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    a.persistence.limit = shared_limit - 1;
    a.timeline.limit = shared_limit - 1;
    set_events(&mut a, vec![write_event(1, 1, 3, 1, 3)]);
    assert!(a.finish().is_err());
    assert_eq!(a.persistence.persisted_frames, 3);
    assert_eq!(a.timeline.persisted_events, 0);
    assert!(timeline_rows(&directory.0).is_empty());

    let mut timeline = TimelinePersistence::from_sink(
        Ok(TestSink {
            failure: Failure::PartialWrite,
            ..Default::default()
        }),
        DISK_LIMIT,
    );
    assert!(timeline.persist(&[event(1)]).is_err());
    assert_eq!(timeline.persisted_bytes, 0);
    assert_eq!(
        timeline.reserved_bytes,
        serde_json::to_vec(&event(1)).unwrap().len() as u64 + 1
    );
    assert_eq!(
        timeline.sink.as_ref().unwrap().bytes.lock().unwrap().len(),
        1
    );
}

#[test]
fn timeline_batch_bound_and_counter_exhaustion_refuse_before_io() {
    let mut p = TimelinePersistence::from_sink(Ok(TestSink::default()), DISK_LIMIT);
    let oversized: Vec<_> = (1..=DRAIN_LIMIT + 1).map(|n| event(n as u64)).collect();
    assert!(p.persist(&oversized).is_err());
    assert!(p.sink.as_ref().unwrap().bytes.lock().unwrap().is_empty());
    let mut p = TimelinePersistence::from_sink(Ok(TestSink::default()), DISK_LIMIT);
    p.persisted_bytes = u64::MAX;
    assert!(p.persist(&[event(1)]).is_err());
    assert!(p.sink.as_ref().unwrap().bytes.lock().unwrap().is_empty());
}

#[test]
fn timeline_failure_does_not_block_joined_owner_audio_finalization() {
    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    set_events(&mut a, vec![event(1), event(2)]);
    a.timeline =
        TimelinePersistence::from_sink(Err(std::io::Error::other("injected create")), DISK_LIMIT);
    a.poll();
    assert_eq!(a.reader.event_pops, 0);
    assert!(!a.reader.events_finished());
    assert!(
        a.writer_finished(),
        "joined owner must still finalize its audio archive"
    );
    assert!(a.finish().is_err());
    let saved = receipt(&directory.0);
    assert_eq!(saved["audio_persistence_complete"], true);
    assert_eq!(saved["timeline"]["writer_closed_and_drained"], false);
    assert_eq!(saved["timeline"]["persistence_complete"], false);

    let directory = TemporaryDirectory::new();
    let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
    a.timeline =
        TimelinePersistence::from_sink(Err(std::io::Error::other("injected create")), DISK_LIMIT);
    a.reader.summary = None;
    a.poll();
    assert!(
        !a.writer_finished(),
        "diagnostic failure cannot substitute for native join"
    );
}

fn write_event(sequence: u64, packet: u64, frames: u32, hr: i32, accepted: u64) -> RenderEvent {
    RenderEvent {
        packet_sequence: Some(packet),
        accepted_frames: accepted,
        sample: RenderEventSample {
            kind: RenderEventKind::Write,
            requested_frames: u64::from(frames),
            submitted_frames: frames,
            release_hresult: Some(hr),
            failed: hr < 0,
            ..event(sequence).sample
        },
        ..event(sequence)
    }
}

#[test]
fn timeline_packet_crosscheck_rejects_contradictions_without_hiding_audio() {
    let valid = write_event(1, 1, 3, 1, 3);
    let changes: &[fn(&mut RenderEvent)] = &[
        |e| e.packet_sequence = Some(2),
        |e| e.sample.submitted_frames = 2,
        |e| e.sample.requested_frames = 2,
        |e| e.sample.release_hresult = Some(-42),
        |e| e.sample.release_hresult = None,
        |e| e.accepted_frames = 4,
        |e| e.accepted_frames = 2,
        |e| e.sample.failed = true,
        |e| e.sample.kind = RenderEventKind::Wait,
        |e| e.packet_sequence = None,
    ];
    for change in changes {
        let directory = TemporaryDirectory::new();
        let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
        let mut bad = valid;
        change(&mut bad);
        set_events(&mut a, vec![bad]);
        assert!(a.finish().is_err(), "contradictory event accepted: {bad:?}");
        let saved = receipt(&directory.0);
        assert_eq!(saved["audio_persistence_complete"], true);
        assert_eq!(saved["timeline"]["persistence_complete"], false);
        assert_eq!(saved["persistence_complete"], false);
        assert!(saved["timeline"]["error"].is_string());
        assert_eq!(
            fs::read(a.directory.join("samples.bin")).unwrap(),
            frame(0).bytes[..8].repeat(3)
        );
        assert_eq!(
            timeline_rows(&directory.0)[0],
            serde_json::to_value(bad).unwrap()
        );
    }
}

#[test]
fn timeline_packet_crosscheck_rejects_missing_duplicate_and_unbacked_frontiers() {
    let valid = write_event(1, 1, 3, 1, 3);
    let mut zero_write = event(1);
    zero_write.accepted_frames = 0;
    zero_write.sample.kind = RenderEventKind::Write;
    zero_write.sample.requested_frames = 3;
    let mut before = event(1);
    before.accepted_frames = 1;
    let after = RenderEvent {
        accepted_frames: 4,
        ..event(2)
    };
    for events in [
        vec![],
        vec![zero_write],
        vec![
            valid,
            RenderEvent {
                sequence: 2,
                ..valid
            },
        ],
        vec![
            before,
            RenderEvent {
                sequence: 2,
                ..valid
            },
        ],
        vec![valid, after],
    ] {
        let directory = TemporaryDirectory::new();
        let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
        set_events(&mut a, events.clone());
        assert!(
            a.finish().is_err(),
            "unbacked timeline accepted: {events:?}"
        );
        let saved = receipt(&directory.0);
        assert_eq!(saved["audio_persistence_complete"], true);
        assert_eq!(saved["timeline"]["persistence_complete"], false);
        assert_eq!(saved["persistence_complete"], false);
        assert!(a.finish().is_err());
    }
}

#[test]
fn timeline_packet_crosscheck_accepts_partial_zero_failed_and_rejected_writes() {
    let directory = TemporaryDirectory::new();
    let mut frames = vec![frame(0), frame(1), frame(2)];
    frames.push(RenderEvidenceFrame {
        packet_sequence: 2,
        packet_frames: 1,
        release_hresult: -42,
        ..frame(0)
    });
    let mut a = archive(&directory.0, frames);
    let mut partial = write_event(2, 1, 3, 1, 3);
    partial.sample.requested_frames = 64;
    let zero = RenderEvent {
        accepted_frames: 3,
        sample: RenderEventSample {
            kind: RenderEventKind::Write,
            requested_frames: 64,
            ..event(3).sample
        },
        ..event(3)
    };
    let failed = RenderEvent {
        accepted_frames: 3,
        sample: RenderEventSample {
            kind: RenderEventKind::Write,
            requested_frames: 64,
            failed: true,
            ..event(4).sample
        },
        ..event(4)
    };
    let mut before = event(1);
    before.accepted_frames = 0;
    let mut after = event(6);
    after.accepted_frames = 3;
    after.sample.kind = RenderEventKind::Stop;
    set_events(
        &mut a,
        vec![
            before,
            partial,
            zero,
            RenderEvent {
                sequence: 4,
                ..failed
            },
            write_event(5, 2, 1, -42, 3),
            after,
        ],
    );
    let timeline = a.reader.summary.unwrap().timeline;
    a.reader.summary = Some(RenderEvidenceSummary {
        attempts: 2,
        accepted_frames: 3,
        rejected_frames: 1,
        stored_frames: 4,
        timeline,
        ..summary()
    });
    assert!(a.finish().unwrap_err().to_string().contains("rejected"));
    let saved = receipt(&directory.0);
    assert_eq!(saved["audio_persistence_complete"], true);
    assert_eq!(saved["timeline"]["persistence_complete"], true);
    assert_eq!(saved["persistence_complete"], true);
    assert!(a.persistence_complete());
}

#[test]
fn timeline_sidecar_parser_bounds_lines_and_refuses_partial_or_invalid_json() {
    let mut line = Vec::new();
    let mut exact = vec![b' '; SIDECAR_LINE_LIMIT - 3];
    exact.extend_from_slice(b"{}\n");
    let mut input = std::io::Cursor::new(exact.clone());
    assert_eq!(
        next_sidecar_line::<serde_json::Value>(&mut input, &mut line).unwrap(),
        Some(serde_json::json!({}))
    );
    assert!(
        next_sidecar_line::<serde_json::Value>(&mut input, &mut line)
            .unwrap()
            .is_none()
    );
    exact.insert(0, b' ');
    for invalid in [
        exact,
        b"{}".to_vec(),
        b"not json\n".to_vec(),
        b"\n".to_vec(),
    ] {
        let mut input = std::io::Cursor::new(invalid);
        assert!(next_sidecar_line::<serde_json::Value>(&mut input, &mut line).is_err());
        assert!(line.len() <= SIDECAR_LINE_LIMIT + 1);
    }
}

#[test]
fn timeline_crosscheck_read_or_parse_failure_still_closes_independent_audio() {
    for name in ["packets.jsonl", "timeline.jsonl"] {
        for truncate in [false, true] {
            let directory = TemporaryDirectory::new();
            let mut a = archive(&directory.0, vec![frame(0), frame(1), frame(2)]);
            a.poll();
            a.flush();
            let path = a.directory.join(name);
            let mut bytes = fs::read(&path).unwrap();
            if truncate {
                bytes.pop();
            } else {
                bytes[0] = b'!';
            }
            fs::write(path, bytes).unwrap();
            assert!(a.finish().is_err());
            let saved = receipt(&directory.0);
            assert_eq!(saved["audio_persistence_complete"], true);
            assert_eq!(saved["timeline"]["persistence_complete"], false);
            assert_eq!(saved["persistence_complete"], false);
            assert!(saved["timeline"]["error"].is_string());
            assert!(a.finish().is_err());
        }
    }
}
