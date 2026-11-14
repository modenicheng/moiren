use crate::{cli, pcm};
use moiren_windows_audio::latency::MarkerGenerator;
use serde_json::{Value, json};
use std::io::Cursor;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moiren-pcm-rate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
    fn json(&self, name: &str, value: &Value) -> std::path::PathBuf {
        let path = self.path(name);
        std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn schedule() -> Value {
    json!({
        "schema_version":1, "sample_rate":48000, "reference_channel":0,
        "processed_channel":1, "marker_frames":4096, "marker_amplitude":0.1,
        "reference_search_frames":16, "max_delay_frames":64,
        "max_marker_frames":4200, "max_window_frames":4200, "max_markers":3,
        "max_pcm_frames":20000,
        "max_correlation_samples_per_marker":4200 * (17 + 65) * 9,
        "max_total_correlation_samples":4200 * (17 + 65) * 9 * 3,
        "max_total_window_frames":4200 * 3,
        "min_correlation":0.98, "min_peak_margin":0.15, "min_rms":0.0001,
        "rate_search": {"min_rate_ppm":-2000,"max_rate_ppm":2000,
            "rate_step_ppm":500,"max_hypotheses":9,
            "max_total_correlation_samples":4200 * 65 * 9},
        "windows":[{"start_frame":0},{"start_frame":5000},{"start_frame":10000}]
    })
}

fn raw_fixture(ppm: i32) -> Vec<u8> {
    let mut marker = vec![0.0; 4096];
    MarkerGenerator::new(4096, 0.1).unwrap().fill(&mut marker);
    let mut output = vec![[0.0_f32; 2]; 15000];
    for origin in [7, 5007, 10007] {
        for (offset, sample) in marker.iter().enumerate() {
            output[origin + offset][0] = *sample;
        }
        for (frame, channels) in output.iter_mut().enumerate().skip(origin + 23).take(4200) {
            let source = (frame - origin - 23) as f64 / (1.0 + f64::from(ppm) / 1_000_000.0);
            if source >= marker.len() as f64 {
                break;
            }
            let low = source.floor() as usize;
            let fraction = (source - low as f64) as f32;
            channels[1] = -(marker[low] * (1.0 - fraction)
                + marker.get(low + 1).copied().unwrap_or(0.0) * fraction)
                * 0.4;
        }
    }
    output
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect()
}

fn analyze(raw: Vec<u8>, schedule: Value) -> Value {
    let bytes = raw.len() as u64;
    serde_json::to_value(
        pcm::analyze(
            &mut Cursor::new(raw),
            bytes,
            serde_json::from_value(schedule).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn pcm_rate_search_detects_stretched_markers_and_preserves_raw_common_clock_delays() {
    for ppm in [-2000, 1000, 2000] {
        let raw = raw_fixture(ppm);
        let mut legacy = schedule();
        legacy.as_object_mut().unwrap().remove("rate_search");
        assert_eq!(analyze(raw.clone(), legacy)["failed_markers"], 3);
        let result = analyze(raw, schedule());
        assert_eq!(result["failed_markers"], 0, "{result}");
        assert_eq!(result["distribution"]["marker_count"], 3);
        for (index, entry) in result["results"].as_array().unwrap().iter().enumerate() {
            assert_eq!(entry["delay_frames"], 23);
            assert_eq!(entry["reference"]["absolute_frame"], index * 5000 + 7);
            assert_eq!(entry["processed"]["absolute_frame"], index * 5000 + 30);
            assert_eq!(
                entry["reference"]["rate_hypothesis"]["template_rate_ppm"],
                0
            );
            assert_eq!(
                entry["processed"]["rate_hypothesis"]["template_rate_ppm"],
                ppm
            );
            assert_eq!(
                entry["processed"]["rate_hypothesis"]["onset_resolution_frames"],
                1
            );
        }
        assert!(
            result["rate_search_scope"]
                .as_str()
                .unwrap()
                .contains("not_clock_calibration")
        );
        assert_eq!(result["qualification"], "not_evaluated");
        assert_eq!(result["schema_version"], 3);
    }
}

#[test]
fn pcm_legacy_output_omits_all_rate_search_fields() {
    let mut settings = schedule();
    settings.as_object_mut().unwrap().remove("rate_search");
    let output = analyze(raw_fixture(0), settings);
    assert_eq!(output["failed_markers"], 0);
    assert_eq!(output["schema_version"], 1);
    assert!(output.get("rate_search_scope").is_none());
    assert!(output["schedule"].get("rate_search").is_none());
    for result in output["results"].as_array().unwrap() {
        assert!(result["reference"].get("rate_hypothesis").is_none());
        assert!(result["processed"].get("rate_hypothesis").is_none());
    }
}

#[test]
fn pcm_rate_search_global_resampling_retains_accumulated_positive_and_negative_delay_drift() {
    let mut source = vec![0.0; 15000];
    let mut marker = vec![0.0; 4096];
    MarkerGenerator::new(4096, 0.1).unwrap().fill(&mut marker);
    for origin in [0, 5000, 10000] {
        source[origin..origin + 4096].copy_from_slice(&marker);
    }
    for ppm in [-2000, 2000] {
        let mut raw = Vec::with_capacity(15000 * 8);
        let ratio = 1.0 + f64::from(ppm) / 1_000_000.0;
        for (frame, reference) in source.iter().enumerate() {
            raw.extend(reference.to_le_bytes());
            let offset = (frame as f64 - 23.0) / ratio;
            let processed = if offset < 0.0 || offset >= source.len() as f64 {
                0.0
            } else {
                let low = offset.floor() as usize;
                let fraction = (offset - low as f64) as f32;
                (source[low] * (1.0 - fraction)
                    + source.get(low + 1).copied().unwrap_or(0.0) * fraction)
                    * -0.4
            };
            raw.extend(processed.to_le_bytes());
        }
        let mut settings = schedule();
        settings["trend"] = json!({"min_markers":3,"min_span_frames":10000});
        let value = analyze(raw, settings);
        assert_eq!(value["failed_markers"], 0, "{value}");
        for (index, result) in value["results"].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                result["delay_frames"],
                23 + index as i32 * ppm / 200,
                "accumulated source-clock drift must not be subtracted from delay"
            );
        }
        assert_eq!(value["distribution"]["marker_count"], 3);
        assert_eq!(value["schema_version"], 5);
        let fit = &value["trend"]["segments"][0]["fit"];
        assert!((fit["relative_delay_change_ppm"].as_f64().unwrap() - f64::from(ppm)).abs() < 1e-8);
        assert!((fit["intercept_at_first_reference_frames"].as_f64().unwrap() - 23.0).abs() < 1e-8);
    }
}

struct Unreadable;
impl std::io::Read for Unreadable {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("must refuse before reading PCM")
    }
}
impl std::io::Seek for Unreadable {
    fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
        panic!("must refuse before seeking PCM")
    }
}

