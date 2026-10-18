//! Small application-owned wrapper around `flexi_logger`.
//!
//! Keeping the setup here gives the rest of the application ordinary `log!`
//! macros while the logger crate owns terminal detection, ANSI handling,
//! formatting, file writes, and rotation.

use std::{
    io,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use flexi_logger::{
    AdaptiveFormat, Cleanup, Criterion, Duplicate, FileSpec, Logger, LoggerHandle, Naming,
};

pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
pub const ROTATED_LOG_COUNT: usize = 5;

static INIT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static LOGGER_HANDLE: OnceLock<LoggerHandle> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Returns the active diagnostics file path after logging has been initialized.
pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().map(PathBuf::as_path)
}

/// Initializes file and stdout logging once, using the per-user preferences directory.
pub fn initialize() -> io::Result<&'static Path> {
    let lock = INIT_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(path) = LOG_PATH.get() {
        return Ok(path);
    }

    let preferences_path = match crate::project::preferences_path() {
        Ok(path) => path,
        Err(error) => {
            install_stdout_fallback()?;
            return Err(io::Error::other(error));
        }
    };
    let path = log_path_for(&preferences_path);
    let logger = logger_builder()
        .log_to_file(file_spec_for(&path))
        .append()
        .rotate(
            Criterion::Size(MAX_LOG_BYTES),
            Naming::TimestampsCustomFormat {
                // Keep the active filename stable for the UI and support tools.
                current_infix: Some(""),
                format: "%Y-%m-%d_%H-%M-%S",
            },
            Cleanup::KeepLogFiles(ROTATED_LOG_COUNT),
        )
        // Mirror every level accepted by the logger on stdout; the active
        // level still comes from `RUST_LOG` or the default `info` filter.
        .duplicate_to_stdout(Duplicate::Trace)
        .start()
        .map_err(io::Error::other);

    let logger = match logger {
        Ok(logger) => logger,
        Err(file_error) => {
            // A read-only or unavailable profile directory should not remove
            // interactive diagnostics from the process entirely.
            install_stdout_fallback()?;
            return Err(file_error);
        }
    };

    LOGGER_HANDLE.set(logger).map_err(|_| {
        io::Error::new(io::ErrorKind::AlreadyExists, "logger was initialized twice")
    })?;
    let _ = LOG_PATH.set(path);
    Ok(LOG_PATH
        .get()
        .expect("log path is set after logger initialization"))
}

fn logger_builder() -> Logger {
    Logger::try_with_env_or_str("info")
        .unwrap_or_else(|_| Logger::with(log::LevelFilter::Info))
        // Let flexi_logger choose its colored detailed formatter only when
        // stdout is a terminal; files always use the plain formatter.
        .adaptive_format_for_stdout(AdaptiveFormat::Detailed)
        .format_for_files(flexi_logger::detailed_format)
        .panic_if_error_channel_is_broken(false)
}

fn install_stdout_fallback() -> io::Result<()> {
    let logger = logger_builder()
        .log_to_stdout()
        .start()
        .map_err(io::Error::other)?;
    LOGGER_HANDLE
        .set(logger)
        .map_err(|_| io::Error::new(io::ErrorKind::AlreadyExists, "logger was initialized twice"))
}

fn file_spec_for(path: &Path) -> FileSpec {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let basename = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "moiren".into());
    let suffix = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());

    FileSpec::default()
        .directory(directory.to_owned())
        .basename(basename)
        .o_suffix(suffix)
        .suppress_timestamp()
}

fn log_path_for(preferences_path: &Path) -> PathBuf {
    preferences_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("logs")
        .join("moiren.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_uses_the_preferences_directory_and_logs_subdirectory() {
        assert_eq!(
            log_path_for(Path::new(
                "C:/Users/example/AppData/Roaming/Moiren/preferences.json"
            )),
            PathBuf::from("C:/Users/example/AppData/Roaming/Moiren/logs/moiren.log")
        );
    }

    #[test]
    fn file_spec_keeps_the_active_filename_without_terminal_formatting() {
        let path = Path::new("C:/Users/example/AppData/Roaming/Moiren/logs/moiren.log");
        assert_eq!(file_spec_for(path).as_pathbuf(Some("")), path);
    }

    #[test]
    fn rotated_file_names_keep_the_timestamp_infix() {
        let path = Path::new("C:/Users/example/AppData/Roaming/Moiren/logs/moiren.log");
        let spec = file_spec_for(path);
        let rotated = spec.as_pathbuf(Some("2026-10-18_13-30-00"));

        assert_eq!(rotated.parent(), path.parent());
        assert_eq!(
            rotated.file_name().map(|name| name.to_string_lossy()),
            Some("moiren_2026-10-18_13-30-00.log".into())
        );
    }
}
