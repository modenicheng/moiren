use super::*;
use moiren_core::graph::NodeKind;
use moiren_engine::{
    compiler::{compile, compile_update},
    host::{BackendOwner, EngineProject, OwnerStatus},
    node::{InputConfig, OutputConfig, VirtualEndpointId},
};
#[path = "tests/fakes.rs"]
mod fakes;
use fakes::*;
use std::sync::atomic::Ordering;

#[path = "packet_phase_tests.rs"]
mod packet_phase;

fn virtual_project(channels: usize, rate: u32) -> EngineProject {
    let mut project = EngineProject::default();
    project.processing.engine.processing_sr = f64::from(rate);
    let source = project
        .graph
        .create_node(NodeKind::Source, channels)
        .unwrap();
    let sink = project.graph.create_node(NodeKind::Sink, channels).unwrap();
    project
        .graph
        .connect(
            project.graph.get_node(source).unwrap().outputs()[0].id(),
            project.graph.get_node(sink).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::VirtualPlayback {
                endpoint_id: VirtualEndpointId::from_bytes([1; 16]),
            },
            channels,
            device: Default::default(),
        }),
    );
    project.nodes.insert(
        sink,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::VirtualMicrophone {
                endpoint_id: VirtualEndpointId::from_bytes([2; 16]),
            },
            channels,
            device: Default::default(),
        }),
    );
    project
}

fn request<'a>(
    project: &'a EngineProject,
    signal_host: &'a moiren_signal::HostControl,
) -> PrepareRequest<'a> {
    PrepareRequest {
        project,
        mode: PrepareMode::Initial,
        reuse: &[],
        graph_revision: 1,
        owner_generation: 7,
        resource_generation: 1,
        signal_host,
    }
}

#[test]
fn virtual_mono_does_not_require_a_physical_master() {
    let project = virtual_project(1, 96_000);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let request = request(&project, &host);
    assert_eq!(validate(&request, true), Ok(96_000));
    assert_eq!(
        select_clock(&request, &[]),
        Ok(ProjectClock::VirtualRealtime)
    );
}

fn backend<S: ProcessingSample>(probe: &Arc<Probe>) -> ProfessionalBackend<S> {
    ProfessionalBackend::new()
        .unwrap()
        .with_virtual_provider(Arc::new(FakeProvider::new(probe.clone())))
        .with_virtual_clock_factory(Arc::new(FakeClockFactory(probe.clone())))
}

fn native_devices(faults: Faults) -> FakeNativeDevices {
    native_devices_absent(faults, Vec::new())
}

fn native_devices_absent(faults: Faults, absent: Vec<String>) -> FakeNativeDevices {
    FakeNativeDevices {
        probe: Arc::default(),
        faults,
        absent,
        unplug: Vec::new(),
        hang: Vec::new(),
        hang_gate: Gate::default(),
        open_barrier: None,
        teardown_gate: Gate::default(),
    }
}

/// A fake whose `unplug` endpoints die with the runtime-unplug absence error
/// on their first run-loop wait — a device yanked mid-run (issue #105).
fn native_devices_unplugging(unplug: Vec<String>) -> FakeNativeDevices {
    FakeNativeDevices {
        unplug,
        ..native_devices_absent(Default::default(), Vec::new())
    }
}

/// A fake whose `hang` endpoints block their stream open on the returned
/// gate until the test releases it — the wedged-driver shape of issue #105.
fn native_devices_hanging(faults: Faults, hang: Vec<String>) -> (FakeNativeDevices, Gate) {
    let gate = Gate::default();
    (
        FakeNativeDevices {
            probe: Arc::default(),
            faults,
            absent: Vec::new(),
            unplug: Vec::new(),
            hang,
            hang_gate: gate.clone(),
            open_barrier: None,
            teardown_gate: Gate::default(),
        },
        gate,
    )
}

fn physical_device(role: ClockRole) -> DeviceOptions {
    DeviceOptions {
        backend: AudioBackend::Asio,
        mode: DeviceMode::Exclusive,
        hardware_sample_rate: Some(96_000),
        sample_format: SampleFormat::Float64,
        period_frames: Some(128),
        clock_role: role,
    }
}

fn endpoint(id: &str) -> EndpointSelector {
    EndpointSelector::PinnedEndpoint {
        endpoint_id: id.into(),
        stable_id: None,
    }
}

fn add_physical_output(project: &mut EngineProject, channels: usize) -> NodeId {
    let sink = project.graph.create_node(NodeKind::Sink, channels).unwrap();
    let source = *project.nodes.keys().next().unwrap();
    if channels == 2 && project.graph.get_node(source).unwrap().outputs()[0].channels() == 2 {
        project
            .graph
            .connect(
                project.graph.get_node(source).unwrap().outputs()[0].id(),
                project.graph.get_node(sink).unwrap().inputs()[0].id(),
                Default::default(),
            )
            .unwrap();
    }
    project.nodes.insert(
        sink,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Physical(endpoint("native")),
            channels,
            device: physical_device(ClockRole::Master),
        }),
    );
    sink
}

fn replace_with_physical_capture(project: &mut EngineProject) -> NodeId {
    let source = *project.nodes.keys().next().unwrap();
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::Physical(endpoint("native")),
            channels: 2,
            device: physical_device(ClockRole::Follower),
        }),
    );
    source
}

fn attach<S: ProcessingSample>(
    project: &EngineProject,
    prepared: PreparedBackend<S>,
    cancel: CancellationToken,
) -> Box<dyn BackendOwner<S>> {
    let mut compiled = compile(
        &project.graph,
        prepared.bindings,
        project.processing.compile_config(1, 1),
    )
    .unwrap();
    compiled.engine.enable_plan_switching(2).unwrap();
    prepared
        .transport
        .unwrap()
        .attach(compiled.engine, &compiled.bindings, cancel)
        .unwrap_or_else(|failure| panic!("attach failed: {}", failure.error))
}

fn assert_sessions_joined(probe: &Probe, expected: usize) {
    assert_eq!(probe.prepared.load(Ordering::Acquire), expected);
    assert_eq!(probe.joined.load(Ordering::Acquire), expected);
    assert_eq!(probe.dropped.load(Ordering::Acquire), expected);
    assert!(probe.cancelled.load(Ordering::Acquire) >= expected);
}

