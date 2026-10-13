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

#[test]
fn fade_is_segment_independent_and_requires_successful_submission() {
    for chunk in [1, 3, 8, 32] {
        let (engine, output) = graph(48_000.0);
        let mut renderer = DemandRenderer::new(engine, output).unwrap();
        let fade = renderer.fade_control();
        fade.request_fade_in(8).unwrap();
        let mut samples = [0.0; 64];
        COUNTS.with(|count| count.set((0, 0)));
        TRACK.with(|track| track.set(true));
        for segment in samples[..16].chunks_mut(chunk * 2) {
            renderer.render_interleaved(segment).unwrap();
        }
        fade.request_fade_out(8).unwrap();
        renderer.render_interleaved(&mut []).unwrap();
        assert!(!fade.fade_out_submitted());
        for segment in samples[16..32].chunks_mut(chunk * 2) {
            renderer.render_interleaved(segment).unwrap();
        }
        assert!(!fade.fade_out_submitted());
        renderer.acknowledge_submission();
        assert!(fade.fade_out_submitted());
        renderer.render_interleaved(&mut samples[32..]).unwrap();
        TRACK.with(|track| track.set(false));
        assert_eq!(COUNTS.with(Cell::get), (0, 0));
        for frame in 0..8 {
            let expected = frame as f32 / 1000.0 * (frame + 1) as f32 / 8.0;
            assert!((samples[frame * 2] - expected).abs() < 1e-8);
        }
        for frame in 8..16 {
            let expected = frame as f32 / 1000.0 * (15 - frame) as f32 / 8.0;
            assert!((samples[frame * 2] - expected).abs() < 1e-8);
            assert_eq!(samples[frame * 2], -samples[frame * 2 + 1]);
        }
        assert!(samples[32..].iter().all(|sample| *sample == 0.0));
        assert_eq!(renderer.timeline(), 32);
        assert!(fade.request_fade_out(8).is_err());
    }
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
fn timeline_observer_rejects_plan_switching_engines() {
    let (mut engine, reader) = graph(48000.0);
    engine.enable_plan_switching(1).unwrap();

    match DemandRenderer::new(engine, reader) {
        Err(error) => assert_eq!(
            error.to_string(),
            "demand renderer does not support plan switching"
        ),
        Ok(_) => panic!("demand renderer accepted a switchable engine"),
    }
}

struct ConstantSource(f32);
impl RtAudioSource<f32> for ConstantSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        context: &moiren_engine::processor::ProcessContext,
        mut output: moiren_engine::buffer::AudioBlockMut<'_, f32>,
    ) -> BoundaryReport {
        for samples in output.channels_mut() {
            samples.fill(self.0);
        }
        BoundaryReport {
            transferred_frames: context.frames,
            ..BoundaryReport::default()
        }
    }
}

fn managed_graph() -> (
    LogicalGraph,
    NodeId,
    NodeId,
    CompiledGraph<f32>,
    AudioReader<f32>,
) {
    let (writer, reader) = audio_bridge::<f32>(2, 8, 4096).unwrap();
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
    bindings.bind_source(source, ConstantSource(0.25)).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let compiled = compile(&graph, bindings, managed_config(1)).unwrap();
    (graph, source, sink, compiled, reader)
}

fn managed_config(revision: u64) -> CompileConfig {
    CompileConfig {
        engine: EngineConfig {
            processing_sr: 48_000.0,
            max_block_frames: 8,
            max_events_per_block: 8,
        },
        audio_byte_budget: 4096,
        plan_revision: revision,
        timeline_epoch: 1,
        control_capacity: 8,
        control_horizon_frames: 48_000,
    }
}

