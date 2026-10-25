use moiren_core::{graph::*, protocol::*};
use moiren_engine::{
    boundary::*, compiler::*, control::ControlError, processor::*, runtime::*,
    sample::ProcessingSample,
};

#[path = "../runtime/allocation.rs"]
mod allocation;
#[path = "../common/mod.rs"]
mod common;
mod compressor;
mod nodes;
mod observers;
mod optimization;
mod optimized_mix;
mod rack;
mod rack_initial;
mod reference;
mod rendering;
mod sends;
mod signal_measurement;
mod validation;

// First-port graph helpers and the canonical pump budget live in `common`;
// `connect` keeps its historical parameter-taking name for the submodules.
use common::{connect_with as connect, input, output, pump};

fn config() -> CompileConfig {
    CompileConfig {
        engine: EngineConfig {
            max_events_per_block: 16,
            ..common::engine_config()
        },
        audio_byte_budget: 65_536,
        control_capacity: 16,
        control_horizon_frames: 48_000,
        ..common::compile_config(7)
    }
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

/// Slice comparison with the shared 1e-6 tolerance policy; the reader-based
/// twin lives in `common::assert_samples_close` and the exact arena-based
/// twin in `src/processor/tests.rs::assert_samples_exact`.
fn assert_samples_close<S: ProcessingSample>(actual: &[S], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual.to_f64() - expected).abs() < 1e-6,
            "{} != {expected}",
            actual.to_f64()
        );
    }
}
