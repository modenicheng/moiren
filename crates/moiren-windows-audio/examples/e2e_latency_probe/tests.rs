use super::{cli, pcm};
use moiren_windows_audio::latency::MarkerGenerator;
use moiren_windows_audio::latency::qualification::QualificationReport;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Cursor, Seek, SeekFrom},
    path::{Path, PathBuf},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moiren-e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn json(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn schedule() -> Value {
    json!({
        "schema_version": 1, "sample_rate": 48000,
        "reference_channel": 0, "processed_channel": 1,
        "marker_frames": 255, "marker_amplitude": 0.1,
        "reference_search_frames": 16, "max_delay_frames": 128,
        "max_marker_frames": 255, "max_window_frames": 399,
        "max_markers": 8, "max_pcm_frames": 48000_u64 * 7200,
        "max_correlation_samples_per_marker": 255 * (17 + 129),
        "max_total_correlation_samples": 255 * (17 + 129) * 8,
        "max_total_window_frames": 399 * 8,
        "min_correlation": 0.85, "min_peak_margin": 0.15, "min_rms": 0.0001,
        "windows": [{"start_frame": 0}, {"start_frame": 512}, {"start_frame": 1024}]
    })
}

fn raw_fixture(missing: bool) -> Vec<u8> {
    let mut marker = vec![0.0; 255];
    MarkerGenerator::new(255, 0.1).unwrap().fill(&mut marker);
    let mut frames = vec![[0.0_f32; 2]; 1600];
    for (index, delay) in [0, 64, 128].into_iter().enumerate() {
        let origin = index * 512 + 7;
        for (offset, sample) in marker.iter().enumerate() {
            frames[origin + offset][0] = *sample;
            if !missing || index != 1 {
                frames[origin + delay + offset][1] = -*sample * 0.4;
            }
        }
    }
    frames
        .iter()
        .flat_map(|frame| frame.iter().flat_map(|sample| sample.to_le_bytes()))
        .collect()
}

fn analyze(raw: Vec<u8>, schedule: Value) -> anyhow::Result<pcm::PcmAnalysis> {
    let length = raw.len() as u64;
    pcm::analyze(
        &mut Cursor::new(raw),
        length,
        serde_json::from_value(schedule).unwrap(),
    )
}

#[test]
fn stereo_pcm_reports_delays_relative_to_detected_reference_and_actual_percentiles() {
    let output = analyze(raw_fixture(false), schedule()).unwrap();
    let value = serde_json::to_value(output).unwrap();
    assert_eq!(value["failed_markers"], 0);
    assert_eq!(value["results"].as_array().unwrap().len(), 3);
    for (index, delay) in [0, 64, 128].into_iter().enumerate() {
        assert_eq!(value["results"][index]["delay_frames"], delay);
        assert_eq!(
            value["results"][index]["reference"]["absolute_frame"],
            index * 512 + 7
        );
    }
    assert!((value["distribution"]["p50_ms"].as_f64().unwrap() - 64.0 / 48.0).abs() < 1e-10);
    assert!((value["distribution"]["p95_ms"].as_f64().unwrap() - 128.0 / 48.0).abs() < 1e-10);
    assert_eq!(value["qualification"], "not_evaluated");
    assert!(!value["gaps"].as_array().unwrap().is_empty());
}

#[test]
fn missing_clipped_nonfinite_and_truncated_windows_remain_in_results() {
    let output = serde_json::to_value(analyze(raw_fixture(true), schedule()).unwrap()).unwrap();
    assert_eq!(output["failed_markers"], 1);
    assert_eq!(output["results"].as_array().unwrap().len(), 3);
    assert!(
        output["results"][1]["error"]
            .as_str()
            .unwrap()
            .contains("processed")
    );
    assert_eq!(output["distribution"]["marker_count"], 2);
    for sample in [1.0_f32, f32::NAN] {
        let mut raw = raw_fixture(false);
        raw[8 * (512 + 10) + 4..8 * (512 + 10) + 8].copy_from_slice(&sample.to_le_bytes());
        let output = serde_json::to_value(analyze(raw, schedule()).unwrap()).unwrap();
        assert_eq!(output["failed_markers"], 1);
    }
    let mut raw = raw_fixture(false);
    raw.truncate(8 * 1100);
    let output = serde_json::to_value(analyze(raw, schedule()).unwrap()).unwrap();
    assert_eq!(output["failed_markers"], 1);
    assert!(
        output["results"][2]["error"]
            .as_str()
            .unwrap()
            .contains("window")
    );
}

#[test]
fn invalid_schedule_channels_rates_limits_order_and_arithmetic_are_refused() {
    for (key, value) in [
        ("sample_rate", json!(0)),
        ("reference_channel", json!(1)),
        ("processed_channel", json!(2)),
        ("marker_frames", json!(256)),
        ("max_window_frames", json!(398)),
        ("max_markers", json!(2)),
        ("max_total_correlation_samples", json!(1)),
        ("max_total_window_frames", json!(1)),
        ("max_correlation_samples_per_marker", json!(1)),
        ("max_pcm_frames", json!(10)),
        ("max_delay_frames", json!(u64::MAX)),
        ("min_correlation", json!(0)),
        ("marker_amplitude", json!(1.0)),
        ("schema_version", json!(2)),
    ] {
        let mut settings = schedule();
        settings[key] = value;
        assert!(
            analyze(raw_fixture(false), settings).is_err(),
            "accepted {key}"
        );
    }
    let mut settings = schedule();
    settings["windows"][1]["start_frame"] = json!(1);
    assert!(analyze(raw_fixture(false), settings).is_err());
    let mut settings = schedule();
    settings["windows"] = json!([]);
    assert!(analyze(raw_fixture(false), settings).is_err());
    assert!(analyze(vec![0; 9], schedule()).is_err());
}

struct WindowReader {
    bytes: Vec<u8>,
    position: u64,
    length: u64,
    reads: u64,
}
impl std::io::Read for WindowReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        assert!(
            self.position < self.bytes.len() as u64,
            "attempted whole-file read"
        );
        let read = (&self.bytes[self.position as usize..]).read(output)?;
        self.position += read as u64;
        self.reads += read as u64;
        Ok(read)
    }
}
impl Seek for WindowReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.position = match position {
            SeekFrom::Start(value) => value,
            SeekFrom::End(value) => self.length.checked_add_signed(value).unwrap(),
            SeekFrom::Current(value) => self.position.checked_add_signed(value).unwrap(),
        };
        Ok(self.position)
    }
}

