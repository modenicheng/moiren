use super::*;
use crate::virtual_owner::{
    ClockWake, PreparedVirtualMicrophone, PreparedVirtualPlayback, VirtualClockFactory,
    VirtualRealtimeClock, VirtualResourceSession,
};
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::audio_bridge,
    compiler::compile,
    host::{
        BackendError, CancellationToken, EngineProject, OwnerStatus, PrepareMode, PrepareRequest,
    },
    node::{
        DeviceOptions, InputConfig, InputSource, OutputConfig, OutputTarget, VirtualEndpointId,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

#[derive(Default)]
struct SessionState {
    activated: AtomicUsize,
    joined: AtomicUsize,
    prepared_playback: AtomicUsize,
    prepared_microphone: AtomicUsize,
}

struct FakeSession {
    state: Arc<SessionState>,
    active: AtomicUsize,
}

impl FakeSession {
    fn new(state: Arc<SessionState>) -> Self {
        Self {
            state,
            active: AtomicUsize::new(0),
        }
    }
}

impl VirtualResourceSession for FakeSession {
    fn activate(&self) -> Result<(), BackendError> {
        self.active.store(1, Ordering::Release);
        self.state.activated.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn cancel(&self) {
        self.active.store(0, Ordering::Release);
    }

    fn status(&self) -> OwnerStatus {
        if self.active.load(Ordering::Acquire) == 1 {
            OwnerStatus::Running
        } else {
            OwnerStatus::Starting
        }
    }

    fn stop_and_join(self: Box<Self>) -> Result<(), BackendError> {
        self.state.joined.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

struct FakeProvider {
    state: Arc<SessionState>,
}

impl VirtualBoundaryProvider for FakeProvider {
    fn prepare_playback(
        &self,
        request: VirtualBoundaryRequest,
        _: &CancellationToken,
    ) -> Result<PreparedVirtualPlayback, BackendError> {
        self.state.prepared_playback.fetch_add(1, Ordering::AcqRel);
        let (_, reader) = audio_bridge(
            2,
            request.max_block_frames * 2 + 64,
            request.audio_byte_budget,
        )
        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        Ok(PreparedVirtualPlayback {
            carrier: reader,
            reset_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session: Box::new(FakeSession::new(self.state.clone())),
        })
    }

    fn prepare_microphone(
        &self,
        request: VirtualBoundaryRequest,
        _: &CancellationToken,
    ) -> Result<PreparedVirtualMicrophone, BackendError> {
        self.state
            .prepared_microphone
            .fetch_add(1, Ordering::AcqRel);
        let (writer, _) = audio_bridge(
            2,
            request.max_block_frames * 2 + 64,
            request.audio_byte_budget,
        )
        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        Ok(PreparedVirtualMicrophone {
            carrier: writer,
            reset_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session: Box::new(FakeSession::new(self.state.clone())),
        })
    }
}

struct FakeClock {
    now: u64,
    waits: usize,
    render_wakes: usize,
}

impl VirtualRealtimeClock for FakeClock {
    fn frequency(&self) -> u64 {
        48_000
    }

    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        Ok(self.now)
    }

    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
        self.waits += 1;
        if self.render_wakes == 4 {
            return Ok(ClockWake::Stopped);
        }
        self.now = self
            .now
            .checked_add(ticks)
            .ok_or_else(|| BackendError::Failed("fake clock overflow".into()))?;
        self.render_wakes += 1;
        Ok(ClockWake::Timer)
    }
}

struct FakeClockFactory;

impl VirtualClockFactory for FakeClockFactory {
    fn prepare(&self, _: StopSignal) -> Result<Box<dyn VirtualRealtimeClock>, BackendError> {
        Ok(Box::new(FakeClock {
            now: 0,
            waits: 0,
            render_wakes: 0,
        }))
    }
}

fn virtual_project() -> EngineProject {
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
    let playback = VirtualEndpointId::from_bytes([1; 16]);
    let microphone = VirtualEndpointId::from_bytes([2; 16]);
    let mut project = EngineProject {
        graph,
        ..Default::default()
    };
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::VirtualPlayback {
                endpoint_id: playback,
            },
            channels: 1,
            device: DeviceOptions::default(),
        }),
    );
    project.nodes.insert(
        sink,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::VirtualMicrophone {
                endpoint_id: microphone,
            },
            channels: 1,
            device: DeviceOptions::default(),
        }),
    );
    project
}

