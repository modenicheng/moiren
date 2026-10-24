use anyhow::{Context, bail, ensure};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Options {
    pub exercise: bool,
    pub pcm: bool,
    pub help: bool,
    pub timeout_seconds: u64,
    pub soak_idle_seconds: u64,
    pub soak_pcm_seconds: u64,
    /// Looping WAV material driving the PCM soak instead of alternating
    /// constant sources; requires a PCM soak.
    pub soak_pcm_file: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
}

impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut options = Self {
            exercise: false,
            pcm: false,
            help: false,
            timeout_seconds: 30,
            soak_idle_seconds: 0,
            soak_pcm_seconds: 0,
            soak_pcm_file: None,
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
                }
                value
                    if value.starts_with("--soak-idle-seconds=")
                        && options.soak_idle_seconds == 0 =>
                {
                    options.soak_idle_seconds =
                        soak_duration(&value["--soak-idle-seconds=".len()..])?;
                }
                value
                    if value.starts_with("--soak-pcm-seconds=")
                        && options.soak_pcm_seconds == 0 =>
                {
                    options.soak_pcm_seconds =
                        soak_duration(&value["--soak-pcm-seconds=".len()..])?;
                }
                value if value.starts_with("--output-dir=") && options.output_dir.is_none() => {
                    let path = &value["--output-dir=".len()..];
                    ensure!(
                        !path.trim().is_empty() && !path.contains('\0'),
                        "--output-dir requires a nonempty path without NUL"
                    );
                    options.output_dir = Some(path.into());
                }
                value
                    if value.starts_with("--soak-pcm-file=") && options.soak_pcm_file.is_none() =>
                {
                    let path = &value["--soak-pcm-file=".len()..];
                    ensure!(
                        !path.trim().is_empty() && !path.contains('\0'),
                        "--soak-pcm-file requires a nonempty path without NUL"
                    );
                    options.soak_pcm_file = Some(path.into());
                }
                _ => bail!("unknown or repeated argument: {argument}"),
            }
        }
        ensure!(
            options.exercise || options.help || (!timeout_seen && options.output_dir.is_none()),
            "--timeout-seconds and --output-dir require --exercise"
        );
        ensure!(
            options.exercise || (!options.pcm && !options.soaking()),
            "--pcm and soak durations require --exercise"
        );
        options.pcm |= options.soak_pcm_seconds > 0;
        if let Some(path) = &options.soak_pcm_file {
            ensure!(
                options.exercise && options.soak_pcm_seconds > 0,
                "--soak-pcm-file requires --exercise with --soak-pcm-seconds"
            );
            ensure!(
                path.is_file(),
                "--soak-pcm-file is not a readable file: {}",
                path.display()
            );
        }
        if options.soaking() {
            let requested = options
                .soak_idle_seconds
                .checked_add(options.soak_pcm_seconds)
                .ok_or_else(|| anyhow::anyhow!("aggregate soak duration overflow"))?;
            let minimum = requested.checked_add(90).context("soak timeout overflow")?;
            let maximum = requested
                .checked_add(3_600)
                .context("soak timeout overflow")?;
            if !timeout_seen {
                options.timeout_seconds = requested
                    .checked_add(180)
                    .context("soak timeout overflow")?;
            }
            ensure!(
                (minimum..=maximum).contains(&options.timeout_seconds),
                "soak timeout must be aggregate requested seconds + 90..=3600 seconds"
            );
        } else {
            ensure!(
                (10..=120).contains(&options.timeout_seconds),
                "--timeout-seconds must be in 10..=120 without soak"
            );
        }
        if options.pcm && !options.soaking() {
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

    pub fn soaking(&self) -> bool {
        self.soak_idle_seconds != 0 || self.soak_pcm_seconds != 0
    }

    #[cfg(any(windows, test))]
    pub fn work_budget(&self) -> std::time::Duration {
        let timeout = std::time::Duration::from_secs(self.timeout_seconds);
        if self.soaking() {
            timeout.saturating_sub(std::time::Duration::from_secs(30))
        } else {
            timeout.mul_f64(2.0 / 3.0)
        }
    }
}

