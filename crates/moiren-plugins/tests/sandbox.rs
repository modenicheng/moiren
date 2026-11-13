//! Sandbox acceptance tests: the parent-side out-of-process scan against the
//! real scanner binary. Gated off Miri for the same reason as the runtime
//! plugin suite — spawning processes and (for the fixture test) compiling a
//! native cdylib are outside Miri's process model.
#![cfg(not(miri))]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use moiren_plugins::sandbox::{SandboxConfig, SandboxError, scan_root_sandboxed, scan_sandboxed};

/// Serializes the tests that flip the scanner's environment hooks: the hooks
/// are process-global and cargo runs #[test]s in parallel threads.
static ENV_HOOK: Mutex<()> = Mutex::new(());

/// SAFETY: every set_var/remove_var on these names happens under
/// `ENV_HOOK`, so no parallel test observes a half-configured hook.
fn set_hook(name: &str, value: Option<&Path>) {
    unsafe {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

/// The scanner binary cargo built for this test run.
fn scanner_config() -> SandboxConfig {
    SandboxConfig::new(
        env!("CARGO_BIN_EXE_moiren-plugin-scanner"),
        Duration::from_secs(60),
    )
}

/// A unique-per-test directory root that exists but scans empty.
fn empty_root(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sandbox-empty-{tag}"));
    std::fs::create_dir_all(&dir).expect("empty root");
    dir
}

/// Build the gain fixture once and expose a dedicated scan root containing
/// only its CLAP alias, so the sandboxed scan reports exactly one candidate.
fn fixture_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let binary = tmp.join(format!(
            "moiren_test_gain.{}",
            std::env::consts::DLL_EXTENSION
        ));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gain_plugin.rs");
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "cdylib", "-O"])
            .arg(&source)
            .args(["-o", binary.to_str().expect("utf-8 tmpdir")])
            .status()
            .expect("rustc must be available to build the fixture");
        assert!(status.success(), "fixture compilation failed");
        let root = tmp.join("sandbox-fixture-root");
        std::fs::create_dir_all(&root).expect("fixture root");
        std::fs::copy(&binary, root.join("moiren_test_gain.clap")).expect("alias copy");
        root
    })
}

#[test]
fn empty_root_reports_an_empty_report() {
    let report = scan_root_sandboxed(&empty_root("empty"), &scanner_config())
        .expect("an empty root scans cleanly");
    assert!(report.descriptors.is_empty());
    assert!(report.failures.is_empty());
}

#[test]
fn fixture_root_reports_its_probed_parameter_table() {
    let report = scan_root_sandboxed(fixture_root(), &scanner_config())
        .expect("the fixture root scans cleanly");
    assert_eq!(report.failures, Vec::new(), "no failures expected");
    assert_eq!(report.descriptors.len(), 1, "one CLAP candidate");
    let probed = &report.descriptors[0];
    assert_eq!(probed.descriptor.format.label(), "CLAP");
    assert_eq!(probed.descriptor.id, "org.moiren.test.gain");
    // The probe ran a live instance in the child: the parameter table is
    // present and reports the fixture's gain shape.
    assert_eq!(probed.parameters.len(), 1);
    assert_eq!(probed.parameters[0].name, "Gain");
    assert_eq!(probed.parameters[0].default, 1.0);
}

#[test]
fn hung_scanner_is_killed_and_mapped_to_the_timeout_error() {
    let _guard = ENV_HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let root = empty_root("hang");
    set_hook("MOIREN_SCANNER_TEST_HANG", Some(&root));
    let config = SandboxConfig::new(
        env!("CARGO_BIN_EXE_moiren-plugin-scanner"),
        Duration::from_millis(300),
    );
    let error = scan_root_sandboxed(&root, &config)
        .expect_err("the hung scanner must time out")
        .1;
    set_hook("MOIREN_SCANNER_TEST_HANG", None);
    assert_eq!(error, SandboxError::Timeout);
}

#[test]
fn crashed_scanner_maps_to_the_typed_crash_error() {
    let _guard = ENV_HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let root = empty_root("crash");
    set_hook("MOIREN_SCANNER_TEST_CRASH", Some(&root));
    let error = scan_root_sandboxed(&root, &scanner_config())
        .expect_err("the crashing scanner must not report")
        .1;
    set_hook("MOIREN_SCANNER_TEST_CRASH", None);
    assert_eq!(error, SandboxError::Crashed(Some(0xC0DE)));
}

#[test]
fn aggregate_scan_continues_past_a_crashed_root() {
    let _guard = ENV_HOOK.lock().unwrap_or_else(|p| p.into_inner());
    let dead = empty_root("aggregate-crash");
    let alive = empty_root("aggregate-alive");
    set_hook("MOIREN_SCANNER_TEST_CRASH", Some(&dead));
    let report = scan_sandboxed(&[dead.clone(), alive], &scanner_config());
    set_hook("MOIREN_SCANNER_TEST_CRASH", None);
    // The isolation contract: the crashed root costs exactly itself.
    assert_eq!(report.descriptors.len(), 0);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path, dead);
    assert!(report.failures[0].reason.contains("crashed"));
}
