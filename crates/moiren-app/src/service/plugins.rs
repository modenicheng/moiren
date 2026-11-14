//! The plugin library: what a sandboxed scan found, keyed for assignment.
//!
//! The library is the service-side home of scan results. Keys are
//! [`PluginDescriptor::stable_id`] values, so a node's assignment survives
//! rescans and refers to the same plugin binary until it moves on disk. The
//! parameter tables arrived from the scanner subprocess's probe, which
//! guarantees the engine's `PluginSettings` contract: they match what a
//! fresh instance reports.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

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

/// Run one full sandboxed scan over the standard roots. Blocking — submit
/// through the job pool; never call on the service thread. Failures (a
/// missing scanner executable, for instance) surface as `Err` while
/// per-plugin problems stay inside the report as failures.
pub fn run_sandbox_scan() -> Result<PluginLibrary, super::BackendError> {
    let config = sandbox::SandboxConfig::discover()
        .map_err(|error| super::BackendError::new("plugin-scan", error.to_string()))?;
    let roots = scan::default_directories();
    let mut library = PluginLibrary::from_sandbox_report(sandbox::scan_sandboxed(&roots, &config));
    library.roots = roots;
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
}
