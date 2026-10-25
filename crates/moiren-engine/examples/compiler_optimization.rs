//! Compare reference/optimized CPU work, excluding compilation and audio IO.
//! Run with `cargo run --release -p moiren-engine --example compiler_optimization`.
//! Append `-- --ramps` to measure active gain/pan automation.
use moiren_core::{
    graph::{LogicalGraph, NodeKind, SendParams},
    protocol::*,
};
use moiren_engine::{
    boundary::{BoundaryReport, ConstantSource, RtAudioSink},
    buffer::AudioBlock,
    compiler::{CompileConfig, CompiledGraph, NodeBindings, compile, compile_reference},
    processor::ProcessContext,
    runtime::EngineConfig,
};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

struct Discard;
impl RtAudioSink<f64> for Discard {
    fn channel_count(&self) -> usize {
        2
    }
    fn write(&mut self, ctx: &ProcessContext, input: AudioBlock<'_, f64>) -> BoundaryReport {
        for channel in input.channels() {
            black_box(channel);
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}

fn prepared(
    inputs: usize,
    frames: usize,
    optimized: bool,
    ramps: bool,
) -> anyhow::Result<CompiledGraph<f64>> {
    let mut graph = LogicalGraph::new();
    let bus = graph.create_node(NodeKind::Bus, 2)?;
    let sink = graph.create_node(NodeKind::Sink, 2)?;
    let mut bindings = NodeBindings::new();
    let mut sends = Vec::new();
    for index in 0..inputs {
        let source = graph.create_node(NodeKind::Source, 2)?;
        let input = graph.add_input_port(bus, 2)?;
        let output = graph.get_node(source)?.outputs()[0].id();
        sends.push(graph.connect(
            output,
            input,
            SendParams {
                gain: 0.3 + index as f64 * 0.01,
                pan: 0.2,
                ..SendParams::default()
            },
        )?);
        bindings.bind_source(
            source,
            ConstantSource {
                channels: 2,
                value: 0.1,
            },
        )?;
    }
    graph.connect(
        graph.get_node(bus)?.outputs()[0].id(),
        graph.get_node(sink)?.inputs()[0].id(),
        SendParams::default(),
    )?;
    bindings.bind_sink(sink, Discard)?;
    let config = CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: frames,
            max_events_per_block: 128,
        },
        audio_byte_budget: 2 * 1024 * 1024,
        plan_revision: 1,
        timeline_epoch: 1,
        control_capacity: 128,
        control_horizon_frames: 48_000,
    };
    let mut compiled = if optimized {
        compile(&graph, bindings, config)
    } else {
        compile_reference(&graph, bindings, config)
    }?;
    if ramps {
        for (index, edge) in sends.into_iter().enumerate() {
            let keys = compiled.bindings.edge(edge).unwrap();
            for (offset, target, value) in [(0, keys.gain, 0.8), (1, keys.pan, -0.3)] {
                let reply = compiled.control.submit(
                    ParameterRequest {
                        request_id: (index * 2 + offset) as u64,
                        plan_revision: 1,
                        timeline_epoch: 1,
                        target,
                        at: ApplyAt::NextBlock,
                        value: ParamValue::Float(value),
                        ramp_frames: u32::MAX,
                    },
                    0,
                );
                anyhow::ensure!(reply.code == ReplyCode::Accepted, "ramp was rejected");
            }
        }
    }
    Ok(compiled)
}

fn measure(compiled: &mut CompiledGraph<f64>, frames: usize) -> anyhow::Result<Duration> {
    let start = Instant::now();
    for _ in 0..1000 {
        black_box(compiled.engine.render(black_box(frames))?);
    }
    Ok(start.elapsed() / 1000)
}

fn main() -> anyhow::Result<()> {
    let ramps = std::env::args().any(|arg| arg == "--ramps");
    let mode = if ramps {
        "active ramps"
    } else {
        "constant parameters"
    };
    println!(
        "f64 stereo, {mode}; median of 5 x 1000 renders; no device/bridge latency measurement"
    );
    println!("inputs frames  ops ref/opt  slots ref/opt  ns/render ref/opt  speedup");
    for inputs in [1, 8, 32] {
        for frames in [16, 64, 256] {
            let mut reference = prepared(inputs, frames, false, ramps)?;
            let mut optimized = prepared(inputs, frames, true, ramps)?;
            for _ in 0..64 {
                reference.engine.render(frames)?;
                optimized.engine.render(frames)?;
            }
            let mut reference_times = Vec::new();
            let mut optimized_times = Vec::new();
            for round in 0..5 {
                if round % 2 == 0 {
                    reference_times.push(measure(&mut reference, frames)?);
                    optimized_times.push(measure(&mut optimized, frames)?);
                } else {
                    optimized_times.push(measure(&mut optimized, frames)?);
                    reference_times.push(measure(&mut reference, frames)?);
                }
            }
            reference_times.sort_unstable();
            optimized_times.sort_unstable();
            let ref_ns = reference_times[2].as_nanos();
            let opt_ns = optimized_times[2].as_nanos();
            println!(
                "{inputs:6} {frames:6}  {:3}/{:3}      {:3}/{:3}       {ref_ns:7}/{opt_ns:7}   {:.2}x",
                reference.stats.operation_count,
                optimized.stats.operation_count,
                reference.stats.slot_count,
                optimized.stats.slot_count,
                ref_ns as f64 / opt_ns as f64
            );
        }
    }
    Ok(())
}
