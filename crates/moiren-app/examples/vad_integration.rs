//! Opt-in service-only VAD integration. No native client or driver opens by default.
#[path = "vad_integration/options.rs"]
mod options;
#[cfg(any(windows, test))]
#[path = "vad_integration/project.rs"]
mod project;
#[cfg(any(windows, test))]
#[path = "vad_integration/report.rs"]
mod report;
#[cfg(windows)]
#[path = "vad_integration/target.rs"]
mod target;

const HELP: &str = "vad_integration --exercise [--timeout-seconds=30] [--output-dir=NEW_DIRECTORY]\n\
    Stage1: persisted v2 stereo VirtualPlayback -> Gain(0.5) -> VirtualMicrophone;\n\
    production service startup/discovery, Apply with no probe native clients,\n\
    Stop with closed gates and retained endpoints, then service exit/join.\n\
    Stage1 does NOT validate PCM or final PDO disappearance.\n\
    Default/no arguments and --help do not access the driver or write files.\n\
    --exercise exclusively owns the optional VAD control session through App.\n\
    Close other driver probes/apps first; requires a separately installed driver.\n\
    Timeout is an overall 10..=120 second limit, including cleanup; a deadline\n\
    ends the probe process (exit 2), so native cleanup is not reported as passed.\n\
    JSON on stdout and in a fresh output directory; no user preferences change.\n\
    No installation, defaults, power settings, or restart operations.\n\
    Exit codes: 0 stage1 passed/help, 1 failed, 2 overall timeout.";

fn main() -> anyhow::Result<()> {
    let options = options::Options::parse(std::env::args().skip(1))?;
    if options.help || !options.exercise {
        println!("{HELP}");
        return Ok(());
    }
    #[cfg(windows)]
    {
        target::exercise(options)
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("--exercise requires Windows and a separately installed VAD driver")
    }
}
