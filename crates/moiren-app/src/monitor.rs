//! Compiler-prepared capture routing, independent of Windows device ownership.
use moiren_core::graph::*;
use moiren_engine::{
    boundary::{AudioReader, BridgeError, RtAudioSource, audio_bridge},
    compiler::{CompileConfig, CompileError, CompiledGraph, NodeBindings, compile},
    runtime::EngineConfig,
};
use thiserror::Error;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{MonitorOptions, MonitorReport, MonitorSession, MonitorStatus, start_monitor};

#[derive(Debug, Clone, Copy)]
pub struct MonitorConfig {
    pub gain: f64,
    pub pan: f64,
    pub max_block_frames: usize,
}
impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            gain: 0.05,
            pan: 0.0,
            max_block_frames: 256,
        }
    }
}
impl MonitorConfig {
    pub fn validate(self) -> Result<(), MonitorError> {
        if !self.gain.is_finite()
            || !(0.0..=1.0).contains(&self.gain)
            || !self.pan.is_finite()
            || !(-1.0..=1.0).contains(&self.pan)
            || !(1..=4096).contains(&self.max_block_frames)
        {
            return Err(MonitorError::InvalidConfig);
        }
        Ok(())
    }
}
#[derive(Debug, Error)]
pub enum MonitorError {
    #[error(
        "monitor requires finite gain in [0,1], pan in [-1,1], a 1..4096 frame block and a stereo source"
    )]
    InvalidConfig,
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[cfg(windows)]
    #[error(transparent)]
    Capture(#[from] moiren_windows_audio::capture::CaptureError),
    #[cfg(windows)]
    #[error(transparent)]
    Render(#[from] moiren_windows_audio::render::RenderError),
}
pub struct MonitorGraph {
    pub compiled: CompiledGraph<f32>,
    pub output: AudioReader<f32>,
    pub gain_node: NodeId,
    pub pan_node: NodeId,
}
pub fn prepare_monitor(
    source: impl RtAudioSource<f32> + 'static,
    config: MonitorConfig,
) -> Result<MonitorGraph, MonitorError> {
    config.validate()?;
    if source.channel_count() != 2 {
        return Err(MonitorError::InvalidConfig);
    }
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2)?;
    let gain_node = graph.create_node(NodeKind::Gain, 2)?;
    let pan_node = graph.create_node(NodeKind::Pan, 2)?;
    let sink = graph.create_node(NodeKind::Sink, 2)?;
    for (a, b) in [(input, gain_node), (gain_node, pan_node), (pan_node, sink)] {
        graph.connect(
            graph.get_node(a)?.outputs()[0].id(),
            graph.get_node(b)?.inputs()[0].id(),
            SendParams::default(),
        )?;
    }
    let (writer, output) = audio_bridge(2, config.max_block_frames, 8 * 1024 * 1024)?;
    let mut bindings = NodeBindings::new();
    bindings.bind_source(input, source)?;
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
    Ok(MonitorGraph {
        compiled,
        output,
        gain_node,
        pan_node,
    })
}
