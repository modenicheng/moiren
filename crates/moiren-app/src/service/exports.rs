//! Control-side export intent and lifecycle observations. No driver is opened here.
use std::collections::{BTreeMap, BTreeSet};

use moiren_core::graph::{LogicalGraph, NodeId, NodeKind};
use moiren_engine::node::VirtualEndpointId;
use serde::{Deserialize, Serialize};

mod reconcile;
pub(crate) mod runtime;
pub use reconcile::*;
#[cfg(test)]
mod tests;

pub const MAX_EXPORTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportDirection {
    Playback,
    Microphone,
}

impl ExportDirection {
    pub fn node_kind(self) -> NodeKind {
        match self {
            Self::Playback => NodeKind::Source,
            Self::Microphone => NodeKind::Sink,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportIntent {
    pub endpoint_id: VirtualEndpointId,
    pub direction: ExportDirection,
    pub name: String,
    pub channels: usize,
    pub enabled: bool,
    /// A graph IO node, never a native handle or a runtime driver identity.
    pub binding: Option<NodeId>,
}

impl ExportIntent {
    pub fn validate(&self) -> Result<(), String> {
        validate_export_name_channels(&self.name, self.channels)
    }
}

pub(crate) fn validate_export_name_channels(name: &str, channels: usize) -> Result<(), String> {
    if name.trim().is_empty()
        || name.encode_utf16().count() > 63
        || name.chars().any(|c| c < '\u{20}')
    {
        return Err(
            "export name must contain 1..63 UTF-16 units without control characters".into(),
        );
    }
    if !(1..=2).contains(&channels) {
        return Err("virtual exports support only mono or stereo".into());
    }
    Ok(())
}

pub fn validate_exports(exports: &[ExportIntent], graph: &LogicalGraph) -> Result<(), String> {
    if exports.len() > MAX_EXPORTS {
        return Err("at most 64 virtual exports are supported".into());
    }
    let mut ids = BTreeSet::new();
    let mut bindings = BTreeSet::new();
    for export in exports {
        export.validate()?;
        if !ids.insert(export.endpoint_id) {
            return Err("duplicate virtual endpoint identity".into());
        }
        if let Some(binding) = export.binding {
            let node = graph.get_node(binding).map_err(|error| error.to_string())?;
            if node.kind() != export.direction.node_kind() || node.channels() != export.channels {
                return Err("export direction or channels differ from its graph binding".into());
            }
            if !bindings.insert(binding) {
                return Err("a graph IO node may bind only one virtual export".into());
            }
        }
    }
    Ok(())
}

/// Machine-local authorization. This type is deliberately absent from projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportAuthorization {
    pub endpoint_id: VirtualEndpointId,
    pub allow_default: bool,
}

pub fn validate_export_authorizations(values: &[ExportAuthorization]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    if values.len() > 4096 || values.iter().any(|value| !ids.insert(value.endpoint_id)) {
        return Err("export authorizations exceed 4096 entries or repeat an identity".into());
    }
    Ok(())
}

#[derive(Default)]
pub struct ExportService {
    pub(super) available: bool,
    pub(super) authorizations: BTreeMap<VirtualEndpointId, bool>,
    pub(super) observations: BTreeMap<VirtualEndpointId, ExportObservation>,
    /// Probe-only observations, never consulted for native action authority.
    #[cfg(feature = "vad-integration-probe")]
    pub(super) native_stamps: BTreeMap<VirtualEndpointId, [u64; 5]>,
}

impl ExportService {
    pub fn set_authorizations(&mut self, values: &[ExportAuthorization]) -> Result<(), String> {
        validate_export_authorizations(values)?;
        self.authorizations = values
            .iter()
            .map(|value| (value.endpoint_id, value.allow_default))
            .collect();
        Ok(())
    }

    pub fn allow_default(&self, id: VirtualEndpointId) -> bool {
        self.authorizations.get(&id).copied().unwrap_or(false)
    }

    /// Only a real lifecycle adapter may report availability and observations.
    /// Enabling a project export never calls this method or grants default use.
    pub fn set_available(&mut self, available: bool) {
        if !available {
            #[cfg(feature = "vad-integration-probe")]
            self.native_stamps.clear();
            for observation in self.observations.values_mut() {
                // Keep actual resource/gate observations until the adapter
                // closes them, but recovery must establish fresh admission.
                observation.active_revision = None;
            }
        }
        self.available = available;
    }

    pub fn observe(&mut self, observation: ExportObservation) -> Result<(), String> {
        observation.intent.validate()?;
        if !self.available || observation.generation == 0 {
            return Err(
                "virtual export lifecycle adapter is unavailable or generation is zero".into(),
            );
        }
        if self
            .observations
            .get(&observation.intent.endpoint_id)
            .is_some_and(|old| observation.generation <= old.generation)
        {
            return Err("stale virtual export lifecycle observation".into());
        }
        if (observation.active_revision.is_some()
            && observation.resource != ExportResourceState::Present)
            || (observation.gate_open
                && matches!(
                    observation.resource,
                    ExportResourceState::Absent | ExportResourceState::Unavailable
                ))
        {
            return Err(
                "export activity or gate observation contradicts its resource state".into(),
            );
        }
        // A newer unstamped observation must not inherit a previous raw stamp.
        #[cfg(feature = "vad-integration-probe")]
        self.native_stamps.remove(&observation.intent.endpoint_id);
        self.observations
            .insert(observation.intent.endpoint_id, observation);
        Ok(())
    }

    /// Native lifecycle presence alone cannot authorize PCM or replace an
    /// active graph. Audio integration will supply a separate admission hook.
    pub fn preflight_audio(&self, exports: &[ExportIntent]) -> Result<(), String> {
        if exports.iter().any(|export| export.enabled) {
            Err("virtual export audio transport is not integrated".into())
        } else {
            Ok(())
        }
    }
}
