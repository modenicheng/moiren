use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::audio_bridge,
    compiler::{CompileConfig, NodeBindings, compile},
    runtime::EngineConfig,
    sample::ProcessingSample,
};
use moiren_windows_audio::virtual_owner::{VirtualMicrophoneSink, VirtualPlaybackSource};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

mod common;

#[test]
fn virtual_src_starvation_backpressure_and_stream_reset_allocate_nothing() {
    for drift in [false, true] {
        check_virtual_src::<f32>(drift);
        check_virtual_src::<f64>(drift);
    }
}

fn check_virtual_src<S: ProcessingSample>(drift: bool) {
    for sample_rate in [44_100_u32, 48_000, 88_200, 96_000] {
        for channels in [1, 2] {
            let epoch = Arc::new(AtomicU64::new(0));
            let playback_producer = Arc::new(AtomicU64::new(0));
            let playback_consumer = Arc::new(AtomicU64::new(0));
            let microphone_producer = Arc::new(AtomicU64::new(0));
            let microphone_consumer = Arc::new(AtomicU64::new(0));
            let (mut input, reader) = audio_bridge(2, 1024, 1 << 20).unwrap();
            let (writer, mut output) = audio_bridge(2, 1024, 1 << 20).unwrap();
            let source = VirtualPlaybackSource::new(reader, channels, 128, sample_rate, 1 << 20)
                .unwrap()
                .with_reset_fence(
                    epoch.clone(),
                    playback_producer.clone(),
                    playback_consumer.clone(),
                );
            let sink = VirtualMicrophoneSink::new(writer, channels, 128, sample_rate, 1 << 20)
                .unwrap()
                .with_reset_fence(
                    epoch.clone(),
                    microphone_producer.clone(),
                    microphone_consumer.clone(),
                );
            let source = if drift {
                source.with_clock_drift().unwrap()
            } else {
                source
            };
            let sink = if drift {
                sink.with_clock_drift().unwrap()
            } else {
                sink
            };
            let mut graph = LogicalGraph::new();
            let source_node = graph.create_node(NodeKind::Source, channels).unwrap();
            let sink_node = graph.create_node(NodeKind::Sink, channels).unwrap();
            graph
                .connect(
                    graph.get_node(source_node).unwrap().outputs()[0].id(),
                    graph.get_node(sink_node).unwrap().inputs()[0].id(),
                    SendParams::default(),
                )
                .unwrap();
            let mut bindings = NodeBindings::<S>::new();
            bindings.bind_source(source_node, source).unwrap();
            bindings.bind_sink(sink_node, sink).unwrap();
            let mut engine = compile(
                &graph,
                bindings,
                CompileConfig {
                    engine: EngineConfig {
                        processing_sr: f64::from(sample_rate),
                        max_block_frames: 128,
                        max_events_per_block: 8,
                    },
                    audio_byte_budget: 1 << 20,
                    plan_revision: 1,
                    timeline_epoch: 1,
                    control_capacity: 8,
                    control_horizon_frames: u64::from(sample_rate),
                },
            )
            .unwrap()
            .engine;
            let pcm = [0.25; 128 * 2];
            let mut scratch = [f32::NAN; 256 * 2];
            common::assert_allocates_nothing(|| {
                for block in 0..128 {
                    if !(16..96).contains(&block) {
                        input.write_interleaved(&pcm).unwrap();
                    }
                    if block == 64 {
                        epoch.fetch_add(1, Ordering::AcqRel);
                    }
                    if block == 65 {
                        // The fake native producer is now fully stopped. RT
                        // playback must have returned silence, not waited.
                        let current = epoch.load(Ordering::Acquire);
                        playback_producer.store(current, Ordering::Release);
                        microphone_producer.store(current, Ordering::Release);
                    }
                    engine.render(128).unwrap();
                    let current = epoch.load(Ordering::Acquire);
                    if microphone_producer.load(Ordering::Acquire) == current
                        && microphone_consumer.load(Ordering::Acquire) != current
                    {
                        output.discard();
                        microphone_consumer.store(current, Ordering::Release);
                    }
                    if block >= 65 {
                        assert_eq!(playback_consumer.load(Ordering::Acquire), current);
                    }
                    if block >= 64 {
                        output.read_interleaved(&mut scratch).unwrap();
                        if channels == 1 {
                            assert!(
                                scratch
                                    .as_chunks::<2>()
                                    .0
                                    .iter()
                                    .all(|frame| frame[1] == 0.0)
                            );
                        }
                    }
                }
            });
        }
    }
}

#[test]
fn rejected_src_budget_allocates_only_the_diagnostic() {
    for microphone in [false, true] {
        let (writer, reader) = audio_bridge(2, 1024, 8192).unwrap();
        let (rejected, counts) = if microphone {
            common::alloc_guard::count_allocations(|| {
                VirtualMicrophoneSink::new(writer, 1, 128, 44_100, 8192).is_err()
            })
        } else {
            common::alloc_guard::count_allocations(|| {
                VirtualPlaybackSource::new(reader, 1, 128, 44_100, 8192).is_err()
            })
        };
        assert!(rejected);
        // The prepared carrier's other endpoint keeps its storage alive.
        // No staging or FIR allocation is admitted by this insufficient budget.
        assert_eq!(counts, (1, 1));
    }
}
