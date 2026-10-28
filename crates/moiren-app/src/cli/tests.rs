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