fn prepare_request<'a>(
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
fn virtual_only_backend_prepares_attaches_and_runs_one_owner_without_clients() {
    let project = virtual_project();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (signal_host, _rt_host) = bus.create_host(8).unwrap();
    let state = Arc::new(SessionState::default());
    let backend = WindowsBackend::new(Default::default())
        .unwrap()
        .with_virtual_provider(Arc::new(FakeProvider {
            state: state.clone(),
        }))
        .with_virtual_clock_factory(Arc::new(FakeClockFactory));

    let prepared = backend
        .prepare(
            prepare_request(&project, &signal_host),
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(state.prepared_playback.load(Ordering::Acquire), 1);
    assert_eq!(state.prepared_microphone.load(Ordering::Acquire), 1);
    let transport = prepared.transport.expect("virtual owner transport");
    let compiled = compile(
        &project.graph,
        prepared.bindings,
        project.processing.compile_config(1, 1),
    )
    .unwrap();
    let mut engine = compiled.engine;
    engine.enable_plan_switching(1).unwrap();
    let owner = match transport.attach(engine, &compiled.bindings, CancellationToken::default()) {
        Ok(owner) => owner,
        Err(failure) => panic!("virtual owner attachment failed: {}", failure.error),
    };
    owner.activate().unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while owner.timeline().is_none_or(|timeline| timeline.frame == 0) {
        assert!(
            std::time::Instant::now() < deadline,
            "virtual backend owner did not publish a rendered timeline"
        );
        std::thread::yield_now();
    }
    let returned = owner.stop_and_join();
    assert!(
        returned.error.is_none(),
        "owner failed: {:?}",
        returned.error
    );
    assert!(returned.engine.timeline() >= 8);
    assert_eq!(state.activated.load(Ordering::Acquire), 2);
    assert_eq!(state.joined.load(Ordering::Acquire), 2);
}

#[test]
fn virtual_intent_without_provider_stays_unavailable_before_preparation() {
    let project = virtual_project();
    let backend = WindowsBackend::new(Default::default()).unwrap();
    assert!(matches!(
        backend.validate_project(&project),
        Err(BackendError::ResourceUnavailable(message))
            if message.contains("virtual") && message.contains("not integrated")
    ));
}

#[test]
fn virtual_io_refuses_unadmitted_latency_profiles_before_opening_resources() {
    use crate::latency::{LatencyBudgetPolicy, RenderQueuePolicy};
    let project = virtual_project();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (signal_host, _rt_host) = bus.create_host(8).unwrap();
    let render = RenderQueuePolicy::Target {
        frames: 480,
        wake_margin_frames: 48,
        callback_budget_frames: 48,
    };
    for config in [
        WindowsBackendConfig {
            latency_profile: super::super::NativeLatencyProfile::Balanced,
            ..Default::default()
        },
        WindowsBackendConfig {
            latency_profile: super::super::NativeLatencyProfile::Low,
            ..Default::default()
        },
        WindowsBackendConfig {
            render_queue: render,
            ..Default::default()
        },
        WindowsBackendConfig {
            latency_budget: LatencyBudgetPolicy::Custom {
                input_capacity_frames: 2048,
                input_target_frames: 960,
                input_jitter_frames: 240,
                render: RenderQueuePolicy::FullCapacity,
            },
            ..Default::default()
        },
        WindowsBackendConfig {
            render_queue: render,
            latency_budget: LatencyBudgetPolicy::Custom {
                input_capacity_frames: 2048,
                input_target_frames: 960,
                input_jitter_frames: 240,
                render,
            },
            ..Default::default()
        },
    ] {
        let state = Arc::new(SessionState::default());
        let backend = WindowsBackend::new(config)
            .unwrap()
            .with_virtual_provider(Arc::new(FakeProvider {
                state: state.clone(),
            }));
        let result = backend.prepare(
            prepare_request(&project, &signal_host),
            &CancellationToken::default(),
        );
        assert!(
            matches!(result, Err(BackendError::Unsupported(message)) if message.contains("packet/jitter"))
        );
        assert_eq!(state.prepared_playback.load(Ordering::Acquire), 0);
        assert_eq!(state.prepared_microphone.load(Ordering::Acquire), 0);
    }
}

#[test]
fn virtual_qpc_owner_adapts_packet_phase_without_a_physical_master() {
    use moiren_engine::boundary::{AudioReader, AudioWriter};
    struct PacketProvider {
        producer: Mutex<Option<AudioWriter<f32>>>,
        consumer: Mutex<Option<AudioReader<f32>>>,
    }
    impl VirtualBoundaryProvider for PacketProvider {
        fn prepare_playback(
            &self,
            request: VirtualBoundaryRequest,
            _: &CancellationToken,
        ) -> Result<PreparedVirtualPlayback, BackendError> {
            let (writer, reader) = audio_bridge(2, 4800, request.audio_byte_budget).unwrap();
            *self.producer.lock().unwrap() = Some(writer);
            Ok(PreparedVirtualPlayback {
                carrier: reader,
                reset_epoch: Arc::new(AtomicU64::new(0)),
                producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
                consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
                session: Box::new(FakeSession::new(Arc::default())),
            })
        }
        fn prepare_microphone(
            &self,
            request: VirtualBoundaryRequest,
            _: &CancellationToken,
        ) -> Result<PreparedVirtualMicrophone, BackendError> {
            let (writer, reader) = audio_bridge(2, 4800, request.audio_byte_budget).unwrap();
            *self.consumer.lock().unwrap() = Some(reader);
            Ok(PreparedVirtualMicrophone {
                carrier: writer,
                reset_epoch: Arc::new(AtomicU64::new(0)),
                producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
                consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
                session: Box::new(FakeSession::new(Arc::default())),
            })
        }
    }
    let mut project = virtual_project();
    project.processing.engine.max_block_frames = 256;
    project.processing.engine.processing_sr = 48_000.0;
    project.processing.audio_byte_budget = 1 << 20;
    let provider = Arc::new(PacketProvider {
        producer: Mutex::new(None),
        consumer: Mutex::new(None),
    });
    let backend = WindowsBackend::new(Default::default())
        .unwrap()
        .with_virtual_provider(provider.clone());
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let prepared = backend
        .prepare(
            prepare_request(&project, &host),
            &CancellationToken::default(),
        )
        .unwrap();
    let mut engine = compile(
        &project.graph,
        prepared.bindings,
        project.processing.compile_config(1, 1),
    )
    .unwrap()
    .engine;
    let mut producer = provider.producer.lock().unwrap().take().unwrap();
    let mut consumer = provider.consumer.lock().unwrap().take().unwrap();
    let mut next_packet = 480_u64;
    let mut packets = 0;
    let mut samples = [f32::NAN; 1024];
    let mut silent = 0;
    let mut checked = 0;
    // Native and Engine use the same nominal QPC rate but independent phases.
    // A one-ms delivery shift after settling must not drain a zero-reserve
    // source. All scheduling is simulated; no audio device or sleeps occur.
    for now in (256_u64..=480_000).step_by(256) {
        let jitter = if packets >= 200 { 48 } else { 0 };
        if next_packet + jitter <= now {
            assert!(producer.write_interleaved_exact(&[0.25; 960]).unwrap());
            next_packet += 480;
            packets += 1;
        }
        engine.render(256).unwrap();
        let frames = consumer.available_frames();
        assert!(frames <= samples.len() / 2);
        consumer
            .read_interleaved(&mut samples[..frames * 2])
            .unwrap();
        if now > 48_000 {
            checked += frames;
            for frame in samples[..frames * 2].as_chunks::<2>().0 {
                silent += usize::from(frame[0].abs() <= 0.0001);
                assert_eq!(frame[1], 0.0, "mono never duplicates its channel");
            }
        }
    }
    assert!(checked > 400_000);
    assert_eq!(silent, 0, "packet-phase starvation after settling");
}