#[test]
fn managed_swap_preserves_the_only_output_writer_and_coherent_timeline_without_rt_allocation() {
    let (graph, source, sink, mut compiled, reader) = managed_graph();
    let initial_sink = compiled.bindings.node(sink).unwrap();
    let basis = compiled.engine.plan_snapshot();
    let mut plans = compiled.engine.enable_plan_switching(1).unwrap();
    let mut renderer =
        DemandRenderer::new_managed_with_generation(compiled.engine, reader, initial_sink, 17)
            .unwrap();
    let timeline = renderer.timeline_observer();
    renderer.render_interleaved(&mut [0.0; 6]).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, ConstantSource(0.75)).unwrap();
    let update = compile_update(
        &graph,
        bindings,
        managed_config(2),
        &basis,
        &[(sink, initial_sink)],
        None,
    )
    .unwrap();
    plans.publish(update.plan).unwrap();
    let mut samples = [0.0; 38];
    COUNTS.with(|counts| counts.set((0, 0)));
    TRACK.with(|tracking| tracking.set(true));
    let rendered = renderer.render_interleaved(&mut samples);
    TRACK.with(|tracking| tracking.set(false));
    assert!(rendered.is_ok());
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert_eq!(samples, [0.75; 38]);
    assert_eq!(plans.active_revision(), 2);
    assert_eq!(
        timeline.managed_snapshot(),
        Some(ManagedTimelineSnapshot {
            owner_generation: 17,
            epoch: 1,
            revision: 2,
            frame: 22,
        })
    );
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Replaced {
            active_revision: 2,
            frame: 3,
        }
    );
    let (engine, _) = renderer.into_parts();
    assert_eq!(engine.revision(), 2);
}

#[test]
fn managed_renderer_rejects_arbitrary_candidate_output_and_keeps_the_original_bridge() {
    let (_, _, sink, mut compiled, reader) = managed_graph();
    let sink = compiled.bindings.node(sink).unwrap();
    let mut plans = compiled.engine.enable_plan_switching(1).unwrap();
    let mut renderer = DemandRenderer::new_managed(compiled.engine, reader, sink).unwrap();
    let (graph, source, sink_node, _, _) = managed_graph();
    let (writer, mut candidate_reader) = audio_bridge::<f32>(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, ConstantSource(0.75)).unwrap();
    bindings.bind_sink(sink_node, writer).unwrap();
    let candidate = compile(&graph, bindings, managed_config(2)).unwrap();
    plans
        .publish(PreparedPlan::new(candidate.engine).unwrap())
        .unwrap();
    let mut samples = [0.0; 16];
    renderer.render_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.25; 16]);
    assert_eq!(plans.active_revision(), 1);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::MissingReuse)
    );
    assert_eq!(
        candidate_reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        0
    );
}

#[test]
fn managed_cancel_and_full_retirement_keep_output_live_until_sink_migrates() {
    let (graph, source, sink, mut compiled, reader) = managed_graph();
    let mut basis = compiled.engine.plan_snapshot();
    let mut sink_processor = compiled.bindings.node(sink).unwrap();
    let mut plans = compiled.engine.enable_plan_switching(1).unwrap();
    let mut renderer =
        DemandRenderer::new_managed(compiled.engine, reader, sink_processor).unwrap();
    let prepare = |revision, value, basis: &PlanSnapshot, sink_processor| {
        let mut bindings = NodeBindings::new();
        bindings.bind_source(source, ConstantSource(value)).unwrap();
        compile_update(
            &graph,
            bindings,
            managed_config(revision),
            basis,
            &[(sink, sink_processor)],
            None,
        )
        .unwrap()
    };
    let cancelled = prepare(2, 0.5, &basis, sink_processor);
    plans.publish(cancelled.plan).unwrap();
    plans.cancel_pending();
    let mut samples = [0.0; 2];
    renderer.render_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.25; 2]);
    let update = prepare(3, 0.75, &basis, sink_processor);
    basis = update.plan.snapshot();
    sink_processor = update.bindings.node(sink).unwrap();
    plans.publish(update.plan).unwrap();
    renderer.render_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.25; 2]);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::Cancelled)
    );
    renderer.render_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.75; 2]);
    drop(plans.poll_retired().unwrap());
    let update = prepare(4, 0.125, &basis, sink_processor);
    plans.publish(update.plan).unwrap();
    renderer.render_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [0.125; 2]);
    assert_eq!(renderer.timeline_observer().snapshot().revision, 4);
    drop(plans.poll_retired().unwrap());
}

