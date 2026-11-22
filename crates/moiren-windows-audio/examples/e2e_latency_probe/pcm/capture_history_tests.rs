use super::PacketTimeline;
use moiren_windows_audio::capture::{PcmArtifactConfig, PcmEvidencePacket, PcmRecorder};
use serde_json::{Value, json};
use std::io::Cursor;

fn fixture(capacity: usize, packet_capacity: usize, count: u64) -> Vec<Value> {
    let mut recorder = PcmRecorder::new(
        PcmArtifactConfig::new(capacity).with_packet_capacity(packet_capacity),
        48_000,
        2,
    )
    .unwrap();
    for index in 0..count {
        recorder
            .record_native_packet(
                &[0; 64],
                PcmEvidencePacket {
                    frames: 8,
                    flags: 0,
                    device_position_frames: 100 + index * 8,
                    qpc_100ns: 1_000_000 + index * 8 * 10_000_000 / 48_000,
                },
            )
            .unwrap();
    }
    let artifact = recorder.finish();
    let mut rows = vec![json!({
        "schema_version": 1,
        "format": "moiren_native_capture_packet_history",
        "capture": artifact.metadata(),
    })];
    rows.extend(
        artifact
            .packet_history()
            .iter()
            .map(|row| serde_json::to_value(row).unwrap()),
    );
    rows
}

fn jsonl(rows: &[Value]) -> Vec<u8> {
    rows.iter()
        .flat_map(|row| format!("{row}\n").into_bytes())
        .collect()
}

fn parse(rows: &[Value], frames: u64) -> anyhow::Result<PacketTimeline> {
    PacketTimeline::read(Cursor::new(jsonl(rows)), 100_000, 100, frames, 48_000)
}

