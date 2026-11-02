//! Shared dynamic-library loading with one live handle per module file.
//!
//! Formats that pair a module-level init/deinit with their entry point
//! (CLAP) must run the pair once per actual load, not once per instance.
//! The registry below deduplicates concurrent loads of the same file by
//! canonical path, so entry `init` is called on first acquire and the
//! matching `deinit` runs exactly when the last instance drops its handle
//! (which then unloads the library).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

use libloading::Library;

use crate::spec::PluginError;

/// Keeps one library mapped while any instance uses it. `deinit` runs in
/// `Drop`, before the library handle is released.
pub(crate) struct SharedModule {
    pub library: Library,
    deinit: Option<unsafe extern "C" fn()>,
}

// A loaded module is address-independent state; every format spec allows
// creating and destroying instances from any thread. The deinit function
// belongs to the same module and is invoked exactly once here.
unsafe impl Send for SharedModule {}
unsafe impl Sync for SharedModule {}

impl Drop for SharedModule {
    fn drop(&mut self) {
        if let Some(deinit) = self.deinit {
            // SAFETY: called once, when the last handle drops, after every
            // plugin instance has been destroyed. The format contract
            // guarantees the module stays registered until deinit returns.
            unsafe { deinit() };
        }
    }
}

fn registry() -> &'static Mutex<HashMap<PathBuf, Weak<SharedModule>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<SharedModule>>>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

/// Acquire the shared handle for `path`, loading it if no live handle
/// exists. `init` receives the freshly loaded library exactly once per
/// actual load and may run module-level initialization (CLAP entry init);
/// it returns the module-level deinit to run at final release, if any.
pub(crate) fn acquire_module(
    path: &Path,
    init: impl FnOnce(&Library) -> Result<Option<unsafe extern "C" fn()>, PluginError>,
) -> Result<Arc<SharedModule>, PluginError> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut registry = registry().lock().expect("module registry poisoned");
    if let Some(live) = registry.get(&canonical).and_then(Weak::upgrade) {
        return Ok(live);
    }
    // Loading is serialized under the registry lock by design: it can run
    // arbitrary module code and take unbounded time, which is acceptable
    // control-side, and it keeps the Weak bookkeeping single-threaded.
    let library = unsafe { Library::new(&canonical) }
        .map_err(|error| PluginError::Load(format!("{canonical:?}: {error}")))?;
    let deinit = init(&library)?;
    let module = Arc::new(SharedModule { library, deinit });
    registry.insert(canonical, Arc::downgrade(&module));
    Ok(module)
}
