use super::*;
use crate::service::{SpectrumBinding, SpectrumTopic};
use moiren_engine::spectrum::{SpectrumSettings, SpectrumWindow};
use moiren_signal::{Subscriber, TopicLifecycle};

#[test]
fn spectrum_source_sine_has_correct_fft_peak_frequency_and_peak_amplitude() {
    let mut rig = Rig::new();
    rig.state
        .spectrum_tone_hz
        .store(3000_f64.to_bits(), Ordering::Release);
    let node = analyzer(&mut rig);
    rig.start();
    let binding = binding(&mut rig, node);
    let mut subscriber = subscribe(&mut rig, &binding);
    let (header, bins) = window(&mut rig, &mut subscriber);
    for channel in bins.as_chunks::<33>().0 {
        let peak = channel
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        assert_eq!(peak.0, 4);
        assert_eq!(
            peak.0 as f64 * header.sample_rate / header.fft_size as f64,
            3000.
        );
        assert!((*peak.1 - 0.5).abs() < 0.00001);
    }
}

fn analyzer(rig: &mut Rig) -> NodeId {
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::SpectrumAnalyzer,
        channels: 2,
    }));
    let node = rig.node(NodeKind::SpectrumAnalyzer);
    let graph = rig.c.graph.snapshot();
    let source = rig.node(NodeKind::Source);
    rig.request(AppRequest::EditGraph(GraphEdit::Connect {
        source: graph.graph.get_node(source).unwrap().outputs()[0].id(),
        destination: graph.graph.get_node(node).unwrap().inputs()[0].id(),
    }));
    rig.request(AppRequest::EditGraph(GraphEdit::SetSpectrumFft {
        node,
        fft_size: 64,
    }));
    rig.request(AppRequest::EditGraph(GraphEdit::SetSpectrumHop {
        node,
        hop_frames: 32,
    }));
    node
}

fn wide_graph(
    channels: usize,
) -> (
    moiren_core::graph::LogicalGraph,
    Vec<GraphNodeState>,
    NodeId,
) {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, channels).unwrap();
    let analyzer = graph
        .create_node(NodeKind::SpectrumAnalyzer, channels)
        .unwrap();
    let sink = graph.create_node(NodeKind::Sink, channels).unwrap();
    for (from, to) in [(source, analyzer), (analyzer, sink)] {
        graph
            .connect(
                graph.get_node(from).unwrap().outputs()[0].id(),
                graph.get_node(to).unwrap().inputs()[0].id(),
                Default::default(),
            )
            .unwrap();
    }
    let nodes = graph
        .nodes()
        .iter()
        .map(|node| GraphNodeState {
            id: node.id(),
            label: format!("{:?}", node.kind()),
            gain: 1.,
            pan: 0.,
            mono: false,
            swap: false,
            mixer: Default::default(),
            compressor: Default::default(),
            builtin: if node.kind() == NodeKind::SpectrumAnalyzer {
                BuiltinState::SpectrumAnalyzer(SpectrumSettings {
                    fft_size: 8192,
                    hop_frames: 8192,
                })
            } else {
                BuiltinState::for_kind(node.kind())
            },
            source: (node.kind() == NodeKind::Source)
                .then_some(InputSelection::Tone { frequency_hz: 440. }),
            output_endpoint_id: (node.kind() == NodeKind::Sink).then(|| "output".into()),
        })
        .collect();
    (graph, nodes, analyzer)
}

fn wide_spectrum(channels: usize) {
    let mut rig = Rig::new();
    let (graph, nodes, node) = wide_graph(channels);
    rig.c.graph.replace(graph, nodes).unwrap();
    rig.state.timed_meter_frames.store(256, Ordering::Release);
    rig.start();
    let binding = binding(&mut rig, node);
    let mut reader = subscribe(&mut rig, &binding);
    let (header, bins) = window(&mut rig, &mut reader);
    assert_eq!(header.channels, channels);
    assert_eq!(header.fft_size, 8192);
    assert_eq!(bins.len(), channels * 4097);
    for channel in bins.as_chunks::<4097>().0 {
        assert!((channel[0] - 1.).abs() < 0.00001, "real source DC missing");
        assert!(channel[2..].iter().all(|value| value.abs() < 0.00001));
    }
}

#[test]
fn spectrum_real_engine_8192_fft_delivers_all_64_channels() {
    wide_spectrum(64);
}

#[test]
fn spectrum_real_engine_8192_fft_delivers_all_256_channels() {
    wide_spectrum(256);
}