#[test]
fn managed_swap_returns_old_controls_for_explicit_request_termination() {
    use moiren_core::protocol::{ApplyAt, ParamValue, ParameterRequest, ReplyCode};
    let (graph, source, sink, mut compiled, reader) = managed_graph();
    let target = compiled.bindings.edge(graph.edges()[0].id()).unwrap().gain;
    assert_eq!(
        compiled
            .control
            .submit(
                ParameterRequest {
                    request_id: 11,
                    plan_revision: 1,
                    timeline_epoch: 1,
                    target,
                    at: ApplyAt::Frame(128),
                    value: ParamValue::Float(0.5),
                    ramp_frames: 0,
                },
                0
            )
            .code,
        ReplyCode::Accepted
    );
    let basis = compiled.engine.plan_snapshot();
    let sink = compiled.bindings.node(sink).unwrap();
    let mut plans = compiled.engine.enable_plan_switching(1).unwrap();
    let mut renderer = DemandRenderer::new_managed(compiled.engine, reader, sink).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, ConstantSource(0.75)).unwrap();
    let sink_node = graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Sink)
        .unwrap()
        .id();
    let update = compile_update(
        &graph,
        bindings,
        managed_config(2),
        &basis,
        &[(sink_node, sink)],
        None,
    )
    .unwrap();
    plans.publish(update.plan).unwrap();
    renderer.render_interleaved(&mut [0.0; 2]).unwrap();
    let mut retired = plans.poll_retired().unwrap();
    assert_eq!(retired.reject_pending(), 0);
    let reply = compiled.control.poll_applied().unwrap();
    assert_eq!(reply.request_id, 11);
    assert_eq!(reply.code, ReplyCode::StaleRevision);
    assert!(compiled.control.poll_applied().is_none());
}

#[test]
fn managed_preparation_error_returns_engine_to_caller() {
    let (_, _, sink, mut compiled, reader) = managed_graph();
    let sink = compiled.bindings.node(sink).unwrap();
    compiled.engine.enable_plan_switching(1).unwrap();
    let failure = match DemandRenderer::new_managed_returning(
        compiled.engine,
        reader,
        moiren_core::protocol::ProcessorId(u64::MAX),
        7,
    ) {
        Err(failure) => failure,
        Ok(_) => panic!("missing sink accepted"),
    };
    assert_eq!(
        failure.error,
        RenderError::PlanSwap(PlanSwapError::MissingProcessor)
    );
    assert_eq!(failure.engine.revision(), 1);
    assert_eq!(failure.output.capacity_frames(), 8);
    assert_ne!(sink, moiren_core::protocol::ProcessorId(u64::MAX));
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
                queue: moiren_windows_audio::latency::RenderQueuePolicy::default(),
                latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
                shared_period: moiren_windows_audio::shared_stream::SharedPeriodPolicy::default(),
                endpoint_id: endpoint.into(),
                duration: moiren_windows_audio::SessionDuration::For(Duration::from_secs(10))
            }
            .validate(),
            Err(RenderError::InvalidEndpoint)
        );
    }
    for duration in [Duration::ZERO, Duration::from_secs(601)] {
        assert_eq!(
            RenderOptions {
                queue: moiren_windows_audio::latency::RenderQueuePolicy::default(),
                latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
                shared_period: moiren_windows_audio::shared_stream::SharedPeriodPolicy::default(),
                endpoint_id: "id".into(),
                duration: moiren_windows_audio::SessionDuration::For(duration)
            }
            .validate(),
            Err(RenderError::InvalidDuration)
        );
    }
    assert!(
        RenderOptions {
            queue: moiren_windows_audio::latency::RenderQueuePolicy::default(),
            latency_telemetry: moiren_windows_audio::latency::TelemetryMode::Summary,
            shared_period: moiren_windows_audio::shared_stream::SharedPeriodPolicy::default(),
            endpoint_id: "id".into(),
            duration: moiren_windows_audio::SessionDuration::For(Duration::from_secs(10))
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn target_prefill_and_irregular_refills_advance_only_submitted_audio() {
    use moiren_windows_audio::latency::{
        NativeStreamTiming, RenderQueuePolicy, prepare_render_queue,
    };
    let (engine, reader) = graph(48000.0);
    let mut renderer = DemandRenderer::new(engine, reader).unwrap();
    let queue = prepare_render_queue(
        NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(8),
            capacity_frames: 32,
        },
        RenderQueuePolicy::Target {
            frames: 16,
            wake_margin_frames: 4,
            callback_budget_frames: 4,
        },
    )
    .unwrap();
    let mut staging = [0.0; 64];
    let mut submitted = 0;
    for padding in [0, 16, 9, 21, 2, 32] {
        let demand = queue.demand(padding).unwrap();
        let before = renderer.timeline();
        renderer
            .render_interleaved(&mut staging[..demand as usize * 2])
            .unwrap();
        submitted += u64::from(demand);
        assert_eq!(renderer.timeline(), before + u64::from(demand));
    }
    assert_eq!(submitted, 37);
    assert_eq!(renderer.counters().frames, 37);
}
