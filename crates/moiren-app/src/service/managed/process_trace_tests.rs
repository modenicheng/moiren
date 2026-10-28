//! Real managed host retirement with a private, software-only trace source.
//! The source uses actual bridge histories but remains CallerSupplied; only
//! audio-crate retained-handle tests establish native origin.
use super::*;
use crate::service::managed::process_traces::{Collection, SessionTrace};
use moiren_windows_audio::{
    clock_bridge::{
        BridgeObserver, CaptureIngress, CapturePacket, ClockBridgeConfig, ClockSource,
        capture_bridge,
    },
    process_loopback::{
        ObservedProcessTrace, ProcessIdentity, ProcessTraceArchive, ProcessTraceCollector,
    },
};
use std::sync::atomic::AtomicUsize;

struct SoftwareCollection {
    ingress: Option<CaptureIngress>,
    source: Option<ClockSource>,
    bridge: BridgeObserver,
    collector: ProcessTraceCollector,
    stops: Arc<State>,
    final_polls: Arc<AtomicUsize>,
}

impl Collection for SoftwareCollection {
    fn refusal(&self) -> Option<String> {
        self.collector
            .collection_refusal()
            .map(|e| format!("{e:?}"))
    }
    fn poll(&mut self) {
        if self.stops.stops.load(Ordering::SeqCst) != 0 && self.ingress.is_some() {
            self.ingress
                .as_mut()
                .unwrap()
                .push_packet_with_publication_clock(
                    &[],
                    CapturePacket {
                        frames: 4,
                        flags: moiren_windows_audio::stats::SILENT,
                        device_position_frames: 0,
                        qpc_100ns: 0,
                    },
                    || Some(2000),
                )
                .unwrap();
            self.source
                .as_mut()
                .unwrap()
                .read_interleaved_with_publication_clock(&mut [0.; 8], || Some(2010))
                .unwrap();
            self.ingress.take();
            self.source.take();
            self.final_polls.fetch_add(1, Ordering::SeqCst);
        }
        self.collector.poll(&self.bridge);
    }

    fn artifact(&self) -> ProcessTraceArchive {
        self.collector.artifact()
    }

    fn ordered_trace(&self) -> Result<ObservedProcessTrace, String> {
        let a = self.collector.artifact();
        a.trace(&a.process, a.owner_generation)
            .map_err(|e| e.to_string())
    }
}

fn install(r: &mut Rig) -> Arc<AtomicUsize> {
    let owner = r.driver.owner.as_ref().unwrap();
    let host = owner.observer.read();
    let process = ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "software-fixture.exe".into(),
    };
    let (mut ingress, mut source, bridge) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    ingress
        .push_packet_with_publication_clock(
            &[],
            CapturePacket {
                frames: 4,
                flags: moiren_windows_audio::stats::SILENT,
                device_position_frames: 0,
                qpc_100ns: 0,
            },
            || Some(1000),
        )
        .unwrap();
    source
        .read_interleaved_with_publication_clock(&mut [0.; 8], || Some(1010))
        .unwrap();
    let final_polls = Arc::new(AtomicUsize::new(0));
    let origin = ProcessTraceSessionOrigin {
        generation: owner.generation,
        document: owner.document,
        node: r.node(NodeKind::Source),
        process: process.clone(),
        native_owner_generation: host.state.owner_generation,
        host_revision: owner.active_revision.0,
        graph_revision: owner.active_revision.1,
        audio_settings: owner.audio_settings.clone(),
        transport_latency: owner.transport_latency,
        latency_profile: owner.profile,
    };
    r.driver.process_trace = Some(SessionTrace::new(
        origin,
        Box::new(SoftwareCollection {
            ingress: Some(ingress),
            source: Some(source),
            bridge,
            collector: ProcessTraceCollector::new(
                process,
                host.state.owner_generation,
                48_000,
                48_000,
                4,
            ),
            stops: r.state.clone(),
            final_polls: final_polls.clone(),
        }),
    ));
    final_polls
}

