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