#[test]
fn pcm_rate_search_preflights_combined_channel_work_windows_and_hypotheses() {
    let exact_pairs: i64 = (-2000_i64..=2000)
        .step_by(500)
        .map(|ppm| (4096_i64 * (1_000_000 + ppm) + 999_999) / 1_000_000)
        .sum::<i64>()
        * (18 + 66);
    let mut exact = schedule();
    exact["max_correlation_samples_per_marker"] = json!(exact_pairs);
    exact["max_total_correlation_samples"] = json!(exact_pairs * 3);
    exact["max_total_window_frames"] = json!(4185 * 3);
    assert_eq!(
        analyze(raw_fixture(2000), exact.clone())["failed_markers"],
        0
    );
    for (key, value) in [
        ("max_correlation_samples_per_marker", json!(exact_pairs - 1)),
        ("max_total_correlation_samples", json!(exact_pairs * 3 - 1)),
        ("max_total_window_frames", json!(4185 * 3 - 1)),
        ("max_window_frames", json!(4184)),
        ("max_marker_frames", json!(4104)),
    ] {
        let mut settings = exact.clone();
        settings[key] = value;
        assert!(
            pcm::analyze(
                &mut Unreadable,
                15000 * 8,
                serde_json::from_value(settings).unwrap()
            )
            .is_err(),
            "{key}"
        );
    }
    let mut settings = exact;
    settings["rate_search"]["max_hypotheses"] = json!(8);
    assert!(
        pcm::analyze(
            &mut Unreadable,
            15000 * 8,
            serde_json::from_value(settings).unwrap()
        )
        .is_err()
    );
}

