//! Exact production physical topology; preparation stays off the audio thread.
use super::Options;
use anyhow::{Context, Result};
use moiren_core::graph::{
    LogicalGraph, NodeId, NodeKind, SendParams, edit::connect_to_new_bus_input,
};
use moiren_engine::{
    boundary::ClockRole,
    host::{EngineProject, NodeConfig},
    node::{DeviceOptions, EndpointSelector, InputConfig, InputSource, OutputConfig, OutputTarget},
};

pub(super) struct PhysicalProject {
    pub project: EngineProject,
    pub input: NodeId,
    pub output: NodeId,
}

pub(super) fn prepare(options: &Options) -> Result<PhysicalProject> {
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2)?;
    let bus = graph.create_node(NodeKind::Bus, 2)?;
    let output = graph.create_node(NodeKind::Sink, 2)?;
    let source_port = graph.get_node(input)?.outputs()[0].id();
    let bus_port = graph.get_node(bus)?.outputs()[0].id();
    let sink_port = graph.get_node(output)?.inputs()[0].id();
    connect_to_new_bus_input(&mut graph, source_port, bus, SendParams::default())?;
    graph.connect(bus_port, sink_port, SendParams::default())?;
    let mut project = EngineProject {
        graph,
        ..Default::default()
    };
    project.processing.engine.max_block_frames = options.engine_block_frames;
    project.nodes.insert(
        input,
        NodeConfig::Input(InputConfig {
            source: InputSource::Physical(EndpointSelector::PinnedEndpoint {
                endpoint_id: options.input_endpoint_id.clone(),
                stable_id: None,
            }),
            channels: 2,
            device: options.capture.apply(DeviceOptions {
                clock_role: ClockRole::Follower,
                ..Default::default()
            }),
        }),
    );
    project.nodes.insert(bus, NodeConfig::Bus);
    project.nodes.insert(
        output,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
                endpoint_id: options.output_endpoint_id.clone(),
                stable_id: None,
            }),
            channels: 2,
            device: options.render.apply(DeviceOptions {
                clock_role: ClockRole::Master,
                ..Default::default()
            }),
        }),
    );
    project
        .validate()
        .context("validate physical Input -> Bus -> Output")?;
    Ok(PhysicalProject {
        project,
        input,
        output,
    })
}
