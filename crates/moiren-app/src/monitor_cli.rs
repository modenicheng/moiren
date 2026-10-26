//! Parsing is pure; no capture starts until both explicit selectors validate.
use super::monitor::{MonitorConfig, MonitorError, MonitorLatencyConfig, MonitorLatencyProfile};
use std::collections::BTreeSet;
use thiserror::Error;

pub const MONITOR_HELP: &str = "Moiren input monitoring\nUsage:\n  moiren-app monitor --list\n  moiren-app monitor --input <capture ID> --output <render ID> [--profile compatible|balanced|custom] [latency options]\n  moiren-app monitor --process <PID> --output <render ID> [--profile compatible]\nLatency options: --period-ms default|0.1..100, --input-target-frames 2..8190, --input-jitter-frames 0..8192, --render-target-frames 2..8192, --wake-margin-frames 0..8192, --callback-budget-frames 1..8192. Balanced defaults to input 1024, jitter 96, render 480, wake 96, callback 48 frames. Custom requires all latency frame options. Other defaults: 10 seconds, gain 0.05, centered stereo. Physical input: valid PCM/float mix formats are converted by Windows Shared to 48 kHz stereo f32, then clock-adapted. Process input: include target process tree, Windows-converted 48 kHz stereo f32; creation time pinned and checked at startup. Targets containing this host are rejected. Output: 48 kHz stereo f32 Shared, with Windows conversion for compatible endpoint mix formats. Linear SRC with bounded clock correction.";
#[derive(Debug)]
pub enum MonitorCommand {
    List,
    Help,
    Run {
        input_endpoint_id: String,
        output_endpoint_id: String,
        seconds: u32,
        config: MonitorConfig,
    },
    Process {
        pid: u32,
        output_endpoint_id: String,
        seconds: u32,
        config: MonitorConfig,
    },
}
#[derive(Debug, Error)]
pub enum MonitorCliError {
    #[error("monitor requires one of --input <ID> or --process <PID>, plus --output <ID>")]
    MissingEndpoint,
    #[error("--input and --process are mutually exclusive")]
    ConflictingInput,
    #[error("{0} requires a value")]
    MissingValue(String),
    #[error("{0} was specified more than once")]
    DuplicateArgument(String),
    #[error("unknown monitor argument: {0}")]
    UnknownArgument(String),
    #[error("invalid value for {0}")]
    InvalidValue(String),
    #[error("--list and --help must be used alone")]
    ExclusiveMode,
    #[error(transparent)]
    Config(#[from] MonitorError),
    #[error("process loopback supports only --profile compatible")]
    ProcessProfileUnsupported,
}
pub fn parse_monitor_args(
    args: impl IntoIterator<Item = String>,
) -> Result<MonitorCommand, MonitorCliError> {
    let mut args = args.into_iter();
    let mut seen = BTreeSet::new();
    let mut list = false;
    let mut help = false;
    let mut input = None;
    let mut process = None;
    let mut output = None;
    let mut seconds = 10u32;
    let mut config = MonitorConfig::default();
    let mut profile = MonitorLatencyProfile::Compatible;
    let mut period_ms = config.latency.shared_period_ms;
    let mut input_target_frames = None;
    let mut input_jitter_frames = None;
    let mut render_target_frames = None;
    let mut wake_margin_frames = None;
    let mut callback_budget_frames = None;
    while let Some(arg) = args.next() {
        let key = if arg == "-h" {
            "--help".to_owned()
        } else {
            arg
        };
        if !seen.insert(key.clone()) {
            return Err(MonitorCliError::DuplicateArgument(key));
        }
        if key == "--list" {
            list = true;
            continue;
        }
        if key == "--help" {
            help = true;
            continue;
        }
        if ![
            "--input",
            "--process",
            "--output",
            "--seconds",
            "--gain",
            "--pan",
            "--profile",
            "--period-ms",
            "--input-target-frames",
            "--input-jitter-frames",
            "--render-target-frames",
            "--wake-margin-frames",
            "--callback-budget-frames",
        ]
        .contains(&key.as_str())
        {
            return Err(MonitorCliError::UnknownArgument(key));
        }
        let value = args
            .next()
            .ok_or_else(|| MonitorCliError::MissingValue(key.clone()))?;
        let invalid = || MonitorCliError::InvalidValue(key.clone());
        match key.as_str() {
            "--process" => {
                let pid: u32 = value.parse().map_err(|_| invalid())?;
                if pid == 0 {
                    return Err(invalid());
                }
                process = Some(pid);
            }
            "--input" | "--output" => {
                if value.trim().is_empty() || value.contains('\0') {
                    return Err(invalid());
                }
                if key == "--input" {
                    input = Some(value);
                } else {
                    output = Some(value);
                }
            }
            "--seconds" => {
                seconds = value.parse().map_err(|_| invalid())?;
                if !(1..=600).contains(&seconds) {
                    return Err(invalid());
                }
            }
            "--gain" => config.gain = value.parse().map_err(|_| invalid())?,
            "--pan" => config.pan = value.parse().map_err(|_| invalid())?,
            "--profile" => {
                profile = match value.as_str() {
                    "compatible" => MonitorLatencyProfile::Compatible,
                    "balanced" => MonitorLatencyProfile::Balanced,
                    "custom" => MonitorLatencyProfile::Custom,
                    _ => return Err(invalid()),
                };
            }
            "--period-ms" => {
                period_ms = if value == "default" {
                    None
                } else {
                    Some(value.parse().map_err(|_| invalid())?)
                };
            }
            "--input-target-frames" => {
                input_target_frames = Some(value.parse().map_err(|_| invalid())?)
            }
            "--input-jitter-frames" => {
                input_jitter_frames = Some(value.parse().map_err(|_| invalid())?)
            }
            "--render-target-frames" => {
                render_target_frames = Some(value.parse().map_err(|_| invalid())?)
            }
            "--wake-margin-frames" => {
                wake_margin_frames = Some(value.parse().map_err(|_| invalid())?)
            }
            "--callback-budget-frames" => {
                callback_budget_frames = Some(value.parse().map_err(|_| invalid())?)
            }
            _ => unreachable!("known monitor option"),
        }
    }
    if list || help {
        if seen.len() != 1 {
            return Err(MonitorCliError::ExclusiveMode);
        }
        return Ok(if list {
            MonitorCommand::List
        } else {
            MonitorCommand::Help
        });
    }
    let defaults = match profile {
        MonitorLatencyProfile::Compatible => MonitorLatencyConfig::default(),
        MonitorLatencyProfile::Balanced => MonitorLatencyConfig::balanced(),
        MonitorLatencyProfile::Custom => MonitorLatencyConfig::custom(),
    };
    config.latency = MonitorLatencyConfig {
        profile,
        shared_period_ms: period_ms,
        input_target_frames: input_target_frames.or(defaults.input_target_frames),
        input_jitter_frames: input_jitter_frames.or(defaults.input_jitter_frames),
        render_target_frames: render_target_frames.or(defaults.render_target_frames),
        wake_margin_frames: wake_margin_frames.or(defaults.wake_margin_frames),
        callback_budget_frames: callback_budget_frames.or(defaults.callback_budget_frames),
    };
    config.validate()?;
    if input.is_some() && process.is_some() {
        return Err(MonitorCliError::ConflictingInput);
    }
    if let Some(pid) = process {
        return Ok(MonitorCommand::Process {
            pid,
            output_endpoint_id: output.ok_or(MonitorCliError::MissingEndpoint)?,
            seconds,
            config,
        });
    }
    Ok(MonitorCommand::Run {
        input_endpoint_id: input.ok_or(MonitorCliError::MissingEndpoint)?,
        output_endpoint_id: output.ok_or(MonitorCliError::MissingEndpoint)?,
        seconds,
        config,
    })
}
