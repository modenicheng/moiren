use moiren_core::{graph::*, protocol::*};
use moiren_engine::{
    boundary::*, compiler::*, control::ControlError, processor::*, runtime::*,
    sample::ProcessingSample,
};

#[path = "../runtime/allocation.rs"]
mod allocation;
mod compressor;
mod reference;
mod rendering;
mod sends;
mod validation;

fn config() -> CompileConfig {
    CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 16,
        },
        audio_byte_budget: 65_536,
        plan_revision: 7,
        timeline_epoch: 3,
        control_capacity: 16,
        control_horizon_frames: 48_000,
    }
}

fn output(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().outputs()[0].id()
}
fn input(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().inputs()[0].id()
}
fn connect(graph: &mut LogicalGraph, a: NodeId, b: NodeId, params: SendParams) -> EdgeId {
    graph
        .connect(output(graph, a), input(graph, b), params)
        .unwrap()
}
fn constant<S: ProcessingSample>(bindings: &mut NodeBindings<S>, node: NodeId, channels: usize) {
    bindings
        .bind_source(
            node,
            ConstantSource {
                channels,
                value: 0.375,
            },
        )
        .unwrap();
}
fn assert_samples<S: ProcessingSample>(actual: &[S], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual.to_f64() - expected).abs() < 1e-6,
            "{} != {expected}",
            actual.to_f64()
        );
    }
}
