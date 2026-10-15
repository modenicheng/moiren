use moiren_core::graph::{GraphError, LogicalGraph, NodeId, NodeKind, SendParams};
use moiren_engine::{
    boundary::{AudioReader, RtAudioSource, audio_bridge},
    compiler::{CompileConfig, CompiledGraph, NodeBindings, compile},
    runtime::EngineConfig,
};

pub(crate) struct SessionGraph {
    pub graph: LogicalGraph,
    pub input: NodeId,
    pub gain: NodeId,
    pub pan: NodeId,
    pub output: NodeId,
}

pub(crate) struct PreparedSessionGraph {
    pub compiled: CompiledGraph<f32>,
    pub output: AudioReader<f32>,
    pub controls: Option<(NodeId, NodeId)>,
}

/// Device ownership is prepared elsewhere. This function consumes the exact
/// service candidate and binds every DSP node on a non-RT preparation worker.
pub(crate) fn prepare_routing(
    routing: &crate::service::GraphPreparation,
    source: impl RtAudioSource<f32> + 'static,
    max_block_frames: usize,
    signals: Option<&mut crate::service::SignalPreparation>,
) -> Result<PreparedSessionGraph, crate::monitor::MonitorError> {
    use crate::monitor::MonitorError;
    let (input, sink) = routing
        .io_nodes()
        .map_err(|_| MonitorError::InvalidConfig)?;
    if source.channel_count() != 2 {
        return Err(MonitorError::InvalidConfig);
    }
    let (writer, output) = audio_bridge(2, max_block_frames, 8 * 1024 * 1024)?;
    let mut bindings = NodeBindings::new();
    bindings.bind_source(input, source)?;
    bindings.bind_sink(sink, writer)?;
    routing.bind_processors(&mut bindings)?;
    let config = CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames,
            max_events_per_block: 64,
        },
        audio_byte_budget: 8 * 1024 * 1024,
        plan_revision: routing.revision,
        timeline_epoch: crate::timeline::next_epoch()
            .ok_or(MonitorError::TimelineEpochExhausted)?,
        control_capacity: 64,
        control_horizon_frames: 48_000,
    };
    let compiled = if let Some(signals) = signals {
        signals.compile(
            &routing.graph,
            bindings,
            config,
            routing.graph.get_node(input)?.outputs()[0].id(),
            routing.output_meter_port(sink)?,
        )?
    } else {
        compile(&routing.graph, bindings, config)?
    };
    Ok(PreparedSessionGraph {
        compiled,
        output,
        controls: routing.gain_pan_nodes(),
    })
}

impl SessionGraph {
    pub fn new() -> Result<Self, GraphError> {
        let mut graph = LogicalGraph::new();
        let input = graph.create_node(NodeKind::Source, 2)?;
        let gain = graph.create_node(NodeKind::Gain, 2)?;
        let pan = graph.create_node(NodeKind::Pan, 2)?;
        let output = graph.create_node(NodeKind::Sink, 2)?;
        for (source, destination) in [(input, gain), (gain, pan), (pan, output)] {
            graph.connect(
                graph.get_node(source)?.outputs()[0].id(),
                graph.get_node(destination)?.inputs()[0].id(),
                SendParams::default(),
            )?;
        }
        Ok(Self {
            graph,
            input,
            gain,
            pan,
            output,
        })
    }
}