#[test]
fn spectrum_channel_reshape_replaces_binding_after_new_owner_is_confirmed() {
    let mut rig = Rig::new();
    let (graph, nodes, node) = wide_graph(2);
    rig.c.graph.replace(graph, nodes).unwrap();
    rig.state.timed_meter_frames.store(256, Ordering::Release);
    rig.start();
    let old = binding(&mut rig, node);
    let mut reader = subscribe(&mut rig, &old);
    let _ = window(&mut rig, &mut reader);
    let (graph, nodes, same_node) = wide_graph(1);
    assert_eq!(same_node, node);
    rig.request(AppRequest::ReplaceGraph { graph, nodes });
    rig.tick();
    assert_eq!(rig.c.signals.topics.node_spectra[&node].channels, 2);
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| {
        r.c.signals
            .topics
            .node_spectra
            .get(&node)
            .is_some_and(|binding| binding.channels == 1)
    });
    assert_eq!(old.topic.status().lifecycle, TopicLifecycle::Closed);
    assert!(reader.read_latest().is_none());
    let new = binding(&mut rig, node);
    assert_ne!(new.topic.id(), old.topic.id());
    let mut reader = subscribe(&mut rig, &new);
    assert_eq!(window(&mut rig, &mut reader).1.len(), 4097);
}

fn binding(rig: &mut Rig, node: NodeId) -> SpectrumBinding {
    rig.wait(|r| r.c.signals.topics.node_spectra.contains_key(&node));
    rig.c.signals.topics.node_spectra[&node].clone()
}

fn subscribe(rig: &mut Rig, binding: &SpectrumBinding) -> Subscriber<SpectrumTopic> {
    let (mut ticket, request) =
        crate::service::signals::subscription_request(binding.topic.clone(), Default::default());
    rig.c.signals.execute(request);
    ticket.try_take().unwrap().unwrap()
}

fn window(rig: &mut Rig, subscriber: &mut Subscriber<SpectrumTopic>) -> (SpectrumWindow, Vec<f32>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        rig.tick();
        rig.c.signals.pump_if_due(Instant::now());
        if let Some(window) = subscriber.read_latest() {
            return (*window.header(), window.elements().to_vec());
        }
        assert!(
            Instant::now() < deadline,
            "spectrum not delivered: {:?}",
            rig.c.graph.snapshot().last_error
        );
    }
}

#[test]
fn spectrum_binding_is_active_only_and_delivers_actual_source_dc_on_demand() {
    let mut rig = Rig::new();
    let node = analyzer(&mut rig);
    assert!(rig.c.signals.topics.node_spectra.is_empty());
    rig.start();
    let binding = binding(&mut rig, node);
    assert_eq!(
        binding.settings,
        SpectrumSettings {
            fft_size: 64,
            hop_frames: 32
        }
    );
    assert_eq!(binding.channels, 2);
    assert_eq!(binding.topic.business_subscribers(), 0);
    for _ in 0..8 {
        rig.tick();
    }
    assert_eq!(binding.topic.stats().attempts, 0);
    let mut subscriber = subscribe(&mut rig, &binding);
    let (header, values) = window(&mut rig, &mut subscriber);
    assert_eq!(header.fft_size, 64);
    assert_eq!(header.channels, 2);
    assert_eq!(header.sample_rate, 48_000.);
    assert_eq!(header.frame_end - header.frame_start, 64);
    assert_eq!(values.len(), 66);
    for channel in values.as_chunks::<33>().0 {
        assert!(
            (channel[0] - 1.).abs() < 0.00001,
            "source DC amplitude, not synthetic UI data"
        );
        assert!(channel[2..].iter().all(|value| value.abs() < 0.00001));
    }
    drop(subscriber);
    rig.tick();
    let attempts = binding.topic.stats().attempts;
    for _ in 0..8 {
        rig.tick();
    }
    assert_eq!(binding.topic.stats().attempts, attempts);
    assert_eq!(binding.topic.business_subscribers(), 0);
}

#[test]
fn spectrum_paused_source_does_not_republish_retained_windows_and_stop_closes_queued_data() {
    let mut rig = Rig::new();
    let node = analyzer(&mut rig);
    rig.start();
    let binding = binding(&mut rig, node);
    let mut subscriber = subscribe(&mut rig, &binding);
    let _ = window(&mut rig, &mut subscriber);
    rig.state.pause_audio.store(true, Ordering::Release);
    // Allow any in-flight callback and both bounded fanout stages to settle.
    for _ in 0..50 {
        rig.tick();
        rig.c.signals.pump_if_due(Instant::now());
        let _ = subscriber.read_latest();
    }
    let attempts = binding.topic.stats().attempts;
    for _ in 0..10 {
        rig.tick();
        rig.c.signals.pump_if_due(Instant::now());
        assert!(subscriber.read_latest().is_none());
    }
    assert_eq!(binding.topic.stats().attempts, attempts);
    rig.state.pause_audio.store(false, Ordering::Release);
    rig.wait(|_| binding.topic.stats().attempts > attempts);
    rig.c.signals.pump_if_due(Instant::now());
    rig.request(AppRequest::Stop);
    assert_eq!(binding.topic.status().lifecycle, TopicLifecycle::Closed);
    assert!(
        subscriber.read_latest().is_none(),
        "Stop must invalidate queued latest data"
    );
    for _ in 0..5 {
        rig.tick();
    }
    assert!(rig.c.signals.topics.node_spectra.is_empty());
}

