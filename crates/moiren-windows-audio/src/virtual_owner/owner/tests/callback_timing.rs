use super::*;
use crate::virtual_owner::callback_timing::CallbackTiming;

#[test]
fn prepared_callback_helper_records_one_real_engine_call() {
    let (mut engine, _reader) = callback_engine::<f32>();
    let probe = VirtualClockProbe::new(48_000, 8, 8).with_callback_timing(true);
    let callback = probe.callback_probe().unwrap();
    CallbackTiming::default()
        .render(&mut engine, 8, Some(callback), &mut || callback.now())
        .unwrap();
    assert_eq!(probe.callback_snapshot().unwrap().completed, 1);
}

fn callback_engine<S: ProcessingSample>() -> (Engine<S>, AudioReader<f32>) {
    callback_engine_mode(false, false)
}

struct PanickingSource;
impl<S: ProcessingSample> RtAudioSource<S> for PanickingSource {
    fn channel_count(&self) -> usize {
        1
    }
    fn read(
        &mut self,
        _: &moiren_engine::processor::ProcessContext,
        _: moiren_engine::buffer::AudioBlockMut<'_, S>,
    ) -> moiren_engine::boundary::BoundaryReport {
        panic!("callback panic fixture")
    }
}

fn callback_engine_mode<S: ProcessingSample>(
    missing_host: bool,
    panic: bool,
) -> (Engine<S>, AudioReader<f32>) {
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
    if panic {
        bindings.bind_source(source, PanickingSource).unwrap();
    } else {
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 1,
                    value: 0.25,
                },
            )
            .unwrap();
    }
    bindings
        .bind_sink(
            sink,
            VirtualMicrophoneSink::new(writer, 1, 8, 48_000, 4096).unwrap(),
        )
        .unwrap();
    let config = CompileConfig {
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
    };
    let mut compiled = if missing_host {
        let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = bus.create_host(8).unwrap();
        moiren_engine::compiler::compile_with_signals(&graph, bindings, config, &host)
    } else {
        compile(&graph, bindings, config)
    }
    .unwrap();
    drop(compiled.engine.enable_plan_switching(1).unwrap());
    (compiled.engine, reader)
}

fn actual_owner_callback_evidence<S: ProcessingSample>() {
    let (mut engine, _reader) = callback_engine::<S>();
    let published = Published::new(&engine);
    let probe = VirtualClockProbe::new(48_000, 8, super::super::super::DEBT_CAP_PERIODS)
        .with_callback_timing(true);
    let mut clock = FakeClock {
        now: 0,
        wakes: 0,
        limit: 2,
        stall_after_first_render: false,
        reads_after_wait: 0,
    };
    run_with_recovery_cap(
        &mut engine,
        &mut clock,
        &published,
        &[],
        &StopSignal::new().unwrap(),
        &mut ClockTrace::default(),
        super::super::super::DEBT_CAP_PERIODS,
        &probe,
    )
    .unwrap();
    let sample = probe
        .callback_snapshot()
        .expect("actual owner must publish callback evidence");
    assert_eq!(sample.attempts, 2);
    assert_eq!(sample.completed, 2);
    let latest = sample.latest.unwrap();
    assert_eq!(latest.attempt_sequence, 2);
    assert_eq!(latest.epoch_before, 3);
    assert_eq!(latest.epoch_after, Some(3));
    assert_eq!(latest.frame_before, 8);
    assert_eq!(latest.frame_after, Some(16));
    assert_eq!(latest.requested_frames, 8);
    assert_eq!(engine.timeline(), 16);
    assert_eq!(clock.wakes, 2);
}

#[test]
fn actual_virtual_callback_records_engine_boundaries_in_both_precisions() {
    actual_owner_callback_evidence::<f32>();
    actual_owner_callback_evidence::<f64>();
}

