use super::allocation::track_allocations;
use moiren_core::{graph::*, protocol::*};
use moiren_engine::{
    boundary::{AudioReader, BoundaryReport, RtAudioSource, audio_bridge},
    buffer::AudioBlockMut,
    compiler::{CompileConfig, CompiledGraph, NodeBindings, compile},
    processor::{Gain, ProcessContext},
    runtime::{EngineConfig, PlanSwapError, PreparedPlan, ProcessorReuse, RetireOutcome},
    sample::ProcessingSample,
};

struct CounterSource {
    next: f64,
}
impl<S: ProcessingSample> RtAudioSource<S> for CounterSource {
    fn channel_count(&self) -> usize {
        1
    }
    fn read(&mut self, ctx: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        for sample in output.channel_mut(0) {
            *sample = S::from_f64(self.next);
            self.next += 1.0;
        }
        BoundaryReport {
            transferred_frames: ctx.frames,
            ..BoundaryReport::default()
        }
    }
}

fn compiled<S: ProcessingSample>(
    revision: u64,
    gain: f64,
    next: f64,
) -> (CompiledGraph<S>, AudioReader<S>) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let gain_node = graph.create_node(NodeKind::Gain, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    for (a, b) in [(source, gain_node), (gain_node, sink)] {
        graph
            .connect(
                graph.get_node(a).unwrap().outputs()[0].id(),
                graph.get_node(b).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
    }
    let (writer, reader) = audio_bridge(1, 64, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(source, CounterSource { next })
        .unwrap();
    bindings.bind_gain(gain_node, gain).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let result = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
            audio_byte_budget: 4096,
            plan_revision: revision,
            timeline_epoch: 3,
            control_capacity: 4,
            control_horizon_frames: 1024,
        },
    )
    .unwrap();
    (result, reader)
}

fn request(revision: u64, value: f64, ramp: u32, frame: u64) -> ParameterRequest {
    ParameterRequest {
        request_id: revision,
        plan_revision: revision,
        timeline_epoch: 3,
        target: ParameterKey {
            processor: ProcessorId(2),
            parameter: Gain::LEVEL,
        },
        at: ApplyAt::Frame(frame),
        value: ParamValue::Float(value),
        ramp_frames: ramp,
    }
}

fn samples<S: ProcessingSample>(reader: &mut AudioReader<S>, expected: &[f64]) {
    let mut output = vec![S::ZERO; expected.len()];
    assert_eq!(
        reader
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        expected.len()
    );
    for (actual, expected) in output.iter().zip(expected) {
        assert!(
            (actual.to_f64() - expected).abs() < 1e-6,
            "{} != {expected}",
            actual.to_f64()
        );
    }
}

fn boundary_swap<S: ProcessingSample>() {
    let (mut old, mut old_output) = compiled::<S>(7, 1.0, 1.0);
    let (new, mut new_output) = compiled::<S>(8, 0.5, 10.0);
    let mut plans = old.engine.enable_plan_switching(1).unwrap();
    old.engine.render(2).unwrap();
    plans
        .publish(PreparedPlan::new(new.engine).unwrap())
        .unwrap();
    assert_eq!(plans.active_revision(), 7);
    let (rendered, counts) = track_allocations(|| old.engine.render(2));
    assert_eq!(counts, (0, 0));
    let rendered = rendered.unwrap();
    assert_eq!((rendered.start, rendered.end), (2, 4));
    assert_eq!(old.engine.revision(), 8);
    assert_eq!(plans.active_revision(), 8);
    samples(&mut old_output, &[1.0, 2.0]);
    samples(&mut new_output, &[5.0, 5.5]);
    let retired = plans.poll_retired().unwrap();
    assert_eq!(retired.revision(), 7);
    assert_eq!(
        retired.outcome(),
        RetireOutcome::Replaced {
            active_revision: 8,
            frame: 2
        }
    );
}

#[test]
fn swap_occurs_at_block_boundary_preserves_timeline_without_rt_allocation() {
    boundary_swap::<f32>();
    boundary_swap::<f64>();
}

