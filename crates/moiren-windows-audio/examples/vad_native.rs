//! Explicit target-only PCM validation. Ordinary tests never open audio devices.

use moiren_windows_audio::vad::Intent;
use std::ffi::OsString;

#[cfg(any(windows, test))]
#[path = "vad_native/validation.rs"]
mod validation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
enum NativeMode {
    #[serde(rename = "exclusive-event")]
    Event,
    #[serde(rename = "exclusive-timer")]
    Timer,
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    exercise: bool,
    seconds: u32,
    help: bool,
    mode: NativeMode,
    intent_filter: Option<Intent>,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> anyhow::Result<Self> {
        let mut options = Self {
            exercise: false,
            seconds: 2,
            help: false,
            mode: NativeMode::Event,
            intent_filter: None,
        };
        let mut seconds_seen = false;
        for argument in arguments {
            let argument = argument
                .into_string()
                .map_err(|_| anyhow::anyhow!("arguments must be valid UTF-8"))?;
            match argument.as_str() {
                "--exercise" if !options.exercise => options.exercise = true,
                "--help" | "-h" => options.help = true,
                "--timer" if options.mode == NativeMode::Event => options.mode = NativeMode::Timer,
                value if value.starts_with("--intent=") && options.intent_filter.is_none() => {
                    options.intent_filter = Some(match &value["--intent=".len()..] {
                        "playback" => Intent::Playback,
                        "microphone" => Intent::Microphone,
                        _ => anyhow::bail!("--intent must be playback or microphone"),
                    });
                }
                value if value.starts_with("--seconds=") && !seconds_seen => {
                    options.seconds = value["--seconds=".len()..].parse()?;
                    anyhow::ensure!(
                        (1..=60).contains(&options.seconds),
                        "--seconds must be in 1..=60"
                    );
                    seconds_seen = true;
                }
                _ => anyhow::bail!("unknown or repeated argument: {argument}"),
            }
        }
        anyhow::ensure!(
            !seconds_seen || options.exercise || options.help,
            "--seconds requires --exercise"
        );
        anyhow::ensure!(
            options.mode != NativeMode::Timer || options.exercise || options.help,
            "--timer requires --exercise"
        );
        anyhow::ensure!(
            options.intent_filter.is_none() || options.exercise || options.help,
            "--intent requires --exercise"
        );
        Ok(options)
    }
}

fn main() -> anyhow::Result<()> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    if options.help || !options.exercise {
        println!(
            "vad_native --exercise [--seconds=2] [--timer] [--intent=playback|microphone]\n\
             Explicit installed-driver probe: four owned virtual endpoints,\n\
             MMDevice/adapter identity, exclusive 48 kHz PCM16/Float32,\n\
             mono/stereo in both directions and microphone gate-close silence.\n\
             --seconds: signal duration per case, 1..=60; eight cases total.\n\
             --intent: diagnostic subset of four cases; report identifies the selected direction.\n\
             Default: exclusive event-driven; --timer: exclusive no-event,\n\
             padding/position-driven probe with no fallback if unsupported.\n\
             No arguments or --help: display this help without contacting a driver.\n\
             Never installs a driver, selects defaults or changes endpoint settings."
        );
        return Ok(());
    }
    run(options)
}

#[cfg(not(windows))]
fn run(_options: Options) -> anyhow::Result<()> {
    anyhow::bail!("the native VAD probe requires Windows")
}

#[cfg(windows)]
fn run(options: Options) -> anyhow::Result<()> {
    target::exercise(options.seconds, options.mode, options.intent_filter)
}

#[cfg(windows)]
#[path = "vad_native/target.rs"]
mod target;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<Options> {
        Options::parse(args.iter().map(Into::into))
    }

    #[test]
    fn default_and_help_do_not_contact_a_driver() {
        assert!(!parse(&[]).unwrap().exercise);
        assert_eq!(parse(&[]).unwrap().mode, NativeMode::Event);
        assert_eq!(parse(&["--exercise"]).unwrap().intent_filter, None);
        assert_eq!(parse(&["--exercise"]).unwrap().mode, NativeMode::Event);
        assert!(parse(&["--help"]).unwrap().help);
        assert!(parse(&["-h"]).unwrap().help);
    }

    #[test]
    fn exercise_is_explicit_and_duration_is_bounded() {
        assert!(parse(&["--exercise"]).unwrap().exercise);
        assert_eq!(parse(&["--seconds=60", "--exercise"]).unwrap().seconds, 60);
        for arguments in [
            vec!["--seconds=2"],
            vec!["--exercise", "--seconds=0"],
            vec!["--exercise", "--seconds=61"],
            vec!["--exercise", "--seconds=abc"],
            vec!["--exercise", "--seconds=1", "--seconds=2"],
            vec!["--exercise", "--exercise"],
            vec!["--set-default"],
            vec!["--endpoint=physical"],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
    }

    #[test]
    fn timer_mode_requires_explicit_exercise_and_cannot_repeat() {
        assert_eq!(
            parse(&["--exercise", "--timer"]).unwrap().mode,
            NativeMode::Timer
        );
        assert!(parse(&["--timer", "--seconds=1", "--exercise"]).is_ok());
        assert!(parse(&["--timer", "--help"]).unwrap().help);
        assert!(parse(&["--timer"]).is_err());
        assert!(parse(&["--exercise", "--timer", "--timer"]).is_err());
    }

    #[test]
    fn intent_selection_is_explicit_bounded_and_requires_exercise() {
        for (intent, expected) in [
            ("playback", Intent::Playback),
            ("microphone", Intent::Microphone),
        ] {
            let argument = format!("--intent={intent}");
            assert_eq!(
                parse(&["--exercise", &argument]).unwrap().intent_filter,
                Some(expected)
            );
            assert!(parse(&["--help", &argument]).is_ok());
            assert!(parse(&[&argument]).is_err());
            assert!(parse(&["--exercise", &argument, &argument]).is_err());
        }
        assert!(parse(&["--exercise", "--intent=other"]).is_err());
        assert!(parse(&["--exercise", "--intent=microphone", "--intent=playback"]).is_err());
    }
}
