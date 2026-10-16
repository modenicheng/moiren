//! The master executes the graph once; every follower consumes its own sink
//! through the same bounded continuous clock adapter used by native owners.
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource, audio_bridge},
    buffer::AudioBlockMut,
    compiler::{CompileConfig, NodeBindings, compile},
    processor::ProcessContext,
    runtime::EngineConfig,
};
use moiren_windows_audio::{
    clock_bridge::{BridgeObserver, ClockBridgeConfig, ClockSource, capture_bridge},
    engine_backend::NativeLatencyProfile,
    latency::{LatencyBudgetPolicy, NativeStreamTiming},
    render::DemandRenderer,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct Allocator;
thread_local! { static TRACK: Cell<bool> = const { Cell::new(false) }; static COUNTS: Cell<(usize,usize)> = const { Cell::new((0,0)) }; }
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            COUNTS.with(|count| {
                let (a, d) = count.get();
                count.set((a + 1, d));
            });
        }
        // SAFETY: Forward the caller's unchanged allocation contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            COUNTS.with(|count| {
                let (a, d) = count.get();
                count.set((a, d + 1));
            });
        }
        // SAFETY: The allocation originated from this System allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
struct Constant(f32);
impl RtAudioSource<f32> for Constant {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        context: &ProcessContext,
        mut output: AudioBlockMut<'_, f32>,
    ) -> BoundaryReport {
        for channel in output.channels_mut() {
            channel.fill(self.0);
        }
        BoundaryReport {
            transferred_frames: context.frames,
            ..Default::default()
        }
    }
}
fn graph(config: ClockBridgeConfig) -> (DemandRenderer, ClockSource, BridgeObserver) {
    let mut graph = LogicalGraph::new();
    let sources = [
        graph.create_node(NodeKind::Source, 2).unwrap(),
        graph.create_node(NodeKind::Source, 2).unwrap(),
    ];
    let sinks = [
        graph.create_node(NodeKind::Sink, 2).unwrap(),
        graph.create_node(NodeKind::Sink, 2).unwrap(),
    ];
    for index in 0..2 {
        graph
            .connect(
                graph.get_node(sources[index]).unwrap().outputs()[0].id(),
                graph.get_node(sinks[index]).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
    }
    let (writer, reader) = audio_bridge(2, 1024, 1024 * 1024).unwrap();
    let (follower, source, observer) = capture_bridge(config).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(sources[0], Constant(0.25)).unwrap();
    bindings.bind_source(sources[1], Constant(-0.5)).unwrap();
    bindings.bind_sink(sinks[0], writer).unwrap();
    bindings.bind_sink(sinks[1], follower).unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48000.0,
                max_block_frames: 1024,
                max_events_per_block: 8,
            },
            audio_byte_budget: 1024 * 1024,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48000,
        },
    )
    .unwrap();
    let plans = compiled.engine.enable_plan_switching(1).unwrap();
    drop(plans);
    let protected = sinks.map(|sink| compiled.bindings.node(sink).unwrap());
    let renderer =
        match DemandRenderer::new_managed_outputs_returning(compiled.engine, reader, &protected, 1)
        {
            Ok(renderer) => renderer,
            Err(_) => panic!("valid master renderer"),
        };
    (renderer, source, observer)
}
fn timing(period: u32, capacity: u32) -> NativeStreamTiming {
    NativeStreamTiming {
        sample_rate: 48000,
        period_frames: Some(period),
        capacity_frames: capacity,
    }
}
#[test]
fn every_output_routes_its_own_graph_signal_without_rt_allocation_or_destruction() {
    let (mut master, mut follower, observer) = graph(ClockBridgeConfig {
        capacity_frames: 8192,
        target_fill_frames: 2048,
        ..Default::default()
    });
    let mut master_samples = [0.0; 672];
    let mut follower_samples = [0.0; 480];
    let mut routed_frames = 0;
    COUNTS.with(|count| count.set((0, 0)));
    TRACK.with(|track| track.set(true));
    for _ in 0..1000 {
        master.render_interleaved(&mut master_samples).unwrap();
        follower.read_interleaved(&mut follower_samples).unwrap();
        routed_frames += follower_samples
            .iter()
            .filter(|&&sample| sample == -0.5)
            .count();
    }
    TRACK.with(|track| track.set(false));
    assert_eq!(COUNTS.with(Cell::get), (0, 0));
    assert!(
        routed_frames > 0,
        "follower must render its own nonzero signal"
    );
    assert!(master_samples.iter().all(|sample| *sample == 0.25));
    assert!(
        follower_samples
            .iter()
            .all(|sample| *sample == 0.0 || *sample == -0.5)
    );
    assert!(
        observer.snapshot().dropped_frames > 0,
        "slow consumer must exhibit bounded backpressure"
    );
    assert!(observer.snapshot().fill_frames <= 8194);
    assert_eq!(master.timeline(), 336000);
}
#[test]
fn asymmetric_native_periods_and_bounded_drift_do_not_need_steady_state_recovery() {
    for (drift_ppm, phase) in [
        (-750, 0),
        (-750, 120),
        (-750, 239),
        (0, 0),
        (0, 120),
        (0, 239),
        (750, 0),
        (750, 120),
        (750, 239),
    ] {
        let master_queue = moiren_windows_audio::latency::prepare_render_queue(
            timing(336, 738),
            NativeLatencyProfile::Low
                .render_policy(timing(336, 738), Default::default())
                .unwrap(),
        )
        .unwrap();
        let input = NativeLatencyProfile::Low
            .follower_input(
                timing(336, 738),
                timing(240, 528),
                LatencyBudgetPolicy::Compatible,
                master_queue,
            )
            .unwrap();
        let (mut master, mut follower, observer) =
            graph(ClockBridgeConfig::with_prepared_input(48000, 2, input));
        let mut master_samples = [0.0; 1008];
        let mut follower_samples = [0.0; 720];
        master.render_interleaved(&mut master_samples).unwrap();
        follower.read_interleaved(&mut follower_samples).unwrap();
        let mut master_at: f64 = 336.0;
        let mut follower_at: f64 = 240.0 + f64::from(phase);
        let step = 240.0 / (1.0 + f64::from(drift_ppm) / 1_000_000.0);
        let mut master_event = 1_u64;
        let mut previous_master_at = 0.0;
        while master_at.min(follower_at) < 48000.0 * 30.0 {
            if master_at <= follower_at {
                let demand = (master_at - previous_master_at) as usize;
                assert!(demand <= 504);
                master
                    .render_interleaved(&mut master_samples[..demand * 2])
                    .unwrap();
                previous_master_at = master_at;
                master_event += 1;
                master_at = (master_event * 336) as f64
                    + if master_event.is_multiple_of(11) {
                        168.0
                    } else {
                        0.0
                    };
            } else {
                follower
                    .read_interleaved(&mut follower_samples[..480])
                    .unwrap();
                follower_at += step;
            }
        }
        let counters = observer.snapshot();
        assert_eq!(
            counters.dropped_frames, 0,
            "drift {drift_ppm}: {counters:?}"
        );
        assert_eq!(
            counters.underrun_frames, 0,
            "drift {drift_ppm}: {counters:?}"
        );
        assert_eq!(
            counters.backlog_recoveries, 0,
            "drift {drift_ppm}: {counters:?}"
        );
        assert!(counters.correction_ppm.abs() <= 2000.0);
        assert!(follower_samples[..480].iter().all(|sample| *sample == -0.5));
    }
}

