//! Opt-in real service/VAD integration. No native client or driver opens by default.
#[path = "vad_integration/options.rs"]
mod options;
#[cfg(any(windows, test))]
#[path = "vad_integration/progress.rs"]
mod progress;
#[cfg(any(windows, test))]
#[path = "vad_integration/project.rs"]
mod project;
#[cfg(any(windows, test))]
#[path = "vad_integration/report.rs"]
mod report;
#[cfg(any(windows, test))]
#[path = "vad_integration/soak.rs"]
mod soak;
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

const HELP: &str = "vad_integration --exercise [--pcm] [--timeout-seconds=N] [--output-dir=NEW_DIRECTORY]\n\
    [--soak-idle-seconds=1..28800] [--soak-pcm-seconds=1..28800]\n\
    Stage1: persisted v2 stereo VirtualPlayback -> Gain(0.5) -> VirtualMicrophone;\n\
    production service startup/discovery, Apply with no probe native clients,\n\
    Stop with closed gates and retained endpoints, then service exit/join.\n\
    Stage1 does NOT validate PCM or final PDO disappearance.\n\
    Stage2 --pcm: hold 4s with no probe native clients, check actual Engine signal\n\
    frame_end progress beyond 3s; exact-root/stable-ID WASAPI exclusive Float32\n\
    clients verify stereo [.25,-.5] -> Gain .5 -> [.125,-.25], then a distinct\n\
    second signal; keep capture RUN during Stop and verify real captured silence.\n\
    Running gate closures require the same native identity and an observed newer\n\
    stream generation; re-admission is bounded to 5s and the existing phase limit.\n\
    Both gates must be open at successful hold/signal-phase completion.\n\
    --pcm defaults to a 90s overall watchdog and requires at least 60s if set.\n\
    Soak: one persisted graph, one App owner. Idle observes actual frame_end\n\
    throughout the requested interval, with two advances per one-second window.\n\
    PCM soak implies --pcm and keeps the same real native pair RUN; alternates\n\
    source every 30s with exactly 1s finite-only settling, otherwise validates\n\
    every frame against Gain(.5), rejects silence/gaps and samples native clocks.\n\
    Both soak phases require their complete requested duration and fresh state.\n\
    Soak timeout defaults to idle+pcm+180s; explicit budget is aggregate +90..3600s,\n\
    with a fixed 30s cleanup reserve. Eight hours each: --timeout-seconds=57780.\n\
    progress.json is atomically replaced/flushed every 5s and is NOT qualification;\n\
    final report.json includes soak_idle/soak_pcm requested_ms/elapsed_ms/completed.\n\
    Default/no arguments and --help do not access the driver or write files.\n\
    --exercise exclusively owns the optional VAD control session through App.\n\
    Close other driver probes/apps first; requires a separately installed driver.\n\
    Without soak, timeout is an overall 10..=120s limit including cleanup; a deadline\n\
    ends the probe process (exit 2), so native cleanup is not reported as passed.\n\
    JSON on stdout and in a fresh output directory; no user preferences change.\n\
    --exercise logs to stderr only (default info); RUST_LOG may override it,\n\
    e.g. info,moiren_windows_audio::vad=debug. No logger files are created.\n\
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
