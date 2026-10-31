//! Control-only preparation facts, never an active-plan or PCM witness.
use super::{NodeId, serialize_node};
#[cfg(any(windows, test))]
use moiren_core::graph::{NodeKind, SendParams};
use moiren_engine::node::VirtualEndpointId;
#[cfg(any(windows, test))]
use moiren_engine::{
    host::{EngineProject, NodeConfig},
    node::{InputSource, OutputTarget},
    processor::StripConfig,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedSrcRoute {
    pub schema_version: u32,
    pub prepared_graph_revision: u64,
    #[serde(serialize_with = "serialize_node")]
    pub source: NodeId,
    #[serde(serialize_with = "serialize_node")]
    pub bus: NodeId,
    #[serde(serialize_with = "serialize_node")]
    pub sink: NodeId,
    pub source_endpoint: VirtualEndpointId,
    pub sink_endpoint: VirtualEndpointId,
    pub channels: usize,
    pub engine_sample_rate: u32,
    pub engine_block_frames: usize,
}

/// Initial project only. Even `invalidated=false` does not establish current
/// parameter values, activation, a plan revision at a callback or clean PCM.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NativeSrcRouteObservation {
    pub prepared: Option<PreparedSrcRoute>,
    /// Sticky once compatible-update preparation is seen for this owner.
    /// Failed/cancelled candidates may invalidate unnecessarily. The original
    /// facts remain available; no candidate is promoted to active here.
    pub invalidated: bool,
}

#[derive(Default)]
pub(crate) struct RouteState {
    pub prepared: Option<PreparedSrcRoute>,
    pub invalidated: AtomicBool,
}
impl RouteState {
    pub fn snapshot(&self) -> NativeSrcRouteObservation {
        NativeSrcRouteObservation {
            prepared: self.prepared.clone(),
            invalidated: self.invalidated.load(Ordering::Acquire),
        }
    }
}

/// Deliberately narrow: do not infer a route through mixes, side branches,
/// unused ports, nonidentity strips or desired controls. Missing metadata does
/// not reject otherwise usable projects. This runs before any audio ownership.
#[cfg(any(windows, test))]
pub(crate) fn prepared_route(project: &EngineProject, revision: u64) -> Option<PreparedSrcRoute> {
    let rate = project.processing.engine.processing_sr;
    if !moiren_engine::audio_config::SAMPLE_RATES
        .iter()
        .any(|&supported| f64::from(supported) == rate)
        || !(1..=4096).contains(&project.processing.engine.max_block_frames)
        || project.graph.nodes().len() != 3
        || project.graph.edges().len() != 2
        || !project.parameters.is_empty()
        || project
            .strips
            .values()
            .any(|strip| *strip != StripConfig::default())
        || project.validate().is_err()
    {
        return None;
    }
    let find = |kind| {
        project
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == kind)
    };
    let (source, bus, sink) = (
        find(NodeKind::Source)?,
        find(NodeKind::Bus)?,
        find(NodeKind::Sink)?,
    );
    let (NodeConfig::Input(input), NodeConfig::Bus, NodeConfig::Output(output)) = (
        project.nodes.get(&source.id())?,
        project.nodes.get(&bus.id())?,
        project.nodes.get(&sink.id())?,
    ) else {
        return None;
    };
    let (
        InputSource::VirtualPlayback {
            endpoint_id: source_endpoint,
        },
        OutputTarget::VirtualMicrophone {
            endpoint_id: sink_endpoint,
        },
    ) = (&input.source, &output.target)
    else {
        return None;
    };
    let channels = source.channels();
    if !(1..=2).contains(&channels)
        || bus.channels() != channels
        || sink.channels() != channels
        || bus.inputs().len() != 1
        || source_endpoint == sink_endpoint
        || project
            .graph
            .edges()
            .iter()
            .any(|edge| *edge.params() != SendParams::default())
    {
        return None;
    }
    let edge_exists = |src: NodeId, dst: NodeId| {
        project
            .graph
            .edges()
            .iter()
            .any(|edge| edge.src() == src && edge.dst() == dst)
    };
    if !edge_exists(source.id(), bus.id()) || !edge_exists(bus.id(), sink.id()) {
        return None;
    }
    Some(PreparedSrcRoute {
        schema_version: 1,
        prepared_graph_revision: revision,
        source: source.id(),
        bus: bus.id(),
        sink: sink.id(),
        source_endpoint: *source_endpoint,
        sink_endpoint: *sink_endpoint,
        channels,
        engine_sample_rate: project.processing.engine.processing_sr as u32,
        engine_block_frames: project.processing.engine.max_block_frames,
    })
}