fn start_process(r: &mut Rig) {
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Process {
            pid: 42,
            creation_time_100ns: 123,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.,
        pan: 0.,
        max_block_frames: 256,
    }));
    r.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.graph.snapshot().active_revision == Some(r.c.graph.snapshot().candidate_revision)
            && r.driver.confirm_parameters.is_none()
    });
}

#[test]
fn managed_prefix_real_admission_rejects_unsettled_and_mismatched_requests() {
    let mut r = Rig::new();
    start_process(&mut r);
    let host = r.driver.owner.as_ref().unwrap().observer.read();
    let origin = r.driver.process_trace_origin(&r.c, &host).unwrap().unwrap();
    assert_eq!(origin.process.pid, 42);
    assert_eq!(origin.node, r.node(NodeKind::Source));
    for fault in 0..8 {
        let mut changed = host.clone();
        match fault {
            0 => {
                r.driver.apply = true;
            }
            1 => {
                r.driver.auto_apply_deadline = Some(Instant::now());
            }
            2 => {
                r.c.audio_settings.processing.block_frames *= 2;
            }
            3 => {
                changed.state.execution = ExecutionState::Preparing;
            }
            4 => {
                changed.state.active_revision = Some(999);
            }
            5 => {
                r.c.graph
                    .preparing(r.c.core.generation(), origin.graph_revision);
            }
            6 => {
                r.driver.document += 1;
            }
            _ => {
                changed.state.desired_revision += 1;
            }
        }
        assert!(
            r.driver
                .process_trace_origin(&r.c, &changed)
                .unwrap()
                .is_none(),
            "fault {fault}"
        );
        r.driver.apply = false;
        r.driver.auto_apply_deadline = None;
        r.c.audio_settings = origin.audio_settings.clone();
        r.c.graph.clear_pending();
        r.driver.document = origin.document;
    }
    let mut changed = host.clone();
    let mut active = (**changed.active.as_ref().unwrap()).clone();
    if let host::NodeConfig::Input(input) = active.nodes.get_mut(&origin.node).unwrap() {
        input.source = moiren_engine::node::InputSource::Software {
            name: "not the Process source".into(),
        };
    }
    changed.active = Some(Arc::new(active));
    assert!(
        r.driver
            .process_trace_origin(&r.c, &changed)
            .unwrap()
            .is_none()
    );
    assert!(
        r.driver.process_trace.is_none(),
        "unavailable registry cannot be forged from a DTO"
    );
}

#[test]
fn managed_prefix_multiple_or_missing_process_sources_are_explicitly_bounded() {
    let mut r = Rig::new();
    r.start();
    let host = r.driver.owner.as_ref().unwrap().observer.read();
    assert!(
        r.driver
            .process_trace_origin(&r.c, &host)
            .unwrap()
            .is_none()
    );
    start_process(&mut r);
    r.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Source,
        channels: 2,
    }));
    let other =
        r.c.graph
            .snapshot()
            .nodes
            .iter()
            .find(|node| {
                node.id != r.node(NodeKind::Source)
                    && r.c
                        .graph
                        .snapshot()
                        .graph
                        .get_node(node.id)
                        .is_ok_and(|node| node.kind() == NodeKind::Source)
            })
            .unwrap()
            .id;
    r.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: other,
        source: InputSelection::Process {
            pid: 43,
            creation_time_100ns: 456,
        },
    }));
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| {
        r.c.core.phase() == SessionPhase::Running
            && r.c.graph.snapshot().active_revision == Some(r.c.graph.snapshot().candidate_revision)
            && r.driver.confirm_parameters.is_none()
            && r.driver.auto_apply_deadline.is_none()
    });
    let host = r.driver.owner.as_ref().unwrap().observer.read();
    assert!(
        r.driver
            .process_trace_origin(&r.c, &host)
            .unwrap_err()
            .contains("exactly one")
    );
    r.tick();
    assert!(
        r.c.process_trace_status
            .as_ref()
            .unwrap()
            .refusal
            .as_ref()
            .unwrap()
            .contains("exactly one")
    );
    assert!(r.driver.process_trace.is_none());
}

