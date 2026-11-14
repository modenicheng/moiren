use super::*;
use crate::live::source::{INDEXED_WAVEFORM_VERSION, SourceCursor, SourceSchedule};
use serde_json::{Value, json};
use std::io::Cursor;

fn source() -> SourceSchedule {
    SourceSchedule {
        schema_version: 1,
        schedule_id: "indexed-detector-fixture".into(),
        waveform_version: INDEXED_WAVEFORM_VERSION.into(),
        sample_rate: 48_000,
        channels: 2,
        marker_frames: 64,
        marker_amplitude: 0.1,
        total_source_frames: 1000,
        starts: vec![100, 500, 900],
    }
}

fn config() -> IndexedConfig {
    IndexedConfig {
        schema_version: 1,
        recorder_origin: "stored_pcm_frame_zero".into(),
        sample_rate: 48_000,
        channel: 0,
        max_pcm_frames: 4096,
        max_correlation_samples: 1_000_000,
        min_correlation: 0.7,
        min_peak_margin: 0.05,
        min_rms: 0.001,
        // These are recorder windows, not source labels or a clock mapping.
        windows: [80, 480, 880]
            .map(|start_frame| RecorderWindow {
                start_frame,
                frames: 128,
            })
            .to_vec(),
    }
}

fn waveform() -> Vec<f32> {
    let mut cursor = SourceCursor::new(Some(source().prepare(1).unwrap()));
    let mut samples = vec![0.0; 2048];
    cursor.submit(&mut samples, false, |_| Ok(())).unwrap();
    samples
}

fn packets(frames: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, start) in (0..frames).step_by(100).enumerate() {
        let packet = json!({
            "packet_sequence": index + 1,
            "flags": 0,
            "device_position_frames": 1000 + start,
            "qpc_100ns": 1_000_000 + start as u64 * 10_000_000 / 48_000,
            "frames": (frames - start).min(100),
            "stored_start_frame": start
        });
        bytes.extend(serde_json::to_vec(&packet).unwrap());
        bytes.push(b'\n');
    }
    bytes
}

fn run(samples: &[f32], source: SourceSchedule, config: IndexedConfig) -> Result<Value> {
    let bytes: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    analyze(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        source,
        config,
        Cursor::new(packets(samples.len() / 2)),
        100_000,
        100,
    )
}

#[test]
fn exact_v2_codes_resolve_schedule_indices_without_clock_origin() {
    let result = run(&waveform(), source(), config()).unwrap();
    assert_eq!(result["identity_set_complete"], true);
    assert_eq!(result["qualified"], false);
    assert_eq!(result["source_to_recorder_origin"], "unknown_not_estimated");
    for (index, frame) in [100, 500, 900].into_iter().enumerate() {
        assert_eq!(result["results"][index]["identity_index"], index);
        assert_eq!(result["results"][index]["recording_frame"], frame);
        assert_eq!(result["results"][index]["status"], "unique");
    }
    assert_eq!(result["spacing"][0]["source_delta_frames"], 400);
    assert_eq!(result["spacing"][0]["recording_delta_frames"], 400);
    assert!(result.get("delay_frames").is_none());
}

#[test]
fn absent_first_marker_never_renumbers_later_codewords() {
    let mut samples = waveform();
    samples[200..328].fill(0.0);
    let result = run(&samples, source(), config()).unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["missing_identities"], json!([0]));
    assert_eq!(result["results"][0]["identity_index"], Value::Null);
    assert_eq!(result["results"][1]["identity_index"], 1);
    assert_eq!(result["results"][2]["identity_index"], 2);
}

#[test]
fn repeated_code_is_retained_as_duplicate_not_relabelled_by_peak_order() {
    let mut samples = waveform();
    let repeated = samples[1000..1128].to_vec();
    samples[200..328].copy_from_slice(&repeated);
    let result = run(&samples, source(), config()).unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["duplicate_identities"], json!([1]));
    assert_eq!(result["missing_identities"], json!([0]));
    assert_eq!(result["results"][0]["status"], "duplicate");
    assert_eq!(result["results"][1]["status"], "duplicate");
}