fn actual_owner_script<S: ProcessingSample>(enabled: bool) {
    let (mut engine, _reader) = callback_engine::<S>();
    let published = Published::new(&engine);
    let probe = VirtualClockProbe::new(48_000, 8, 8).with_callback_timing(enabled);
    let stop = StopSignal::new().unwrap();
    let mut clock = FakeClock {
        now: 0,
        wakes: 0,
        limit: 4,
        stall_after_first_render: false,
        reads_after_wait: 0,
    };
    let stamps = [
        Some(100),
        Some(120),
        None,
        Some(140),
        Some(160),
        Some(150),
        Some(200),
        Some(205),
    ];
    let mut calls = 0;
    let operation = || {
        run_with_callback_clock(
            &mut engine,
            &mut clock,
            &published,
            &[],
            &stop,
            &mut ClockTrace::default(),
            8,
            &probe,
            &mut || {
                let value = stamps[calls];
                calls += 1;
                value
            },
        )
    };
    let result = crate::test_allocation::assert_allocates_nothing(operation);
    result.unwrap();
    assert_eq!(engine.timeline(), 32);
    assert_eq!(clock.wakes, 4);
    if enabled {
        let snapshot = probe.callback_snapshot().unwrap();
        assert_eq!(
            (snapshot.attempts, snapshot.completed, snapshot.errors),
            (4, 4, 0)
        );
        assert_eq!(
            (
                snapshot.valid_timings,
                snapshot.missing_clocks,
                snapshot.regressed_clocks
            ),
            (2, 1, 1)
        );
        assert_eq!(snapshot.max_duration_100ns, Some(20));
        assert_eq!(snapshot.latest.unwrap().duration_100ns, Some(5));
        assert_eq!(calls, 8);
    } else {
        assert!(probe.callback_snapshot().is_none());
        assert_eq!(calls, 0);
    }
}

#[test]
fn actual_virtual_callback_disabled_preserves_pacing_without_clock_reads_or_heap_work() {
    actual_owner_script::<f32>(false);
    actual_owner_script::<f64>(false);
}

#[test]
fn actual_virtual_callback_invalid_clocks_preserve_audio_and_cumulative_max_without_heap_work() {
    actual_owner_script::<f32>(true);
    actual_owner_script::<f64>(true);
}

#[test]
fn actual_virtual_callback_engine_error_publishes_before_owner_returns_in_both_precisions() {
    actual_owner_error::<f32>();
    actual_owner_error::<f64>();
}

fn actual_owner_error<S: ProcessingSample>() {
    let (mut engine, _reader) = callback_engine_mode::<S>(true, false);
    let published = Published::new(&engine);
    let probe = VirtualClockProbe::new(48_000, 8, 8).with_callback_timing(true);
    let mut clock = FakeClock {
        now: 0,
        wakes: 0,
        limit: 1,
        stall_after_first_render: false,
        reads_after_wait: 0,
    };
    let mut reads = 0;
    let result = run_with_callback_clock(
        &mut engine,
        &mut clock,
        &published,
        &[],
        &StopSignal::new().unwrap(),
        &mut ClockTrace::default(),
        8,
        &probe,
        &mut || {
            reads += 1;
            Some(100)
        },
    );
    assert!(matches!(result, Err(BackendError::Failed(_))));
    assert_eq!(reads, 2);
    let snapshot = probe.callback_snapshot().unwrap();
    assert_eq!(
        (snapshot.attempts, snapshot.completed, snapshot.errors),
        (1, 1, 1)
    );
    let latest = snapshot.latest.unwrap();
    assert_eq!(
        latest.outcome,
        crate::virtual_owner::VirtualCallbackOutcome::Error
    );
    assert_eq!(latest.requested_frames, 8);
    assert_eq!(latest.frame_after, Some(0));
    assert_eq!(engine.timeline(), 0);
}

#[test]
fn actual_virtual_callback_panic_retains_unfinished_without_inventing_completion() {
    let (mut engine, _reader) = callback_engine_mode::<f32>(false, true);
    let published = Published::new(&engine);
    let probe = VirtualClockProbe::new(48_000, 8, 8).with_callback_timing(true);
    let mut clock = FakeClock {
        now: 0,
        wakes: 0,
        limit: 1,
        stall_after_first_render: false,
        reads_after_wait: 0,
    };
    let mut reads = 0;
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_with_callback_clock(
            &mut engine,
            &mut clock,
            &published,
            &[],
            &StopSignal::new().unwrap(),
            &mut ClockTrace::default(),
            8,
            &probe,
            &mut || {
                reads += 1;
                Some(100)
            },
        )
    }));
    assert!(result.is_err());
    assert_eq!(reads, 1);
    let sample = probe.callback_snapshot().unwrap();
    assert_eq!(
        (
            sample.attempts,
            sample.completed,
            sample.errors,
            sample.valid_timings
        ),
        (1, 0, 0, 0)
    );
    let latest = sample.latest.unwrap();
    assert_eq!(
        latest.outcome,
        crate::virtual_owner::VirtualCallbackOutcome::Unfinished
    );
    assert_eq!(
        (
            latest.epoch_before,
            latest.frame_before,
            latest.requested_frames
        ),
        (3, 0, 8)
    );
    assert_eq!(
        (
            latest.epoch_after,
            latest.frame_after,
            latest.qpc_after_100ns,
            latest.duration_100ns
        ),
        (None, None, None, None)
    );
}
