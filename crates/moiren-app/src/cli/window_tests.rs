use super::*;

const MEASURED: [&str; 8] = [
    "--measure-seconds",
    "1",
    "--target-frames",
    "256",
    "--capacity-frames",
    "8192",
    "--jitter-frames",
    "32",
];

const WINDOW: [(&str, &str); 5] = [
    ("--window-max-initial-unserved-frames", "0"),
    ("--window-max-initial-prime-discarded-frames", "0"),
    ("--window-min-output-frames", "1024"),
    ("--window-min-output-reads", "4"),
    ("--window-min-output-span-100ns", "100000"),
];

fn monitor(extra: &[&str]) -> Result<MonitorOptions> {
    let values = [
        "monitor",
        "--process",
        "4294967294",
        "--output",
        "pinned-output",
    ]
    .into_iter()
    .chain(extra.iter().copied())
    .map(String::from);
    let Command::Monitor(options) = parse(values)? else {
        panic!("expected monitor")
    };
    Ok(*options)
}

fn window_arguments() -> Vec<&'static str> {
    MEASURED
        .into_iter()
        .chain(WINDOW.into_iter().flat_map(|(flag, value)| [flag, value]))
        .collect()
}

#[test]
fn window_cli_accepts_all_five_explicit_thresholds_with_measured_budget() {
    let options = monitor(&window_arguments()).unwrap();
    assert_eq!(options.measure_seconds, Some(1));
    assert_eq!(options.measured_target().unwrap(), Some((8192, 256, 32)));
    assert_eq!(
        options.transport().unwrap(),
        crate::service::TransportLatencySettings::default()
    );
    let policy = options.window_policy().unwrap().unwrap();
    assert_eq!(policy.max_initial_unserved_frames, 0);
    assert_eq!(policy.max_initial_prime_discarded_frames, 0);
    assert_eq!(policy.min_output_frames, 1024);
    assert_eq!(policy.min_output_reads, 4);
    assert_eq!(policy.min_output_span_100ns, 100_000);
}

#[test]
fn window_cli_requires_every_threshold_without_guessing_defaults() {
    for mask in 1..31 {
        let mut arguments = MEASURED.to_vec();
        for (index, (flag, value)) in WINDOW.iter().enumerate() {
            if mask & (1 << index) != 0 {
                arguments.extend([*flag, *value]);
            }
        }
        let error = monitor(&arguments).unwrap_err().to_string();
        assert!(error.contains("all five"), "mask={mask}: {error}");
    }
}

#[test]
fn window_cli_requires_measurement_and_complete_existing_budget() {
    let window: Vec<_> = WINDOW
        .into_iter()
        .flat_map(|(flag, value)| [flag, value])
        .collect();
    let error = monitor(&window).unwrap_err().to_string();
    assert!(error.contains("--measure-seconds"), "{error}");
    for missing in 0..3 {
        let mut arguments = vec!["--measure-seconds", "1"];
        for (index, pair) in MEASURED[2..].as_chunks::<2>().0.iter().enumerate() {
            if index != missing {
                arguments.extend_from_slice(pair);
            }
        }
        arguments.extend_from_slice(&window);
        let error = monitor(&arguments).unwrap_err().to_string();
        assert!(error.contains("target, capacity and jitter"), "{error}");
    }
}

#[test]
fn window_cli_refuses_invalid_numbers_duplicates_and_missing_values() {
    for (flag, _) in WINDOW {
        for invalid in ["-1", "1.5", "NaN", "18446744073709551616"] {
            let mut arguments = window_arguments();
            let index = arguments.iter().position(|value| *value == flag).unwrap();
            arguments[index + 1] = invalid;
            assert!(monitor(&arguments).is_err(), "{flag}={invalid}");
        }
        let mut duplicate = window_arguments();
        duplicate.extend([flag, "1"]);
        assert!(monitor(&duplicate).is_err());
        let mut missing = MEASURED.to_vec();
        missing.push(flag);
        assert!(monitor(&missing).is_err());
    }
    for (flag, invalid) in [
        ("--window-min-output-frames", "0"),
        ("--window-min-output-reads", "0"),
        ("--window-min-output-reads", "1"),
        ("--window-min-output-span-100ns", "0"),
    ] {
        let mut arguments = window_arguments();
        let index = arguments.iter().position(|value| *value == flag).unwrap();
        arguments[index + 1] = invalid;
        assert!(monitor(&arguments).is_err(), "{flag}={invalid}");
    }
}

#[test]
fn window_cli_help_limits_the_claim_to_observed_finite_prefix() {
    for (flag, _) in WINDOW {
        assert!(MONITOR_HELP.contains(flag), "missing {flag}");
    }
    assert!(MONITOR_HELP.contains("finite-prefix"));
    assert!(MONITOR_HELP.contains("capped prefix refuses"));
    assert!(MONITOR_HELP.contains("not a stability or latency guarantee"));
    assert!(MONITOR_HELP.contains("sequence-one packet/read histories (8192 events each)"));
    assert!(MONITOR_HELP.contains("complete-drain/whole-wake histories (256 events each)"));
    assert!(MONITOR_HELP.contains("overwrites refuse"));
}

#[test]
fn window_cli_omission_preserves_per_wake_measurement_and_compatible_defaults() {
    for arguments in [vec![], MEASURED.to_vec()] {
        let options = monitor(&arguments).unwrap();
        assert_eq!(options.window_policy().unwrap(), None);
        assert_eq!(
            options.transport().unwrap(),
            crate::service::TransportLatencySettings::default()
        );
    }
}

#[test]
fn window_cli_thresholds_are_exact_u64_values_without_hidden_limits() {
    let mut arguments = MEASURED.to_vec();
    for (flag, _) in WINDOW {
        arguments.extend([flag, "18446744073709551615"]);
    }
    let policy = monitor(&arguments)
        .unwrap()
        .window_policy()
        .unwrap()
        .unwrap();
    let value = serde_json::to_value(policy).unwrap();
    for field in [
        "max_initial_unserved_frames",
        "max_initial_prime_discarded_frames",
        "min_output_frames",
        "min_output_reads",
        "min_output_span_100ns",
    ] {
        assert_eq!(value[field], u64::MAX);
    }
}

#[test]
fn window_cli_opt_in_does_not_bypass_existing_measured_mode_refusals() {
    for (flag, value) in [
        ("--max-packet-frames", "96"),
        ("--max-gap-frames", "96"),
        ("--age-limit-100ns", "1"),
        ("--max-ingress-elapsed-100ns", "100000"),
    ] {
        let mut arguments = window_arguments();
        arguments.extend([flag, value]);
        let error = monitor(&arguments).unwrap_err().to_string();
        assert!(!error.contains("unknown monitor option"), "{flag}: {error}");
    }
}