fn virtual_matrix<S: ProcessingSample>() {
    for rate in SAMPLE_RATES {
        for channels in [1, 2] {
            let mut project = virtual_project(channels, rate);
            for config in project.nodes.values_mut() {
                let device = match config {
                    NodeConfig::Input(input) => &mut input.device,
                    NodeConfig::Output(output) => &mut output.device,
                    _ => unreachable!(),
                };
                device.backend = AudioBackend::Asio;
                device.mode = DeviceMode::Exclusive;
                device.sample_format = SampleFormat::Float64;
                device.hardware_sample_rate = Some(48_000);
            }
            let probe = Arc::new(Probe::default());
            let native = native_devices(Default::default());
            let backend = backend::<S>(&probe);
            let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
            let (host, _rt) = bus.create_host(8).unwrap();
            let cancel = CancellationToken::default();
            let prepared = backend
                .prepare_with_native(request(&project, &host), &cancel, &native)
                .unwrap();
            assert_eq!(probe.activated.load(Ordering::Acquire), 0);
            assert_eq!(native.probe.opened.load(Ordering::Acquire), 0);
            for request in probe.requests.lock().unwrap().iter() {
                assert_eq!(request.channels, channels);
                assert_eq!(request.engine_sample_rate, rate);
                assert_eq!(request.owner_generation, 7);
                assert_eq!(
                    request.max_block_frames,
                    project.processing.engine.max_block_frames
                );
            }
            // The finite provider seed can sit below the drift boundary's
            // content-prime threshold. Supply real native packets before the
            // owner starts; never bypass priming or weaken the PCM assertions.
            let packet = [0.25, -0.5].repeat(480);
            for playback in probe.playback.lock().unwrap().iter_mut() {
                let target = playback.capacity_frames() * 3 / 4;
                while playback.capacity_frames() - playback.available_frames() < target {
                    assert!(playback.write_interleaved_exact(&packet).unwrap());
                }
            }
            let owner = attach(&project, prepared, cancel);
            assert_eq!(owner.timeline().unwrap().frame, 0);
            owner.activate().unwrap();
            wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 512));
            owner.cancel();
            let returned = owner.stop_and_join();
            assert_eq!(returned.error, None);
            assert!(returned.engine.as_ref().unwrap().timeline() >= 512);
            assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 1);
            assert_eq!(probe.activated.load(Ordering::Acquire), 2);
            let mut samples = [0.0; 1024];
            let report = probe.microphones.lock().unwrap()[0]
                .read_interleaved(&mut samples)
                .unwrap();
            assert!(report.transferred_frames > 0);
            assert!(samples.as_chunks::<2>().0.iter().any(|pair| pair[0] > 0.1));
            if channels == 1 {
                assert!(samples.as_chunks::<2>().0.iter().all(|pair| pair[1] == 0.0));
            } else {
                assert!(samples.as_chunks::<2>().0.iter().any(|pair| pair[1] < -0.1));
            }
            assert_sessions_joined(&probe, 2);
        }
    }
}

#[test]
fn virtual_owner_runs_f32_mono_and_stereo_at_all_professional_rates() {
    virtual_matrix::<f32>();
}
#[test]
fn virtual_owner_runs_f64_mono_and_stereo_at_all_professional_rates() {
    virtual_matrix::<f64>();
}

#[test]
fn virtual_intent_without_provider_never_opens_native_devices_or_clock() {
    let mut project = virtual_project(2, 48_000);
    add_physical_output(&mut project, 2);
    let native = native_devices(Default::default());
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let result = ProfessionalBackend::<f64>::new()
        .unwrap()
        .prepare_with_native(
            request(&project, &host),
            &CancellationToken::default(),
            &native,
        );
    assert!(matches!(result, Err(BackendError::ResourceUnavailable(_))));
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 0);
}

#[test]
fn virtual_contract_and_native_constraints_are_validated_independently() {
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    for device in [
        DeviceOptions {
            hardware_sample_rate: Some(44_100),
            ..Default::default()
        },
        DeviceOptions {
            period_frames: Some(128),
            ..Default::default()
        },
        DeviceOptions {
            clock_role: ClockRole::Master,
            ..Default::default()
        },
    ] {
        let mut project = virtual_project(2, 48_000);
        let NodeConfig::Output(output) = project.nodes.values_mut().last().unwrap() else {
            unreachable!()
        };
        output.device = device;
        assert!(validate(&request(&project, &host), true).is_err());
    }
    for device in [
        DeviceOptions {
            hardware_sample_rate: Some(32_000),
            ..physical_device(ClockRole::Master)
        },
        DeviceOptions {
            period_frames: Some(100),
            ..physical_device(ClockRole::Master)
        },
    ] {
        let mut project = virtual_project(2, 48_000);
        let sink = add_physical_output(&mut project, 2);
        let NodeConfig::Output(output) = project.nodes.get_mut(&sink).unwrap() else {
            unreachable!()
        };
        output.device = device;
        assert!(matches!(
            validate(&request(&project, &host), true),
            Err(BackendError::Unsupported(_))
        ));
    }
    {
        // A follower output passes admission; the missing master is a clock
        // constraint, checked once endpoint presence is known.
        let mut project = virtual_project(2, 48_000);
        let sink = add_physical_output(&mut project, 2);
        let NodeConfig::Output(output) = project.nodes.get_mut(&sink).unwrap() else {
            unreachable!()
        };
        output.device = physical_device(ClockRole::Follower);
        let request = request(&project, &host);
        assert!(validate(&request, true).is_ok());
        assert!(matches!(
            select_clock(&request, &[]),
            Err(BackendError::Unsupported(_))
        ));
    }
    let mut mono = virtual_project(2, 48_000);
    add_physical_output(&mut mono, 1);
    assert!(validate(&request(&mono, &host), true).is_err());
    let mut duplicate = virtual_project(2, 48_000);
    let NodeConfig::Output(output) = duplicate.nodes.values_mut().last().unwrap() else {
        unreachable!()
    };
    output.target = OutputTarget::VirtualMicrophone {
        endpoint_id: VirtualEndpointId::from_bytes([1; 16]),
    };
    assert!(matches!(validate(&request(&duplicate, &host), true),
        Err(BackendError::Unsupported(message)) if message.contains("duplicate virtual")));
}

#[test]
fn prepared_streams_record_their_service_endpoint_identities() {
    // Every registration site records the identity the graph bound — the
    // service-side catalog row id — not the canonical id a native open
    // resolves to. ASIO pins `asio:{CLSID}`; the observation must keep that
    // exact string or the devices page cannot attribute the stream.
    // (The process-loopback site is the fourth registrant; its identity is
    // the parsed `moiren:process:{pid}:{creation}` pair, which needs a real
    // foreign process to exercise and is covered by construction.)
    let mut project = virtual_project(2, 48_000);
    let physical_id = "asio:{01234567-89AB-CDEF-0123-456789ABCDEF}".to_string();
    let physical_source = project.graph.create_node(NodeKind::Source, 2).unwrap();
    let physical_sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
    project
        .graph
        .connect(
            project.graph.get_node(physical_source).unwrap().outputs()[0].id(),
            project.graph.get_node(physical_sink).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    project.nodes.insert(
        physical_source,
        NodeConfig::Input(InputConfig {
            source: InputSource::Physical(endpoint(&physical_id)),
            channels: 2,
            device: physical_device(ClockRole::Follower),
        }),
    );
    project.nodes.insert(
        physical_sink,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Physical(endpoint(&physical_id)),
            channels: 2,
            device: physical_device(ClockRole::Master),
        }),
    );
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = backend::<f64>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    let streams = &backend.observations().snapshot().unwrap().streams;
    let mut identities: Vec<(NodeId, StreamEndpoint)> = streams
        .iter()
        .map(|stream| (stream.node, stream.endpoint.clone()))
        .collect();
    identities.sort_by_key(|(node, _)| format!("{node:?}"));
    let mut expected = vec![
        (
            physical_source,
            StreamEndpoint::Physical(physical_id.clone()),
        ),
        (physical_sink, StreamEndpoint::Physical(physical_id)),
    ];
    for (node, config) in &project.nodes {
        let endpoint = match config {
            NodeConfig::Input(input) => match &input.source {
                InputSource::VirtualPlayback { endpoint_id } => {
                    StreamEndpoint::Virtual(*endpoint_id)
                }
                _ => continue,
            },
            NodeConfig::Output(output) => match &output.target {
                OutputTarget::VirtualMicrophone { endpoint_id } => {
                    StreamEndpoint::Virtual(*endpoint_id)
                }
                _ => continue,
            },
            _ => continue,
        };
        expected.push((*node, endpoint));
    }
    expected.sort_by_key(|(node, _)| format!("{node:?}"));
    assert_eq!(identities, expected);
    owner.cancel();
    let _ = owner.stop_and_join();
}

