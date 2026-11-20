use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    stats::SILENT,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

const HARD_CAP: u64 = 16_777_216;

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

fn window_policy() -> Value {
    json!({
        "schema_version": 1,
        "policy": {
            "max_initial_unserved_frames": 1000,
            "max_initial_prime_discarded_frames": 1000,
            "min_output_frames": 8,
            "min_output_reads": 2,
            "min_output_span_100ns": 1
        }
    })
}

fn window_arguments(fixture: &Fixture) -> Vec<String> {
    let mut arguments = fixture.arguments();
    arguments.extend([
        "--window-policy".into(),
        fixture.path("window.json").to_string_lossy().into_owned(),
    ]);
    arguments
}

#[test]
fn window_policy_accepts_actual_replay_without_promoting_qualification_or_origin() {
    let baseline = Fixture::new();
    baseline.inputs();
    assert_eq!(crate::run_cli(baseline.arguments()), 0);
    let raw = baseline.output();
    assert!(raw.get("window_policy").is_none());
    assert!(raw.get("window_accepted").is_none());
    assert!(raw["candidates"][0].get("assessment").is_none());

    let fixture = Fixture::new();
    fixture.inputs();
    fixture.json("window.json", &window_policy());
    assert_eq!(super::super::run(window_arguments(&fixture)).unwrap(), 0);
    let output = fixture.output();
    assert_eq!(output["window_policy"], window_policy()["policy"]);
    assert_eq!(output["window_accepted"], true);
    assert_eq!(output["qualified"], false);
    assert_eq!(output["trace_origin"], "caller_supplied");
    assert_eq!(output["candidates"][0]["status"], "replayed");
    assert_eq!(
        output["candidates"][0]["report"],
        raw["candidates"][0]["report"]
    );
    let assessment = &output["candidates"][0]["assessment"];
    assert_eq!(assessment["status"], "no_violation_in_window");
    assert_eq!(assessment["policy"], output["window_policy"]);
    assert_eq!(assessment["violations"], json!([]));
}

#[test]
fn window_policy_violation_and_inconclusive_results_keep_raw_reports_and_exit_one() {
    for (field, threshold, expected) in [
        ("max_initial_unserved_frames", 0, "observed_violation"),
        ("min_output_frames", 1000, "inconclusive"),
        ("min_output_reads", 1000, "inconclusive"),
        ("min_output_span_100ns", 1000, "inconclusive"),
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut policy = window_policy();
        policy["policy"][field] = json!(threshold);
        fixture.json("window.json", &policy);
        assert_eq!(crate::run_cli(window_arguments(&fixture)), 1, "{field}");
        let output = fixture.output();
        assert_eq!(output["status"], "completed");
        assert_eq!(output["window_accepted"], false);
        assert_eq!(output["qualified"], false);
        let result = &output["candidates"][0];
        assert_eq!(result["status"], "replayed");
        assert_eq!(result["assessment"]["status"], expected, "{field}");
        assert_eq!(result["report"]["requested_frames"], 32);
        assert_eq!(result["report"]["qualified"], false);
        assert!(result.get("error").is_none());
        if expected == "observed_violation" {
            assert_eq!(
                result["assessment"]["violations"],
                json!(["initial_unserved"])
            );
        }
    }
}

#[test]
fn window_policy_requires_every_candidate_and_does_not_hide_refusals() {
    for values in [
        vec![candidate(12, 32), candidate(128, 256)],
        vec![candidate(12, 32), candidate(2, 32)],
        vec![candidate(2, 32), candidate(12, 16)],
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        fixture.json("candidates.json", &candidates(values.clone()));
        fixture.json("window.json", &window_policy());
        assert_eq!(crate::run_cli(window_arguments(&fixture)), 1);
        let output = fixture.output();
        assert_eq!(output["status"], "completed");
        assert_eq!(output["window_accepted"], false);
        for (index, options) in values.iter().enumerate() {
            let result = &output["candidates"][index];
            assert_eq!(result["options"], *options);
            if options["input"]["target_frames"] == 12 && options["input"]["capacity_frames"] == 32
            {
                assert_eq!(result["assessment"]["status"], "no_violation_in_window");
            } else if options["input"]["target_frames"] == 128 {
                assert_eq!(result["assessment"]["status"], "inconclusive");
                assert!(result["report"]["candidate"]["output_frames"] == 0);
            } else {
                assert_eq!(result["status"], "refused");
                assert_eq!(result["error"]["stage"], "candidate_replay");
                assert!(result.get("assessment").is_none());
                assert!(result.get("report").is_none());
            }
        }
    }
}

