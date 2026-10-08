#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
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
                        .ok_or("--pid requires a process ID")?
                        .parse::<u32>()?,
                )
            }
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or("--seconds requires a duration")?
                    .parse::<u32>()?
            }
            "--help" | "-h" => {
                println!(
                    "w00_probe --list | --pid <PID> [--seconds 1..600]\nStatistics only. JSON on stdout after capture stops; no PCM file or playback."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if list == pid.is_some() {
        return Err("choose exactly one of --list or --pid <PID>".into());
    }
    let report = moiren_windows_audio::probe::run(pid, seconds)?;
    // Write only after streaming has stopped and owner resources have been released.
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
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
