//! Explicit target-only PCM validation. Ordinary tests never open audio devices.

use moiren_windows_audio::vad::Intent;
use std::ffi::OsString;

#[cfg(any(windows, test))]
#[path = "vad_native/period.rs"]
mod period;

#[cfg(any(windows, test))]
#[path = "vad_native/validation.rs"]
mod validation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
enum NativeMode {
    #[serde(rename = "exclusive-event")]
    Event,
    #[serde(rename = "exclusive-timer")]
    Timer,
    #[serde(rename = "shared-capture-pcm16")]
    SharedCapture,
    #[serde(rename = "shared-raw-capture-pcm16")]
    SharedRawCapture,
}

impl NativeMode {
    fn is_shared_capture(self) -> bool {
        matches!(self, Self::SharedCapture | Self::SharedRawCapture)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    exercise: bool,
    seconds: u32,
    help: bool,
    mode: NativeMode,
    intent_filter: Option<Intent>,
    minimum_period: bool,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> anyhow::Result<Self> {
        let mut options = Self {
            exercise: false,
            seconds: 2,
            help: false,
            mode: NativeMode::Event,
            intent_filter: None,
            minimum_period: false,
        };
        let mut seconds_seen = false;
        let mut raw = false;
        for argument in arguments {
            let argument = argument
                .into_string()
                .map_err(|_| anyhow::anyhow!("arguments must be valid UTF-8"))?;
            match argument.as_str() {
                "--exercise" if !options.exercise => options.exercise = true,
                "--help" | "-h" => options.help = true,
                "--timer" if options.mode == NativeMode::Event => options.mode = NativeMode::Timer,
                "--shared-microphone" if options.mode == NativeMode::Event => {
                    options.mode = NativeMode::SharedCapture;
                }
                "--minimum-period" if !options.minimum_period => options.minimum_period = true,
                "--raw" if !raw => raw = true,
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
            !options.minimum_period || options.exercise || options.help,
            "--minimum-period requires --exercise or --help"
        );
        anyhow::ensure!(
            options.intent_filter.is_none() || options.exercise || options.help,
            "--intent requires --exercise"
        );
        anyhow::ensure!(
            !raw || options.mode == NativeMode::SharedCapture,
            "--raw requires --shared-microphone"
        );
        if raw {
            options.mode = NativeMode::SharedRawCapture;
        }
        if options.mode.is_shared_capture() {
            anyhow::ensure!(
                options.exercise || options.help,
                "--shared-microphone requires --exercise or --help"
            );
            anyhow::ensure!(
                options.help || options.intent_filter == Some(Intent::Microphone),
                "--shared-microphone requires --intent=microphone"
            );
            anyhow::ensure!(
                !options.minimum_period,
                "--shared-microphone cannot request an exclusive minimum period"
            );
        }
        Ok(options)
    }
}

fn main() -> anyhow::Result<()> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    if options.help || !options.exercise {
        println!(
            "vad_native --exercise [--seconds=2] [--timer] [--intent=playback|microphone] [--minimum-period]\n\
             vad_native --exercise --shared-microphone --intent=microphone [--seconds=2] [--raw]\n\
             Explicit installed-driver probe: four owned virtual endpoints,\n\
             MMDevice/adapter identity, exclusive 48 kHz PCM16/Float32,\n\
             mono/stereo in both directions and microphone gate-close silence.\n\
             --seconds: signal duration per case, 1..=60; eight cases total.\n\
             --intent: diagnostic subset of four cases; report identifies the selected direction.\n\
             Default: exclusive event-driven; --timer: exclusive no-event,\n\
             padding/position-driven probe with no fallback if unsupported.\n\
             Default period: max(device default, 10 ms), unchanged baseline.\n\
             --minimum-period: request the queried exclusive minimum, without clamp/fallback.\n\
             Requests, buffer capacity and driver packets are separate evidence; always unqualified.\n\
             --shared-microphone: one owned stereo PCM16 Shared capture with direct known-pattern WRITE,\n\
             exact 48 kHz/480-frame driver shape; no Engine/Bus, format or mode fallback.\n\
             --raw: explicit Shared Other/RAW processing control; failure never falls back.\n\
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
    target::exercise(
        options.seconds,
        options.mode,
        options.intent_filter,
        options.minimum_period,
    )
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
    fn raw_shared_microphone_is_explicit_and_never_an_automatic_fallback() {
        let raw = parse(&[
            "--exercise",
            "--shared-microphone",
            "--intent=microphone",
            "--raw",
        ])
        .unwrap();
        assert_eq!(
            serde_json::to_value(raw.mode).unwrap(),
            "shared-raw-capture-pcm16"
        );
        for args in [
            vec!["--exercise", "--raw"],
            vec!["--raw"],
            vec!["--exercise", "--timer", "--raw"],
            vec![
                "--exercise",
                "--shared-microphone",
                "--intent=microphone",
                "--raw",
                "--raw",
            ],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        assert_eq!(
            parse(&["--exercise", "--shared-microphone", "--intent=microphone"])
                .unwrap()
                .mode,
            NativeMode::SharedCapture
        );
    }

    #[test]
    fn default_and_help_do_not_contact_a_driver() {
        assert!(!parse(&[]).unwrap().exercise);
        assert_eq!(parse(&[]).unwrap().mode, NativeMode::Event);
        assert!(!parse(&[]).unwrap().minimum_period);
        assert!(!parse(&["--exercise"]).unwrap().minimum_period);
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
    fn minimum_period_is_explicit_once_and_requires_exercise_or_help() {
        for arguments in [
            vec!["--exercise", "--minimum-period"],
            vec!["--minimum-period", "--exercise", "--timer"],
            vec!["--minimum-period", "--help"],
            vec!["-h", "--minimum-period"],
        ] {
            assert!(parse(&arguments).unwrap().minimum_period, "{arguments:?}");
        }
        for arguments in [
            vec!["--minimum-period"],
            vec!["--exercise", "--minimum-period", "--minimum-period"],
            vec!["--help", "--minimum-period", "--minimum-period"],
            vec!["--exercise", "--minimum-period=1"],
            vec!["--exercise", "--minimum-period=false"],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
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

    #[test]
    fn shared_microphone_diagnostic_is_explicit_and_cannot_fallback() {
        for arguments in [
            vec!["--exercise", "--shared-microphone", "--intent=microphone"],
            vec!["--intent=microphone", "--shared-microphone", "--exercise"],
            vec!["--shared-microphone", "--help"],
        ] {
            let options = parse(&arguments).unwrap();
            assert_eq!(
                serde_json::to_value(options.mode).unwrap(),
                "shared-capture-pcm16"
            );
        }
        for arguments in [
            vec!["--shared-microphone"],
            vec!["--exercise", "--shared-microphone"],
            vec!["--exercise", "--shared-microphone", "--intent=playback"],
            vec![
                "--exercise",
                "--shared-microphone",
                "--intent=microphone",
                "--timer",
            ],
            vec![
                "--exercise",
                "--timer",
                "--shared-microphone",
                "--intent=microphone",
            ],
            vec![
                "--exercise",
                "--shared-microphone",
                "--intent=microphone",
                "--minimum-period",
            ],
            vec![
                "--exercise",
                "--shared-microphone",
                "--shared-microphone",
                "--intent=microphone",
            ],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
        assert_eq!(
            serde_json::to_value(parse(&["--exercise"]).unwrap().mode).unwrap(),
            "exclusive-event"
        );
    }
}
