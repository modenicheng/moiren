//! Editable topology compiled into a runnable Bus/Pan graph. No devices open.
use moiren_core::{
    graph::{edit::connect_to_new_bus_input, *},
    protocol::*,
};
use moiren_engine::{boundary::*, compiler::*, processor::Pan, runtime::EngineConfig};

fn input(graph: &LogicalGraph, id: NodeId) -> PortId {
    graph.get_node(id).expect("example node").inputs()[0].id()
}
fn output(graph: &LogicalGraph, id: NodeId) -> PortId {
    graph.get_node(id).expect("example node").outputs()[0].id()
}

fn main() -> anyhow::Result<()> {
    let mut graph = LogicalGraph::new();
    let a = graph.create_node(NodeKind::Source, 2)?;
    let b = graph.create_node(NodeKind::Source, 2)?;
    let bus = graph.create_node(NodeKind::Bus, 2)?;
    let pan = graph.create_node(NodeKind::Pan, 2)?;
    let sink = graph.create_node(NodeKind::Sink, 2)?;
    let bus_input = graph.add_input_port(bus, 2)?;
    graph.connect(output(&graph, a), bus_input, SendParams::default())?;
    let b_output = output(&graph, b);
    connect_to_new_bus_input(&mut graph, b_output, bus, SendParams::default())?;
    graph.connect(
        output(&graph, bus),
        input(&graph, pan),
        SendParams::default(),
    )?;
    graph.connect(
        output(&graph, pan),
        input(&graph, sink),
        SendParams::default(),
    )?;
    graph.validate()?;
    let order = graph.topological_order()?;
    assert_eq!(order, [a, b, bus, pan, sink]);
    println!(
        "logical nodes: {}, edges: {}, order: {order:?}",
        graph.nodes().len(),
        graph.edges().len()
    );

    // Bind only external IO and initial values. The compiler assigns processors,
    // slots, sends and PreparedIo from the graph, including future topology edits.
    let (output_writer, mut output_reader) = audio_bridge::<f64>(2, 16, 4096)?;
    let mut io = NodeBindings::new();
    io.bind_source(
        a,
        ConstantSource {
            channels: 2,
            value: 0.25,
        },
    )?;
    io.bind_source(
        b,
        ConstantSource {
            channels: 2,
            value: 0.125,
        },
    )?;
    io.bind_pan(pan, -1.0)?;
    io.bind_sink(sink, output_writer)?;
    let CompiledGraph {
        mut engine,
        mut control,
        bindings,
        stats,
    } = compile(
        &graph,
        io,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )?;
    println!("compiled: {stats:?}");
    let accepted = control.submit(
        ParameterRequest {
            request_id: 1,
            plan_revision: 1,
            timeline_epoch: 1,
            target: ParameterKey {
                processor: bindings.node(pan).expect("compiled pan binding"),
                parameter: Pan::POSITION,
            },
            at: ApplyAt::Frame(2),
            value: ParamValue::Float(1.0),
            ramp_frames: 4,
        },
        engine.timeline(),
    );
    assert_eq!(accepted.code, ReplyCode::Accepted);
    for frames in [3, 1, 4] {
        println!("render: {:?}", engine.render(frames)?);
    }
    let mut samples = [0.0; 16];
    assert_eq!(
        output_reader
            .read_interleaved(&mut samples)?
            .transferred_frames,
        8
    );
    assert_eq!(
        samples,
        [
            0.375, 0.0, 0.375, 0.0, 0.375, 0.1875, 0.375, 0.375, 0.1875, 0.375, 0.0, 0.375, 0.0,
            0.375, 0.0, 0.375
        ]
    );
    println!("stereo output: {samples:?}");
    println!("applied: {:?}", control.poll_applied());
    drop(engine.into_parts());
    Ok(())
}
