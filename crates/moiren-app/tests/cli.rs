use std::process::Command;

#[test]
fn application_help_advertises_the_real_process_monitor_entry() {
    let output = Command::new(env!("CARGO_BIN_EXE_moiren-app"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("monitor --process"), "{help}");
}

#[test]
fn monitor_help_needs_no_process_or_audio_device() {
    let output = Command::new(env!("CARGO_BIN_EXE_moiren-app"))
        .args(["monitor", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--target-frames"), "{help}");
    assert!(help.contains("--capacity-frames"), "{help}");
    assert!(help.contains("not a latency qualification"), "{help}");
}
