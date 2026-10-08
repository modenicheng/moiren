#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let mut iterations = 3;
    let mut observe_pid = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--iterations" => {
                iterations = args
                    .next()
                    .context("--iterations requires 1..10")?
                    .parse::<u32>()
                    .context("--iterations expects a number")?
            }
            "--observe-pid" => {
                observe_pid = Some(
                    args.next()
                        .context("--observe-pid requires PID")?
                        .parse::<u32>()
                        .context("--observe-pid expects a numeric PID")?,
                )
            }
            "--output" => output = Some(args.next().context("--output requires a path")?),
            "--help" | "-h" => {
                println!(
                    "w00_swdevice [--iterations 1..10] [--observe-pid PID] [--output PATH]\nCreates only temporary MoirenW00 PnP nodes and uninstalls those exact instances. No driver packages or audio endpoints installed. Administrator rights required for successful device creation."
                );
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    let report = moiren_windows_audio::swdevice::run(iterations, observe_pid)?;
    if let Some(path) = output {
        serde_json::to_writer_pretty(
            std::fs::File::create(&path)
                .with_context(|| format!("creating report file {path:?}"))?,
            &report,
        )
        .with_context(|| format!("writing report file {path:?}"))?;
    } else {
        serde_json::to_writer_pretty(std::io::stdout().lock(), &report)
            .context("writing the JSON report to stdout")?;
    }
    if report
        .cycles
        .iter()
        .any(|cycle| cycle.status != "completed")
        || !report.observed_state_changes.is_empty()
        || report.audio_ids_before != report.audio_ids_after
    {
        std::process::exit(1);
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Software-device probe requires Windows.");
    std::process::exit(1);
}
