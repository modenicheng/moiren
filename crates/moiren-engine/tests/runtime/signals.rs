use super::allocation::track_allocations;
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_core::protocol::ProcessorId;
use moiren_engine::runtime::{
    Engine, ExecutionPlan, OpSpec, PlanSwapError, PreparedPlan, ProcessorInstance, ProcessorReuse,
    RtResources,
};
use moiren_engine::{
    boundary::{AudioReader, ConstantSource, audio_bridge},
    buffer::{IoMode, PreparedIo, ProcessIo, ReadPorts},
    compiler::{CompileConfig, CompiledGraph, NodeBindings, compile, compile_with_signals},
    control::ProcessParameters,
    processor::{ProcessContext, ProcessorError, ProcessorRole, RtObserver, RtProcessor},
    runtime::{RuntimeError, SignalAttachError},
    signal::ProcessSignals,
};
use moiren_signal::{
    ActivateError, BlockInit, Bus, ErasedBinding, HostControl, LatestBlock, LatestValue,
    PreparedPublisher, PublishError, PublisherHandle, PumpBudget, StreamValue, Topic,
};

fn c3_render(engine: &mut Engine<f64>, frames: usize) {
    let (result, counts) = super::allocation::track_heap(|| engine.render(frames));
    result.unwrap();
    assert_eq!(counts, (0, 0, 0));
}

fn swap_source(
    host: &HostControl,
    latest: &Topic<LatestValue<u64>>,
    stream: &Topic<StreamValue<u64>>,
    revision: u64,
    id: u64,
    publish: bool,
) -> Engine<f64> {
    let key = host.new_binding().unwrap();
    let latest = host.prepare(latest, key).unwrap();
    let stream = host.prepare(stream, key).unwrap();
    source_engine(
        Some(host),
        revision,
        id,
        SignalSource {
            latest: latest.handle(),
            stream: stream.handle(),
            bindings: [latest.erase(), stream.erase()],
            calls: 0,
            publish,
        },
    )
}

fn source_engine(
    host: Option<&HostControl>,
    revision: u64,
    id: u64,
    source: impl RtProcessor<f64> + 'static,
) -> Engine<f64> {
    use moiren_engine::{
        buffer::{BufferArena, BufferSlotLayout, PortAccess},
        control::parameter_channel,
    };
    let resources =
        RtResources::new(vec![ProcessorInstance::new(ProcessorId(id), source)]).unwrap();
    let (_, parameters) = parameter_channel(vec![], revision, 3, 16, 1024).unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    let specs = vec![OpSpec {
        processor: ProcessorId(id),
        io,
    }];
    let plan = match host {
        Some(host) => ExecutionPlan::prepare_with_signals(
            arena,
            specs,
            &resources,
            &parameters,
            config(revision).engine,
            host,
        ),
        None => ExecutionPlan::prepare(
            arena,
            specs,
            &resources,
            &parameters,
            config(revision).engine,
        ),
    }
    .unwrap();
    Engine::new(plan, resources, parameters).unwrap()
}

#[test]
fn c3_replacement_preserves_latest_and_queued_stream_without_resubscribing() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("swap/latest", 0_u64).unwrap();
    let stream = bus.register_stream_value("swap/stream", 0_u64, 8).unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut latest_reader = bus.subscribe(&latest, Default::default()).unwrap();
    let mut witness = bus.subscribe(&latest, Default::default()).unwrap();
    let mut stream_reader = bus.subscribe(&stream, Default::default()).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let mut plans = engine.enable_plan_switching(4).unwrap();
    c3_render(&mut engine, 1);
    pump(&mut bus);
    let before = witness.read_latest().unwrap();
    assert_eq!(before.value, 1);
    let generation = before.meta.activation_generation;
    let first_stream = stream_reader.next().unwrap();
    assert_eq!((first_stream.meta.sequence, first_stream.value), (1, 1));
    c3_render(&mut engine, 1);
    c3_render(&mut engine, 1);
    assert_eq!(stream.stats().attempts, 3);
    assert!(stream_reader.next().is_none());
    plans
        .publish(PreparedPlan::new(swap_source(&control, &latest, &stream, 2, 20, false)).unwrap())
        .unwrap();
    c3_render(&mut engine, 1);
    pump(&mut bus);
    let unchanged = latest_reader.read_latest().unwrap();
    assert_eq!(unchanged.value, 3);
    assert_eq!(unchanged.meta.sequence, 3);
    assert_eq!(unchanged.meta.activation_generation, generation);
    assert_eq!(unchanged.meta.len, before.meta.len);
    plans
        .publish(PreparedPlan::new(swap_source(&control, &latest, &stream, 3, 30, true)).unwrap())
        .unwrap();
    c3_render(&mut engine, 1);
    pump(&mut bus);
    let after = latest_reader.read_latest().unwrap();
    assert_eq!(after.value, 1);
    assert_eq!(after.meta.sequence, 4);
    assert_eq!(after.meta.activation_generation, generation);
    for (sequence, value) in [(2, 2), (3, 3), (4, 1)] {
        let read = stream_reader.next().unwrap();
        assert_eq!((read.meta.sequence, read.value), (sequence, value));
        assert_eq!(read.meta.activation_generation, generation);
    }
    assert!(stream_reader.next().is_none());
}

