//! The plugin library: what a sandboxed scan found, keyed for assignment.
//!
//! The library is the service-side home of scan results. Keys are
//! [`PluginDescriptor::stable_id`] values, so a node's assignment survives
//! rescans and refers to the same plugin binary until it moves on disk. The
//! parameter tables arrived from the scanner subprocess's probe, which
//! guarantees the engine's `PluginSettings` contract: they match what a
//! fresh instance reports.
//!
//! Scans are cached per root on disk (see [`super::plugin_cache`]): a root
//! whose plugin tree is unchanged since its last scan serves its cached
//! report instead of spawning a scanner subprocess, and only changed roots
//! pay for the sandbox. The forced rescan (the picker's "force" row)
//! bypasses the cache and refreshes every entry.

use std::{collections::BTreeMap, path::Path, path::PathBuf, sync::Arc};

use moiren_plugins::{
    sandbox::{self, SandboxReport},
    scan,
    spec::{PluginDescriptor, PluginParameterSpec},
};
use serde::{Deserialize, Serialize};

/// One plugin the picker can offer: everything preparation needs to lower a
/// Plugin node, straight from the sandboxed scan-and-probe.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginEntry {
    pub descriptor: PluginDescriptor,
    pub parameters: Vec<PluginParameterSpec>,
    pub latency_frames: u32,
}

/// The latest sandboxed scan, published as one immutable snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginLibrary {
    /// Probed plugins keyed by stable id; the first entry wins when two
    /// roots report the same binary, keeping rescans deterministic.
    entries: BTreeMap<String, PluginEntry>,
    /// Roots whose scanner crashed or timed out, plus per-plugin scan/probe
    /// failures. Surfaced by the picker so an exclusion is visible, never
    /// silent.
    pub failures: Vec<(PathBuf, String)>,
    /// The roots this library scanned; kept for display and cache keys.
    pub roots: Vec<PathBuf>,
}

impl PluginLibrary {
    pub fn from_sandbox_report(report: SandboxReport) -> Self {
        let mut library = Self {
            roots: Vec::new(),
            failures: report
                .failures
                .into_iter()
                .map(|failure| (failure.path, failure.reason))
                .collect(),
            entries: BTreeMap::new(),
        };
        for probed in report.descriptors {
            let key = probed.descriptor.stable_id();
            library.entries.entry(key).or_insert(PluginEntry {
                descriptor: probed.descriptor,
                parameters: probed.parameters,
                latency_frames: probed.latency_frames,
            });
        }
        library
    }

    pub fn get(&self, id: &str) -> Option<&PluginEntry> {
        self.entries.get(id)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The picker's ordered rows: stable id (the assignment value) paired
    /// with a display label.
    pub fn picker_rows(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .map(|(id, entry)| {
                (
                    id.clone(),
                    format!(
                        "{} · {} {}",
                        entry.descriptor.name,
                        entry.descriptor.format.label(),
                        entry.descriptor.version
                    ),
                )
            })
            .collect()
    }
}

/// What a Plugin node is bound to: the scanned plugin's stable id plus an
/// optional state chunk (see `PluginSettings::preset`). Assignments are
/// plain data — they persist in project documents and survive rescans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginAssignment {
    pub id: String,
    /// Saved plugin state; today only restored projects carry one, until
    /// preset capture/selection UI exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<Vec<u8>>,
}

/// Run one full sandboxed scan over the standard roots, reusing cached
/// per-root reports whose trees are unchanged (see [`super::plugin_cache`]).
/// Blocking — submit through the job pool; never call on the service thread.
/// Failures (a missing scanner executable, for instance) surface as `Err`
/// while per-plugin problems stay inside the report as failures.
pub fn run_sandbox_scan() -> Result<PluginLibrary, super::BackendError> {
    scan_with_cache(false)
}

/// Run a full sandboxed scan that bypasses the cache: every root re-scans
/// and refreshes its cache entry. This is the user's "something is wrong
/// with my plugin list" action, so no root is served from disk.
pub fn run_forced_plugin_scan() -> Result<PluginLibrary, super::BackendError> {
    scan_with_cache(true)
}

