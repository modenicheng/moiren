use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct TestSink {
    bytes: Arc<Mutex<Vec<u8>>>,
    fail_sync: bool,
    fail_write: bool,
}

impl Write for TestSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.fail_write {
            return Err(std::io::Error::other("injected write failure"));
        }
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ArchiveSink for TestSink {
    fn sync_all(&self) -> std::io::Result<()> {
        if self.fail_sync {
            Err(std::io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    }
}

fn persistence() -> PcmPersistence<TestSink> {
    PcmPersistence::from_sinks(TestSink::default(), TestSink::default(), 2, DISK_LIMIT)
}

fn frame(offset: u32) -> NativePcmFrame {
    NativePcmFrame {
        sample_bits: [0x8000_0000, 0x7fc0_1234],
        packet_sequence: 1,
        offered_before_frames: 0,
        accepted_before_frames: 0,
        packet_frames: 3,
        packet_accepted_frames: 2,
        packet_frame_offset: offset,
        flags: 0,
        generation: 1,
        device_position_frames: 100,
        qpc_100ns: 234,
    }
}

fn summary(frames: u64, accepted: u64, packets: u64) -> NativePcmSummary {
    NativePcmSummary {
        capacity_frames: 4096,
        sample_rate: 48000,
        channels: 2,
        packets,
        offered_frames: frames,
        accepted_frames: accepted,
        stored_frames: frames,
        lost_frames: 0,
        overflow_packets: 0,
        reader_abandoned_packets: 0,
        saturated: false,
        writer_finished: true,
    }
}

#[test]
fn pcm_preserves_raw_bits_and_packet_frontiers_across_flushes() {
    let mut p = persistence();
    p.persist(&[frame(0)]).unwrap();
    p.persist(&[frame(1), frame(2)]).unwrap();
    assert_eq!(p.persisted_frames, 3);
    assert_eq!(p.persisted_packets, 1);
    let bytes = p.samples.get_ref().bytes.lock().unwrap().clone();
    assert_eq!(
        bytes,
        [0x8000_0000_u32.to_le_bytes(), 0x7fc0_1234_u32.to_le_bytes()]
            .concat()
            .repeat(3)
    );
    let metadata = p.packets.get_ref().bytes.lock().unwrap().clone();
    let line: serde_json::Value = serde_json::from_slice(&metadata).unwrap();
    assert_eq!(line["stored_before_frames"], 0);
    assert_eq!(line["packet_frames"], 3);
    assert_eq!(line["packet_accepted_frames"], 2);
    assert_eq!(line["device_position_frames"], 100);
    assert!(line.get("sample_bits").is_none());
    assert!(line.get("packet_frame_offset").is_none());
    assert!(p.validate_final(summary(3, 2, 1), true).is_ok());
}

#[test]
fn pcm_accepts_zero_accepted_packet_and_tracks_offered_without_tail_reindexing() {
    let mut p = persistence();
    p.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    let next = NativePcmFrame {
        packet_sequence: 2,
        packet_frames: 1,
        packet_accepted_frames: 0,
        packet_frame_offset: 0,
        offered_before_frames: 3,
        accepted_before_frames: 2,
        generation: 2,
        ..frame(0)
    };
    p.persist(&[next]).unwrap();
    let lines = p.packets.get_ref().bytes.lock().unwrap().clone();
    let rows: Vec<serde_json::Value> = String::from_utf8(lines)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["stored_before_frames"], 3);
    assert_eq!(rows[1]["offered_before_frames"], 3);
    assert_eq!(rows[1]["accepted_before_frames"], 2);
    assert_eq!(rows[1]["packet_accepted_frames"], 0);
    assert!(p.validate_final(summary(4, 2, 2), true).is_ok());
}

#[test]
fn pcm_rejects_packet_gaps_offsets_metadata_changes_and_generation_regression() {
    let mut malformed = vec![
        NativePcmFrame {
            packet_sequence: 2,
            ..frame(0)
        },
        NativePcmFrame {
            packet_frames: 0,
            ..frame(0)
        },
        NativePcmFrame {
            packet_accepted_frames: 4,
            ..frame(0)
        },
        NativePcmFrame {
            offered_before_frames: 1,
            ..frame(0)
        },
        NativePcmFrame {
            accepted_before_frames: 1,
            ..frame(0)
        },
        NativePcmFrame {
            generation: 0,
            ..frame(0)
        },
        NativePcmFrame {
            flags: moiren_windows_audio::stats::SILENT,
            ..frame(0)
        },
    ];
    malformed.push(frame(1));
    for bad in malformed {
        let mut p = persistence();
        assert!(p.persist(&[bad]).is_err());
        assert_eq!(p.persisted_frames, 0);
        assert!(p.persist(&[frame(0)]).is_err(), "refusal is sticky");
    }
    for bad in [
        NativePcmFrame {
            qpc_100ns: 235,
            ..frame(1)
        },
        NativePcmFrame {
            packet_frame_offset: 2,
            ..frame(1)
        },
        NativePcmFrame {
            generation: 2,
            ..frame(1)
        },
    ] {
        let mut p = persistence();
        p.persist(&[frame(0)]).unwrap();
        assert!(p.persist(&[bad]).is_err());
        assert_eq!(p.persisted_frames, 1);
    }
}

#[test]
fn pcm_refuses_partial_packets_open_writer_loss_and_mismatched_final_tallies() {
    let mut partial = persistence();
    partial.persist(&[frame(0), frame(1)]).unwrap();
    assert!(partial.validate_final(summary(3, 2, 1), true).is_err());
    assert!(partial.mapping.partial_packet().is_some());
    let mut p = persistence();
    p.persist(&[frame(0), frame(1), frame(2)]).unwrap();
    for s in [
        NativePcmSummary {
            writer_finished: false,
            ..summary(3, 2, 1)
        },
        NativePcmSummary {
            lost_frames: 1,
            ..summary(3, 2, 1)
        },
        NativePcmSummary {
            overflow_packets: 1,
            ..summary(3, 2, 1)
        },
        NativePcmSummary {
            reader_abandoned_packets: 1,
            ..summary(3, 2, 1)
        },
        NativePcmSummary {
            saturated: true,
            ..summary(3, 2, 1)
        },
        summary(3, 3, 1),
        summary(3, 2, 2),
        summary(4, 2, 1),
    ] {
        assert!(p.validate_final(s, true).is_err());
    }
    assert!(p.validate_final(summary(3, 2, 1), false).is_err());
    assert!(p.validate_final(summary(3, 2, 1), true).is_ok());
}

#[test]
fn pcm_disk_cap_and_write_or_sync_failures_never_count_success() {
    for (samples, packets) in [
        (
            TestSink {
                fail_write: true,
                ..Default::default()
            },
            TestSink::default(),
        ),
        (
            TestSink {
                fail_sync: true,
                ..Default::default()
            },
            TestSink::default(),
        ),
        (
            TestSink::default(),
            TestSink {
                fail_write: true,
                ..Default::default()
            },
        ),
        (
            TestSink::default(),
            TestSink {
                fail_sync: true,
                ..Default::default()
            },
        ),
    ] {
        let mut p = PcmPersistence::from_sinks(samples, packets, 2, DISK_LIMIT);
        assert!(p.persist(&[frame(0)]).is_err());
        assert_eq!(p.persisted_frames, 0);
        assert_eq!(p.persisted_bytes, 0);
        assert_eq!(p.persisted_packets, 0);
        assert!(p.error.is_some());
    }
    let mut p = PcmPersistence::from_sinks(TestSink::default(), TestSink::default(), 2, 8);
    assert!(p.persist(&[frame(0)]).is_err());
    assert_eq!(p.persisted_frames, 0);
    assert!(p.samples.get_ref().bytes.lock().unwrap().is_empty());
    assert!(p.packets.get_ref().bytes.lock().unwrap().is_empty());
}

#[test]
fn pcm_batch_memory_is_bounded_and_mono_padding_is_verified() {
    let mut p = persistence();
    assert!(p.persist(&vec![frame(0); DRAIN_LIMIT + 1]).is_err());
    assert_eq!(p.persisted_frames, 0);
    let mut mono =
        PcmPersistence::from_sinks(TestSink::default(), TestSink::default(), 1, DISK_LIMIT);
    assert!(mono.persist(&[frame(0)]).is_err());
    let mut mono =
        PcmPersistence::from_sinks(TestSink::default(), TestSink::default(), 1, DISK_LIMIT);
    mono.persist(&[NativePcmFrame {
        sample_bits: [1.0_f32.to_bits(), 0],
        ..frame(0)
    }])
    .unwrap();
}

fn native_reader(
    capacity: usize,
) -> (
    moiren_windows_audio::clock_bridge::CaptureIngress,
    moiren_windows_audio::clock_bridge::ClockSource,
    NativePcmReader,
) {
    use moiren_windows_audio::clock_bridge::{
        ClockBridgeConfig, capture_bridge, native_pcm::NativePcmConfig,
    };
    let (ingress, source, observer) = capture_bridge(ClockBridgeConfig {
        native_pcm: Some(NativePcmConfig {
            capacity_frames: capacity,
        }),
        capacity_frames: 16384,
        ..Default::default()
    })
    .unwrap();
    (ingress, source, observer.take_native_pcm_reader().unwrap())
}

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moiren-pcm-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

fn native_packet(ingress: &mut moiren_windows_audio::clock_bridge::CaptureIngress, frames: usize) {
    let bytes = [0x8000_0000_u32.to_le_bytes(), 0x7fc0_1234_u32.to_le_bytes()]
        .concat()
        .repeat(frames);
    ingress
        .push_packet(
            &bytes,
            moiren_windows_audio::clock_bridge::CapturePacket {
                frames,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 123,
            },
        )
        .unwrap();
}

#[test]
fn pcm_real_reader_final_drain_is_complete_and_archive_never_overwrites() {
    let directory = TemporaryDirectory::new();
    let (mut ingress, _source, reader) = native_reader(8192);
    let mut archive = PcmArchive::new(&directory.0, reader).unwrap();
    native_packet(&mut ingress, 5000);
    drop(ingress);
    archive.finish().unwrap();
    assert!(archive.persistence_complete());
    assert_eq!(
        fs::metadata(directory.0.join("native-pcm/samples.f32le"))
            .unwrap()
            .len(),
        5000 * 8
    );
    let final_json: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.0.join("native-pcm/final.json")).unwrap())
            .unwrap();
    assert_eq!(final_json["persistence_complete"], true);
    assert_eq!(final_json["persisted_frames"], 5000);
    assert_eq!(final_json["persisted_packets"], 1);
    assert_eq!(final_json["summary"]["accepted_frames"], 5000);
    assert_eq!(final_json["partial_packet"], serde_json::Value::Null);
    let (_, _, reader) = native_reader(8);
    assert!(PcmArchive::new(&directory.0, reader).is_err());
    assert_eq!(
        fs::metadata(directory.0.join("native-pcm/samples.f32le"))
            .unwrap()
            .len(),
        5000 * 8
    );
}