#[test]
fn equal_strength_different_codes_in_one_unlabelled_window_are_ambiguous() {
    let mut config = config();
    config.windows = vec![RecorderWindow {
        start_frame: 80,
        frames: 900,
    }];
    let result = run(&waveform(), source(), config).unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["results"][0]["status"], "ambiguous");
    assert_eq!(result["results"][0]["identity_index"], Value::Null);
}

#[test]
fn packet_loss_reset_silence_and_malformed_metadata_preserve_hits_but_refuse_complete_set() {
    let samples = waveform();
    let pcm: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let original = String::from_utf8(packets(samples.len() / 2)).unwrap();
    for field in [
        "packet_sequence",
        "device_position_frames",
        "qpc_100ns",
        "flags",
        "stored_start_frame",
    ] {
        let mut lines: Vec<Value> = original
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        lines[4][field] = json!(if field == "flags" { 1 } else { 0 });
        let metadata: Vec<_> = lines
            .iter()
            .flat_map(|v| {
                let mut bytes = serde_json::to_vec(v).unwrap();
                bytes.push(b'\n');
                bytes
            })
            .collect();
        let result = analyze(
            &mut Cursor::new(&pcm),
            pcm.len() as u64,
            source(),
            config(),
            Cursor::new(metadata),
            100_000,
            100,
        )
        .unwrap();
        assert_eq!(result["identity_set_complete"], false, "{field}");
        assert!(!result["issues"].as_array().unwrap().is_empty(), "{field}");
        assert_eq!(result["results"][1]["identity_index"], 1);
    }
    let result = analyze(
        &mut Cursor::new(&pcm),
        pcm.len() as u64,
        source(),
        config(),
        Cursor::new(b"bad\n"),
        100_000,
        100,
    )
    .unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["issues"][0], "invalid_packet_metadata");
}

#[test]
fn packet_fault_outside_search_windows_is_not_silently_ignored() {
    let samples = waveform();
    let pcm: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut lines: Vec<Value> = String::from_utf8(packets(samples.len() / 2))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    lines[3]["flags"] = json!(2);
    let metadata: Vec<_> = lines
        .iter()
        .flat_map(|v| {
            let mut b = serde_json::to_vec(v).unwrap();
            b.push(b'\n');
            b
        })
        .collect();
    let result = analyze(
        &mut Cursor::new(&pcm),
        pcm.len() as u64,
        source(),
        config(),
        Cursor::new(metadata),
        100_000,
        100,
    )
    .unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["missing_identities"], json!([]));
}

#[test]
fn nonfinite_or_clipped_samples_even_outside_windows_refuse_without_fabricated_hits() {
    for value in [f32::NAN, f32::INFINITY, 1.0, -1.0] {
        let mut samples = waveform();
        samples[700] = value;
        let result = run(&samples, source(), config()).unwrap();
        assert_eq!(result["identity_set_complete"], false);
        assert_eq!(result["results"], json!([]));
        assert!(
            result["issues"][0]
                .as_str()
                .unwrap()
                .starts_with("invalid_pcm:")
        );
    }
}

#[test]
fn fixed_identity_coordinates_survive_recording_translation_and_expose_spacing_not_delay() {
    let mut samples = vec![0.0; 122 * 2];
    samples.extend(waveform());
    let mut cfg = config();
    for w in &mut cfg.windows {
        w.start_frame += 122;
    }
    let result = run(&samples, source(), cfg).unwrap();
    assert_eq!(result["identity_set_complete"], true);
    assert_eq!(result["results"][0]["recording_frame"], 222);
    assert_eq!(result["spacing"][0]["spacing_residual_frames"], 0);
    assert_eq!(result["source_to_recorder_origin"], "unknown_not_estimated");
}

#[test]
fn reversed_code_order_refuses_complete_set_without_sorting_detected_peaks_into_identity() {
    let mut samples = waveform();
    let first = samples[200..328].to_vec();
    let last = samples[1800..1928].to_vec();
    samples[200..328].copy_from_slice(&last);
    samples[1800..1928].copy_from_slice(&first);
    let result = run(&samples, source(), config()).unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["results"][0]["identity_index"], 2);
    assert_eq!(result["results"][2]["identity_index"], 0);
    assert!(
        result["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "recorded_identity_order_differs_from_source_schedule")
    );
}

#[test]
fn silent_recording_has_missing_identity_not_a_zero_origin() {
    let result = run(&vec![0.0; 2048], source(), config()).unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["missing_identities"], json!([0, 1, 2]));
    assert!(
        result["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["recording_frame"].is_null())
    );
}

