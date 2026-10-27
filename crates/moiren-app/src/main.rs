use anyhow::{Context, Ok, bail};
use moiren_app::{AppConfig, OfflineApp};

fn main() -> anyhow::Result<()> {
    if let Err(error) = moiren_app::logging::initialize() {
        eprintln!("Moiren file logging unavailable: {error}");
    } else if let Some(path) = moiren_app::logging::log_path() {
        log::info!(target: "moiren::app", "file logging initialized: {}", path.display());
    }
    let result = run();
    if let Err(error) = &result {
        log::error!(target: "moiren::app", "application exited with an error: {error:#}");
    }
    result
}

fn run() -> anyhow::Result<()> {
    let mut config = AppConfig::default();
    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Moiren\nUsage: moiren-app [--gain 0..16]\nStarts the GUI; --gain runs the offline software IO demo.\nUse moiren-app render --help for a Windows output, or monitor --help for physical/process input monitoring."
                );
                return Ok(());
            }
            "--gain" => {
                config.initial_gain = args
                    .next()
                    .context("--gain requires a value")?
                    .parse()
                    .context("invalid --gain value")?;
            }
            _ => bail!("unknown argument: {arg}; use --help"),
        }
    }
    if std::env::args().any(|arg| arg == "--gain") {
        let mut app = OfflineApp::new(config).context("preparing the offline application")?;
        let input = [0.25, -0.5, 0.5, -0.25, 1.0, -1.0, 0.0, 0.0];
        let mut output = [0.0; 8];
        let report = app.process_interleaved(&input, &mut output)?;
        println!("processed: {report:?}\noutput: {output:?}");
        app.stop();
        return Ok(());
    }
    moiren_app::ui::run()
}
