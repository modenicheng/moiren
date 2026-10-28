use super::*;
use crate::service::{DeliveryProvenance, ProcessingPrecision, TransportLatencySettings};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn monitor(extra: &[&str]) -> Result<MonitorOptions> {
    let mut values = args(&[
        "monitor",
        "--process",
        "4294967294",
        "--output",
        "pinned-output",
    ]);
    values.extend(args(extra));
    match parse(values)? {
        Command::Monitor(options) => Ok(options),
        command => panic!("expected monitor: {command:?}"),
    }
}

#[test]
fn gui_and_offline_entry_remain_distinct() {
    assert!(matches!(parse(Vec::new()).unwrap(), Command::Gui));
    let Command::Offline(config) = parse(args(&["--gain", "0.25"])).unwrap() else {
        panic!()
    };
    assert_eq!(config.initial_gain, 0.25);
    for invalid in [
        vec!["--gain"],
        vec!["--gain", "NaN"],
        vec!["--gain", "17"],
        vec!["--gain", "1", "--gain", "2"],
    ] {
        assert!(parse(args(&invalid)).is_err());
    }
}

#[test]
fn monitor_compatibility_does_not_invent_explicit_queue_or_output_requests() {
    let options = monitor(&[]).unwrap();
    assert_eq!(options.measure_seconds, None);
    assert_eq!(options.measured_target().unwrap(), None);
    assert_eq!(
        options.transport().unwrap(),
        TransportLatencySettings::default()
    );
    assert_eq!(options.audio.output.buffer_frames, None);
    assert_eq!(options.seconds, 30);
    assert_eq!(options.audio.processing.precision, ProcessingPrecision::F32);
}

#[test]
fn complete_expert_tuple_is_declared_and_independent_of_output_buffer() {
    let options = monitor(&[
        "--target-frames",
        "1024",
        "--capacity-frames",
        "8192",
        "--jitter-frames",
        "96",
        "--max-packet-frames",
        "480",
        "--max-gap-frames",
        "480",
        "--output-buffer-frames",
        "64",
        "--precision",
        "f64",
    ])
    .unwrap();
    let process = options.transport().unwrap().process.unwrap();
    assert_eq!(process.target_frames, 1024);
    assert_eq!(process.capacity_frames, 8192);
    assert_eq!(process.provenance, DeliveryProvenance::Declared);
    assert_eq!(options.measure_seconds, None);
    assert_eq!(options.measured_target().unwrap(), None);
    assert_eq!(options.audio.output.buffer_frames, Some(64));
    assert_eq!(options.audio.processing.precision, ProcessingPrecision::F64);
}

#[test]
fn partial_duplicate_unknown_unbounded_and_unsafe_intent_is_refused() {
    for invalid in [
        vec!["--target-frames", "512"],
        vec!["--seconds", "0"],
        vec!["--seconds", "601"],
        vec!["--seconds", "1", "--seconds", "2"],
        vec!["--startup-timeout-ms", "30001"],
        vec!["--engine-block-frames", "3"],
        vec!["--output-buffer-frames", "0"],
        vec!["--precision", "f16"],
        vec!["--wat", "1"],
        vec!["--age-limit-100ns", "100000"],
        vec![
            "--target-frames",
            "64",
            "--capacity-frames",
            "128",
            "--jitter-frames",
            "96",
            "--max-packet-frames",
            "64",
            "--max-gap-frames",
            "64",
        ],
    ] {
        assert!(monitor(&invalid).is_err(), "accepted {invalid:?}");
    }
    assert!(parse(args(&["monitor"])).is_err());
    assert!(parse(args(&["monitor", "--process", "0", "--output", "output"])).is_err());
    assert!(parse(args(&["monitor", "--process", "12"])).is_err());
}

#[test]
fn explicit_age_with_complete_tuple_is_refused_without_fallback() {
    let error = monitor(&[
        "--target-frames",
        "1024",
        "--capacity-frames",
        "8192",
        "--jitter-frames",
        "96",
        "--max-packet-frames",
        "480",
        "--max-gap-frames",
        "480",
        "--age-limit-100ns",
        "100000",
    ])
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("sample-age enforcement unsupported")
    );
}

#[test]
fn measured_budget_uses_compatible_baseline_without_declared_packet_or_gap() {
    for seconds in ["1", "60"] {
        let options = monitor(&[
            "--measure-seconds",
            seconds,
            "--target-frames",
            "256",
            "--capacity-frames",
            "8192",
            "--jitter-frames",
            "32",
            "--seconds",
            "2",
            "--output-buffer-frames",
            "64",
        ])
        .unwrap();
        assert_eq!(options.seconds, 2);
        assert_eq!(
            options.measure_seconds,
            Some(seconds.parse::<u16>().unwrap())
        );
        assert_eq!(options.measured_target().unwrap(), Some((8192, 256, 32)));
        assert_eq!(options.audio.output.buffer_frames, Some(64));
        assert_eq!(
            options.transport().unwrap(),
            TransportLatencySettings::default()
        );
    }
}

