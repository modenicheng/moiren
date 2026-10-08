//! Control-side parsing, available on all platforms for pure tests.
use crate::tone::{ToneConfig, ToneError};
use std::collections::BTreeSet;
use thiserror::Error;

pub const RENDER_HELP: &str = "Moiren test-tone output\nUsage:\n  moiren-app render --list\n  moiren-app render --endpoint <ID> [--seconds 1..600] [--frequency Hz] [--gain 0..1] [--pan -1..1]\nDefaults: 10 seconds, 440 Hz, gain 0.05, centered stereo. Native 48 kHz stereo f32 Shared only.";

#[derive(Debug)]
pub enum RenderCommand {
    List,
    Help,
    Tone {
        endpoint_id: String,
        seconds: u32,
        tone: ToneConfig,
    },
}
#[derive(Debug, Error)]
pub enum RenderCliError {
    #[error("render requires --endpoint <ID>; use render --list to choose one")]
    MissingEndpoint,
    #[error("{0} requires a value")]
    MissingValue(String),
    #[error("{0} was specified more than once")]
    DuplicateArgument(String),
    #[error("unknown render argument: {0}")]
    UnknownArgument(String),
    #[error("invalid value for {0}")]
    InvalidValue(String),
    #[error("--list and --help must be used alone")]
    ExclusiveMode,
    #[error(transparent)]
    Tone(#[from] ToneError),
}
pub fn parse_render_args(
    args: impl IntoIterator<Item = String>,
) -> Result<RenderCommand, RenderCliError> {
    let mut args = args.into_iter();
    let mut seen = BTreeSet::new();
    let mut list = false;
    let mut help = false;
    let mut endpoint_id = None;
    let mut seconds = 10u32;
    let mut tone = ToneConfig::default();
    while let Some(arg) = args.next() {
        let key = if arg == "-h" {
            "--help".to_owned()
        } else {
            arg
        };
        if !seen.insert(key.clone()) {
            return Err(RenderCliError::DuplicateArgument(key));
        }
        if key == "--list" {
            list = true;
            continue;
        }
        if key == "--help" {
            help = true;
            continue;
        }
        if !["--endpoint", "--seconds", "--frequency", "--gain", "--pan"].contains(&key.as_str()) {
            return Err(RenderCliError::UnknownArgument(key));
        }
        let value = args
            .next()
            .ok_or_else(|| RenderCliError::MissingValue(key.clone()))?;
        let invalid = || RenderCliError::InvalidValue(key.clone());
        match key.as_str() {
            "--endpoint" => {
                if value.trim().is_empty() || value.contains('\0') {
                    return Err(invalid());
                }
                endpoint_id = Some(value);
            }
            "--seconds" => {
                seconds = value.parse().map_err(|_| invalid())?;
                if !(1..=600).contains(&seconds) {
                    return Err(invalid());
                }
            }
            "--frequency" => tone.frequency_hz = value.parse().map_err(|_| invalid())?,
            "--gain" => tone.gain = value.parse().map_err(|_| invalid())?,
            "--pan" => tone.pan = value.parse().map_err(|_| invalid())?,
            _ => unreachable!("known value option"),
        }
    }
    if list || help {
        if seen.len() != 1 {
            return Err(RenderCliError::ExclusiveMode);
        }
        return Ok(if list {
            RenderCommand::List
        } else {
            RenderCommand::Help
        });
    }
    tone.validate()?;
    Ok(RenderCommand::Tone {
        endpoint_id: endpoint_id.ok_or(RenderCliError::MissingEndpoint)?,
        seconds,
        tone,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<RenderCommand, RenderCliError> {
        parse_render_args(args.iter().map(|s| (*s).to_owned()))
    }
    #[test]
    fn explicit_endpoint_defaults_and_list_are_unambiguous() {
        let RenderCommand::Tone {
            endpoint_id,
            seconds,
            tone,
        } = parse(&["--endpoint", "opaque ID"]).unwrap()
        else {
            panic!("tone expected")
        };
        assert_eq!(endpoint_id, "opaque ID");
        assert_eq!(seconds, 10);
        assert_eq!(tone.gain, 0.05);
        assert!(matches!(parse(&["--list"]), Ok(RenderCommand::List)));
        assert!(matches!(parse(&["-h"]), Ok(RenderCommand::Help)));
        assert!(matches!(parse(&[]), Err(RenderCliError::MissingEndpoint)));
    }
    #[test]
    fn invalid_or_ambiguous_cli_does_not_start_playback() {
        for args in [
            vec!["--endpoint"],
            vec!["--list", "--endpoint", "id"],
            vec!["--endpoint", "id", "--endpoint", "id"],
            vec!["--endpoint", "id", "--seconds", "0"],
            vec!["--endpoint", "id", "--seconds", "601"],
            vec!["--endpoint", "id", "--gain", "NaN"],
            vec!["--endpoint", "id", "--pan", "2"],
            vec!["--unknown"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
