//! Opt-in control-thread probe. Never installs a driver or changes audio defaults.

use std::{error::Error, ffi::OsString};

#[cfg(any(windows, test))]
#[path = "vad_lifecycle/simultaneous.rs"]
mod simultaneous;

#[derive(Debug, PartialEq, Eq)]
struct Options {
    exercise: bool,
    simultaneous: bool,
    rounds: u32,
    help: bool,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self {
            exercise: false,
            simultaneous: false,
            rounds: 1,
            help: false,
        };
        let mut rounds_seen = false;
        for argument in arguments {
            let argument = argument
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_owned())?;
            match argument.as_str() {
                "--exercise" if !options.exercise => options.exercise = true,
                "--simultaneous" if !options.simultaneous => options.simultaneous = true,
                "--help" | "-h" => options.help = true,
                value if value.starts_with("--rounds=") && !rounds_seen => {
                    options.rounds = value["--rounds=".len()..]
                        .parse()
                        .map_err(|_| "--rounds requires an integer in 1..=10000".to_owned())?;
                    if !(1..=10_000).contains(&options.rounds) {
                        return Err("--rounds requires an integer in 1..=10000".to_owned());
                    }
                    rounds_seen = true;
                }
                _ => return Err(format!("unknown or repeated argument: {argument}")),
            }
        }
        if rounds_seen && !options.exercise && !options.help {
            return Err("--rounds requires --exercise".to_owned());
        }
        if options.simultaneous && !options.exercise && !options.help {
            return Err("--simultaneous requires --exercise".to_owned());
        }
        Ok(options)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    if options.help {
        println!(
            "vad_lifecycle [--exercise [--rounds=100] [--simultaneous]]\n\
             Default: enumerate present control interfaces and query capabilities.\n\
             Opening even for a capability query creates an exclusive driver session.\n\
             --exercise: stable-ID creation/preparation/query/gate-close/removal.\n\
             --simultaneous: hold four combinations, then remove/recreate each\n\
             while checking that its three sibling resources stay present.\n\
             No audio streams, driver installation, or default-device changes."
        );
        return Ok(());
    }
    run(options)
}

#[cfg(not(windows))]
fn run(_options: Options) -> Result<(), Box<dyn Error>> {
    Err("the VAD control probe requires Windows".into())
}