fn soak_duration(value: &str) -> anyhow::Result<u64> {
    let seconds = value.parse()?;
    ensure!(
        (1..=28_800).contains(&seconds),
        "soak duration must be in 1..=28800 seconds"
    );
    Ok(seconds)
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

    #[test]
    fn soak_accepts_eight_hours_each_and_checked_aggregate_watchdog() {
        let options = parse(&[
            "--exercise",
            "--soak-idle-seconds=28800",
            "--soak-pcm-seconds=28800",
            "--timeout-seconds=57780",
        ])
        .expect("explicit eight-hour idle and PCM soak");
        assert!(options.pcm, "PCM soak implies native PCM exercise");
        assert_eq!(options.timeout_seconds, 57_780);
        assert_eq!(options.work_budget().as_secs(), 57_750);
        assert_eq!(
            parse(&["--exercise", "--soak-idle-seconds=8"])
                .unwrap()
                .timeout_seconds,
            188
        );
        assert!(!parse(&["--exercise", "--soak-idle-seconds=8"]).unwrap().pcm);
    }

    #[test]
    fn soak_rejects_missing_opt_in_overflow_duplicates_and_short_aggregate_budget() {
        for invalid in [
            vec!["soak-idle-seconds=10"],
            vec!["soak-pcm-seconds=10"],
            vec!["--exercise", "--soak-idle-seconds=0"],
            vec!["--exercise", "--soak-pcm-seconds=28801"],
            vec!["--exercise", "--soak-idle-seconds=18446744073709551615"],
            vec!["--exercise", "--soak-pcm-seconds=18446744073709551616"],
            vec![
                "--exercise",
                "--soak-idle-seconds=1",
                "--soak-idle-seconds=2",
            ],
            vec!["--exercise", "--soak-pcm-seconds=1", "--soak-pcm-seconds=2"],
            vec![
                "--exercise",
                "--soak-idle-seconds=28800",
                "--timeout-seconds=28900",
                "--soak-pcm-seconds=28800",
            ],
            vec!["--exercise", "--soak-pcm-seconds=1", "--timeout-seconds=90"],
        ] {
            assert!(parse(&invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn music_material_requires_exercise_pcm_soak_and_a_readable_file() {
        let material =
            std::env::temp_dir().join(format!("moiren-soak-material-{}", std::process::id()));
        std::fs::write(&material, b"RIFF").unwrap();
        let file = material.display().to_string();
        let accepted = parse(&[
            "--exercise",
            "--soak-pcm-seconds=60",
            &format!("--soak-pcm-file={file}"),
        ])
        .unwrap();
        assert_eq!(accepted.soak_pcm_file, Some(material.clone()));
        for invalid in [
            vec![format!("--soak-pcm-file={file}")],
            vec!["--exercise".into(), format!("--soak-pcm-file={file}")],
            vec![
                "--exercise".into(),
                "--soak-idle-seconds=8".into(),
                format!("--soak-pcm-file={file}"),
            ],
            vec![
                "--exercise".into(),
                "--soak-pcm-seconds=8".into(),
                "--soak-pcm-file=".into(),
            ],
            vec![
                "--exercise".into(),
                "--soak-pcm-seconds=8".into(),
                format!("--soak-pcm-file={file}"),
                format!("--soak-pcm-file={file}"),
            ],
            vec![
                "--exercise".into(),
                "--soak-pcm-seconds=8".into(),
                format!("--soak-pcm-file={}\\0", material.display()),
            ],
            vec![
                "--exercise".into(),
                "--soak-pcm-seconds=8".into(),
                format!(
                    "--soak-pcm-file={}\\missing.wav",
                    std::env::temp_dir().display()
                ),
            ],
        ] {
            let arguments = invalid.iter().map(String::as_str).collect::<Vec<_>>();
            assert!(parse(&arguments).is_err(), "accepted {invalid:?}");
        }
        std::fs::remove_file(&material).unwrap();
    }
}
