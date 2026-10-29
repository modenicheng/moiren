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

#[path = "measured_preparation_tests.rs"]
mod measured_preparation;
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
    assert_eq!(
        validate(&request(&project, &host), true),
        Ok((96_000, ProjectClock::VirtualRealtime))
    );
}

#[test]
fn small_virtual_target_prepares_and_repays_only_the_configured_recovery_burst() {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    use crate::virtual_owner::{ClockWake, VirtualBufferOptions, VirtualRealtimeClock};

    struct LateClock {
        now: u64,
    }
    impl VirtualRealtimeClock for LateClock {
        fn frequency(&self) -> u64 {
            48_000
        }
        fn now_ticks(&mut self) -> Result<u64, BackendError> {
            Ok(self.now)
        }
        fn wait_ticks(&mut self, _: u64) -> Result<ClockWake, BackendError> {
            if self.now != 0 {
                return Ok(ClockWake::Stopped);
            }
            self.now = 10 * 256;
            Ok(ClockWake::Timer)
        }
    }
    struct LateFactory;
    impl VirtualClockFactory for LateFactory {
        fn prepare(
            &self,
            _: crate::StopSignal,
        ) -> Result<Box<dyn VirtualRealtimeClock>, BackendError> {
            Ok(Box::new(LateClock { now: 0 }))
        }
    }

    let project = virtual_project(2, 48_000);
    assert_eq!(project.processing.engine.max_block_frames, 256);
    let probe = Arc::new(Probe::default());
    let buffer = VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 16_384,
            target_frames: 480,
            jitter_frames: 32,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 128,
            max_gap_frames: 128,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(1),
    };
    let backend = backend::<f32>(&probe)
        .with_virtual_clock_factory(Arc::new(LateFactory))
        .with_virtual_buffer(buffer)
        .unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(
            request(&project, &host),
            &cancel,
            &native_devices(Default::default()),
        )
        .expect("the one-period burst fits without silently reserving eight periods");
    for boundary in probe.requests.lock().unwrap().iter() {
        assert_eq!(boundary.max_demand_frames, 256);
        assert_eq!(boundary.buffer.recovery_cap_periods, Some(1));
        let admitted = boundary.prepared_budget().unwrap();
        assert_eq!(admitted.engine_burst_frames, 256);
        assert_eq!(admitted.queue.unwrap().prime_frames, 480);
        assert_eq!(admitted.queue.unwrap().modeled_minimum_frames, Some(449));
        assert_eq!(admitted.native_packet_frames, None);
    }
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    let snapshot = backend.native_observations().snapshot().unwrap();
    assert_eq!(snapshot.streams.len(), 2);
    for kind in [
        NativeStreamKind::VirtualPlayback,
        NativeStreamKind::VirtualMicrophone,
    ] {
        let stream = snapshot
            .streams
            .iter()
            .find(|stream| stream.kind == kind)
            .unwrap();
        let budget = stream.virtual_budget.expect("published VAD admission");
        assert_eq!(budget, buffer.prepare_recovery(48_000, 256).unwrap());
        assert_eq!(stream.input_queue, budget.queue);
        assert_eq!(budget.native_packet_frames, None);
    }
    wait_until(|| {
        owner
            .timeline()
            .is_some_and(|timeline| timeline.frame >= 256)
    });
    let returned = owner.stop_and_join();
    assert_eq!(returned.error, None);
    assert_eq!(
        returned.engine.timeline(),
        256,
        "nine clamped periods are not repaid"
    );
    assert_sessions_joined(&probe, 2);
}

fn backend<S: ProcessingSample>(probe: &Arc<Probe>) -> ProfessionalBackend<S> {
    ProfessionalBackend::new()
        .unwrap()
        .with_virtual_provider(Arc::new(FakeProvider::new(probe.clone())))
        .with_virtual_clock_factory(Arc::new(FakeClockFactory(probe.clone())))
}