#[test]
fn c3_reused_processor_publishes_with_its_retained_handle() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("reuse/latest", 0_u64).unwrap();
    let stream = bus.register_stream_value("reuse/stream", 0_u64, 8).unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut reader = bus.subscribe(&latest, Default::default()).unwrap();
    let _stream_reader = bus.subscribe(&stream, Default::default()).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let basis = engine.plan_snapshot();
    let mut plans = engine.enable_plan_switching(2).unwrap();
    c3_render(&mut engine, 1);
    let candidate = PreparedPlan::new(swap_source(&control, &latest, &stream, 2, 20, true))
        .unwrap()
        .with_reuse(
            &basis,
            &[ProcessorReuse {
                old: ProcessorId(10),
                new: ProcessorId(20),
            }],
        )
        .unwrap();
    plans.publish(candidate).unwrap();
    c3_render(&mut engine, 1);
    pump(&mut bus);
    let read = reader.read_latest().unwrap();
    assert_eq!((read.value, read.meta.sequence), (2, 2));
    assert_eq!(engine.revision(), 2);
}

#[test]
fn c3_replacing_reuse_mappings_restores_disposable_processor_bindings() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus
        .register_latest_value("reuse-reset/latest", 0_u64)
        .unwrap();
    let stream = bus
        .register_stream_value("reuse-reset/stream", 0_u64, 8)
        .unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut reader = bus.subscribe(&latest, Default::default()).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let basis = engine.plan_snapshot();
    let mut plans = engine.enable_plan_switching(1).unwrap();
    c3_render(&mut engine, 1);
    let candidate = PreparedPlan::new(swap_source(&control, &latest, &stream, 2, 20, true))
        .unwrap()
        .with_reuse(
            &basis,
            &[ProcessorReuse {
                old: ProcessorId(10),
                new: ProcessorId(20),
            }],
        )
        .unwrap()
        .with_reuse(&basis, &[])
        .unwrap();
    plans.publish(candidate).unwrap();
    c3_render(&mut engine, 1);
    pump(&mut bus);
    let read = reader.read_latest().unwrap();
    assert_eq!((read.value, read.meta.sequence), (1, 2));
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MeterHeader {
    timeline_epoch: u64,
    frame_start: u64,
    frame_end: u64,
}

struct MeterPublisher {
    publisher: PublisherHandle<LatestBlock<MeterHeader, f32>>,
    binding: ErasedBinding,
    publish: bool,
}

impl RtProcessor<f64> for MeterPublisher {
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Source
    }

    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() == IoMode::Separate
            && io.input_count() == 0
            && io.output_count() == 1
            && io.output_channels(0) == Some(1)
        {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }

    fn signal_bindings(&self) -> &[ErasedBinding] {
        std::slice::from_ref(&self.binding)
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, f64>,
        _params: ProcessParameters<'_>,
        mut signals: ProcessSignals<'_>,
    ) {
        let ProcessIo::Separate { mut outputs, .. } = io else {
            unreachable!()
        };
        outputs.get_mut(0).unwrap().channel_mut(0).fill(0.0);
        if self.publish {
            signals
                .publish_slice(
                    &self.publisher,
                    MeterHeader {
                        timeline_epoch: 77,
                        frame_start: 100,
                        frame_end: 103,
                    },
                    &[0.25, 0.5, 0.75],
                )
                .unwrap();
        }
    }
}

fn meter_engine(
    host: &HostControl,
    topic: &Topic<LatestBlock<MeterHeader, f32>>,
    revision: u64,
    id: u64,
    publish: bool,
) -> (Engine<f64>, PublisherHandle<LatestBlock<MeterHeader, f32>>) {
    use moiren_engine::buffer::{BufferArena, BufferSlotLayout, PortAccess};
    use moiren_engine::control::parameter_channel;
    let publisher = host.prepare(topic, host.new_binding().unwrap()).unwrap();
    let handle = publisher.handle();
    let bindings = publisher.erase();
    let resources = RtResources::new(vec![ProcessorInstance::new(
        ProcessorId(id),
        MeterPublisher {
            publisher: handle,
            binding: bindings,
            publish,
        },
    )])
    .unwrap();
    let (_, parameters) = parameter_channel(vec![], revision, 3, 16, 1024).unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    let plan = ExecutionPlan::prepare_with_signals(
        arena,
        vec![OpSpec {
            processor: ProcessorId(id),
            io,
        }],
        &resources,
        &parameters,
        config(revision).engine,
        host,
    )
    .unwrap();
    (Engine::new(plan, resources, parameters).unwrap(), handle)
}