#[test]
fn prepared_sessions_join_after_provider_binding_or_cancellation_failure() {
    let project = virtual_project(2, 44_100);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    for case in 0..3 {
        let probe = Arc::new(Probe::default());
        let mut provider = FakeProvider::new(probe.clone());
        provider.fail_microphone = case == 0;
        provider.short_carrier = case == 1;
        provider.cancel_after_playback = case == 2;
        let backend = ProfessionalBackend::<f64>::new()
            .unwrap()
            .with_virtual_provider(Arc::new(provider));
        let result = backend.prepare(request(&project, &host), &CancellationToken::default());
        assert!(result.is_err());
        assert_sessions_joined(&probe, 1);
        assert_eq!(probe.activated.load(Ordering::Acquire), 0);
    }
}

#[test]
fn native_prepare_failures_join_every_virtual_session_without_fallback() {
    let mut project = virtual_project(2, 88_200);
    add_physical_output(&mut project, 2);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    for faults in [
        Faults {
            open: true,
            ..Default::default()
        },
        Faults {
            wrong_rate: true,
            ..Default::default()
        },
        Faults {
            wrong_format: true,
            ..Default::default()
        },
        Faults {
            wrong_period: true,
            ..Default::default()
        },
    ] {
        let probe = Arc::new(Probe::default());
        let native = native_devices(faults);
        let backend = backend::<f32>(&probe);
        let result = backend.prepare_with_native(
            request(&project, &host),
            &CancellationToken::default(),
            &native,
        );
        assert!(result.is_err());
        assert_sessions_joined(&probe, 2);
        assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 0);
        assert_eq!(probe.activated.load(Ordering::Acquire), 0);
        assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
        assert_eq!(
            native.probe.dropped.load(Ordering::Acquire),
            usize::from(!faults.open)
        );
    }
}

#[test]
fn physical_master_keeps_one_engine_and_reports_virtual_session_errors() {
    let mut project = virtual_project(2, 96_000);
    add_physical_output(&mut project, 2);
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = backend::<f64>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
    owner.activate().unwrap();
    wait_until(|| {
        owner.status() == OwnerStatus::Running && owner.timeline().is_some_and(|t| t.frame > 0)
    });
    assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 0);
    assert_eq!(native.probe.master_groups.load(Ordering::Acquire), 1);
    probe.fail_session.store(true, Ordering::Release);
    assert_eq!(owner.status(), OwnerStatus::Failed(session_error()));
    let returned = owner.stop_and_join();
    assert_eq!(returned.error, Some(session_error()));
    assert_sessions_joined(&probe, 2);
    assert_eq!(native.probe.stopped.load(Ordering::Acquire), 1);
    assert_eq!(native.probe.released.load(Ordering::Acquire), 1);
    assert_eq!(native.probe.dropped.load(Ordering::Acquire), 1);
}

#[test]
fn virtual_master_runs_native_capture_as_follower_and_retains_actual_failure() {
    for faults in [
        Faults::default(),
        Faults {
            wait: true,
            recover: true,
            ..Default::default()
        },
        Faults {
            wait: true,
            recover: true,
            stop: true,
            ..Default::default()
        },
        Faults {
            wait: true,
            recover: true,
            release: true,
            ..Default::default()
        },
        Faults {
            wait: true,
            recover: true,
            drop: true,
            ..Default::default()
        },
    ] {
        let mut project = virtual_project(2, 44_100);
        replace_with_physical_capture(&mut project);
        let probe = Arc::new(Probe::default());
        let native = native_devices(faults);
        let backend = backend::<f64>(&probe);
        let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = bus.create_host(8).unwrap();
        let cancel = CancellationToken::default();
        let prepared = backend
            .prepare_with_native(request(&project, &host), &cancel, &native)
            .unwrap();
        let owner = attach(&project, prepared, cancel);
        owner.activate().unwrap();
        wait_until(|| native.probe.started.load(Ordering::Acquire) == 1);
        if faults.wait {
            wait_until(|| matches!(owner.status(), OwnerStatus::Failed(_)));
        } else {
            wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 256));
        }
        owner.cancel();
        let returned = owner.stop_and_join();
        let expected = if faults.drop {
            Some(BackendError::WorkerPanicked)
        } else if faults.release {
            Some(native_error("release"))
        } else if faults.stop {
            Some(native_error("stop"))
        } else if faults.wait {
            Some(native_error("wait"))
        } else {
            None
        };
        assert_eq!(returned.error, expected);
        let can_recover = faults.recover && !faults.stop && !faults.release && !faults.drop;
        assert_eq!(backend.poll_resource().is_some(), can_recover);
        assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 1);
        assert_eq!(native.probe.master_groups.load(Ordering::Acquire), 0);
        assert_eq!(native.probe.stopped.load(Ordering::Acquire), 1);
        assert_eq!(native.probe.released.load(Ordering::Acquire), 1);
        assert_eq!(native.probe.dropped.load(Ordering::Acquire), 1);
        assert_sessions_joined(&probe, 1);
    }
}

#[test]
fn capture_only_project_runs_on_the_virtual_clock_without_a_provider() {
    let mut project = virtual_project(2, 48_000);
    replace_with_physical_capture(&mut project);
    // Drop the virtual microphone sink: a capture-only project with no
    // outputs at all, paced purely by the QPC fallback clock.
    let sink = *project.nodes.keys().last().unwrap();
    project.nodes.remove(&sink);
    project.graph.remove_node(sink).unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    // A provider is only an admission requirement for virtual endpoints, so
    // the capture-only project validates without one.
    assert_eq!(validate(&request(&project, &host), false), Ok(48_000));
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_virtual_clock_factory(Arc::new(FakeClockFactory(probe.clone())));
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| native.probe.started.load(Ordering::Acquire) == 1);
    wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 256));
    owner.cancel();
    let returned = owner.stop_and_join();
    assert_eq!(returned.error, None);
    assert!(returned.engine.as_ref().unwrap().timeline() >= 256);
    assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 1);
    assert_eq!(native.probe.master_groups.load(Ordering::Acquire), 0);
    assert_eq!(native.probe.stopped.load(Ordering::Acquire), 1);
    assert_eq!(native.probe.released.load(Ordering::Acquire), 1);
    assert_eq!(native.probe.dropped.load(Ordering::Acquire), 1);
    // The capture native session is the only joined resource: no virtual
    // boundary sessions exist to prepare.
    assert_sessions_joined(&probe, 0);
}

