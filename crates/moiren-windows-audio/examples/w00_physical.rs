#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut endpoints = Vec::new();
    let mut seconds = 60;
    let mut observe_pid = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => endpoints.push(
                args.next()
                    .ok_or("--endpoint requires an opaque endpoint ID")?,
            ),
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or("--seconds requires a duration")?
                    .parse::<u32>()?
            }
            "--observe-pid" => {
                observe_pid = Some(
                    args.next()
                        .ok_or("--observe-pid requires a process ID")?
                        .parse::<u32>()?,
                )
            }
            "--help" | "-h" => {
                println!(
                    "w00_physical --endpoint <ID> [--endpoint <ID> ...] [--seconds 1..600] [--observe-pid <PID>]\nNative f32 physical capture and silent Shared render; statistics only."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let report = moiren_windows_audio::physical::run(endpoints, seconds, observe_pid)?;
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
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
