use super::{PacketTimeline, analyze};
use moiren_windows_audio::latency::MarkerGenerator;
use serde_json::{Value, json};
use std::io::Cursor;

fn schedule(count: usize) -> Value {
    json!({
        "schema_version":1,"sample_rate":48000,"reference_channel":0,"processed_channel":1,
        "marker_frames":255,"marker_amplitude":0.1,"reference_search_frames":16,
        "max_delay_frames":128,"max_marker_frames":255,"max_window_frames":399,
        "max_markers":16,"max_pcm_frames":16384,
        "max_correlation_samples_per_marker":255 * (17 + 129),
        "max_total_correlation_samples":255 * (17 + 129) * 16,
        "max_total_window_frames":399 * 16,
        "min_correlation":0.98,"min_peak_margin":0.15,"min_rms":0.0001,
        "trend":{"min_markers":3,"min_span_frames":1000},
        "windows":(0..count).map(|index|json!({"start_frame":index*512})).collect::<Vec<_>>()
    })
}

fn raw(delays: &[Option<usize>]) -> Vec<u8> {
    let mut marker = vec![0.0; 255];
    MarkerGenerator::new(255, 0.1).unwrap().fill(&mut marker);
    let mut pcm = vec![[0.0_f32; 2]; delays.len() * 512];
    for (index, delay) in delays.iter().enumerate() {
        for (offset, value) in marker.iter().enumerate() {
            pcm[index * 512 + 7 + offset][0] = *value;
            if let Some(delay) = delay {
                pcm[index * 512 + 7 + delay + offset][1] = -*value * 0.4;
            }
        }
    }
    pcm.into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect()
}