#[test]
fn a_two_hour_capture_reads_only_admitted_windows() {
    let length = 48000_u64 * 7200 * 8;
    let mut reader = WindowReader {
        bytes: raw_fixture(false),
        position: 0,
        length,
        reads: 0,
    };
    let output = pcm::analyze(
        &mut reader,
        length,
        serde_json::from_value(schedule()).unwrap(),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(output).unwrap()["failed_markers"], 0);
    assert_eq!(reader.reads, 399 * 8 * 3);
}

#[test]
fn channel_mapping_is_explicit_and_ambiguous_or_negative_delays_are_retained_as_failures() {
    let mut swapped = raw_fixture(false);
    for frame in swapped.as_chunks_mut::<8>().0 {
        for index in 0..4 {
            frame.swap(index, index + 4);
        }
    }
    let mut settings = schedule();
    settings["reference_channel"] = json!(1);
    settings["processed_channel"] = json!(0);
    assert_eq!(
        serde_json::to_value(analyze(swapped, settings).unwrap()).unwrap()["failed_markers"],
        0
    );

    let mut marker = vec![0.0; 255];
    MarkerGenerator::new(255, 0.1).unwrap().fill(&mut marker);
    let mut raw = raw_fixture(false);
    // Replace the final processed marker with one preceding the observed reference.
    for frame in raw[8 * 1024..].as_chunks_mut::<8>().0 {
        frame[4..].fill(0);
    }
    for (index, sample) in marker.iter().enumerate() {
        raw[(1024 + index) * 8 + 4..(1024 + index) * 8 + 8].copy_from_slice(&sample.to_le_bytes());
    }
    let value = serde_json::to_value(analyze(raw, schedule()).unwrap()).unwrap();
    assert_eq!(value["failed_markers"], 1);
    assert!(
        value["results"][2]["error"]
            .as_str()
            .unwrap()
            .contains("processed")
    );

    let mut settings = schedule();
    settings["windows"] = json!([{"start_frame": 0}]);
    settings["max_delay_frames"] = json!(600);
    settings["max_window_frames"] = json!(871);
    settings["max_correlation_samples_per_marker"] = json!(255 * (17 + 601));
    settings["max_total_correlation_samples"] = json!(255 * (17 + 601));
    settings["max_total_window_frames"] = json!(871);
    let mut raw = vec![0; 8 * 900];
    for (index, sample) in marker.iter().enumerate() {
        raw[(7 + index) * 8..(7 + index) * 8 + 4].copy_from_slice(&sample.to_le_bytes());
        for delay in [0, 512] {
            raw[(7 + delay + index) * 8 + 4..(7 + delay + index) * 8 + 8]
                .copy_from_slice(&sample.to_le_bytes());
        }
    }
    let value = serde_json::to_value(analyze(raw, settings).unwrap()).unwrap();
    assert_eq!(value["failed_markers"], 1);
    assert!(
        value["results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("ambiguous")
    );
}

fn run_arguments(command: &str, inputs: &[(&str, &Path)], output: &Path) -> Vec<String> {
    let mut args = vec![command.into()];
    for (flag, path) in inputs {
        args.extend([flag.to_string(), path.to_string_lossy().into_owned()]);
    }
    args.extend([
        "--max-input-bytes".into(),
        "1048576".into(),
        "--output".into(),
        output.to_string_lossy().into_owned(),
    ]);
    args
}

#[test]
fn cli_retains_failed_pcm_markers_and_returns_nonzero_without_overwriting_evidence() {
    let fixture = Fixture::new();
    let raw = fixture.path("capture f32.raw");
    fs::write(&raw, raw_fixture(true)).unwrap();
    let settings = fixture.json("schedule.json", &schedule());
    let output = fixture.path("analysis.json");
    let args = run_arguments(
        "analyze-pcm",
        &[("--pcm", &raw), ("--schedule", &settings)],
        &output,
    );
    assert_eq!(cli::run(args.clone()).unwrap(), 1);
    let retained = fs::read(&output).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&retained).unwrap()["failed_markers"],
        1
    );
    assert!(cli::run(args).is_err());
    assert_eq!(fs::read(&output).unwrap(), retained);
}