#[cfg(windows)]
fn run(options: Options) -> Result<(), Box<dyn Error>> {
    use moiren_windows_audio::vad::{
        Channels, ControlClient, Intent, PdoState, StableEndpointId, discover_present_interfaces,
    };
    use std::{collections::HashSet, time::Duration};

    let interfaces = discover_present_interfaces()?;
    println!(
        "Present Moiren VAD control interfaces: {}",
        interfaces.len()
    );
    if options.exercise && interfaces.is_empty() {
        return Err("--exercise requires a separately installed, present VAD driver".into());
    }
    for interface in interfaces {
        println!("Control interface: {}", interface.display_path());
        // Keep this one exclusive file object for the entire session. Reopening
        // between requests would trigger cleanup and invalidate all PDO tokens.
        let mut client = ControlClient::connect(&interface)?;
        let capabilities = client.capabilities();
        println!(
            "Capabilities: flags={:#x}, max_raw_pdos={}, streams_per_pin={}, \
             target_hz={}, formats={:#x}, channels={:#x}, intents={:#x}, \
             session_epoch={}, request_slots={}, max_packet_frames={}",
            capabilities.flags(),
            capabilities.max_endpoints(),
            capabilities.max_streams_per_pin(),
            capabilities.target_sample_rate(),
            capabilities.sample_formats(),
            capabilities.channel_mask(),
            capabilities.intent_mask(),
            capabilities.session_epoch(),
            capabilities.request_slots(),
            capabilities.max_packet_frames()
        );
        println!(
            "PCM admission capability available: {}. A raw PDO is not a WASAPI endpoint.",
            capabilities.require_pcm().is_ok()
        );
        if !options.exercise {
            continue;
        }
        if options.simultaneous {
            capabilities.require_audio()?;
            simultaneous::exercise(&mut client, options.rounds)?;
            continue;
        }
        let mut observed_tokens = HashSet::new();
        for round in 1..=options.rounds {
            for intent in [Intent::Playback, Intent::Microphone] {
                for channels in [Channels::Mono, Channels::Stereo] {
                    // Stable across rounds; independent of runtime token reuse
                    // checks. Default eligibility is a separate local preference.
                    let stable_id = StableEndpointId::new(
                        intent as u64 | ((channels as u64) << 32),
                        0x4d6f_6972_656e_5052,
                    );
                    let created = client.create(
                        stable_id,
                        intent,
                        channels,
                        "Moiren VAD lifecycle probe",
                        false,
                    )?;
                    let token = created.pdo_token();
                    if !observed_tokens.insert(token) {
                        return Err(format!("driver reused runtime PDO token {token}").into());
                    }
                    let present = client.wait_present(created, Duration::from_secs(15))?;
                    let queried = client.query(present.key())?;
                    queried.require_created(stable_id, intent, channels)?;
                    let closed = client.close_gate(queried)?;
                    closed.require_created(stable_id, intent, channels)?;
                    let removed = client.remove_and_wait(closed.key(), Duration::from_secs(15))?;
                    if removed.state() != PdoState::Removed {
                        return Err("removal was accepted but did not complete".into());
                    }
                    println!(
                        "round={round}/{} intent={intent:?} channels={channels:?} \
                         stable_id={stable_id} runtime_pdo={token} \
                         endpoint_generation={} gate_epoch={} -> Removed",
                        options.rounds,
                        removed.endpoint_generation(),
                        removed.gate_epoch()
                    );
                }
            }
        }
        println!(
            "Completed {} sequential endpoint lifecycles; no native stream or PCM IO was opened.",
            u64::from(options.rounds) * 4
        );
        // Dropping the client closes the owning file object. The driver revokes
        // the session and requests cleanup of any children left after an error.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Options;

    fn parse(values: &[&str]) -> Result<Options, String> {
        Options::parse(values.iter().map(Into::into))
    }

    #[test]
    fn default_only_queries_capabilities() {
        assert_eq!(
            parse(&[]).unwrap(),
            Options {
                exercise: false,
                simultaneous: false,
                rounds: 1,
                help: false
            }
        );
    }

    #[test]
    fn exercise_and_rounds_are_explicit_and_bounded() {
        assert_eq!(parse(&["--exercise", "--rounds=100"]).unwrap().rounds, 100);
        assert_eq!(parse(&["--rounds=100", "--exercise"]).unwrap().rounds, 100);
        for arguments in [
            vec!["--rounds=100"],
            vec!["--exercise", "--rounds=0"],
            vec!["--exercise", "--rounds=10001"],
            vec!["--exercise", "--rounds=-1"],
            vec!["--exercise", "--rounds=abc"],
            vec!["--exercise", "--rounds=4294967296"],
            vec!["--exercise", "--exercise"],
            vec!["--exercise", "--rounds=1", "--rounds=2"],
            vec!["--install"],
            vec!["--set-default"],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
    }

    #[test]
    fn help_does_not_require_a_session() {
        assert!(parse(&["--help"]).unwrap().help);
        assert!(parse(&["-h"]).unwrap().help);
    }

    #[test]
    fn simultaneous_mode_requires_explicit_exercise_and_is_not_repeated() {
        assert!(
            parse(&["--exercise", "--simultaneous"])
                .unwrap()
                .simultaneous
        );
        assert!(parse(&["--simultaneous", "--exercise", "--rounds=2"]).is_ok());
        assert!(parse(&["--simultaneous"]).is_err());
        assert!(parse(&["--exercise", "--simultaneous", "--simultaneous"]).is_err());
    }
}
