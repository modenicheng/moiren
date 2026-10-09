use moiren_app::host::*;
use moiren_core::{
    graph::{NodeKind, SendParams},
    protocol::{ApplyAt, ParamValue, ReplyCode},
};
use moiren_engine::{
    boundary::{BoundaryReport, ConstantSource, RtAudioSource},
    buffer::AudioBlockMut,
    processor::{CompressorSettings, Gain, ProcessContext},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[path = "host/allocation.rs"]
mod allocation;

fn prepare(capacity: usize) -> (AudioHost, HostRenderer) {
    AudioHost::prepare(HostConfig {
        max_block_frames: 8,
        control_capacity: capacity,
        ..Default::default()
    })
    .unwrap()
}
fn constant(value: f64) -> ConstantSource {
    ConstantSource { channels: 2, value }
}
fn render(renderer: &mut HostRenderer) -> [f32; 16] {
    let mut output = [f32::NAN; 16];
    renderer.render_interleaved(&mut output).unwrap();
    output
}
fn activate(host: &mut AudioHost, renderer: &mut HostRenderer) -> [f32; 16] {
    let revision = host.publish().unwrap();
    let output = render(renderer);
    assert!(host.poll().iter().any(
        |event| matches!(event, HostEvent::PlanApplied { revision: r, .. } if *r == revision)
    ));
    output
}

#[test]
fn empty_bus_mixes_independent_strips_and_gates() {
    let (mut host, mut renderer) = prepare(16);
    assert_eq!(render(&mut renderer), [0.0; 16]);
    let a = host.add_source(constant(1.0)).unwrap();
    let b = host.add_source(constant(0.25)).unwrap();
    assert_eq!(activate(&mut host, &mut renderer), [1.25; 16]);
    let gain = host.set_gain(a, 0.5, 0).unwrap();
    let pan = host.set_pan(b, 1.0, 0).unwrap();
    assert_eq!(gain.code, ReplyCode::Accepted);
    assert_eq!(pan.code, ReplyCode::Accepted);
    let out = render(&mut renderer);
    for frame in out.as_chunks::<2>().0 {
        assert_eq!(*frame, [0.5, 0.75]);
    }
    assert_eq!(host.poll().len(), 2);
    host.source_gate(a).unwrap().set_available(false);
    for frame in render(&mut renderer).as_chunks::<2>().0 {
        assert_eq!(*frame, [0.0, 0.25]);
    }
    host.source_gate(a).unwrap().set_available(true);
    host.source_gate(b).unwrap().set_available(false);
    assert_eq!(render(&mut renderer), [0.5; 16]);
    assert_eq!(host.runtime_snapshot().timeline, 40);
    assert_eq!(host.runtime_snapshot().peak, 0.5);
    assert!(host.finish(renderer).is_empty());
}

#[test]
fn first_audible_block_uses_initial_source_settings() {
    let (mut host, mut renderer) = prepare(16);
    let source = host
        .add_source_with_settings(
            constant(1.0),
            SourceSettings {
                gain: 0.05,
                pan: 1.0,
                available: true,
            },
        )
        .unwrap();
    for frame in activate(&mut host, &mut renderer).as_chunks::<2>().0 {
        assert_eq!(*frame, [0.0, 0.05]);
    }
    assert_eq!(host.graph_snapshot().sources[0].id, source);
    assert_eq!(
        host.runtime_snapshot().desired_revision,
        host.runtime_snapshot().active_revision
    );
    host.finish(renderer);
}

struct CountingSource {
    reads: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    value: f32,
}
impl Drop for CountingSource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
impl RtAudioSource<f32> for CountingSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, f32>) -> BoundaryReport {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let frames = output.frames();
        output.channel_mut(0).fill(self.value);
        output.channel_mut(1).fill(self.value);
        BoundaryReport {
            transferred_frames: frames,
            ..Default::default()
        }
    }
}

#[test]
fn retained_source_and_output_bridge_survive_recompile_and_replacement() {
    let (mut host, mut renderer) = prepare(16);
    let reads = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let a = host
        .add_source(CountingSource {
            reads: reads.clone(),
            drops: drops.clone(),
            value: 0.5,
        })
        .unwrap();
    assert_eq!(activate(&mut host, &mut renderer), [0.5; 16]);
    let b = host.add_source(constant(0.25)).unwrap();
    assert_eq!(activate(&mut host, &mut renderer), [0.75; 16]);
    host.remove_source(b).unwrap();
    assert_eq!(activate(&mut host, &mut renderer), [0.5; 16]);
    assert_eq!(reads.load(Ordering::Relaxed), 3);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    let old_gate = host.source_gate(a).unwrap();
    host.replace_source(a, constant(0.125)).unwrap();
    old_gate.set_available(false);
    assert_eq!(activate(&mut host, &mut renderer), [0.125; 16]);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(host.graph_snapshot().sources[0].generation, 2);
    host.finish(renderer);
}

