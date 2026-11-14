//! The project status line's typed payload and its paint-time translation
//! (design 21 §4): state stores what happened, paint picks the sentence.
use rust_i18n::t;

/// The project status line's typed payload (design 21 §4): [`ProjectIo`]
/// stores what happened, and the paint sites translate it through
/// [`message_text`]. No prose is frozen into state, so a locale switch
/// relabels the stored event on the next repaint.
///
/// [`ProjectIo`]: super::ProjectIo
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProjectMessage {
    Saved,
    Opened,
    LoadRequested,
    /// The service declined the load; `cause` is the service's stable
    /// cause string (`snapshot.graph.last_error`).
    NotLoaded {
        cause: String,
    },
    /// A persistence-layer failure, stored as [`PersistedFailure`] (the
    /// cloneable mirror of the closed `PersistenceError`); the sentence is
    /// chosen at paint by matching the variant.
    Persisted(PersistedFailure),
    /// The native save dialog failed to open; `detail` is the dialog
    /// layer's Display prose (infrastructure diagnostics).
    SaveDialogFailed {
        detail: String,
    },
    /// The native open dialog failed to open; `detail` as above.
    LoadDialogFailed {
        detail: String,
    },
    /// The background autosave write failed; `cause` is the persistence prose.
    AutoSaveFailed {
        cause: String,
    },
    PreferencesSaved,
    PreferencesSaveQueued,
    /// The preferences write failed; `cause` is the persistence prose.
    PreferencesSaveFailed {
        cause: String,
    },
}

/// The UI-side, cloneable mirror of the closed
/// [`crate::project::PersistenceError`]: one variant each, with the
/// `io::Error`/`serde_json::Error` sources degraded to their Display prose
/// (diagnostic data; the sentence around it is translated at paint). The
/// mirror exists because the status message rides `Clone`/`PartialEq`
/// plumbing (settings key, tray snapshot) the raw errors cannot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PersistedFailure {
    Io {
        operation: &'static str,
        path: std::path::PathBuf,
        source: String,
    },
    Json {
        path: std::path::PathBuf,
        source: String,
    },
    Invalid {
        field: String,
        reason: String,
    },
    UnsupportedVersion {
        document: &'static str,
        version: u32,
    },
    TooLarge {
        path: std::path::PathBuf,
    },
    ConfigurationDirectoryUnavailable,
}

impl From<crate::project::PersistenceError> for PersistedFailure {
    fn from(error: crate::project::PersistenceError) -> Self {
        match error {
            crate::project::PersistenceError::Io {
                operation,
                path,
                source,
            } => Self::Io {
                operation,
                path,
                source: source.to_string(),
            },
            crate::project::PersistenceError::Json { path, source } => Self::Json {
                path,
                source: source.to_string(),
            },
            crate::project::PersistenceError::Invalid { field, reason } => {
                Self::Invalid { field, reason }
            }
            crate::project::PersistenceError::UnsupportedVersion { document, version } => {
                Self::UnsupportedVersion { document, version }
            }
            crate::project::PersistenceError::TooLarge { path } => Self::TooLarge { path },
            crate::project::PersistenceError::ConfigurationDirectoryUnavailable => {
                Self::ConfigurationDirectoryUnavailable
            }
        }
    }
}

/// Paint-time translation of the stored status message: one literal `t!`
/// sentence per case, payloads interpolated as data. Reads the process
/// locale, exactly like the other `t!` paint sites.
pub(crate) fn message_text(message: &ProjectMessage) -> String {
    message_text_in(message, &rust_i18n::locale())
}

/// [`message_text`] under an explicit locale — the seam the locale-flip
/// test uses so parallel tests never touch the process-global locale.
pub(super) fn message_text_in(message: &ProjectMessage, locale: &str) -> String {
    match message {
        ProjectMessage::Saved => t!("Project saved", locale = locale).to_string(),
        ProjectMessage::Opened => t!("Project opened", locale = locale).to_string(),
        ProjectMessage::LoadRequested => t!("Project load requested", locale = locale).to_string(),
        ProjectMessage::NotLoaded { cause } => t!(
            "Project was not loaded: %{error}",
            locale = locale,
            error = cause
        )
        .to_string(),
        ProjectMessage::SaveDialogFailed { detail } => t!(
            "Could not open save dialog: %{error}",
            locale = locale,
            error = detail
        )
        .to_string(),
        ProjectMessage::LoadDialogFailed { detail } => t!(
            "Could not open file dialog: %{error}",
            locale = locale,
            error = detail
        )
        .to_string(),
        ProjectMessage::AutoSaveFailed { cause } => t!(
            "Could not auto-save: %{error}",
            locale = locale,
            error = cause
        )
        .to_string(),
        ProjectMessage::PreferencesSaved => t!("Preferences saved", locale = locale).to_string(),
        ProjectMessage::PreferencesSaveQueued => {
            t!("Preferences save queued", locale = locale).to_string()
        }
        ProjectMessage::PreferencesSaveFailed { cause } => t!(
            "Could not save preferences: %{error}",
            locale = locale,
            error = cause
        )
        .to_string(),
        ProjectMessage::Persisted(error) => persistence_error_text(error, locale),
    }
}

/// One literal sentence per closed [`PersistedFailure`] variant; each
/// variant's own fields ride along as interpolated diagnostic detail.
fn persistence_error_text(error: &PersistedFailure, locale: &str) -> String {
    match error {
        PersistedFailure::Io {
            operation,
            path,
            source,
        } => t!(
            "Could not %{operation} %{path}: %{source}",
            locale = locale,
            operation = operation,
            path = path.display().to_string(),
            source = source,
        )
        .to_string(),
        PersistedFailure::Json { path, source } => t!(
            "Invalid JSON in %{path}: %{source}",
            locale = locale,
            path = path.display().to_string(),
            source = source,
        )
        .to_string(),
        PersistedFailure::Invalid { field, reason } => t!(
            "Invalid %{field}: %{reason}",
            locale = locale,
            field = field,
            reason = reason,
        )
        .to_string(),
        PersistedFailure::UnsupportedVersion { document, version } => t!(
            "Unsupported %{document} document version %{version}",
            locale = locale,
            document = document,
            version = version,
        )
        .to_string(),
        PersistedFailure::TooLarge { path } => t!(
            "Document %{path} exceeds the 8 MiB limit",
            locale = locale,
            path = path.display().to_string(),
        )
        .to_string(),
        PersistedFailure::ConfigurationDirectoryUnavailable => t!(
            "User configuration directory is unavailable",
            locale = locale
        )
        .to_string(),
    }
}