#[test]
fn managed_prefix_running_refusal_is_visible_without_stop_and_current_settings_changes_are_fenced()
{
    struct Refused;
    impl Collection for Refused {
        fn poll(&mut self) {}
        fn refusal(&self) -> Option<String> {
            Some("EventLimit".into())
        }
        fn artifact(&self) -> ProcessTraceArchive {
            panic!("live status must not clone an archive")
        }
        fn ordered_trace(&self) -> Result<ObservedProcessTrace, String> {
            panic!("not finalized")
        }
    }
    let mut r = Rig::new();
    start_process(&mut r);
    install(&mut r);
    let origin = r.driver.process_trace.take().unwrap().origin;
    r.driver.process_trace = Some(SessionTrace::new(origin, Box::new(Refused)));
    r.tick();
    assert!(
        r.c.process_trace_status
            .as_ref()
            .unwrap()
            .refusal
            .as_ref()
            .unwrap()
            .contains("EventLimit")
    );
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
    r.driver.process_trace = None;
    install(&mut r);
    r.c.audio_settings.processing.block_frames *= 2;
    r.tick();
    assert!(r.c.process_trace_status.as_ref().unwrap().refusal.is_some());
    assert_eq!(r.c.core.phase(), SessionPhase::Running);
}

#[test]
fn managed_prefix_finalizes_after_real_host_join_and_survives_stop_snapshot_filtering() {
    let mut r = Rig::new();
    r.start();
    let final_polls = install(&mut r);
    let original = r.c.core.generation();
    r.request(AppRequest::Stop);
    assert!(r.c.last_cleanup.is_none());
    r.wait(|r| r.driver.cleaning == 0);
    let cleanup = r.c.last_cleanup.as_ref().unwrap();
    let trace = cleanup.process_trace.as_ref().unwrap();
    assert_eq!(trace.origin.generation, original);
    assert_eq!(final_polls.load(Ordering::SeqCst), 1);
    assert!(trace.refusal.is_none(), "{:?}", trace.refusal);
    assert_eq!(trace.artifact.packet_history.total_events, 2);
    assert!(
        trace.artifact.packet_history.writer_finished
            && trace.artifact.queue_history.writer_finished
    );
    assert!(trace.ordered_trace().is_ok());
    let projected = crate::service::snapshot(&r.c);
    let shared = projected
        .last_cleanup
        .as_ref()
        .unwrap()
        .process_trace
        .as_ref()
        .unwrap();
    assert!(Arc::ptr_eq(shared, trace));
    assert!(projected.native_observations.is_none());
    r.start();
    assert_ne!(r.c.core.generation(), original);
    assert_eq!(
        r.c.last_cleanup
            .as_ref()
            .unwrap()
            .process_trace
            .as_ref()
            .unwrap()
            .origin
            .generation,
        original
    );
}

#[test]
fn managed_prefix_preserves_partial_archive_but_refuses_cleanup_failure() {
    let mut r = Rig::new();
    r.start();
    let final_polls = install(&mut r);
    r.state.fail_stop.store(true, Ordering::SeqCst);
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.cleaning == 0);
    let cleanup = r.c.last_cleanup.as_ref().unwrap();
    let trace = cleanup.process_trace.as_ref().unwrap();
    assert!(cleanup.state.cleanup_error.is_some());
    assert_eq!(final_polls.load(Ordering::SeqCst), 1);
    assert_eq!(trace.artifact.packet_history.total_events, 2);
    assert!(trace.refusal.as_ref().unwrap().contains("cleanup"));
    assert!(trace.ordered_trace().is_err());
}

