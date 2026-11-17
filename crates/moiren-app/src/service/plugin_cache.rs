//! Disk cache for sandboxed scan reports, keyed by scan root.
//!
//! A full scan spawns one sandboxed subprocess per root and probes every
//! plugin binary it finds — expensive enough that repeating it unchanged
//! (the picker's refresh row, a rescan after an unrelated edit) is worth
//! avoiding. The cache stores, per scanned root, the completed
//! [`SandboxReport`] plus the recursive maximum modification time of the
//! root's tree at scan time. A later scan reuses the cached report when that
//! stamp still matches the tree and re-scans the root when it does not.
//!
//! Why a recursive stamp instead of the root directory's own mtime: a
//! directory's mtime only moves when its direct children change, so an
//! in-place plugin update inside an existing bundle would go unseen. The
//! walk is stat-only — orders of magnitude cheaper than the sandbox it
//! guards — and catches creations, deletions, renames, and overwrites.
//!
//! Storage follows the project file conventions: a versioned JSON document
//! beside `preferences.json`, written through the same atomic
//! temp-file-rename helper. The cache is pure derived data, so every read
//! failure (missing, corrupt, wrong version, oversized) degrades to "no
//! cache" and the next scan rebuilds it; no failure ever blocks a scan.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use log::warn;
use moiren_plugins::sandbox::SandboxReport;
use serde::{Deserialize, Serialize};

/// Beside the preferences and autosave documents (see
/// `crate::project::storage`), same directory conventions.
const CACHE_FILENAME: &str = "plugin-scan-cache.json";
/// Bumped whenever the entry shape changes incompatibly; a mismatching file
/// is discarded wholesale rather than migrated — the cache rebuilds itself.
const CACHE_VERSION: u32 = 1;

/// A filesystem modification time, normalized so it can be compared and
/// serialized. `None` means "unstampable" (the walk failed): such a root can
/// never be a cache hit and is re-scanned every time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct TreeStamp {
    secs: u64,
    nanos: u32,
}

impl TreeStamp {
    fn of(time: SystemTime) -> Self {
        // Pre-epoch clocks are pathological; clamping to the epoch keeps the
        // stamp orderable instead of failing the walk.
        let since = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        Self {
            secs: since.as_secs(),
            nanos: since.subsec_nanos(),
        }
    }
}

/// One scanned root's cached outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ScanCacheEntry {
    /// Unix seconds when the scan completed. Display/debug provenance; the
    /// invalidation decision reads only the tree stamp.
    scanned_at: u64,
    /// The recursive max mtime of the root's tree at scan time; `None` when
    /// the tree could not be stamped (never a hit).
    tree_stamp: Option<TreeStamp>,
    report: SandboxReport,
}

/// The persisted document: deterministic ordering (`BTreeMap`), versioned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ScanCacheFile {
    version: u32,
    /// Roots keyed by their display string; roots are absolute host paths
    /// and effectively always Unicode, and a string key keeps the JSON and
    /// the ordering stable across platforms.
    roots: BTreeMap<String, ScanCacheEntry>,
}

impl Default for ScanCacheFile {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            roots: BTreeMap::new(),
        }
    }
}

fn cache_key(root: &Path) -> String {
    root.display().to_string()
}

/// Where the cache document lives: beside the preferences document, per the
/// project storage conventions (`preferences.json`, `autosave.moiren.json`).
pub(super) fn cache_path() -> Result<PathBuf, crate::project::PersistenceError> {
    crate::project::preferences_path().map(|preferences| preferences.with_file_name(CACHE_FILENAME))
}

/// The recursive maximum modification time over the root's tree, covering
/// directories and files. Any walk failure (vanished root, denied entry)
/// surfaces as `Err`: the caller treats the root as unstampable, which
/// always costs a re-scan and never a stale hit.
pub(super) fn tree_stamp(root: &Path) -> io::Result<Option<TreeStamp>> {
    fn walk(path: &Path, stamp: &mut Option<TreeStamp>) -> io::Result<()> {
        let metadata = fs::metadata(path)?;
        let modified = TreeStamp::of(metadata.modified()?);
        if stamp.is_none_or(|current| modified > current) {
            *stamp = Some(modified);
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                walk(&entry?.path(), stamp)?;
            }
        }
        Ok(())
    }
    let mut stamp = None;
    walk(root, &mut stamp)?;
    Ok(stamp)
}