#[test]
fn c3_latest_block_header_payload_and_length_survive_real_engine_swap() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_latest_block(
            "swap/meter-block",
            BlockInit {
                header: MeterHeader {
                    timeline_epoch: 0,
                    frame_start: 0,
                    frame_end: 0,
                },
                element: 0.0_f32,
                capacity: 4,
            },
        )
        .unwrap();
    let mut reader = bus.subscribe(&topic, Default::default()).unwrap();
    let mut witness = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, host) = bus.create_host(2).unwrap();
    let (mut engine, _) = meter_engine(&control, &topic, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    c3_render(&mut engine, 1);
    pump(&mut bus);
    let first = witness.read_latest().unwrap();
    assert_eq!(
        *first.header(),
        MeterHeader {
            timeline_epoch: 77,
            frame_start: 100,
            frame_end: 103
        }
    );
    assert_eq!(first.elements(), &[0.25, 0.5, 0.75]);
    let first_meta = first.meta();
    let retained_address = first.elements().as_ptr();

    let mut plans = engine.enable_plan_switching(2).unwrap();
    let (candidate, _) = meter_engine(&control, &topic, 2, 20, false);
    let candidate = PreparedPlan::new(candidate).unwrap();
    plans.publish(candidate).unwrap();
    c3_render(&mut engine, 1);
    assert_eq!(engine.revision(), 2);
    assert_eq!(first.elements().as_ptr(), retained_address);
    assert_eq!(first.elements(), &[0.25, 0.5, 0.75]);
    assert_eq!(first.meta(), first_meta);
    pump(&mut bus);
    let retained = reader.read_latest().unwrap();
    assert_eq!(
        *retained.header(),
        MeterHeader {
            timeline_epoch: 77,
            frame_start: 100,
            frame_end: 103
        }
    );
    assert_eq!(retained.elements(), &[0.25, 0.5, 0.75]);
    assert_eq!(retained.meta(), first_meta);
    drop(retained);

    plans
        .publish(PreparedPlan::new(meter_engine(&control, &topic, 3, 30, true).0).unwrap())
        .unwrap();
    c3_render(&mut engine, 1);
    pump(&mut bus);
    let next = reader.read_latest().unwrap();
    assert_eq!(
        *next.header(),
        MeterHeader {
            timeline_epoch: 77,
            frame_start: 100,
            frame_end: 103
        }
    );
    assert_eq!(next.elements(), &[0.25, 0.5, 0.75]);
    assert_eq!(next.meta().sequence, first_meta.sequence + 1);
    assert_eq!(
        next.meta().activation_generation,
        first_meta.activation_generation
    );
    assert_eq!(first.elements().as_ptr(), retained_address);
    assert_eq!(first.elements(), &[0.25, 0.5, 0.75]);
    assert_eq!(topic.stats().attempts, first_meta.sequence + 1);
    drop(plans.poll_retired().unwrap());
}

#[test]
fn c3_attached_host_is_not_a_candidate() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus
        .register_latest_value("candidate/latest", 0_u64)
        .unwrap();
    let stream = bus
        .register_stream_value("candidate/stream", 0_u64, 8)
        .unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    assert!(matches!(
        PreparedPlan::new(engine),
        Err(PlanSwapError::InvalidCandidate)
    ));
}

#[test]
fn c3_foreign_update_is_retired_without_replacing_active_permissions() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("foreign/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("foreign/stream", 0_u64, 8)
        .unwrap();
    let foreign_latest = bus
        .register_latest_value("foreign/other-latest", 0_u64)
        .unwrap();
    let foreign_stream = bus
        .register_stream_value("foreign/other-stream", 0_u64, 8)
        .unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let (foreign_control, foreign_host) = bus.create_host(4).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let mut plans = engine.enable_plan_switching(2).unwrap();
    plans
        .publish(
            PreparedPlan::new(swap_source(
                &foreign_control,
                &foreign_latest,
                &foreign_stream,
                2,
                20,
                false,
            ))
            .unwrap(),
        )
        .unwrap();
    c3_render(&mut engine, 1);
    assert_eq!(engine.revision(), 1);
    assert_eq!(
        latest.status().publisher,
        moiren_signal::PublisherStatus::Bound
    );
    assert_eq!(
        foreign_latest.status().publisher,
        moiren_signal::PublisherStatus::NoPublisher
    );
    drop(plans.poll_retired().unwrap());
    drop(foreign_host);
}

#[test]
fn c3_into_parts_keeps_the_unique_host_alive_until_control_cleanup() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("parts/latest", 0_u64).unwrap();
    let stream = bus.register_stream_value("parts/stream", 0_u64, 8).unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let parts = engine.into_parts();
    assert_eq!(
        latest.status().publisher,
        moiren_signal::PublisherStatus::Bound
    );
    drop(parts);
    assert_eq!(
        latest.status().publisher,
        moiren_signal::PublisherStatus::NoPublisher
    );
}

