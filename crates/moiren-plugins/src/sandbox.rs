//! Out-of-process plugin scanning.
//!
//! Loading a third-party plugin binary runs its module entry code, so a
//! misbehaving plugin can crash or hang the process that hosts it. Scanning
//! therefore never loads a plugin in this process: the parent spawns the
//! `moiren-plugin-scanner` subprocess once per scan root with a deadline,
//! and the child enumerates, loads and probes every candidate binary it
//! finds there. A scanner that crashes or exceeds its deadline costs exactly
//! one root — the failure is recorded as a typed [`SandboxError`] and the
//! remaining roots scan on; the parent process stays untouched.
//!
//! The child hands its report back through a temp file (passed as its second
//! argument), not stdout: plugins are free to print to their own stdout, and
//! one `printf` inside a loaded module would corrupt a stdout-carried JSON
//! report.

use std::{
    fmt,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::spec::{PluginDescriptor, PluginParameterSpec};

/// The scanner executable this crate ships beside the host binary.
pub fn scanner_binary_name() -> String {
    format!("moiren-plugin-scanner{}", std::env::consts::EXE_SUFFIX)
}

/// How long one scan root may take before the parent kills the child. A
/// healthy root enumerates and probes every plugin in well under this; a
/// plugin that deadlocks its factory or instantiation is the target.
pub const DEFAULT_ROOT_TIMEOUT: Duration = Duration::from_secs(30);

/// The sample-rate/block-size pair the scanner probes instances at. The
/// collected parameter tables do not depend on either value; they only need
/// to match what a fresh instance reports, which they do by construction.
pub const PROBE_SAMPLE_RATE: f64 = 48_000.0;
pub const PROBE_MAX_BLOCK_FRAMES: u32 = 256;

/// One probed plugin: the scanned descriptor plus the table only a live
/// instance can report. The probe ran in the scanner subprocess, so this
/// table is guaranteed to match a fresh instance of the same binary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SandboxDescriptor {
    pub descriptor: PluginDescriptor,
    pub parameters: Vec<PluginParameterSpec>,
    pub latency_frames: u32,
}

/// A per-path scan or probe failure, in the same shape the in-process
/// scanner's `ScanOutcome` collects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxFailure {
    pub path: PathBuf,
    pub reason: String,
}

/// Everything one sandboxed scan produced.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SandboxReport {
    pub descriptors: Vec<SandboxDescriptor>,
    pub failures: Vec<SandboxFailure>,
}

/// Why one root's sandboxed scan did not produce a report. Typed so callers
/// can distinguish "this plugin is broken" (per-path failures inside a
/// report) from "the sandbox for this root died" (this error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// The scanner process could not be spawned.
    Spawn(String),
    /// The scanner exceeded its deadline and was killed.
    Timeout,
    /// The scanner exited non-zero: it crashed, or one of the plugins it
    /// loaded took the process down with it.
    Crashed(Option<i32>),
    /// The scanner exited zero but its report could not be read or parsed.
    InvalidReport(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "scanner process failed to start: {error}"),
            Self::Timeout => write!(f, "scanner exceeded its deadline and was killed"),
            Self::Crashed(code) => match code {
                Some(code) => write!(f, "scanner crashed (exit code {code})"),
                None => write!(f, "scanner crashed (no exit code)"),
            },
            Self::InvalidReport(error) => write!(f, "scanner report unreadable: {error}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// How the parent runs the scanner: which executable, and one root's
/// deadline.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub scanner: PathBuf,
    pub timeout: Duration,
}

impl SandboxConfig {
    pub fn new(scanner: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            scanner: scanner.into(),
            timeout,
        }
    }

    /// The deployed layout: the scanner sits beside the running host
    /// binary. `Err` describes why the sibling could not be resolved, so a
    /// missing scanner surfaces as a typed spawn failure instead of a
    /// silent empty library.
    pub fn discover() -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        let scanner = executable
            .parent()
            .ok_or_else(|| io::Error::other("the host executable has no parent directory"))?
            .join(scanner_binary_name());
        Ok(Self::new(scanner, DEFAULT_ROOT_TIMEOUT))
    }
}