#[test]
fn persistent_native_demand_deficit_is_not_hidden_by_healthy_wake_intervals() {
    // The CABLE trace contains 0 then 360 frame demands in place of two
    // ordinary 240 frame callbacks, even with every wake below 7.5 ms. Each
    // pair loses 120 frames of consumption; repeated every 0.6 s this exceeds
    // the admitted 2000 ppm clock servo. This is not an exact driver replay.
    for (paired_demands, recovery_expected) in
        [([240, 240], false), ([120, 360], false), ([0, 360], true)]
    {
        let master_queue = moiren_windows_audio::latency::prepare_render_queue(
            timing(336, 738),
            NativeLatencyProfile::Low
                .render_policy(timing(336, 738), Default::default())
                .unwrap(),
        )
        .unwrap();
        let input = NativeLatencyProfile::Low
            .follower_input(
                timing(336, 738),
                timing(240, 528),
                LatencyBudgetPolicy::Compatible,
                master_queue,
            )
            .unwrap();
        let (mut master, mut follower, observer) =
            graph(ClockBridgeConfig::with_prepared_input(48000, 2, input));
        let mut master_samples = [0.0; 1008];
        let mut follower_samples = [0.0; 720];
        master.render_interleaved(&mut master_samples).unwrap();
        follower.read_interleaved(&mut follower_samples).unwrap();
        let mut master_at = 336;
        let mut follower_at = 240;
        let mut follower_event = 1_usize;
        COUNTS.with(|count| count.set((0, 0)));
        TRACK.with(|track| track.set(true));
        while master_at.min(follower_at) < 48000 * 30 {
            if master_at <= follower_at {
                master
                    .render_interleaved(&mut master_samples[..672])
                    .unwrap();
                master_at += 336;
            } else {
                let demand = match follower_event % 120 {
                    0 => paired_demands[0],
                    1 if follower_event > 1 => paired_demands[1],
                    _ => 240,
                };
                follower
                    .read_interleaved(&mut follower_samples[..demand * 2])
                    .unwrap();
                follower_at += 240;
                follower_event += 1;
            }
        }
        TRACK.with(|track| track.set(false));
        assert_eq!(COUNTS.with(Cell::get), (0, 0));
        let counters = observer.snapshot();
        assert_eq!(counters.dropped_frames, 0, "{counters:?}");
        assert_eq!(counters.underrun_frames, 0, "{counters:?}");
        assert!(counters.correction_ppm.abs() <= 2000.0);
        if recovery_expected {
            assert!(counters.backlog_recoveries > 0, "{counters:?}");
            assert!(counters.backlog_discarded_frames > 0);
            assert_eq!(counters.resets, counters.backlog_recoveries);
        } else {
            assert_eq!(counters.backlog_recoveries, 0, "{counters:?}");
        }
    }
}

