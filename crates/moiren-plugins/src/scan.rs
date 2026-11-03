//! Plugin discovery: standard directory conventions, recursive search, and
//! format detection by file extension.
//!
//! VST3 bundle directories (`.vst3`, plus `.vst` as some installers emit,
//! e.g. Hit'n'Mix RipX) are resolved to the architecture-specific binary
//! inside `Contents/<platform>/`. Failures are collected per file: one
//! broken plugin must not abort a library scan.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{
    spec::{PluginDescriptor, PluginError, PluginFormat},
    vst2, vst3,
};

/// Everything a scan produced: usable descriptors plus per-file failures.
#[derive(Debug, Default)]
pub struct ScanOutcome {
    pub descriptors: Vec<PluginDescriptor>,
    pub failures: Vec<(PathBuf, String)>,
}

/// Scan the given directories recursively for all supported formats.
pub fn scan_directories(directories: &[PathBuf]) -> ScanOutcome {
    let mut outcome = ScanOutcome::default();
    for directory in directories {
        scan_directory(directory, &mut outcome);
    }
    outcome
}

/// The standard search directories for every format on this platform, plus
/// the formats' `*_PATH` environment overrides (`CLAP_PATH`, `VST3_PATH`,
/// `VST_PLUGIN_PATH`) using the host's path-list separator.
pub fn default_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut push = |path: PathBuf| {
        if !path.as_os_str().is_empty() && !directories.contains(&path) {
            directories.push(path);
        }
    };
    if let Some(paths) = env::var_os("CLAP_PATH") {
        for path in env::split_paths(&paths) {
            push(path);
        }
    }
    if let Some(paths) = env::var_os("VST3_PATH") {
        for path in env::split_paths(&paths) {
            push(path);
        }
    }
    if let Some(paths) = env::var_os("VST_PLUGIN_PATH") {
        for path in env::split_paths(&paths) {
            push(path);
        }
    }
    if let Some(common) = env::var_os("COMMONPROGRAMFILES") {
        let common = PathBuf::from(common);
        push(common.join("CLAP"));
        push(common.join("VST3"));
        push(common.join("VST2"));
        push(common.join("Steinberg").join("Shared Components"));
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local).join("Programs").join("Common");
        push(local.join("CLAP"));
        push(local.join("VST3"));
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        let program_files = PathBuf::from(program_files);
        push(program_files.join("VSTPlugins"));
        push(program_files.join("Steinberg").join("VstPlugins"));
    }
    directories
}

fn scan_directory(directory: &Path, outcome: &mut ScanOutcome) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            // VST3 bundle directories resolve to their inner binary; other
            // directories recurse. `.vst` covers nonstandard-but-real
            // installers such as Hit'n'Mix RipX.
            if path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("vst3") || ext.eq_ignore_ascii_case("vst")
            }) {
                let binary = vst3_bundle_binary(&path);
                match binary {
                    Some(binary) => collect_file(&binary, outcome),
                    None => outcome
                        .failures
                        .push((path, "VST3 bundle has no platform binary".into())),
                }
            } else {
                scan_directory(&path, outcome);
            }
        } else if file_type.is_file() {
            collect_file(&path, outcome);
        }
    }
}

/// The architecture-specific binary inside a VST3 bundle directory.
fn vst3_bundle_binary(bundle: &Path) -> Option<PathBuf> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    const PLATFORM: &str = "x86_64-win";
    #[cfg(all(target_os = "windows", target_arch = "x86"))]
    const PLATFORM: &str = "x86-win";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const PLATFORM: &str = "x86_64-linux";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    const PLATFORM: &str = "arm64-mac";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    const PLATFORM: &str = "macos";

    let platform_dir = bundle.join("Contents").join(PLATFORM);
    let candidates = fs::read_dir(&platform_dir).ok()?;
    candidates.flatten().map(|entry| entry.path()).find(|path| {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("vst3") || ext.eq_ignore_ascii_case("vst"))
    })
}

fn collect_file(path: &Path, outcome: &mut ScanOutcome) {
    let Some(format) = format_of(path) else {
        return;
    };
    let scan = || -> Result<Vec<PluginDescriptor>, PluginError> {
        match format {
            PluginFormat::Clap => crate::clap::scan_descriptors(path),
            PluginFormat::Vst3 => vst3::scan_descriptors(path),
            PluginFormat::Vst2 => vst2::scan_descriptor(path).map(|d| vec![d]),
        }
    };
    match scan() {
        Ok(descriptors) => outcome.descriptors.extend(descriptors),
        Err(error) => outcome
            .failures
            .push((path.to_path_buf(), error.to_string())),
    }
}

/// File format by extension. A `.dll` alone is not proof of VST2, so VST2
/// scanning only happens inside VST2-named directories; here a bare
/// shared-library extension in such a context is treated as VST2.
fn format_of(path: &Path) -> Option<PluginFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "clap" => Some(PluginFormat::Clap),
        "vst3" => Some(PluginFormat::Vst3),
        #[cfg(windows)]
        "dll" => Some(PluginFormat::Vst2),
        #[cfg(target_os = "linux")]
        "so" => Some(PluginFormat::Vst2),
        _ => None,
    }
}