fn native_devices(faults: Faults) -> FakeNativeDevices {
    FakeNativeDevices {
        probe: Arc::default(),
        faults,
    }
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

#[test]
fn virtual_budget_observations_preserve_whole_physical_master_demand() {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    use crate::virtual_owner::VirtualBufferOptions;

    let mut project = virtual_project(2, 48_000);
    let physical = add_physical_output(&mut project, 2);
    let buffer = VirtualBufferOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 16_384,
            target_frames: 1024,
            jitter_frames: 32,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 128,
            max_gap_frames: 128,
            source: DeliveryTimingSource::Declared,
        }),
        recovery_cap_periods: Some(1),
    };
    let probe = Arc::new(Probe::default());
    let backend = backend::<f32>(&probe).with_virtual_buffer(buffer).unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(
            request(&project, &host),
            &cancel,
            &native_devices(Default::default()),
        )
        .unwrap();
    // The 96 kHz master's 128-frame period needs 64 Engine frames, plus
    // the existing 64-frame FIR allowance and two Graph256 rounding blocks.
    let budget = buffer.prepare_budget(48_000, 256, 640).unwrap();
    let requests = probe.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert_eq!(request.prepared_budget().unwrap(), budget);
    }
    drop(requests);
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    let snapshot = backend.native_observations().snapshot().unwrap();
    assert_eq!(snapshot.streams.len(), 3);
    for kind in [
        NativeStreamKind::VirtualPlayback,
        NativeStreamKind::VirtualMicrophone,
    ] {
        let stream = snapshot
            .streams
            .iter()
            .find(|stream| stream.kind == kind)
            .unwrap();
        assert_eq!(stream.virtual_budget, Some(budget));
        assert_eq!(stream.input_queue, budget.queue);
        assert_eq!(stream.virtual_budget.unwrap().native_packet_frames, None);
    }
    assert_eq!(
        snapshot
            .streams
            .iter()
            .find(|stream| stream.node == physical)
            .unwrap()
            .virtual_budget,
        None
    );
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    assert!(backend.native_observations().snapshot().is_none());
    assert_sessions_joined(&probe, 2);
}

#[test]
fn process_bridge_storage_is_admitted_within_the_project_audio_budget() {
    let mut project = EngineProject::default();
    project.processing.engine.processing_sr = 48_000.0;
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
    project.processing.audio_byte_budget = 3 * 1024 * 1024;
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::Application {
                executable: "moiren:process:1:1".into(),
                intent: CaptureIntent::PassiveTap,
            },
            channels: 2,
            device: Default::default(),
        }),
    );
    project.nodes.insert(
        sink,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Physical(endpoint("native")),
            channels: 2,
            device: physical_device(ClockRole::Master),
        }),
    );
    let bridge = crate::process_loopback::ProcessBridgeOptions {
        input: crate::latency::transport::TransportInputPolicy::Target {
            capacity_frames: 65_536,
            target_frames: 8_192,
            jitter_frames: 0,
        },
        delivery: Some(crate::latency::transport::InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 480,
            max_gap_frames: 480,
            source: crate::latency::transport::DeliveryTimingSource::Declared,
        }),
        engine_sample_rate: 48_000,
        max_demand_frames: 256,
        age_limit_100ns: None,
        max_ingress_elapsed_100ns: None,
    };
    let backend = ProfessionalBackend::<f32>::new()
        .unwrap()
        .with_process_bridge(bridge)
        .unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    assert!(
        backend
            .prepare(request(&project, &host), &CancellationToken::default())
            .is_err()
    );
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
            assert!(returned.engine.timeline() >= 512);
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
        physical_device(ClockRole::Follower),
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
    returned.engine.render(64).unwrap();
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
    let native = backend.native_observations().snapshot().unwrap();
    assert_eq!(native.owner_generation, snapshot.owner_generation);
    assert!(native.running);
    assert_eq!(native.streams.len(), 2);
    assert_eq!(native.boundary_probes.len(), 2);
    assert!(native.streams.iter().any(|stream| stream.node == source && stream.kind == NativeStreamKind::VirtualPlayback));
    assert!(native.streams.iter().any(|stream| stream.node == sink && stream.kind == NativeStreamKind::VirtualMicrophone));
    let budget = crate::virtual_owner::VirtualBufferOptions::default()
        .prepare_recovery(48_000, 256)
        .unwrap();
    for stream in &native.streams {
        assert_eq!(stream.virtual_budget, Some(budget));
        assert_eq!(stream.input_queue, None);
        assert_eq!(stream.virtual_budget.unwrap().native_packet_frames, None);
    }
    owner.cancel();
    assert_eq!(owner.stop_and_join().error, None);
    assert!(backend.observations().snapshot().is_none());
    assert!(backend.native_observations().snapshot().is_none());
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