#[test]
fn c3_cancel_retire_full_and_snapshot_leases_keep_current_bindings() {
    use moiren_engine::runtime::RetireOutcome;
    use moiren_signal::{PublisherStatus, UnregisterError};
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("cancel/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("cancel/stream", 0_u64, 8)
        .unwrap();
    let new_latest = bus
        .register_latest_value("cancel/new-latest", 0_u64)
        .unwrap();
    let new_stream = bus
        .register_stream_value("cancel/new-stream", 0_u64, 8)
        .unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut reader = bus.subscribe(&latest, Default::default()).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let mut plans = engine.enable_plan_switching(1).unwrap();
    let candidate =
        PreparedPlan::new(swap_source(&control, &new_latest, &new_stream, 2, 20, true)).unwrap();
    let snapshot = candidate.snapshot();
    assert_eq!(bus.unregister(&new_latest), Err(UnregisterError::Busy));
    plans.publish(candidate).unwrap();
    plans.cancel_pending();
    c3_render(&mut engine, 1);
    assert_eq!(engine.revision(), 1);
    assert_eq!(latest.status().publisher, PublisherStatus::Bound);
    assert_eq!(new_latest.status().publisher, PublisherStatus::NoPublisher);
    let candidate =
        PreparedPlan::new(swap_source(&control, &latest, &stream, 3, 30, true)).unwrap();
    plans.publish(candidate).unwrap();
    c3_render(&mut engine, 1);
    assert_eq!(engine.revision(), 1);
    pump(&mut bus);
    assert_eq!(reader.read_latest().unwrap().value, 2);
    let retired = plans.poll_retired().unwrap();
    assert_eq!(
        retired.outcome(),
        RetireOutcome::Rejected(PlanSwapError::Cancelled)
    );
    assert_eq!(bus.unregister(&new_latest), Err(UnregisterError::Busy));
    drop(retired);
    assert_eq!(bus.unregister(&new_latest), Err(UnregisterError::Busy));
    drop(snapshot);
    bus.unregister(&new_latest).unwrap();
    bus.unregister(&new_stream).unwrap();
    c3_render(&mut engine, 1);
    assert_eq!(engine.revision(), 3);
    pump(&mut bus);
    let read = reader.read_latest().unwrap();
    assert_eq!((read.value, read.meta.sequence), (1, 3));
}

#[test]
fn c3_stale_signal_update_and_stale_reuse_basis_retire_without_changing_audio() {
    use moiren_engine::runtime::RetireOutcome;
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("stale/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("stale/stream", 0_u64, 16)
        .unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut reader = bus.subscribe(&latest, Default::default()).unwrap();
    let (mut active, mut audio) = pipeline(&control, &latest, &stream, 1, true);
    active.engine.attach_signal_host(host).unwrap();
    let basis = active.engine.plan_snapshot();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    let (first, _) = pipeline(&control, &latest, &stream, 2, true);
    let (stale, _) = pipeline(&control, &latest, &stream, 3, false);
    let source = ProcessorId(0);
    let sink = ProcessorId(2);
    let candidate = PreparedPlan::new(first.engine)
        .unwrap()
        .with_reuse(
            &basis,
            &[
                ProcessorReuse {
                    old: source,
                    new: source,
                },
                ProcessorReuse {
                    old: sink,
                    new: sink,
                },
            ],
        )
        .unwrap();
    plans.publish(candidate).unwrap();
    c3_render(&mut active.engine, 1);
    drop(plans.poll_retired().unwrap());
    plans
        .publish(PreparedPlan::new(stale.engine).unwrap())
        .unwrap();
    c3_render(&mut active.engine, 1);
    assert_eq!(active.engine.revision(), 2);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::SignalActivation(ActivateError::StaleBasis))
    );
    let (stale_reuse, _) = pipeline(&control, &latest, &stream, 4, false);
    let candidate = PreparedPlan::new(stale_reuse.engine)
        .unwrap()
        .with_reuse(
            &basis,
            &[ProcessorReuse {
                old: source,
                new: source,
            }],
        )
        .unwrap();
    plans.publish(candidate).unwrap();
    c3_render(&mut active.engine, 1);
    assert_eq!(active.engine.revision(), 2);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::StaleBasis)
    );
    pump(&mut bus);
    assert_eq!(reader.read_latest().unwrap().value, 3);
    let mut original_audio = [0.0; 3];
    assert_eq!(
        audio
            .read_interleaved(&mut original_audio)
            .unwrap()
            .transferred_frames,
        3
    );
    assert_eq!(original_audio, [1.0, 2.0, 3.0]);
}

