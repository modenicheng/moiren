#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use moiren_windows_audio::exclusive_probe::{Observation, run};
    let (mut pid, mut endpoint, mut seconds) = (None, None, 3);
    let mut seen = std::collections::BTreeSet::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!(
                "exclusive_probe --pid <PID> --endpoint <ID> [--seconds 1..60]\n\
                Continuous process capture: baseline -> exclusive silent render -> released.\n\
                Defaults: 3 seconds per phase plus 500 ms settling per phase.\n\
                Temporarily interrupts ALL shared playback on that endpoint.\n\
                JSON statistics only; no PCM saved. Use w00_probe --list to enumerate.\n\
                Exit 0: sustained capture observed; 2: inconclusive/no sustained signal; 1: error."
            );
            return Ok(());
        }
        anyhow::ensure!(
            ["--pid", "--endpoint", "--seconds"].contains(&arg.as_str()),
            "unknown argument: {arg}"
        );
        anyhow::ensure!(seen.insert(arg.clone()), "duplicate argument: {arg}");
        let value = args
            .next()
            .with_context(|| format!("{arg} requires a value"))?;
        match arg.as_str() {
            "--pid" => pid = Some(value.parse::<u32>().context("invalid PID")?),
            "--endpoint" => endpoint = Some(value),
            "--seconds" => seconds = value.parse::<u32>().context("invalid seconds")?,
            _ => unreachable!(),
        }
    }
    let report = run(
        pid.context("--pid is required")?,
        endpoint.context("--endpoint is required")?,
        seconds,
    )?;
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
    std::process::exit(match report.observation {
        Observation::SignalObservedDuringExclusive => 0,
        Observation::ExperimentFailed => 1,
        _ => 2,
    });
}

#[cfg(not(windows))]
fn main() {
    eprintln!("exclusive_probe requires Windows and explicit hardware opt-in.");
    std::process::exit(1);
}
