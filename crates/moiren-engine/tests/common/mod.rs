#![allow(dead_code)]
//! Shared integration-test scaffolding for the moiren-engine test binaries.
//!
//! The `compiler` and `runtime` binaries each compile their own copy via
//! `#[path = "../common/mod.rs"] mod common;`, so each binary sees only the
//! subset it uses (hence the module-level `allow(dead_code)`). The `host`
//! binary drives engines through `EngineHost` and keeps its own
//! `tests/host/support.rs`.
//!
//! Embedded `#[cfg(test)]` modules inside `src/` cannot import from here:
//! integration tests see only the compiled library. The src side keeps a
//! parallel `pump` in `src/test_support.rs` and the arena-based
//! `assert_samples_exact` in `src/processor/tests.rs` under the same names.
//!
//! Per-file tuning stays visible at the call site: scenario-specific fields
//! (event budget, epoch, control capacity, horizon) override the defaults
//! below via struct-update syntax instead of accumulating parameters here.

use moiren_core::graph::{EdgeId, LogicalGraph, NodeId, PortId, SendParams};
use moiren_core::protocol::ProcessorId;
use moiren_engine::boundary::{AudioReader, BoundaryReport, RtAudioSource};
use moiren_engine::buffer::{AudioBlockMut, BufferArena, BufferSlotLayout, PortAccess};
use moiren_engine::compiler::CompileConfig;
use moiren_engine::control::{ParamSpec, parameter_channel};
use moiren_engine::processor::{ProcessContext, RtProcessor};
use moiren_engine::runtime::{
    Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources,
};
use moiren_engine::sample::ProcessingSample;
use moiren_signal::{Bus, HostControl, PumpBudget};

/// Default compile configuration for graph-compiled test plans.
///
/// Scenarios override only the fields they exercise, e.g.
/// `CompileConfig { timeline_epoch: 17, ..common::compile_config(2) }`.
pub fn compile_config(revision: u64) -> CompileConfig {
    CompileConfig {
        engine: engine_config(),
        audio_byte_budget: 4096,
        plan_revision: revision,
        timeline_epoch: 3,
        control_capacity: 8,
        control_horizon_frames: 1024,
    }
}

/// Default engine configuration shared by compiled and hand-built test plans.
pub fn engine_config() -> EngineConfig {
    EngineConfig {
        processing_sr: 48_000.0,
        max_block_frames: 8,
        max_events_per_block: 8,
    }
}

/// First output port of `node`.
pub fn output(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().outputs()[0].id()
}

/// First input port of `node`.
pub fn input(graph: &LogicalGraph, node: NodeId) -> PortId {
    graph.get_node(node).unwrap().inputs()[0].id()
}

/// Connects the source's first output to the sink's first input with default
/// send parameters.
pub fn connect(graph: &mut LogicalGraph, source: NodeId, sink: NodeId) -> EdgeId {
    connect_with(graph, source, sink, SendParams::default())
}

/// Connects the source's first output to the sink's first input with explicit
/// send parameters.
pub fn connect_with(
    graph: &mut LogicalGraph,
    source: NodeId,
    sink: NodeId,
    params: SendParams,
) -> EdgeId {
    graph
        .connect(output(graph, source), input(graph, sink), params)
        .unwrap()
}

/// Monophonic boundary source producing a sample ramp `next`, `next + 1`, ...
///
/// Shared by render-order, history-preservation, and protected-IO assertions
/// that must distinguish every rendered frame.
pub struct CountingSource {
    pub next: f64,
}

impl<S: ProcessingSample> RtAudioSource<S> for CountingSource {
    fn channel_count(&self) -> usize {
        1
    }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        for sample in output.channel_mut(0) {
            *sample = S::from_f64(self.next);
            self.next += 1.0;
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}

/// IO wiring shape of a [`test_engine`] plan.
pub enum TestIo {
    /// One slot; the processor writes output port 0 (source role).
    SourceWrite,
    /// Two slots; the processor reads input port 0 and writes output port 0
    /// (separate-mode interior processor).
    SeparateReadWrite,
}

/// Hand-built single-processor engine: the one scaffolding for runtime tests
/// that prepare plans directly (signal hosts, custom parameter domains,
/// separate IO) instead of going through the graph compiler.
///
/// Slot capacity follows `config.max_block_frames`; the arena keeps the
/// shared 4096-byte budget and the parameter channel the shared 1024-frame
/// horizon. `host` selects `prepare_with_signals` over `prepare`.
#[allow(clippy::too_many_arguments)]
pub fn test_engine<P: RtProcessor<f64> + 'static>(
    processor: (ProcessorId, P),
    params: Vec<ParamSpec>,
    revision: u64,
    epoch: u64,
    queue: usize,
    config: EngineConfig,
    io: TestIo,
    host: Option<&HostControl>,
) -> Engine<f64> {
    let (id, processor) = processor;
    let resources = RtResources::new(vec![ProcessorInstance::new(id, processor)]).unwrap();
    let (_, parameters) = parameter_channel(params, revision, epoch, queue, 1024).unwrap();
    let slots = match io {
        TestIo::SourceWrite => vec![BufferSlotLayout {
            channels: 1,
            capacity_frames: config.max_block_frames,
        }],
        TestIo::SeparateReadWrite => vec![
            BufferSlotLayout {
                channels: 1,
                capacity_frames: config.max_block_frames,
            };
            2
        ],
    };
    let arena = BufferArena::new(&slots, 4096).unwrap();
    let access = match io {
        TestIo::SourceWrite => vec![PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }],
        TestIo::SeparateReadWrite => vec![
            PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            },
            PortAccess::Write {
                port: 0,
                slot: arena.slot(1).unwrap(),
            },
        ],
    };
    let io = arena.prepare_io(&access).unwrap();
    let plan = match host {
        Some(host) => ExecutionPlan::prepare_with_signals(
            arena,
            vec![OpSpec { processor: id, io }],
            &resources,
            &parameters,
            config,
            host,
        ),
        None => ExecutionPlan::prepare(
            arena,
            vec![OpSpec { processor: id, io }],
            &resources,
            &parameters,
            config,
        ),
    }
    .unwrap();
    Engine::new(plan, resources, parameters).unwrap()
}

/// One bounded pump pass with the canonical test budget.
pub fn pump(bus: &mut Bus) {
    bus.pump(PumpBudget {
        topic_checks: 1024,
        deliveries: 128,
        copy_bytes: 65_536,
    });
}

/// Reads `expected.len()` interleaved frames and compares each with a 1e-6
/// tolerance — the policy for DSP paths where float evaluation order may
/// legitimately differ by rounding.
pub fn assert_samples_close<S: ProcessingSample>(reader: &mut AudioReader<S>, expected: &[f64]) {
    let mut output = vec![S::ZERO; expected.len()];
    assert_eq!(
        reader
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        expected.len()
    );
    for (actual, expected) in output.iter().zip(expected) {
        assert!(
            (actual.to_f64() - expected).abs() < 1e-6,
            "{} != {expected}",
            actual.to_f64()
        );
    }
}

/// Reads `expected.len()` interleaved frames and requires exact equality —
/// the policy for bit-exact DSP, ordering, and ramp-preservation assertions.
pub fn assert_samples_exact<S: ProcessingSample>(reader: &mut AudioReader<S>, expected: &[f64]) {
    let mut output = vec![S::ZERO; expected.len()];
    assert_eq!(
        reader
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        expected.len()
    );
    for (actual, expected) in output.iter().zip(expected) {
        assert_eq!(actual.to_f64(), *expected);
    }
}
