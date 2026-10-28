use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    stats::SILENT,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

const HARD_CAP: u64 = 1_048_576;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "moiren-process-replay-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn json(&self, name: &str, value: &Value) {
        fs::write(self.path(name), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn arguments(&self) -> Vec<String> {
        vec![
            "replay-process".into(),
            "--trace".into(),
            self.path("trace.json").to_string_lossy().into_owned(),
            "--candidates".into(),
            self.path("candidates.json").to_string_lossy().into_owned(),
            "--process-pid".into(),
            "42".into(),
            "--process-created-100ns".into(),
            "9007199254741115".into(),
            "--owner-generation".into(),
            "7".into(),
            "--max-input-bytes".into(),
            HARD_CAP.to_string(),
            "--output".into(),
            self.path("output.json").to_string_lossy().into_owned(),
        ]
    }

    fn inputs(&self) {
        self.json("trace.json", &artifact());
        self.json("candidates.json", &candidates(vec![candidate(12, 32)]));
    }

    fn output(&self) -> Value {
        serde_json::from_slice(&fs::read(self.path("output.json")).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn artifact() -> Value {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 8,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    })
    .unwrap();
    for index in 0..8 {
        let mut stamp = 1000 + index * 100;
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 4,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || {
                    stamp += 1;
                    Some(stamp)
                },
            )
            .unwrap();
        stamp += 10;
        source
            .read_interleaved_with_publication_clock(&mut [0.0; 8], || {
                stamp += 1;
                Some(stamp)
            })
            .unwrap();
    }
    drop(ingress);
    drop(source);
    json!({
        "schema_version": 1,
        "process": {
            "pid": 42,
            "creation_time_100ns": 9_007_199_254_741_115_u64,
            "executable_name": "not-a-real-process.exe"
        },
        "owner_generation": 7,
        "input_sample_rate": 48000,
        "consumer_sample_rate": 48000,
        "prepared_consumer_demand_frames": 4,
        "packet_history": observer.packet_history().unwrap(),
        "queue_history": observer.queue_history().unwrap()
    })
}

fn candidate(target: u32, capacity: u32) -> Value {
    json!({
        "input": {"policy": "target", "capacity_frames": capacity, "target_frames": target, "jitter_frames": 0},
        "delivery": {"sample_rate": 48000, "max_packet_frames": 4, "max_gap_frames": 4, "source": "declared"},
        "engine_sample_rate": 48000,
        "max_demand_frames": 4
    })
}

fn candidates(values: Vec<Value>) -> Value {
    json!({"schema_version": 1, "candidates": values})
}

fn set_option(arguments: &mut [String], name: &str, value: impl ToString) {
    let index = arguments.iter().position(|arg| arg == name).unwrap();
    arguments[index + 1] = value.to_string();
}

