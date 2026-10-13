#[cfg(windows)]
use moiren_core::{
    graph::{LogicalGraph, NodeKind},
    protocol::ProcessorId,
};
#[cfg(windows)]
use moiren_engine::host::{
    BackendError, CancellationToken, EngineBackend, EngineProject, NodeConfig, PrepareMode,
    PrepareRequest,
};
use moiren_engine::node::{DefaultDeviceRole, EndpointSelector, InputSource};
use moiren_windows_audio::{
    engine_backend::{
        WindowsBackendConfig, physical_input, physical_output, process_input, tone_input,
    },
    process_loopback::ProcessIdentity,
};
use std::time::Duration;

#[test]
fn typed_helpers_preserve_default_roles_and_pinned_process_identity() {
    let selector = EndpointSelector::FollowDefault(DefaultDeviceRole::Communications);
    assert_eq!(
        physical_input(selector.clone()).source,
        InputSource::Physical(selector.clone())
    );
    let output = physical_output(selector);
    assert_eq!(output.channels, 2);
    let identity = ProcessIdentity {
        pid: 123,
        creation_time_100ns: 456,
        executable_name: "chosen.exe".into(),
    };
    let input = process_input(&identity).unwrap();
    assert!(
        matches!(input.source, InputSource::Application { executable, .. } if executable == "moiren:process:123:456")
    );
    assert!(
        process_input(&ProcessIdentity {
            creation_time_100ns: 0,
            ..identity
        })
        .is_err()
    );
    for frequency in [0.0, -1.0, 24_000.0, f64::NAN, f64::INFINITY] {
        assert!(tone_input(frequency).is_err());
    }
    assert!(tone_input(440.0).is_ok());
}

