//! Parsing is pure; no capture starts until both explicit selectors validate.
use super::monitor::{MonitorConfig, MonitorError};
use std::collections::BTreeSet;
use thiserror::Error;

pub const MONITOR_HELP: &str = "Moiren input monitoring\nUsage:\n  moiren-app monitor --list\n  moiren-app monitor --input <capture ID> --output <render ID> [--seconds 1..600] [--gain 0..1] [--pan -1..1]\n  moiren-app monitor --process <PID> --output <render ID> [--seconds 1..600] [--gain 0..1] [--pan -1..1]\nDefaults: 10 seconds, gain 0.05, centered stereo. Physical input: native 44.1/48 kHz mono/stereo f32. Process input: include target process tree, Windows-converted 48 kHz stereo f32; creation time pinned and checked at startup. Targets containing this host are rejected. Output: 48 kHz stereo f32 Shared. Linear SRC with bounded clock correction.";
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