#[test]
fn pcm_rate_search_full_warped_observation_and_partial_failures_are_retained() {
    let mut settings = schedule();
    settings["windows"] = json!([{"start_frame":0}]);
    let mut raw = raw_fixture(2000);
    raw.truncate(4185 * 8);
    assert_eq!(analyze(raw.clone(), settings.clone())["failed_markers"], 0);
    raw.truncate(4184 * 8);
    let value = analyze(raw, settings);
    assert_eq!(value["failed_markers"], 1);
    assert!(
        value["results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("insufficient observation")
    );
    let mut raw = raw_fixture(1000);
    for frame in raw[5000 * 8..9200 * 8].as_chunks_mut::<8>().0 {
        frame[4..].fill(0);
    }
    let value = analyze(raw, schedule());
    assert_eq!(value["failed_markers"], 1);
    assert_eq!(value["results"].as_array().unwrap().len(), 3);
    assert_eq!(value["distribution"]["marker_count"], 2);
    assert!(value["results"][1]["processed"].is_null());
}

#[test]
fn pcm_rate_search_live_loader_analyzes_imported_pcm_without_native_or_qualification() {
    let fixture = Fixture::new();
    let raw = fixture.path("capture.raw");
    std::fs::write(&raw, raw_fixture(-2000)).unwrap();
    let settings = fixture.json("schedule.json", &schedule());
    let directory = fixture.path("run");
    assert_eq!(
        cli::run(vec![
            "live-process-loopback".into(),
            "--dry-run".into(),
            "--source-pid".into(),
            "1".into(),
            "--output".into(),
            "unopened".into(),
            "--artifact-dir".into(),
            directory.to_string_lossy().into_owned(),
            "--measurement-pcm".into(),
            raw.to_string_lossy().into_owned(),
            "--schedule".into(),
            settings.to_string_lossy().into_owned()
        ])
        .unwrap(),
        1
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(directory.join("report.json")).unwrap()).unwrap();
    assert_eq!(
        report["artifacts"]["measurement_pcm_status"],
        "externally_supplied_and_analyzed"
    );
    assert_eq!(report["qualified"], false);
    assert_eq!(report["qualification"]["steady_state"], false);
    assert_eq!(report["qualification"]["controlled_load"], false);
    assert_eq!(report["qualification"]["distribution"]["marker_count"], 3);
    assert_eq!(report["rate_search_measurement"]["schema_version"], 3);
    assert_eq!(
        report["rate_search_measurement"]["results"][0]["processed"]["rate_hypothesis"]["template_rate_ppm"],
        -2000
    );
    assert!(report["native"]["capture"].is_null());
}

#[test]
fn pcm_rate_and_trend_live_import_retains_one_complete_analysis_not_duplicate_reports() {
    let fixture = Fixture::new();
    let raw = fixture.path("capture.raw");
    std::fs::write(&raw, raw_fixture(2000)).unwrap();
    let mut config = schedule();
    config["trend"] = json!({"min_markers":3,"min_span_frames":10000});
    let settings = fixture.json("schedule.json", &config);
    let directory = fixture.path("run");
    assert_eq!(
        cli::run(vec![
            "live-process-loopback".into(),
            "--dry-run".into(),
            "--source-pid".into(),
            "1".into(),
            "--output".into(),
            "unopened".into(),
            "--artifact-dir".into(),
            directory.to_string_lossy().into_owned(),
            "--measurement-pcm".into(),
            raw.to_string_lossy().into_owned(),
            "--schedule".into(),
            settings.to_string_lossy().into_owned()
        ])
        .unwrap(),
        1
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(directory.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["qualified"], false);
    let analysis = &report["rate_search_measurement"];
    assert_eq!(analysis["schema_version"], 5);
    assert_eq!(analysis["trend"]["segments"][0]["marker_count"], 3);
    assert_eq!(
        analysis["results"][0]["processed"]["rate_hypothesis"]["template_rate_ppm"],
        2000
    );
    assert!(report.get("trend_measurement").is_none());
}

#[test]
fn pcm_rate_search_cli_retains_metadata_tail_reset_without_a_latency_pass() {
    let fixture = Fixture::new();
    let raw = raw_fixture(2000);
    let raw_path = fixture.path("capture.raw");
    std::fs::write(&raw_path, &raw).unwrap();
    let settings = fixture.json("schedule.json", &schedule());
    let mut packets = String::new();
    for frame in (0..15000).step_by(8) {
        // Legacy window ends at4176; the admitted warped tail ends at4185.
        let packet = json!({"packet_sequence":frame/8+1,"stored_start_frame":frame,
            "device_position_frames":frame+100, "qpc_100ns":1_000_000+frame*10_000_000_u64/48000,
            "frames":8,"flags":if frame==4184 {1} else {0}});
        packets.push_str(&format!("{packet}\n"));
    }
    let metadata = fixture.path("packets.jsonl");
    std::fs::write(&metadata, packets).unwrap();
    let output = fixture.path("analysis.json");
    let args = vec![
        "analyze-pcm".into(),
        "--pcm".into(),
        raw_path.to_string_lossy().into_owned(),
        "--schedule".into(),
        settings.to_string_lossy().into_owned(),
        "--packets".into(),
        metadata.to_string_lossy().into_owned(),
        "--max-packets".into(),
        "2000".into(),
        "--max-input-bytes".into(),
        "1048576".into(),
        "--output".into(),
        output.to_string_lossy().into_owned(),
    ];
    assert_eq!(cli::run(args).unwrap(), 1);
    let value: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(
        value["results"][0]["processed"]["rate_hypothesis"]["template_rate_ppm"],
        2000
    );
    assert!(value["results"][0]["delay_frames"].is_null());
    assert!(
        value["results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("packet")
    );
    assert_eq!(value["failed_markers"], 1);
    assert_eq!(value["distribution"]["marker_count"], 2);
    assert_eq!(value["schema_version"], 4);
}
