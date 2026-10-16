use super::*;
#[path = "test_backend.rs"]
mod test_backend;
use moiren_core::graph::NodeKind;
use std::sync::atomic::Ordering;
use test_backend::{FakeBackend, State};

struct Rig {
    driver: ManagedDriver,
    c: ServiceContext,
    pool: JobPool,
    state: Arc<State>,
}

#[test]
fn each_strip_publishes_its_own_post_fader_level_only_on_demand() {
    let mut rig = Rig::new();
    rig.start();
    let gain = rig.node(NodeKind::Gain);
    let pan = rig.node(NodeKind::Pan);
    let deadline = Instant::now() + Duration::from_secs(8);
    while !rig.c.signals.topics.node_levels.contains_key(&gain) {
        assert!(Instant::now() < deadline);
        rig.tick();
    }
    let topic = rig.c.signals.topics.node_levels[&gain].clone();
    assert_eq!(topic.business_subscribers(), 0);
    assert_eq!(
        rig.c.signals.topics.node_levels[&pan].business_subscribers(),
        0
    );
    let (mut ticket, request) =
        crate::service::signals::subscription_request(topic.clone(), Default::default());
    rig.c.signals.execute(request);
    let mut subscriber = ticket.try_take().unwrap().unwrap();
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: gain,
        state: crate::service::MixerState {
            gain: 0.5,
            ..Default::default()
        },
    }));
    rig.request(AppRequest::ApplyGraph);
    loop {
        rig.tick();
        rig.c.signals.pump_if_due(Instant::now());
        if let Some(window) = subscriber.read_latest()
            && window.header().frame_end > window.header().frame_start
            && window.elements().iter().all(|level| {
                (level.peak - 0.5).abs() < 0.00001 && (level.rms - 0.5).abs() < 0.00001
            })
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "gain strip must publish its actual post-fader samples: {:?}",
            rig.c.graph.snapshot().last_error
        );
    }
    assert_eq!(
        rig.c.signals.topics.node_levels[&pan].business_subscribers(),
        0
    );
    drop(subscriber);
    assert_eq!(topic.business_subscribers(), 0);
}

#[test]
fn compressor_updates_are_acknowledged_live_without_rebuilding_or_touching_other_nodes() {
    use crate::service::CompressorParameter;
    let mut rig = Rig::new();
    rig.start();
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Compressor,
        channels: 2,
    }));
    let compressor = rig.node(NodeKind::Compressor);
    let snapshot = rig.c.graph.snapshot();
    let source = rig.node(NodeKind::Source);
    let sink = rig.node(NodeKind::Sink);
    let final_edge = snapshot
        .graph
        .edges()
        .iter()
        .find(|edge| edge.dst() == sink)
        .unwrap()
        .id();
    rig.request(AppRequest::EditGraph(GraphEdit::Disconnect(final_edge)));
    for (source, destination) in [(source, compressor), (compressor, sink)] {
        rig.request(AppRequest::EditGraph(GraphEdit::Connect {
            source: snapshot.graph.get_node(source).unwrap().outputs()[0].id(),
            destination: snapshot.graph.get_node(destination).unwrap().inputs()[0].id(),
        }));
    }
    rig.request(AppRequest::EditGraph(GraphEdit::SetCompressor {
        node: compressor,
        parameter: CompressorParameter::Ratio,
        value: 1.,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.sample() == 1.
    });
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    rig.request(AppRequest::EditGraph(GraphEdit::SetCompressor {
        node: compressor,
        parameter: CompressorParameter::OutputGain,
        value: -6.020599913279624,
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && (rig.sample() - 0.5).abs() < 0.00001
    });
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.c.graph.snapshot().node(source).unwrap().mixer,
        snapshot.node(source).unwrap().mixer
    );
}