#[test]
fn json_byte_caps_unknown_and_duplicate_options_are_enforced() {
    let fixture = Fixture::new();
    let raw = fixture.path("capture.raw");
    fs::write(&raw, raw_fixture(false)).unwrap();
    let settings = fixture.json("schedule.json", &schedule());
    let output = fixture.path("out.json");
    let mut args = run_arguments(
        "analyze-pcm",
        &[("--pcm", &raw), ("--schedule", &settings)],
        &output,
    );
    let limit = args
        .iter()
        .position(|value| value == "--max-input-bytes")
        .unwrap()
        + 1;
    args[limit] = "8".into();
    assert!(cli::run(args).is_err());
    for extra in [vec!["--unknown", "1"], vec!["--max-input-bytes", "10"]] {
        let mut args = run_arguments(
            "analyze-pcm",
            &[("--pcm", &raw), ("--schedule", &settings)],
            &output,
        );
        args.extend(extra.into_iter().map(str::to_string));
        assert!(cli::run(args).is_err());
    }
}

fn packet_fixture() -> Vec<Value> {
    (0..50_u64)
        .map(|index| {
            json!({
                "packet_sequence": index + 1, "flags": 0,
                "device_position_frames": index * 32,
                "qpc_100ns": 1_000_000 + index * 32 * 10_000_000 / 48000,
                "frames": 32, "stored_start_frame": index * 32
            })
        })
        .collect()
}

fn packet_analysis(raw: Vec<u8>, packets: &[Value]) -> (i32, Value) {
    let fixture = Fixture::new();
    let raw_path = fixture.path("capture.f32le");
    fs::write(&raw_path, raw).unwrap();
    let settings = fixture.json("schedule.json", &schedule());
    let metadata = fixture.path("packets.jsonl");
    let records: String = packets.iter().map(|packet| format!("{packet}\n")).collect();
    fs::write(&metadata, records).unwrap();
    let output = fixture.path("analysis.json");
    let mut args = run_arguments(
        "analyze-pcm",
        &[
            ("--pcm", &raw_path),
            ("--schedule", &settings),
            ("--packets", &metadata),
        ],
        &output,
    );
    args.extend(["--max-packets".into(), "50".into()]);
    let code = cli::run(args).unwrap();
    (
        code,
        serde_json::from_slice(&fs::read(output).unwrap()).unwrap(),
    )
}

#[test]
fn packet_analysis_known_delay_and_drift_stay_diagnostic_with_legacy_unchanged() {
    let (code, value) = packet_analysis(raw_fixture(false), &packet_fixture());
    assert_eq!(code, 0);
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["qualification"], "not_evaluated");
    assert_eq!(value["packet_evidence"]["native_epoch"], "not_recorded");
    assert_eq!(value["distribution_scope"], "successful_windows_only");
    for (index, delay) in [0, 64, 128].into_iter().enumerate() {
        assert_eq!(value["results"][index]["delay_frames"], delay);
    }
    assert_eq!(value["distribution"]["marker_count"], 3);
    let legacy = serde_json::to_value(analyze(raw_fixture(false), schedule()).unwrap()).unwrap();
    assert_eq!(legacy["schema_version"], 1);
    assert!(legacy.get("packet_evidence").is_none());
}