#[test]
fn compressor_repeated_insert_change_remove_applies_new_settings() {
    let (mut host, mut renderer) = prepare(32);
    let source = host.add_source(constant(1.0)).unwrap();
    activate(&mut host, &mut renderer);
    for _ in 0..3 {
        host.set_compressor(
            source,
            Some(CompressorSettings {
                mix: 0.0,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_eq!(activate(&mut host, &mut renderer), [1.0; 16]);
        host.set_compressor(
            source,
            Some(CompressorSettings {
                threshold_db: -20.0,
                ratio: 100.0,
                attack_ms: 0.0,
                knee_db: 0.0,
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(activate(&mut host, &mut renderer).iter().all(|&s| s < 0.15));
        host.set_compressor(source, None).unwrap();
        assert_eq!(activate(&mut host, &mut renderer), [1.0; 16]);
    }
    let before = host.graph_snapshot().desired;
    assert!(matches!(
        host.set_compressor(
            source,
            Some(CompressorSettings {
                ratio: f64::NAN,
                ..Default::default()
            })
        ),
        Err(HostError::InvalidCompressor)
    ));
    assert_eq!(host.graph_snapshot().desired, before);
    host.finish(renderer);
}

#[test]
fn pending_backpressure_and_old_requests_have_terminal_receipts() {
    let (mut host, mut renderer) = prepare(2);
    let source = host.add_source(constant(1.0)).unwrap();
    activate(&mut host, &mut renderer);
    // Fill the reply queue, then refill the request queue before publication.
    let a = host.set_gain(source, 0.8, 0).unwrap();
    let b = host.set_pan(source, 0.0, 0).unwrap();
    render(&mut renderer);
    let c = host.set_gain(source, 0.6, 0).unwrap();
    let d = host.set_pan(source, 0.5, 0).unwrap();
    host.set_compressor(
        source,
        Some(CompressorSettings {
            mix: 0.0,
            ..Default::default()
        }),
    )
    .unwrap();
    let revision = host.publish().unwrap();
    assert!(matches!(host.publish(), Err(HostError::Busy)));
    assert!(matches!(
        host.add_source(constant(1.0)),
        Err(HostError::Busy)
    ));
    assert!(host.graph_snapshot().plan.active_revision < revision);
    render(&mut renderer);
    let events = host.poll();
    for (request, code) in [
        (a, ReplyCode::Applied),
        (b, ReplyCode::Applied),
        (c, ReplyCode::StaleRevision),
        (d, ReplyCode::StaleRevision),
    ] {
        assert!(events.iter().any(|event| matches!(event, HostEvent::Parameter(reply) if reply.request_id == request.request_id && reply.code == code)), "missing {request:?} in {events:?}");
    }
    assert_eq!(host.graph_snapshot().plan.active_revision, revision);
    host.finish(renderer);
}

#[test]
fn parameters_during_pending_stay_on_active_revision_and_cancel_is_reported() {
    let (mut host, mut renderer) = prepare(8);
    let source = host.add_source(constant(1.0)).unwrap();
    activate(&mut host, &mut renderer);
    let active = host.graph_snapshot().plan.active_revision;
    host.set_compressor(source, Some(Default::default()))
        .unwrap();
    let candidate = host.publish().unwrap();
    let receipt = host.set_gain(source, 0.25, 0).unwrap();
    assert_eq!(receipt.plan_revision, active);
    assert_eq!(receipt.code, ReplyCode::Accepted);
    host.cancel_pending();
    assert_eq!(render(&mut renderer), [0.25; 16]);
    let events = host.poll();
    assert!(
        events.iter().any(
            |e| matches!(e, HostEvent::PlanRejected { revision, .. } if *revision == candidate)
        )
    );
    assert_eq!(host.graph_snapshot().plan.active_revision, active);
    assert!(host.graph_snapshot().sources[0].compressor_node.is_none());
    host.finish(renderer);
}

#[test]
fn shutdown_drains_full_replies_and_future_requests_without_rendering() {
    let (mut host, mut renderer) = prepare(2);
    let source = host.add_source(constant(1.0)).unwrap();
    activate(&mut host, &mut renderer);
    let a = host.set_gain(source, 0.75, 0).unwrap();
    let b = host.set_pan(source, 0.0, 0).unwrap();
    render(&mut renderer);
    let node = host.graph_snapshot().sources[0].gain_node;
    let c = host
        .submit_parameter(
            node,
            Gain::LEVEL,
            ParamValue::Float(0.5),
            ApplyAt::Frame(1000),
            0,
        )
        .unwrap();
    let d = host
        .submit_parameter(
            node,
            Gain::LEVEL,
            ParamValue::Float(0.25),
            ApplyAt::Frame(1001),
            0,
        )
        .unwrap();
    host.add_source(constant(0.1)).unwrap();
    host.publish().unwrap();
    let events = host.finish(renderer);
    for (request, code) in [
        (a, ReplyCode::Applied),
        (b, ReplyCode::Applied),
        (c, ReplyCode::StaleRevision),
        (d, ReplyCode::StaleRevision),
    ] {
        assert_eq!(request.code, ReplyCode::Accepted);
        assert_eq!(events.iter().filter(|e| matches!(e, HostEvent::Parameter(reply) if reply.request_id == request.request_id && reply.code == code)).count(), 1);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, HostEvent::PlanRejected { .. }))
    );
}

#[test]
fn graph_commands_are_transactional_and_changed_sends_are_not_reused() {
    let (mut host, mut renderer) = prepare(16);
    let source = host.add_source(constant(1.0)).unwrap();
    activate(&mut host, &mut renderer);
    let snapshot = host.graph_snapshot();
    let edge = snapshot
        .desired
        .edges()
        .iter()
        .find(|e| e.dst() == snapshot.sink)
        .unwrap()
        .id();
    assert!(matches!(
        host.graph_command(GraphCommand::Disconnect(edge)),
        Err(HostError::ProtectedInfrastructure)
    ));
    assert!(matches!(
        host.graph_command(GraphCommand::RemoveNode(snapshot.sources[0].node)),
        Err(HostError::ProtectedInfrastructure)
    ));
    assert!(matches!(
        host.graph_command(GraphCommand::CreateNode {
            kind: NodeKind::Source,
            channels: 2
        }),
        Err(HostError::UnsupportedGraph)
    ));
    assert_eq!(host.graph_snapshot().desired, snapshot.desired);
    host.graph_command(GraphCommand::SetSend {
        edge,
        params: SendParams {
            gain: 0.25,
            ..Default::default()
        },
    })
    .unwrap();
    assert_eq!(activate(&mut host, &mut renderer), [0.25; 16]);
    assert_eq!(
        host.set_gain(source, f64::NAN, 0).unwrap().code,
        ReplyCode::InvalidValue
    );
    host.finish(renderer);
}

#[test]
fn actual_multi_source_render_and_swaps_never_allocate_or_deallocate() {
    let (mut host, mut renderer) = prepare(16);
    let source = host.add_source(constant(0.25)).unwrap();
    host.add_source(constant(0.5)).unwrap();
    host.publish().unwrap();
    let mut output = [0.0; 32];
    let (result, counts) =
        allocation::track_allocations(|| renderer.render_interleaved(&mut output));
    result.unwrap();
    assert_eq!(counts, (0, 0));
    assert_eq!(output, [0.75; 32]);
    host.poll();
    for settings in [
        Some(CompressorSettings {
            mix: 0.0,
            ..Default::default()
        }),
        None,
    ] {
        host.set_compressor(source, settings).unwrap();
        host.publish().unwrap();
        let (result, counts) =
            allocation::track_allocations(|| renderer.render_interleaved(&mut output));
        result.unwrap();
        assert_eq!(counts, (0, 0));
        assert_eq!(output, [0.75; 32]);
        host.poll();
    }
    host.finish(renderer);
}

#[test]
fn compile_failure_preserves_prepared_backend_for_corrected_graph() {
    let (mut host, mut renderer) = AudioHost::prepare(HostConfig {
        max_block_frames: 8,
        audio_byte_budget: 4096,
        ..Default::default()
    })
    .unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    host.add_source(CountingSource {
        reads: reads.clone(),
        drops: drops.clone(),
        value: 0.5,
    })
    .unwrap();
    let nodes: Vec<_> = (0..100)
        .map(|_| {
            let GraphEdit::Node(node) = host
                .graph_command(GraphCommand::CreateNode {
                    kind: NodeKind::Gain,
                    channels: 2,
                })
                .unwrap()
            else {
                panic!("node edit")
            };
            node
        })
        .collect();
    assert!(matches!(host.publish(), Err(HostError::Compile(_))));
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    assert_eq!(render(&mut renderer), [0.0; 16]);
    for node in nodes {
        host.graph_command(GraphCommand::RemoveNode(node)).unwrap();
    }
    assert_eq!(activate(&mut host, &mut renderer), [0.5; 16]);
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    host.finish(renderer);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}
