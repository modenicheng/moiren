use anyhow::{bail, ensure};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Options {
    pub exercise: bool,
    pub pcm: bool,
    pub help: bool,
    pub timeout_seconds: u64,
    pub output_dir: Option<PathBuf>,
}

impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut options = Self {
            exercise: false,
            pcm: false,
            help: false,
            timeout_seconds: 30,
            output_dir: None,
        };
        let mut timeout_seen = false;
        for argument in arguments {
            match argument.as_str() {
                "--exercise" if !options.exercise => options.exercise = true,
                "--pcm" if !options.pcm => options.pcm = true,
                "--help" | "-h" => options.help = true,
                value if value.starts_with("--timeout-seconds=") && !timeout_seen => {
                    timeout_seen = true;
                    options.timeout_seconds = value["--timeout-seconds=".len()..].parse()?;
                    ensure!(
                        (10..=120).contains(&options.timeout_seconds),
                        "--timeout-seconds must be in 10..=120"
                    );
                }
                value if value.starts_with("--output-dir=") && options.output_dir.is_none() => {
                    let path = &value["--output-dir=".len()..];
                    ensure!(
                        !path.trim().is_empty() && !path.contains('\0'),
                        "--output-dir requires a nonempty path without NUL"
                    );
                    options.output_dir = Some(path.into());
                }
                _ => bail!("unknown or repeated argument: {argument}"),
            }
        }
        ensure!(
            options.exercise || options.help || (!timeout_seen && options.output_dir.is_none()),
            "--timeout-seconds and --output-dir require --exercise"
        );
        ensure!(
            options.exercise || !options.pcm,
            "--pcm requires --exercise"
        );
        if options.pcm {
            if !timeout_seen {
                options.timeout_seconds = 90;
            }
            ensure!(
                options.timeout_seconds >= 60,
                "--pcm needs an overall timeout of at least 60 seconds"
            );
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> anyhow::Result<Options> {
        Options::parse(arguments.iter().map(|argument| (*argument).into()))
    }

    #[test]
    fn default_and_help_never_select_driver_exercise() {
        assert!(!parse(&[]).unwrap().exercise);
        assert!(parse(&["--help"]).unwrap().help);
        assert!(parse(&["-h"]).unwrap().help);
    }

    #[test]
    fn exercise_and_bounded_timeout_are_explicit() {
        let options = parse(&["--exercise", "--timeout-seconds=10"]).unwrap();
        assert!(options.exercise);
        assert_eq!(options.timeout_seconds, 10);
        assert_eq!(parse(&["--exercise"]).unwrap().timeout_seconds, 30);
        assert_eq!(
            parse(&["--timeout-seconds=120", "--exercise"])
                .unwrap()
                .timeout_seconds,
            120
        );
        for invalid in [
            vec!["--timeout-seconds=10"],
            vec!["--exercise", "--timeout-seconds=9"],
            vec!["--exercise", "--timeout-seconds=121"],
            vec!["--exercise", "--timeout-seconds=-1"],
            vec!["--exercise", "--timeout-seconds=oops"],
            vec!["--exercise", "--timeout-seconds=10", "--timeout-seconds=11"],
            vec!["--exercise", "--exercise"],
            vec!["--set-default"],
            vec!["--timer"],
            vec!["--install"],
        ] {
            assert!(parse(&invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn output_directory_requires_exercise_and_cannot_repeat_or_be_empty() {
        assert_eq!(
            parse(&["--exercise", "--output-dir=probe-output"])
                .unwrap()
                .output_dir,
            Some(PathBuf::from("probe-output"))
        );
        for invalid in [
            vec!["--output-dir=probe-output"],
            vec!["--exercise", "--output-dir="],
            vec!["--exercise", "--output-dir=\0"],
            vec!["--exercise", "--output-dir=a", "--output-dir=b"],
        ] {
            assert!(parse(&invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn pcm_is_opt_in_and_reserves_a_long_overall_watchdog() {
        assert!(parse(&["--pcm"]).is_err());
        assert!(parse(&["--exercise", "--pcm"]).is_ok());
        assert_eq!(parse(&["--exercise", "--pcm"]).unwrap().timeout_seconds, 90);
        assert_eq!(
            parse(&["--pcm", "--timeout-seconds=60", "--exercise"])
                .unwrap()
                .timeout_seconds,
            60
        );
        assert!(parse(&["--exercise", "--pcm", "--timeout-seconds=59"]).is_err());
        assert!(parse(&["--exercise", "--pcm", "--pcm"]).is_err());
    }
}
