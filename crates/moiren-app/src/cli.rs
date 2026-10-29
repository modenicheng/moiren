//! Application entry points share the desktop service, not the probe owners.
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeSet;

mod monitor;
mod options;

use crate::{AppConfig, OfflineApp};
use options::MonitorOptions;

const HELP: &str = "Moiren
Usage: moiren-app [--gain 0..16]
       moiren-app monitor --process PID --output ENDPOINT [OPTIONS]
Starts the GUI; --gain runs the offline software IO demo.
Use monitor --help for the finite Process -> Bus -> physical output route.";

const MONITOR_HELP: &str = "Moiren Process monitor (not a latency qualification)
Usage: moiren-app monitor --process PID --output ENDPOINT [OPTIONS]
  --seconds 1..600              Run duration after startup (default 30)
  --startup-timeout-ms 1..30000  Control-side startup deadline (default 15000)
  --engine-block-frames N       Supported Engine block size (default 256)
  --precision f32|f64           Engine processing precision (default f32)
  --output-buffer-frames N      Independent native output buffer request
  --output-mode shared|exclusive (default shared)
Explicit Process input budget (all five required together, in 48 kHz frames):
  --target-frames N --capacity-frames N --jitter-frames N
  --max-packet-frames N --max-gap-frames N
  --age-limit-100ns N           Requested sample age; currently refused
Delivery bounds are declared assumptions, not measured periods or latency.
Independent Process policy (compatible or declared; not measured mode):
  --max-ingress-elapsed-100ns N Positive host time since publication began;
                               not source sample age or physical latency
Measured Process input budget (exactly three budget fields; no packet/gap bounds):
  --measure-seconds 1..60       Measure a compatible baseline before applying
  --target-frames N --capacity-frames N --jitter-frames N
Two phases: baseline measurement -> owner replacement with measured admission
-> run for the requested --seconds. Measurement adds 1..60 seconds before that
run; startup and cleanup are separate. Sample age remains unsupported.
Measurement/admission is per-wake evidence, not a latency qualification or a
queue-stability/replay guarantee. The requested target may be refused.
No options changes saved desktop settings. Preparation may refuse unsafe
targets against actual native output demand. Completion is operational only.";

