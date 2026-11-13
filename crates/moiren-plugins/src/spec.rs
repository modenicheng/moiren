//! Shared plugin metadata types and the realtime instance contract.
//!
//! The engine automates plugins as [`PluginParameterSpec`] tables whose units
//! are format-specific: CLAP exposes plain values, VST2 and VST3 expose
//! normalized `0..1` values. `min`/`max` therefore describe the format's own
//! unit and map one-to-one onto the engine's `ParamDomain::Float`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    Clap,
    Vst3,
    Vst2,
}

impl PluginFormat {
    /// Human-facing label; also the storage tag of serialized settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Vst2 => "VST2",
        }
    }
}

/// One host-facing plugin parameter in the format's automation unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginParameterSpec {
    /// Format-specific stable parameter identity (CLAP param id, VST3 ParamID,
    /// VST2 parameter index).
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub automatable: bool,
    /// The parameter moves in whole steps. Display hint only: automation
    /// values stay floating point in every format.
    pub stepped: bool,
}

/// Everything the host knows about a loadable plugin. Scanned descriptors may
/// carry zero channel counts when a format cannot answer without
/// instantiation; a live instance always reports the real values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub format: PluginFormat,
    /// Module binary (or bundle root) the plugin was loaded from.
    pub path: PathBuf,
    /// Stable format-specific identity: CLAP plugin id, VST3 class CID text,
    /// VST2 `uniqueId` plus effect name.
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// Main audio bus channel counts. This host only links plugins whose main
    /// input and output buses have equal width.
    pub input_channels: u32,
    pub output_channels: u32,
    pub is_synth: bool,
    pub has_editor: bool,
}

impl PluginDescriptor {
    /// The bus width a plugin node must have; this host requires equal-width
    /// main buses so the node's single `channels` value serves both ports.
    pub fn channels(&self) -> u32 {
        self.output_channels
    }

    /// Stable library key for one scanned plugin: the format, the
    /// format-specific plugin id, and the module path, joined with `|`. Two
    /// descriptors share a key only if the same plugin id lives in the same
    /// binary file; the path component keeps duplicate installs of one id in
    /// different files distinct. Keys survive rescans as long as the module
    /// does not move.
    pub fn stable_id(&self) -> String {
        format!(
            "{}|{}|{}",
            self.format.label(),
            self.id,
            self.path.display()
        )
    }
}

/// One parameter change to apply at the first frame of a `process` call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PluginParamEvent {
    pub parameter: u32,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PluginError {
    #[error("plugin module failed to load: {0}")]
    Load(String),
    #[error("plugin entry point missing or invalid: {0}")]
    Entry(String),
    #[error("plugin rejected initialization or activation: {0}")]
    Init(String),
    #[error("plugin has no usable main audio bus pair (input {input}, output {output})")]
    NoAudioPorts { input: u32, output: u32 },
    #[error("plugin process call failed: {0}")]
    Process(String),
    #[error("plugin state transfer failed: {0}")]
    State(String),
}
