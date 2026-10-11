use super::allocation::track_allocations;
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
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
    ActivateError, Bus, ErasedBinding, HostControl, LatestValue, PreparedPublisher, PublishError,
    PublisherHandle, PumpBudget, StreamValue, Topic,
};

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
