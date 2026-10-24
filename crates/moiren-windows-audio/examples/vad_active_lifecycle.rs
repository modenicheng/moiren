//! Opt-in active-client retirement probe; stage 1 is NOT PCM qualification.

use std::ffi::OsString;

#[derive(Debug, PartialEq, Eq)]
struct Options {
    exercise: bool,
    hold_seconds: u32,
    help: bool,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> anyhow::Result<Self> {
        let mut options = Self {
            exercise: false,
            hold_seconds: 15,
            help: false,
        };
        let mut hold_seen = false;
        for argument in arguments {
            let argument = argument
                .into_string()
                .map_err(|_| anyhow::anyhow!("arguments must be valid UTF-8"))?;
            match argument.as_str() {
                "--exercise" if !options.exercise => options.exercise = true,
                "--help" | "-h" => options.help = true,
                value if value.starts_with("--hold-seconds=") && !hold_seen => {
                    options.hold_seconds = value["--hold-seconds=".len()..].parse()?;
                    anyhow::ensure!(
                        (1..=15).contains(&options.hold_seconds),
                        "--hold-seconds must be in 1..=15"
                    );
                    hold_seen = true;
                }
                _ => anyhow::bail!("unknown or repeated argument: {argument}"),
            }
        }
        anyhow::ensure!(
            !hold_seen || options.exercise || options.help,
            "--hold-seconds requires --exercise"
        );
        Ok(options)
    }
}

fn main() -> anyhow::Result<()> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    if options.help || !options.exercise {
        println!(
            "vad_active_lifecycle --exercise [--hold-seconds=15]\n\
             Stage 1: defaultsafe stereo microphone, exact owned MMDevice identity,\n\
             exclusive PCM16 capture RUN, gate open/close, active-client remove,\n\
             bounded old-client hold, release, final Removed and same-ID recreate.\n\
             No arguments or --help: help only; no owner handle or COM/audio client.\n\
             Holds the original client up to 1..=15 seconds after remove acceptance;\n\
             never opens a new session or recreates before final Removed.\n\
             NOT PCM/silence qualification, crash cleanup or default-policy qualification.\n\
             Never installs a driver or selects/modifies a default audio endpoint."
        );
        return Ok(());
    }
    run(options)
}

#[cfg(not(windows))]
fn run(_options: Options) -> anyhow::Result<()> {
    anyhow::bail!("the active-client VAD probe requires Windows")
}

#[cfg(windows)]
fn run(options: Options) -> anyhow::Result<()> {
    target::exercise(options.hold_seconds)
}

#[cfg(windows)]
#[path = "vad_active_lifecycle/target.rs"]
mod target;

#[cfg(any(windows, test))]
#[path = "vad_active_lifecycle/validation.rs"]
mod validation;

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> anyhow::Result<Options> {
        Options::parse(arguments.iter().map(Into::into))
    }

    #[test]
    fn default_has_no_owner_and_holds_for_at_most_fifteen_seconds() {
        assert_eq!(
            parse(&[]).unwrap(),
            Options {
                exercise: false,
                hold_seconds: 15,
                help: false,
            }
        );
    }

    #[test]
    fn exercise_is_explicit_and_help_always_stays_read_only() {
        assert!(parse(&["--exercise"]).unwrap().exercise);
        assert!(parse(&["--exercise", "--help"]).unwrap().help);
        assert!(parse(&["-h"]).unwrap().help);
    }

    #[test]
    fn hold_duration_requires_exercise_and_is_bounded() {
        assert_eq!(
            parse(&["--hold-seconds=1", "--exercise"])
                .unwrap()
                .hold_seconds,
            1
        );
        assert_eq!(
            parse(&["--exercise", "--hold-seconds=15"])
                .unwrap()
                .hold_seconds,
            15
        );
        for arguments in [
            vec!["--hold-seconds=1"],
            vec!["--exercise", "--hold-seconds=0"],
            vec!["--exercise", "--hold-seconds=16"],
            vec!["--exercise", "--hold-seconds=-1"],
            vec!["--exercise", "--hold-seconds=abc"],
            vec!["--exercise", "--hold-seconds=1", "--hold-seconds=2"],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
    }

    #[test]
    fn repeated_exercise_and_implicit_mutation_flags_are_rejected() {
        for arguments in [
            vec!["--exercise", "--exercise"],
            vec!["--set-default"],
            vec!["--endpoint=physical"],
            vec!["--install"],
            vec!["--crash"],
            vec!["--pcm"],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
    }
}
