//! Compiler-prepared test signal; no Windows types or device access.
use moiren_core::graph::*;
use moiren_engine::{
    boundary::{AudioReader, BoundaryReport, BridgeError, RtAudioSource, audio_bridge},
    buffer::AudioBlockMut,
    compiler::{CompileConfig, CompileError, CompiledGraph, NodeBindings, compile},
    processor::ProcessContext,
    runtime::EngineConfig,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct ToneConfig {
    pub frequency_hz: f64,
    pub gain: f64,
    pub pan: f64,
    pub max_block_frames: usize,
}
impl Default for ToneConfig {
    fn default() -> Self {
        Self {
            frequency_hz: 440.0,
            gain: 0.05,
            pan: 0.0,
            max_block_frames: 256,
        }
    }
}
impl ToneConfig {
    pub fn validate(self) -> Result<(), ToneError> {
        if !self.frequency_hz.is_finite()
            || self.frequency_hz <= 0.0
            || self.frequency_hz >= 24_000.0
            || !self.gain.is_finite()
            || !(0.0..=1.0).contains(&self.gain)
            || !self.pan.is_finite()
            || !(-1.0..=1.0).contains(&self.pan)
            || self.max_block_frames == 0
            || self.max_block_frames > 4096
        {
            return Err(ToneError::InvalidConfig);
        }
        Ok(())
    }
}
#[derive(Debug, Error)]
pub enum ToneError {
    #[error(
        "tone requires finite 0 < frequency < 24000 Hz, gain in [0, 1], pan in [-1, 1], and a 1..4096 frame block"
    )]
    InvalidConfig,
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
}

struct SineSource {
    phase: f64,
    step: f64,
}
impl RtAudioSource<f32> for SineSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, f32>) -> BoundaryReport {
        for frame in 0..ctx.frames {
            let value = self.phase.sin() as f32;
            output.channel_mut(0)[frame] = value;
            output.channel_mut(1)[frame] = value;
            self.phase += self.step;
            if self.phase >= std::f64::consts::TAU {
                self.phase -= std::f64::consts::TAU;
            }
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}

pub struct ToneSession {
    pub compiled: CompiledGraph<f32>,
    pub output: AudioReader<f32>,
    pub gain_node: NodeId,
    pub pan_node: NodeId,
}
pub fn prepare_tone(config: ToneConfig) -> Result<ToneSession, ToneError> {
    config.validate()?;
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2)?;
    let gain_node = graph.create_node(NodeKind::Gain, 2)?;
    let pan_node = graph.create_node(NodeKind::Pan, 2)?;
    let sink = graph.create_node(NodeKind::Sink, 2)?;
    for (a, b) in [(source, gain_node), (gain_node, pan_node), (pan_node, sink)] {
        graph.connect(
            graph.get_node(a)?.outputs()[0].id(),
            graph.get_node(b)?.inputs()[0].id(),
            SendParams::default(),
        )?;
    }
    let (writer, output) = audio_bridge::<f32>(2, config.max_block_frames, 8 * 1024 * 1024)?;
    let mut bindings = NodeBindings::new();
    bindings.bind_source(
        source,
        SineSource {
            phase: 0.0,
            step: std::f64::consts::TAU * config.frequency_hz / 48_000.0,
        },
    )?;
    bindings.bind_gain(gain_node, config.gain)?;
    bindings.bind_pan(pan_node, config.pan)?;
    bindings.bind_sink(sink, writer)?;
    let compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: config.max_block_frames,
                max_events_per_block: 64,
            },
            audio_byte_budget: 8 * 1024 * 1024,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 64,
            control_horizon_frames: 48_000,
        },
    )?;
    Ok(ToneSession {
        compiled,
        output,
        gain_node,
        pan_node,
    })
}
