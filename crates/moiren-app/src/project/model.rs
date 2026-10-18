use std::collections::BTreeSet;

use moiren_engine::audio_config::{AudioSettings, ProcessingSettings};
use serde::{Deserialize, Serialize};

use super::{GraphDocument, PersistenceError};
use crate::service::{AudioLatencyProfile, InputSelection, RunLimit, SessionSpec};

pub const PROJECT_VERSION: u32 = 1;
pub const PREFERENCES_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDocument {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub session: Option<SessionDocument>,
    #[serde(default)]
    pub graph: Option<GraphDocument>,
    /// Processing travels with a project; hardware defaults and device overrides
    /// remain local user preferences so moving a project cannot rebind devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing: Option<ProcessingSettings>,
    #[serde(default)]
    pub layout: ProjectLayout,
}

impl ProjectDocument {
    pub fn new(name: impl Into<String>, session: &SessionSpec) -> Self {
        Self {
            version: PROJECT_VERSION,
            name: name.into(),
            session: Some(session.into()),
            graph: None,
            processing: None,
            layout: ProjectLayout::default(),
        }
    }

    /// A partially configured candidate remains a valid project. Its unbound
    /// resources are checked when starting audio, not when saving the graph.
    pub fn from_graph(
        name: impl Into<String>,
        graph: &crate::service::GraphSnapshot,
    ) -> Result<Self, PersistenceError> {
        let document = Self {
            version: PROJECT_VERSION,
            name: name.into(),
            session: None,
            graph: Some(GraphDocument::from_snapshot(graph)?),
            processing: None,
            layout: ProjectLayout::default(),
        };
        document.validate()?;
        Ok(document)
    }

