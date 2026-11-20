//! Hardware-free fixture using the real backend, owner and Engine bindings.
use moiren_core::graph::NodeKind;
use moiren_engine::{
    boundary::{AudioReader, AudioWriter, audio_bridge},
    host::{BackendError, CancellationToken, EngineHost, EngineProject, NodeConfig, OwnerStatus},
    node::{InputConfig, InputSource, OutputConfig, OutputTarget, VirtualEndpointId},
};
use moiren_windows_audio::{
    engine_backend::{NativeObservations, ProfessionalBackend},
    virtual_owner::{
        BoundaryProbe, ClockWake, PreparedVirtualMicrophone, PreparedVirtualPlayback,
        VirtualBoundaryProvider, VirtualBoundaryRequest, VirtualClockFactory, VirtualRealtimeClock,
        VirtualResourceSession,
    },
};
use std::sync::{Arc, Mutex, atomic::AtomicU64, mpsc};

#[derive(Default)]
struct Provider {
    playback: Mutex<Vec<AudioWriter<f32>>>,
    microphone: Mutex<Vec<AudioReader<f32>>>,
}
struct Session;
impl VirtualResourceSession for Session {
    fn activate(&self) -> Result<(), BackendError> {
        Ok(())
    }
    fn cancel(&self) {}
    fn status(&self) -> OwnerStatus {
        OwnerStatus::Running
    }
    fn stop_and_join(self: Box<Self>) -> Result<(), BackendError> {
        Ok(())
    }
}
impl VirtualBoundaryProvider for Provider {
    fn prepare_playback(
        &self,
        _: VirtualBoundaryRequest,
        _: &CancellationToken,
    ) -> Result<PreparedVirtualPlayback, BackendError> {
        let (writer, reader) = audio_bridge(2, 16_384, 1 << 20).unwrap();
        self.playback.lock().unwrap().push(writer);
        Ok(PreparedVirtualPlayback {
            carrier: reader,
            reset_epoch: Arc::new(AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            probe: Arc::new(BoundaryProbe::default()),
            session: Box::new(Session),
        })
    }
    fn prepare_microphone(
        &self,
        _: VirtualBoundaryRequest,
        _: &CancellationToken,
    ) -> Result<PreparedVirtualMicrophone, BackendError> {
        let (writer, reader) = audio_bridge(2, 16_384, 1 << 20).unwrap();
        self.microphone.lock().unwrap().push(reader);
        Ok(PreparedVirtualMicrophone {
            carrier: writer,
            reset_epoch: Arc::new(AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            probe: Arc::new(BoundaryProbe::default()),
            session: Box::new(Session),
        })
    }
}
struct ClockFactory(Mutex<Option<mpsc::Receiver<usize>>>);
impl VirtualClockFactory for ClockFactory {
    fn prepare(
        &self,
        _: moiren_windows_audio::StopSignal,
    ) -> Result<Box<dyn VirtualRealtimeClock>, BackendError> {
        Ok(Box::new(Clock {
            commands: self.0.lock().unwrap().take().unwrap(),
            remaining: 0,
            now: 0,
        }))
    }
}
struct Clock {
    commands: mpsc::Receiver<usize>,
    remaining: usize,
    now: u64,
}
impl VirtualRealtimeClock for Clock {
    fn frequency(&self) -> u64 {
        48_000
    }
    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        Ok(self.now)
    }
    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
        if self.remaining == 0 {
            self.remaining = self
                .commands
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap_or(0);
        }
        if self.remaining == 0 {
            return Ok(ClockWake::Stopped);
        }
        self.remaining -= 1;
        self.now += ticks;
        Ok(ClockWake::Timer)
    }
}

pub(super) struct Fixture {
    pub host: Option<EngineHost<f32>>,
    pub observations: NativeObservations,
    commands: Option<mpsc::Sender<usize>>,
    target: u64,
}
impl Fixture {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let backend = ProfessionalBackend::<f32>::new()
            .unwrap()
            .with_virtual_provider(Arc::new(Provider::default()))
            .with_virtual_clock_factory(Arc::new(ClockFactory(Mutex::new(Some(receiver)))));
        let observations = backend.native_observations();
        let mut project = EngineProject::default();
        project.processing.engine.max_block_frames = 32;
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
        let direct = project.graph.edges()[0].id();
        project.graph.disconnect(direct).unwrap();
        let bus = project.graph.create_node(NodeKind::Bus, 2).unwrap();
        let source_port = project.graph.get_node(source).unwrap().outputs()[0].id();
        moiren_core::graph::edit::connect_to_new_bus_input(
            &mut project.graph,
            source_port,
            bus,
            Default::default(),
        )
        .unwrap();
        project
            .graph
            .connect(
                project.graph.get_node(bus).unwrap().outputs()[0].id(),
                project.graph.get_node(sink).unwrap().inputs()[0].id(),
                Default::default(),
            )
            .unwrap();
        project.nodes.insert(bus, NodeConfig::Bus);
        project.nodes.insert(
            source,
            NodeConfig::Input(InputConfig {
                source: InputSource::VirtualPlayback {
                    endpoint_id: VirtualEndpointId::from_bytes([1; 16]),
                },
                channels: 2,
                device: Default::default(),
            }),
        );
        project.nodes.insert(
            sink,
            NodeConfig::Output(OutputConfig {
                target: OutputTarget::VirtualMicrophone {
                    endpoint_id: VirtualEndpointId::from_bytes([2; 16]),
                },
                channels: 2,
                device: Default::default(),
            }),
        );
        let host = EngineHost::open(Default::default(), project, backend).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while observations.retain_session().is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        Self {
            host: Some(host),
            observations,
            commands: Some(sender),
            target: 0,
        }
    }
    pub fn callbacks(&mut self, count: usize) {
        self.target += count as u64 * 32;
        self.commands.as_ref().unwrap().send(count).unwrap();
        let observer = self.host.as_ref().unwrap().observer();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while observer
            .snapshot()
            .timeline
            .is_none_or(|time| time.frame < self.target)
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    pub fn callback_count(&self) -> u64 {
        self.target / 32
    }
    pub fn close(&mut self) {
        drop(self.commands.take());
        drop(self.host.take());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.close();
    }
}

