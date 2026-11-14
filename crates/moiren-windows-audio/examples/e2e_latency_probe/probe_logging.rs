use anyhow::{Context, Result};
use flexi_logger::{Logger, LoggerHandle};

pub(super) fn initialize() -> Result<Option<LoggerHandle>> {
    let filter = match std::env::var("RUST_LOG") {
        Ok(filter) => filter,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(error) => return Err(error).context("RUST_LOG must contain valid Unicode"),
    };
    // Parse strictly: accepting a partial specification would silently lose
    // requested evidence. No file writer or preference directory is created.
    let handle = Logger::try_with_str(filter)
        .context("invalid RUST_LOG filter")?
        .log_to_stderr()
        .start()
        .context("initialize probe stderr logger")?;
    // This control-side startup record proves filtering/routing, not an audio
    // clock gap or any other realtime event.
    log::debug!(
        target: "moiren::audio::vad",
        "event=probe_logger_ready destination=stderr"
    );
    Ok(Some(handle))
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Output};

    const READY: &str = "event=probe_logger_ready destination=stderr";
    const HELP: &str = "Offline evidence analysis";
    const CHILD: &str = "MOIREN_PROBE_LOGGER_TEST_CHILD";

    #[test]
    fn help_child() {
        if std::env::var_os(CHILD).is_some() {
            std::process::exit(crate::run_cli(vec!["--help".into()]));
        }
    }

    fn help_with_filter(filter: Option<&str>) -> Output {
        // Each child owns its global logger and environment; parallel tests
        // never change the parent process environment.
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "probe_logging::tests::help_child", "--nocapture"])
            .env(CHILD, "1")
            .env_remove("RUST_LOG");
        if let Some(filter) = filter {
            command.env("RUST_LOG", filter);
        }
        command.output().unwrap()
    }

    fn assert_quiet_help(filter: Option<&str>) {
        let output = help_with_filter(filter);
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(HELP));
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains(READY));
    }

    #[test]
    fn requested_target_debug_is_stderr_only() {
        let output = help_with_filter(Some("info,moiren::audio::vad=debug"));
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(HELP));
        assert!(!String::from_utf8_lossy(&output.stdout).contains(READY));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(READY),
            "{output:?}"
        );
    }

    #[test]
    fn missing_rust_log_keeps_help_quiet() {
        assert_quiet_help(None);
    }

    #[test]
    fn restrictive_filter_keeps_target_debug_quiet() {
        assert_quiet_help(Some("info,moiren::audio::vad=info"));
    }

    #[test]
    fn unrelated_target_debug_does_not_enable_probe_target() {
        assert_quiet_help(Some("info,unrelated::target=debug"));
    }

    #[test]
    fn invalid_explicit_filter_fails_before_cli_output() {
        let output = help_with_filter(Some("info,moiren::audio::vad=not-a-level"));
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("RUST_LOG"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains(HELP));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(READY));
    }
}