#[test]
fn window_policy_trace_refusal_retains_policy_and_per_candidate_errors() {
    let fixture = Fixture::new();
    fixture.inputs();
    fixture.json("window.json", &window_policy());
    let mut trace = artifact();
    trace["packet_history"]["writer_finished"] = json!(false);
    fixture.json("trace.json", &trace);
    assert_eq!(crate::run_cli(window_arguments(&fixture)), 1);
    let output = fixture.output();
    assert_eq!(output["status"], "refused");
    assert_eq!(output["window_policy"], window_policy()["policy"]);
    assert_eq!(output["window_accepted"], false);
    assert_eq!(output["error"]["code"], "incomplete_history");
    assert_eq!(
        output["candidates"][0]["error"]["stage"],
        "trace_validation"
    );
    assert!(output["candidates"][0].get("assessment").is_none());
    assert!(output["candidates"][0].get("report").is_none());
}

#[test]
fn window_policy_preserves_all_u64_digits_in_policy_and_assessment() {
    let fixture = Fixture::new();
    fixture.inputs();
    let mut policy = window_policy();
    for field in [
        "max_initial_unserved_frames",
        "max_initial_prime_discarded_frames",
    ] {
        policy["policy"][field] = json!(9_007_199_254_741_115_u64);
    }
    for field in [
        "min_output_frames",
        "min_output_reads",
        "min_output_span_100ns",
    ] {
        policy["policy"][field] = json!(u64::MAX);
    }
    fixture.json("window.json", &policy);
    assert_eq!(crate::run_cli(window_arguments(&fixture)), 1);
    let output = fixture.output();
    assert_eq!(output["window_policy"], policy["policy"]);
    assert_eq!(
        output["candidates"][0]["assessment"]["policy"],
        policy["policy"]
    );
    assert_eq!(
        output["candidates"][0]["assessment"]["status"],
        "inconclusive"
    );
}

#[test]
fn window_policy_schema_fields_and_thresholds_are_strict_before_creating_output() {
    for fault in [
        "schema",
        "missing_schema",
        "missing_policy",
        "unknown_envelope",
        "unknown_policy",
        "missing_allowance",
        "missing_frames",
        "missing_reads",
        "missing_span",
        "zero_frames",
        "one_read",
        "zero_span",
        "negative",
        "overflow",
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut policy = window_policy();
        match fault {
            "schema" => policy["schema_version"] = json!(2),
            "missing_schema" => {
                policy.as_object_mut().unwrap().remove("schema_version");
            }
            "missing_policy" => {
                policy.as_object_mut().unwrap().remove("policy");
            }
            "unknown_envelope" => policy["fallback"] = json!(true),
            "unknown_policy" => policy["policy"]["fallback"] = json!(true),
            "missing_allowance" => {
                policy["policy"]
                    .as_object_mut()
                    .unwrap()
                    .remove("max_initial_prime_discarded_frames");
            }
            "missing_frames" => {
                policy["policy"]
                    .as_object_mut()
                    .unwrap()
                    .remove("min_output_frames");
            }
            "missing_reads" => {
                policy["policy"]
                    .as_object_mut()
                    .unwrap()
                    .remove("min_output_reads");
            }
            "missing_span" => {
                policy["policy"]
                    .as_object_mut()
                    .unwrap()
                    .remove("min_output_span_100ns");
            }
            "zero_frames" => policy["policy"]["min_output_frames"] = json!(0),
            "one_read" => policy["policy"]["min_output_reads"] = json!(1),
            "zero_span" => policy["policy"]["min_output_span_100ns"] = json!(0),
            "negative" => policy["policy"]["max_initial_unserved_frames"] = json!(-1),
            "overflow" => policy["policy"]["min_output_frames"] = json!("18446744073709551616"),
            _ => unreachable!(),
        }
        fixture.json("window.json", &policy);
        let error = super::super::run(window_arguments(&fixture)).unwrap_err();
        assert!(
            !error.to_string().contains("unknown option"),
            "{fault}: {error:#}"
        );
        assert_eq!(crate::run_cli(window_arguments(&fixture)), 2, "{fault}");
        assert!(!fixture.path("output.json").exists());
    }
}