#[test]
fn managed_prefix_origin_change_is_latched_without_stopping_or_replacing_audio() {
    for fault in 0..5 {
        let mut r = Rig::new();
        r.start();
        install(&mut r);
        match fault {
            0 => r.driver.document += 1,
            1 => {
                r.driver
                    .process_trace
                    .as_mut()
                    .unwrap()
                    .origin
                    .native_owner_generation += 1
            }
            2 => {
                r.driver
                    .process_trace
                    .as_mut()
                    .unwrap()
                    .origin
                    .host_revision += 1
            }
            3 => {
                r.driver
                    .process_trace
                    .as_mut()
                    .unwrap()
                    .origin
                    .audio_settings
                    .processing
                    .block_frames *= 2
            }
            _ => {
                r.driver
                    .process_trace
                    .as_mut()
                    .unwrap()
                    .origin
                    .graph_revision += 1
            }
        }
        r.tick();
        assert_eq!(r.c.core.phase(), SessionPhase::Running);
        assert_eq!(r.state.stops.load(Ordering::SeqCst), 0);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.cleaning == 0);
        let trace =
            r.c.last_cleanup
                .as_ref()
                .unwrap()
                .process_trace
                .as_ref()
                .unwrap();
        assert!(trace.refusal.is_some(), "fault {fault}");
        assert_eq!(trace.artifact.packet_history.total_events, 2);
        assert!(trace.ordered_trace().is_err());
    }
}

#[test]
fn managed_prefix_rejects_a_recovery_or_revision_change_after_its_last_live_poll() {
    struct Closed(ProcessTraceArchive);
    impl Collection for Closed {
        fn poll(&mut self) {}
        fn artifact(&self) -> ProcessTraceArchive {
            self.0.clone()
        }
        fn ordered_trace(&self) -> Result<ObservedProcessTrace, String> {
            self.0
                .trace(&self.0.process, self.0.owner_generation)
                .map_err(|e| e.to_string())
        }
    }
    for recovery in [true, false] {
        let mut r = Rig::new();
        r.start();
        install(&mut r);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.cleaning == 0);
        let cleanup = r.c.last_cleanup.as_ref().unwrap();
        let original = cleanup.process_trace.as_ref().unwrap();
        assert!(original.ordered_trace().is_ok());
        let trace = SessionTrace::new(
            original.origin.clone(),
            Box::new(Closed(original.artifact.clone())),
        );
        let mut final_state = cleanup.state.clone();
        if recovery {
            final_state.owner_generation += 1;
        } else {
            final_state.desired_revision += 1;
        }
        let evidence = trace.finish(&final_state);
        assert!(evidence.refusal.is_some(), "recovery={recovery}");
        assert!(evidence.ordered_trace().is_err());
    }
}

#[test]
fn managed_prefix_final_evidence_is_immutable_and_projects_diagnostics_while_stopped() {
    let mut r = Rig::new();
    r.start();
    install(&mut r);
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.cleaning == 0);
    let rows = crate::ui::presentation::process_trace_rows(&crate::service::snapshot(&r.c));
    let text = format!("{rows:?}");
    assert!(text.contains("Process trace"), "{text}");
    assert!(text.contains("2 packets / 2 reads"), "{text}");
    assert!(text.contains("not latency qualification"), "{text}");
    let evidence =
        r.c.last_cleanup
            .as_ref()
            .unwrap()
            .process_trace
            .as_ref()
            .unwrap();
    let mut copy = evidence.artifact().clone();
    copy.owner_generation += 1;
    assert_ne!(
        evidence.origin().native_owner_generation,
        copy.owner_generation
    );
    let report = evidence
        .ordered_trace()
        .unwrap()
        .replay(
            &evidence.origin().process,
            evidence.origin().native_owner_generation,
            moiren_windows_audio::process_loopback::ProcessBridgeOptions {
                max_demand_frames: 4,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        report.owner_generation,
        evidence.origin().native_owner_generation
    );
}