#[test]
fn full_retire_queue_defers_swap_and_full_pending_returns_candidate() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let (b, _) = compiled::<f64>(8, 1.0, 1.0);
    plans.publish(PreparedPlan::new(b.engine).unwrap()).unwrap();
    let (c, _) = compiled::<f64>(9, 1.0, 1.0);
    let failure = plans
        .publish(PreparedPlan::new(c.engine).unwrap())
        .unwrap_err();
    assert_eq!(failure.reason, PlanSwapError::PendingFull);
    let candidate = failure.plan;
    active.engine.render(1).unwrap(); // fills retire
    plans.publish(candidate).unwrap();
    let (result, counts) = track_allocations(|| active.engine.render(1));
    result.unwrap();
    assert_eq!(counts, (0, 0));
    assert_eq!(active.engine.revision(), 8);
    drop(plans.poll_retired().unwrap());
    active.engine.render(1).unwrap();
    assert_eq!(active.engine.revision(), 9);
    assert_eq!(plans.poll_retired().unwrap().revision(), 8);
}

#[test]
fn reuse_preserves_source_state_sink_identity_and_in_progress_gain_ramp() {
    let (mut old, mut output) = compiled::<f64>(7, 1.0, 1.0);
    let basis = old.engine.plan_snapshot();
    let mut plans = old.engine.enable_plan_switching(1).unwrap();
    assert_eq!(
        old.control.submit(request(7, 0.0, 4, 0), 0).code,
        ReplyCode::Accepted
    );
    old.engine.render(2).unwrap();
    let (new, mut unused_output) = compiled::<f64>(8, 8.0, 100.0);
    let candidate = PreparedPlan::new(new.engine)
        .unwrap()
        .with_reuse(
            &basis,
            &[
                ProcessorReuse {
                    old: ProcessorId(0),
                    new: ProcessorId(0),
                },
                ProcessorReuse {
                    old: ProcessorId(2),
                    new: ProcessorId(2),
                },
                ProcessorReuse {
                    old: ProcessorId(4),
                    new: ProcessorId(4),
                },
            ],
        )
        .unwrap();
    plans.publish(candidate).unwrap();
    let (rendered, counts) = track_allocations(|| old.engine.render(2));
    rendered.unwrap();
    assert_eq!(counts, (0, 0));
    samples(&mut output, &[0.75, 1.0, 0.75, 0.0]);
    let mut empty = [0.0];
    assert_eq!(
        unused_output
            .read_interleaved(&mut empty)
            .unwrap()
            .transferred_frames,
        0
    );
}

#[test]
fn old_parameters_are_rejected_and_accepted_future_events_are_retired() {
    let (mut old, _) = compiled::<f64>(7, 1.0, 1.0);
    let mut plans = old.engine.enable_plan_switching(1).unwrap();
    assert_eq!(
        old.control.submit(request(7, 0.0, 0, 100), 0).code,
        ReplyCode::Accepted
    );
    let (mut new, mut output) = compiled::<f64>(8, 0.5, 10.0);
    assert_eq!(
        new.control.submit(request(8, 0.25, 0, 0), 0).code,
        ReplyCode::Accepted
    );
    plans
        .publish(PreparedPlan::new(new.engine).unwrap())
        .unwrap();
    old.engine.render(1).unwrap();
    assert_eq!(
        old.control.submit(request(7, 0.0, 0, 101), 1).code,
        ReplyCode::StaleRevision
    );
    let mut retired = plans.poll_retired().unwrap();
    assert_eq!(retired.reject_pending(), 0);
    assert_eq!(
        old.control.poll_applied().unwrap().code,
        ReplyCode::StaleRevision
    );
    assert_eq!(new.control.poll_applied().unwrap().code, ReplyCode::Applied);
    samples(&mut output, &[2.5]);
}