#[test]
fn pcm_real_reader_open_writer_and_overflow_refuse_but_do_not_stop_capture() {
    for close in [false, true] {
        let directory = TemporaryDirectory::new();
        let (mut ingress, _source, reader) = native_reader(2);
        let mut archive = PcmArchive::new(&directory.0, reader).unwrap();
        native_packet(&mut ingress, 3);
        let mut ingress = Some(ingress);
        if close {
            drop(ingress.take());
        }
        archive.finish().unwrap();
        assert!(!archive.persistence_complete());
        let final_json: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.0.join("native-pcm/final.json")).unwrap())
                .unwrap();
        assert_eq!(final_json["persistence_complete"], false);
        assert!(final_json["error"].is_string());
        if let Some(ingress) = &mut ingress {
            native_packet(ingress, 1);
        }
    }
}

#[test]
fn pcm_receipt_failure_still_writes_independent_cursor_receipt() {
    let directory = TemporaryDirectory::new();
    let (mut ingress, _source, reader) = native_reader(8);
    let mut pcm = PcmArchive::new(&directory.0, reader).unwrap();
    native_packet(&mut ingress, 2);
    drop(ingress);
    let path = directory.0.join("native-pcm/final.json");
    fs::write(&path, b"preserve conflicting receipt").unwrap();
    let error = super::super::finish_pcm_and_cursor(Some(&mut pcm), |pcm_error, _| {
        write_new(
            &directory.0.join("final.json"),
            &serde_json::json!({
                "qualified": false, "native_pcm_error": pcm_error,
                "persistence_complete": false, "cursor_receipt_retained": true,
            }),
        )
    })
    .unwrap_err();
    assert!(error.to_string().contains("PCM final receipt"));
    assert_eq!(fs::read(path).unwrap(), b"preserve conflicting receipt");
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.0.join("final.json")).unwrap()).unwrap();
    assert_eq!(receipt["cursor_receipt_retained"], true);
    assert!(receipt["native_pcm_error"].is_string());
    assert!(!pcm.persistence_complete());
}

#[test]
fn pcm_failure_after_a_committed_batch_retains_exact_successful_prefix() {
    let mut p = persistence();
    p.persist(&[frame(0)]).unwrap();
    let prefix_bytes = p.persisted_bytes;
    p.packets.get_mut().fail_sync = true;
    assert!(p.persist(&[frame(1), frame(2)]).is_err());
    assert_eq!(p.persisted_frames, 1);
    assert_eq!(p.persisted_packets, 1);
    assert_eq!(p.persisted_bytes, prefix_bytes);
    assert_eq!(p.mapping.stored_frames, 1);
    assert_eq!(p.mapping.offset, 1);
    p.packets.get_mut().fail_sync = false;
    assert!(
        p.persist(&[frame(1), frame(2)]).is_err(),
        "a partial durable write is never retried as success"
    );
    assert!(p.validate_final(summary(3, 2, 1), true).is_err());
}
