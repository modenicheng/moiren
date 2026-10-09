//! Low-amplitude owned child generator with an isolated WASAPI session.
use anyhow::{Result, bail};
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource, audio_bridge},
    buffer::AudioBlockMut,
    compiler::{CompileConfig, NodeBindings, compile},
    processor::ProcessContext,
    runtime::EngineConfig,
};
use moiren_windows_audio::render::{DemandRenderer, RenderOptions, RenderStatus, start_render};
use std::time::Duration;
struct Tone(f64);
impl RtAudioSource<f32> for Tone {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, ctx: &ProcessContext, mut out: AudioBlockMut<'_, f32>) -> BoundaryReport {
        for frame in 0..out.frames() {
            let value = (self.0.sin() * 0.02) as f32;
            out.channel_mut(0)[frame] = value;
            out.channel_mut(1)[frame] = value;
            self.0 = (self.0 + std::f64::consts::TAU * 440.0 / ctx.processing_sr)
                % std::f64::consts::TAU;
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}

pub(super) fn run(endpoint: String) -> Result<()> {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2)?;
    let sink = graph.create_node(NodeKind::Sink, 2)?;
    graph.connect(
        graph.get_node(source)?.outputs()[0].id(),
        graph.get_node(sink)?.inputs()[0].id(),
        SendParams::default(),
    )?;
    let (writer, reader) = audio_bridge::<f32>(2, 256, 4096)?;
    let mut io = NodeBindings::new();
    io.bind_source(source, Tone(0.0))?;
    io.bind_sink(sink, writer)?;
    let compiled = compile(
        &graph,
        io,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 256,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )?;
    let renderer = DemandRenderer::new(compiled.engine, reader)?;
    let report = start_render(
        RenderOptions {
            endpoint_id: endpoint,
            duration: Duration::from_secs(60),
        },
        renderer,
    )?
    .join()?;
    if report.status == RenderStatus::Failed {
        bail!("owned generator failed: {:?}", report.failure);
    }
    Ok(())
}