#[test]
fn stale_reuse_basis_returns_candidate_to_control_without_replacing_active() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let basis = active.engine.plan_snapshot();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let (next, _) = compiled::<f64>(8, 1.0, 1.0);
    plans
        .publish(PreparedPlan::new(next.engine).unwrap())
        .unwrap();
    active.engine.render(1).unwrap();
    drop(plans.poll_retired().unwrap());
    let (later, _) = compiled::<f64>(9, 1.0, 1.0);
    let candidate = PreparedPlan::new(later.engine)
        .unwrap()
        .with_reuse(
            &basis,
            &[ProcessorReuse {
                old: ProcessorId(2),
                new: ProcessorId(2),
            }],
        )
        .unwrap();
    plans.publish(candidate).unwrap();
    let (rendered, counts) = track_allocations(|| active.engine.render(1));
    rendered.unwrap();
    assert_eq!(counts, (0, 0));
    assert_eq!(active.engine.revision(), 8);
    let retired = plans.poll_retired().unwrap();
    assert_eq!(retired.revision(), 9);
    assert_eq!(
        retired.outcome(),
        RetireOutcome::Rejected(PlanSwapError::StaleBasis)
    );
}

#[test]
fn invalid_render_keeps_pending_and_stale_revision_is_rejected_before_publish() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let (same, _) = compiled::<f64>(7, 1.0, 1.0);
    let failure = plans
        .publish(PreparedPlan::new(same.engine).unwrap())
        .unwrap_err();
    assert_eq!(failure.reason, PlanSwapError::StaleRevision);
    let (next, _) = compiled::<f64>(8, 1.0, 1.0);
    plans
        .publish(PreparedPlan::new(next.engine).unwrap())
        .unwrap();
    assert!(active.engine.render(0).is_err());
    assert_eq!(active.engine.revision(), 7);
    assert!(plans.poll_retired().is_none());
    active.engine.render(1).unwrap();
    assert_eq!(active.engine.revision(), 8);
}

#[test]
fn dropping_control_with_a_pending_plan_keeps_render_free_of_destruction() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let (next, _) = compiled::<f64>(8, 1.0, 1.0);
    plans
        .publish(PreparedPlan::new(next.engine).unwrap())
        .unwrap();
    drop(plans);
    let (rendered, counts) = track_allocations(|| active.engine.render(2));
    rendered.unwrap();
    assert_eq!(counts, (0, 0));
    assert_eq!(active.engine.revision(), 7);
    drop(active); // stopped owner cleans pending queues on control
}

#[test]
fn cancelling_pending_returns_its_ownership_without_advancing_active_revision() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let (mut next, _) = compiled::<f64>(8, 1.0, 1.0);
    assert_eq!(
        next.control.submit(request(8, 0.0, 0, 20), 0).code,
        ReplyCode::Accepted
    );
    plans
        .publish(PreparedPlan::new(next.engine).unwrap())
        .unwrap();
    plans.cancel_pending();
    let (rendered, counts) = track_allocations(|| active.engine.render(1));
    rendered.unwrap();
    assert_eq!(counts, (0, 0));
    assert_eq!(active.engine.revision(), 7);
    let mut retired = plans.poll_retired().unwrap();
    assert_eq!(retired.revision(), 8);
    assert_eq!(
        retired.outcome(),
        RetireOutcome::Rejected(PlanSwapError::Cancelled)
    );
    assert_eq!(retired.reject_pending(), 0);
    assert_eq!(
        next.control.poll_applied().unwrap().code,
        ReplyCode::StaleRevision
    );
}

#[test]
fn retired_parameter_rejection_respects_reply_backpressure() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    for _ in 0..4 {
        assert_eq!(
            active.control.submit(request(7, 1.0, 0, 0), 0).code,
            ReplyCode::Accepted
        );
    }
    active.engine.render(1).unwrap(); // fills reply queue
    assert_eq!(
        active.control.submit(request(7, 0.0, 0, 100), 1).code,
        ReplyCode::Accepted
    );
    let (next, _) = compiled::<f64>(8, 1.0, 1.0);
    plans
        .publish(PreparedPlan::new(next.engine).unwrap())
        .unwrap();
    active.engine.render(1).unwrap();
    let mut retired = plans.poll_retired().unwrap();
    assert_eq!(retired.reject_pending(), 1);
    for _ in 0..4 {
        assert_eq!(
            active.control.poll_applied().unwrap().code,
            ReplyCode::Applied
        );
    }
    assert_eq!(retired.reject_pending(), 0);
    assert_eq!(
        active.control.poll_applied().unwrap().code,
        ReplyCode::StaleRevision
    );
}