#[test]
fn mixer_edits_wait_for_apply_then_activate_without_changing_sends() {
    let mut rig = Rig::new();
    rig.start();
    let source = rig.node(NodeKind::Source);
    let before = rig.c.graph.snapshot();
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            gain: 0.5,
            ..Default::default()
        },
    }));
    for _ in 0..4 {
        rig.tick();
    }
    assert_eq!(rig.sample(), 1.);
    assert_eq!(
        rig.c.graph.snapshot().active_revision,
        before.active_revision
    );
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.sample() == 0.5
    });
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(rig.c.graph.snapshot().graph.edges(), before.graph.edges());
    rig.request(AppRequest::EditGraph(GraphEdit::SetMixer {
        node: source,
        state: crate::service::MixerState {
            muted: true,
            ..Default::default()
        },
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|rig| {
        rig.c.graph.snapshot().active_revision == Some(rig.c.graph.snapshot().candidate_revision)
            && rig.sample() == 0.
    });
}
impl Rig {
    fn new() -> Self {
        let backend = FakeBackend::default();
        let state = backend.0.clone();
        let driver = ManagedDriver::new(move |_| Ok(Box::new(backend.clone())));
        Self {
            driver,
            c: ServiceContext::default(),
            pool: JobPool::new().unwrap(),
            state,
        }
    }
    fn request(&mut self, request: AppRequest) {
        self.driver.request(&mut self.c, &mut self.pool, request);
    }
    fn tick(&mut self) {
        self.tick_with_budget(RoundBudget::default());
    }
    fn tick_with_budget(&mut self, budget: RoundBudget) {
        self.driver.poll(&mut self.c, &mut self.pool, budget);
        self.c.dispatch(&mut self.pool);
        std::thread::sleep(Duration::from_millis(2));
    }
    fn wait(&mut self, predicate: impl Fn(&Self) -> bool) {
        self.wait_with_budget(predicate, RoundBudget::default());
    }
    fn wait_with_budget(&mut self, predicate: impl Fn(&Self) -> bool, budget: RoundBudget) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate(self) {
            assert!(
                Instant::now() < deadline,
                "phase {:?}, graph {:?}",
                self.c.core.phase(),
                self.c.graph.snapshot()
            );
            self.tick_with_budget(budget);
        }
    }
    fn start(&mut self) {
        self.request(AppRequest::Start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.0,
            pan: 0.0,
            max_block_frames: 256,
        }));
        self.wait(|r| r.c.core.phase() == SessionPhase::Running);
    }
    fn confirmation_revision(&self) -> Option<(u64, u64)> {
        self.driver
            .confirm_parameters
            .map(|confirmation| (confirmation.revision, confirmation.host_revision))
    }
    fn sample(&self) -> f64 {
        f64::from_bits(self.state.last_sample.load(Ordering::SeqCst))
    }
    fn node(&self, kind: NodeKind) -> moiren_core::graph::NodeId {
        self.c
            .graph
            .snapshot()
            .graph
            .nodes()
            .iter()
            .find(|n| n.kind() == kind)
            .unwrap()
            .id()
    }
}

#[test]
fn late_topology_activation_preserves_newer_parameter_apply_ack_barrier() {
    let mut r = Rig::new();
    r.start();
    let original = r.c.graph.snapshot().active_revision;
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let topology_revision = r.c.graph.snapshot().candidate_revision;
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    let parameter_revision = r.c.graph.snapshot().candidate_revision;
    let no_parameter_submissions = RoundBudget {
        applied: 0,
        ..RoundBudget::default()
    };
    r.wait_with_budget(
        |r| r.confirmation_revision() == Some((parameter_revision, 2)),
        no_parameter_submissions,
    );
    assert!(!r.driver.confirmation_values.is_empty());
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait_with_budget(
        |r| {
            r.driver.edits.is_empty()
                && r.driver
                    .owner
                    .as_ref()
                    .unwrap()
                    .observer
                    .snapshot()
                    .active_revision
                    == Some(2)
        },
        no_parameter_submissions,
    );
    assert_eq!(r.confirmation_revision(), Some((parameter_revision, 2)));
    assert!(!r.driver.confirmation_values.is_empty());
    assert_eq!(r.c.graph.snapshot().active_revision, original);
    assert_eq!(
        r.c.graph.snapshot().pending_revision,
        Some(parameter_revision)
    );
    assert_ne!(
        r.driver.owner.as_ref().unwrap().active_revision.1,
        topology_revision
    );
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(parameter_revision));
    assert_eq!(r.sample(), 0.25);
}