#[test]
fn unsupported_origin_version_bounds_and_lowered_thresholds_are_explicit_errors() {
    for (field, value) in [
        ("schema_version", json!(2)),
        ("recorder_origin", json!("source_frame_zero")),
        ("sample_rate", json!(44100)),
        ("channel", json!(2)),
        ("min_correlation", json!(0.69)),
        ("min_peak_margin", json!(0)),
        ("min_rms", json!(0)),
        ("max_correlation_samples", json!(1)),
        ("max_pcm_frames", json!(1)),
        ("windows", json!([])),
        ("windows", json!([{"start_frame":u64::MAX,"frames":128}])),
        ("windows", json!([{"start_frame":0,"frames":32}])),
        (
            "windows",
            json!([{"start_frame":0,"frames":128},{"start_frame":100,"frames":128}]),
        ),
    ] {
        let mut value_config = serde_json::to_value(config()).unwrap();
        value_config[field] = value;
        let cfg = serde_json::from_value(value_config).unwrap();
        assert!(run(&waveform(), source(), cfg).is_err(), "{field}");
    }
    let mut v1 = source();
    v1.waveform_version = crate::live::source::WAVEFORM_VERSION.into();
    assert!(
        run(&waveform(), v1, config())
            .unwrap_err()
            .to_string()
            .contains("indexed-v2")
    );
    let mut bad = source();
    bad.starts = vec![100, 100];
    assert!(run(&waveform(), bad, config()).is_err());
    assert!(
        run(&[0.0], source(), config())
            .unwrap_err()
            .to_string()
            .contains("complete stereo")
    );
}

#[test]
fn a_competing_code_just_below_detection_threshold_still_contributes_to_identity_margin() {
    let prepared = source().prepare(1).unwrap();
    let first = prepared.indexed_marker(0).unwrap();
    let second = prepared.indexed_marker(1).unwrap();
    let noisy = |code: &[f32], score: f64| -> Vec<f32> {
        let mut output = Vec::new();
        let global_mean = code.iter().map(|v| f64::from(*v)).sum::<f64>() / code.len() as f64;
        for section in code.chunks(16) {
            let mean = section.iter().map(|v| f64::from(*v)).sum::<f64>() / 16.0;
            let centered: Vec<_> = section.iter().map(|v| f64::from(*v) - mean).collect();
            let energy = centered.iter().map(|v| v * v).sum::<f64>();
            let mut noise: Vec<_> = (0..16)
                .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                .collect();
            let dot = noise.iter().zip(&centered).map(|(a, b)| a * b).sum::<f64>();
            for (n, c) in noise.iter_mut().zip(&centered) {
                *n -= dot / energy * c;
            }
            let norm = noise.iter().map(|v| v * v).sum::<f64>();
            let full_mean_energy = section
                .iter()
                .map(|v| (f64::from(*v) - global_mean).powi(2))
                .sum::<f64>();
            let scale = (full_mean_energy * (1.0 / (score * score) - 1.0) / norm).sqrt();
            output.extend(
                section
                    .iter()
                    .zip(noise)
                    .map(|(a, b)| (f64::from(*a) + b * scale) as f32),
            );
        }
        output
    };
    let mut samples = vec![0.0; 2048];
    for (start, code, score) in [(100, first, 0.72), (500, second, 0.69)] {
        for (pair, sample) in samples[start * 2..(start + 64) * 2]
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(noisy(code, score))
        {
            pair.fill(sample);
        }
    }
    let mut cfg = config();
    cfg.windows = vec![RecorderWindow {
        start_frame: 80,
        frames: 600,
    }];
    let result = run(&samples, source(), cfg).unwrap();
    assert_eq!(result["results"][0]["status"], "ambiguous");
    assert_eq!(result["results"][0]["identity_index"], Value::Null);
}