fn gain_engine(
    config: EngineConfig,
    revision: u64,
    epoch: u64,
    min: f64,
) -> moiren_engine::runtime::Engine<f64> {
    use moiren_engine::{
        buffer::{BufferArena, BufferSlotLayout, PortAccess},
        control::{ParamDomain, parameter_channel},
        runtime::*,
    };
    let resources = RtResources::new(vec![ProcessorInstance::new(ProcessorId(2), Gain)]).unwrap();
    let (_, parameters) = parameter_channel(
        vec![moiren_engine::control::ParamSpec {
            domain: ParamDomain::Float { min, max: 16.0 },
            ..Gain::parameter(ProcessorId(2), 1.0)
        }],
        revision,
        epoch,
        4,
        1024,
    )
    .unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: config.max_block_frames,
        }; 2],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[
            PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            },
            PortAccess::Write {
                port: 0,
                slot: arena.slot(1).unwrap(),
            },
        ])
        .unwrap();
    let plan = ExecutionPlan::prepare(
        arena,
        vec![OpSpec {
            processor: ProcessorId(2),
            io,
        }],
        &resources,
        &parameters,
        config,
    )
    .unwrap();
    Engine::new(plan, resources, parameters).unwrap()
}

#[test]
fn config_epoch_type_and_schema_mismatch_are_rejected_on_control_side() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let config = active.engine.config();
    let basis = active.engine.plan_snapshot();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let changed = gain_engine(
        EngineConfig {
            processing_sr: 44100.0,
            ..config
        },
        8,
        3,
        0.0,
    );
    assert_eq!(
        plans
            .publish(PreparedPlan::new(changed).unwrap())
            .unwrap_err()
            .reason,
        PlanSwapError::IncompatibleConfig
    );
    let changed = gain_engine(config, 8, 4, 0.0);
    assert_eq!(
        plans
            .publish(PreparedPlan::new(changed).unwrap())
            .unwrap_err()
            .reason,
        PlanSwapError::StaleEpoch
    );
    let (candidate, _) = compiled::<f64>(8, 1.0, 1.0);
    assert!(matches!(
        PreparedPlan::new(candidate.engine).unwrap().with_reuse(
            &basis,
            &[ProcessorReuse {
                old: ProcessorId(0),
                new: ProcessorId(2)
            }]
        ),
        Err(PlanSwapError::IncompatibleProcessor)
    ));
    let changed = gain_engine(config, 8, 3, 0.5);
    assert!(matches!(
        PreparedPlan::new(changed).unwrap().with_reuse(
            &basis,
            &[ProcessorReuse {
                old: ProcessorId(2),
                new: ProcessorId(2)
            }]
        ),
        Err(PlanSwapError::IncompatibleParameters)
    ));
    let (candidate, _) = compiled::<f64>(8, 1.0, 1.0);
    let reuse = ProcessorReuse {
        old: ProcessorId(2),
        new: ProcessorId(2),
    };
    assert!(matches!(
        PreparedPlan::new(candidate.engine)
            .unwrap()
            .with_reuse(&basis, &[reuse, reuse]),
        Err(PlanSwapError::DuplicateReuse)
    ));
    let (candidate, _) = compiled::<f64>(8, 1.0, 1.0);
    assert!(matches!(
        PreparedPlan::new(candidate.engine).unwrap().with_reuse(
            &basis,
            &[ProcessorReuse {
                old: ProcessorId(999),
                new: ProcessorId(2)
            }]
        ),
        Err(PlanSwapError::MissingProcessor)
    ));
    assert_eq!(active.engine.revision(), 7);
}