#[test]
fn src_route_engine_host_update_invalidates_same_owner_preparation_without_replacing_facts() {
    let mut fixture = Fixture::new();
    fixture.callbacks(4);
    let owner = fixture.observations.retain_session().unwrap();
    let original = owner.src_route();
    assert!(!original.invalidated);
    let route = original.prepared.clone().unwrap();
    let before = fixture.host.as_ref().unwrap().observer().snapshot();
    let initial_plan = route.prepared_plan_revision;
    for point in owner
        .snapshot()
        .boundary_probes
        .iter()
        .filter_map(|p| p.src_checkpoint)
    {
        assert_eq!(point.execution.unwrap().plan_revision, initial_plan);
        assert_eq!(point.execution.unwrap().control_generation, 0);
    }
    let receipt = fixture
        .host
        .as_ref()
        .unwrap()
        .control()
        .edit(move |project| {
            project.graph.remove_node(route.bus)?;
            let source = project.graph.get_node(route.source)?.outputs()[0].id();
            let sink = project.graph.get_node(route.sink)?.inputs()[0].id();
            project.graph.connect(source, sink, Default::default())?;
            Ok(())
        })
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !owner.src_route().invalidated {
        assert!(
            std::time::Instant::now() < deadline,
            "compatible update did not invalidate preparation"
        );
        std::thread::yield_now();
    }
    for _ in 0..100 {
        fixture.callbacks(1);
        if receipt.outcome().is_terminal() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(matches!(
        receipt.outcome(),
        moiren_engine::host::EditOutcome::Activated { .. }
    ));
    let after = fixture.host.as_ref().unwrap().observer().snapshot();
    assert_eq!(before.owner_generation, after.owner_generation);
    assert_ne!(before.active_revision, after.active_revision);
    fixture.callbacks(1);
    for point in owner
        .snapshot()
        .boundary_probes
        .iter()
        .filter_map(|p| p.src_checkpoint)
    {
        assert_ne!(point.execution.unwrap().plan_revision, initial_plan);
        assert_eq!(point.execution.unwrap().control_generation, 0);
    }
    assert_eq!(owner.src_route().prepared, original.prepared);
    fixture.close();
    assert!(owner.src_route().invalidated);
    assert_eq!(owner.src_route().prepared, original.prepared);
}

#[test]
fn src_execution_host_direct_control_changes_stamp_without_preparation_invalidation() {
    use moiren_core::protocol::ParamValue;
    use moiren_engine::host::{ParameterOutcome, ParameterTarget, ParameterTiming, SendParameter};
    let mut fixture = Fixture::new();
    fixture.callbacks(4);
    let owner = fixture.observations.retain_session().unwrap();
    let route = owner.src_route().prepared.unwrap();
    let edge = fixture
        .host
        .as_ref()
        .unwrap()
        .observer()
        .read()
        .active
        .as_ref()
        .unwrap()
        .graph
        .edges()[0]
        .id();
    let receipt = fixture
        .host
        .as_ref()
        .unwrap()
        .control()
        .parameter(
            ParameterTarget::Edge {
                edge,
                parameter: SendParameter::Gain,
            },
            ParamValue::Float(0.5),
            ParameterTiming::NextBlock,
            8,
        )
        .unwrap();
    for _ in 0..100 {
        fixture.callbacks(1);
        if matches!(receipt.outcome(), ParameterOutcome::Applied(_)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        matches!(receipt.outcome(), ParameterOutcome::Applied(_)),
        "{:?}",
        receipt.outcome()
    );
    fixture.callbacks(1);
    assert!(
        !owner.src_route().invalidated,
        "direct controls bypass preparation"
    );
    let mut checked = 0;
    for point in owner
        .snapshot()
        .boundary_probes
        .iter()
        .filter_map(|p| p.src_checkpoint)
    {
        assert_eq!(
            point.execution.unwrap().plan_revision,
            route.prepared_plan_revision
        );
        assert!(point.execution.unwrap().control_generation > 0);
        checked += 1;
    }
    assert_eq!(checked, 2);
    fixture.close();
}
