//! The scanner half of the out-of-process scan sandbox (see
//! `moiren_plugins::sandbox`). The parent spawns this binary once per scan
//! root with `ROOT REPORT_PATH` as its arguments; this process enumerates,
//! loads and probes every plugin binary it finds under the root, writes the
//! JSON report to `REPORT_PATH`, and exits zero. Any crash, hang, or plugin
//! that takes the process down surfaces to the parent as a typed
//! `SandboxError` for this root only — never in the host process.
//!
//! The report travels through the file argument, not stdout: loaded plugins
//! may print to this process's stdout, and one stray write would corrupt a
//! stream-carried report.
//!
//! Two environment hooks exist for the sandbox test suite (they compare
//! against the root path and are inert for every other value):
//! `MOIREN_SCANNER_TEST_HANG=<root>` parks forever, and
//! `MOIREN_SCANNER_TEST_CRASH=<root>` exits with a distinctive code.

use std::{env, fs, io::Write as _, path::PathBuf};

use moiren_plugins::{
    instantiate,
    sandbox::{
        PROBE_MAX_BLOCK_FRAMES, PROBE_SAMPLE_RATE, SandboxDescriptor, SandboxFailure, SandboxReport,
    },
    scan,
};

/// The exit code of the deliberate test crash; the suite matches it. Kept
/// inside one byte: POSIX exit statuses truncate to the low 8 bits, so a
/// wider "distinctive" code would read differently across platforms.
const TEST_CRASH_EXIT_CODE: i32 = 0x5C;

fn main() {
    let mut arguments = env::args_os().skip(1);
    let (Some(root), Some(report_path)) = (arguments.next(), arguments.next()) else {
        // A malformed invocation is a caller bug, not a plugin failure.
        std::process::exit(2);
    };
    let root = PathBuf::from(root);
    if env::var_os("MOIREN_SCANNER_TEST_HANG").is_some_and(|hook| hook == root) {
        loop {
            std::thread::park();
        }
    }
    if env::var_os("MOIREN_SCANNER_TEST_CRASH").is_some_and(|hook| hook == root) {
        std::process::exit(TEST_CRASH_EXIT_CODE);
    }

    let outcome = scan::scan_directories(std::slice::from_ref(&root));
    let mut report = SandboxReport {
        descriptors: Vec::new(),
        failures: outcome
            .failures
            .into_iter()
            .map(|(path, reason)| SandboxFailure { path, reason })
            .collect(),
    };
    for descriptor in outcome.descriptors {
        // The probe is the dangerous half of a scan — it runs real plugin
        // code — which is exactly why it lives in this process.
        match instantiate(&descriptor, PROBE_SAMPLE_RATE, PROBE_MAX_BLOCK_FRAMES) {
            Ok(instance) => report.descriptors.push(SandboxDescriptor {
                parameters: instance.parameters().to_vec(),
                latency_frames: instance.latency_frames(),
                descriptor,
            }),
            Err(error) => report.failures.push(SandboxFailure {
                path: descriptor.path.clone(),
                reason: format!(
                    "{} {} could not be instantiated: {error}",
                    descriptor.format.label(),
                    descriptor.name
                ),
            }),
        }
    }

    let report_path = PathBuf::from(report_path);
    let write = || -> std::io::Result<()> {
        let mut file = fs::File::create(&report_path)?;
        serde_json::to_writer(&mut file, &report)?;
        file.flush()
    };
    if let Err(error) = write() {
        // Exit non-zero so the parent sees a crash rather than trusting a
        // half-written file; stderr carries the cause for anyone running the
        // scanner by hand (the parent attaches no stream to it).
        eprintln!("report write failed: {error}");
        std::process::exit(3);
    }
}
