use super::*;
use moiren_engine::processor::{ProcessContext, ProcessorError, RtObserver};
use moiren_engine::signal::ProcessSignals;
use moiren_signal::{Bus, HostError, LatestValue, PublishOutcome, PublisherHandle};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

struct PublishedObserver {
    handle: PublisherHandle<LatestValue<f64>>,
    binding: moiren_signal::ErasedBinding,
    value: Arc<AtomicU64>,
    published: Arc<AtomicU64>,
}

#[test]
fn sink_input_observer_measures_post_send_audio_and_disconnected_silence() {
    for (connected, mute, expected) in [(true, false, 0.25), (true, true, 0.0), (false, false, 0.0)]
    {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 1).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
        if connected {
            connect(
                &mut graph,
                source,
                sink,
                SendParams {
                    gain: 0.25,
                    mute,
                    ..Default::default()
                },
            );
        }
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 1,
                    value: 1.0,
                },
            )
            .unwrap();
        let (writer, mut reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_value("test/sink-input", 0.0_f64)
            .unwrap();
        let (control, host) = bus.create_host(1).unwrap();
        let publisher = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let observed = Arc::new(AtomicU64::new(f64::NAN.to_bits()));
        bindings
            .observe_input(
                input(&graph, sink),
                PublishedObserver {
                    handle: publisher.handle(),
                    binding: publisher.erase(),
                    value: observed.clone(),
                    published: Arc::new(AtomicU64::new(0)),
                },
            )
            .unwrap();
        let mut compiled = compile_with_signals(&graph, bindings, config(), &control).unwrap();
        compiled.engine.attach_signal_host(host).unwrap();
        compiled.engine.render(4).unwrap();
        assert_eq!(f64::from_bits(observed.load(Ordering::Relaxed)), expected);
        let mut samples = [f64::NAN; 4];
        reader.read_interleaved(&mut samples).unwrap();
        assert_eq!(samples, [expected; 4]);
    }
}

#[test]
fn input_observer_rejects_output_port_in_preparation() {
    let (graph, _, _, _, output_port) = signal_graph();
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus
        .register_latest_value("test/wrong-input", 0.0_f64)
        .unwrap();
    let (control, _host) = bus.create_host(1).unwrap();
    let publisher = control
        .prepare(&topic, control.new_binding().unwrap())
        .unwrap();
    let mut bindings = NodeBindings::<f64>::new();
    bindings
        .observe_input(
            output_port,
            PublishedObserver {
                handle: publisher.handle(),
                binding: publisher.erase(),
                value: Arc::new(AtomicU64::new(0)),
                published: Arc::new(AtomicU64::new(0)),
            },
        )
        .unwrap();
    assert!(
        matches!(compile_with_signals(&graph, bindings, config(), &control),
        Err(CompileError::ObserverOutputPort { port }) if port == output_port)
    );
}

impl RtObserver<f64> for PublishedObserver {
    fn validate_inputs(
        &self,
        io: &moiren_engine::buffer::PreparedIo,
    ) -> Result<(), ProcessorError> {
        if io.input_count() == 1 && io.input_channels(0) == Some(1) {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }

    fn observe(
        &mut self,
        _ctx: &ProcessContext,
        inputs: moiren_engine::buffer::ReadPorts<'_, f64>,
        mut signals: ProcessSignals<'_>,
    ) {
        let sample = inputs.get(0).unwrap().channel(0)[0];
        self.value.store(sample.to_bits(), Ordering::Relaxed);
        if matches!(
            signals.publish(&self.handle, sample),
            Ok(PublishOutcome::Published(_))
        ) {
            self.published.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn signal_bindings(&self) -> &[moiren_signal::ErasedBinding] {
        std::slice::from_ref(&self.binding)
    }
}

fn signal_graph() -> (LogicalGraph, NodeId, NodeId, NodeId, PortId) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 1).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 1).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
    let source_port = output(&graph, source);
    graph
        .connect(source_port, input(&graph, gain), SendParams::default())
        .unwrap();
    graph
        .connect(
            output(&graph, gain),
            input(&graph, sink),
            SendParams::default(),
        )
        .unwrap();
    (graph, source, gain, sink, source_port)
}