    pub fn session_spec(&self) -> Result<Option<SessionSpec>, PersistenceError> {
        let mut session = self
            .session
            .as_ref()
            .map(SessionDocument::to_spec)
            .transpose()?;
        if let (Some(session), Some(processing)) = (&mut session, self.processing) {
            processing
                .validate()
                .map_err(|error| PersistenceError::invalid("processing", error.to_string()))?;
            // Old documents store the block size on the session. Explicit
            // project processing is authoritative when both forms are present.
            session.max_block_frames = processing.block_frames as usize;
        }
        Ok(session)
    }

    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.version != PROJECT_VERSION {
            return Err(PersistenceError::UnsupportedVersion {
                document: "project",
                version: self.version,
            });
        }
        if self.name.trim().is_empty() || self.name.len() > 256 || self.name.contains('\0') {
            return Err(PersistenceError::invalid(
                "name",
                "must contain 1 to 256 bytes without NUL",
            ));
        }
        if self.session.is_none() && self.graph.is_none() {
            return Err(PersistenceError::invalid(
                "project",
                "must contain a candidate graph or session configuration",
            ));
        }
        if let Some(session) = &self.session {
            session.validate()?;
        }
        if let Some(graph) = &self.graph {
            graph.to_graph()?;
        }
        if let Some(processing) = self.processing {
            processing
                .validate()
                .map_err(|error| PersistenceError::invalid("processing", error.to_string()))?;
        }
        self.layout.validate()?;
        if let Some(graph) = &self.graph {
            let node_ids: BTreeSet<_> = graph.nodes.iter().map(|node| node.id).collect();
            if self
                .layout
                .mixer_order
                .iter()
                .any(|id| !node_ids.contains(id))
            {
                return Err(PersistenceError::invalid(
                    "layout.mixer_order",
                    "missing graph node",
                ));
            }
            for layout in &self.layout.nodes {
                if !node_ids.contains(&layout.node_id) {
                    return Err(PersistenceError::invalid(
                        "layout.nodes",
                        "position references a missing graph node",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDocument {
    pub source: SourceDocument,
    pub output_endpoint_id: String,
    pub limit: RunLimitDocument,
    pub gain: f64,
    pub pan: f64,
    pub max_block_frames: usize,
}

impl From<&SessionSpec> for SessionDocument {
    fn from(spec: &SessionSpec) -> Self {
        Self {
            source: (&spec.source).into(),
            output_endpoint_id: spec.output_endpoint_id.clone(),
            limit: spec.limit.into(),
            gain: spec.gain,
            pan: spec.pan,
            max_block_frames: spec.max_block_frames,
        }
    }
}

impl SessionDocument {
    pub fn validate(&self) -> Result<(), PersistenceError> {
        self.source.validate()?;
        validate_identity(&self.output_endpoint_id, "session.output_endpoint_id")?;
        validate_gain_pan(self.gain, self.pan, "session")?;
        if !(1..=4096).contains(&self.max_block_frames) {
            return Err(PersistenceError::invalid(
                "session.max_block_frames",
                "must be in 1..=4096",
            ));
        }
        if matches!(self.limit, RunLimitDocument::Seconds(n) if !(1..=600).contains(&n)) {
            return Err(PersistenceError::invalid(
                "session.limit",
                "duration must be in 1..=600 seconds",
            ));
        }
        Ok(())
    }

    pub fn to_spec(&self) -> Result<SessionSpec, PersistenceError> {
        self.validate()?;
        Ok(SessionSpec {
            source: self.source.to_selection(),
            output_endpoint_id: self.output_endpoint_id.clone(),
            limit: self.limit.into(),
            gain: self.gain,
            pan: self.pan,
            max_block_frames: self.max_block_frames,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceDocument {
    Physical { endpoint_id: String },
    Process { pid: u32, creation_time_100ns: u64 },
    Tone { frequency_hz: f64 },
}

impl From<&InputSelection> for SourceDocument {
    fn from(source: &InputSelection) -> Self {
        match source {
            InputSelection::Physical { endpoint_id } => Self::Physical {
                endpoint_id: endpoint_id.clone(),
            },
            InputSelection::Process {
                pid,
                creation_time_100ns,
            } => Self::Process {
                pid: *pid,
                creation_time_100ns: *creation_time_100ns,
            },
            InputSelection::Tone { frequency_hz } => Self::Tone {
                frequency_hz: *frequency_hz,
            },
        }
    }
}

impl SourceDocument {
    pub fn validate(&self) -> Result<(), PersistenceError> {
        match self {
            Self::Physical { endpoint_id } => validate_identity(endpoint_id, "source.endpoint_id"),
            Self::Process { pid, .. } if *pid == 0 => {
                Err(PersistenceError::invalid("source.pid", "must be nonzero"))
            }
            Self::Tone { frequency_hz }
                if !frequency_hz.is_finite()
                    || *frequency_hz <= 0.0
                    || *frequency_hz >= 24000.0 =>
            {
                Err(PersistenceError::invalid(
                    "source.frequency_hz",
                    "must be finite and between 0 and 24000 Hz",
                ))
            }
            _ => Ok(()),
        }
    }

    pub fn to_selection(&self) -> InputSelection {
        match self {
            Self::Physical { endpoint_id } => InputSelection::Physical {
                endpoint_id: endpoint_id.clone(),
            },
            Self::Process {
                pid,
                creation_time_100ns,
            } => InputSelection::Process {
                pid: *pid,
                creation_time_100ns: *creation_time_100ns,
            },
            Self::Tone { frequency_hz } => InputSelection::Tone {
                frequency_hz: *frequency_hz,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLimitDocument {
    UntilStopped,
    Seconds(u16),
}

impl From<RunLimit> for RunLimitDocument {
    fn from(value: RunLimit) -> Self {
        match value {
            RunLimit::UntilStopped => Self::UntilStopped,
            RunLimit::Seconds(seconds) => Self::Seconds(seconds),
        }
    }
}

impl From<RunLimitDocument> for RunLimit {
    fn from(value: RunLimitDocument) -> Self {
        match value {
            RunLimitDocument::UntilStopped => Self::UntilStopped,
            RunLimitDocument::Seconds(seconds) => Self::Seconds(seconds),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectLayout {
    pub mixer_order: Vec<u64>,
    pub mixer_width: f32,
    pub mixer_dock_width: f32,
    pub workspace: WorkspaceDocument,
    pub camera_x: f32,
    pub camera_y: f32,
    pub zoom: f32,
    pub nodes: Vec<NodeLayout>,
}

impl Default for ProjectLayout {
    fn default() -> Self {
        Self {
            workspace: WorkspaceDocument::Graph,
            mixer_order: Vec::new(),
            mixer_width: 176.0,
            mixer_dock_width: 360.0,
            camera_x: 0.0,
            camera_y: 0.0,
            zoom: 1.0,
            nodes: Vec::new(),
        }
    }
}

impl ProjectLayout {
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if !self.mixer_width.is_finite()
            || !(136.0..=280.0).contains(&self.mixer_width)
            || !self.mixer_dock_width.is_finite()
            || !(160.0..=640.0).contains(&self.mixer_dock_width)
            || self.mixer_order.iter().collect::<BTreeSet<_>>().len() != self.mixer_order.len()
        {
            return Err(PersistenceError::invalid(
                "layout.mixer",
                "invalid widths or duplicate node IDs",
            ));
        }
        if !self.camera_x.is_finite()
            || !self.camera_y.is_finite()
            || !self.zoom.is_finite()
            || !(f32::EPSILON..=4.0).contains(&self.zoom)
        {
            return Err(PersistenceError::invalid(
                "layout.camera",
                "coordinates must be finite and zoom must be in f32::EPSILON..=4.0",
            ));
        }
        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            if !node.x.is_finite() || !node.y.is_finite() || !ids.insert(node.node_id) {
                return Err(PersistenceError::invalid(
                    "layout.nodes",
                    "positions must be finite with unique node IDs",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceDocument {
    #[default]
    Graph,
    Mixer,
    Matrix,
    Devices,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeLayout {
    /// Document-local node ordinal, remapped to a LogicalGraph identity on load.
    pub node_id: u64,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiDensity {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Dark,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppPreferences {
    pub version: u32,
    pub density: UiDensity,
    pub appearance: Appearance,
    /// Requested for the next audio apply/start, not a measured device result.
    pub latency_profile: AudioLatencyProfile,
    pub audio: AudioSettings,
    pub restore_last_project: bool,
    pub last_project_path: Option<std::path::PathBuf>,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            version: PREFERENCES_VERSION,
            density: UiDensity::Comfortable,
            appearance: Appearance::Dark,
            latency_profile: AudioLatencyProfile::Compatible,
            audio: AudioSettings::default(),
            restore_last_project: false,
            last_project_path: None,
        }
    }
}

impl AppPreferences {
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.version != PREFERENCES_VERSION {
            return Err(PersistenceError::UnsupportedVersion {
                document: "preferences",
                version: self.version,
            });
        }
        self.audio
            .validate()
            .map_err(|error| PersistenceError::invalid("audio", error.to_string()))?;
        if let Some(path) = &self.last_project_path
            && (path.as_os_str().is_empty() || path.to_string_lossy().contains('\0'))
        {
            return Err(PersistenceError::invalid(
                "last_project_path",
                "must be a nonempty path without NUL",
            ));
        }
        Ok(())
    }
}

pub(super) fn validate_identity(value: &str, field: &str) -> Result<(), PersistenceError> {
    if value.is_empty() || value.contains('\0') || value.len() > 32768 {
        return Err(PersistenceError::invalid(
            field,
            "must contain 1 to 32768 bytes without NUL",
        ));
    }
    Ok(())
}

pub(super) fn validate_gain_pan(gain: f64, pan: f64, field: &str) -> Result<(), PersistenceError> {
    if !gain.is_finite()
        || !(0.0..=1.0).contains(&gain)
        || !pan.is_finite()
        || !(-1.0..=1.0).contains(&pan)
    {
        return Err(PersistenceError::invalid(
            field,
            "gain must be in 0..=1 and pan in -1..=1",
        ));
    }
    Ok(())
}