#[test]
fn spectrum_load_reused_node_id_stays_with_active_owner_until_apply_then_closes_old_topic() {
    let mut rig = Rig::new();
    let node = analyzer(&mut rig);
    rig.start();
    let old = binding(&mut rig, node);
    let mut reader = subscribe(&mut rig, &old);
    let _ = window(&mut rig, &mut reader);
    let old_host = rig.driver.owner.as_ref().unwrap().observer.read().host_id;
    let document = crate::project::ProjectDocument::from_graph(
        "Reused spectrum identity",
        &rig.c.graph.snapshot(),
    )
    .unwrap();
    rig.request(AppRequest::LoadProject(document));
    rig.tick();
    assert_eq!(
        rig.c.signals.topics.node_spectra[&node].topic.id(),
        old.topic.id()
    );
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| {
        r.driver
            .owner
            .as_ref()
            .is_some_and(|owner| owner.observer.read().host_id != old_host)
            && r.c
                .signals
                .topics
                .node_spectra
                .get(&node)
                .is_some_and(|binding| binding.topic.id() != old.topic.id())
    });
    assert_eq!(old.topic.status().lifecycle, TopicLifecycle::Closed);
    assert!(reader.read_latest().is_none());
    let current = binding(&mut rig, node);
    let mut reader = subscribe(&mut rig, &current);
    let (header, bins) = window(&mut rig, &mut reader);
    assert_eq!(header.fft_size, 64);
    assert_eq!(bins.len(), 66);
}

#[test]
fn spectrum_hop_only_change_recreates_binding_and_rejects_cached_old_data() {
    let mut rig = Rig::new();
    let node = analyzer(&mut rig);
    rig.start();
    let old = binding(&mut rig, node);
    let mut subscriber = subscribe(&mut rig, &old);
    let _ = window(&mut rig, &mut subscriber);
    rig.state.block_update.store(true, Ordering::Release);
    rig.request(AppRequest::EditGraph(GraphEdit::SetSpectrumHop {
        node,
        hop_frames: 16,
    }));
    assert_eq!(
        rig.c.signals.topics.node_spectra[&node].topic.id(),
        old.topic.id()
    );
    rig.request(AppRequest::ApplyGraph);
    rig.state.block_update.store(false, Ordering::Release);
    rig.wait(|r| {
        r.c.signals
            .topics
            .node_spectra
            .get(&node)
            .is_some_and(|b| b.settings.hop_frames == 16)
    });
    let new = rig.c.signals.topics.node_spectra[&node].clone();
    assert_ne!(new.topic.id(), old.topic.id());
    assert_eq!(old.topic.status().lifecycle, TopicLifecycle::Closed);
    assert!(subscriber.read_latest().is_none());
    let mut subscriber = subscribe(&mut rig, &new);
    let (header, _) = window(&mut rig, &mut subscriber);
    assert_eq!(header.fft_size, 64);
}

#[test]
fn spectrum_fft_reshape_stop_restart_and_removal_retire_old_bindings() {
    let mut rig = Rig::new();
    let node = analyzer(&mut rig);
    rig.start();
    let old = binding(&mut rig, node);
    let mut subscriber = subscribe(&mut rig, &old);
    let _ = window(&mut rig, &mut subscriber);
    rig.request(AppRequest::EditGraph(GraphEdit::SetSpectrumFft {
        node,
        fft_size: 128,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| {
        r.c.signals
            .topics
            .node_spectra
            .get(&node)
            .is_some_and(|b| b.settings.fft_size == 128)
    });
    assert_eq!(old.topic.status().lifecycle, TopicLifecycle::Closed);
    assert!(subscriber.read_latest().is_none());
    let resized = binding(&mut rig, node);
    let mut subscriber = subscribe(&mut rig, &resized);
    assert_eq!(window(&mut rig, &mut subscriber).1.len(), 130);
    rig.request(AppRequest::Stop);
    assert!(rig.c.signals.topics.node_spectra.is_empty());
    assert_eq!(resized.topic.status().lifecycle, TopicLifecycle::Closed);
    rig.wait(|r| r.driver.cleaning == 0 && r.c.core.phase() == SessionPhase::Idle);
    rig.start();
    let restarted = binding(&mut rig, node);
    assert_ne!(restarted.topic.id(), resized.topic.id());
    let mut subscriber = subscribe(&mut rig, &restarted);
    let _ = window(&mut rig, &mut subscriber);
    rig.request(AppRequest::EditGraph(GraphEdit::RemoveNode(node)));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| !r.c.signals.topics.node_spectra.contains_key(&node));
    assert_eq!(restarted.topic.status().lifecycle, TopicLifecycle::Closed);
    assert!(subscriber.read_latest().is_none());
}