#[test]
fn fan_out_observer_precedes_both_non_unit_sends_and_preserves_every_sample() {
    let mut rendered = Vec::new();
    for observe in [false, true] {
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 1).unwrap();
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
        let mut readers = Vec::new();
        for (send_gain, node_gain) in [(0.5, 2.0), (4.0, 3.0)] {
            let gain = graph.create_node(NodeKind::Gain, 1).unwrap();
            let sink = graph.create_node(NodeKind::Sink, 1).unwrap();
            connect(
                &mut graph,
                source,
                gain,
                SendParams {
                    gain: send_gain,
                    ..Default::default()
                },
            );
            connect(&mut graph, gain, sink, SendParams::default());
            bindings.bind_gain(gain, node_gain).unwrap();
            let (writer, reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
            bindings.bind_sink(sink, writer).unwrap();
            readers.push(reader);
        }
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_value("test/fan-out-source", 0.0_f64)
            .unwrap();
        let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, host) = bus.create_host(4).unwrap();
        let observed = Arc::new(AtomicU64::new(0));
        let published = Arc::new(AtomicU64::new(0));
        if observe {
            let publisher = control
                .prepare(&topic, control.new_binding().unwrap())
                .unwrap();
            bindings
                .observe_output(
                    output(&graph, source),
                    PublishedObserver {
                        handle: publisher.handle(),
                        binding: publisher.erase(),
                        value: Arc::clone(&observed),
                        published: Arc::clone(&published),
                    },
                )
                .unwrap();
        }
        let mut compiled = if observe {
            compile_with_signals(&graph, bindings, config(), &control).unwrap()
        } else {
            compile(&graph, bindings, config()).unwrap()
        };
        if observe {
            compiled.engine.attach_signal_host(host).unwrap();
        }
        compiled.engine.render(4).unwrap();
        let audio: Vec<[f64; 4]> = readers
            .iter_mut()
            .map(|reader| {
                let mut samples = [0.0; 4];
                reader.read_interleaved(&mut samples).unwrap();
                samples
            })
            .collect();
        assert_eq!(audio, [[0.25; 4], [3.0; 4]]);
        if observe {
            assert_eq!(f64::from_bits(observed.load(Ordering::Relaxed)), 0.25);
            assert_eq!(published.load(Ordering::Relaxed), 1);
            bus.pump(moiren_signal::PumpBudget {
                topic_checks: 1024,
                deliveries: 16,
                copy_bytes: 4096,
            });
            assert_eq!(subscriber.read_latest().unwrap().value, 0.25);
        }
        rendered.push(audio);
    }
    assert_eq!(rendered[0], rendered[1]);
}

#[test]
fn observer_reads_exact_producer_output_before_send_and_preserves_audio() {
    let (graph, source, gain, sink, source_port) = signal_graph();
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("test/source", 0.0_f64).unwrap();
    let mut subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let (control, host) = bus.create_host(4).unwrap();
    let key = control.new_binding().unwrap();
    let publisher = control.prepare(&topic, key).unwrap();
    let observed = Arc::new(AtomicU64::new(0));
    let published = Arc::new(AtomicU64::new(0));
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
    bindings.bind_gain(gain, 2.0).unwrap();
    let (writer, mut reader) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    bindings
        .observe_output(
            source_port,
            PublishedObserver {
                handle: publisher.handle(),
                binding: publisher.erase(),
                value: Arc::clone(&observed),
                published: Arc::clone(&published),
            },
        )
        .unwrap();
    let mut compiled = compile_with_signals(&graph, bindings, config(), &control).unwrap();
    compiled.engine.attach_signal_host(host).unwrap();
    compiled.engine.render(4).unwrap();
    let mut audio = [0.0; 4];
    reader.read_interleaved(&mut audio).unwrap();
    assert_eq!(audio, [0.5; 4]);
    assert_eq!(f64::from_bits(observed.load(Ordering::Relaxed)), 0.25);
    assert_eq!(published.load(Ordering::Relaxed), 1);
    bus.pump(moiren_signal::PumpBudget {
        topic_checks: 1024,
        deliveries: 16,
        copy_bytes: 4096,
    });
    assert_eq!(subscriber.read_latest().unwrap().value, 0.25);

    let (plain_source, mut plain_sink) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    let (plain_graph, plain_source_node, plain_gain_node, plain_sink_node, _) = signal_graph();
    let mut plain_bindings = NodeBindings::new();
    plain_bindings
        .bind_source(
            plain_source_node,
            ConstantSource {
                channels: 1,
                value: 0.25,
            },
        )
        .unwrap();
    plain_bindings.bind_gain(plain_gain_node, 2.0).unwrap();
    plain_bindings
        .bind_sink(plain_sink_node, plain_source)
        .unwrap();
    let mut plain = compile(&plain_graph, plain_bindings, config()).unwrap();
    plain.engine.render(4).unwrap();
    let mut plain_audio = [0.0; 4];
    plain_sink.read_interleaved(&mut plain_audio).unwrap();
    assert_eq!(audio, plain_audio);
}

