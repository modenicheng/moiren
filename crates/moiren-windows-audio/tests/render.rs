use moiren_core::graph::*;
use moiren_engine::{boundary::*, compiler::*, runtime::*};
use moiren_windows_audio::render::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    time::Duration,
};

struct CountingAllocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static COUNTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK.try_with(|t| {
            if t.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a + 1, d));
                });
            }
        });
        // SAFETY: Forward the caller's unchanged allocation contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = TRACK.try_with(|t| {
            if t.get() {
                let _ = COUNTS.try_with(|n| {
                    let (a, d) = n.get();
                    n.set((a, d + 1));
                });
            }
        });
        // SAFETY: The allocation originated from this System allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct RampSource;
impl RtAudioSource<f32> for RampSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        ctx: &moiren_engine::processor::ProcessContext,
        mut output: moiren_engine::buffer::AudioBlockMut<'_, f32>,
    ) -> BoundaryReport {
        for (channel, samples) in output.channels_mut().enumerate() {
            for (index, sample) in samples.iter_mut().enumerate() {
                let value = (ctx.timeline_start + index as u64) as f32 / 1000.0;
                *sample = if channel == 0 { value } else { -value };
            }
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}
fn graph(sample_rate: f64) -> (Engine<f32>, AudioReader<f32>) {
    let (writer, reader) = audio_bridge::<f32>(2, 8, 4096).unwrap();
    (graph_with_sink(sample_rate, writer), reader)
}
fn graph_with_sink(sample_rate: f64, writer: impl RtAudioSink<f32> + 'static) -> Engine<f32> {
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
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, RampSource).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: sample_rate,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48000,
        },
    )
    .unwrap();
    compiled.engine
}

#[test]
fn zero_and_variable_demand_preserve_complete_frames_and_timeline() {
    let (engine, reader) = graph(48000.0);
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    assert_eq!(renderer.render_interleaved(&mut []).unwrap().blocks, 0);
    assert_eq!(renderer.timeline(), 0);
    let mut frame = 0;
    for frames in [1usize, 19, 3, 8] {
        let mut samples = vec![99.0; frames * 2];
        let report = renderer.render_interleaved(&mut samples).unwrap();
        assert_eq!(report.frames, frames);
        assert_eq!(report.blocks, frames.div_ceil(8));
        for pair in samples.as_chunks::<2>().0 {
            assert_eq!(pair, &[frame as f32 / 1000.0, -(frame as f32 / 1000.0)]);
            frame += 1;
        }
    }
    assert_eq!(renderer.timeline(), 31);
}

#[test]
fn malformed_requests_and_wrong_bridge_or_rate_do_not_render() {
    let (engine, reader) = graph(44100.0);
    assert!(matches!(
        DemandRenderer::new(engine, reader),
        Err(RenderError::UnsupportedFormat)
    ));
    let (engine, _) = graph(48000.0);
    let (_writer, reader) = audio_bridge::<f32>(1, 8, 4096).unwrap();
    assert!(matches!(
        DemandRenderer::new(engine, reader),
        Err(RenderError::UnsupportedFormat)
    ));
    let (engine, _) = graph(48000.0);
    let (_writer, reader) = audio_bridge::<f32>(2, 4, 4096).unwrap();
    assert!(matches!(
        DemandRenderer::new(engine, reader),
        Err(RenderError::BridgeCapacity)
    ));
    let (engine, reader) = graph(48000.0);
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    assert_eq!(
        renderer.render_interleaved(&mut [1.0; 3]),
        Err(RenderError::InvalidSamples)
    );
    assert_eq!(renderer.timeline(), 0);
}