#[test]
fn capture_history_reads_exact_native_offers_without_inventing_epoch_or_source_time() {
    let rows = fixture(24, 3, 3);
    let timeline = parse(&rows, 24).unwrap();
    assert!(!timeline.diagnostic_invalid());
    let evidence = serde_json::to_value(timeline.diagnostic_evidence()).unwrap();
    assert_eq!(evidence["schema_version"], 2);
    assert_eq!(evidence["packet_count"], 3);
    assert_eq!(evidence["coverage_complete"], true);
    assert_eq!(evidence["native_epoch"], "not_recorded");
    assert_eq!(
        evidence["capture_history"]["scope"],
        "native_getbuffer_offers_before_release_not_capture_completion_or_physical_time"
    );
    assert_eq!(evidence["capture_history"]["header"], rows[0]);
    assert!(
        evidence["capture_history"]["issues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(evidence["startup"]["capture_record"], rows[1]);
}

#[test]
fn capture_history_unknown_null_zero_and_timestamp_error_stay_raw_and_invalid() {
    for stamp in [Value::Null, json!(0), json!(1_000_000)] {
        let mut rows = fixture(24, 3, 3);
        rows[2]["qpc_100ns"] = stamp.clone();
        if stamp == json!(1_000_000) {
            rows[2]["flags"] = json!(4);
            rows[0]["capture"]["timestamp_error_packets"] = json!(1);
        }
        let timeline = parse(&rows, 24).unwrap();
        assert!(timeline.diagnostic_invalid());
        let evidence = serde_json::to_value(timeline.diagnostic_evidence()).unwrap();
        assert_eq!(evidence["unknown_timestamp_packets"], 1);
        assert_eq!(
            evidence["observations"][0]["capture_record"]["qpc_100ns"],
            stamp
        );
        assert_eq!(evidence["native_epoch"], "not_recorded");
    }
}

#[test]
fn capture_history_partial_pcm_or_omitted_packets_never_become_complete_coverage() {
    for (capacity, packet_capacity, count) in [(17, 3, 3), (24, 2, 3), (8, 2, 3)] {
        let rows = fixture(capacity, packet_capacity, count);
        let timeline = parse(&rows, capacity as u64).unwrap();
        assert!(timeline.diagnostic_invalid());
        let evidence = serde_json::to_value(timeline.diagnostic_evidence()).unwrap();
        assert_eq!(evidence["coverage_complete"], false);
        assert!(
            !evidence["capture_history"]["issues"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let mut rows = fixture(24, 3, 3);
    rows[0]["capture"]["packet_history"]["counters_saturated"] = json!(true);
    rows[0]["capture"]["packet_history"]["complete"] = json!(false);
    let timeline = parse(&rows, 24).unwrap();
    assert!(timeline.diagnostic_invalid());
    let evidence = serde_json::to_value(timeline.diagnostic_evidence()).unwrap();
    assert!(
        evidence["capture_history"]["issues"]
            .as_array()
            .unwrap()
            .contains(&json!("native_capture_counters_saturated"))
    );
}

#[test]
fn capture_history_refuses_truncation_mixed_shapes_and_contradictory_totals() {
    let valid = fixture(24, 3, 3);
    for fault in [
        "truncated",
        "extra",
        "observed_frontier",
        "stored_frontier",
        "stored_exceeds_offer",
        "aggregate_packets",
        "aggregate_frames",
        "aggregate_silent",
        "declared_complete_with_loss",
        "bad_rate",
        "bad_channels",
        "unknown_header",
        "unknown_row",
        "unknown_version",
        "unknown_format",
        "missing_qpc",
        "native_without_header",
        "mixed_legacy",
    ] {
        let mut rows = valid.clone();
        match fault {
            "truncated" => {
                rows.pop();
            }
            "extra" => rows.push(rows[3].clone()),
            "observed_frontier" => rows[2]["observed_frame_start"] = json!(9),
            "stored_frontier" => rows[2]["stored_frame_start"] = json!(9),
            "stored_exceeds_offer" => rows[2]["stored_frames"] = json!(9),
            "aggregate_packets" => rows[0]["capture"]["packets"] = json!(4),
            "aggregate_frames" => rows[0]["capture"]["observed_frames"] = json!(25),
            "aggregate_silent" => rows[0]["capture"]["silent_frames"] = json!(8),
            "declared_complete_with_loss" => {
                rows[0]["capture"]["packet_history"]["omitted_packets"] = json!(1)
            }
            "bad_rate" => rows[0]["capture"]["sample_rate"] = json!(44_100),
            "bad_channels" => rows[0]["capture"]["channels"] = json!(1),
            "unknown_header" => rows[0]["epoch"] = json!(1),
            "unknown_row" => rows[2]["epoch"] = json!(1),
            "unknown_version" => rows[0]["schema_version"] = json!(2),
            "unknown_format" => rows[0]["format"] = json!("other"),
            "missing_qpc" => {
                rows[2].as_object_mut().unwrap().remove("qpc_100ns");
            }
            "native_without_header" => {
                rows.remove(0);
            }
            "mixed_legacy" => {
                rows[2] = json!({"packet_sequence":2,"flags":0,"device_position_frames":108,"qpc_100ns":1_001_666,"frames":8,"stored_start_frame":8})
            }
            _ => unreachable!(),
        }
        assert!(parse(&rows, 24).is_err(), "accepted {fault}");
    }
}

#[test]
fn capture_history_preserves_legacy_diagnostic_flags_and_sequence_clock_refusals() {
    for fault in [
        "discontinuity",
        "silent",
        "flags",
        "sequence",
        "qpc_jump",
        "position",
    ] {
        let mut rows = fixture(24, 3, 3);
        match fault {
            "discontinuity" => {
                rows[2]["flags"] = json!(1);
                rows[0]["capture"]["discontinuity_events"] = json!(1);
            }
            "silent" => {
                rows[2]["flags"] = json!(2);
                rows[0]["capture"]["silent_frames"] = json!(8);
            }
            "flags" => rows[2]["flags"] = json!(8),
            "sequence" => rows[2]["packet_sequence"] = json!(7),
            "qpc_jump" => rows[2]["qpc_100ns"] = json!(9_000_000),
            "position" => {
                rows[2]["device_position_frames"] = json!(1);
                rows[0]["capture"]["gap_events"] = json!(2);
                rows[0]["capture"]["gap_frames"] = json!(214);
            }
            _ => unreachable!(),
        }
        let timeline = parse(&rows, 24).unwrap();
        assert!(timeline.diagnostic_invalid(), "accepted {fault}");
    }
    let mut rows = fixture(24, 3, 3);
    rows[0]["capture"]["detect_position_gaps"] = json!(false);
    for row in &mut rows[1..] {
        row["device_position_frames"] = json!(0);
    }
    let timeline = parse(&rows, 24).unwrap();
    assert!(!timeline.diagnostic_invalid());
    assert_eq!(
        serde_json::to_value(timeline.diagnostic_evidence()).unwrap()["unknown_position_packets"],
        3
    );
}

#[test]
fn capture_history_header_and_rows_share_existing_byte_line_and_packet_caps() {
    let raw = jsonl(&fixture(24, 3, 3));
    let read = |bytes: Vec<u8>, limit, packets| {
        PacketTimeline::read(Cursor::new(bytes), limit, packets, 24, 48_000)
    };
    assert!(read(raw.clone(), raw.len() as u64, 3).is_ok());
    assert!(read(raw.clone(), raw.len() as u64 - 1, 3).is_err());
    assert!(read(raw.clone(), raw.len() as u64, 2).is_err());
    assert!(read(raw[..raw.len() - 1].to_vec(), 100_000, 3).is_err());
    let mut duplicate = raw.clone();
    duplicate.splice(1..1, b"\"schema_version\":1,".iter().copied());
    assert!(read(duplicate, 100_000, 3).is_err());
}

#[test]
fn capture_history_omitted_suffix_must_reconcile_pcm_frontiers_and_flag_totals() {
    for fault in [
        "stored_suffix",
        "silent_suffix",
        "gap_prefix",
        "not_a_full_prefix",
    ] {
        let mut rows = fixture(24, 2, 3);
        match fault {
            "stored_suffix" => {
                rows[0]["capture"]["stored_frames"] = json!(17);
                rows[0]["capture"]["dropped_frames"] = json!(7);
            }
            "silent_suffix" => rows[0]["capture"]["silent_frames"] = json!(16),
            "gap_prefix" => {
                rows[2]["device_position_frames"] = json!(1);
            }
            "not_a_full_prefix" => {
                rows[0]["capture"]["packet_history"]["capacity_packets"] = json!(3)
            }
            _ => unreachable!(),
        }
        let frames = rows[0]["capture"]["stored_frames"].as_u64().unwrap();
        assert!(parse(&rows, frames).is_err(), "accepted {fault}");
    }
}

#[test]
fn capture_history_declared_position_gaps_cannot_be_clean_for_zero_positions() {
    let mut recorder = PcmRecorder::new(
        PcmArtifactConfig::new(24).with_packet_capacity(3),
        48_000,
        2,
    )
    .unwrap();
    for index in 0..3 {
        recorder
            .record_native_packet(
                &[0; 64],
                PcmEvidencePacket {
                    frames: 8,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 1_000_000 + index * 8 * 10_000_000 / 48_000,
                },
            )
            .unwrap();
    }
    let artifact = recorder.finish();
    assert_eq!(artifact.metadata().gap_events, 2);
    assert_eq!(artifact.metadata().gap_frames, 16);
    let mut rows = vec![json!({
        "schema_version": 1,
        "format": "moiren_native_capture_packet_history",
        "capture": artifact.metadata(),
    })];
    rows.extend(
        artifact
            .packet_history()
            .iter()
            .map(|row| serde_json::to_value(row).unwrap()),
    );
    let timeline = parse(&rows, 24).unwrap();
    assert!(timeline.diagnostic_invalid());
    let evidence = serde_json::to_value(timeline.diagnostic_evidence()).unwrap();
    assert!(
        evidence["capture_history"]["issues"]
            .as_array()
            .unwrap()
            .contains(&json!("native_capture_position_gaps"))
    );
}

#[test]
fn capture_history_omitted_overflow_count_must_fit_exact_remaining_storage() {
    for (capacity, count, overflows, valid) in [
        (8, 2, 0, false),
        (8, 2, 1, true),
        // Omitted offer sizes are unknown: 9+7 can overflow twice with eight
        // frames free, even though this producer fixture originally used 8+8.
        (16, 3, 2, true),
        (16, 3, 1, true),
        (16, 3, 0, false),
        (23, 3, 2, false),
        (23, 3, 1, true),
        (24, 3, 1, false),
        (24, 3, 0, true),
    ] {
        let mut rows = fixture(capacity, 1, count);
        rows[0]["capture"]["overflow_events"] = json!(overflows);
        let frames = rows[0]["capture"]["stored_frames"].as_u64().unwrap();
        assert_eq!(
            parse(&rows, frames).is_ok(),
            valid,
            "capacity={capacity}, count={count}, overflows={overflows}"
        );
    }
}

#[test]
fn capture_history_omitted_frames_cannot_exceed_native_u32_packet_lengths() {
    for (omitted, valid) in [
        (u64::from(u32::MAX), true),
        (u64::from(u32::MAX) + 1, false),
    ] {
        let mut rows = fixture(8, 1, 2);
        rows[0]["capture"]["observed_frames"] = json!(8 + omitted);
        rows[0]["capture"]["dropped_frames"] = json!(omitted);
        rows[0]["capture"]["packet_history"]["omitted_frames"] = json!(omitted);
        assert_eq!(parse(&rows, 8).is_ok(), valid, "omitted={omitted}");
    }
}

#[test]
fn capture_history_omitted_overflows_match_exhaustive_small_native_offers() {
    use std::collections::BTreeSet;

    let mut possible = BTreeSet::new();
    for a in 1_u64..=12 {
        for b in 1_u64..=12 {
            for c in 1_u64..=12 {
                for packets in 1..=3 {
                    let sizes = &[a, b, c][..packets];
                    let total: u64 = sizes.iter().sum();
                    if total > 12 {
                        continue;
                    }
                    for remaining in 0_u64..=12 {
                        let mut free = remaining;
                        let mut overflows = 0;
                        for &frames in sizes {
                            overflows += u64::from(frames > free);
                            free = free.saturating_sub(frames);
                        }
                        possible.insert((packets as u64, total, remaining, overflows));
                    }
                }
            }
        }
    }
    for packets in 1_u64..=3 {
        for total in packets..=12 {
            for remaining in 0_u64..=12 {
                for overflows in 0..=packets {
                    let mut rows = fixture(8, 1, 2);
                    rows[0]["capture"]["capacity_frames"] = json!(8 + remaining);
                    rows[0]["capture"]["packets"] = json!(1 + packets);
                    rows[0]["capture"]["observed_frames"] = json!(8 + total);
                    let stored = 8 + remaining.min(total);
                    rows[0]["capture"]["stored_frames"] = json!(stored);
                    rows[0]["capture"]["dropped_frames"] = json!(total.saturating_sub(remaining));
                    rows[0]["capture"]["overflow_events"] = json!(overflows);
                    rows[0]["capture"]["packet_history"]["omitted_packets"] = json!(packets);
                    rows[0]["capture"]["packet_history"]["omitted_frames"] = json!(total);
                    let key = (packets, total, remaining, overflows);
                    assert_eq!(
                        parse(&rows, stored).is_ok(),
                        possible.contains(&key),
                        "{key:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn capture_history_legacy_report_fields_stay_unchanged_with_no_new_section() {
    let rows: Vec<_> = (0..3).map(|index| json!({
        "packet_sequence":index+1,"flags":if index==1 {1}else{0},
        "device_position_frames":100+index*8,"qpc_100ns":1_000_000+index*8*10_000_000/48_000,
        "frames":8,"stored_start_frame":index*8
    })).collect();
    let timeline = parse(&rows, 24).unwrap();
    let evidence = serde_json::to_value(timeline.diagnostic_evidence()).unwrap();
    assert_eq!(evidence["schema_version"], 1);
    assert!(evidence.get("capture_history").is_none());
    assert!(evidence["startup"].get("capture_record").is_none());
    assert_eq!(evidence["startup"]["packet"], rows[0]);
    for (key, value) in rows[1].as_object().unwrap() {
        assert_eq!(&evidence["observations"][0][key], value);
    }
    assert!(evidence["observations"][0].get("capture_record").is_none());
}

#[test]
fn capture_history_short_storage_invalidates_existing_pcm_delay_windows() {
    use moiren_windows_audio::latency::MarkerGenerator;
    let mut marker = vec![0.0_f32; 255];
    MarkerGenerator::new(255, 0.1).unwrap().fill(&mut marker);
    let mut samples = vec![[0.0_f32; 2]; 512];
    for (index, &value) in marker.iter().enumerate() {
        samples[7 + index][0] = value;
        samples[30 + index][1] = value;
    }
    let raw: Vec<_> = samples
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    for incomplete in [false, true] {
        let mut recorder = PcmRecorder::new(
            PcmArtifactConfig::new(512).with_packet_capacity(65),
            48_000,
            2,
        )
        .unwrap();
        for (index, bytes) in raw.chunks_exact(64).enumerate() {
            recorder
                .record_native_packet(
                    bytes,
                    PcmEvidencePacket {
                        frames: 8,
                        flags: 0,
                        device_position_frames: 100 + index as u64 * 8,
                        qpc_100ns: 1_000_000 + index as u64 * 8 * 10_000_000 / 48_000,
                    },
                )
                .unwrap();
        }
        if incomplete {
            recorder
                .record_native_packet(
                    &[0; 64],
                    PcmEvidencePacket {
                        frames: 8,
                        flags: 0,
                        device_position_frames: 612,
                        qpc_100ns: 1_106_666,
                    },
                )
                .unwrap();
        }
        let artifact = recorder.finish();
        let mut rows = vec![
            json!({"schema_version":1,"format":"moiren_native_capture_packet_history","capture":artifact.metadata()}),
        ];
        rows.extend(
            artifact
                .packet_history()
                .iter()
                .map(|r| serde_json::to_value(r).unwrap()),
        );
        let timeline =
            PacketTimeline::read(Cursor::new(jsonl(&rows)), 100_000, 65, 512, 48_000).unwrap();
        let schedule = serde_json::from_value(json!({
            "schema_version":1,"sample_rate":48000,"reference_channel":0,"processed_channel":1,
            "marker_frames":255,"marker_amplitude":0.1,"reference_search_frames":16,
            "max_delay_frames":128,"max_marker_frames":255,"max_window_frames":399,
            "max_markers":1,"max_pcm_frames":512,"max_correlation_samples_per_marker":255*146,
            "max_total_correlation_samples":255*146,"max_total_window_frames":399,
            "min_correlation":0.98,"min_peak_margin":0.15,"min_rms":0.0001,
            "windows":[{"start_frame":0}]
        }))
        .unwrap();
        let analysis = timeline
            .analyze(
                &mut Cursor::new(artifact.pcm_bytes()),
                raw.len() as u64,
                schedule,
            )
            .unwrap();
        assert_eq!(analysis.failed(), incomplete);
        let report = serde_json::to_value(analysis).unwrap();
        assert_eq!(report["qualification"], "not_evaluated");
        if incomplete {
            assert!(report["results"][0]["delay_frames"].is_null());
            assert!(report["distribution"].is_null());
        } else {
            assert_eq!(report["results"][0]["delay_frames"], 23);
        }
    }
}