#[test]
fn rendered_retired_and_pending_processors_are_destroyed_on_the_control_thread() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };
    use std::thread;
    struct Audit {
        owner: thread::ThreadId,
        drops: AtomicUsize,
        wrong_thread: AtomicBool,
    }
    struct Probe(Arc<Audit>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            if thread::current().id() != self.0.owner {
                self.0.wrong_thread.store(true, Ordering::Relaxed);
            }
        }
    }
    impl RtAudioSource<f64> for Probe {
        fn channel_count(&self) -> usize {
            1
        }
        fn read(
            &mut self,
            ctx: &ProcessContext,
            _output: AudioBlockMut<'_, f64>,
        ) -> BoundaryReport {
            BoundaryReport {
                transferred_frames: ctx.frames,
                ..BoundaryReport::default()
            }
        }
    }
    fn prepare(revision: u64, audit: &Arc<Audit>) -> moiren_engine::runtime::Engine<f64> {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 1).unwrap();
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(source, Probe(Arc::clone(audit)))
            .unwrap();
        compile(
            &graph,
            bindings,
            CompileConfig {
                engine: EngineConfig {
                    processing_sr: 48000.0,
                    max_block_frames: 8,
                    max_events_per_block: 8,
                },
                audio_byte_budget: 4096,
                plan_revision: revision,
                timeline_epoch: 3,
                control_capacity: 4,
                control_horizon_frames: 1024,
            },
        )
        .unwrap()
        .engine
    }
    let audit = Arc::new(Audit {
        owner: thread::current().id(),
        drops: AtomicUsize::new(0),
        wrong_thread: AtomicBool::new(false),
    });
    let mut engine = prepare(7, &audit);
    let mut plans = engine.enable_plan_switching(1).unwrap();
    let (commands, receiver) = mpsc::channel();
    let (reports, observed) = mpsc::channel();
    let worker = thread::spawn(move || {
        while receiver.recv().unwrap() {
            let (result, counts) = track_allocations(|| engine.render(1));
            reports.send((result.unwrap(), counts)).unwrap();
        }
        engine // callback stopped; ownership, including queues, returns to control
    });
    plans
        .publish(PreparedPlan::new(prepare(8, &audit)).unwrap())
        .unwrap();
    commands.send(true).unwrap();
    assert_eq!(observed.recv().unwrap().1, (0, 0));
    assert_eq!(audit.drops.load(Ordering::Relaxed), 0);
    drop(plans.poll_retired().unwrap());
    assert_eq!(audit.drops.load(Ordering::Relaxed), 1);
    plans
        .publish(PreparedPlan::new(prepare(9, &audit)).unwrap())
        .unwrap();
    drop(plans); // closes control while candidate is still pending
    commands.send(true).unwrap();
    assert_eq!(observed.recv().unwrap().1, (0, 0));
    assert_eq!(audit.drops.load(Ordering::Relaxed), 1);
    commands.send(false).unwrap();
    let engine = worker.join().unwrap();
    assert_eq!(engine.revision(), 8);
    drop(engine);
    assert_eq!(audit.drops.load(Ordering::Relaxed), 3);
    assert!(!audit.wrong_thread.load(Ordering::Relaxed));
}

#[test]
fn stopped_parameter_and_plan_channels_reject_new_submissions() {
    let (mut active, _) = compiled::<f64>(7, 1.0, 1.0);
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    drop(active.engine); // stopped render owner returns and releases resources
    assert_eq!(
        active.control.submit(request(7, 0.0, 0, 0), 0).code,
        ReplyCode::StaleRevision
    );
    let (next, _) = compiled::<f64>(8, 1.0, 1.0);
    assert_eq!(
        plans
            .publish(PreparedPlan::new(next.engine).unwrap())
            .unwrap_err()
            .reason,
        PlanSwapError::Disconnected
    );
}