#[test]
fn late_topology_activation_cannot_regress_completed_newer_parameter_apply() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    let parameter_revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.driver.owner.as_ref().unwrap().active_revision == (2, parameter_revision));
    assert_eq!(
        r.driver
            .owner
            .as_ref()
            .unwrap()
            .observer
            .snapshot()
            .active_revision,
        Some(1)
    );
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(parameter_revision));
    for _ in 0..10 {
        r.tick();
    }
    assert_eq!(
        r.driver.owner.as_ref().unwrap().active_revision,
        (2, parameter_revision)
    );
    assert_eq!(
        r.c.graph.snapshot().active_revision,
        Some(parameter_revision)
    );
}

#[test]
fn queued_same_structure_apply_requires_acks_despite_an_older_observer_basis() {
    use std::sync::atomic::AtomicBool;
    struct ReleaseOnDrop(Arc<AtomicBool>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let mut r = Rig::new();
    r.start();
    let original = r.c.graph.snapshot().active_revision;
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let release_on_drop = ReleaseOnDrop(release.clone());
    let callback_entered = entered.clone();
    let callback_release = release.clone();
    let _gate_receipt = r
        .driver
        .owner
        .as_ref()
        .unwrap()
        .control
        .edit(move |_| {
            callback_entered.store(true, Ordering::SeqCst);
            while !callback_release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })
        .unwrap();
    r.wait(|_| entered.load(Ordering::SeqCst));
    r.state.pause_audio.store(true, Ordering::SeqCst);
    let no_parameter_submissions = RoundBudget {
        applied: 0,
        ..RoundBudget::default()
    };
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.tick_with_budget(no_parameter_submissions);
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    r.tick_with_budget(no_parameter_submissions);
    let parameter_revision = r.c.graph.snapshot().candidate_revision;
    assert_eq!(
        r.driver
            .owner
            .as_ref()
            .unwrap()
            .observer
            .snapshot()
            .desired_revision,
        1
    );
    drop(release_on_drop);
    r.wait_with_budget(
        |r| r.confirmation_revision() == Some((parameter_revision, 2)),
        no_parameter_submissions,
    );
    assert!(!r.driver.confirmation_values.is_empty());
    assert_eq!(r.c.graph.snapshot().active_revision, original);
    r.state.pause_audio.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(parameter_revision));
    assert_eq!(r.sample(), 0.25);
}

#[test]
fn apply_ack_for_a_new_parameter_target_waits_for_pending_topology_mapping() {
    let mut r = Rig::new();
    r.start();
    let original = r.c.graph.snapshot().active_revision;
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    let gain = r.c.graph.snapshot().graph.nodes().last().unwrap().id();
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.confirmation_revision() == Some((revision, 2)));
    for _ in 0..10 {
        r.tick();
    }
    assert_eq!(r.c.graph.snapshot().active_revision, original);
    assert_eq!(r.c.graph.snapshot().pending_revision, Some(revision));
    assert!(r.c.graph.snapshot().last_error.is_none());
    assert!(
        r.driver
            .parameters
            .iter()
            .any(|pending| pending
                .confirmation
                .is_some_and(|confirmation| confirmation.revision == revision
                    && confirmation.host_revision == 2))
    );
    assert!(
        r.driver
            .parameters
            .iter()
            .all(|pending| !matches!(pending.receipt.outcome(), ParameterOutcome::Rejected(_)))
    );
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    assert!(matches!(
        r.c.control.last_result.unwrap().code,
        moiren_core::protocol::ReplyCode::Applied | moiren_core::protocol::ReplyCode::AppliedLate
    ));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn repeated_same_candidate_apply_keeps_its_own_future_target_ack_barrier() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| {
        r.confirmation_revision() == Some((revision, 2)) && r.driver.confirmation_values.is_empty()
    });
    for _ in 0..10 {
        r.tick();
    }
    let older = r
        .driver
        .parameters
        .iter()
        .find(|pending| {
            pending.confirmation.is_some_and(|confirmation| {
                confirmation.revision == revision && confirmation.host_revision == 2
            }) && pending.receipt.outcome() == ParameterOutcome::Accepted
        })
        .expect("new Gain request waits for topology mapping")
        .receipt
        .clone();
    r.request(AppRequest::ApplyGraph);
    r.wait(|_| older.outcome() == ParameterOutcome::Superseded);
    for _ in 0..3 {
        r.tick();
    }
    assert_eq!(r.c.graph.snapshot().candidate_revision, revision);
    assert_eq!(r.c.graph.snapshot().pending_revision, Some(revision));
    assert_eq!(r.confirmation_revision(), Some((revision, 2)));
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| {
        r.c.graph.snapshot().active_revision == Some(revision) && r.c.control.accepted_pending == 0
    });
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.c.core.begin_exit();
        self.driver.begin_exit(&mut self.c, &mut self.pool);
        self.wait(|r| r.driver.is_drained() && r.pool.is_idle() && r.c.pending_jobs() == 0);
    }
}

