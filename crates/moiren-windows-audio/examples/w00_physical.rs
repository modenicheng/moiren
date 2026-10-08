#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let mut endpoints = Vec::new();
    let mut seconds = 60;
    let mut observe_pid = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => endpoints.push(
                args.next()
                    .context("--endpoint requires an opaque endpoint ID")?,
            ),
            "--seconds" => {
                seconds = args
                    .next()
                    .context("--seconds requires a duration")?
                    .parse::<u32>()
                    .context("--seconds expects a number of seconds")?
            }
            "--observe-pid" => {
                observe_pid = Some(
                    args.next()
                        .context("--observe-pid requires a process ID")?
                        .parse::<u32>()
                        .context("--observe-pid expects a numeric PID")?,
                )
            }
            "--help" | "-h" => {
                println!(
                    "w00_physical --endpoint <ID> [--endpoint <ID> ...] [--seconds 1..600] [--observe-pid <PID>]\nNative f32 physical capture and silent Shared render; statistics only."
                );
                return Ok(());
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    let report = moiren_windows_audio::physical::run(endpoints, seconds, observe_pid)?;
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)
        .context("writing the JSON report to stdout")?;
    if report
        .endpoints
        .iter()
        .any(|endpoint| endpoint.status != "completed" || !endpoint.stop_succeeded)
        || !report.errors.is_empty()
    {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Physical endpoint probes require Windows.");
    std::process::exit(1);
}