#[test]
fn empty_project_runs_on_the_virtual_clock_without_native_devices() {
    let mut project = EngineProject::default();
    project.processing.engine.processing_sr = 48_000.0;
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    assert_eq!(validate(&request(&project, &host), false), Ok(48_000));
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_virtual_clock_factory(Arc::new(FakeClockFactory(probe.clone())));
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 0);
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 256));
    owner.cancel();
    let returned = owner.stop_and_join();
    assert_eq!(returned.error, None);
    assert!(returned.engine.as_ref().unwrap().timeline() >= 256);
    assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 1);
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 0);
    assert_sessions_joined(&probe, 0);
}

#[test]
fn physical_master_protects_the_virtual_microphone_across_plan_updates() {
    let mut project = virtual_project(2, 48_000);
    let virtual_sink = *project.nodes.keys().last().unwrap();
    add_physical_output(&mut project, 2);
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = backend::<f32>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let prepared = backend
        .prepare_with_native(
            request(&project, &host),
            &CancellationToken::default(),
            &native,
        )
        .unwrap();
    let mut compiled = compile(
        &project.graph,
        prepared.bindings,
        project.processing.compile_config(1, 1),
    )
    .unwrap();
    let mut plans = compiled.engine.enable_plan_switching(2).unwrap();
    let basis = compiled.engine.plan_snapshot();
    let owner = prepared
        .transport
        .unwrap()
        .attach(
            compiled.engine,
            &compiled.bindings,
            CancellationToken::default(),
        )
        .unwrap_or_else(|failure| panic!("{}", failure.error));
    let mut returned = owner.stop_and_join();
    assert_eq!(returned.error, None);
    project.graph.remove_node(virtual_sink).unwrap();
    project.nodes.remove(&virtual_sink);
    let reuse: Vec<_> = project
        .nodes
        .keys()
        .map(|&node| (node, compiled.bindings.node(node).unwrap()))
        .collect();
    let update = compile_update::<f32>(
        &project.graph,
        NodeBindings::new(),
        project.processing.compile_config(2, 1),
        &basis,
        &reuse,
        None,
    )
    .unwrap();
    plans
        .publish(update.plan)
        .unwrap_or_else(|_| panic!("publish candidate"));
    returned.engine.as_mut().unwrap().render(64).unwrap();
    assert_eq!(
        plans.active_revision(),
        1,
        "live virtual sink must migrate on update"
    );
    assert!(plans.poll_retired().is_some());
    assert_sessions_joined(&probe, 2);
}

#[test]
fn unchanged_virtual_io_update_reuses_bindings_without_new_sessions_or_clock() {
    let project = virtual_project(1, 96_000);
    let probe = Arc::new(Probe::default());
    let backend = backend::<f64>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let reuse: Vec<_> = project
        .nodes
        .keys()
        .enumerate()
        .map(|(index, &node)| (node, moiren_core::protocol::ProcessorId(index as u64)))
        .collect();
    let mut update = request(&project, &host);
    update.mode = PrepareMode::Update;
    update.reuse = &reuse;
    let prepared = backend
        .prepare(update, &CancellationToken::default())
        .unwrap();
    assert!(prepared.transport.is_none());
    assert!(
        project
            .nodes
            .keys()
            .all(|&node| !prepared.bindings.contains_node(node))
    );
    assert_eq!(probe.prepared.load(Ordering::Acquire), 0);
    assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 0);
    let mut changed = request(&project, &host);
    changed.mode = PrepareMode::Update;
    assert!(
        backend
            .prepare(changed, &CancellationToken::default())
            .is_err()
    );
    assert_eq!(probe.prepared.load(Ordering::Acquire), 0);
}

#[test]
fn prepared_drop_and_attach_failure_join_all_sessions_before_activation() {
    for physical_master in [false, true] {
        for attach_failure in [false, true] {
            let mut project = virtual_project(2, 48_000);
            if physical_master {
                add_physical_output(&mut project, 2);
            }
            let probe = Arc::new(Probe::default());
            let backend = backend::<f32>(&probe);
            let native = native_devices(Default::default());
            let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
            let (host, _rt) = bus.create_host(8).unwrap();
            let prepared = backend
                .prepare_with_native(
                    request(&project, &host),
                    &CancellationToken::default(),
                    &native,
                )
                .unwrap();
            if attach_failure {
                let empty = compile::<f32>(
                    &Default::default(),
                    NodeBindings::new(),
                    project.processing.compile_config(1, 1),
                )
                .unwrap();
                let failure = prepared
                    .transport
                    .unwrap()
                    .attach(empty.engine, &empty.bindings, CancellationToken::default())
                    .err()
                    .expect("missing sink must fail attachment");
                assert!(matches!(failure.error, BackendError::Unsupported(_)));
            } else {
                drop(prepared);
            }
            assert_eq!(probe.activated.load(Ordering::Acquire), 0);
            assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
            assert_sessions_joined(&probe, 2);
        }
    }
}

#[test]
fn virtual_clock_preparation_failure_retires_native_capture_and_virtual_resources() {
    struct FailedClock;
    impl VirtualClockFactory for FailedClock {
        fn prepare(
            &self,
            _: crate::StopSignal,
        ) -> Result<Box<dyn crate::virtual_owner::VirtualRealtimeClock>, BackendError> {
            Err(BackendError::ResourceUnavailable(
                "injected QPC preparation".into(),
            ))
        }
    }
    let mut project = virtual_project(2, 88_200);
    replace_with_physical_capture(&mut project);
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = backend::<f64>(&probe).with_virtual_clock_factory(Arc::new(FailedClock));
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let result = backend.prepare_with_native(
        request(&project, &host),
        &CancellationToken::default(),
        &native,
    );
    assert!(
        matches!(result, Err(BackendError::ResourceUnavailable(message)) if message.contains("QPC"))
    );
    assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
    assert_eq!(native.probe.dropped.load(Ordering::Acquire), 1);
    assert_sessions_joined(&probe, 1);
}

#[test]
fn explicit_cancellation_fences_capture_follower_recovery_authorization() {
    let mut project = virtual_project(2, 48_000);
    replace_with_physical_capture(&mut project);
    let probe = Arc::new(Probe::default());
    let native = native_devices(Faults {
        wait: true,
        recover: true,
        ..Default::default()
    });
    let backend = backend::<f32>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel.clone());
    owner.activate().unwrap();
    wait_until(|| native.probe.dropped.load(Ordering::Acquire) == 1);
    cancel.cancel();
    let returned = owner.stop_and_join();
    assert_eq!(returned.error, Some(native_error("wait")));
    assert!(backend.poll_resource().is_none());
    assert_sessions_joined(&probe, 1);
}