#[test]
fn compatible_topology_swap_retains_owner_epoch_and_confirms_latest_revision() {
    let mut r = Rig::new();
    r.start();
    let before = r.driver.owner.as_ref().unwrap().observer.snapshot();
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    let revision = r.c.graph.snapshot().candidate_revision;
    assert_ne!(r.c.graph.snapshot().active_revision, Some(revision));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    let after = r.driver.owner.as_ref().unwrap().observer.snapshot();
    assert_eq!(before.owner_generation, after.owner_generation);
    assert_eq!(
        before.timeline.unwrap().epoch,
        after.timeline.unwrap().epoch
    );
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(r.sample(), 1.0);
}

#[test]
fn rejected_compilation_preserves_running_output_and_active_revision() {
    let mut r = Rig::new();
    r.start();
    let active = r.c.graph.snapshot().active_revision;
    let receipt = r
        .driver
        .owner
        .as_ref()
        .unwrap()
        .control
        .edit(|p| {
            p.processing.audio_byte_budget = 1;
            Ok(())
        })
        .unwrap();
    r.wait(|r| {
        matches!(
            r.driver.owner.as_ref().unwrap().observer.snapshot().update,
            UpdateState::Blocked { .. }
        )
    });
    assert!(matches!(receipt.outcome(), EditOutcome::Accepted { .. }));
    assert_eq!(r.c.core.phase(), SessionPhase::Running);
    assert_eq!(r.c.graph.snapshot().active_revision, active);
    assert_eq!(r.sample(), 1.0);
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn node_send_and_stereo_parameters_receive_actual_realtime_acks() {
    let mut r = Rig::new();
    r.start();
    let gain = r.node(NodeKind::Gain);
    let pan = r.node(NodeKind::Pan);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.5,
    }));
    r.request(AppRequest::EditGraph(GraphEdit::SetPan {
        node: pan,
        pan: 0.5,
    }));
    r.request(AppRequest::EditGraph(GraphEdit::SetMono {
        node: pan,
        mono: true,
    }));
    r.wait(|r| {
        (r.sample() - 0.375).abs() < 1e-6
            && r.c.control.accepted_pending == 0
            && r.c.control.pending_node_parameters == 0
    });
    assert!(matches!(
        r.c.control.last_result.unwrap().code,
        moiren_core::protocol::ReplyCode::Applied | moiren_core::protocol::ReplyCode::AppliedLate
    ));
    let edge = r.c.graph.snapshot().graph.edges()[2].id();
    r.request(AppRequest::EditGraph(GraphEdit::SetSend {
        edge,
        params: moiren_core::graph::SendParams {
            gain: 0.5,
            ..Default::default()
        },
    }));
    r.wait(|r| {
        (r.sample() - 0.1875).abs() < 1e-6
            && r.c.control.accepted_pending == 0
            && r.c.control.pending_node_parameters == 0
    });
}