#[test]
fn a_wrong_output_bridge_returns_shortfall_with_initialized_silence() {
    let (engine, _actual_reader) = graph(48000.0);
    let (_unused_writer, reader) = audio_bridge::<f32>(2, 8, 4096).unwrap();
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    let mut samples = [99.0; 38];
    assert_eq!(
        renderer.render_interleaved(&mut samples),
        Err(RenderError::IncompleteTransfer {
            expected: 8,
            actual: 0
        })
    );
    assert_eq!(samples, [0.0; 38]);
    assert_eq!(renderer.counters().frames, 8);
    assert_eq!(renderer.counters().blocks, 1);
    assert_eq!(renderer.counters().segments, 1);
}

struct FirstBlockOnly(AudioWriter<f32>);
impl RtAudioSink<f32> for FirstBlockOnly {
    fn channel_count(&self) -> usize {
        2
    }
    fn write(
        &mut self,
        ctx: &moiren_engine::processor::ProcessContext,
        input: moiren_engine::buffer::AudioBlock<'_, f32>,
    ) -> BoundaryReport {
        if ctx.timeline_start == 0 {
            self.0.write(ctx, input)
        } else {
            BoundaryReport::default()
        }
    }
}

#[test]
fn failed_later_transfer_retains_all_completed_dsp_counts() {
    let (writer, reader) = audio_bridge::<f32>(2, 8, 4096).unwrap();
    let engine = graph_with_sink(48000.0, FirstBlockOnly(writer));
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    let mut samples = [99.0; 38];
    assert_eq!(
        renderer.render_interleaved(&mut samples),
        Err(RenderError::IncompleteTransfer {
            expected: 8,
            actual: 0
        })
    );
    assert_eq!(renderer.timeline(), 16);
    assert_eq!(
        renderer.counters(),
        DemandCounters {
            frames: 16,
            blocks: 2,
            segments: 2
        }
    );
    assert_eq!(&samples[2..4], &[0.001, -0.001]);
    assert_eq!(&samples[16..], &[0.0; 22]);
}

#[test]
fn demand_counters_exclude_engine_work_before_preparation() {
    let (mut engine, mut reader) = graph(48000.0);
    engine.render(3).unwrap();
    reader.read_interleaved(&mut [0.0; 6]).unwrap();
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    assert_eq!(renderer.counters(), DemandCounters::default());
    renderer.render_interleaved(&mut [0.0; 4]).unwrap();
    assert_eq!(renderer.timeline(), 5);
    assert_eq!(
        renderer.counters(),
        DemandCounters {
            frames: 2,
            blocks: 1,
            segments: 1
        }
    );
}

#[test]
fn render_demand_has_no_allocations_or_deallocations() {
    let (engine, reader) = graph(48000.0);
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    let mut samples = [0.0; 38];
    COUNTS.with(|n| n.set((0, 0)));
    TRACK.with(|t| t.set(true));
    let result = renderer.render_interleaved(&mut samples);
    TRACK.with(|t| t.set(false));
    assert!(result.is_ok());
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
}

#[test]
fn padding_and_explicit_render_options_are_checked() {
    assert_eq!(writable_frames(1024, 1024), Ok(0));
    assert_eq!(writable_frames(1024, 17), Ok(1007));
    assert_eq!(writable_frames(0, 0), Err(RenderError::InvalidPadding));
    assert_eq!(writable_frames(10, 11), Err(RenderError::InvalidPadding));
    for endpoint in ["", " ", "id\0other"] {
        assert_eq!(
            RenderOptions {
                endpoint_id: endpoint.into(),
                duration: Duration::from_secs(10)
            }
            .validate(),
            Err(RenderError::InvalidEndpoint)
        );
    }
    for duration in [Duration::ZERO, Duration::from_secs(601)] {
        assert_eq!(
            RenderOptions {
                endpoint_id: "id".into(),
                duration
            }
            .validate(),
            Err(RenderError::InvalidDuration)
        );
    }
    assert!(
        RenderOptions {
            endpoint_id: "id".into(),
            duration: Duration::from_secs(10)
        }
        .validate()
        .is_ok()
    );
}