#[test]
fn graph_output_gap_overflow_and_abandoned_peer_have_bounded_explicit_recovery() {
    use moiren_engine::{
        boundary::RtAudioSink,
        buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
    };
    let (mut sink, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 16,
        target_fill_frames: 4,
        max_correction_ppm: 0.0,
        trim_on_prime: false,
        ..Default::default()
    })
    .unwrap();
    let mut arena = BufferArena::<f32>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 40,
        }],
        1024 * 1024,
    )
    .unwrap();
    let slot = arena.slot(0).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write { port: 0, slot }])
        .unwrap();
    let read = arena
        .prepare_io(&[PortAccess::Read { port: 0, slot }])
        .unwrap();
    arena
        .with_io(&write, 0..40, |io| {
            if let ProcessIo::Separate { mut outputs, .. } = io {
                for (_, mut block) in outputs.iter_mut() {
                    for channel in block.channels_mut() {
                        channel.fill(0.5);
                    }
                }
            }
        })
        .unwrap();
    let mut push = |sink: &mut moiren_windows_audio::clock_bridge::CaptureIngress, at, frames| {
        arena
            .with_io(&read, 0..frames, |io| match io {
                ProcessIo::ReadOnly { inputs } => sink.write(
                    &ProcessContext {
                        timeline_epoch: 1,
                        timeline_start: at,
                        frames,
                        processing_sr: 48000.0,
                    },
                    inputs.get(0).unwrap(),
                ),
                _ => unreachable!(),
            })
            .unwrap()
    };
    assert_eq!(push(&mut sink, 0, 8).transferred_frames, 8);
    source.read_interleaved(&mut [0.0; 8]).unwrap();
    assert!(push(&mut sink, 100, 8).discontinuity);
    let mut samples = [f32::NAN; 32];
    let report = source.read_interleaved(&mut samples).unwrap();
    assert!(report.discontinuity);
    assert!(samples.iter().all(|sample| sample.is_finite()));
    let overflow = push(&mut sink, 108, 40);
    assert!(overflow.discontinuity);
    assert!(overflow.transferred_frames <= 16);
    assert!(observer.snapshot().dropped_frames > 0);
    drop(sink);
    for _ in 0..4 {
        source.read_interleaved(&mut samples).unwrap();
    }
    let counters = observer.snapshot();
    assert!(observer.producer_finished());
    assert!(counters.resets > 0);
    assert!(counters.underrun_frames > 0);
    assert_eq!(counters.discontinuities, 1);
    assert_eq!(counters.timestamp_errors, 0);
    assert!(samples.iter().all(|sample| *sample == 0.0));
}

#[test]
fn managed_fanout_refuses_missing_or_duplicate_sink_guards_without_losing_engine() {
    for duplicate in [false, true] {
        let (renderer, _source, _observer) = graph(Default::default());
        let (engine, reader) = renderer.into_parts();
        let sinks: Vec<_> = engine.plan_snapshot().sink_processors().collect();
        let protected = if duplicate {
            vec![sinks[0], sinks[1], sinks[0]]
        } else {
            vec![sinks[0]]
        };
        let failure =
            match DemandRenderer::new_managed_outputs_returning(engine, reader, &protected, 1) {
                Err(failure) => failure,
                Ok(_) => panic!("incomplete native sink set accepted"),
            };
        assert_eq!(failure.engine.timeline(), 0);
        assert_eq!(failure.engine.plan_snapshot().sink_processors().count(), 2);
    }
}
