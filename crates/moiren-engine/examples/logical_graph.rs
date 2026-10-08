//! Editable topology plus a matching manually prepared execution plan.
//! This demonstrates Bus/Pan, not a general Graph Compiler. No devices open.
use moiren_core::{
    graph::{edit::connect_to_new_bus_input, *},
    protocol::*,
};
use moiren_engine::{boundary::*, buffer::*, control::*, processor::*, runtime::*};

const SOURCE_A: ProcessorId = ProcessorId(1);
const SOURCE_B: ProcessorId = ProcessorId(2);
const BUS: ProcessorId = ProcessorId(3);
const PAN: ProcessorId = ProcessorId(4);
const SINK: ProcessorId = ProcessorId(5);

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

    // Bind this known topology explicitly. Logical IDs and ProcessorIds are
    // distinct; a future compiler will produce this mapping and buffer plan.
    let (output_writer, mut output_reader) = audio_bridge::<f64>(2, 16, 4096)?;
    let resources = RtResources::new(vec![
        ProcessorInstance::new(
            SOURCE_A,
            SourceAdapter(ConstantSource {
                channels: 2,
                value: 0.25,
            }),
        ),
        ProcessorInstance::new(
            SOURCE_B,
            SourceAdapter(ConstantSource {
                channels: 2,
                value: 0.125,
            }),
        ),
        ProcessorInstance::new(BUS, Bus),
        ProcessorInstance::new(PAN, Pan),
        ProcessorInstance::new(SINK, SinkAdapter(output_writer)),
    ])?;
    let (mut control, parameters) =
        parameter_channel(vec![Pan::parameter(PAN, -1.0)], 1, 1, 8, 48000)?;
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 8,
        }; 3],
        4096,
    )?;
    let a_slot = arena.slot(0).expect("source a slot");
    let b_slot = arena.slot(1).expect("source b slot");
    let bus_slot = arena.slot(2).expect("bus slot");
    let specs = vec![
        OpSpec {
            processor: SOURCE_A,
            io: arena.prepare_io(&[PortAccess::Write {
                port: 0,
                slot: a_slot,
            }])?,
        },
        OpSpec {
            processor: SOURCE_B,
            io: arena.prepare_io(&[PortAccess::Write {
                port: 0,
                slot: b_slot,
            }])?,
        },
        OpSpec {
            processor: BUS,
            io: arena.prepare_io(&[
                PortAccess::Read {
                    port: 0,
                    slot: a_slot,
                },
                PortAccess::Read {
                    port: 1,
                    slot: b_slot,
                },
                PortAccess::Write {
                    port: 0,
                    slot: bus_slot,
                },
            ])?,
        },
        OpSpec {
            processor: PAN,
            io: arena.prepare_io(&[PortAccess::InPlace {
                input: 0,
                output: 0,
                slot: bus_slot,
            }])?,
        },
        OpSpec {
            processor: SINK,
            io: arena.prepare_io(&[PortAccess::Read {
                port: 0,
                slot: bus_slot,
            }])?,
        },
    ];
    let plan = ExecutionPlan::prepare(
        arena,
        specs,
        &resources,
        &parameters,
        EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 8,
        },
    )?;
    let mut engine = Engine::new(plan, resources, parameters)?;
    let accepted = control.submit(
        ParameterRequest {
            request_id: 1,
            plan_revision: 1,
            timeline_epoch: 1,
            target: ParameterKey {
                processor: PAN,
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