#[test]
fn compressor_envelope_survives_plan_swap_and_keeps_releasing_in_both_precisions() {
    use moiren_engine::{
        boundary::ConstantSource,
        processor::{Compressor, CompressorSettings},
    };
    fn check<S: ProcessingSample>() {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 1).unwrap();
        let compressor = graph.create_node(NodeKind::Compressor, 1).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
        for (from, to) in [(source, compressor), (compressor, sink)] {
            graph
                .connect(
                    graph.get_node(from).unwrap().outputs()[0].id(),
                    graph.get_node(to).unwrap().inputs()[0].id(),
                    SendParams::default(),
                )
                .unwrap();
        }
        let prepare = |revision, value| {
            let (writer, reader) = audio_bridge::<S>(1, 32, 4096).unwrap();
            let mut io = NodeBindings::new();
            io.bind_source(source, ConstantSource { channels: 1, value })
                .unwrap();
            io.bind_sink(sink, writer).unwrap();
            io.bind_compressor(
                compressor,
                CompressorSettings {
                    threshold_db: -12.0,
                    ratio: 4.0,
                    attack_ms: 0.0,
                    release_ms: 1.0,
                    knee_db: 0.0,
                    ..CompressorSettings::default()
                },
            )
            .unwrap();
            (
                compile(
                    &graph,
                    io,
                    CompileConfig {
                        engine: EngineConfig {
                            processing_sr: 48000.0,
                            max_block_frames: 8,
                            max_events_per_block: 8,
                        },
                        audio_byte_budget: 4096,
                        plan_revision: revision,
                        timeline_epoch: 3,
                        control_capacity: 4,
                        control_horizon_frames: 1024,
                    },
                )
                .unwrap(),
                reader,
            )
        };
        let (mut active, mut output) = prepare(7, 1.0);
        let basis = active.engine.plan_snapshot();
        let mut plans = active.engine.enable_plan_switching(1).unwrap();
        active.engine.render(1).unwrap();
        samples(&mut output, &[10.0_f64.powf(-9.0 / 20.0)]);
        let (mut next, _) = prepare(8, 0.1);
        let new_compressor = next.bindings.node(compressor).unwrap();
        // Candidate automation acts on the transferred envelope at activation.
        assert_eq!(
            next.control
                .submit(
                    ParameterRequest {
                        target: ParameterKey {
                            processor: new_compressor,
                            parameter: Compressor::MAKEUP_GAIN
                        },
                        value: ParamValue::Float(6.0),
                        at: ApplyAt::Frame(1),
                        ..request(8, 6.0, 0, 1)
                    },
                    1
                )
                .code,
            ReplyCode::Accepted
        );
        let reuse: Vec<_> = [compressor, sink]
            .map(|node| ProcessorReuse {
                old: active.bindings.node(node).unwrap(),
                new: next.bindings.node(node).unwrap(),
            })
            .into();
        let candidate = PreparedPlan::new(next.engine)
            .unwrap()
            .with_reuse(&basis, &reuse)
            .unwrap();
        plans.publish(candidate).unwrap();
        let (result, counts) = track_allocations(|| active.engine.render(4));
        result.unwrap();
        assert_eq!(counts, (0, 0));
        let expected: Vec<_> = (1..=4)
            .map(|step| {
                let reduction = 9.0 * (-(step as f64) / 48.0).exp();
                0.1 * 10.0_f64.powf((6.0 - reduction) / 20.0)
            })
            .collect();
        samples(&mut output, &expected);
        assert_eq!(
            next.control.poll_applied().unwrap().code,
            ReplyCode::Applied
        );
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn reusing_a_registry_in_a_new_engine_does_not_make_an_old_snapshot_current() {
    use moiren_engine::{
        buffer::{BufferArena, BufferSlotLayout, PortAccess},
        control::parameter_channel,
        runtime::*,
    };
    let config = EngineConfig {
        processing_sr: 48000.0,
        max_block_frames: 8,
        max_events_per_block: 8,
    };
    let original = gain_engine(config, 7, 3, 0.0);
    let obsolete = original.plan_snapshot();
    let (_, resources, _) = original.into_parts();
    // The same registry now runs against a different parameter table/order.
    let (_, parameters) = parameter_channel(
        vec![
            Gain::parameter(ProcessorId(999), 8.0),
            Gain::parameter(ProcessorId(2), 0.5),
        ],
        8,
        3,
        4,
        1024,
    )
    .unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 8,
        }; 2],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[
            PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            },
            PortAccess::Write {
                port: 0,
                slot: arena.slot(1).unwrap(),
            },
        ])
        .unwrap();
    let plan = ExecutionPlan::prepare(
        arena,
        vec![OpSpec {
            processor: ProcessorId(2),
            io,
        }],
        &resources,
        &parameters,
        config,
    )
    .unwrap();
    let mut active = Engine::new(plan, resources, parameters).unwrap();
    let mut plans = active.enable_plan_switching(1).unwrap();
    let candidate = PreparedPlan::new(gain_engine(config, 9, 3, 0.0))
        .unwrap()
        .with_reuse(
            &obsolete,
            &[ProcessorReuse {
                old: ProcessorId(2),
                new: ProcessorId(2),
            }],
        )
        .unwrap();
    plans.publish(candidate).unwrap();
    let (result, counts) = track_allocations(|| active.render(1));
    result.unwrap();
    assert_eq!(counts, (0, 0));
    assert_eq!(active.revision(), 8);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::StaleBasis)
    );
}