#[test]
fn probing_configuration_has_finite_bounds() {
    for interval in [
        Duration::ZERO,
        Duration::from_millis(99),
        Duration::from_secs(61),
    ] {
        assert!(
            WindowsBackendConfig {
                probe_interval: interval,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    for capacity in [0, 257] {
        assert!(
            WindowsBackendConfig {
                max_resources: capacity,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    assert!(WindowsBackendConfig::default().validate().is_ok());
}

#[cfg(windows)]
fn project() -> (
    EngineProject,
    moiren_core::graph::NodeId,
    moiren_core::graph::NodeId,
) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let mut project = EngineProject {
        graph,
        ..Default::default()
    };
    project
        .nodes
        .insert(source, NodeConfig::Input(tone_input(440.0).unwrap()));
    project.nodes.insert(
        sink,
        NodeConfig::Output(physical_output(EndpointSelector::PinnedEndpoint {
            endpoint_id: "deterministic-fake-endpoint".into(),
            stable_id: None,
        })),
    );
    (project, source, sink)
}

#[cfg(windows)]
#[test]
fn update_preparation_leaves_reused_io_unbound_and_never_creates_a_second_transport() {
    use moiren_windows_audio::engine_backend::WindowsBackend;
    let backend = WindowsBackend::new(WindowsBackendConfig {
        probe_interval: Duration::from_secs(60),
        ..Default::default()
    })
    .unwrap();
    let (project, source, sink) = project();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (signal_host, _) = bus.create_host(8).unwrap();
    let reused = [(source, ProcessorId(1)), (sink, ProcessorId(2))];
    let request = PrepareRequest {
        project: &project,
        mode: PrepareMode::Update,
        reuse: &reused,
        graph_revision: 2,
        owner_generation: 1,
        resource_generation: 1,
        signal_host: &signal_host,
    };
    let prepared = backend
        .prepare(request, &CancellationToken::default())
        .unwrap();
    assert!(prepared.transport.is_none());
    drop(prepared);
    assert!(backend.poll_resource().is_none());
    let before = std::time::Instant::now();
    drop(backend);
    assert!(before.elapsed() < Duration::from_secs(1));
}

#[cfg(windows)]
#[test]
fn static_unsupported_and_cancelled_requests_do_not_probe_or_prepare_native_streams() {
    use moiren_windows_audio::engine_backend::WindowsBackend;
    let backend = WindowsBackend::new(WindowsBackendConfig {
        probe_interval: Duration::from_secs(60),
        ..Default::default()
    })
    .unwrap();
    let (mut project, _, _) = project();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (signal_host, _) = bus.create_host(8).unwrap();
    project.processing.engine.processing_sr = 96_000.0;
    let prepare = |cancel: &CancellationToken| {
        backend.prepare(
            PrepareRequest {
                project: &project,
                mode: PrepareMode::Initial,
                reuse: &[],
                graph_revision: 1,
                owner_generation: 1,
                resource_generation: 1,
                signal_host: &signal_host,
            },
            cancel,
        )
    };
    assert!(matches!(
        prepare(&CancellationToken::default()),
        Err(BackendError::Unsupported(_))
    ));
    let cancel = CancellationToken::default();
    cancel.cancel();
    assert!(matches!(prepare(&cancel), Err(BackendError::Cancelled)));
    assert!(backend.poll_resource().is_none());
}

#[cfg(windows)]
#[test]
#[ignore = "requires a live default WASAPI output; the graph is muted throughout"]
fn silent_native_host_automatically_starts_updates_replaces_owner_and_closes() {
    use moiren_core::graph::SendParams;
    use moiren_engine::host::{EditOutcome, EngineHost, ExecutionState, HostConfig, HostState};
    use moiren_windows_audio::engine_backend::WindowsBackend;
    let (mut project, source, sink) = project();
    project.nodes.insert(
        sink,
        NodeConfig::Output(physical_output(EndpointSelector::FollowDefault(
            DefaultDeviceRole::Multimedia,
        ))),
    );
    let gain = project.graph.create_node(NodeKind::Gain, 2).unwrap();
    project
        .nodes
        .insert(gain, NodeConfig::Gain { initial: 0.0 });
    project
        .graph
        .connect(
            project.graph.get_node(source).unwrap().outputs()[0].id(),
            project.graph.get_node(gain).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let edge = project
        .graph
        .connect(
            project.graph.get_node(gain).unwrap().outputs()[0].id(),
            project.graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let backend = WindowsBackend::new(Default::default()).unwrap();
    let host = EngineHost::<f32>::open(HostConfig::default(), project, backend).unwrap();
    let observer = host.observer();
    let wait = |predicate: &dyn Fn(&moiren_engine::host::HostSnapshot) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let snapshot = observer.snapshot();
            if predicate(&snapshot) {
                break snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native host did not progress: {snapshot:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let initial = wait(&|snapshot| {
        snapshot.execution == ExecutionState::Running && snapshot.timeline.is_some()
    });
    let updated = host
        .control()
        .edit(move |project| {
            project.graph.disconnect(edge)?;
            let added = project.graph.create_node(NodeKind::Gain, 2)?;
            project
                .nodes
                .insert(added, NodeConfig::Gain { initial: 0.5 });
            project.graph.connect(
                project.graph.get_node(gain)?.outputs()[0].id(),
                project.graph.get_node(added)?.inputs()[0].id(),
                SendParams::default(),
            )?;
            project.graph.connect(
                project.graph.get_node(added)?.outputs()[0].id(),
                project.graph.get_node(sink)?.inputs()[0].id(),
                SendParams::default(),
            )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        updated.wait(Duration::from_secs(15)),
        EditOutcome::Activated {
            graph_revision: 2,
            ..
        }
    ));
    let active =
        wait(&|snapshot| snapshot.active_revision == Some(2) && snapshot.timeline.is_some());
    assert_eq!(active.owner_generation, initial.owner_generation);
    assert_eq!(
        active.timeline.unwrap().epoch,
        initial.timeline.unwrap().epoch
    );
    assert!(active.timeline.unwrap().revision > initial.timeline.unwrap().revision);
    let replaced = host
        .control()
        .edit(move |project| {
            let Some(NodeConfig::Output(config)) = project.nodes.get_mut(&sink) else {
                unreachable!();
            };
            config.target = moiren_engine::node::OutputTarget::Physical(
                EndpointSelector::FollowDefault(DefaultDeviceRole::Console),
            );
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        replaced.wait(Duration::from_secs(15)),
        EditOutcome::Activated {
            graph_revision: 3,
            ..
        }
    ));
    let restarted =
        wait(&|snapshot| snapshot.active_revision == Some(3) && snapshot.timeline.is_some());
    assert!(restarted.owner_generation > active.owner_generation);
    assert!(restarted.timeline.unwrap().epoch > active.timeline.unwrap().epoch);
    println!("initial={initial:?}\nhot_update={active:?}\nrestarted={restarted:?}");
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
    assert!(observer.snapshot().fault.is_none());
    assert!(observer.snapshot().cleanup_error.is_none());
}

#[cfg(windows)]
#[test]
#[ignore = "requires live default capture/render devices; the output gain is zero"]
fn muted_native_capture_and_render_owners_start_and_close_together() {
    use moiren_core::graph::SendParams;
    use moiren_engine::host::{EngineHost, ExecutionState, HostConfig, HostState};
    use moiren_windows_audio::engine_backend::WindowsBackend;
    let (mut project, source, sink) = project();
    project.nodes.insert(
        source,
        NodeConfig::Input(physical_input(EndpointSelector::FollowDefault(
            DefaultDeviceRole::Console,
        ))),
    );
    project.nodes.insert(
        sink,
        NodeConfig::Output(physical_output(EndpointSelector::FollowDefault(
            DefaultDeviceRole::Multimedia,
        ))),
    );
    let gain = project.graph.create_node(NodeKind::Gain, 2).unwrap();
    project
        .nodes
        .insert(gain, NodeConfig::Gain { initial: 0.0 });
    project
        .graph
        .connect(
            project.graph.get_node(source).unwrap().outputs()[0].id(),
            project.graph.get_node(gain).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    project
        .graph
        .connect(
            project.graph.get_node(gain).unwrap().outputs()[0].id(),
            project.graph.get_node(sink).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let host = EngineHost::<f32>::open(
        HostConfig::default(),
        project,
        WindowsBackend::new(Default::default()).unwrap(),
    )
    .unwrap();
    let observer = host.observer();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let snapshot = observer.snapshot();
        if snapshot.execution == ExecutionState::Running && snapshot.timeline.is_some() {
            println!("physical_capture_and_render={snapshot:?}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "native monitor did not start: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(host);
    assert_eq!(observer.snapshot().host, HostState::Closed);
    assert!(observer.snapshot().cleanup_error.is_none());
    assert!(observer.snapshot().fault.is_none());
}