#[test]
fn indexed_cli_writes_create_new_unqualified_identity_artifact_and_retains_refusal() {
    let fixture = crate::tests::Fixture::new();
    let source_path = fixture.path("source.json");
    let config_path = fixture.path("indexed.json");
    let pcm_path = fixture.path("capture.f32le");
    let packet_path = fixture.path("packets.jsonl");
    let output = fixture.path("result.json");
    std::fs::write(&source_path, serde_json::to_vec(&source()).unwrap()).unwrap();
    std::fs::write(&config_path, serde_json::to_vec(&config()).unwrap()).unwrap();
    let pcm: Vec<_> = waveform().iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&pcm_path, pcm).unwrap();
    std::fs::write(&packet_path, packets(1024)).unwrap();
    let args = vec![
        "analyze-indexed-pcm".into(),
        "--pcm".into(),
        pcm_path.to_string_lossy().into(),
        "--source-schedule".into(),
        source_path.to_string_lossy().into(),
        "--analysis-config".into(),
        config_path.to_string_lossy().into(),
        "--packets".into(),
        packet_path.to_string_lossy().into(),
        "--max-input-bytes".into(),
        "100000".into(),
        "--max-packets".into(),
        "100".into(),
        "--output".into(),
        output.to_string_lossy().into(),
    ];
    assert_eq!(crate::cli::run(args.clone()).unwrap(), 0);
    let saved = std::fs::read(&output).unwrap();
    let result: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(result["qualified"], false);
    assert_eq!(result["identity_set_complete"], true);
    assert!(crate::cli::run(args.clone()).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), saved);
    let failed = fixture.path("refused.json");
    let mut fail_args = args;
    *fail_args.last_mut().unwrap() = failed.to_string_lossy().into();
    std::fs::write(&packet_path, b"malformed\n").unwrap();
    assert_eq!(crate::cli::run(fail_args).unwrap(), 1);
    let refused: Value = serde_json::from_slice(&std::fs::read(failed).unwrap()).unwrap();
    assert_eq!(refused["identity_set_complete"], false);
}

#[test]
fn all_29_codes_compete_with_explicit_work_cap_and_unordered_observation_labels() {
    let mut plan = source();
    plan.marker_frames = 1024;
    plan.starts = (0..29).map(|index| 24000 + 48000 * index).collect();
    plan.total_source_frames = 1_440_000;
    let prepared = plan.clone().prepare(30).unwrap();
    let indices = [28, 0, 14];
    let mut samples = vec![0.0; 3200 * 2];
    for (slot, index) in indices.into_iter().enumerate() {
        for (pair, value) in samples[slot * 1050 * 2..(slot * 1050 + 1024) * 2]
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(prepared.indexed_marker(index).unwrap())
        {
            pair.fill(*value);
        }
    }
    let mut cfg = config();
    cfg.windows = (0..3)
        .map(|slot| RecorderWindow {
            start_frame: slot * 1050,
            frames: 1024,
        })
        .collect();
    cfg.max_correlation_samples = 29 * 1024 * 3;
    let result = run(&samples, plan.clone(), cfg.clone()).unwrap();
    assert_eq!(result["admitted_correlation_samples"], 29 * 1024 * 3);
    for (slot, index) in indices.into_iter().enumerate() {
        assert_eq!(result["results"][slot]["identity_index"], index);
    }
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["missing_identities"].as_array().unwrap().len(), 26);
    cfg.max_correlation_samples -= 1;
    assert!(
        run(&samples, plan, cfg)
            .unwrap_err()
            .to_string()
            .contains("work cap")
    );
}

#[test]
fn an_erased_quarter_cannot_hide_behind_a_high_global_correlation() {
    let mut samples = waveform();
    samples[200..232].fill(0.0);
    let result = run(&samples, source(), config()).unwrap();
    assert_eq!(result["identity_set_complete"], false);
    assert_eq!(result["results"][0]["status"], "missing");
    assert_eq!(result["results"][0]["identity_index"], Value::Null);
    assert_eq!(
        result["results"][0]["best_candidate"]["sections_match"],
        false
    );
}

#[test]
fn duplicated_onsets_inside_one_window_are_ambiguous_not_single_best_peak() {
    let mut samples = waveform();
    let repeated = samples[200..328].to_vec();
    samples[600..728].copy_from_slice(&repeated);
    let mut cfg = config();
    cfg.windows = vec![RecorderWindow {
        start_frame: 80,
        frames: 320,
    }];
    let result = run(&samples, source(), cfg).unwrap();
    assert_eq!(result["results"][0]["status"], "ambiguous");
    assert_eq!(result["results"][0]["identity_index"], Value::Null);
    assert_eq!(
        result["results"][0]["best_candidate"]["onset_peak_margin"],
        0.0
    );
}