#[test]
fn in_place_compressor_automation_renders_without_rt_allocations_in_both_precisions() {
    use moiren_engine::{
        boundary::{ConstantSource, SinkAdapter, SourceAdapter},
        buffer::{BufferArena, BufferSlotLayout, PortAccess},
        control::parameter_channel,
        processor::{Compressor, CompressorSettings},
        runtime::*,
    };
    fn check<S: ProcessingSample>() {
        let (writer, mut output) = audio_bridge::<S>(1, 8, 4096).unwrap();
        let resources = RtResources::new(vec![
            ProcessorInstance::new(
                ProcessorId(0),
                SourceAdapter(ConstantSource {
                    channels: 1,
                    value: 1.0,
                }),
            ),
            ProcessorInstance::new(ProcessorId(2), Compressor::new()),
            ProcessorInstance::new(ProcessorId(4), SinkAdapter(writer)),
        ])
        .unwrap();
        let (mut control, parameters) = parameter_channel(
            Compressor::parameters(
                ProcessorId(2),
                CompressorSettings {
                    threshold_db: -12.0,
                    ratio: 4.0,
                    attack_ms: 0.0,
                    release_ms: 0.0,
                    knee_db: 0.0,
                    ..CompressorSettings::default()
                },
            )
            .to_vec(),
            7,
            3,
            4,
            1024,
        )
        .unwrap();
        for (parameter, frame, ramp) in [(Compressor::THRESHOLD, 2, 4), (Compressor::MIX, 5, 0)] {
            assert_eq!(
                control
                    .submit(
                        ParameterRequest {
                            target: ParameterKey {
                                processor: ProcessorId(2),
                                parameter
                            },
                            ..request(7, 0.0, ramp, frame)
                        },
                        0
                    )
                    .code,
                ReplyCode::Accepted
            );
        }
        let arena = BufferArena::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 8,
            }],
            4096,
        )
        .unwrap();
        let slot = arena.slot(0).unwrap();
        let operations = vec![
            OpSpec {
                processor: ProcessorId(0),
                io: arena
                    .prepare_io(&[PortAccess::Write { port: 0, slot }])
                    .unwrap(),
            },
            OpSpec {
                processor: ProcessorId(2),
                io: arena
                    .prepare_io(&[PortAccess::InPlace {
                        input: 0,
                        output: 0,
                        slot,
                    }])
                    .unwrap(),
            },
            OpSpec {
                processor: ProcessorId(4),
                io: arena
                    .prepare_io(&[PortAccess::Read { port: 0, slot }])
                    .unwrap(),
            },
        ];
        let plan = ExecutionPlan::prepare(
            arena,
            operations,
            &resources,
            &parameters,
            EngineConfig {
                processing_sr: 48000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
        )
        .unwrap();
        let mut engine = Engine::new(plan, resources, parameters).unwrap();
        let (results, counts) = track_allocations(|| [engine.render(3), engine.render(5)]);
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(counts, (0, 0));
        let expected: Vec<_> = [-9.0, -9.0, -6.75, -4.5, -2.25, 0.0, 0.0, 0.0]
            .into_iter()
            .map(|db| 10.0_f64.powf(db / 20.0))
            .collect();
        samples(&mut output, &expected);
    }
    check::<f32>();
    check::<f64>();
}
