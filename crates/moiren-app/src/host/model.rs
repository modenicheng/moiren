use super::*;
use moiren_engine::{
    boundary::BridgeError,
    compiler::CompileError,
    runtime::{PlanSwapError, RuntimeError},
};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

#[derive(Debug, Clone, Copy)]
pub struct HostConfig {
    /// The host's channel strips use stereo balance Pan.
    pub channels: usize,
    pub processing_sr: f64,
    pub max_block_frames: usize,
    pub audio_byte_budget: usize,
    pub control_capacity: usize,
    pub control_horizon_frames: u64,
}
impl Default for HostConfig {
    fn default() -> Self {
        Self {
            channels: 2,
            processing_sr: 48_000.0,
            max_block_frames: 256,
            audio_byte_budget: 8 * 1024 * 1024,
            control_capacity: 64,
            control_horizon_frames: 480_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u64);

#[derive(Debug, Clone, Copy)]
pub struct SourceSettings {
    pub gain: f64,
    pub pan: f64,
    pub available: bool,
}
impl Default for SourceSettings {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            available: true,
        }
    }
}

/// Independent backend availability, deliberately separate from user gain/mute.
/// A stopped or failed capture can silence just its source from any owner.
#[derive(Debug, Clone)]
pub struct SourceGate(Arc<AtomicBool>);
impl Default for SourceGate {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }
}
impl SourceGate {
    pub fn set_available(&self, available: bool) {
        self.0.store(available, Ordering::Release);
    }
    pub fn is_available(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("host requires stereo, a positive sample rate, block size and control capacity")]
    InvalidConfig,
    #[error("a candidate is pending; poll its completion before another graph edit")]
    Busy,
    #[error("source channel count differs from the host")]
    ChannelMismatch,
    #[error("source gain must be finite in [0, 16] and pan finite in [-1, 1]")]
    InvalidSourceSettings,
    #[error("unknown source {0:?}")]
    UnknownSource(SourceId),
    #[error("source {0:?} is not active")]
    SourceNotActive(SourceId),
    #[error("host identity counter overflowed")]
    IdOverflow,
    #[error("graph edit would remove or strand host-owned IO or strip nodes")]
    ProtectedInfrastructure,
    #[error("graph uses unsupported prepared IO bindings or send semantics")]
    UnsupportedGraph,
    #[error("invalid compressor settings")]
    InvalidCompressor,
    #[error("there are no unpublished graph edits")]
    NoChanges,
    #[error("output must contain complete stereo frames")]
    InvalidSamples,
    #[error("output bridge did not transfer a full block")]
    IncompleteTransfer,
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Swap(#[from] PlanSwapError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

#[derive(Debug, Clone)]
pub struct SourceSnapshot {
    pub id: SourceId,
    pub generation: u64,
    pub node: NodeId,
    pub gain_node: NodeId,
    pub pan_node: NodeId,
    pub compressor_node: Option<NodeId>,
    pub available: bool,
    /// Latest accepted control target (or initial setting), not RT readback.
    /// Inspect terminal Parameter events to distinguish Applied from StaleRevision.
    pub gain: f64,
    /// Latest accepted stereo balance target; see `gain` for receipt semantics.
    pub pan: f64,
}
#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    pub desired: LogicalGraph,
    /// Confirmed at the last poll; runtime_snapshot may observe a newer revision.
    pub active: LogicalGraph,
    pub sources: Vec<SourceSnapshot>,
    pub bus: NodeId,
    pub sink: NodeId,
    pub plan: PlanInfo,
}
#[derive(Debug, Clone, Copy)]
pub struct PlanInfo {
    pub active_revision: u64,
    pub pending_revision: Option<u64>,
    pub stats: CompileStats,
}
#[derive(Debug, Clone, Copy)]
pub struct RuntimeSnapshot {
    pub timeline: u64,
    pub peak: f32,
    pub rendered_blocks: u64,
    pub active_revision: u64,
    pub desired_revision: u64,
    pub pending: bool,
    pub dirty: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEvent {
    Parameter(ControlReply),
    PlanApplied {
        revision: u64,
        frame: u64,
    },
    PlanRejected {
        revision: u64,
        reason: PlanSwapError,
    },
}

#[derive(Default)]
pub(super) struct Telemetry {
    pub timeline: AtomicU64,
    pub peak: AtomicU32,
    pub blocks: AtomicU64,
}
#[derive(Clone)]
pub(super) struct SourceState {
    pub id: SourceId,
    pub node: NodeId,
    pub gain: NodeId,
    pub pan: NodeId,
    pub bus_port: PortId,
    pub generation: u64,
    pub gate: SourceGate,
    pub level: f64,
    pub position: f64,
    pub compressor: Option<NodeId>,
}
impl SourceState {
    pub fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot {
            id: self.id,
            generation: self.generation,
            node: self.node,
            gain_node: self.gain,
            pan_node: self.pan,
            compressor_node: self.compressor,
            available: self.gate.is_available(),
            gain: self.level,
            pan: self.position,
        }
    }
}
#[derive(Clone)]
pub(super) struct State {
    pub graph: LogicalGraph,
    pub bus: NodeId,
    pub sink: NodeId,
    pub sources: BTreeMap<SourceId, SourceState>,
    pub compressors: BTreeMap<NodeId, CompressorSettings>,
}
pub(super) struct Active {
    pub state: State,
    pub control: ControlPort,
    pub bindings: CompiledBindings,
    pub snapshot: PlanSnapshot,
    pub stats: CompileStats,
}
pub(super) struct Pending {
    pub active: Active,
    // The candidate sink is only a placeholder. Keep its consumer alive until
    // that writer has returned in the retired package; drop both on control.
    pub _placeholder_reader: AudioReader<f32>,
}