#[test]
fn measured_budget_refuses_incomplete_mixed_or_unbounded_intent() {
    let complete = [
        "--measure-seconds",
        "1",
        "--target-frames",
        "256",
        "--capacity-frames",
        "8192",
        "--jitter-frames",
        "32",
    ];
    for extra in [
        vec!["--max-packet-frames", "48"],
        vec!["--max-gap-frames", "96"],
        vec!["--max-packet-frames", "48", "--max-gap-frames", "96"],
        vec!["--age-limit-100ns", "1"],
        vec!["--measure-seconds", "2"],
    ] {
        let mut invalid = complete.to_vec();
        invalid.extend(extra);
        assert!(monitor(&invalid).is_err(), "accepted {invalid:?}");
    }
    for invalid in [
        vec!["--measure-seconds", "1"],
        vec!["--measure-seconds", "1", "--target-frames", "256"],
        vec!["--measure-seconds", "1", "--capacity-frames", "8192"],
        vec!["--measure-seconds", "1", "--jitter-frames", "32"],
        vec![
            "--measure-seconds",
            "1",
            "--target-frames",
            "256",
            "--capacity-frames",
            "8192",
        ],
        vec![
            "--measure-seconds",
            "1",
            "--target-frames",
            "256",
            "--jitter-frames",
            "32",
        ],
        vec![
            "--measure-seconds",
            "1",
            "--capacity-frames",
            "8192",
            "--jitter-frames",
            "32",
        ],
        vec!["--measure-seconds", "0"],
        vec!["--measure-seconds", "61"],
        vec!["--measure-seconds", "65536"],
        vec!["--measure-seconds", "-1"],
        vec!["--measure-seconds", "NaN"],
    ] {
        assert!(monitor(&invalid).is_err(), "accepted {invalid:?}");
    }
    let mut missing = args(&[
        "monitor",
        "--process",
        "4294967294",
        "--output",
        "pinned-output",
    ]);
    missing.push("--measure-seconds".into());
    assert!(parse(missing).is_err());
}

#[test]
fn measured_duration_limits_are_enforced_even_with_a_complete_budget() {
    for seconds in ["0", "61", "65536", "-1", "1.5", "NaN"] {
        assert!(
            monitor(&[
                "--measure-seconds",
                seconds,
                "--target-frames",
                "256",
                "--capacity-frames",
                "8192",
                "--jitter-frames",
                "32",
            ])
            .is_err(),
            "accepted measurement duration {seconds:?}"
        );
    }
}

#[test]
fn measured_budget_shape_is_bounded_without_inventing_delivery_evidence() {
    for (capacity, target, jitter, accepted) in [
        (3, 2, 0, false),
        (4, 2, 0, true),
        (262144, 262143, 262143, true),
        (0, 2, 0, false),
        (2, 2, 0, false),
        (8192, 0, 0, false),
        (8192, 1, 0, false),
        (8192, 8192, 0, false),
        (262145, 256, 0, false),
        (8192, 256, 8192, false),
        (u32::MAX, 256, 0, false),
    ] {
        let values = [
            "--measure-seconds".to_string(),
            "1".to_string(),
            "--capacity-frames".to_string(),
            capacity.to_string(),
            "--target-frames".to_string(),
            target.to_string(),
            "--jitter-frames".to_string(),
            jitter.to_string(),
        ];
        let values: Vec<_> = values.iter().map(String::as_str).collect();
        assert_eq!(
            monitor(&values).is_ok(),
            accepted,
            "capacity={capacity} target={target} jitter={jitter}"
        );
    }
}

#[test]
fn monitor_help_explains_two_phase_measured_run_without_qualification() {
    let Command::Help(help) = parse(args(&["monitor", "--help"])).unwrap() else {
        panic!("expected help without Process identity or native preparation")
    };
    for expected in [
        "--measure-seconds 1..60",
        "baseline",
        "owner replacement",
        "--seconds",
        "not a latency qualification",
        "no packet/gap",
        "all five required together",
    ] {
        assert!(help.contains(expected), "missing {expected:?}: {help}");
    }
}

#[cfg(not(windows))]
#[test]
fn measured_monitor_is_explicitly_refused_on_nonwindows_at_runtime() {
    let options = monitor(&[
        "--measure-seconds",
        "1",
        "--target-frames",
        "256",
        "--capacity-frames",
        "8192",
        "--jitter-frames",
        "32",
    ])
    .unwrap();
    let error = super::monitor::run(options).unwrap_err();
    assert!(error.to_string().contains("only supported on Windows"));
}
