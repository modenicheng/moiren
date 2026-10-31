use anyhow::{Context, bail, ensure};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Options {
    pub exercise: bool,
    pub pcm: bool,
    pub rename: bool,
    pub rename_fixture_seed: Option<u64>,
    pub help: bool,
    pub timeout_seconds: u64,
    pub soak_idle_seconds: u64,
    pub soak_pcm_seconds: u64,
    /// Looping WAV material driving the PCM soak instead of alternating
    /// constant sources; requires a PCM soak.
    pub soak_pcm_file: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub input_capture_frames: Option<u32>,
}

impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut options = Self {
            exercise: false,
            pcm: false,
            rename: false,
            rename_fixture_seed: None,
            help: false,
            timeout_seconds: 30,
            soak_idle_seconds: 0,
            soak_pcm_seconds: 0,
            soak_pcm_file: None,
            output_dir: None,
            input_capture_frames: None,
        };
        let mut timeout_seen = false;
        for argument in arguments {
            match argument.as_str() {
                "--exercise" if !options.exercise => options.exercise = true,
                "--pcm" if !options.pcm => options.pcm = true,
                "--rename" if !options.rename => options.rename = true,
                value
                    if value.starts_with("--rename-fixture-seed=")
                        && options.rename_fixture_seed.is_none() =>
                {
                    let value = &value["--rename-fixture-seed=".len()..];
                    ensure!(
                        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
                        "--rename-fixture-seed requires a positive decimal u64"
                    );
                    let seed: u64 = value.parse().context("--rename-fixture-seed exceeds u64")?;
                    ensure!(seed > 0, "--rename-fixture-seed must be positive");
                    options.rename_fixture_seed = Some(seed);
                }
                "--help" | "-h" => options.help = true,
                value
                    if value.starts_with("--input-capture-frames=")
                        && options.input_capture_frames.is_none() =>
                {
                    let value = &value["--input-capture-frames=".len()..];
                    ensure!(
                        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                        "--input-capture-frames requires decimal 1..=480000"
                    );
                    let frames: u32 = value.parse()?;
                    ensure!(
                        (1..=480_000).contains(&frames),
                        "--input-capture-frames must be 1..=480000"
                    );
                    options.input_capture_frames = Some(frames);
                }
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
        ensure!(
            !options.rename || options.exercise,
            "--rename requires --exercise"
        );
        ensure!(
            options.rename_fixture_seed.is_none() || (options.exercise && options.rename),
            "--rename-fixture-seed requires --exercise --rename"
        );
        ensure!(
            !options.rename
                || (!options.pcm && !options.soaking() && options.soak_pcm_file.is_none()),
            "--rename cannot be combined with --pcm or any soak option"
        );
        options.pcm |= options.soak_pcm_seconds > 0;
        ensure!(
            options.input_capture_frames.is_none() || (options.exercise && !options.rename),
            "--input-capture-frames requires --exercise and cannot accompany --rename"
        );
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
        if (options.pcm || options.rename) && !options.soaking() {
            if !timeout_seen {
                options.timeout_seconds = 90;
            }
            ensure!(
                options.timeout_seconds >= 60,
                "{} needs an overall timeout of at least 60 seconds",
                if options.rename { "--rename" } else { "--pcm" }
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
    fn input_capture_is_explicit_bounded_and_does_not_start_native_clients() {
        for arguments in [vec![], vec!["--help"], vec!["--exercise"]] {
            assert_eq!(parse(&arguments).unwrap().input_capture_frames, None);
        }
        for frames in [1, 480_000] {
            let argument = format!("--input-capture-frames={frames}");
            let options = parse(&["--exercise", &argument]).unwrap();
            assert_eq!(options.input_capture_frames, Some(frames));
            assert!(!options.pcm && !options.rename);
        }
        for arguments in [
            vec!["--input-capture-frames=1"],
            vec!["--help", "--input-capture-frames=1"],
            vec!["--exercise", "--input-capture-frames=0"],
            vec!["--exercise", "--input-capture-frames=480001"],
            vec!["--exercise", "--input-capture-frames=+1"],
            vec!["--exercise", "--input-capture-frames=-1"],
            vec!["--exercise", "--input-capture-frames="],
            vec!["--exercise", "--input-capture-frames=4294967296"],
            vec![
                "--exercise",
                "--input-capture-frames=1",
                "--input-capture-frames=2",
            ],
            vec!["--exercise", "--rename", "--input-capture-frames=1"],
        ] {
            assert!(parse(&arguments).is_err(), "accepted {arguments:?}");
        }
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
    fn rename_is_explicit_exclusive_and_reserves_cleanup_budget() {
        let options = parse(&["--exercise", "--rename"]).expect("explicit rename probe");
        assert_eq!(options.timeout_seconds, 90);
        assert_eq!(options.work_budget().as_secs(), 60);
        assert_eq!(
            parse(&["--rename", "--exercise", "--timeout-seconds=60"])
                .unwrap()
                .timeout_seconds,
            60
        );
        for invalid in [
            vec!["--rename"],
            vec!["--help", "--rename"],
            vec!["--exercise", "--rename", "--rename"],
            vec!["--exercise", "--rename", "--pcm"],
            vec!["--exercise", "--pcm", "--rename"],
            vec!["--exercise", "--rename", "--soak-idle-seconds=1"],
            vec!["--exercise", "--rename", "--soak-pcm-seconds=1"],
            vec!["--exercise", "--rename", "--soak-pcm-file=missing.wav"],
            vec!["--exercise", "--rename", "--timeout-seconds=59"],
        ] {
            assert!(parse(&invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn rename_fixture_seed_is_explicit_positive_and_requires_exercise_rename() {
        for seed in [1, 2, 0x0102_0304_0506_0708, u64::MAX] {
            let argument = format!("--rename-fixture-seed={seed}");
            let options = parse(&[&argument, "--rename", "--exercise"]).unwrap();
            assert_eq!(options.rename_fixture_seed, Some(seed));
            assert_eq!(options.timeout_seconds, 90);
        }
        for arguments in [
            vec![],
            vec!["--exercise"],
            vec!["--exercise", "--rename"],
            vec!["--exercise", "--pcm"],
        ] {
            assert_eq!(parse(&arguments).unwrap().rename_fixture_seed, None);
        }
    }

    #[test]
    fn rename_fixture_seed_rejects_zero_invalid_repeated_and_nonrename_arguments() {
        for value in [
            "0",
            "-1",
            "+1",
            "",
            "abc",
            "1.0",
            "0x01",
            " 1",
            "1 ",
            "18446744073709551616",
        ] {
            let argument = format!("--rename-fixture-seed={value}");
            assert!(
                parse(&["--exercise", "--rename", &argument]).is_err(),
                "accepted {argument:?}"
            );
        }
        for invalid in [
            vec!["--rename-fixture-seed=1"],
            vec!["--help", "--rename-fixture-seed=1"],
            vec!["--rename", "--rename-fixture-seed=1"],
            vec!["--exercise", "--rename-fixture-seed=1"],
            vec!["--exercise", "--pcm", "--rename-fixture-seed=1"],
            vec![
                "--exercise",
                "--soak-idle-seconds=1",
                "--rename-fixture-seed=1",
            ],
            vec!["--exercise", "--rename", "--pcm", "--rename-fixture-seed=1"],
            vec![
                "--exercise",
                "--rename",
                "--soak-pcm-seconds=1",
                "--rename-fixture-seed=1",
            ],
            vec![
                "--exercise",
                "--rename",
                "--rename-fixture-seed=1",
                "--rename-fixture-seed=2",
            ],
            vec![
                "--exercise",
                "--rename",
                "--rename-fixture-seed=1",
                "--rename-fixture-seed=1",
            ],
        ] {
            assert!(parse(&invalid).is_err(), "accepted {invalid:?}");
        }
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