fn analysis(delays: &[Option<usize>], settings: Value) -> Value {
    let raw = raw(delays);
    serde_json::to_value(
        analyze(
            &mut Cursor::new(&raw),
            raw.len() as u64,
            serde_json::from_value(settings).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn near(actual: &Value, expected: f64) {
    assert!(
        (actual.as_f64().unwrap() - expected).abs() < 1e-8,
        "{actual} != {expected}"
    );
}

#[test]
fn pcm_trend_keeps_raw_delays_and_fits_positive_negative_and_constant_relative_slopes() {
    for (delays, slope) in [
        ([23, 33, 43], 10.0 / 512.0),
        ([23, 13, 3], -10.0 / 512.0),
        ([23, 23, 23], 0.0),
    ] {
        let report = analysis(&delays.map(Some), schedule(3));
        assert_eq!(report["failed_markers"], 0);
        assert_eq!(report["schema_version"], 5);
        let trend = &report["trend"];
        assert_eq!(trend["continuity"], "unverified_no_packet_metadata");
        assert_eq!(trend["segments"].as_array().unwrap().len(), 1);
        let segment = &trend["segments"][0];
        assert_eq!(segment["marker_count"], 3);
        assert_eq!(segment["reference_span_frames"], 1024);
        near(&segment["fit"]["slope_frames_per_reference_frame"], slope);
        near(
            &segment["fit"]["relative_delay_change_ppm"],
            slope * 1_000_000.0,
        );
        near(&segment["fit"]["intercept_at_first_reference_frames"], 23.0);
        near(&segment["fit"]["residual_rms_frames"], 0.0);
        for (index, delay) in delays.iter().enumerate() {
            assert_eq!(report["results"][index]["delay_frames"], *delay);
        }
        assert_eq!(report["distribution"]["marker_count"], 3);
        assert_eq!(report["qualification"], "not_evaluated");
    }
}

#[test]
fn pcm_trend_splits_at_failed_markers_and_retains_insufficient_segments() {
    let report = analysis(&[Some(23), Some(23), None, Some(23), Some(23)], schedule(5));
    assert_eq!(report["failed_markers"], 1);
    let segments = report["trend"]["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2);
    for (id, segment) in segments.iter().enumerate() {
        assert_eq!(segment["segment_id"], id);
        assert_eq!(segment["marker_count"], 2);
        assert!(segment["fit"].is_null());
        assert_eq!(segment["refusal"], "insufficient_marker_count");
    }
    assert_eq!(segments[1]["first_window_index"], 3);
}

#[test]
fn pcm_trend_retains_insufficient_span_without_changing_distribution() {
    let mut settings = schedule(3);
    settings["trend"]["min_span_frames"] = json!(1025);
    let report = analysis(&[Some(23); 3], settings);
    assert_eq!(
        report["trend"]["segments"][0]["refusal"],
        "insufficient_reference_span"
    );
    assert_eq!(report["distribution"]["marker_count"], 3);
}

#[test]
fn pcm_trend_is_opt_in_and_legacy_fields_remain_absent() {
    let mut settings = schedule(3);
    settings.as_object_mut().unwrap().remove("trend");
    let report = analysis(&[Some(23); 3], settings);
    assert_eq!(report["schema_version"], 1);
    assert!(report.get("trend").is_none());
    assert!(report["schedule"].get("trend").is_none());
}

fn packets(count: usize, invalid_start: Option<usize>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for frame in (0..count * 512).step_by(8) {
        let packet = json!({"packet_sequence":frame/8+1,"frames":8,
            "stored_start_frame":frame,"device_position_frames":100+frame,
            "qpc_100ns":1_000_000+frame as u64*10_000_000/48000,
            "flags":usize::from(invalid_start==Some(frame))});
        bytes.extend(format!("{packet}\n").bytes());
    }
    bytes
}

#[test]
fn pcm_trend_packet_fault_only_between_windows_splits_without_invalidating_markers() {
    let raw = raw(&[Some(23); 6]);
    let timeline = PacketTimeline::read(
        Cursor::new(packets(6, Some(1440))),
        1_048_576,
        1000,
        raw.len() as u64 / 8,
        48000,
    )
    .unwrap();
    let output = timeline
        .analyze(
            &mut Cursor::new(&raw),
            raw.len() as u64,
            serde_json::from_value(schedule(6)).unwrap(),
        )
        .unwrap();
    assert!(output.failed());
    let report = serde_json::to_value(output).unwrap();
    assert_eq!(report["failed_markers"], 0);
    assert_eq!(report["distribution"]["marker_count"], 6);
    assert_eq!(report["schema_version"], 6);
    assert_eq!(
        report["trend"]["continuity"],
        "diagnostic_packet_metadata_not_native_epoch"
    );
    let segments = report["trend"]["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2);
    for segment in segments {
        assert_eq!(segment["marker_count"], 3);
        assert!(segment["fit"].is_object());
    }
    assert_eq!(report["packet_evidence"]["native_epoch"], "not_recorded");
}

#[test]
fn pcm_trend_packet_barriers_cover_half_open_edges_and_spanning_faults() {
    let raw = raw(&[Some(23); 6]);
    // 1416..1424 overlaps window2 ending1423;1424..1432 and1528..1536
    // are gaps (latter touches window3 start1536);1536..1544 overlaps3.
    for (fault, failed, counts) in [
        (1416, 1, vec![2, 3]),
        (1424, 0, vec![3, 3]),
        (1528, 0, vec![3, 3]),
        (1536, 1, vec![3, 2]),
    ] {
        let timeline = PacketTimeline::read(
            Cursor::new(packets(6, Some(fault))),
            1_048_576,
            1000,
            3072,
            48000,
        )
        .unwrap();
        let output = timeline
            .analyze(
                &mut Cursor::new(&raw),
                raw.len() as u64,
                serde_json::from_value(schedule(6)).unwrap(),
            )
            .unwrap();
        let report = serde_json::to_value(output).unwrap();
        assert_eq!(report["failed_markers"], failed, "{fault}");
        let segments = report["trend"]["segments"].as_array().unwrap();
        assert_eq!(
            segments
                .iter()
                .map(|s| s["marker_count"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            counts,
            "{fault}"
        );
    }

    let mut metadata = String::new();
    for (seq, start, frames, flags) in [(1, 0, 512, 0), (2, 512, 1536, 1), (3, 2048, 1024, 0)] {
        metadata.push_str(&format!(
            "{}\n",
            json!({"packet_sequence":seq,"frames":frames,
            "stored_start_frame":start,"device_position_frames":100+start,
            "qpc_100ns":1_000_000+start as u64*10_000_000/48000,"flags":flags})
        ));
    }
    let timeline = PacketTimeline::read(metadata.as_bytes(), 1_048_576, 1000, 3072, 48000).unwrap();
    let report = serde_json::to_value(
        timeline
            .analyze(
                &mut Cursor::new(&raw),
                raw.len() as u64,
                serde_json::from_value(schedule(6)).unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["failed_markers"], 3);
    assert_eq!(report["trend"]["segments"][0]["marker_count"], 1);
    assert_eq!(report["trend"]["segments"][1]["marker_count"], 2);
}

#[test]
fn pcm_trend_incomplete_packet_coverage_cannot_retain_a_prevalidation_fit() {
    let raw = raw(&[Some(23); 6]);
    let metadata = packets(5, None);
    let timeline =
        PacketTimeline::read(Cursor::new(metadata), 1_048_576, 1000, 3072, 48000).unwrap();
    let output = timeline
        .analyze(
            &mut Cursor::new(&raw),
            raw.len() as u64,
            serde_json::from_value(schedule(6)).unwrap(),
        )
        .unwrap();
    assert!(output.failed());
    let report = serde_json::to_value(output).unwrap();
    assert_eq!(report["failed_markers"], 6);
    assert!(report["distribution"].is_null());
    assert_eq!(report["trend"]["segments"], json!([]));
}

#[test]
fn pcm_trend_packet_fault_exactly_between_half_open_windows_preserves_both() {
    let raw = raw(&[Some(23); 6]);
    let mut metadata = Vec::new();
    // Window2 ends at1423; window3 starts1536. The invalid middle packet
    // touches both but overlaps neither, so only the trend must be split.
    for (sequence, start, end) in [(1, 0, 1423), (2, 1423, 1536), (3, 1536, 3072)] {
        let packet = json!({"packet_sequence":sequence,"frames":end-start,
            "stored_start_frame":start,"device_position_frames":100+start,
            "qpc_100ns":1_000_000+start as u64*10_000_000/48000,
            "flags":usize::from(sequence==2)});
        metadata.extend(format!("{packet}\n").bytes());
    }
    let timeline = PacketTimeline::read(Cursor::new(metadata), 1_048_576, 10, 3072, 48000).unwrap();
    let output = timeline
        .analyze(
            &mut Cursor::new(&raw),
            raw.len() as u64,
            serde_json::from_value(schedule(6)).unwrap(),
        )
        .unwrap();
    assert!(output.failed());
    let report = serde_json::to_value(output).unwrap();
    assert_eq!(report["failed_markers"], 0);
    assert_eq!(report["trend"]["segments"].as_array().unwrap().len(), 2);
    for segment in report["trend"]["segments"].as_array().unwrap() {
        assert_eq!(segment["marker_count"], 3);
        assert!(segment["fit"].is_object());
    }
}

#[test]
fn pcm_trend_gap_packet_loss_unknown_timestamp_and_unknown_flags_are_all_barriers() {
    let raw = raw(&[Some(23); 6]);
    for kind in ["sequence_loss", "unknown_timestamp", "unknown_flags"] {
        let mut metadata = Vec::new();
        for frame in (0..3072).step_by(8) {
            let mut packet = json!({"packet_sequence":frame/8+1,"frames":8,
                "stored_start_frame":frame,"device_position_frames":100+frame,
                "qpc_100ns":1_000_000+frame as u64*10_000_000/48000,"flags":0});
            if kind == "sequence_loss" && frame >= 1440 {
                packet["packet_sequence"] = json!(frame / 8 + 2);
            } else if kind == "unknown_timestamp" && frame == 1440 {
                packet["qpc_100ns"] = json!(0);
            } else if kind == "unknown_flags" && frame == 1440 {
                packet["flags"] = json!(1_u32 << 31);
            }
            metadata.extend(format!("{packet}\n").bytes());
        }
        let timeline =
            PacketTimeline::read(Cursor::new(metadata), 1_048_576, 1000, 3072, 48000).unwrap();
        let output = timeline
            .analyze(
                &mut Cursor::new(&raw),
                raw.len() as u64,
                serde_json::from_value(schedule(6)).unwrap(),
            )
            .unwrap();
        assert!(output.failed());
        let report = serde_json::to_value(output).unwrap();
        assert_eq!(report["failed_markers"], 0, "{kind}");
        assert_eq!(
            report["trend"]["segments"].as_array().unwrap().len(),
            2,
            "{kind}"
        );
    }
}

fn points(origin: u64, offsets: &[u64], delays: &[usize]) -> Vec<super::WindowResult> {
    offsets
        .iter()
        .zip(delays)
        .enumerate()
        .map(|(index, (&offset, &delay))| {
            let alignment = |absolute_frame| super::Alignment {
                absolute_frame,
                confidence: 1.0,
                peak_margin: 1.0,
                inverted: false,
                rate_hypothesis: None,
            };
            super::WindowResult {
                index,
                start_frame: origin + offset,
                reference: Some(alignment(origin + offset)),
                processed: Some(alignment(origin + offset + delay as u64)),
                delay_frames: Some(delay),
                error: None,
            }
        })
        .collect()
}

#[test]
fn pcm_trend_large_origins_are_centered_before_float_conversion() {
    let points = points(u64::MAX - 2000, &[0, 512, 1024], &[23, 33, 43]);
    let trend = serde_json::to_value(super::trend::analyze(
        &points,
        super::trend::TrendConfig {
            min_markers: 3,
            min_span_frames: 1000,
        },
        None,
    ))
    .unwrap();
    near(
        &trend["segments"][0]["fit"]["relative_delay_change_ppm"],
        10.0 / 512.0 * 1_000_000.0,
    );
    near(
        &trend["segments"][0]["fit"]["intercept_at_first_reference_frames"],
        23.0,
    );
}

#[test]
fn pcm_trend_refuses_precision_loss_and_nonincreasing_reference_positions() {
    let config = super::trend::TrendConfig {
        min_markers: 3,
        min_span_frames: 1,
    };
    for (offsets, reason) in [
        ([0, 1, 1 << 53], "exact_integer_precision_policy_exceeded"),
        ([0, 512, 512], "nonincreasing_reference_positions"),
        ([0, 512, 511], "nonincreasing_reference_positions"),
    ] {
        let trend = serde_json::to_value(super::trend::analyze(
            &points(0, &offsets, &[23; 3]),
            config,
            None,
        ))
        .unwrap();
        assert_eq!(trend["segments"][0]["refusal"], reason);
        assert!(trend["segments"][0]["fit"].is_null());
    }
    let trend = serde_json::to_value(super::trend::analyze(
        &points(0, &[0, 512, 1024], &[23, 1_usize << 53, 23]),
        config,
        None,
    ))
    .unwrap();
    assert_eq!(
        trend["segments"][0]["refusal"],
        "exact_integer_precision_policy_exceeded"
    );
}

#[test]
fn pcm_trend_outliers_are_retained_in_raw_distribution_and_residuals() {
    let report = analysis(&[Some(23), Some(83), Some(23)], schedule(3));
    assert_eq!(report["distribution"]["marker_count"], 3);
    assert_eq!(report["results"][1]["delay_frames"], 83);
    let fit = &report["trend"]["segments"][0]["fit"];
    near(&fit["relative_delay_change_ppm"], 0.0);
    near(&fit["intercept_at_first_reference_frames"], 43.0);
    near(&fit["residual_rms_frames"], 800.0_f64.sqrt());
    near(&fit["residual_max_abs_frames"], 40.0);
}

#[test]
fn pcm_trend_invalid_config_refuses_before_any_pcm_access() {
    struct Unreadable;
    impl std::io::Read for Unreadable {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("read before validation")
        }
    }
    impl std::io::Seek for Unreadable {
        fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
            panic!("seek before validation")
        }
    }
    for config in [
        json!({"min_markers":2,"min_span_frames":1}),
        json!({"min_markers":17,"min_span_frames":1}),
        json!({"min_markers":3,"min_span_frames":0}),
        json!({"min_markers":3,"min_span_frames":1_u64<<53}),
    ] {
        let mut settings = schedule(3);
        settings["trend"] = config;
        let error = analyze(
            &mut Unreadable,
            1536 * 8,
            serde_json::from_value(settings).unwrap(),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("trend"));
    }
}

#[test]
fn pcm_trend_only_live_import_retains_full_unqualified_measurement() {
    let directory = std::env::temp_dir().join(format!("moiren-trend-live-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let capture = directory.join("capture.raw");
    let settings = directory.join("schedule.json");
    let artifacts = directory.join("run");
    std::fs::write(&capture, raw(&[Some(23), Some(33), Some(43)])).unwrap();
    std::fs::write(&settings, serde_json::to_vec(&schedule(3)).unwrap()).unwrap();
    assert_eq!(
        crate::cli::run(vec![
            "live-process-loopback".into(),
            "--dry-run".into(),
            "--source-pid".into(),
            "1".into(),
            "--output".into(),
            "unopened".into(),
            "--artifact-dir".into(),
            artifacts.to_string_lossy().into_owned(),
            "--measurement-pcm".into(),
            capture.to_string_lossy().into_owned(),
            "--schedule".into(),
            settings.to_string_lossy().into_owned()
        ])
        .unwrap(),
        1
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(artifacts.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["qualified"], false);
    assert_eq!(report["trend_measurement"]["schema_version"], 5);
    assert_eq!(
        report["trend_measurement"]["trend"]["segments"][0]["marker_count"],
        3
    );
    assert_eq!(report["qualification"]["distribution"]["marker_count"], 3);
    assert_eq!(report["qualification"]["steady_state"], false);
    assert!(report.get("rate_search_measurement").is_none());
    assert!(report["native"]["capture"].is_null());
}
