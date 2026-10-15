use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use super::{AppPreferences, ProjectDocument};

const MAX_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("could not {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid {field}: {reason}")]
    Invalid { field: String, reason: String },
    #[error("unsupported {document} document version {version}")]
    UnsupportedVersion {
        document: &'static str,
        version: u32,
    },
    #[error("document {path} exceeds the 8 MiB limit")]
    TooLarge { path: PathBuf },
    #[error("user configuration directory is unavailable")]
    ConfigurationDirectoryUnavailable,
}

impl PersistenceError {
    pub(super) fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Invalid {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> PersistenceError {
    PersistenceError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

pub fn load_project(path: impl AsRef<Path>) -> Result<ProjectDocument, PersistenceError> {
    let document: ProjectDocument = read_json(path.as_ref())?;
    document.validate()?;
    Ok(document)
}

pub fn save_project(
    path: impl AsRef<Path>,
    document: &ProjectDocument,
) -> Result<(), PersistenceError> {
    document.validate()?;
    write_json(path.as_ref(), document)
}

/// Resolves a user-owned configuration directory without writing into the repo.
pub fn preferences_path() -> Result<PathBuf, PersistenceError> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join(".config"))
        });
    base.map(|base| base.join("Moiren").join("preferences.json"))
        .ok_or(PersistenceError::ConfigurationDirectoryUnavailable)
}

pub fn load_preferences() -> Result<AppPreferences, PersistenceError> {
    load_preferences_at(preferences_path()?)
}

pub fn save_preferences(preferences: &AppPreferences) -> Result<(), PersistenceError> {
    save_preferences_at(preferences_path()?, preferences)
}

/// A missing preference file is a first launch; corrupted files remain errors.
pub fn load_preferences_at(path: impl AsRef<Path>) -> Result<AppPreferences, PersistenceError> {
    match read_json::<AppPreferences>(path.as_ref()) {
        Ok(preferences) => {
            preferences.validate()?;
            Ok(preferences)
        }
        Err(PersistenceError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(AppPreferences::default())
        }
        Err(error) => Err(error),
    }
}

pub fn save_preferences_at(
    path: impl AsRef<Path>,
    preferences: &AppPreferences,
) -> Result<(), PersistenceError> {
    preferences.validate()?;
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| io_error("create configuration directory", parent, error))?;
    write_json(path, preferences)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, PersistenceError> {
    let file = File::open(path).map_err(|error| io_error("open", path, error))?;
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read", path, error))?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(PersistenceError::TooLarge {
            path: path.to_owned(),
        });
    }
    serde_json::from_slice(&bytes).map_err(|source| PersistenceError::Json {
        path: path.to_owned(),
        source,
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), PersistenceError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| PersistenceError::Json {
        path: path.to_owned(),
        source,
    })?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(PersistenceError::TooLarge {
            path: path.to_owned(),
        });
    }
    let filename = path
        .file_name()
        .ok_or_else(|| PersistenceError::invalid("path", "must name a file"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary_name = filename.to_os_string();
    temporary_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = parent.join(temporary_name);
    // create_new never overwrites another process's temporary file. The final
    // rename stays on one filesystem, replacing the destination atomically on
    // Windows and Unix; failure leaves the previously saved project intact.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| io_error("create temporary file", &temporary, error))?;
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|error| io_error("write temporary file", &temporary, error))?;
        file.sync_all()
            .map_err(|error| io_error("sync temporary file", &temporary, error))?;
        drop(file);
        fs::rename(&temporary, path).map_err(|error| io_error("replace saved file", path, error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