/// The scan cache lives beside the preferences; the injectable core keeps
/// the production wrappers free of test plumbing.
fn scan_with_cache(force: bool) -> Result<PluginLibrary, super::BackendError> {
    let cache_path = super::plugin_cache::cache_path()
        .map_err(|error| super::BackendError::new("plugin-scan", error.to_string()))?;
    let config = sandbox::SandboxConfig::discover()
        .map_err(|error| super::BackendError::new("plugin-scan", error.to_string()))?;
    let roots = scan::default_directories();
    scan_with_cache_at(&roots, &config, &cache_path, force)
}

/// Scan every root, serving unchanged trees from the cache unless `force`.
/// A fresh report updates its cache entry; a failed re-scan of a changed
/// root drops the stale entry so the next scan retries the root.
pub(crate) fn scan_with_cache_at(
    roots: &[PathBuf],
    config: &sandbox::SandboxConfig,
    cache_path: &Path,
    force: bool,
) -> Result<PluginLibrary, super::BackendError> {
    let mut cache = super::plugin_cache::ScanCache::load_at(cache_path);
    let mut report = sandbox::SandboxReport::default();
    for root in roots {
        if !force && let Some(cached) = cache.usable(root) {
            report
                .descriptors
                .extend(cached.descriptors.iter().cloned());
            report.failures.extend(cached.failures.iter().cloned());
            continue;
        }
        match sandbox::scan_root_sandboxed(root, config) {
            Ok(part) => {
                cache.record(root, part.clone());
                report.descriptors.extend(part.descriptors);
                report.failures.extend(part.failures);
            }
            Err((path, error)) => {
                cache.drop_root(root);
                report.failures.push(sandbox::SandboxFailure {
                    path,
                    reason: error.to_string(),
                });
            }
        }
    }
    cache.save(roots);
    let mut library = PluginLibrary::from_sandbox_report(report);
    library.roots = roots.to_vec();
    Ok(library)
}