#[test]
fn c3_prepared_new_topics_activate_without_heap_work_and_old_leases_retire() {
    use moiren_signal::{PublisherStatus, UnregisterError};
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("new/old-latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("new/old-stream", 0_u64, 8)
        .unwrap();
    let new_latest = bus.register_latest_value("new/latest", 0_u64).unwrap();
    let new_stream = bus.register_stream_value("new/stream", 0_u64, 8).unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let mut reader = bus.subscribe(&new_latest, Default::default()).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let mut plans = engine.enable_plan_switching(1).unwrap();
    plans
        .publish(
            PreparedPlan::new(swap_source(&control, &new_latest, &new_stream, 2, 20, true))
                .unwrap(),
        )
        .unwrap();
    c3_render(&mut engine, 1);
    assert_eq!(latest.status().publisher, PublisherStatus::NoPublisher);
    assert_eq!(new_latest.status().publisher, PublisherStatus::Bound);
    assert_eq!(bus.unregister(&latest), Err(UnregisterError::Busy));
    pump(&mut bus);
    let read = reader.read_latest().unwrap();
    assert_eq!(
        (
            read.value,
            read.meta.sequence,
            read.meta.activation_generation
        ),
        (1, 1, 1)
    );
    drop(plans.poll_retired().unwrap());
    bus.unregister(&latest).unwrap();
    bus.unregister(&stream).unwrap();
    assert_eq!(bus.unregister(&new_latest), Err(UnregisterError::Busy));
}

#[test]
fn c3_new_topic_slot_capacity_and_duplicate_owners_fail_before_activation() {
    use moiren_signal::{HostError, UnregisterError};
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("capacity/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("capacity/stream", 0_u64, 8)
        .unwrap();
    let new_topic = bus.register_latest_value("capacity/new", 0_u64).unwrap();
    let (control, host) = bus.create_host(2).unwrap();
    let mut engine = swap_source(&control, &latest, &stream, 1, 10, true);
    engine.attach_signal_host(host).unwrap();
    let key = control.new_binding().unwrap();
    let (result, counts) = super::allocation::track_heap(|| control.prepare(&new_topic, key));
    assert!(matches!(result, Err(HostError::HostCapacity)));
    assert_eq!(counts, (0, 0, 0));
    bus.unregister(&new_topic).unwrap();
    let publisher = control.prepare(&latest, key).unwrap();
    assert!(matches!(
        control.prepare_update(&[publisher.erase(), publisher.erase()]),
        Err(HostError::PublisherConflict)
    ));
    c3_render(&mut engine, 1);
    assert_eq!(engine.revision(), 1);
    assert_eq!(bus.unregister(&latest), Err(UnregisterError::Busy));
}

#[test]
fn c3_missing_host_and_missing_update_reject_to_the_same_retire_queue() {
    use moiren_engine::{boundary::SourceAdapter, runtime::RetireOutcome};
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("missing/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("missing/stream", 0_u64, 8)
        .unwrap();
    let (control, host) = bus.create_host(2).unwrap();
    let mut plain = source_engine(
        None,
        1,
        10,
        SourceAdapter(ConstantSource {
            channels: 1,
            value: 0.25,
        }),
    );
    let mut plain_plans = plain.enable_plan_switching(1).unwrap();
    plain_plans
        .publish(PreparedPlan::new(swap_source(&control, &latest, &stream, 2, 20, true)).unwrap())
        .unwrap();
    c3_render(&mut plain, 1);
    assert_eq!(plain.revision(), 1);
    assert_eq!(
        plain_plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::MissingSignalHost)
    );
    let (mut active, mut audio) = pipeline(&control, &latest, &stream, 3, true);
    active.engine.attach_signal_host(host).unwrap();
    let mut plans = active.engine.enable_plan_switching(1).unwrap();
    plans
        .publish(
            PreparedPlan::new(source_engine(
                None,
                4,
                30,
                SourceAdapter(ConstantSource {
                    channels: 1,
                    value: 0.25,
                }),
            ))
            .unwrap(),
        )
        .unwrap();
    c3_render(&mut active.engine, 1);
    assert_eq!(active.engine.revision(), 3);
    assert_eq!(
        plans.poll_retired().unwrap().outcome(),
        RetireOutcome::Rejected(PlanSwapError::MissingSignalPlan)
    );
    let mut output = [0.0; 1];
    assert_eq!(
        audio
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(output, [1.0]);
    plans
        .publish(
            PreparedPlan::new(source_engine(
                Some(&control),
                5,
                30,
                SourceAdapter(ConstantSource {
                    channels: 1,
                    value: 0.25,
                }),
            ))
            .unwrap(),
        )
        .unwrap();
    c3_render(&mut active.engine, 1);
    assert_eq!(active.engine.revision(), 5);
    assert_eq!(
        latest.status().publisher,
        moiren_signal::PublisherStatus::NoPublisher
    );
}

#[test]
fn c3_drop_probe_and_signal_ownership_return_to_control_after_worker_stops() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
    };
    struct Audit {
        owner: thread::ThreadId,
        drops: AtomicUsize,
        wrong_thread: AtomicBool,
    }
    struct DropProbe(Arc<Audit>);
    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.drops.fetch_add(1, Ordering::Relaxed);
            if thread::current().id() != self.0.owner {
                self.0.wrong_thread.store(true, Ordering::Relaxed);
            }
        }
    }
    struct AuditedSource {
        source: SignalSource,
        _probe: DropProbe,
    }
    impl RtProcessor<f64> for AuditedSource {
        fn role(&self) -> ProcessorRole {
            ProcessorRole::Source
        }
        fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
            self.source.validate_io(io)
        }
        fn signal_bindings(&self) -> &[ErasedBinding] {
            self.source.signal_bindings()
        }
        fn process(
            &mut self,
            ctx: &ProcessContext,
            io: ProcessIo<'_, f64>,
            params: ProcessParameters<'_>,
            signals: ProcessSignals<'_>,
        ) {
            self.source.process(ctx, io, params, signals)
        }
    }
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("cleanup/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("cleanup/stream", 0_u64, 16)
        .unwrap();
    let baseline = bus.budget_used();
    let (control, host) = bus.create_host(4).unwrap();
    let audit = Arc::new(Audit {
        owner: thread::current().id(),
        drops: AtomicUsize::new(0),
        wrong_thread: AtomicBool::new(false),
    });
    let prepare_source = |host: &HostControl,
                          latest: &Topic<LatestValue<u64>>,
                          stream: &Topic<StreamValue<u64>>,
                          revision| {
        let key = host.new_binding().unwrap();
        let latest = host.prepare(latest, key).unwrap();
        let stream = host.prepare(stream, key).unwrap();
        source_engine(
            Some(host),
            revision,
            revision * 10,
            AuditedSource {
                source: SignalSource {
                    latest: latest.handle(),
                    stream: stream.handle(),
                    bindings: [latest.erase(), stream.erase()],
                    calls: 0,
                    publish: true,
                },
                _probe: DropProbe(Arc::clone(&audit)),
            },
        )
    };
    let prepare = |revision| prepare_source(&control, &latest, &stream, revision);
    let mut engine = prepare(1);
    engine.attach_signal_host(host).unwrap();
    let basis = engine.plan_snapshot();
    let stale_update = PreparedPlan::new(prepare(6)).unwrap();
    let stale_reuse = PreparedPlan::new(prepare(7))
        .unwrap()
        .with_reuse(
            &basis,
            &[ProcessorReuse {
                old: ProcessorId(10),
                new: ProcessorId(70),
            }],
        )
        .unwrap();
    let mut plans = engine.enable_plan_switching(1).unwrap();
    let (commands, receiver) = mpsc::channel();
    let (reports, observed) = mpsc::channel();
    let worker = thread::spawn(move || {
        while receiver.recv().unwrap() {
            let (result, counts) = super::allocation::track_heap(|| engine.render(1));
            reports.send((result, counts)).unwrap();
        }
        engine
    });
    let render = || {
        commands.send(true).unwrap();
        let (result, counts) = observed.recv().unwrap();
        result.unwrap();
        assert_eq!(counts, (0, 0, 0));
    };
    plans
        .publish(PreparedPlan::new(prepare(2)).unwrap())
        .unwrap();
    render();
    assert_eq!(audit.drops.load(Ordering::Relaxed), 0);
    let before_retire = bus.budget_used();
    drop(plans.poll_retired().unwrap());
    assert!(bus.budget_used() < before_retire);
    assert_eq!(audit.drops.load(Ordering::Relaxed), 1);
    plans
        .publish(PreparedPlan::new(prepare(3)).unwrap())
        .unwrap();
    plans.cancel_pending();
    render();
    plans
        .publish(PreparedPlan::new(prepare(4)).unwrap())
        .unwrap();
    render();
    assert_eq!(audit.drops.load(Ordering::Relaxed), 1);
    drop(plans.poll_retired().unwrap());
    assert_eq!(audit.drops.load(Ordering::Relaxed), 2);
    render();
    drop(plans.poll_retired().unwrap());
    assert_eq!(audit.drops.load(Ordering::Relaxed), 3);
    plans.publish(stale_update).unwrap();
    render();
    assert_eq!(audit.drops.load(Ordering::Relaxed), 3);
    let retired = plans.poll_retired().unwrap();
    assert_eq!(
        retired.outcome(),
        moiren_engine::runtime::RetireOutcome::Rejected(PlanSwapError::SignalActivation(
            ActivateError::StaleBasis
        ))
    );
    drop(retired);
    assert_eq!(audit.drops.load(Ordering::Relaxed), 4);
    plans.publish(stale_reuse).unwrap();
    render();
    assert_eq!(audit.drops.load(Ordering::Relaxed), 4);
    let retired = plans.poll_retired().unwrap();
    assert_eq!(
        retired.outcome(),
        moiren_engine::runtime::RetireOutcome::Rejected(PlanSwapError::StaleBasis)
    );
    drop(retired);
    assert_eq!(audit.drops.load(Ordering::Relaxed), 5);
    let foreign_latest = bus
        .register_latest_value("cleanup/foreign-latest", 0_u64)
        .unwrap();
    let foreign_stream = bus
        .register_stream_value("cleanup/foreign-stream", 0_u64, 8)
        .unwrap();
    let (foreign_control, foreign_host) = bus.create_host(2).unwrap();
    plans
        .publish(
            PreparedPlan::new(prepare_source(
                &foreign_control,
                &foreign_latest,
                &foreign_stream,
                8,
            ))
            .unwrap(),
        )
        .unwrap();
    render();
    assert_eq!(audit.drops.load(Ordering::Relaxed), 5);
    let retired = plans.poll_retired().unwrap();
    assert_eq!(
        retired.outcome(),
        moiren_engine::runtime::RetireOutcome::Rejected(PlanSwapError::SignalActivation(
            ActivateError::ForeignHost
        ))
    );
    drop(retired);
    assert_eq!(audit.drops.load(Ordering::Relaxed), 6);
    plans
        .publish(PreparedPlan::new(prepare(9)).unwrap())
        .unwrap();
    drop(plans);
    render();
    assert_eq!(audit.drops.load(Ordering::Relaxed), 6);
    commands.send(false).unwrap();
    let engine = worker.join().unwrap();
    assert_eq!(engine.revision(), 4);
    drop(engine.into_parts());
    drop(basis);
    drop((foreign_host, foreign_control));
    bus.unregister(&foreign_latest).unwrap();
    bus.unregister(&foreign_stream).unwrap();
    drop((foreign_latest, foreign_stream));
    drop(control);
    assert_eq!(audit.drops.load(Ordering::Relaxed), 8);
    assert!(!audit.wrong_thread.load(Ordering::Relaxed));
    assert_eq!(bus.budget_used(), baseline);
    bus.unregister(&latest).unwrap();
    bus.unregister(&stream).unwrap();
    assert!(bus.budget_used() < baseline);
}