/// The loaded cache. `path` is `None` when the configuration directory is
/// unavailable: the cache then degrades to in-memory only for this scan.
pub(super) struct ScanCache {
    file: ScanCacheFile,
    path: Option<PathBuf>,
    dirty: bool,
}

impl ScanCache {
    /// Load the cache from `path`. Every failure is an empty cache: a
    /// missing file is a first scan, and a damaged one must never block or
    /// skew a scan.
    pub(super) fn load_at(path: &Path) -> Self {
        let file = match crate::project::read_json::<ScanCacheFile>(path) {
            Ok(file) if file.version == CACHE_VERSION => file,
            Ok(file) => {
                warn!(
                    target: "moiren::audio::service",
                    "plugin scan cache version {} discarded (expected {CACHE_VERSION})",
                    file.version
                );
                ScanCacheFile::default()
            }
            Err(crate::project::PersistenceError::Io { source, .. })
                if source.kind() == io::ErrorKind::NotFound =>
            {
                ScanCacheFile::default()
            }
            Err(error) => {
                warn!(
                    target: "moiren::audio::service",
                    "plugin scan cache unreadable, rescanning: {error}"
                );
                ScanCacheFile::default()
            }
        };
        Self {
            file,
            path: Some(path.to_path_buf()),
            dirty: false,
        }
    }

    /// The cached report for `root` when the tree's current stamp still
    /// matches the one recorded at scan time. A changed, vanished, or
    /// unstamped tree misses.
    pub(super) fn usable(&self, root: &Path) -> Option<&SandboxReport> {
        let entry = self.file.roots.get(&cache_key(root))?;
        match (entry.tree_stamp, tree_stamp(root).ok().flatten()) {
            (Some(recorded), Some(current)) if recorded == current => Some(&entry.report),
            _ => None,
        }
    }

    /// Record a completed report for `root`, stamped with the tree's current
    /// state. An unstampable tree is recorded with `None` so the report is
    /// kept for this scan's merge but can never satisfy a later hit.
    pub(super) fn record(&mut self, root: &Path, report: SandboxReport) {
        let tree_stamp = tree_stamp(root).ok().flatten();
        let changed = self
            .file
            .roots
            .get(&cache_key(root))
            .is_none_or(|entry| entry.tree_stamp != tree_stamp || entry.report != report);
        self.file.roots.insert(
            cache_key(root),
            ScanCacheEntry {
                scanned_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_secs(),
                tree_stamp,
                report,
            },
        );
        self.dirty |= changed;
    }

    /// Drop a root's entry: a re-scan of a changed root that failed leaves
    /// no stale data behind, and the next scan retries the root.
    pub(super) fn drop_root(&mut self, root: &Path) {
        if self.file.roots.remove(&cache_key(root)).is_some() {
            self.dirty = true;
        }
    }

    /// Retain only the roots this scan covered (the cache mirrors the last
    /// scan, so roots dropped from the standard set do not linger) and write
    /// best-effort. A failed write logs and continues: the scan results are
    /// already in hand, and the next scan rebuilds the cache.
    pub(super) fn save(&mut self, scanned_roots: &[PathBuf]) {
        if !self.dirty {
            return;
        }
        let keys: Vec<String> = scanned_roots.iter().map(|root| cache_key(root)).collect();
        self.file.roots.retain(|key, _| keys.contains(key));
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            warn!(
                target: "moiren::audio::service",
                "plugin scan cache directory unavailable: {error}"
            );
            return;
        }
        match crate::project::write_json(path, &self.file) {
            Ok(()) => self.dirty = false,
            Err(error) => {
                warn!(
                    target: "moiren::audio::service",
                    "plugin scan cache write failed: {error}"
                );
            }
        }
    }
}

