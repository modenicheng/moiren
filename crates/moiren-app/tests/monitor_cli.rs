use moiren_app::monitor_cli::{MonitorCommand, parse_monitor_args};

fn parse(args: &[&str]) -> Result<MonitorCommand, moiren_app::monitor_cli::MonitorCliError> {
    parse_monitor_args(args.iter().map(|x| (*x).to_owned()))
}
#[test]
fn explicit_input_output_defaults_and_readonly_modes() {
    let MonitorCommand::Run {
        input_endpoint_id,
        output_endpoint_id,
        seconds,
        config,
    } = parse(&["--input", "mic", "--output", "speakers"]).unwrap()
    else {
        panic!("run expected");
    };
    assert_eq!(
        (
            input_endpoint_id.as_str(),
            output_endpoint_id.as_str(),
            seconds
        ),
        ("mic", "speakers", 10)
    );
    assert_eq!(config.gain, 0.05);
    assert!(matches!(parse(&["--list"]), Ok(MonitorCommand::List)));
    assert!(matches!(parse(&["-h"]), Ok(MonitorCommand::Help)));
}
#[test]
fn malformed_cli_cannot_start_capture_or_render() {
    for args in [
        vec![],
        vec!["--input", "mic"],
        vec!["--output", "out"],
        vec!["--input"],
        vec!["--list", "--input", "mic"],
        vec!["--input", "mic", "--input", "mic", "--output", "out"],
        vec!["--input", "", "--output", "out"],
        vec!["--input", "bad\0id", "--output", "out"],
        vec!["--input", "mic", "--output", "out", "--seconds", "0"],
        vec!["--input", "mic", "--output", "out", "--seconds", "601"],
        vec!["--input", "mic", "--output", "out", "--gain", "NaN"],
        vec!["--input", "mic", "--output", "out", "--pan", "2"],
        vec!["--unknown"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}

#[test]
fn process_selection_is_explicit_and_exclusive() {
    let MonitorCommand::Process {
        pid,
        output_endpoint_id,
        seconds,
        config,
    } = parse(&["--process", "123", "--output", "speakers"]).unwrap()
    else {
        panic!("process expected");
    };
    assert_eq!(
        (pid, output_endpoint_id.as_str(), seconds, config.gain),
        (123, "speakers", 10, 0.05)
    );
    for args in [
        vec!["--process", "0", "--output", "out"],
        vec!["--process", "-1", "--output", "out"],
        vec!["--process", "4294967296", "--output", "out"],
        vec!["--process", "123"],
        vec!["--process", "123", "--input", "mic", "--output", "out"],
        vec!["--process", "123", "--process", "124", "--output", "out"],
        vec!["--list", "--process", "123"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}

#[test]
fn balanced_profile_exposes_joint_latency_budget() {
    let MonitorCommand::Run { config, .. } = parse(&[
        "--input",
        "mic",
        "--output",
        "speakers",
        "--profile",
        "balanced",
        "--period-ms",
        "1.5",
    ])
    .unwrap() else {
        panic!("run expected");
    };
    assert_eq!(
        config.latency.profile,
        moiren_app::monitor::MonitorLatencyProfile::Balanced
    );
    assert_eq!(config.latency.shared_period_ms, Some(1.5));
    assert_eq!(config.latency.input_target_frames, Some(1024));
    assert_eq!(config.latency.input_jitter_frames, Some(96));
    assert_eq!(config.latency.render_target_frames, Some(480));
    assert_eq!(config.latency.wake_margin_frames, Some(96));
    assert_eq!(config.latency.callback_budget_frames, Some(48));
}

#[test]
fn custom_profile_requires_explicit_budget_and_process_mode_accepts_budgeted_profiles() {
    assert!(
        parse(&[
            "--input",
            "mic",
            "--output",
            "speakers",
            "--profile",
            "custom",
        ])
        .is_err()
    );
    let command = parse(&[
        "--process",
        "123",
        "--output",
        "speakers",
        "--profile",
        "balanced",
    ]);
    let MonitorCommand::Process { config, .. } = command.unwrap() else {
        panic!("process expected");
    };
    assert_eq!(
        config.latency.profile,
        moiren_app::monitor::MonitorLatencyProfile::Balanced
    );
}

#[test]
fn latency_values_are_strictly_validated() {
    for args in [
        vec!["--input", "mic", "--output", "out", "--period-ms", "0"],
        vec![
            "--input",
            "mic",
            "--output",
            "out",
            "--period-ms",
            "default",
            "--profile",
            "balanced",
            "--input-target-frames",
            "1",
        ],
        vec![
            "--input",
            "mic",
            "--output",
            "out",
            "--profile",
            "custom",
            "--input-target-frames",
            "1024",
            "--input-jitter-frames",
            "96",
            "--render-target-frames",
            "480",
            "--wake-margin-frames",
            "96",
            "--callback-budget-frames",
            "0",
        ],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}