#[test]
fn loaded_document_equal_node_ids_cannot_control_previous_project() {
    let mut r = Rig::new();
    r.start();
    let old_host = r.driver.owner.as_ref().unwrap().observer.read().host_id;
    let snapshot = r.c.graph.snapshot();
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::ReplaceGraph {
        graph: snapshot.graph,
        nodes: snapshot.nodes,
    });
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    for _ in 0..10 {
        r.tick();
    }
    assert_eq!(r.sample(), 1.0);
    assert_eq!(r.c.control.accepted_pending, 0);
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.core.phase() == SessionPhase::Running && (r.sample() - 0.25).abs() < 1e-6);
    assert_ne!(
        old_host,
        r.driver.owner.as_ref().unwrap().observer.read().host_id
    );
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn stop_cancels_pending_preparation_and_destroys_owners_off_service_thread() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    r.request(AppRequest::Stop);
    assert_eq!(r.c.active_latency_profile, None);
    r.wait(|r| r.driver.is_drained() && r.c.core.phase() == SessionPhase::Idle);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
    assert!(
        r.state
            .drops
            .lock()
            .unwrap()
            .iter()
            .all(|name| name != "fake-audio-owner" && name != "app-service")
    );
}

#[test]
fn legacy_meter_subscription_receives_managed_host_audio_windows() {
    let mut r = Rig::new();
    let (mut ticket, request) =
        signals::subscription_request(r.c.signals.topics.tone_output.clone(), Default::default());
    r.c.signals.execute(request);
    let mut subscriber = ticket.try_take().unwrap().unwrap();
    r.start();
    r.wait(|r| {
        r.driver
            .meters
            .as_ref()
            .is_some_and(|_| r.state.last_sample.load(Ordering::SeqCst) != 0)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        r.tick();
        r.c.signals.pump_if_due(Instant::now());
        if let Some(window) = subscriber.read_latest() {
            assert_eq!(window.elements()[0].peak, 1.0);
            assert_eq!(window.elements()[0].rms, 1.0);
            assert!(window.header().frame_end > window.header().frame_start);
            break;
        }
        assert!(Instant::now() < deadline, "managed meter did not publish");
    }
}

#[test]
fn multiple_inputs_mix_through_bus_and_feed_every_bound_output() {
    let mut r = Rig::new();
    for kind in [NodeKind::Source, NodeKind::Sink, NodeKind::Bus] {
        r.request(AppRequest::EditGraph(GraphEdit::AddNode {
            kind,
            channels: 2,
        }));
    }
    let snapshot = r.c.graph.snapshot();
    let source = snapshot.graph.nodes()[4].id();
    let output = snapshot.graph.nodes()[5].id();
    let bus = snapshot.graph.nodes()[6].id();
    let first_output = r.node(NodeKind::Sink);
    let pan = r.node(NodeKind::Pan);
    r.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: source,
        source: InputSelection::Tone {
            frequency_hz: 880.0,
        },
    }));
    r.request(AppRequest::EditGraph(GraphEdit::BindOutput {
        node: output,
        endpoint_id: "second".into(),
    }));
    r.request(AppRequest::EditGraph(GraphEdit::AddBusInput(bus)));
    r.request(AppRequest::EditGraph(GraphEdit::AddBusInput(bus)));
    let snapshot = r.c.graph.snapshot();
    r.request(AppRequest::EditGraph(GraphEdit::Disconnect(
        snapshot.graph.edges()[2].id(),
    )));
    let ports = [
        (
            snapshot.graph.get_node(pan).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(bus).unwrap().inputs()[0].id(),
        ),
        (
            snapshot.graph.get_node(source).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(bus).unwrap().inputs()[1].id(),
        ),
        (
            snapshot.graph.get_node(bus).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(first_output).unwrap().inputs()[0].id(),
        ),
        (
            snapshot.graph.get_node(bus).unwrap().outputs()[0].id(),
            snapshot.graph.get_node(output).unwrap().inputs()[0].id(),
        ),
    ];
    for (source, destination) in ports {
        r.request(AppRequest::EditGraph(GraphEdit::Connect {
            source,
            destination,
        }));
    }
    r.start();
    r.wait(|r| r.sample() == 2.0);
    assert_eq!(r.state.sources.load(Ordering::SeqCst), 2);
    assert_eq!(r.state.sinks.load(Ordering::SeqCst), 2);
}