#[test]
fn packet_analysis_retains_failures_for_loss_reorder_and_epoch_like_resets() {
    for fault in [
        "sequence_gap",
        "sequence_repeat",
        "position_repeat",
        "position_rewind",
        "qpc_repeat",
        "qpc_rewind",
        "discontinuity",
        "timestamp_error",
    ] {
        let mut packets = packet_fixture();
        let index = 20;
        match fault {
            "sequence_gap" => {
                for packet in &mut packets[index..] {
                    packet["packet_sequence"] =
                        json!(packet["packet_sequence"].as_u64().unwrap() + 1);
                }
            }
            "sequence_repeat" => {
                packets[index]["packet_sequence"] = packets[index - 1]["packet_sequence"].clone()
            }
            "position_repeat" => {
                packets[index]["device_position_frames"] =
                    packets[index - 1]["device_position_frames"].clone()
            }
            "position_rewind" => packets[index]["device_position_frames"] = json!(0),
            "qpc_repeat" => packets[index]["qpc_100ns"] = packets[index - 1]["qpc_100ns"].clone(),
            "qpc_rewind" => packets[index]["qpc_100ns"] = json!(1),
            "discontinuity" => packets[index]["flags"] = json!(1),
            "timestamp_error" => packets[index]["flags"] = json!(4),
            _ => unreachable!(),
        }
        let (code, value) = packet_analysis(raw_fixture(false), &packets);
        assert_eq!(code, 1, "accepted {fault}");
        assert_eq!(value["failed_markers"], 1, "{fault}: {value}");
        assert!(value["results"][1]["delay_frames"].is_null(), "{fault}");
        assert!(
            value["results"][1]["error"]
                .as_str()
                .unwrap()
                .contains("packet")
        );
        assert_eq!(value["distribution"]["marker_count"], 2);
        assert!(
            !value["packet_evidence"]["observations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn packet_analysis_detects_actual_dropped_pcm_and_metadata_coverage_failures() {
    let mut packets = packet_fixture();
    packets.remove(20);
    for packet in &mut packets[20..] {
        packet["stored_start_frame"] = json!(packet["stored_start_frame"].as_u64().unwrap() - 32);
    }
    let mut raw = raw_fixture(false);
    raw.drain(640 * 8..672 * 8);
    let (code, value) = packet_analysis(raw, &packets);
    assert_eq!(code, 1);
    assert!(value["results"][1]["delay_frames"].is_null());
    assert_eq!(value["results"].as_array().unwrap().len(), 3);
    for fault in ["missing_record", "overlap", "tail"] {
        let mut packets = packet_fixture();
        match fault {
            "missing_record" => {
                packets.remove(20);
            }
            "overlap" => packets[20]["stored_start_frame"] = json!(608),
            "tail" => {
                packets.pop();
            }
            _ => unreachable!(),
        }
        let (code, value) = packet_analysis(raw_fixture(false), &packets);
        assert_eq!(code, 1, "accepted {fault}");
        assert_eq!(value["packet_evidence"]["coverage_complete"], false);
    }
}

#[test]
fn packet_analysis_silence_unknown_timestamps_and_startup_are_not_qualified() {
    let (code, value) = packet_analysis(vec![0; 1600 * 8], &packet_fixture());
    assert_eq!(code, 1);
    assert_eq!(value["failed_markers"], 3);
    assert!(value["distribution"].is_null());
    let mut packets = packet_fixture();
    for packet in &mut packets {
        packet["qpc_100ns"] = json!(0);
        packet["device_position_frames"] = json!(0);
    }
    let (code, value) = packet_analysis(raw_fixture(false), &packets);
    assert_eq!(code, 1);
    assert_eq!(value["packet_evidence"]["unknown_timestamp_packets"], 50);
    assert_eq!(
        value["packet_evidence"]["timestamp_provenance"],
        "unverified_native_packet_fields_not_hardware_or_acoustics"
    );
    let mut packets = packet_fixture();
    packets[0]["flags"] = json!(1);
    let (_, value) = packet_analysis(raw_fixture(false), &packets);
    assert!(
        !value["packet_evidence"]["startup"]["issues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        value["packet_evidence"]["observations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(value["failed_markers"], 1);
    assert_eq!(value["qualification"], "not_evaluated");
}

fn packet_jsonl(packets: &[Value]) -> Vec<u8> {
    packets
        .iter()
        .flat_map(|packet| format!("{packet}\n").into_bytes())
        .collect()
}

#[test]
fn packet_parser_enforces_sink_shape_byte_packet_line_and_arithmetic_bounds() {
    let raw = packet_jsonl(&packet_fixture());
    let parse = |raw: Vec<u8>, limit, cap| {
        pcm::PacketTimeline::read(Cursor::new(raw), limit, cap, 1600, 48000)
    };
    assert!(parse(raw.clone(), raw.len() as u64, 50).is_ok());
    assert!(parse(raw.clone(), raw.len() as u64 - 1, 50).is_err());
    assert!(parse(raw.clone(), raw.len() as u64, 49).is_err());
    assert!(parse(raw.clone(), 0, 50).is_err());
    assert!(parse(raw.clone(), 256 * 1024 * 1024 + 1, 50).is_err());
    assert!(parse(raw.clone(), 1048576, 0).is_err());
    assert!(parse(raw.clone(), 1048576, 1_000_001).is_err());
    for invalid in [
        vec![],
        b"\n".to_vec(),
        b"{}\n".to_vec(),
        b"{} {}\n".to_vec(),
        raw[..raw.len() - 1].to_vec(),
        vec![b' '; 1025],
    ] {
        assert!(parse(invalid, 1048576, 50).is_err());
    }
    for fault in [
        "unknown_epoch",
        "missing_field",
        "zero_frames",
        "oversized_frames",
        "negative_qpc",
        "float_position",
        "zero_sequence",
        "stored_overflow",
        "duplicate_key",
    ] {
        let mut packet = packet_fixture().remove(0);
        match fault {
            "unknown_epoch" => packet["epoch"] = json!(1),
            "missing_field" => {
                packet.as_object_mut().unwrap().remove("qpc_100ns");
            }
            "zero_frames" => packet["frames"] = json!(0),
            "oversized_frames" => packet["frames"] = json!(u64::MAX),
            "negative_qpc" => packet["qpc_100ns"] = json!(-1),
            "float_position" => packet["device_position_frames"] = json!(1.5),
            "zero_sequence" => packet["packet_sequence"] = json!(0),
            "stored_overflow" => packet["stored_start_frame"] = json!(u64::MAX),
            "duplicate_key" => {}
            _ => unreachable!(),
        }
        let mut encoded = packet_jsonl(&[packet]);
        if fault == "duplicate_key" {
            encoded = String::from_utf8(encoded)
                .unwrap()
                .replacen('{', "{\"frames\":32,", 1)
                .into_bytes();
        }
        assert!(parse(encoded, 1048576, 50).is_err(), "accepted {fault}");
    }
}

#[test]
fn packet_parser_stops_a_growing_stream_and_an_overlong_line_before_allocation() {
    struct CountingReader {
        reads: usize,
        byte: u8,
    }
    impl std::io::Read for CountingReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.reads += output.len();
            output.fill(self.byte);
            Ok(output.len())
        }
    }
    let mut reader = CountingReader {
        reads: 0,
        byte: b' ',
    };
    assert!(pcm::PacketTimeline::read(&mut reader, 20, 50, 1600, 48000).is_err());
    assert_eq!(reader.reads, 21);
    let mut reader = CountingReader {
        reads: 0,
        byte: b' ',
    };
    assert!(pcm::PacketTimeline::read(&mut reader, 1048576, 50, 1600, 48000).is_err());
    assert!(reader.reads <= 8192, "read an unbounded line");
}

#[test]
fn packet_analysis_all_zero_positions_are_unknown_and_small_clock_drift_is_not_a_reset() {
    let mut packets = packet_fixture();
    for (index, packet) in packets.iter_mut().enumerate() {
        packet["device_position_frames"] = json!(0);
        // 250 ppm packet clock drift is not a delay estimate or an epoch.
        packet["qpc_100ns"] = json!(1_000_000 + index as u64 * 32 * 10_002_500 / 48000);
    }
    let (code, value) = packet_analysis(raw_fixture(false), &packets);
    assert_eq!(code, 0);
    assert_eq!(value["packet_evidence"]["unknown_position_packets"], 50);
    assert_eq!(value["packet_evidence"]["unknown_timestamp_packets"], 0);
    assert_eq!(value["packet_evidence"]["native_epoch"], "not_recorded");
    assert_eq!(value["qualification"], "not_evaluated");
}

#[test]
fn packet_analysis_fixed_delay_fixture_does_not_replace_sample_alignment_with_packet_clocks() {
    let mut marker = vec![0.0; 255];
    MarkerGenerator::new(255, 0.1).unwrap().fill(&mut marker);
    let mut raw = vec![0; 1600 * 8];
    for window in [0, 512, 1024] {
        for (index, sample) in marker.iter().enumerate() {
            let reference = (window + 7 + index) * 8;
            let processed = (window + 7 + 64 + index) * 8 + 4;
            raw[reference..reference + 4].copy_from_slice(&sample.to_le_bytes());
            raw[processed..processed + 4].copy_from_slice(&sample.to_le_bytes());
        }
    }
    let (code, value) = packet_analysis(raw, &packet_fixture());
    assert_eq!(code, 0);
    for result in value["results"].as_array().unwrap() {
        assert_eq!(result["delay_frames"], 64);
    }
    for percentile in ["p50_ms", "p95_ms", "p99_ms", "max_ms"] {
        assert!((value["distribution"][percentile].as_f64().unwrap() - 64.0 / 48.0).abs() < 1e-10);
    }
}

#[test]
fn packet_analysis_observes_faults_outside_windows_without_hiding_marker_failures() {
    let mut packets = packet_fixture();
    packets[48]["flags"] = json!(1);
    let (code, value) = packet_analysis(raw_fixture(false), &packets);
    assert_eq!(code, 1);
    assert_eq!(value["failed_markers"], 0);
    assert_eq!(value["distribution"]["marker_count"], 3);
    assert_eq!(
        value["packet_evidence"]["observations"][0]["stored_start_frame"],
        1536
    );
    packets[20]["flags"] = json!(1);
    let (_, value) = packet_analysis(raw_fixture(true), &packets);
    let error = value["results"][1]["error"].as_str().unwrap();
    assert!(
        error.contains("processed") && error.contains("packet"),
        "{error}"
    );
}

#[test]
fn packet_analysis_rejects_silent_flags_unknown_flags_overflow_and_missing_prefix() {
    for fault in ["silent", "unknown_flags", "position_overflow", "prefix"] {
        let mut packets = packet_fixture();
        match fault {
            "silent" => packets[20]["flags"] = json!(2),
            "unknown_flags" => packets[20]["flags"] = json!(8),
            "position_overflow" => packets[20]["device_position_frames"] = json!(u64::MAX),
            "prefix" => {
                for packet in &mut packets {
                    packet["packet_sequence"] =
                        json!(packet["packet_sequence"].as_u64().unwrap() + 1);
                }
            }
            _ => unreachable!(),
        }
        let (code, value) = packet_analysis(raw_fixture(false), &packets);
        assert_eq!(code, 1, "{fault}");
        assert_eq!(value["failed_markers"], 1, "{fault}");
    }
}

#[test]
fn packet_cli_requires_a_paired_packet_cap_and_preserves_create_new_output() {
    let fixture = Fixture::new();
    let raw = fixture.path("capture.raw");
    fs::write(&raw, raw_fixture(false)).unwrap();
    let settings = fixture.json("schedule.json", &schedule());
    let packets = fixture.path("packets.jsonl");
    fs::write(&packets, packet_jsonl(&packet_fixture())).unwrap();
    let output = fixture.path("analysis.json");
    let args = run_arguments(
        "analyze-pcm",
        &[
            ("--pcm", &raw),
            ("--schedule", &settings),
            ("--packets", &packets),
        ],
        &output,
    );
    assert!(cli::run(args.clone()).is_err());
    let mut args = args;
    args.extend(["--max-packets".into(), "50".into()]);
    assert_eq!(cli::run(args.clone()).unwrap(), 0);
    let retained = fs::read(&output).unwrap();
    assert!(cli::run(args).is_err());
    assert_eq!(fs::read(&output).unwrap(), retained);
    let mut args = run_arguments(
        "analyze-pcm",
        &[("--pcm", &raw), ("--schedule", &settings)],
        &fixture.path("orphan-cap.json"),
    );
    args.extend(["--max-packets".into(), "50".into()]);
    assert!(cli::run(args).is_err());
}

#[test]
fn packet_analysis_qpc_forward_jump_is_not_silently_treated_as_a_continuous_capture() {
    let mut packets = packet_fixture();
    for packet in &mut packets[20..] {
        packet["qpc_100ns"] = json!(packet["qpc_100ns"].as_u64().unwrap() + 10_000_000);
    }
    let (code, value) = packet_analysis(raw_fixture(false), &packets);
    assert_eq!(code, 1);
    assert_eq!(value["failed_markers"], 1);
    assert!(value["results"][1]["delay_frames"].is_null());
    assert_eq!(value["results"][2]["delay_frames"], 128);
    assert_eq!(
        value["packet_evidence"]["observations"][0]["qpc_100ns"],
        packets[20]["qpc_100ns"]
    );
}

#[test]
fn packet_analysis_persistent_capture_reset_invalidates_crossing_not_later_clean_windows() {
    let mut packets = packet_fixture();
    for (index, packet) in packets[20..].iter_mut().enumerate() {
        packet["packet_sequence"] = json!(index + 1);
        packet["device_position_frames"] = json!(index * 32);
        packet["qpc_100ns"] = json!(1 + index as u64 * 32 * 10_000_000 / 48000);
    }
    let (code, value) = packet_analysis(raw_fixture(false), &packets);
    assert_eq!(code, 1);
    assert_eq!(value["failed_markers"], 1);
    assert_eq!(value["results"][0]["delay_frames"], 0);
    assert!(value["results"][1]["delay_frames"].is_null());
    assert_eq!(value["results"][2]["delay_frames"], 128);
    assert_eq!(value["packet_evidence"]["native_epoch"], "not_recorded");
}

#[test]
fn packet_analysis_tracks_small_resampled_clock_drift_or_retains_an_unresolvable_window() {
    let mut marker = vec![0.0; 255];
    MarkerGenerator::new(255, 0.1).unwrap().fill(&mut marker);
    let encode = |ratio: f64| {
        let mut frames = vec![[0.0_f32; 2]; 1600];
        for window in [0, 512, 1024] {
            let origin = window + 7;
            for (offset, sample) in marker.iter().enumerate() {
                frames[origin + offset][0] = *sample;
            }
            for (frame, samples) in frames.iter_mut().enumerate() {
                let offset = (frame as f64 - 64.0) / ratio - origin as f64;
                if (0.0..254.0).contains(&offset) {
                    let low = offset.floor() as usize;
                    let fraction = (offset - low as f64) as f32;
                    samples[1] = marker[low] * (1.0 - fraction) + marker[low + 1] * fraction;
                }
            }
        }
        frames
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect()
    };
    let (code, value) = packet_analysis(encode(1.0002), &packet_fixture());
    assert_eq!(code, 0);
    for result in value["results"].as_array().unwrap() {
        assert_eq!(result["delay_frames"], 64);
    }
    let (code, value) = packet_analysis(encode(1.02), &packet_fixture());
    assert_eq!(code, 1);
    assert_eq!(value["failed_markers"], 3);
    assert!(value["distribution"].is_null());
}

#[test]
fn live_vad_dry_run_retains_stable_identity_and_stays_unqualified_without_calibration() {
    let fixture = Fixture::new();
    let directory = fixture.path("vad-report");
    let code = cli::run(vec![
        "live-vad".into(),
        "--stable-endpoint-id".into(),
        "stable-vad-001".into(),
        "--name".into(),
        "Moiren VAD Probe".into(),
        "--output".into(),
        "physical-output".into(),
        "--artifact-dir".into(),
        directory.to_string_lossy().into_owned(),
        "--seconds".into(),
        "1".into(),
        "--dry-run".into(),
    ])
    .unwrap();
    assert_eq!(code, 1);
    let report: Value =
        serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
    let configuration = &report["qualification"]["configuration"];
    assert_eq!(report["mode"], "live_vad");
    assert_eq!(report["status"], "unqualified");
    assert_eq!(report["qualified"], false);
    assert_eq!(report["source"]["source_kind"], "vad_playback");
    assert_eq!(report["source"]["stable_endpoint_id"], "stable-vad-001");
    assert_eq!(report["source"]["requested_name"], "Moiren VAD Probe");
    assert_eq!(configuration["source_kind"], "vad_playback");
    assert_eq!(
        configuration["source_identity"],
        "stable-endpoint:stable-vad-001"
    );
    assert_eq!(
        configuration["source_details"]["stable_endpoint_id"],
        "stable-vad-001"
    );
    assert!(report["qualification"]["calibration"].is_null());
    assert!(report["gaps"].as_array().unwrap().iter().any(|gap| {
        gap.as_str()
            .is_some_and(|text| text.contains("calibration provenance"))
    }));
}

#[test]
fn live_process_loopback_uses_process_identity_and_legacy_source_kind() {
    let fixture = Fixture::new();
    let directory = fixture.path("process-report");
    let code = cli::run(vec![
        "live-process-loopback".into(),
        "--source-pid".into(),
        "7".into(),
        "--output".into(),
        "physical-output".into(),
        "--artifact-dir".into(),
        directory.to_string_lossy().into_owned(),
        "--seconds".into(),
        "1".into(),
        "--dry-run".into(),
    ])
    .unwrap();
    assert_eq!(code, 1);
    let report: Value =
        serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["mode"], "live_process_loopback");
    assert_eq!(report["source"]["source_kind"], "process_loopback");
    assert_eq!(report["source"]["pid"], 7);
    assert!(report.get("rate_search_measurement").is_none());
    assert!(report.get("trend_measurement").is_none());
    assert_eq!(
        report["qualification"]["configuration"]["source_kind"],
        "process_loopback"
    );
    assert_eq!(
        report["qualification"]["configuration"]["source_details"]["process_id"],
        7
    );
}

fn expected_json() -> Value {
    let format = json!({"sample_rate": 48000, "channels": 2, "sample_format": "f32"});
    json!({
        "source_revision": "test-revision", "binary_hash": "test-binary",
        "current_config_hash": "test-config", "target_machine_id": "test-machine",
        "windows_build": "test-build", "calibration_hash": "test-calibration",
        "evaluated_at_unix_seconds": 900000,
        "configuration": {
            "source_kind": "process_loopback", "source_identity": "test-process",
            "output_endpoint_id": "test-output", "processing_format": format,
            "native_input_format": format, "native_output_format": format,
            "measurement_format": format, "graph_id": "input-bus-output",
            "signal_id": "marker-v1", "controlled_load_id": "test-load"
        }
    })
}

fn report_json() -> Value {
    let expected = expected_json();
    json!({
        "schema_version": 1,
        "identity": {"run_id": "test-run", "source_revision": "test-revision",
            "binary_hash": "test-binary", "config_hash": "test-config",
            "machine_id": "test-machine", "windows_build": "test-build"},
        "expected_config_hash": "test-config", "configuration": expected["configuration"],
        "boundary": "external_dual_channel",
        "calibration": {"reference_id": "synthetic-fixture-only", "evidence_id": "synthetic-calibration",
            "output_endpoint_id": "test-output", "measurement_format": expected["configuration"]["measurement_format"],
            "common_sample_clock": true, "uncertainty_ms": 0.1,
            "valid_until_unix_seconds": 1000000, "calibration_hash": "test-calibration"},
        "input_queue": {"requested_capacity_frames": null, "requested_target_frames": null,
            "effective_capacity_frames": null, "effective_target_frames": null, "actual_period_frames": null},
        "output_queue": {"requested_capacity_frames": null, "requested_target_frames": null,
            "effective_capacity_frames": null, "effective_target_frames": null, "actual_period_frames": null},
        "observed_duration_seconds": 1800.0, "observed_measurement_frames": 86400000,
        "waveform_evidence_id": "synthetic-fixture-only", "steady_state": true, "controlled_load": true,
        "expected_marker_count": 3,
        "distribution": {"marker_count": 3, "sample_rate": 48000, "p50_ms": 5.0,
            "p95_ms": 8.0, "p99_ms": 8.0, "max_ms": 8.0},
        "quality": {"dropped_frames": 0, "silent_frames": 0, "underrun_frames": 0,
            "discontinuities": 0, "epoch_resets": 0, "marker_missing": 0, "sample_gap_frames": 0,
            "delivery_bound_violations": 0}
    })
}

#[test]
fn evaluate_retains_mismatched_uncalibrated_short_and_failed_reports_and_exit_status() {
    let fixture = Fixture::new();
    let expectation = fixture.json("expectation.json", &expected_json());
    for case in 0..6 {
        let mut run = report_json();
        match case {
            0 => run["configuration"]["output_endpoint_id"] = json!("wrong"),
            1 => run["calibration"] = Value::Null,
            2 => run["observed_duration_seconds"] = json!(1799),
            3 => run["quality"]["epoch_resets"] = json!(1),
            4 => run["quality"]["delivery_bound_violations"] = json!(1),
            _ => {}
        }
        let reports_value = json!([run]);
        let reports = fixture.json(&format!("reports-{case}.json"), &reports_value);
        let output = fixture.path(&format!("result-{case}.json"));
        let mut args = run_arguments(
            "evaluate",
            &[("--reports", &reports), ("--expectation", &expectation)],
            &output,
        );
        args.extend([
            "--level".into(),
            "experimental".into(),
            "--max-runs".into(),
            "4".into(),
        ]);
        assert_eq!(cli::run(args).unwrap(), i32::from(case != 5));
        let result: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        let retained_reports: Vec<QualificationReport> =
            serde_json::from_value(result["reports"].clone()).unwrap();
        let original_reports: Vec<QualificationReport> =
            serde_json::from_value(reports_value.clone()).unwrap();
        assert_eq!(retained_reports, original_reports);
        assert_eq!(result["qualified"], case == 5);
        assert!(!result["gaps"].as_array().unwrap().is_empty());
    }
}

#[test]
fn evaluator_enforces_repetition_current_expectation_and_run_caps() {
    let fixture = Fixture::new();
    let expectation = fixture.json("expectation.json", &expected_json());
    let reports = fixture.json("reports.json", &json!([report_json()]));
    let output = fixture.path("default.json");
    let mut args = run_arguments(
        "evaluate",
        &[("--reports", &reports), ("--expectation", &expectation)],
        &output,
    );
    args.extend([
        "--level".into(),
        "default".into(),
        "--max-runs".into(),
        "1".into(),
    ]);
    assert_eq!(cli::run(args).unwrap(), 1);
    let value: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(value["error"], "insufficient_repetitions");
    let mut expected = expected_json();
    expected["current_config_hash"] = json!("changed");
    let expectation = fixture.json("new-expectation.json", &expected);
    let output = fixture.path("stale.json");
    let mut args = run_arguments(
        "evaluate",
        &[("--reports", &reports), ("--expectation", &expectation)],
        &output,
    );
    args.extend([
        "--level".into(),
        "experimental".into(),
        "--max-runs".into(),
        "1".into(),
    ]);
    assert_eq!(cli::run(args).unwrap(), 1);
    let value: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(value["error"], "stale_identity");
}