#[test]
fn physical_asio_duplex_with_virtual_microphone_opens_one_native_session() {
    let mut project = virtual_project(2, 96_000);
    replace_with_physical_capture(&mut project);
    add_physical_output(&mut project, 2);
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = backend::<f64>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| owner.timeline().is_some_and(|t| t.frame > 0));
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 1);
    assert_eq!(native.probe.master_groups.load(Ordering::Acquire), 1);
    assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 0);
    assert_sessions_joined(&probe, 1);
}

#[test]
fn virtual_endpoints_publish_stream_observations() {
    let project = virtual_project(2, 48_000);
    let source = *project.nodes.keys().next().unwrap();
    let sink = *project.nodes.keys().last().unwrap();
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = backend::<f32>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| owner.status() == OwnerStatus::Running);
    let snapshot = backend.observations().snapshot().unwrap();
    assert_eq!(snapshot.owner_generation, 7);
    assert!(snapshot.running);
    let by_node = |node: NodeId| {
        snapshot
            .streams
            .iter()
            .find(|stream| stream.node == node)
            .unwrap_or_else(|| panic!("missing observation for {node:?}"))
    };
    let playback = by_node(source);
    assert!(playback.input);
    assert!(!playback.master);
    assert_eq!(playback.actual.sample_rate, crate::NATIVE_SAMPLE_RATE);
    assert_eq!(playback.actual.channels, 2);
    assert_eq!(playback.actual.buffer_frames, 0);
    let microphone = by_node(sink);
    assert!(!microphone.input);
    assert!(!microphone.master);
    assert_eq!(microphone.actual.channels, 2);
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    assert!(backend.observations().snapshot().is_none());
}

#[test]
fn mono_virtual_boundaries_can_follow_a_stereo_physical_master() {
    let mut project = virtual_project(1, 88_200);
    add_physical_output(&mut project, 2);
    let probe = Arc::new(Probe::default());
    let native = native_devices(Default::default());
    let backend = backend::<f32>(&probe);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 512));
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 0);
    assert_sessions_joined(&probe, 2);
}

/// A stereo chain over the given endpoints: capture on `capture_endpoint`,
/// master render on `sink_endpoint`. Returns (source, sink).
fn absent_stream_project(
    capture_endpoint: &str,
    sink_endpoint: &str,
) -> (EngineProject, NodeId, NodeId) {
    let mut project = EngineProject::default();
    project.processing.engine.processing_sr = 96_000.0;
    let source = project.graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
    project
        .graph
        .connect(
            project.graph.get_node(source).unwrap().outputs()[0].id(),
            project.graph.get_node(sink).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::Physical(endpoint(capture_endpoint)),
            channels: 2,
            device: physical_device(ClockRole::Follower),
        }),
    );
    project.nodes.insert(
        sink,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Physical(endpoint(sink_endpoint)),
            channels: 2,
            device: physical_device(ClockRole::Master),
        }),
    );
    (project, source, sink)
}

/// The same duplex shape on WASAPI devices: two distinct endpoints resolve
/// to two independent groups (the ASIO variant is single-session by
/// validation), which is the layout the concurrent open wave exercises.
fn wasapi_stream_project(
    capture_endpoint: &str,
    sink_endpoint: &str,
) -> (EngineProject, NodeId, NodeId) {
    let mut project = EngineProject::default();
    project.processing.engine.processing_sr = 96_000.0;
    let source = project.graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = project.graph.create_node(NodeKind::Sink, 2).unwrap();
    project
        .graph
        .connect(
            project.graph.get_node(source).unwrap().outputs()[0].id(),
            project.graph.get_node(sink).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    let follower = DeviceOptions {
        backend: AudioBackend::Wasapi,
        hardware_sample_rate: Some(96_000),
        ..physical_device(ClockRole::Follower)
    };
    let master = DeviceOptions {
        backend: AudioBackend::Wasapi,
        hardware_sample_rate: Some(96_000),
        ..physical_device(ClockRole::Master)
    };
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::Physical(endpoint(capture_endpoint)),
            channels: 2,
            device: follower,
        }),
    );
    project.nodes.insert(
        sink,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Physical(endpoint(sink_endpoint)),
            channels: 2,
            device: master,
        }),
    );
    (project, source, sink)
}

#[test]
fn absent_capture_prepares_a_silent_null_stream() {
    // A missing device keeps its graph binding: the source binds constant
    // silence instead of failing the session, the observation carries the
    // absence, and only the present endpoint opens a native session
    // (audit 2026-11-11 AS-3: the graph never loses topology).
    let (project, source, sink) = absent_stream_project("{missing-input}", "native");
    let native = native_devices_absent(Default::default(), vec!["{missing-input}".into()]);
    let backend = ProfessionalBackend::<f64>::new().unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .expect("an absent endpoint prepares as a null stream");
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| native.probe.started.load(Ordering::Acquire) == 1);
    let streams = &backend.observations().snapshot().unwrap().streams;
    let absent: Vec<_> = streams.iter().filter(|stream| stream.absent).collect();
    assert_eq!(absent.len(), 1, "only the missing endpoint is absent");
    assert_eq!(absent[0].node, source);
    assert_eq!(
        absent[0].endpoint,
        StreamEndpoint::Physical("{missing-input}".into())
    );
    assert_eq!(absent[0].actual.buffer_frames, 0);
    assert!(
        streams
            .iter()
            .any(|stream| stream.node == sink && !stream.absent),
        "the present sink observes normally"
    );
    // Only the present sink opened a native session.
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 1);
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
}

#[test]
fn an_absent_master_output_degrades_to_the_virtual_clock() {
    // The only output's device is gone: the project still prepares and
    // runs, paced by the QPC fallback clock, with a discard sink bound to
    // the missing node and the present capture keeping its worker.
    let (project, _source, sink) = absent_stream_project("native", "{missing-output}");
    let probe = Arc::new(Probe::default());
    let native = native_devices_absent(Default::default(), vec!["{missing-output}".into()]);
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_virtual_clock_factory(Arc::new(FakeClockFactory(probe.clone())));
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .expect("an absent master degrades to the virtual clock");
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 1);
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 256));
    let streams = &backend.observations().snapshot().unwrap().streams;
    assert_eq!(
        streams.iter().filter(|stream| stream.absent).count(),
        1,
        "the absent output is the only absent stream"
    );
    assert!(
        streams
            .iter()
            .any(|stream| stream.node == sink && stream.absent)
    );
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 1);
}

