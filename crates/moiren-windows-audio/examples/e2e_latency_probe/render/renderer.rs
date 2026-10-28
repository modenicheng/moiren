use anyhow::{Result, ensure};
use moiren_engine::{boundary::AudioReader, runtime::Engine};
#[cfg(windows)]
use moiren_windows_audio::clock_bridge::{BridgeObserver, BridgeSnapshot};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::live) struct TimelineSnapshot {
    pub frame: u64,
    pub epoch: u64,
    pub revision: u64,
}

#[derive(Clone)]
pub(in crate::live) struct TimelineObserver {
    frame: Arc<AtomicU64>,
    epoch: u64,
    revision: u64,
}
impl TimelineObserver {
    pub(in crate::live) fn try_snapshot(&self) -> Option<TimelineSnapshot> {
        // This probe never switches plans. Epoch/revision are immutable and
        // frame is published only after a complete successful graph block.
        Some(TimelineSnapshot {
            frame: self.frame.load(Ordering::Acquire),
            epoch: self.epoch,
            revision: self.revision,
        })
    }
}

#[derive(Default)]
pub(in crate::live) struct DemandCounters {
    pub frames: u64,
    pub blocks: u64,
    pub segments: u64,
    pub shortfall_frames: u64,
}

pub(in crate::live) struct DemandRenderer {
    engine: Engine<f32>,
    output: AudioReader<f32>,
    counters: DemandCounters,
    timeline: TimelineObserver,
    #[cfg(windows)]
    bridge: Option<BridgeObserver>,
}
impl DemandRenderer {
    pub(in crate::live) fn new(engine: Engine<f32>, output: AudioReader<f32>) -> Result<Self> {
        ensure!(
            !engine.plan_switching_enabled()
                && engine.config().processing_sr == 48_000.0
                && output.channel_count() == 2
                && output.available_frames() == 0
                && output.capacity_frames() >= engine.config().max_block_frames,
            "probe renderer requires an unmodified 48k stereo graph and drained block-sized bridge"
        );
        let timeline = TimelineObserver {
            frame: Arc::new(AtomicU64::new(engine.timeline())),
            epoch: engine.epoch(),
            revision: engine.revision(),
        };
        Ok(Self {
            engine,
            output,
            counters: DemandCounters::default(),
            timeline,
            #[cfg(windows)]
            bridge: None,
        })
    }

    pub(in crate::live) fn timeline_observer(&self) -> TimelineObserver {
        self.timeline.clone()
    }

    #[cfg(windows)]
    pub(in crate::live) fn with_bridge_observer(mut self, observer: BridgeObserver) -> Self {
        self.bridge = Some(observer);
        self
    }

    #[cfg(windows)]
    pub(super) fn bridge_snapshot(&self) -> Option<BridgeSnapshot> {
        self.bridge.as_ref().map(BridgeObserver::snapshot)
    }

    #[cfg(windows)]
    pub(super) fn counters(&self) -> &DemandCounters {
        &self.counters
    }

    /// Caller prepares scratch once, before activation. The successful path
    /// renders and copies bounded blocks without allocating or destroying DSP.
    pub(super) fn render(&mut self, output: &mut [[f64; 2]], scratch: &mut [f32]) -> Result<()> {
        ensure!(
            scratch.len() >= self.engine.config().max_block_frames * 2,
            "probe renderer scratch is smaller than one engine block"
        );
        for frames in output.chunks_mut(self.engine.config().max_block_frames) {
            let rendered = self.engine.render(frames.len())?;
            self.counters.frames += frames.len() as u64;
            self.counters.blocks += 1;
            self.counters.segments += rendered.segments as u64;
            self.timeline
                .frame
                .store(self.engine.timeline(), Ordering::Release);
            let samples = &mut scratch[..frames.len() * 2];
            let copied = self.output.read_interleaved(samples)?;
            self.counters.shortfall_frames += (frames.len() - copied.transferred_frames) as u64;
            for (frame, samples) in frames.iter_mut().zip(samples.as_chunks::<2>().0) {
                *frame = [f64::from(samples[0]), f64::from(samples[1])];
            }
            ensure!(
                copied.transferred_frames == frames.len(),
                "probe graph output bridge shortfall"
            );
        }
        Ok(())
    }

    #[cfg(windows)]
    pub(super) fn scratch_samples(&self) -> usize {
        self.engine.config().max_block_frames * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
    use moiren_engine::{
        boundary::{ConstantSource, audio_bridge},
        compiler::{CompileConfig, NodeBindings, compile},
    };

    fn renderer() -> DemandRenderer {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        graph
            .connect(
                graph.get_node(source).unwrap().outputs()[0].id(),
                graph.get_node(sink).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
        let (writer, reader) = audio_bridge(2, 8192, 1 << 20).unwrap();
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 2,
                    value: 0.25,
                },
            )
            .unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let compiled = compile(
            &graph,
            bindings,
            CompileConfig {
                engine: moiren_engine::runtime::EngineConfig {
                    processing_sr: 48_000.0,
                    max_block_frames: 256,
                    max_events_per_block: 64,
                },
                audio_byte_budget: 1 << 20,
                plan_revision: 1,
                timeline_epoch: 1,
                control_capacity: 64,
                control_horizon_frames: 48_000,
            },
        )
        .unwrap();
        DemandRenderer::new(compiled.engine, reader).unwrap()
    }

    #[test]
    fn probe_renderer_splits_demand_and_publishes_the_fixed_graph_timeline() {
        let mut renderer = renderer();
        let timeline = renderer.timeline_observer();
        let initial = timeline.try_snapshot().unwrap();
        let mut frames = [[f64::NAN; 2]; 777];
        let mut scratch = vec![0.0; renderer.engine.config().max_block_frames * 2];
        renderer.render(&mut frames, &mut scratch).unwrap();
        assert!(frames.iter().all(|frame| *frame == [0.25; 2]));
        let after = timeline.try_snapshot().unwrap();
        assert_eq!(after.frame, initial.frame + 777);
        assert_eq!(
            (after.epoch, after.revision),
            (initial.epoch, initial.revision)
        );
        assert_eq!(renderer.counters.frames, 777);
        assert_eq!(renderer.counters.shortfall_frames, 0);
        renderer.render(&mut [], &mut scratch).unwrap();
        assert_eq!(timeline.try_snapshot().unwrap(), after);
    }

    #[test]
    fn invalid_scratch_does_not_advance_the_graph_or_leave_partially_generated_audio() {
        let mut renderer = renderer();
        let before = renderer.timeline_observer().try_snapshot().unwrap();
        let mut frames = [[0.5; 2]; 2];
        assert!(renderer.render(&mut frames, &mut []).is_err());
        assert_eq!(frames, [[0.5; 2]; 2]);
        assert_eq!(renderer.timeline_observer().try_snapshot().unwrap(), before);
    }
}