#[test]
fn signal_compile_rejects_missing_host_invalid_ports_and_duplicate_owner() {
    let (graph, source, gain, sink, source_port) = signal_graph();
    let mut missing_host = NodeBindings::new();
    missing_host
        .bind_source(
            source,
            ConstantSource {
                channels: 1,
                value: 0.25,
            },
        )
        .unwrap();
    missing_host.bind_gain(gain, 1.0).unwrap();
    let (writer, _) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    missing_host.bind_sink(sink, writer).unwrap();
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_latest_value("missing-host", 0.0_f64).unwrap();
    let (first_control, _first_host) = bus.create_host(4).unwrap();
    let first_key = first_control.new_binding().unwrap();
    let first_publisher = first_control.prepare(&topic, first_key).unwrap();
    missing_host
        .observe_output(
            source_port,
            PublishedObserver {
                handle: first_publisher.handle(),
                binding: first_publisher.erase(),
                value: Arc::new(AtomicU64::new(0)),
                published: Arc::new(AtomicU64::new(0)),
            },
        )
        .unwrap();
    assert!(matches!(
        compile(&graph, missing_host, config()),
        Err(CompileError::SignalsRequireHost)
    ));

    let input_port = input(&graph, sink);
    let mut wrong_direction = NodeBindings::new();
    wrong_direction
        .bind_source(
            source,
            ConstantSource {
                channels: 1,
                value: 0.25,
            },
        )
        .unwrap();
    wrong_direction.bind_gain(gain, 1.0).unwrap();
    let (writer, _) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    wrong_direction.bind_sink(sink, writer).unwrap();
    wrong_direction
        .observe_output(
            input_port,
            PublishedObserver {
                handle: first_publisher.handle(),
                binding: first_publisher.erase(),
                value: Arc::new(AtomicU64::new(0)),
                published: Arc::new(AtomicU64::new(0)),
            },
        )
        .unwrap();
    assert!(matches!(
        compile_with_signals(&graph, wrong_direction, config(), &first_control),
        Err(CompileError::ObserverInputPort { port }) if port == input_port
    ));
    let mut stale = NodeBindings::new();
    let mut removed = graph.clone();
    let stale_port = output(&removed, source);
    removed.remove_node(source).unwrap();
    stale
        .observe_output(
            stale_port,
            PublishedObserver {
                handle: first_publisher.handle(),
                binding: first_publisher.erase(),
                value: Arc::new(AtomicU64::new(0)),
                published: Arc::new(AtomicU64::new(0)),
            },
        )
        .unwrap();
    assert!(matches!(
        compile_with_signals(&removed, stale, config(), &first_control),
        Err(CompileError::ObserverPortNotFound { port }) if port == stale_port
    ));

    let mut duplicate = NodeBindings::new();
    duplicate
        .bind_source(
            source,
            ConstantSource {
                channels: 1,
                value: 0.25,
            },
        )
        .unwrap();
    let (writer, _) = audio_bridge::<f64>(1, 8, 4096).unwrap();
    duplicate.bind_sink(sink, writer).unwrap();
    duplicate
        .observe_output(
            source_port,
            PublishedObserver {
                handle: first_publisher.handle(),
                binding: first_publisher.erase(),
                value: Arc::new(AtomicU64::new(0)),
                published: Arc::new(AtomicU64::new(0)),
            },
        )
        .unwrap();
    duplicate
        .observe_output(
            source_port,
            PublishedObserver {
                handle: first_publisher.handle(),
                binding: first_publisher.erase(),
                value: Arc::new(AtomicU64::new(0)),
                published: Arc::new(AtomicU64::new(0)),
            },
        )
        .unwrap();
    assert!(matches!(
        compile_with_signals(&graph, duplicate, config(), &first_control),
        Err(CompileError::Signal(
            moiren_engine::signal::SignalPlanError::Host(HostError::PublisherConflict)
        ))
    ));
}
