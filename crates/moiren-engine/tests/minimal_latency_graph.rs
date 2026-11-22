//! Portable graph transfer regression, not a device or physical latency probe.
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams, edit::connect_to_new_bus_input};
use moiren_engine::{
    boundary::{BoundaryReport, audio_bridge},
    compiler::{CompileConfig, NodeBindings, compile},
    node::{
        BoundarySnapshot, DeviceOptions, InputConfig, InputNode, InputSource, OutputConfig,
        OutputNode, OutputTarget,
    },
    runtime::EngineConfig,
    sample::ProcessingSample,
};

#[path = "runtime/allocation.rs"]
mod allocation;

const CHANNELS: usize = 2;
const EPOCH: u64 = 17;
const BYTE_BUDGET: usize = 65_536;

#[derive(Clone, Copy)]
enum Signal {
    Impulses,
    Marker,
}

fn samples<S: ProcessingSample>(frames: usize, max_block_frames: usize, signal: Signal) -> Vec<S> {
    (0..frames)
        .flat_map(|frame| {
            (0..CHANNELS).map(move |channel| {
                let value = match signal {
                    Signal::Impulses => {
                        if [
                            0,
                            max_block_frames - 1,
                            max_block_frames,
                            2 * max_block_frames + 1,
                            frames - 1,
                        ]
                        .contains(&frame)
                        {
                            [0.75, -0.5][channel]
                        } else {
                            0.0
                        }
                    }
                    Signal::Marker => {
                        // Distinct frame/channel values, exactly representable
                        // at both precisions; silence cannot pass as a marker.
                        let code = ((frame * CHANNELS + channel) * 73 + 19) % 65_521 + 1;
                        let polarity = if channel == 0 { 1.0 } else { -1.0 };
                        polarity * code as f64 / 131_072.0
                    }
                };
                S::from_f64(value)
            })
        })
        .collect()
}

fn complete_transfer(frames: usize) -> BoundaryReport {
    BoundaryReport {
        transferred_frames: frames,
        ..BoundaryReport::default()
    }
}

fn assert_boundary(snapshot: Option<BoundarySnapshot>, start: usize, end: usize) {
    // Compare the whole record on every segment, including counters that could
    // otherwise hide short reads, recovery, timeline gaps or telemetry loss.
    assert_eq!(
        snapshot,
        Some(BoundarySnapshot {
            timeline_epoch: EPOCH,
            start_frame: start as u64,
            end_frame: end as u64,
            report: complete_transfer(end - start),
            total_transferred_frames: end as u64,
            ..BoundarySnapshot::default()
        })
    );
}

