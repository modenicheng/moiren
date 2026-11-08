//! The Preferences group of StudioShell state (design 20 §4): the live
//! preference values plus the quiet-save machinery that persists them.

use gpui_kit::*;

#[derive(Default)]
pub(crate) struct PreferenceStore {
    pub(crate) values: crate::project::AppPreferences,
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) save: Option<Task<()>>,
    pub(crate) save_revision: u64,
    pub(crate) quitting: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