#[test]
fn profile_selection_waits_until_apply_and_replaces_the_host() {
    let mut r = Rig::new();
    r.start();
    r.request(AppRequest::SetLatencyProfile(AudioLatencyProfile::Low));
    for _ in 0..3 {
        r.tick();
    }
    assert_eq!(r.c.latency_profile, AudioLatencyProfile::Low);
    assert_eq!(
        r.c.active_latency_profile,
        Some(AudioLatencyProfile::Compatible)
    );
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.active_latency_profile == Some(AudioLatencyProfile::Low));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn unavailable_resources_report_diagnostics_and_stop_cancels_host() {
    let mut r = Rig::new();
    r.state.unavailable.store(true, Ordering::SeqCst);
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "missing".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    r.wait(|r| r.c.graph.snapshot().last_error.is_some());
    assert_ne!(r.c.core.phase(), SessionPhase::Running);
    assert_eq!(r.c.graph.snapshot().active_revision, None);
    assert_eq!(r.c.active_latency_profile, None);
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.is_drained());
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn newest_apply_supersedes_pending_candidate_without_replacing_owner() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Gain,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.tick();
    let revision = r.c.graph.snapshot().candidate_revision;
    r.state.block_update.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(r.sample(), 1.0);
}

#[test]
fn initial_resource_recovery_restores_confirmed_revision_and_phase() {
    let mut r = Rig::new();
    r.state.unavailable.store(true, Ordering::SeqCst);
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    }));
    r.wait(|r| r.c.core.phase() == SessionPhase::Failed);
    let output = r.node(NodeKind::Sink);
    let project = r.driver.owner.as_ref().unwrap().observer.project();
    r.state.unavailable.store(false, Ordering::SeqCst);
    r.state
        .events
        .lock()
        .unwrap()
        .push_back(host::ResourceEvent {
            node: output,
            configuration: project.nodes[&output].clone(),
            generation: 1,
            available: true,
        });
    r.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.graph.snapshot().active_revision == Some(r.c.graph.snapshot().candidate_revision)
    });
    assert_eq!(r.c.core.error(), None);
    assert_eq!(
        r.c.active_latency_profile,
        Some(AudioLatencyProfile::Compatible)
    );
}

#[test]
fn service_exit_joins_managed_host_and_preserves_retained_observer_diagnostics() {
    let backend = FakeBackend::default();
    let state = backend.0.clone();
    let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
        Ok(Box::new(backend.clone()))
    }))
    .unwrap();
    let handle = runtime.handle();
    handle
        .try_request(AppRequest::Start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 0.0,
            pan: 0.0,
            max_block_frames: 256,
        }))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle
        .try_snapshot()
        .is_none_or(|s| s.phase != SessionPhase::Running)
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    handle.request_exit();
    while handle
        .try_snapshot()
        .is_none_or(|s| s.phase != SessionPhase::Exited)
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    runtime.join().unwrap();
    assert_eq!(state.stops.load(Ordering::SeqCst), 1);
    assert!(
        state
            .drops
            .lock()
            .unwrap()
            .iter()
            .all(|name| name != "app-service")
    );
}

#[test]
fn parameter_only_apply_waits_for_its_own_ack_when_older_requests_fill_the_ledger() {
    let mut r = Rig::new();
    for _ in 0..64 {
        r.request(AppRequest::EditGraph(GraphEdit::AddNode {
            kind: NodeKind::Gain,
            channels: 2,
        }));
    }
    r.start();
    let old = r.c.graph.snapshot().active_revision;
    r.state.pause_audio.store(true, Ordering::SeqCst);
    let nodes: Vec<_> =
        r.c.graph
            .snapshot()
            .graph
            .nodes()
            .iter()
            .filter(|n| n.kind() == NodeKind::Gain)
            .take(64)
            .map(|n| n.id())
            .collect();
    for node in nodes {
        r.request(AppRequest::EditGraph(GraphEdit::SetGain {
            node,
            gain: 0.5,
        }));
    }
    r.wait(|r| r.driver.parameters.len() == 64);
    r.request(AppRequest::ApplyGraph);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.driver.confirm_parameters.is_some());
    for _ in 0..5 {
        r.tick();
    }
    assert_eq!(r.c.graph.snapshot().active_revision, old);
    assert_eq!(r.c.graph.snapshot().pending_revision, Some(revision));
    r.state.pause_audio.store(false, Ordering::SeqCst);
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    // NextBlock commands may be consumed late under coordinator contention.
    // Both replies prove application; this test checks ACK ownership, not a deadline.
    assert!(matches!(
        r.c.control.last_result.unwrap().code,
        moiren_core::protocol::ReplyCode::Applied | moiren_core::protocol::ReplyCode::AppliedLate
    ));
}