struct SignalSource {
    latest: PublisherHandle<LatestValue<u64>>,
    stream: PublisherHandle<StreamValue<u64>>,
    bindings: [ErasedBinding; 2],
    calls: u64,
    publish: bool,
}

impl RtProcessor<f64> for SignalSource {
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Source
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() == IoMode::Separate
            && io.input_count() == 0
            && io.output_count() == 1
            && io.output_channels(0) == Some(1)
        {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }
    fn signal_bindings(&self) -> &[ErasedBinding] {
        &self.bindings
    }
    fn process(
        &mut self,
        _ctx: &ProcessContext,
        io: ProcessIo<'_, f64>,
        _params: ProcessParameters<'_>,
        mut signals: ProcessSignals<'_>,
    ) {
        self.calls += 1;
        if let ProcessIo::Separate { mut outputs, .. } = io {
            outputs
                .get_mut(0)
                .unwrap()
                .channel_mut(0)
                .fill(self.calls as f64);
        }
        if self.publish {
            signals.publish(&self.latest, self.calls).unwrap();
            signals.publish(&self.stream, self.calls).unwrap();
        }
    }
}

fn config(revision: u64) -> CompileConfig {
    CompileConfig {
        engine: super::support::config(16),
        audio_byte_budget: 4096,
        plan_revision: revision,
        timeline_epoch: 3,
        control_capacity: 16,
        control_horizon_frames: 1024,
    }
}

