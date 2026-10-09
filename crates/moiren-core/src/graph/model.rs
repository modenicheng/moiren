use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GraphError {
    #[error("no such node in the graph")]
    NodeNotFound,
    #[error("no such port in the graph or the specified node")]
    PortNotFound,
    #[error("no such edge in the graph")]
    EdgeNotFound,
    #[error("port ID or per-node input index capacity exhausted")]
    TooManyPorts,
    #[error("node ID capacity exhausted")]
    TooManyNodes,
    #[error("edge ID capacity exhausted")]
    TooManyEdges,
    #[error("channel count is zero or unsupported by the node kind")]
    InvalidChannelCount,
    #[error("source and destination channel counts differ")]
    ChannelMismatch,
    #[error("edges must connect an output port to an input port")]
    PortDirectionMismatch,
    #[error("the input port already has an incoming edge")]
    InputAlreadyConnected,
    #[error("the connection would create a cycle")]
    CycleDetected,
    #[error("only Bus input ports can be added or removed")]
    FixedPortLayout,
    #[error("disconnect the input port before removing it")]
    PortConnected,
    #[error("gain must be finite and nonnegative, pan must be finite in [-1, 1]")]
    InvalidSendParameters,
    #[error("node layout or graph object identity violates the model contract")]
    InvalidNodeLayout,
}

#[derive(Debug, PartialEq, PartialOrd, Ord, Eq, Clone, Copy, Hash)]
pub struct NodeId(pub(super) u64);
#[derive(Debug, PartialEq, PartialOrd, Ord, Eq, Clone, Copy, Hash)]
pub struct EdgeId(pub(super) u64);
#[derive(Debug, PartialEq, PartialOrd, Ord, Eq, Clone, Copy, Hash)]
pub struct PortId(pub(super) u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Source,
    Sink,
    Gain,
    /// Linked peak compressor with one input and output of matching width.
    Compressor,
    Bus,
    /// Stereo balance with one stereo input and output.
    Pan,
}

impl NodeKind {
    pub(super) fn validate_channels(self, channels: usize) -> Result<(), GraphError> {
        if channels == 0 || (self == Self::Pan && channels != 2) {
            return Err(GraphError::InvalidChannelCount);
        }
        Ok(())
    }
    pub(super) fn initial_ports(self) -> (usize, usize) {
        match self {
            Self::Source | Self::Bus => (0, 1),
            Self::Sink => (1, 0),
            Self::Gain | Self::Pan | Self::Compressor => (1, 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortRole {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub(super) id: PortId,
    pub(super) channels: usize,
    pub(super) role: PortRole,
}
impl Port {
    pub fn id(&self) -> PortId {
        self.id
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn role(&self) -> PortRole {
        self.role
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub(super) id: NodeId,
    pub(super) kind: NodeKind,
    pub(super) channels: usize,
    pub(super) inputs: Vec<Port>,
    pub(super) outputs: Vec<Port>,
}
impl Node {
    pub fn id(&self) -> NodeId {
        self.id
    }
    pub fn kind(&self) -> NodeKind {
        self.kind
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn inputs(&self) -> &[Port] {
        &self.inputs
    }
    pub fn outputs(&self) -> &[Port] {
        &self.outputs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendTap {
    PreFader,
    #[default]
    PostFader,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SendParams {
    /// Linear amplitude, preserving headroom. No arbitrary upper gain limit.
    pub gain: f64,
    /// -1 = left, 0 = center, 1 = right; interpreted by future route lowering.
    pub pan: f64,
    pub mute: bool,
    pub tap: SendTap,
}
impl Default for SendParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            mute: false,
            tap: SendTap::PostFader,
        }
    }
}
impl SendParams {
    pub fn validate(self) -> Result<(), GraphError> {
        if !self.gain.is_finite()
            || self.gain < 0.0
            || !self.pan.is_finite()
            || !(-1.0..=1.0).contains(&self.pan)
        {
            return Err(GraphError::InvalidSendParameters);
        }
        Ok(())
    }
}

/// One routing edge, including its send parameters. No parallel Send identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub(super) id: EdgeId,
    pub(super) src: NodeId,
    pub(super) dst: NodeId,
    pub(super) src_port: PortId,
    pub(super) dst_port: PortId,
    pub(super) params: SendParams,
}
impl Edge {
    pub fn id(&self) -> EdgeId {
        self.id
    }
    pub fn src(&self) -> NodeId {
        self.src
    }
    pub fn dst(&self) -> NodeId {
        self.dst
    }
    pub fn src_port(&self) -> PortId {
        self.src_port
    }
    pub fn dst_port(&self) -> PortId {
        self.dst_port
    }
    pub fn params(&self) -> &SendParams {
        &self.params
    }
}
