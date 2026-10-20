//! Explicit target-only PCM validation. Ordinary tests never open audio devices.

use std::ffi::OsString;

#[cfg(any(windows, test))]
#[path = "vad_native/validation.rs"]
mod validation;

#[derive(Debug, PartialEq, Eq)]
struct Options {
    exercise: bool,
    seconds: u32,
    help: bool,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> anyhow::Result<Self> {
        let mut options = Self {
            exercise: false,
            seconds: 2,
            help: false,
        };
        let mut seconds_seen = false;
        for argument in arguments {
            let argument = argument
                .into_string()
                .map_err(|_| anyhow::anyhow!("arguments must be valid UTF-8"))?;
            match argument.as_str() {
                "--exercise" if !options.exercise => options.exercise = true,
                "--help" | "-h" => options.help = true,
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
        Ok(options)
    }
}

fn main() -> anyhow::Result<()> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    if options.help || !options.exercise {
        println!(
            "vad_native --exercise [--seconds=2]\n\
             Explicit installed-driver probe: four owned virtual endpoints,\n\
             MMDevice/adapter identity, exclusive 48 kHz PCM16/Float32,\n\
             mono/stereo in both directions and microphone gate-close silence.\n\
             --seconds: signal duration per case, 1..=60; eight cases total.\n\
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
    target::exercise(options.seconds)
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
}