fn pipeline(
    host: &HostControl,
    latest: &Topic<LatestValue<u64>>,
    stream: &Topic<StreamValue<u64>>,
    revision: u64,
    publish: bool,
) -> (CompiledGraph<f64>, AudioReader<f64>) {
    let key = host.new_binding().unwrap();
    let latest = host.prepare(latest, key).unwrap();
    let stream = host.prepare(stream, key).unwrap();
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
    let mut bindings = NodeBindings::new();
    bindings
        .bind_io(
            source,
            SignalSource {
                latest: latest.handle(),
                stream: stream.handle(),
                bindings: [latest.erase(), stream.erase()],
                calls: 0,
                publish,
            },
        )
        .unwrap();
    let (writer, reader) = audio_bridge::<f64>(1, 32, 4096).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    (
        compile_with_signals(&graph, bindings, config(revision), host).unwrap(),
        reader,
    )
}

fn pump(bus: &mut Bus) {
    bus.pump(PumpBudget {
        topic_checks: 1024,
        deliveries: 128,
        copy_bytes: 65536,
    });
}

#[test]
fn signal_publication_renders_with_zero_heap_work_and_no_demand_still_runs_dsp() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("runtime/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("runtime/stream", 0_u64, 8)
        .unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let (mut graph, mut audio) = pipeline(&control, &latest, &stream, 1, true);
    assert_eq!(graph.engine.render(4), Err(RuntimeError::MissingSignalHost));
    graph.engine.attach_signal_host(host).unwrap();
    let mut latest_reader = bus.subscribe(&latest, Default::default()).unwrap();
    let mut stream_reader = bus.subscribe(&stream, Default::default()).unwrap();
    let (report, counts) = track_allocations(|| graph.engine.render(4).unwrap());
    assert_eq!(counts, (0, 0));
    assert_eq!(report.segments, 1);
    pump(&mut bus);
    assert_eq!(latest_reader.read_latest().unwrap().value, 1);
    assert_eq!(stream_reader.next().unwrap().value, 1);
    drop((latest_reader, stream_reader));
    let (_, counts) = track_allocations(|| graph.engine.render(4).unwrap());
    assert_eq!(counts, (0, 0));
    let mut samples = [0.0; 8];
    audio.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0]);
}

#[test]
fn attach_failure_returns_complete_host_and_preserves_prepared_update_for_retry() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let latest = bus.register_latest_value("attach/latest", 0_u64).unwrap();
    let stream = bus
        .register_stream_value("attach/stream", 0_u64, 4)
        .unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let (_, foreign) = bus.create_host(4).unwrap();
    let (mut graph, _) = pipeline(&control, &latest, &stream, 1, true);
    let failure = graph.engine.attach_signal_host(foreign).unwrap_err();
    assert_eq!(
        failure.error,
        SignalAttachError::Activate(ActivateError::ForeignHost)
    );
    graph.engine.attach_signal_host(host).unwrap();
    let failure = graph.engine.attach_signal_host(failure.host).unwrap_err();
    assert_eq!(failure.error, SignalAttachError::AlreadyAttached);
    graph.engine.render(4).unwrap();
    drop(failure.host);
}

struct ScopedObserver {
    binding: Option<ErasedBinding>,
    own: PublisherHandle<LatestValue<u64>>,
    other: PublisherHandle<LatestValue<u64>>,
}
impl RtObserver<f64> for ScopedObserver {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() == IoMode::ReadOnly && io.input_count() == 1 {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }
    fn signal_bindings(&self) -> &[ErasedBinding] {
        self.binding.as_slice()
    }
    fn observe(
        &mut self,
        _ctx: &ProcessContext,
        _inputs: ReadPorts<'_, f64>,
        mut signals: ProcessSignals<'_>,
    ) {
        assert_eq!(
            signals.publish(&self.other, 10),
            Err(PublishError::InactiveBinding)
        );
        if self.binding.is_some() {
            signals.publish(&self.own, 7).unwrap();
        } else {
            assert!(!signals.demand(&self.own).unwrap().active);
            assert_eq!(
                signals.publish(&self.own, 7),
                Err(PublishError::InactiveBinding)
            );
        }
    }
}
fn observer(
    own: &PreparedPublisher<LatestValue<u64>>,
    other: &PreparedPublisher<LatestValue<u64>>,
    bind: bool,
) -> ScopedObserver {
    ScopedObserver {
        own: own.handle(),
        other: other.handle(),
        binding: bind.then(|| own.erase()),
    }
}