#[test]
fn a_returning_absent_endpoint_publishes_a_resource_event() {
    // When the return probe finds the device again, poll_resource emits the
    // event that drives the supervisor's stop-and-reprepare upgrade from
    // the null stream to the real device.
    let (project, source, _sink) = absent_stream_project("{missing-input}", "native");
    let native = native_devices_absent(Default::default(), vec!["{missing-input}".into()]);
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_absence_probe(Arc::new(|_| Some("{returned}".into())));
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    // The prepared transport is dropped without attaching; the absence watch
    // is a backend-level concern.
    drop(prepared);
    let event = backend
        .poll_resource()
        .expect("the returning device emits an event");
    assert_eq!(event.node, source);
    assert_eq!(event.configuration, project.nodes[&source]);
    assert!(event.available);
    assert!(
        backend.poll_resource().is_none(),
        "the entry released with its event"
    );
}

// ---------------------------------------------------------------------------
// Bounded opens, abandonment, and the stuck-endpoint degradation (issue #105)
// ---------------------------------------------------------------------------

use std::time::{Duration, Instant};

fn wedged_output_backend() -> ProfessionalBackend<f64> {
    ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_open_timeout(Duration::from_millis(200))
        .with_virtual_clock_factory(Arc::new(FakeClockFactory(Arc::default())))
}

#[test]
fn a_timed_out_open_degrades_to_a_null_stream_and_runs_the_rest() {
    // D2: the wedged master output prepares as a discard-sink null stream,
    // the session starts on the QPC fallback clock with the healthy capture
    // worker live, and the transport stays fully usable.
    let (project, source, sink) = wasapi_stream_project("native", "{wedged-output}");
    let (native, gate) = native_devices_hanging(Faults::default(), vec!["{wedged-output}".into()]);
    let backend = wedged_output_backend();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let started = Instant::now();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .expect("a wedged open prepares as a null stream");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "prepare must respect the open deadline"
    );
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 256));
    let streams = &backend.observations().snapshot().unwrap().streams;
    let wedged = streams
        .iter()
        .find(|stream| stream.node == sink)
        .expect("wedged sink keeps its observation");
    assert!(wedged.absent);
    assert_eq!(
        wedged.endpoint,
        StreamEndpoint::Physical("{wedged-output}".into())
    );
    assert_eq!(wedged.actual.buffer_frames, 0);
    assert_eq!(native.probe.started.load(Ordering::Acquire), 1);
    assert!(
        streams
            .iter()
            .any(|stream| stream.node == source && !stream.absent),
        "the healthy capture prepares normally"
    );
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    gate.open();
}

#[test]
fn a_late_open_success_self_cleans_without_activation_or_failure() {
    // The abandoned worker's OS call finally returns: the fence keeps it out
    // of the run loop, its stream unwinds on its own thread, and the shared
    // run state never records a failure.
    let (project, _source, _sink) = wasapi_stream_project("native", "{wedged-output}");
    let (native, gate) = native_devices_hanging(Faults::default(), vec!["{wedged-output}".into()]);
    let backend = wedged_output_backend();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    // The settled capture stream drops with the unattached transport; the
    // abandoned wedged stream drops on its own thread once the gate opens.
    drop(prepared);
    wait_until(|| native.probe.dropped.load(Ordering::Acquire) == 1);
    gate.open();
    wait_until(|| native.probe.dropped.load(Ordering::Acquire) == 2);
    assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
}

#[test]
fn cancellation_during_a_hanging_open_returns_promptly() {
    // Stop/graph edits/app exit cancel the prepare while an open hangs: the
    // wait must return immediately instead of joining the blocked worker.
    let (project, _source, _sink) = wasapi_stream_project("native", "{wedged-output}");
    let (native, gate) = native_devices_hanging(Faults::default(), vec!["{wedged-output}".into()]);
    let backend = wedged_output_backend();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let cancel_timer = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        cancel_timer.cancel();
    });
    let started = Instant::now();
    let result = backend.prepare_with_native(request(&project, &host), &cancel, &native);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancel must not wait for the wedged open"
    );
    assert_eq!(result.err(), Some(BackendError::Cancelled));
    gate.open();
}

#[test]
fn stuck_endpoints_skip_reopens_until_the_device_is_seen_missing() {
    // D3: within one backend instance (one applied session) a timed-out
    // endpoint is not re-opened on re-prepare and the absence watch does not
    // report it back while it still enumerates; observing it missing clears
    // the verdict, and the next sighting emits the recovery event so the
    // following prepare opens the real stream again.
    let (project, _source, sink) = wasapi_stream_project("native", "{wedged-output}");
    let (native, gate) = native_devices_hanging(Faults::default(), vec!["{wedged-output}".into()]);
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_open_timeout(Duration::from_millis(600));
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    // First prepare: the healthy capture opens, the wedged master times out
    // (both factory invocations are counted; the wedged one was abandoned).
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .expect("the timed-out master prepares as a null stream");
    drop(prepared);
    gate.open();
    wait_until(|| native.probe.dropped.load(Ordering::Acquire) == 2);
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 2);

    // Re-prepare with the same backend: the stuck endpoint is skipped — no
    // second open attempt, so no second bounded hang.
    let started = Instant::now();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .expect("the stuck endpoint prepares as a null stream again");
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "the stuck endpoint must not re-open (a re-open would hang the full deadline)"
    );
    drop(prepared);
    assert_eq!(
        native.probe.opened.load(Ordering::Acquire),
        3,
        "only the healthy capture opened again"
    );

    // The device still enumerates: the watch must not report it back.
    let present = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let probe_present = present.clone();
    let backend = backend.with_absence_probe(Arc::new(move |_| {
        probe_present
            .load(Ordering::Acquire)
            .then(|| "{wedged-output}".to_owned())
    }));
    assert!(
        backend.poll_resource().is_none(),
        "a stuck endpoint that still enumerates has not returned"
    );

    // Re-plug cycle: missing once clears the verdict, present again emits
    // the recovery event for the wedged node. The watch probes at most once
    // per second, so each transition waits out the throttle interval.
    present.store(false, Ordering::Release);
    assert!(backend.poll_resource().is_none());
    std::thread::sleep(Duration::from_millis(1100));
    assert!(
        backend.poll_resource().is_none(),
        "the missing observation clears the stuck verdict without an event"
    );
    present.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(1100));
    let event = backend
        .poll_resource()
        .expect("the re-plugged endpoint reports its return");
    assert_eq!(event.node, sink);
    assert_eq!(event.configuration, project.nodes[&sink]);
    assert!(event.available);

    // The recovery re-prepare opens the real stream: the gate is open, so
    // the open settles and both groups run natively.
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .expect("the recovered endpoint opens for real");
    assert_eq!(
        native.probe.opened.load(Ordering::Acquire),
        5,
        "the cleared endpoint is opened again"
    );
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| native.probe.started.load(Ordering::Acquire) == 2);
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
}

