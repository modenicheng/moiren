use super::*;
use crate::virtual_owner::{VirtualMicrophoneSink, VirtualPlaybackSource};
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::{AudioReader, ConstantSource, RtAudioSource, audio_bridge},
    compiler::{CompileConfig, NodeBindings, compile},
    runtime::EngineConfig,
};

struct FakeClock {
    now: u64,
    wakes: usize,
    limit: usize,
    stall_after_first_render: bool,
    reads_after_wait: usize,
}
impl VirtualRealtimeClock for FakeClock {
    fn frequency(&self) -> u64 {
        48_000
    }
    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        self.reads_after_wait += 1;
        if self.stall_after_first_render && self.wakes == 1 && self.reads_after_wait == 2 {
            self.now += 48_000;
        }
        Ok(self.now)
    }
    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
        assert!(
            (1..=8).contains(&ticks),
            "wait after a stall stays in the future"
        );
        if self.wakes == self.limit {
            return Ok(ClockWake::Stopped);
        }
        self.wakes += 1;
        self.now += ticks;
        self.reads_after_wait = 0;
        Ok(ClockWake::Timer)
    }
}

fn engine() -> (Engine<f32>, CompiledBindings, NodeId, AudioReader<f32>) {
    engine_with_source(ConstantSource {
        channels: 1,
        value: 0.25,
    })
}

fn engine_with_source(
    source_boundary: impl RtAudioSource<f32> + 'static,
) -> (Engine<f32>, CompiledBindings, NodeId, AudioReader<f32>) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let (writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings.bind_source(source, source_boundary).unwrap();
    bindings
        .bind_sink(
            sink,
            VirtualMicrophoneSink::new(writer, 1, 8, 48_000, 4096).unwrap(),
        )
        .unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: 1,
            timeline_epoch: 3,
            control_capacity: 8,
            control_horizon_frames: 48_000,
        },
    )
    .unwrap();
    let plans = compiled.engine.enable_plan_switching(1).unwrap();
    drop(plans);
    (compiled.engine, compiled.bindings, sink, reader)
}

struct LateWakeClock(FakeClock);

impl VirtualRealtimeClock for LateWakeClock {
    fn frequency(&self) -> u64 {
        self.0.frequency()
    }
    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        self.0.now_ticks()
    }
    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
        let wake = self.0.wait_ticks(ticks)?;
        if wake == ClockWake::Timer && self.0.wakes == 1 {
            self.0.now += 48_000;
        }
        Ok(wake)
    }
}

#[test]
fn late_clock_wake_discards_old_playback_before_rendering_one_current_block() {
    let reset = Arc::new(AtomicU64::new(0));
    let (mut writer, reader) = audio_bridge(2, 8, 4096).unwrap();
    writer.write_interleaved(&[0.75; 16]).unwrap();
    let source = VirtualPlaybackSource::new(reader, 1, 8, 48_000, 4096)
        .unwrap()
        .with_reset_epoch(reset.clone());
    let (mut engine, _, _, mut microphone) = engine_with_source(source);
    let published = Published::new(&engine);
    let mut clock = LateWakeClock(FakeClock {
        now: 0,
        wakes: 0,
        limit: 1,
        stall_after_first_render: false,
        reads_after_wait: 0,
    });
    let mut trace = ClockTrace::default();
    run(
        &mut engine,
        &mut clock,
        &published,
        std::slice::from_ref(&reset),
        &StopSignal::new().unwrap(),
        &mut trace,
    )
    .unwrap();
    assert_eq!(
        trace.observed().collect::<Vec<_>>(),
        [(48_008, 0, 6000, GapStage::Wake)]
    );
    assert_eq!(reset.load(Ordering::Acquire), 1);
    assert_eq!(engine.timeline(), 8);
    let mut samples = [f32::NAN; 16];
    assert_eq!(
        microphone
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames,
        8
    );
    assert_eq!(samples, [0.0; 16]);
}