#[test]
fn actual_cli_retains_queue_results_and_never_qualifies_even_poor_quality() {
    let fixture = Fixture::new();
    fixture.inputs();
    let values = vec![candidate(12, 32), candidate(24, 64)];
    fixture.json("candidates.json", &candidates(values.clone()));
    assert_eq!(super::super::run(fixture.arguments()).unwrap(), 0);
    let output = fixture.output();
    assert_eq!(output["schema_version"], 1);
    assert_eq!(output["mode"], "offline_process_queue_replay");
    assert_eq!(output["status"], "completed");
    assert_eq!(output["qualified"], false);
    assert_eq!(output["trace_origin"], "caller_supplied");
    assert_eq!(
        output["process"]["creation_time_100ns"].as_u64(),
        Some(9_007_199_254_741_115)
    );
    assert_eq!(output["candidates"].as_array().unwrap().len(), 2);
    for (index, requested) in values.iter().enumerate() {
        let result = &output["candidates"][index];
        assert_eq!(result["status"], "replayed");
        assert_eq!(result["options"], *requested);
        assert_eq!(result["report"]["qualified"], false);
        assert_eq!(result["report"]["trace_origin"], "caller_supplied");
        assert_eq!(result["report"]["requested_frames"], 32);
        assert_eq!(result["report"]["candidate"]["captured_frames"], 32);
        assert!(
            result["report"]["startup_unserved_frames"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(result["report"]["candidate"]["age_discarded_frames"], 0);
        assert!(
            result["report"]["candidate"]
                .get("oldest_frame_age")
                .is_none()
        );
        assert!(result.get("error").is_none());
    }
    assert!(!output["limitations"].as_array().unwrap().is_empty());
}

#[test]
fn invalid_custom_candidates_are_structured_refusals_without_compatible_fallback() {
    let fixture = Fixture::new();
    fixture.inputs();
    let mut wrong_rate = candidate(12, 32);
    wrong_rate["engine_sample_rate"] = json!(96_000);
    let mut age = candidate(12, 32);
    age["age_limit_100ns"] = json!(1);
    let values = vec![
        candidate(2, 32),
        candidate(12, 16),
        wrong_rate,
        age,
        candidate(12, 32),
    ];
    fixture.json("candidates.json", &candidates(values.clone()));
    assert_eq!(super::super::run(fixture.arguments()).unwrap(), 0);
    let output = fixture.output();
    for (index, expected) in values.iter().enumerate() {
        let result = &output["candidates"][index];
        assert_eq!(result["options"], *expected);
        if index == 4 {
            assert_eq!(result["status"], "replayed");
        } else {
            assert_eq!(result["status"], "refused");
            assert_eq!(result["error"]["stage"], "candidate_replay");
            assert!(!result["error"]["code"].is_null());
            assert!(result.get("report").is_none());
        }
    }
    assert_eq!(output["qualified"], false);
}

#[test]
fn invalid_trace_refusals_keep_requested_candidates_and_exit_one() {
    for fault in [
        "truncated",
        "ambiguous",
        "pid",
        "created",
        "generation",
        "schema",
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut trace = artifact();
        let mut arguments = fixture.arguments();
        match fault {
            "truncated" => trace["packet_history"]["writer_finished"] = json!(false),
            "ambiguous" => {
                trace["packet_history"]["events"][0]["sample"]["qpc_after_100ns"] =
                    trace["queue_history"]["events"][0]["sample"]["qpc_before_100ns"].clone();
            }
            "pid" => set_option(&mut arguments, "--process-pid", 43),
            "created" => set_option(&mut arguments, "--process-created-100ns", 123),
            "generation" => set_option(&mut arguments, "--owner-generation", 8),
            "schema" => trace["schema_version"] = json!(2),
            _ => unreachable!(),
        }
        fixture.json("trace.json", &trace);
        assert_eq!(super::super::run(arguments).unwrap(), 1, "{fault}");
        let output = fixture.output();
        assert_eq!(output["status"], "refused");
        assert_eq!(output["qualified"], false);
        assert_eq!(output["trace_origin"], "caller_supplied");
        assert_eq!(output["error"]["stage"], "trace_validation");
        assert!(!output["error"]["code"].is_null());
        assert_eq!(output["candidates"][0]["status"], "refused");
        assert_eq!(output["candidates"][0]["options"], candidate(12, 32));
        assert!(output["candidates"][0].get("report").is_none());
    }
}

#[test]
fn required_flags_duplicates_unknowns_and_numeric_bounds_are_rejected_before_io() {
    let fixture = Fixture::new();
    for name in [
        "--trace",
        "--candidates",
        "--process-pid",
        "--process-created-100ns",
        "--owner-generation",
        "--max-input-bytes",
        "--output",
    ] {
        let mut missing = fixture.arguments();
        let index = missing.iter().position(|arg| arg == name).unwrap();
        missing.drain(index..index + 2);
        assert!(super::super::run(missing).is_err(), "{name}");
        let mut duplicate = fixture.arguments();
        duplicate.extend([name.into(), "1".into()]);
        assert!(super::super::run(duplicate).is_err(), "{name}");
    }
    for (name, value) in [
        ("--unexpected", "1"),
        ("--process-pid", "0"),
        ("--process-pid", "4294967296"),
        ("--process-created-100ns", "0"),
        ("--owner-generation", "0"),
        ("--max-input-bytes", "0"),
        ("--max-input-bytes", "1048577"),
        ("--max-input-bytes", "18446744073709551615"),
    ] {
        let mut arguments = fixture.arguments();
        if name == "--unexpected" {
            arguments.extend([name.into(), value.into()]);
        } else {
            set_option(&mut arguments, name, value);
        }
        assert!(super::super::run(arguments).is_err(), "{name}={value}");
    }
    let mut arguments = fixture.arguments();
    arguments.pop();
    assert!(super::super::run(arguments).is_err());
    assert!(!fixture.path("output.json").exists());
}

#[test]
fn candidates_require_explicit_options_rate_demand_and_strict_envelope() {
    for field in [
        "input",
        "delivery",
        "engine_sample_rate",
        "max_demand_frames",
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut value = candidate(12, 32);
        value.as_object_mut().unwrap().remove(field);
        fixture.json("candidates.json", &candidates(vec![value]));
        assert!(super::super::run(fixture.arguments()).is_err(), "{field}");
        assert!(!fixture.path("output.json").exists());
    }
    for fault in [
        "schema",
        "unknown_envelope",
        "unknown_option",
        "unknown_input",
        "missing_target",
        "unknown_delivery",
        "missing_schema",
        "empty",
        "excess",
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut request = candidates(vec![candidate(12, 32)]);
        match fault {
            "schema" => request["schema_version"] = json!(2),
            "unknown_envelope" => request["default"] = json!(true),
            "unknown_option" => request["candidates"][0]["fallback"] = json!(true),
            "unknown_input" => request["candidates"][0]["input"]["fallback"] = json!(true),
            "missing_target" => {
                request["candidates"][0]["input"]
                    .as_object_mut()
                    .unwrap()
                    .remove("target_frames");
            }
            "unknown_delivery" => request["candidates"][0]["delivery"]["measured"] = json!(true),
            "missing_schema" => {
                request.as_object_mut().unwrap().remove("schema_version");
            }
            "empty" => request["candidates"] = json!([]),
            "excess" => request["candidates"] = json!(vec![candidate(12, 32); 17]),
            _ => unreachable!(),
        }
        fixture.json("candidates.json", &request);
        assert!(super::super::run(fixture.arguments()).is_err(), "{fault}");
        assert!(!fixture.path("output.json").exists());
    }
}

#[test]
fn exactly_sixteen_explicit_candidates_and_an_explicit_compatible_request_are_supported() {
    let fixture = Fixture::new();
    fixture.inputs();
    let compatible = json!({"input":{"policy":"compatible"},"delivery":null,"engine_sample_rate":48000,"max_demand_frames":4});
    let mut values = vec![candidate(12, 32); 15];
    values.push(compatible.clone());
    fixture.json("candidates.json", &candidates(values));
    assert_eq!(super::super::run(fixture.arguments()).unwrap(), 0);
    let output = fixture.output();
    assert_eq!(output["candidates"].as_array().unwrap().len(), 16);
    assert_eq!(output["candidates"][15]["options"], compatible);
    assert_eq!(output["candidates"][15]["status"], "replayed");
    assert!(
        output["candidates"][15]["report"]["unserved_frames"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(output["qualified"], false);
}

#[test]
fn both_files_are_byte_bounded_before_deserialization_and_output_is_create_new() {
    for name in ["trace.json", "candidates.json"] {
        let fixture = Fixture::new();
        fixture.inputs();
        fs::OpenOptions::new()
            .write(true)
            .open(fixture.path(name))
            .unwrap()
            .set_len(HARD_CAP + 1)
            .unwrap();
        let error = super::super::run(fixture.arguments()).unwrap_err();
        assert!(error.to_string().contains("cap"), "{error:#}");
        assert!(!fixture.path("output.json").exists());
    }
    let fixture = Fixture::new();
    fixture.inputs();
    let mut arguments = fixture.arguments();
    set_option(&mut arguments, "--max-input-bytes", 8);
    assert!(super::super::run(arguments).is_err());
    fs::write(fixture.path("output.json"), b"keep-existing-evidence").unwrap();
    assert!(super::super::run(fixture.arguments()).is_err());
    assert_eq!(
        fs::read(fixture.path("output.json")).unwrap(),
        b"keep-existing-evidence"
    );
}

#[test]
fn malformed_json_duplicate_json_fields_and_missing_files_are_errors_not_trace_refusals() {
    for (name, text) in [
        ("trace.json", "{"),
        ("candidates.json", "{"),
        (
            "candidates.json",
            r#"{"schema_version":1,"schema_version":1,"candidates":[]}"#,
        ),
        (
            "candidates.json",
            r#"{"schema_version":1,"candidates":[{"input":{"policy":"compatible"},"input":{"policy":"compatible"},"delivery":null,"engine_sample_rate":48000,"max_demand_frames":4}]}"#,
        ),
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        fs::write(fixture.path(name), text).unwrap();
        assert!(super::super::run(fixture.arguments()).is_err());
        assert!(!fixture.path("output.json").exists());
    }
    let fixture = Fixture::new();
    assert!(super::super::run(fixture.arguments()).is_err());
    assert_eq!(crate::run_cli(fixture.arguments()), 2);
}

#[test]
fn wrapper_distinguishes_completed_all_refused_sweep_from_refused_trace() {
    let fixture = Fixture::new();
    fixture.inputs();
    fixture.json("candidates.json", &candidates(vec![candidate(2, 32)]));
    assert_eq!(crate::run_cli(fixture.arguments()), 0);
    let output = fixture.output();
    assert_eq!(output["status"], "completed");
    assert_eq!(output["qualified"], false);
    assert_eq!(output["candidates"][0]["status"], "refused");
    let fixture = Fixture::new();
    fixture.inputs();
    let mut trace = artifact();
    trace["queue_history"]["writer_finished"] = json!(false);
    fixture.json("trace.json", &trace);
    assert_eq!(crate::run_cli(fixture.arguments()), 1);
    assert_eq!(fixture.output()["error"]["code"], "incomplete_history");
}

#[test]
fn exact_one_mib_inputs_are_accepted_but_more_than_256_history_events_are_not() {
    let fixture = Fixture::new();
    fixture.inputs();
    for name in ["trace.json", "candidates.json"] {
        let mut bytes = fs::read(fixture.path(name)).unwrap();
        bytes.resize(HARD_CAP as usize, b' ');
        fs::write(fixture.path(name), bytes).unwrap();
    }
    assert_eq!(super::super::run(fixture.arguments()).unwrap(), 0);
    for history in ["packet_history", "queue_history"] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut trace = artifact();
        let event = trace[history]["events"][0].clone();
        trace[history]["events"] = json!(vec![event; 257]);
        fixture.json("trace.json", &trace);
        assert!(super::super::run(fixture.arguments()).is_err(), "{history}");
        assert!(!fixture.path("output.json").exists());
    }
}

#[test]
fn command_help_is_offline_and_does_not_require_input_files() {
    assert_eq!(
        super::super::run(vec!["replay-process".into(), "--help".into()]).unwrap(),
        0
    );
    assert!(super::super::HELP.contains("replay-process"));
    assert!(super::super::HELP.contains("not qualification"));
}