#[test]
fn selected_channel_and_inversion_do_not_infer_an_independent_reference() {
    let mut samples = waveform();
    for pair in samples.as_chunks_mut::<2>().0 {
        pair[0] = 0.0;
        pair[1] = -pair[1];
    }
    let mut cfg = config();
    assert_eq!(
        run(&samples, source(), cfg.clone()).unwrap()["identity_set_complete"],
        false
    );
    cfg.channel = 1;
    let result = run(&samples, source(), cfg).unwrap();
    assert_eq!(result["identity_set_complete"], true);
    assert_eq!(result["results"][0]["best_candidate"]["inverted"], true);
    assert_eq!(result["source_to_recorder_origin"], "unknown_not_estimated");
}

#[test]
fn analysis_rejects_bounds_before_reading_pcm_and_never_mutates_input() {
    struct Unreadable;
    impl std::io::Read for Unreadable {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("PCM read before admission")
        }
    }
    impl std::io::Seek for Unreadable {
        fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
            panic!("PCM seek before admission")
        }
    }
    let mut cfg = config();
    cfg.max_correlation_samples = 1;
    assert!(
        analyze(
            &mut Unreadable,
            8192,
            source(),
            cfg,
            Cursor::new([]),
            100,
            10
        )
        .is_err()
    );
    let samples = waveform();
    let original = samples.clone();
    run(&samples, source(), config()).unwrap();
    assert_eq!(samples, original);
}

#[test]
fn all_bounded_inputs_refuse_unknown_fields_nonfinite_thresholds_and_window_truncation() {
    for (name, value) in [
        ("min_rms", f64::NAN),
        ("min_peak_margin", f64::INFINITY),
        ("min_correlation", f64::NEG_INFINITY),
    ] {
        let mut cfg = config();
        match name {
            "min_rms" => cfg.min_rms = value,
            "min_peak_margin" => cfg.min_peak_margin = value,
            _ => cfg.min_correlation = value,
        }
        assert!(run(&waveform(), source(), cfg).is_err(), "{name}");
    }
    let mut cfg = config();
    cfg.windows[2].frames = 200;
    assert!(run(&waveform(), source(), cfg).is_err());
    let mut value = serde_json::to_value(config()).unwrap();
    value["source_origin"] = json!(0);
    assert!(serde_json::from_value::<IndexedConfig>(value).is_err());
    let mut value = serde_json::to_value(config()).unwrap();
    value["windows"][0]["identity_index"] = json!(0);
    assert!(serde_json::from_value::<IndexedConfig>(value).is_err());
}

#[test]
fn a_weaker_second_valid_code_is_not_discarded_by_best_peak_selection() {
    let mut samples = waveform();
    let mut cfg = config();
    cfg.windows = vec![RecorderWindow {
        start_frame: 80,
        frames: 600,
    }];
    // Exact first marker plus a finite distorted but still section-matching
    // second code: two identities, not permission to choose the stronger one.
    for (offset, pair) in samples[1000..1128]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .enumerate()
    {
        let attenuation = if offset.is_multiple_of(2) { 0.4 } else { 1.0 };
        for sample in pair {
            *sample *= attenuation;
        }
    }
    let result = run(&samples, source(), cfg).unwrap();
    assert_eq!(result["results"][0]["status"], "ambiguous");
    assert_eq!(result["results"][0]["identity_index"], Value::Null);
}

#[test]
fn a_weaker_repeated_valid_onset_is_not_hidden_by_onset_peak_margin() {
    let mut samples = waveform();
    let repeated = samples[200..328].to_vec();
    samples[600..728].copy_from_slice(&repeated);
    for (offset, pair) in samples[600..728]
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .enumerate()
    {
        let attenuation = if offset.is_multiple_of(2) { 0.4 } else { 1.0 };
        for sample in pair {
            *sample *= attenuation;
        }
    }
    let mut cfg = config();
    cfg.windows = vec![RecorderWindow {
        start_frame: 80,
        frames: 320,
    }];
    let result = run(&samples, source(), cfg).unwrap();
    assert_eq!(result["results"][0]["status"], "ambiguous");
    assert_eq!(result["results"][0]["identity_index"], Value::Null);
}