/// Scan one root in the scanner subprocess and return its report. The
/// caller-facing error carries the root so an aggregate scan can place the
/// failure without a second lookup.
pub fn scan_root_sandboxed(
    root: &Path,
    config: &SandboxConfig,
) -> Result<SandboxReport, (PathBuf, SandboxError)> {
    let fail = |error| Err((root.to_path_buf(), error));
    let report_path = unique_report_path();
    let mut child = match Command::new(&config.scanner)
        .arg(root)
        .arg(&report_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return fail(SandboxError::Spawn(error.to_string())),
    };
    // Poll-wait with a deadline: std has no timed wait, and a plugin that
    // deadlocks the child must not wedge the parent's scan.
    let deadline = Instant::now() + config.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => return fail(SandboxError::Spawn(error.to_string())),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&report_path);
            return fail(SandboxError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let read_report = || -> io::Result<SandboxReport> {
        let mut bytes = Vec::new();
        std::fs::File::open(&report_path)?.read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    };
    let report = match read_report() {
        Ok(report) => report,
        Err(error) => {
            let _ = std::fs::remove_file(&report_path);
            if !status.success() {
                // Prefer the process fate over the (then meaningless) missing
                // or truncated report file.
                return fail(SandboxError::Crashed(status.code()));
            }
            return fail(SandboxError::InvalidReport(error.to_string()));
        }
    };
    let _ = std::fs::remove_file(&report_path);
    if !status.success() {
        // A scanner that writes a report and still exits non-zero crashed
        // after (or while) writing it; the report cannot be trusted.
        return fail(SandboxError::Crashed(status.code()));
    }
    Ok(report)
}

/// Scan every root, one sandboxed subprocess per root. A root whose scanner
/// crashed or timed out becomes one failure entry; the remaining roots scan
/// on and their findings are merged. This is the isolation contract: the
/// worst a plugin can do to a library scan is lose its own root.
pub fn scan_sandboxed(roots: &[PathBuf], config: &SandboxConfig) -> SandboxReport {
    let mut report = SandboxReport::default();
    for root in roots {
        match scan_root_sandboxed(root, config) {
            Ok(part) => {
                report.descriptors.extend(part.descriptors);
                report.failures.extend(part.failures);
            }
            Err((path, error)) => report.failures.push(SandboxFailure {
                path,
                reason: error.to_string(),
            }),
        }
    }
    report
}

/// A collision-free report path in the system temp directory. The parent
/// owns the name and deletes the file after reading; a leftover from a
/// killed run is inert garbage the OS temp policy reaps.
fn unique_report_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "moiren-plugin-scan-{}-{serial}.json",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_round_trips_through_json() {
        let report = SandboxReport {
            descriptors: vec![SandboxDescriptor {
                descriptor: PluginDescriptor {
                    format: crate::spec::PluginFormat::Clap,
                    path: PathBuf::from("/plugins/gain.clap"),
                    id: "org.example.gain".into(),
                    name: "Gain".into(),
                    vendor: "Example".into(),
                    version: "1.0".into(),
                    input_channels: 2,
                    output_channels: 2,
                    is_synth: false,
                    has_editor: false,
                },
                parameters: vec![PluginParameterSpec {
                    id: 7,
                    name: "Gain".into(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.5,
                    automatable: true,
                    stepped: false,
                }],
                latency_frames: 0,
            }],
            failures: vec![SandboxFailure {
                path: PathBuf::from("/plugins/broken.clap"),
                reason: "plugin module failed to load".into(),
            }],
        };
        let bytes = serde_json::to_vec(&report).unwrap();
        assert_eq!(
            serde_json::from_slice::<SandboxReport>(&bytes).unwrap(),
            report
        );
    }

    #[test]
    fn report_paths_are_unique_per_call() {
        assert_ne!(unique_report_path(), unique_report_path());
    }
}