/// Minimal RAII scratch directory shared by the plugin-service tests; the
/// tests remove their own debris so no developer machine state is touched.
#[cfg(test)]
pub(in crate::service) mod test_support {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    /// How long a mtime-sensitive test waits between two filesystem changes
    /// whose stamps it expects to order strictly. Linux VFS timestamps come
    /// from the kernel's coarse clock, quantized to one scheduler tick (up
    /// to 10 ms at HZ=100; 4 ms at the CI's HZ=250), so changes inside one
    /// tick carry byte-identical mtimes — the CI reproduced exactly that.
    /// Windows/NTFS stamps precisely. One pause past the coarsest common
    /// tick makes the ordering deterministic on both platforms.
    pub(in crate::service) const MTIME_SETTLING: Duration = Duration::from_millis(50);

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub(in crate::service) struct TempDir(pub(in crate::service) PathBuf);

    impl TempDir {
        pub(in crate::service) fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "moiren-plugin-cache-test-{}-{}-{name}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(&dir).expect("scratch directory");
            Self(dir)
        }

        pub(in crate::service) fn path(&self) -> &PathBuf {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_plugins::spec::{PluginDescriptor, PluginFormat};

    fn scratch(name: &str) -> (std::path::PathBuf, test_support::TempDir) {
        let dir = test_support::TempDir::new(name);
        (dir.path().join(CACHE_FILENAME), dir)
    }

    fn descriptor(root: &Path, name: &str) -> PluginDescriptor {
        PluginDescriptor {
            format: PluginFormat::Vst3,
            path: root.join(format!("{name}.vst3")),
            id: format!("org.example.{name}"),
            name: name.into(),
            vendor: "Example".into(),
            version: "1.0".into(),
            input_channels: 2,
            output_channels: 2,
            is_synth: false,
            has_editor: false,
        }
    }

    fn report(root: &Path) -> SandboxReport {
        SandboxReport {
            descriptors: vec![moiren_plugins::sandbox::SandboxDescriptor {
                descriptor: descriptor(root, "gain"),
                parameters: Vec::new(),
                latency_frames: 0,
            }],
            failures: Vec::new(),
        }
    }

    /// A plugin root with one nested plugin bundle, so the walk has depth.
    fn plugin_root(base: &Path) -> PathBuf {
        let root = base.join("VST3");
        let bundle = root.join("gain.vst3").join("Contents").join("x86_64-win");
        fs::create_dir_all(&bundle).expect("nested bundle directories");
        fs::write(bundle.join("gain.vst3"), "binary").expect("plugin file");
        root
    }

    #[test]
    fn tree_stamp_tracks_nested_changes() {
        let guard = test_support::TempDir::new("stamp-depth");
        let root = plugin_root(&guard.0);
        let first = tree_stamp(&root).expect("stampable root").expect("stamp");
        // Overwrite the nested binary: the root's own mtime would not move,
        // but the recursive stamp must.
        std::thread::sleep(test_support::MTIME_SETTLING);
        fs::write(
            root.join("gain.vst3/Contents/x86_64-win/gain.vst3"),
            "updated",
        )
        .expect("overwrite");
        let second = tree_stamp(&root).expect("stampable root").expect("stamp");
        assert!(second > first, "an in-place update must move the stamp");
        // Creating a sibling bundle moves it too. The write above and this
        // mkdir are two metadata changes a coarse-clock filesystem can stamp
        // identically when they share one tick, so they must not share one.
        std::thread::sleep(test_support::MTIME_SETTLING);
        fs::create_dir_all(root.join("other.vst3")).expect("new bundle");
        let third = tree_stamp(&root).expect("stampable root").expect("stamp");
        assert!(third > second);
        // And a vanished root is unstampable, never a false hit.
        assert!(tree_stamp(&root.join("missing")).is_err());
    }

    #[test]
    fn hit_serves_the_cached_report_until_the_tree_changes() {
        let (path, guard) = scratch("hit");
        let root = plugin_root(&guard.0);
        let mut cache = ScanCache::load_at(&path);
        cache.record(&root, report(&root));
        cache.save(std::slice::from_ref(&root));

        let reloaded = ScanCache::load_at(&path);
        let cached = reloaded
            .usable(&root)
            .expect("unchanged tree is a cache hit");
        assert_eq!(cached, &report(&root));

        // Touching a nested file invalidates the entry.
        std::thread::sleep(test_support::MTIME_SETTLING);
        fs::write(
            root.join("gain.vst3/Contents/x86_64-win/gain.vst3"),
            "changed",
        )
        .expect("touch nested file");
        assert!(reloaded.usable(&root).is_none(), "a changed tree must miss");
    }

    #[test]
    fn save_is_skipped_until_a_root_changes_and_prunes_unscanned_roots() {
        let (path, guard) = scratch("save");
        let root = plugin_root(&guard.0);
        let mut cache = ScanCache::load_at(&path);
        cache.record(&root, report(&root));
        // Same report, same stamp: the second record is idempotent.
        cache.record(&root, report(&root));
        cache.save(std::slice::from_ref(&root));
        let first_write = fs::metadata(&path).expect("cache written").len();
        let saved_at = fs::metadata(&path)
            .expect("cache written")
            .modified()
            .expect("mtime");

        // A rescan of an unchanged root records the identical entry and must
        // not touch the file again.
        let mut again = ScanCache::load_at(&path);
        again.record(&root, report(&root));
        again.save(std::slice::from_ref(&root));
        assert_eq!(
            fs::metadata(&path).expect("cache written").modified().ok(),
            Some(saved_at),
            "an unchanged rescan must not rewrite the cache"
        );

        // Roots outside this scan's set are pruned on save.
        let other = guard.0.join("elsewhere");
        fs::create_dir_all(&other).expect("other root");
        let mut pruning = ScanCache::load_at(&path);
        pruning.record(&other, report(&other));
        pruning.save(std::slice::from_ref(&root));
        let pruned = ScanCache::load_at(&path);
        assert!(pruned.usable(&root).is_some(), "scanned root retained");
        assert!(pruned.usable(&other).is_none(), "unscanned root pruned");
        assert!(first_write > 0);
    }

    #[test]
    fn failures_drop_the_entry_and_corrupt_files_degrade_to_empty() {
        let (path, guard) = scratch("drop");
        let root = plugin_root(&guard.0);
        let mut cache = ScanCache::load_at(&path);
        cache.record(&root, report(&root));
        cache.save(std::slice::from_ref(&root));

        // A failed rescan of this root drops the stale entry.
        let mut failed = ScanCache::load_at(&path);
        failed.drop_root(&root);
        failed.save(std::slice::from_ref(&root));
        assert!(
            ScanCache::load_at(&path).usable(&root).is_none(),
            "a dropped root must not serve stale reports"
        );

        // Corrupt bytes, a wrong version, and a directory in place of the
        // file all degrade to an empty cache instead of failing the scan.
        std::fs::write(&path, "{not json").expect("corrupt the cache");
        assert!(
            ScanCache::load_at(&path).usable(&root).is_none(),
            "corrupt cache is not a hit"
        );
        let stale: serde_json::Value = serde_json::json!({
            "version": 0,
            "roots": {},
        });
        std::fs::write(
            &path,
            serde_json::to_vec(&stale).expect("serialize stale version"),
        )
        .expect("write stale version");
        assert!(
            ScanCache::load_at(&path).usable(&root).is_none(),
            "an older version is discarded"
        );
        std::fs::remove_file(&path).expect("remove file");
        std::fs::create_dir(&path).expect("a directory blocks the read");
        assert!(
            ScanCache::load_at(&path).usable(&root).is_none(),
            "an unreadable cache is not a hit"
        );
    }

    #[test]
    fn unstampable_roots_are_recorded_but_never_hit() {
        let (path, guard) = scratch("unstampable");
        let missing = guard.0.join("vanished");
        let mut cache = ScanCache::load_at(&path);
        cache.record(&missing, report(&missing));
        assert!(
            cache.usable(&missing).is_none(),
            "an unstampable root must never be a cache hit"
        );
    }
}