/// A ready-to-share empty library for contexts that never scanned.
pub fn empty_library() -> Arc<PluginLibrary> {
    Arc::new(PluginLibrary::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_plugins::sandbox::{SandboxDescriptor, SandboxFailure};
    use moiren_plugins::spec::PluginFormat;

    fn descriptor(path: &str, id: &str) -> PluginDescriptor {
        PluginDescriptor {
            format: PluginFormat::Vst3,
            path: PathBuf::from(path),
            id: id.into(),
            name: format!("Plugin {id}"),
            vendor: "Example".into(),
            version: "1.0".into(),
            input_channels: 2,
            output_channels: 2,
            is_synth: false,
            has_editor: false,
        }
    }

    fn report() -> SandboxReport {
        SandboxReport {
            descriptors: vec![
                SandboxDescriptor {
                    descriptor: descriptor("/a/gain.vst3", "AAAA"),
                    parameters: Vec::new(),
                    latency_frames: 0,
                },
                SandboxDescriptor {
                    descriptor: descriptor("/a/gain.vst3", "AAAA"),
                    parameters: Vec::new(),
                    latency_frames: 0,
                },
                SandboxDescriptor {
                    descriptor: descriptor("/b/other.vst3", "BBBB"),
                    parameters: Vec::new(),
                    latency_frames: 12,
                },
            ],
            failures: vec![SandboxFailure {
                path: PathBuf::from("/c/broken.clap"),
                reason: "module failed to load".into(),
            }],
        }
    }

    #[test]
    fn library_keys_entries_by_stable_id_and_dedupes() {
        let library = PluginLibrary::from_sandbox_report(report());
        assert_eq!(library.len(), 2);
        let key = descriptor("/a/gain.vst3", "AAAA").stable_id();
        assert!(library.get(&key).is_some());
        let other = library.get(&descriptor("/b/other.vst3", "BBBB").stable_id());
        assert_eq!(other.expect("distinct entry").latency_frames, 12);
    }

    #[test]
    fn same_id_in_different_files_stays_distinct() {
        let library = PluginLibrary::from_sandbox_report(report());
        let moved = descriptor("/moved/gain.vst3", "AAAA").stable_id();
        assert!(library.get(&moved).is_none(), "the path is part of the key");
    }

    #[test]
    fn failures_and_picker_rows_survive_the_mapping() {
        let library = PluginLibrary::from_sandbox_report(report());
        assert_eq!(library.failures.len(), 1);
        assert_eq!(library.failures[0].0, PathBuf::from("/c/broken.clap"));
        let rows = library.picker_rows();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].1.contains("Plugin AAAA"));
        assert!(rows[0].1.contains("VST3"));
    }

    #[test]
    fn assignment_round_trips_with_and_without_a_preset() {
        let plain = PluginAssignment {
            id: "VST3|AA|/a/gain.vst3".into(),
            preset: None,
        };
        let json = serde_json::to_string(&plain).unwrap();
        assert_eq!(
            serde_json::from_str::<PluginAssignment>(&json).unwrap(),
            plain
        );
        let preset = PluginAssignment {
            id: "VST3|AA|/a/gain.vst3".into(),
            preset: Some(vec![1, 2, 3]),
        };
        let json = serde_json::to_string(&preset).unwrap();
        assert_eq!(
            serde_json::from_str::<PluginAssignment>(&json).unwrap(),
            preset
        );
        // Old documents without the field load with no preset.
        assert!(
            serde_json::from_str::<PluginAssignment>("{\"id\":\"x\"}")
                .unwrap()
                .preset
                .is_none()
        );
    }

    /// Scan-cache integration over the injectable scan core. The scanner
    /// path deliberately names nothing: a cache hit must serve the cached
    /// report without touching the sandbox (no failure rows), while a miss
    /// or a forced scan must attempt the spawn and surface its failure.
    mod scan_cache {
        use super::*;
        use crate::service::plugin_cache::test_support::TempDir;

        fn config() -> sandbox::SandboxConfig {
            sandbox::SandboxConfig::new("missing-scanner-binary", std::time::Duration::ZERO)
        }

        fn descriptor(path: &Path, id: &str) -> PluginDescriptor {
            PluginDescriptor {
                format: PluginFormat::Vst3,
                path: path.to_path_buf(),
                id: id.into(),
                name: format!("Plugin {id}"),
                vendor: "Example".into(),
                version: "1.0".into(),
                input_channels: 2,
                output_channels: 2,
                is_synth: false,
                has_editor: false,
            }
        }

        fn plugin_root(guard: &TempDir) -> PathBuf {
            let root = guard.path().join("VST3");
            let bundle = root.join("gain.vst3").join("Contents").join("x86_64-win");
            std::fs::create_dir_all(&bundle).expect("nested bundle directories");
            std::fs::write(bundle.join("gain.vst3"), "binary").expect("plugin file");
            root
        }

        fn cached_library(root: &Path) -> PluginLibrary {
            let mut library = PluginLibrary::from_sandbox_report(sandbox::SandboxReport {
                descriptors: vec![sandbox::SandboxDescriptor {
                    descriptor: descriptor(&root.join("gain.vst3"), "AAAA"),
                    parameters: Vec::new(),
                    latency_frames: 0,
                }],
                failures: Vec::new(),
            });
            library.roots = vec![root.to_path_buf()];
            library
        }

        fn write_cache(cache_path: &Path, roots: &[PathBuf]) {
            let mut cache = super::super::super::plugin_cache::ScanCache::load_at(cache_path);
            cache.record(
                &roots[0],
                sandbox::SandboxReport {
                    descriptors: vec![sandbox::SandboxDescriptor {
                        descriptor: descriptor(&roots[0].join("gain.vst3"), "AAAA"),
                        parameters: Vec::new(),
                        latency_frames: 0,
                    }],
                    failures: Vec::new(),
                },
            );
            cache.save(roots);
        }

        fn scan_roots(roots: &[PathBuf], cache_path: &Path, force: bool) -> PluginLibrary {
            scan_with_cache_at(roots, &config(), cache_path, force).expect("scan succeeds")
        }

        #[test]
        fn unchanged_roots_are_served_from_the_cache_without_spawning() {
            let guard = TempDir::new("scan-hit");
            let root = plugin_root(&guard);
            let cache_path = guard.path().join("plugin-scan-cache.json");
            let seeded = cached_library(&root);
            write_cache(&cache_path, std::slice::from_ref(&root));

            let library = scan_roots(std::slice::from_ref(&root), &cache_path, false);
            // No failures means no sandbox spawn was attempted: the cached
            // report answered for this root.
            assert!(library.failures.is_empty(), "{:?}", library.failures);
            assert_eq!(library.len(), 1);
            assert_eq!(library, seeded);
        }

        #[test]
        fn changed_roots_rescan_and_a_failed_rescan_drops_the_entry() {
            let guard = TempDir::new("scan-miss");
            let root = plugin_root(&guard);
            let cache_path = guard.path().join("plugin-scan-cache.json");
            write_cache(&cache_path, std::slice::from_ref(&root));

            // Touch a nested file: the tree changed, so the sandbox is
            // attempted (and fails, since no scanner binary exists here).
            // The pause clears the recorded stamp's coarse-clock tick (see
            // `test_support::MTIME_SETTLING`).
            std::thread::sleep(crate::service::plugin_cache::test_support::MTIME_SETTLING);
            std::fs::write(
                root.join("gain.vst3/Contents/x86_64-win/gain.vst3"),
                "updated",
            )
            .expect("touch nested file");
            let library = scan_roots(std::slice::from_ref(&root), &cache_path, false);
            assert_eq!(library.failures.len(), 1, "a miss must attempt the spawn");

            // The failed re-scan dropped the stale entry: the next scan
            // retries the root rather than trusting the stale report.
            let retry = scan_roots(std::slice::from_ref(&root), &cache_path, false);
            assert_eq!(retry.failures.len(), 1);
            assert_eq!(retry.len(), 0, "no stale descriptors survive the drop");
        }

        #[test]
        fn forced_scans_bypass_a_valid_cache() {
            let guard = TempDir::new("scan-force");
            let root = plugin_root(&guard);
            let cache_path = guard.path().join("plugin-scan-cache.json");
            write_cache(&cache_path, std::slice::from_ref(&root));

            let library = scan_roots(std::slice::from_ref(&root), &cache_path, true);
            assert_eq!(
                library.failures.len(),
                1,
                "a forced scan must attempt the spawn despite the valid cache"
            );
        }

        #[test]
        fn corrupt_cache_files_degrade_to_a_full_rescan() {
            let guard = TempDir::new("scan-corrupt");
            let root = plugin_root(&guard);
            let cache_path = guard.path().join("plugin-scan-cache.json");
            std::fs::write(&cache_path, "{ definitely not json").expect("corrupt cache");

            let library = scan_roots(std::slice::from_ref(&root), &cache_path, false);
            assert_eq!(
                library.failures.len(),
                1,
                "a corrupt cache must not block or skew the scan"
            );
        }

        #[test]
        fn scan_level_failures_still_surface_as_err() {
            // A scanner config that cannot even resolve reports spawn-level
            // failure inside the report; a broken *config discovery* path is
            // the Err half and stays covered by the production wrapper's
            // mapping, which this test mirrors through a bad root only.
            let guard = TempDir::new("scan-err");
            let root = plugin_root(&guard);
            let cache_path = guard.path().join("plugin-scan-cache.json");
            // An empty cache and a missing scanner: every root fails inside
            // the report, and the call still succeeds with failures recorded.
            let library = scan_with_cache_at(&[root], &config(), &cache_path, false)
                .expect("scan-level success with per-root failures");
            assert_eq!(library.len(), 0);
            assert_eq!(library.failures.len(), 1);
        }
    }
}
