#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let mut pid = None;
    let mut seconds = 60;
    let mut list = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--pid" => {
                pid = Some(
                    args.next()
                        .context("--pid requires a process ID")?
                        .parse::<u32>()
                        .context("--pid expects a numeric PID")?,
                )
            }
            "--seconds" => {
                seconds = args
                    .next()
                    .context("--seconds requires a duration")?
                    .parse::<u32>()
                    .context("--seconds expects a number of seconds")?
            }
            "--help" | "-h" => {
                println!(
                    "w00_probe --list | --pid <PID> [--seconds 1..600]\nStatistics only. JSON on stdout after capture stops; no PCM file or playback."
                );
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    if list == pid.is_some() {
        anyhow::bail!("choose exactly one of --list or --pid <PID>");
    }
    let report = moiren_windows_audio::probe::run(pid, seconds)?;
    // Write only after streaming has stopped and owner resources have been released.
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)
        .context("writing the JSON report to stdout")?;
    if report
        .capture
        .as_ref()
        .is_some_and(|capture| capture.status == "api_failed" || !capture.stop_succeeded)
        || !report.errors.is_empty()
    {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("W00 device probes require Windows; pure statistics tests remain portable.");
    std::process::exit(1);
}