fn exercise<S: ProcessingSample>(max_block_frames: usize, demands: &[usize], signal: Signal) {
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, CHANNELS).unwrap();
    let bus = graph.create_node(NodeKind::Bus, CHANNELS).unwrap();
    let output = graph.create_node(NodeKind::Sink, CHANNELS).unwrap();
    let input_port = graph.get_node(input).unwrap().outputs()[0].id();
    connect_to_new_bus_input(&mut graph, input_port, bus, SendParams::default()).unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(output).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();

    // Capacities are deliberately unrelated to a batching target. Prepare all
    // queues, samples, telemetry and plan storage before entering the RT guard.
    let input_capacity = 3 * max_block_frames + 19;
    let (mut producer, source) = audio_bridge::<S>(CHANNELS, input_capacity, BYTE_BUDGET).unwrap();
    let (sink, mut consumer) =
        audio_bridge::<S>(CHANNELS, max_block_frames + 11, BYTE_BUDGET).unwrap();
    let (input_node, mut input_reports) = InputNode::new(
        InputConfig {
            source: InputSource::Software {
                name: "minimal latency input".into(),
            },
            channels: CHANNELS,
            device: DeviceOptions::default(),
        },
        source,
        1,
    )
    .unwrap();
    let (output_node, mut output_reports) = OutputNode::new(
        OutputConfig {
            target: OutputTarget::Software {
                name: "minimal latency output".into(),
            },
            channels: CHANNELS,
            device: DeviceOptions::default(),
        },
        sink,
        1,
    )
    .unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_io(input, input_node).unwrap();
    bindings.bind_io(output, output_node).unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames,
                max_events_per_block: 4,
            },
            audio_byte_budget: BYTE_BUDGET,
            plan_revision: 5,
            timeline_epoch: EPOCH,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )
    .unwrap();
    assert_eq!(compiled.stats.node_count, 3);
    assert_eq!(compiled.stats.edge_count, 2);
    assert_eq!(compiled.engine.timeline(), 0);

    let total_frames = demands.iter().sum();
    let expected = samples::<S>(total_frames, max_block_frames, signal);
    let poison = S::from_f64(123.0);
    let mut received = vec![poison; max_block_frames * CHANNELS];
    let mut cursor = 0;
    for &demand in demands {
        assert!(demand > 0 && demand <= input_capacity);
        let demand_end = cursor + demand;
        let (written, heap) = allocation::track_heap(|| {
            producer.write_interleaved(&expected[cursor * CHANNELS..demand_end * CHANNELS])
        });
        assert_eq!(heap, (0, 0, 0));
        assert_eq!(written.unwrap(), complete_transfer(demand));
        while cursor < demand_end {
            // The sole execution owner services oversized device demand by
            // rendering its bounded pieces immediately, without a priming call.
            let frames = (demand_end - cursor).min(max_block_frames);
            let end = cursor + frames;
            assert_eq!(consumer.available_frames(), 0);
            received.fill(poison);
            let ((render, available, read, input_snapshot, output_snapshot), heap) =
                allocation::track_heap(|| {
                    let render = compiled.engine.render(frames);
                    let available = consumer.available_frames();
                    let read = consumer.read_interleaved(&mut received[..frames * CHANNELS]);
                    (
                        render,
                        available,
                        read,
                        input_reports.latest(),
                        output_reports.latest(),
                    )
                });
            assert_eq!(heap, (0, 0, 0), "render/bridges must not touch the heap");
            let render = render.unwrap();
            assert_eq!((render.start, render.end), (cursor as u64, end as u64));
            assert_eq!(render.segments, 1);
            assert_eq!(render.applied_events, 0);
            assert_eq!(compiled.engine.timeline(), end as u64);
            assert_eq!(available, frames, "complete output must exist in this call");
            assert_eq!(read.unwrap(), complete_transfer(frames));
            assert_boundary(input_snapshot, cursor, end);
            assert_boundary(output_snapshot, cursor, end);
            for (offset, (&actual, &wanted)) in received[..frames * CHANNELS]
                .iter()
                .zip(&expected[cursor * CHANNELS..end * CHANNELS])
                .enumerate()
            {
                assert_eq!(
                    actual.to_f64(),
                    wanted.to_f64(),
                    "same-call sample mismatch: max_block_frames={max_block_frames}, \
                     precision={}, frame={}, channel={}",
                    std::any::type_name::<S>(),
                    cursor + offset / CHANNELS,
                    offset % CHANNELS,
                );
            }
            assert!(
                received[frames * CHANNELS..]
                    .iter()
                    .all(|&sample| sample.to_f64() == poison.to_f64()),
                "a short segment must not overwrite the unused tail"
            );
            assert_eq!(consumer.available_frames(), 0);
            assert_eq!(
                producer.available_frames(),
                input_capacity - (demand_end - end),
                "exactly this segment's input must be consumed"
            );
            cursor = end;
        }
    }
    assert_eq!(cursor, total_frames);
    assert_eq!(compiled.engine.timeline(), total_frames as u64);
    assert_eq!(producer.available_frames(), input_capacity);
    assert_eq!(consumer.available_frames(), 0);
    assert!(input_reports.latest().is_none());
    assert!(output_reports.latest().is_none());
    // Engine, bounded queues and compiled ownership are reclaimed off RT.
}

#[test]
fn impulses_cross_the_compiled_bus_in_the_same_call_at_64_128_256_frames() {
    for max_block_frames in [64, 128, 256] {
        let demands = [max_block_frames; 4];
        exercise::<f32>(max_block_frames, &demands, Signal::Impulses);
        exercise::<f64>(max_block_frames, &demands, Signal::Impulses);
    }
}

#[test]
fn marker_has_no_graph_block_delay_under_variable_segmented_demand() {
    for max_block_frames in [64, 128, 256] {
        let demands = [
            1,
            7,
            31,
            max_block_frames - 1,
            max_block_frames,
            max_block_frames + 1,
            2 * max_block_frames + 17,
            3,
            max_block_frames / 2 + 5,
            3 * max_block_frames + 11,
            1,
        ];
        exercise::<f32>(max_block_frames, &demands, Signal::Marker);
        exercise::<f64>(max_block_frames, &demands, Signal::Marker);
    }
}