#[test]
fn group_opens_run_concurrently() {
    // Two groups whose opens rendezvous on a barrier: a serial wave could
    // never satisfy both inside one deadline, so success proves bounded
    // parallelism (total latency is the max, not the sum).
    let (project, _source, _sink) = wasapi_stream_project("{parallel-capture}", "{parallel-sink}");
    let native = FakeNativeDevices {
        probe: Arc::default(),
        faults: Faults::default(),
        absent: Vec::new(),
        unplug: Vec::new(),
        hang: Vec::new(),
        hang_gate: Gate::default(),
        open_barrier: Some(Arc::new(std::sync::Barrier::new(2))),
        teardown_gate: Gate::default(),
    };
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_open_timeout(Duration::from_secs(5));
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let started = Instant::now();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .expect("both rendezvous opens settle concurrently");
    // A serial wave would park the first open on the barrier until the 5 s
    // deadline degraded both groups; the concurrent wave settles in
    // milliseconds, and both streams must open natively (not degraded).
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a serial wave would blow the open deadline on the barrier"
    );
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 2);
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| native.probe.started.load(Ordering::Acquire) == 2);
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
}

#[test]
fn prepare_progress_counts_settled_groups_during_the_open() {
    // While one group hangs, the observations channel carries live progress:
    // total groups, how many settled, and the endpoint still opening.
    let (project, _source, _sink) = wasapi_stream_project("native", "{wedged-output}");
    let (native, gate) = native_devices_hanging(Faults::default(), vec!["{wedged-output}".into()]);
    let backend = wedged_output_backend();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let observations = backend.observations();
    let mut progress_seen = None;
    std::thread::scope(|scope| {
        let backend = &backend;
        let native = &native;
        let host = &host;
        let project = &project;
        let cancel = &cancel;
        scope.spawn(move || {
            let prepared = backend
                .prepare_with_native(request(project, host), cancel, native)
                .unwrap();
            drop(prepared);
        });
        // The healthy capture settles quickly; the wedged output holds the
        // wave open until its deadline. Sample the live progress in between.
        let deadline = Instant::now() + Duration::from_millis(500);
        while progress_seen.is_none() && Instant::now() < deadline {
            if let Some(progress) = observations.snapshot_progress()
                && progress.groups_opened == 1
            {
                progress_seen = Some(progress);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    gate.open();
    let progress = progress_seen.expect("the open wave publishes progress");
    assert_eq!(progress.owner_generation, 7);
    assert_eq!(progress.groups_total, 2);
    assert_eq!(progress.groups_opened, 1);
    assert_eq!(progress.endpoint.as_deref(), Some("{wedged-output}"));
    assert!(observations.snapshot_progress().is_none());
}

/// A fake whose stream `stop()` (or run-loop `wait()`, per the fault flag)
/// blocks on the returned gate — the wedged driver teardown of issue #105's
/// residual.
fn native_devices_wedged_teardown(faults: Faults) -> (FakeNativeDevices, Gate) {
    let gate = Gate::default();
    (
        FakeNativeDevices {
            probe: Arc::default(),
            faults,
            absent: Vec::new(),
            unplug: Vec::new(),
            hang: Vec::new(),
            hang_gate: Gate::default(),
            open_barrier: None,
            teardown_gate: gate.clone(),
        },
        gate,
    )
}

#[test]
fn a_wedged_stream_stop_is_abandoned_and_the_engine_still_returns() {
    // The retirement path's driver release is bounded: a stream wedged in
    // stop() past the teardown deadline is fenced and abandoned — Stop
    // completes with a fault instead of parking forever — and because the
    // runtime handoff precedes stop(), the Engine still comes back.
    let (project, _source, _sink) = wasapi_stream_project("native", "{wedged-output}");
    let (native, gate) = native_devices_wedged_teardown(Faults {
        hang_stop: true,
        ..Default::default()
    });
    let backend = ProfessionalBackend::<f64>::new().unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| native.probe.started.load(Ordering::Acquire) == 2);
    wait_until(|| owner.timeline().is_some_and(|t| t.frame >= 128));
    owner.cancel();
    let started = Instant::now();
    let returned = owner.stop_and_join();
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "a wedged stop must not park the cleanup lane"
    );
    let engine = returned
        .engine
        .expect("the runtime handoff precedes stop, so the Engine returns");
    assert!(engine.timeline() >= 128);
    assert!(
        matches!(&returned.error, Some(BackendError::ResourceUnavailable(message))
            if message.contains("teardown")),
        "the abandonment is reported as a teardown fault: {:?}",
        returned.error
    );
    // The abandoned workers unwind on their own threads once released.
    gate.open();
    wait_until(|| native.probe.dropped.load(Ordering::Acquire) == 2);
}

#[test]
fn a_wedged_run_loop_owner_is_abandoned_without_the_engine() {
    // The narrow unrecoverable window: a master wedged mid-run still holds
    // the runtime when its teardown deadline expires. Stop still completes;
    // the missing engine is reported, and the abandoned worker drops the
    // runtime on its own thread when the driver call finally returns.
    let (project, _source, _sink) = wasapi_stream_project("native", "{wedged-output}");
    let (native, gate) = native_devices_wedged_teardown(Faults {
        hang_wait: true,
        ..Default::default()
    });
    let backend = ProfessionalBackend::<f64>::new().unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| native.probe.started.load(Ordering::Acquire) == 2);
    // Both workers must be inside the wedged wait before the stop: one that
    // observes the stop flag first would unwind cleanly and hand the
    // runtime back, which is the healthy path, not the one under test.
    wait_until(|| gate.entered() == 2);
    owner.cancel();
    let started = Instant::now();
    let returned = owner.stop_and_join();
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "a wedged run loop must not park the cleanup lane"
    );
    assert!(
        returned.engine.is_none(),
        "a master wedged mid-run cannot hand the runtime back"
    );
    assert!(returned.error.is_some());
    // Releasing the wedge lets the abandoned worker unwind — the runtime is
    // dropped there, nothing revives into shared state.
    gate.open();
    wait_until(|| native.probe.dropped.load(Ordering::Acquire) == 2);
    assert!(
        backend.poll_resource().is_none(),
        "an abandoned worker never spends a recovery ticket"
    );
}

// ---------------------------------------------------------------------------
// Runtime unplug: a running stream that dies of device removal degrades its
// endpoint to a null stream and upgrades it on the re-plug (issue #105)
// ---------------------------------------------------------------------------

/// A presence probe the test flips like a physical re-plug: `true` resolves
/// the endpoint to its native id, `false` reports it missing.
fn presence_probe(endpoint: &str, present: &Arc<std::sync::atomic::AtomicBool>) -> AbsenceProbe {
    let endpoint = endpoint.to_owned();
    let present = present.clone();
    Arc::new(move |_| present.load(Ordering::Acquire).then(|| endpoint.clone()))
}

