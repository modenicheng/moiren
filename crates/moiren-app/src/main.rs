fn main() -> anyhow::Result<()> {
    if let Err(error) = moiren_app::logging::initialize() {
        eprintln!("Moiren file logging unavailable: {error}");
    } else if let Some(path) = moiren_app::logging::log_path() {
        log::info!(target: "moiren::app", "file logging initialized: {}", path.display());
    }
    let result = moiren_app::cli::run(std::env::args().skip(1));
    if let Err(error) = &result {
        log::error!(target: "moiren::app", "application exited with an error: {error:#}");
    }
    result
}