#[test]
fn window_policy_duplicate_json_fields_and_duplicate_or_missing_flag_values_are_errors() {
    let policy = window_policy();
    let nested = serde_json::to_string(&policy["policy"]).unwrap();
    let duplicated_nested = nested.replacen(
        "\"min_output_reads\":2",
        "\"min_output_reads\":2,\"min_output_reads\":2",
        1,
    );
    for text in [
        format!(r#"{{"schema_version":1,"schema_version":1,"policy":{nested}}}"#),
        format!(r#"{{"schema_version":1,"policy":{nested},"policy":{nested}}}"#),
        format!(r#"{{"schema_version":1,"policy":{duplicated_nested}}}"#),
        "{".into(),
    ] {
        let fixture = Fixture::new();
        fixture.inputs();
        fs::write(fixture.path("window.json"), text).unwrap();
        let error = super::super::run(window_arguments(&fixture)).unwrap_err();
        assert!(!error.to_string().contains("unknown option"), "{error:#}");
        assert!(!fixture.path("output.json").exists());
    }
    let fixture = Fixture::new();
    let mut arguments = window_arguments(&fixture);
    arguments.extend(["--window-policy".into(), "duplicate.json".into()]);
    assert!(
        super::super::run(arguments)
            .unwrap_err()
            .to_string()
            .contains("duplicate option --window-policy")
    );
    let mut arguments = fixture.arguments();
    arguments.push("--window-policy".into());
    assert!(
        super::super::run(arguments)
            .unwrap_err()
            .to_string()
            .contains("separate values")
    );
}

#[test]
fn window_policy_uses_input_cap_and_preserves_create_new_and_output_cap() {
    let fixture = Fixture::new();
    fixture.inputs();
    fixture.json("window.json", &window_policy());
    let largest_input = ["trace.json", "candidates.json"]
        .into_iter()
        .map(|name| fs::metadata(fixture.path(name)).unwrap().len())
        .max()
        .unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.path("window.json"))
        .unwrap()
        .set_len(largest_input + 1)
        .unwrap();
    let mut arguments = window_arguments(&fixture);
    set_option(&mut arguments, "--max-input-bytes", largest_input);
    let error = super::super::run(arguments).unwrap_err();
    assert!(format!("{error:#}").contains("cap"), "{error:#}");
    assert!(!fixture.path("output.json").exists());

    fixture.json("window.json", &window_policy());
    fs::write(fixture.path("output.json"), b"keep-existing-evidence").unwrap();
    assert!(super::super::run(window_arguments(&fixture)).is_err());
    assert_eq!(
        fs::read(fixture.path("output.json")).unwrap(),
        b"keep-existing-evidence"
    );

    let fixture = Fixture::new();
    fixture.inputs();
    fixture.json("window.json", &window_policy());
    let mut trace = artifact();
    trace["process"]["executable_name"] = json!("x".repeat(600_000));
    fixture.json("trace.json", &trace);
    let error = super::super::run(window_arguments(&fixture)).unwrap_err();
    assert!(
        format!("{error:#}").contains("replay output exceeds 1 MiB"),
        "{error:#}"
    );
    assert!(!fixture.path("output.json").exists());
}

#[test]
fn window_policy_file_is_not_ignored_and_help_explains_opt_in_gate() {
    let fixture = Fixture::new();
    fixture.inputs();
    let error = super::super::run(window_arguments(&fixture)).unwrap_err();
    assert!(!error.to_string().contains("unknown option"), "{error:#}");
    assert_eq!(crate::run_cli(window_arguments(&fixture)), 2);
    assert!(!fixture.path("output.json").exists());
    assert!(super::super::HELP.contains("[--window-policy FILE]"));
    assert!(super::super::HELP.contains("no_violation_in_window"));
    assert!(super::super::HELP.contains("inconclusive"));
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
fn ingress_elapsed_candidate_intent_is_retained_in_explicit_replay_refusal() {
    let fixture = Fixture::new();
    fixture.inputs();
    let mut requested = candidate(12, 32);
    requested["max_ingress_elapsed_100ns"] = json!(100_000);
    fixture.json("candidates.json", &candidates(vec![requested]));
    assert_eq!(super::super::run(fixture.arguments()).unwrap(), 0);
    let output = fixture.output();
    let result = &output["candidates"][0];
    assert_eq!(result["options"]["max_ingress_elapsed_100ns"], 100_000);
    assert_eq!(result["status"], "refused");
    assert_eq!(result["error"]["code"], "ingress_elapsed_unsupported");
    assert_eq!(output["qualified"], false);
    assert!(result.get("report").is_none());
}

#[test]
fn invalid_trace_refusals_keep_requested_candidates_and_exit_one() {
    for fault in ["truncated", "ambiguous", "pid", "created", "generation"] {
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
fn replay_output_is_capped_before_creating_a_partial_evidence_file() {
    let fixture = Fixture::new();
    fixture.inputs();
    let mut trace = artifact();
    trace["process"]["executable_name"] = json!("x".repeat(600_000));
    fixture.json("trace.json", &trace);
    assert!(fs::metadata(fixture.path("trace.json")).unwrap().len() < HARD_CAP);
    let error = super::super::run(fixture.arguments()).unwrap_err();
    assert!(
        format!("{error:#}").contains("replay output exceeds 1 MiB"),
        "{error:#}"
    );
    assert!(!fixture.path("output.json").exists());
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
        ("--max-input-bytes", "16777217"),
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
fn exact_sixteen_mib_inputs_are_accepted_but_schema1_still_refuses_257_events() {
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
fn input_range_above_one_mib_up_to_sixteen_mib_is_valid_and_larger_caps_are_not() {
    for limit in [1_048_577_u64, HARD_CAP] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut arguments = fixture.arguments();
        set_option(&mut arguments, "--max-input-bytes", limit);
        assert_eq!(super::super::run(arguments).unwrap(), 0, "limit={limit}");
        assert_eq!(fixture.output()["qualified"], false);
    }
    for limit in [0, HARD_CAP + 1, u64::MAX] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut arguments = fixture.arguments();
        set_option(&mut arguments, "--max-input-bytes", limit);
        let error = super::super::run(arguments).unwrap_err();
        assert!(
            error.to_string().contains("1..16777216"),
            "limit={limit}: {error:#}"
        );
        assert!(!fixture.path("output.json").exists());
    }
}

#[test]
fn schema2_prefix_document_replays_without_promoting_origin_or_qualification() {
    let fixture = Fixture::new();
    fixture.inputs();
    let mut trace = artifact();
    trace["schema_version"] = json!(2);
    trace["collection_refusal"] = Value::Null;
    fixture.json("trace.json", &trace);
    assert_eq!(super::super::run(fixture.arguments()).unwrap(), 0);
    let output = fixture.output();
    assert_eq!(output["status"], "completed");
    assert_eq!(output["trace_origin"], "caller_supplied");
    assert_eq!(output["qualified"], false);
    assert_eq!(output["process"], trace["process"]);
    assert_eq!(output["owner_generation"], trace["owner_generation"]);
    assert_eq!(output["candidates"][0]["status"], "replayed");
    let report = &output["candidates"][0]["report"];
    assert_eq!(report["packet_events"], 8);
    assert_eq!(report["read_events"], 8);
    assert_eq!(report["requested_frames"], 32);
    assert_eq!(report["trace_origin"], "caller_supplied");
    assert_eq!(report["qualified"], false);
    assert_eq!(report["candidate"]["age_discarded_frames"], 0);
    assert!(report["candidate"].get("oldest_frame_age").is_none());
}

#[test]
fn schema2_requires_collection_refusal_and_retains_structured_trace_refusals() {
    for malformed in ["missing_refusal", "unknown_refusal", "unknown_schema"] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut trace = artifact();
        trace["schema_version"] = json!(2);
        trace["collection_refusal"] = Value::Null;
        match malformed {
            "missing_refusal" => {
                trace.as_object_mut().unwrap().remove("collection_refusal");
            }
            "unknown_refusal" => trace["collection_refusal"] = json!("not_a_collection_refusal"),
            "unknown_schema" => trace["schema_version"] = json!(3),
            _ => unreachable!(),
        }
        fixture.json("trace.json", &trace);
        assert!(
            super::super::run(fixture.arguments()).is_err(),
            "{malformed}"
        );
        assert!(!fixture.path("output.json").exists());
    }
    for fault in ["unclosed", "generation"] {
        let fixture = Fixture::new();
        fixture.inputs();
        let mut trace = artifact();
        trace["schema_version"] = json!(2);
        trace["collection_refusal"] = Value::Null;
        let mut arguments = fixture.arguments();
        if fault == "unclosed" {
            trace["queue_history"]["writer_finished"] = json!(false);
        } else {
            set_option(&mut arguments, "--owner-generation", 8);
        }
        fixture.json("trace.json", &trace);
        assert_eq!(super::super::run(arguments).unwrap(), 1, "{fault}");
        let output = fixture.output();
        assert_eq!(output["status"], "refused");
        assert_eq!(output["qualified"], false);
        assert_eq!(output["trace_origin"], "caller_supplied");
        assert_eq!(output["error"]["stage"], "trace_validation");
        assert_eq!(output["candidates"][0]["options"], candidate(12, 32));
        assert_eq!(output["candidates"][0]["status"], "refused");
        assert!(output["candidates"][0].get("report").is_none());
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
    assert!(super::super::HELP.contains("--max-input-bytes 1..16777216"));
    assert!(super::super::HELP.contains("schema 1 or 2"));
    assert!(super::super::HELP.contains("output hard cap 1 MiB"));
}