#[test]
fn a_runtime_unplug_degrades_to_a_null_stream_and_upgrades_on_return() {
    // The capture device is yanked mid-run while it still enumerates: the
    // owner fails absence-class, the immediate re-prepare binds the endpoint
    // as a null stream WITHOUT reopening it (the suppress verdict), the
    // healthy master keeps its native stream, and the watch upgrades the
    // null stream only after the device is seen missing once and returns —
    // the same re-plug contract as a stuck endpoint.
    let (project, source, sink) = wasapi_stream_project("{yanked-input}", "{sink}");
    let native = native_devices_unplugging(vec!["{yanked-input}".into()]);
    let present = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_absence_probe(presence_probe("{yanked-input}", &present));
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel.clone());
    owner.activate().unwrap();
    wait_until(|| {
        matches!(
            owner.status(),
            OwnerStatus::Failed(error) if matches!(error, BackendError::ResourceAbsent(_))
        )
    });
    let returned = owner.stop_and_join();
    assert_eq!(returned.error, Some(absence_error()));
    assert_eq!(native.probe.opened.load(Ordering::Acquire), 2);
    assert_eq!(native.probe.dropped.load(Ordering::Acquire), 2);

    // Degrade re-prepare while the invalidated endpoint still enumerates: it
    // must not be reopened — it prepares as a null stream — while the healthy
    // master reopens natively.
    let healthy = native_devices(Default::default());
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &healthy)
        .expect("the unplugged endpoint degrades instead of failing the prepare");
    assert_eq!(
        healthy.probe.opened.load(Ordering::Acquire),
        1,
        "only the healthy master is reopened"
    );
    let owner = attach(&project, prepared, cancel.clone());
    owner.activate().unwrap();
    wait_until(|| healthy.probe.started.load(Ordering::Acquire) == 1);
    let streams = &backend.observations().snapshot().unwrap().streams;
    let degraded = streams
        .iter()
        .find(|stream| stream.node == source)
        .expect("the degraded endpoint keeps its observation");
    assert!(degraded.absent);
    assert_eq!(degraded.actual.buffer_frames, 0);
    assert!(
        streams
            .iter()
            .any(|stream| stream.node == sink && !stream.absent),
        "the healthy master keeps its native stream"
    );
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);

    // A still-enumerating endpoint has not "returned": the suppress verdict
    // holds, exactly like a stuck endpoint.
    assert!(
        backend.poll_resource().is_none(),
        "the still-enumerating endpoint is suppressed"
    );
    // Re-plug cycle: missing once clears the verdict, the return emits the
    // upgrade event. The watch probes at most once per second, so each
    // transition waits out the throttle interval.
    present.store(false, Ordering::Release);
    assert!(backend.poll_resource().is_none());
    std::thread::sleep(Duration::from_millis(1100));
    assert!(
        backend.poll_resource().is_none(),
        "the missing observation clears the verdict without an event"
    );
    present.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(1100));
    let event = backend
        .poll_resource()
        .expect("the re-plugged endpoint reports its return");
    assert_eq!(event.node, source);
    assert_eq!(event.configuration, project.nodes[&source]);
    assert!(event.available);

    // The upgrade re-prepare opens the real stream again: both endpoints are
    // native and nothing is absent.
    let restored = native_devices(Default::default());
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &restored)
        .expect("the recovered endpoint opens for real");
    assert_eq!(restored.probe.opened.load(Ordering::Acquire), 2);
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| restored.probe.started.load(Ordering::Acquire) == 2);
    let streams = &backend.observations().snapshot().unwrap().streams;
    assert!(
        streams.iter().all(|stream| !stream.absent),
        "the upgraded session runs fully native"
    );
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
}

#[test]
fn a_runtime_unplug_that_leaves_enumeration_upgrades_without_a_second_cycle() {
    // The device is gone from enumeration by the time the degrade re-prepare
    // resolves it: the resolve itself is the missing sighting that consumes
    // the suppress verdict, so the returning device upgrades immediately —
    // no extra unplug cycle may be required to lift the verdict.
    let (project, source, _sink) = wasapi_stream_project("{gone-input}", "{sink}");
    let native = native_devices_unplugging(vec!["{gone-input}".into()]);
    let present = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let backend = ProfessionalBackend::<f64>::new()
        .unwrap()
        .with_absence_probe(presence_probe("{gone-input}", &present));
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel.clone());
    owner.activate().unwrap();
    wait_until(|| matches!(owner.status(), OwnerStatus::Failed(_)));
    assert_eq!(owner.stop_and_join().error, Some(absence_error()));

    // Degrade re-prepare with the endpoint missing from enumeration: the
    // null stream registers the watch carrying the last native id.
    let missing = native_devices_absent(Default::default(), vec!["{gone-input}".into()]);
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &missing)
        .expect("the missing endpoint prepares as a null stream");
    assert_eq!(missing.probe.opened.load(Ordering::Acquire), 1);
    let owner = attach(&project, prepared, cancel.clone());
    owner.activate().unwrap();
    wait_until(|| missing.probe.started.load(Ordering::Acquire) == 1);
    assert!(
        backend
            .observations()
            .snapshot()
            .expect("the degraded owner still observes its streams")
            .streams
            .iter()
            .any(|stream| stream.node == source && stream.absent)
    );
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);

    // The device reappears: the first sighting after the throttle interval
    // emits the upgrade event — the verdict was consumed at resolve time.
    present.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(1100));
    let event = backend
        .poll_resource()
        .expect("the returning device upgrades without a second unplug cycle");
    assert_eq!(event.node, source);
    assert_eq!(event.configuration, project.nodes[&source]);
    assert!(event.available);

    let restored = native_devices(Default::default());
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &restored)
        .expect("the recovered endpoint opens for real");
    assert_eq!(restored.probe.opened.load(Ordering::Acquire), 2);
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    wait_until(|| restored.probe.started.load(Ordering::Acquire) == 2);
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
}

#[test]
fn a_generic_runtime_failure_keeps_the_reopen_contract() {
    // A non-absence runtime error never degrades: the endpoint is not
    // suppressed and the next prepare reopens the device for real. Only the
    // absence class enters the null-stream degrade.
    let (project, _source, _sink) = wasapi_stream_project("{flaky-input}", "{sink}");
    let native = native_devices(Faults {
        wait: true,
        ..Default::default()
    });
    let backend = ProfessionalBackend::<f64>::new().unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &native)
        .unwrap();
    let owner = attach(&project, prepared, cancel.clone());
    owner.activate().unwrap();
    wait_until(|| owner.status() == OwnerStatus::Failed(native_error("wait")));
    assert_eq!(owner.stop_and_join().error, Some(native_error("wait")));

    // No suppress verdict was recorded: both endpoints reopen natively.
    let healthy = native_devices(Default::default());
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &healthy)
        .expect("a generic failure keeps the ordinary reopen contract");
    assert_eq!(healthy.probe.opened.load(Ordering::Acquire), 2);
    let owner = attach(&project, prepared, cancel.clone());
    owner.activate().unwrap();
    wait_until(|| healthy.probe.started.load(Ordering::Acquire) == 2);
    let streams = &backend.observations().snapshot().unwrap().streams;
    assert!(streams.iter().all(|stream| !stream.absent));
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    assert!(
        backend.poll_resource().is_none(),
        "no absence watch was registered for a generic failure"
    );
}
