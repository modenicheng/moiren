use moiren_app::{
    host::{AudioHost, HostConfig, HostEvent, SourceSettings},
    host_cli::*,
};
use moiren_core::protocol::{ControlReply, ReplyCode};
use moiren_engine::boundary::audio_bridge;

fn args(values: &[&str]) -> Result<HostCommand, HostCliError> {
    parse_host_args(values.iter().map(|v| v.to_string()))
}
#[test]
fn default_is_continuous_and_inputs_can_mix_and_repeat() {
    assert_eq!(
        args(&[
            "--output",
            "out",
            "--input",
            "a",
            "--process",
            "42",
            "--input",
            "b"
        ])
        .unwrap(),
        HostCommand::Run {
            output: "out".into(),
            inputs: vec![
                InitialInput::Physical("a".into()),
                InitialInput::Process(42),
                InitialInput::Physical("b".into())
            ],
            seconds: None
        }
    );
    assert!(
        matches!(args(&["--output", "out"]).unwrap(), HostCommand::Run { inputs, .. } if inputs.is_empty())
    );
    assert!(matches!(
        args(&["--output", "out", "--seconds", "7200"]).unwrap(),
        HostCommand::Run {
            seconds: Some(7200),
            ..
        }
    ));
}
#[test]
fn invalid_or_ambiguous_startup_options_fail_before_native_owners() {
    for values in [
        vec![],
        vec!["--output"],
        vec!["--output", ""],
        vec!["--output", "a\0b"],
        vec!["--output", "a", "--output", "b"],
        vec!["--output", "a", "--seconds", "0"],
        vec!["--output", "a", "--seconds", "-1"],
        vec!["--output", "a", "--process", "0"],
        vec!["--output", "a", "--process", "4294967296"],
        vec!["--list", "--output", "a"],
        vec!["--output", "a", "--seconds", "1", "--seconds", "2"],
        vec!["--wat"],
    ] {
        assert!(args(&values).is_err(), "accepted {values:?}");
    }
    assert_eq!(args(&["--help"]).unwrap(), HostCommand::Help);
    assert_eq!(args(&["--list"]).unwrap(), HostCommand::List);
}
#[test]
fn every_command_has_a_pure_parse_path() {
    for line in [
        r#"{"op":"status"}"#,
        r#"{"op":"graph"}"#,
        r#"{"op":"devices"}"#,
        r#"{"op":"processes"}"#,
        r#"{"op":"publish"}"#,
        r#"{"op":"cancel"}"#,
        r#"{"op":"stop"}"#,
        r#"{"op":"gain","source":1,"value":0.1,"ramp_frames":128}"#,
        r#"{"op":"pan","source":1,"value":-1}"#,
        r#"{"op":"compressor","source":1,"enabled":true,"settings":{"threshold_db":-24,"ratio":6}}"#,
        r#"{"op":"compressor","source":1,"enabled":false}"#,
        r#"{"op":"add","selection":{"kind":"physical","endpoint_id":"in"}}"#,
        r#"{"op":"replace","source":1,"selection":{"kind":"process","pid":42,"creation_time_100ns":1234}}"#,
        r#"{"op":"stop_source","source":1}"#,
        r#"{"op":"remove","source":1}"#,
        r#"{"op":"restart","source":1}"#,
        r#"{"op":"enable","source":1,"enabled":false}"#,
    ] {
        assert!(parse_control_command(line).is_ok(), "failed {line}");
    }
    let ControlCommand::Compressor { settings, .. } = parse_control_command(
        r#"{"op":"compressor","source":1,"enabled":true,"settings":{"threshold_db":-24}}"#,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(settings.threshold_db, -24.0);
    assert_eq!(settings.ratio, 4.0);
}
#[test]
fn invalid_commands_and_unpinned_processes_are_rejected() {
    for line in [
        "",
        "{}",
        r#"{"op":"bogus"}"#,
        r#"{"op":"gain","source":1,"value":17}"#,
        r#"{"op":"gain","source":1,"value":-0.1}"#,
        r#"{"op":"pan","source":1,"value":1.1}"#,
        r#"{"op":"pan","source":1,"value":null}"#,
        r#"{"op":"status","extra":true}"#,
        r#"{"op":"add","selection":{"kind":"physical","endpoint_id":""}}"#,
        r#"{"op":"add","selection":{"kind":"process","pid":42}}"#,
        r#"{"op":"replace","source":1,"selection":{"kind":"process","pid":42,"creation_time_100ns":0}}"#,
    ] {
        assert!(parse_control_command(line).is_err(), "accepted {line}");
    }
}
#[test]
fn json_graph_has_typed_ids_and_receipts_retain_request_identity() {
    let (mut host, mut renderer) = AudioHost::prepare(HostConfig::default()).unwrap();
    let (_writer, reader) = audio_bridge(2, 256, 8192).unwrap();
    let id = host
        .add_source_with_settings(
            reader,
            SourceSettings {
                gain: 0.05,
                pan: 0.0,
                available: true,
            },
        )
        .unwrap();
    let revision = host.publish().unwrap();
    renderer.render_interleaved(&mut [0.0; 2]).unwrap();
    let events = host.poll();
    assert!(
        events
            .iter()
            .any(|e| matches!(e,HostEvent::PlanApplied { revision:r, .. } if *r == revision))
    );
    let graph = graph_json(&host.graph_snapshot());
    assert!(graph["bus"].is_u64());
    assert!(graph["active"]["nodes"][0]["id"].is_u64());
    assert_eq!(graph["sources"][0]["id"], id.0);
    assert_eq!(graph["sources"][0]["gain"], 0.05);
    let reply = ControlReply {
        request_id: 12,
        plan_revision: revision,
        timeline_epoch: 1,
        code: ReplyCode::Applied,
        effective_frame: 99,
    };
    let json = event_json(HostEvent::Parameter(reply));
    assert_eq!(json["reply"]["request_id"], 12);
    assert_eq!(json["reply"]["effective_frame"], 99);
    assert_eq!(json["reply"]["code"], "Applied");
    host.finish(renderer);
}

#[cfg(windows)]
#[test]
fn invalid_selections_fail_without_opening_valid_audio_hardware() {
    use moiren_app::host::windows::*;
    let options = |sources| SessionOptions {
        output_endpoint_id: "deliberately-invalid-output-id".into(),
        sources,
        config: HostConfig::default(),
    };
    assert!(
        HostSession::start(options(vec![CaptureSelection::Physical {
            endpoint_id: String::new()
        }]))
        .is_err()
    );
    let target = moiren_windows_audio::process_loopback::ProcessIdentity {
        pid: std::process::id(),
        creation_time_100ns: 1,
        executable_name: "host".into(),
    };
    let result = HostSession::start(options(vec![CaptureSelection::Process {
        identity: target,
    }]));
    assert!(matches!(
        result,
        Err(SessionError::Capture(
            moiren_windows_audio::capture::CaptureError::FeedbackTarget
        ))
    ));
    assert!(
        HostSession::start(SessionOptions {
            output_endpoint_id: String::new(),
            sources: vec![],
            config: HostConfig::default()
        })
        .is_err()
    );
}

#[cfg(windows)]
#[test]
fn initial_process_preflight_failure_emits_one_json_record_and_nonzero_exit() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_moiren-app"))
        .args([
            "host",
            "--output",
            "deliberately-invalid-output-id",
            "--process",
            "4294967295",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "unexpected startup output: {stdout}");
    let event: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(event["type"], "startup_failed");
    assert!(
        event["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
    );
    assert!(String::from_utf8(output.stderr).unwrap().contains("Error:"));
}