#[test]
fn unchanged_apply_stays_pending_until_its_own_realtime_ack() {
    let mut rig = Rig::new();
    rig.start();
    let revision = rig.c.graph.snapshot().candidate_revision;
    rig.state.pause_audio.store(true, Ordering::SeqCst);
    // Let any already-entered callback finish before submitting new requests.
    for _ in 0..3 {
        rig.tick();
    }
    rig.request(AppRequest::ApplyGraph);
    rig.tick();
    assert_eq!(rig.c.graph.snapshot().active_revision, Some(revision));
    assert_eq!(rig.c.graph.snapshot().pending_revision, Some(revision));
    rig.wait(|rig| rig.driver.confirm_parameters.is_some());
    for _ in 0..5 {
        rig.tick();
        assert_eq!(rig.c.graph.snapshot().pending_revision, Some(revision));
    }
    rig.state.pause_audio.store(false, Ordering::SeqCst);
    rig.wait(|rig| rig.c.graph.snapshot().pending_revision.is_none());
    assert_eq!(rig.c.graph.snapshot().active_revision, Some(revision));
    assert!(rig.driver.confirm_parameters.is_none());
}

#[test]
fn later_live_edit_supersedes_parameter_confirmation_without_replaying_old_value() {
    let mut r = Rig::new();
    r.start();
    let gain = r.node(NodeKind::Gain);
    let old = r.c.graph.snapshot().active_revision;
    r.state.pause_audio.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.25,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.driver.confirm_parameters.is_some());
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.75,
    }));
    assert_eq!(r.driver.confirm_parameters, None);
    r.state.pause_audio.store(false, Ordering::SeqCst);
    r.wait(|r| {
        (r.sample() - 0.75).abs() < 1e-6
            && r.c.control.accepted_pending == 0
            && r.c.control.pending_node_parameters == 0
    });
    assert_eq!(r.c.graph.snapshot().active_revision, old);
    r.request(AppRequest::ApplyGraph);
    let revision = r.c.graph.snapshot().candidate_revision;
    r.wait(|r| r.c.graph.snapshot().active_revision == Some(revision));
    assert_eq!(r.sample(), 0.75);
}

#[test]
fn backend_factory_panic_is_a_failed_snapshot_and_service_can_exit() {
    let mut driver = ManagedDriver::new(|_| panic!("injected backend factory panic"));
    let mut c = ServiceContext::default();
    let mut pool = JobPool::new().unwrap();
    driver.request(
        &mut c,
        &mut pool,
        AppRequest::Start(SessionSpec {
            source: InputSelection::Tone {
                frequency_hz: 440.0,
            },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 0.0,
            pan: 0.0,
            max_block_frames: 256,
        }),
    );
    driver.poll(&mut c, &mut pool, RoundBudget::default());
    assert_eq!(c.core.phase(), SessionPhase::Failed);
    assert!(c.core.error().unwrap().contains("factory panicked"));
    assert!(driver.is_drained());
}

#[test]
fn replacement_document_fences_a_late_previous_project_preparation() {
    let mut r = Rig::new();
    r.start();
    r.state.block_update.store(true, Ordering::SeqCst);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Bus,
        channels: 2,
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.state.prepares.load(Ordering::SeqCst) >= 2);
    let snapshot = r.c.graph.snapshot();
    r.request(AppRequest::ReplaceGraph {
        graph: snapshot.graph,
        nodes: snapshot.nodes,
    });
    let gain = r.node(NodeKind::Gain);
    r.request(AppRequest::EditGraph(GraphEdit::SetGain {
        node: gain,
        gain: 0.75,
    }));
    r.state.block_update.store(false, Ordering::SeqCst);
    for _ in 0..20 {
        r.tick();
    }
    assert_eq!(r.sample(), 1.0);
    assert_ne!(
        r.c.graph.snapshot().active_revision,
        Some(r.c.graph.snapshot().candidate_revision)
    );
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.core.phase() == SessionPhase::Running && (r.sample() - 0.75).abs() < 1e-6);
    assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
}