#[test]
fn zero_microphone_clients_still_execute_the_single_engine_after_clock_stall() {
    for stall in [false, true] {
        let (mut engine, _, _, _unconsumed_microphone) = engine();
        let published = Published::new(&engine);
        let mut clock = FakeClock {
            now: 0,
            wakes: 0,
            limit: 5,
            stall_after_first_render: stall,
            reads_after_wait: 0,
        };
        let reset = Arc::new(AtomicU64::new(0));
        let mut trace = ClockTrace::default();
        run(
            &mut engine,
            &mut clock,
            &published,
            std::slice::from_ref(&reset),
            &StopSignal::new().unwrap(),
            &mut trace,
        )
        .unwrap();
        if stall {
            assert_eq!(
                trace.observed().collect::<Vec<_>>(),
                [(48_008, 8, 6000, GapStage::AfterRender)]
            );
        } else {
            assert_eq!(trace.observed().count(), 0);
        }
        assert_eq!(clock.wakes, 5);
        assert_eq!(engine.timeline(), 40);
        assert_eq!(published.snapshot().unwrap().frame, 40);
        assert_eq!(reset.load(Ordering::Acquire), u64::from(stall));
    }
}

struct OverloadedClock {
    now: u64,
    reads: usize,
    stop: StopSignal,
}

impl VirtualRealtimeClock for OverloadedClock {
    fn frequency(&self) -> u64 {
        48_000
    }
    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        self.reads += 1;
        if self.reads == 20 {
            self.stop.request_stop().unwrap();
        }
        if self.reads > 22 {
            return Err(BackendError::Failed("stop was not observed".into()));
        }
        self.now += 48_000;
        Ok(self.now)
    }
    fn wait_ticks(&mut self, _: u64) -> Result<ClockWake, BackendError> {
        panic!("an overloaded owner never reaches its timer wait")
    }
}

#[test]
fn persistently_late_owner_observes_stop_without_waiting_for_clock_headroom() {
    let (mut engine, _, _, _microphone) = engine();
    let published = Published::new(&engine);
    let stop = StopSignal::new().unwrap();
    let mut clock = OverloadedClock {
        now: 0,
        reads: 0,
        stop: stop.clone(),
    };
    run(
        &mut engine,
        &mut clock,
        &published,
        &[],
        &stop,
        &mut ClockTrace::default(),
    )
    .unwrap();
    assert_eq!(clock.reads, 21);
    assert_eq!(engine.timeline(), 80);
}

#[test]
fn prepared_virtual_owner_waits_for_activation_and_returns_engine_to_control() {
    let (engine, bindings, sink, _microphone) = engine();
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let transport = Box::new(VirtualPreparedTransport::new(
        vec![sink],
        VirtualSessions::default(),
        stop,
        gate,
        Box::new(FakeClock {
            now: 0,
            wakes: 0,
            limit: 5,
            stall_after_first_render: false,
            reads_after_wait: 0,
        }),
    ));
    let owner = transport
        .attach(engine, &bindings, CancellationToken::default())
        .unwrap_or_else(|_| panic!("virtual attachment failed"));
    assert_eq!(owner.status(), OwnerStatus::Starting);
    assert_eq!(owner.timeline().unwrap().frame, 0);
    owner.activate().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if owner
            .timeline()
            .is_some_and(|timeline| timeline.frame == 40)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "virtual owner did not execute the bounded fake schedule"
        );
        std::thread::yield_now();
    }
    let returned = owner.stop_and_join();
    assert!(returned.error.is_none());
    assert_eq!(returned.engine.timeline(), 40);
}

#[test]
fn cancellation_before_activation_does_not_execute_a_graph_block() {
    let (engine, bindings, sink, _microphone) = engine();
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let cancel = CancellationToken::default();
    let transport = Box::new(VirtualPreparedTransport::new(
        vec![sink],
        VirtualSessions::default(),
        stop,
        gate,
        Box::new(FakeClock {
            now: 0,
            wakes: 0,
            limit: 5,
            stall_after_first_render: false,
            reads_after_wait: 0,
        }),
    ));
    let owner = transport
        .attach(engine, &bindings, cancel.clone())
        .unwrap_or_else(|_| panic!("virtual attachment failed"));
    cancel.cancel();
    assert_eq!(owner.activate(), Err(BackendError::Cancelled));
    assert_eq!(owner.stop_and_join().engine.timeline(), 0);
}