#[test]
fn independent_observers_with_one_key_are_scoped_and_empty_permissions_are_inactive() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let first = bus.register_latest_value("scope/first", 0_u64).unwrap();
    let second = bus.register_latest_value("scope/second", 0_u64).unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let key = control.new_binding().unwrap();
    let first = control.prepare(&first, key).unwrap();
    let second = control.prepare(&second, key).unwrap();
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let port = graph.get_node(source).unwrap().outputs()[0].id();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 1,
                value: 0.25,
            },
        )
        .unwrap();
    bindings
        .observe_output(port, observer(&first, &second, true))
        .unwrap();
    bindings
        .observe_output(port, observer(&second, &first, true))
        .unwrap();
    bindings
        .observe_output(port, observer(&first, &second, false))
        .unwrap();
    let mut compiled = compile_with_signals(&graph, bindings, config(1), &control).unwrap();
    compiled.engine.attach_signal_host(host).unwrap();
    let (_, counts) = track_allocations(|| compiled.engine.render(4).unwrap());
    assert_eq!(counts, (0, 0));
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 1,
                value: 0.25,
            },
        )
        .unwrap();
    bindings
        .observe_output(port, observer(&first, &second, false))
        .unwrap();
    compile(&graph, bindings, config(1))
        .unwrap()
        .engine
        .render(4)
        .unwrap();
}

#[test]
fn meter_and_compressor_demand_transitions_keep_full_dsp_and_zero_heap_work() {
    use moiren_engine::{
        meter::signal::{AudioWindow, ChannelLevel, SignalLevelMeter},
        processor::CompressorSettings,
    };
    use moiren_signal::BlockInit;

    let mut bus = Bus::new(Default::default()).unwrap();
    let reduction = bus
        .register_latest_value("runtime/compressor", 0.0_f32)
        .unwrap();
    let levels = bus
        .register_latest_block(
            "runtime/levels",
            BlockInit {
                header: AudioWindow {
                    timeline_epoch: 0,
                    frame_start: 0,
                    frame_end: 0,
                    sample_rate: 0.0,
                },
                element: ChannelLevel {
                    peak: 0.0,
                    rms: 0.0,
                },
                capacity: 2,
            },
        )
        .unwrap();
    let (control, host) = bus.create_host(2).unwrap();
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let compressor = graph.create_node(NodeKind::Compressor, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    for (from, to) in [(source, compressor), (compressor, sink)] {
        graph
            .connect(
                graph.get_node(from).unwrap().outputs()[0].id(),
                graph.get_node(to).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
    }
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            ConstantSource {
                channels: 2,
                value: 1.0,
            },
        )
        .unwrap();
    bindings
        .bind_compressor_with_signal(
            compressor,
            CompressorSettings {
                threshold_db: -12.0,
                attack_ms: 0.0,
                knee_db: 0.0,
                ..Default::default()
            },
            control
                .prepare(&reduction, control.new_binding().unwrap())
                .unwrap(),
        )
        .unwrap();
    bindings
        .observe_output(
            graph.get_node(compressor).unwrap().outputs()[0].id(),
            SignalLevelMeter::new(
                2,
                4,
                control
                    .prepare(&levels, control.new_binding().unwrap())
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let (writer, mut audio) = audio_bridge::<f64>(2, 32, 4096).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let mut compiled = compile_with_signals(&graph, bindings, config(1), &control).unwrap();
    compiled.engine.attach_signal_host(host).unwrap();
    for frames in [2, 4] {
        let (_, counts) = track_allocations(|| compiled.engine.render(frames).unwrap());
        assert_eq!(counts, (0, 0));
    }
    assert_eq!(levels.stats().attempts, 0);
    assert_eq!(reduction.stats().attempts, 0);
    let level_reader = bus.subscribe(&levels, Default::default()).unwrap();
    let mut reduction_reader = bus.subscribe(&reduction, Default::default()).unwrap();
    let (_, counts) = track_allocations(|| compiled.engine.render(2).unwrap());
    assert_eq!(counts, (0, 0));
    pump(&mut bus);
    assert_eq!(reduction_reader.read_latest().unwrap().value, 9.0);
    assert_eq!(levels.stats().attempts, 0);
    drop((level_reader, reduction_reader));
    let (_, counts) = track_allocations(|| compiled.engine.render(4).unwrap());
    assert_eq!(counts, (0, 0));
    assert_eq!(levels.stats().attempts, 0);
    assert_eq!(reduction.stats().attempts, 1);
    let mut level_reader = bus.subscribe(&levels, Default::default()).unwrap();
    let mut reduction_reader = bus.subscribe(&reduction, Default::default()).unwrap();
    let (_, counts) = track_allocations(|| compiled.engine.render(2).unwrap());
    assert_eq!(counts, (0, 0));
    pump(&mut bus);
    assert!(level_reader.read_latest().is_none());
    assert_eq!(reduction_reader.read_latest().unwrap().value, 9.0);
    let (_, counts) = track_allocations(|| compiled.engine.render(2).unwrap());
    assert_eq!(counts, (0, 0));
    pump(&mut bus);
    let read = level_reader.read_latest().unwrap();
    assert_eq!(
        *read.header(),
        AudioWindow {
            timeline_epoch: 3,
            frame_start: 12,
            frame_end: 16,
            sample_rate: 48_000.0
        }
    );
    let expected = 10.0_f64.powf(-9.0 / 20.0);
    assert_eq!(
        read.elements(),
        &[ChannelLevel {
            peak: expected as f32,
            rms: expected as f32
        }; 2]
    );
    assert_eq!(levels.stats().attempts, 1);
    assert_eq!(reduction.stats().attempts, 3);
    let mut samples = [0.0; 32];
    audio.read_interleaved(&mut samples).unwrap();
    assert_eq!(samples, [expected; 32]);
}