#[derive(Debug)]
enum Command {
    Gui,
    Offline(AppConfig),
    Monitor(MonitorOptions),
    Help(&'static str),
}

fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Command> {
    let mut args = arguments.into_iter().peekable();
    let Some(first) = args.next() else {
        return Ok(Command::Gui);
    };
    if matches!(first.as_str(), "--help" | "-h") {
        ensure!(
            args.next().is_none(),
            "--help cannot be combined with options"
        );
        return Ok(Command::Help(HELP));
    }
    if first == "--gain" {
        let gain: f64 = args
            .next()
            .context("--gain requires a value")?
            .parse()
            .context("invalid --gain value")?;
        ensure!(
            gain.is_finite() && (0.0..=16.0).contains(&gain),
            "--gain must be finite and in 0..16"
        );
        ensure!(args.next().is_none(), "unexpected argument after --gain");
        return Ok(Command::Offline(AppConfig {
            initial_gain: gain,
            ..AppConfig::default()
        }));
    }
    ensure!(first == "monitor", "unknown argument: {first}; use --help");
    if args
        .peek()
        .is_some_and(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        args.next();
        ensure!(
            args.next().is_none(),
            "monitor --help cannot be combined with options"
        );
        return Ok(Command::Help(MONITOR_HELP));
    }
    let mut seen = BTreeSet::new();
    let mut options = MonitorOptions::default();
    while let Some(flag) = args.next() {
        ensure!(seen.insert(flag.clone()), "duplicate option: {flag}");
        let value = args
            .next()
            .with_context(|| format!("{flag} requires a value"))?;
        options.set(&flag, &value)?;
    }
    options.validate()?;
    Ok(Command::Monitor(options))
}

/// Parse before native initialization. Help and invalid arguments never discover
/// a driver, start a GUI, open a stream or mutate saved machine-local settings.
pub fn run(arguments: impl IntoIterator<Item = String>) -> Result<()> {
    match parse(arguments)? {
        Command::Gui => crate::ui::run(),
        Command::Help(help) => {
            println!("{help}");
            Ok(())
        }
        Command::Offline(config) => {
            let mut app = OfflineApp::new(config).context("preparing the offline application")?;
            let input = [0.25, -0.5, 0.5, -0.25, 1.0, -1.0, 0.0, 0.0];
            let mut output = [0.0; 8];
            let report = app.process_interleaved(&input, &mut output)?;
            println!("processed: {report:?}\noutput: {output:?}");
            app.stop();
            Ok(())
        }
        Command::Monitor(options) => {
            let report = monitor::run(options)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if let Some(failure) = report.failure {
                bail!("Process monitor failed: {failure}");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ingress_elapsed_tests {
    use super::*;

    fn monitor(extra: &[&str]) -> Result<MonitorOptions> {
        let arguments = [
            "monitor",
            "--process",
            "4294967294",
            "--output",
            "pinned-output",
        ]
        .into_iter()
        .chain(extra.iter().copied())
        .map(String::from);
        let Command::Monitor(options) = parse(arguments)? else {
            unreachable!("monitor arguments select the monitor command")
        };
        Ok(options)
    }

    #[test]
    fn ingress_elapsed_cli_preserves_compatible_and_declared_policy() {
        let declared = [
            "--target-frames",
            "2048",
            "--capacity-frames",
            "8192",
            "--jitter-frames",
            "96",
            "--max-packet-frames",
            "480",
            "--max-gap-frames",
            "480",
        ];
        for limit in ["1", "100000", "18446744073709551615"] {
            for expert in [false, true] {
                let mut extra = vec!["--max-ingress-elapsed-100ns", limit];
                if expert {
                    extra.extend(declared);
                }
                let options = monitor(&extra).unwrap();
                let transport = options.transport().unwrap();
                assert_eq!(transport.process.is_some(), expert);
                let serialized = serde_json::to_value(transport).unwrap();
                assert_eq!(
                    serialized["process_max_ingress_elapsed_100ns"],
                    limit.parse::<u64>().unwrap()
                );
                if let Some(input) = transport.process {
                    assert_eq!(input.age_limit_100ns, None);
                    assert_eq!(input.target_frames, 2048);
                }
            }
        }
    }

    #[test]
    fn ingress_elapsed_cli_refuses_measured_instead_of_clearing_the_baseline_limit() {
        let error = monitor(&[
            "--measure-seconds",
            "1",
            "--target-frames",
            "256",
            "--capacity-frames",
            "8192",
            "--jitter-frames",
            "32",
            "--max-ingress-elapsed-100ns",
            "100000",
        ])
        .unwrap_err()
        .to_string();
        assert!(error.contains("measured"), "{error}");
        assert!(error.contains("ingress"), "{error}");
        assert!(!error.contains("unknown monitor option"), "{error}");
    }

    #[test]
    fn ingress_elapsed_cli_refuses_zero_malformed_duplicate_and_missing_values() {
        for invalid in ["0", "-1", "1.5", "NaN", "18446744073709551616"] {
            assert!(monitor(&["--max-ingress-elapsed-100ns", invalid]).is_err());
        }
        assert!(monitor(&["--max-ingress-elapsed-100ns"]).is_err());
        assert!(
            monitor(&[
                "--max-ingress-elapsed-100ns",
                "100000",
                "--max-ingress-elapsed-100ns",
                "200000",
            ])
            .is_err()
        );
        assert!(
            serde_json::to_value(monitor(&[]).unwrap().transport().unwrap())
                .unwrap()
                .get("process_max_ingress_elapsed_100ns")
                .is_none()
        );
    }
}
