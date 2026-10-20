//! Opt-in real service/VAD integration. No native client or driver opens by default.
#[path = "vad_integration/options.rs"]
mod options;
#[cfg(any(windows, test))]
#[path = "vad_integration/project.rs"]
mod project;
#[cfg(any(windows, test))]
#[path = "vad_integration/report.rs"]
mod report;
#[cfg(any(windows, test))]
#[path = "vad_integration/validation.rs"]
mod validation;
// Reuse only read-only topology identity, never the native probe's control owner.
#[cfg(windows)]
#[path = "../../moiren-windows-audio/examples/vad_native/identity.rs"]
mod identity;
#[cfg(windows)]
#[path = "vad_integration/native.rs"]
mod native;
#[cfg(windows)]
#[path = "vad_integration/target.rs"]
mod target;

const HELP: &str = "vad_integration --exercise [--pcm] [--timeout-seconds=30|90] [--output-dir=NEW_DIRECTORY]\n\
    Stage1: persisted v2 stereo VirtualPlayback -> Gain(0.5) -> VirtualMicrophone;\n\
    production service startup/discovery, Apply with no probe native clients,\n\
    Stop with closed gates and retained endpoints, then service exit/join.\n\
    Stage1 does NOT validate PCM or final PDO disappearance.\n\
    Stage2 --pcm: hold 4s with no probe native clients, check actual Engine signal\n\
    frame_end progress beyond 3s; exact-root/stable-ID WASAPI exclusive Float32\n\
    clients verify stereo [.25,-.5] -> Gain .5 -> [.125,-.25], then a distinct\n\
    second signal; keep capture RUN during Stop and verify real captured silence.\n\
    --pcm defaults to a 90s overall watchdog and requires at least 60s if set.\n\
    Default/no arguments and --help do not access the driver or write files.\n\
    --exercise exclusively owns the optional VAD control session through App.\n\
    Close other driver probes/apps first; requires a separately installed driver.\n\
    Timeout is an overall 10..=120 second limit, including cleanup; a deadline\n\
    ends the probe process (exit 2), so native cleanup is not reported as passed.\n\
    JSON on stdout and in a fresh output directory; no user preferences change.\n\
    --exercise logs to stderr only (default info); RUST_LOG may override it,\n\
    e.g. info,moiren::audio::vad=debug. No logger files are created.\n\
    No installation, defaults, power settings, or restart operations.\n\
    Exit codes: 0 requested stages passed/help, 1 failed, 2 overall timeout.";

fn main() -> anyhow::Result<()> {
    let options = options::Options::parse(std::env::args().skip(1))?;
    if options.help || !options.exercise {
        println!("{HELP}");
        return Ok(());
    }
    #[cfg(windows)]
    {
        // Keep the logger alive through target work, cleanup, and the watchdog.
        // This is deliberately after the default/help guard and uses no App prefs.
        let _logger = flexi_logger::Logger::try_with_env_or_str("info")?
            .log_to_stderr()
            .start()?;
        target::exercise(options)
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("--exercise requires Windows and a separately installed VAD driver")
    }
}
