//! End-to-end recovery with the real supervisor, native owner and rendering pump.
//! Only NativeStream is fake: this harness must never activate ASIO or WASAPI.
use super::{
    bridge::{RingSink, Tone},
    native::{
        NativeLatency, NativeRecoveryRequest, NativeStream, NativeStreamInfo, NativeStreamStats,
        StereoFrame,
    },
    observations::{
        Counters, ProfessionalObservationSnapshot, ProfessionalObservations,
        ProfessionalStreamObservation,
    },
    owner::{self, AuxiliaryOwner, Transport, WorkerRequest},
    recovery::{RecoveryMonitor, RecoveryTicket},
};
use moiren_core::graph::{NodeId, NodeKind, SendParams};
use moiren_engine::{
    audio_config::{AudioBackend, SampleFormat},
    boundary::ClockRole,
    compiler::NodeBindings,
    host::{
        BackendError, CancellationToken, Diagnostic, EngineBackend, EngineHost, EngineObserver,
        EngineProject, ExecutionState, HostConfig, HostSnapshot, HostState, NodeConfig,
        PrepareMode, PrepareRequest, PreparedBackend, ResourceEvent, UpdateState,
    },
    node::{
        DeviceMode, DeviceOptions, EndpointSelector, InputConfig, InputSource, OutputConfig,
        OutputTarget,
    },
};
use std::{
    marker::PhantomData,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(2);
const MIN_RENDERED: usize = 1024;
const STABLE_POLLS: usize = 32;

#[derive(Clone, Copy)]
enum Fault {
    Healthy,
    Recover(NativeRecoveryRequest),
    Unsupported,
    Start,
    Wait,
    Cancel,
    Stop,
    Release,
    Combined(super::Faults),
}

impl Fault {
    fn notification(self) -> Option<NativeRecoveryRequest> {
        match self {
            Self::Recover(request) => Some(request),
            Self::Cancel | Self::Stop | Self::Release => Some(NativeRecoveryRequest::Reset),
            Self::Combined(_) => Some(NativeRecoveryRequest::Reset),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Open(usize),
    Start(usize),
    Notification(usize, NativeRecoveryRequest),
    Stop(usize),
    Release(usize),
    Drop(usize),
    Published(u64),
    FollowerStop,
    FollowerRelease,
    FollowerDrop,
    AuxiliaryJoin,
    AuxiliaryDrop,
    EngineDrop(thread::ThreadId),
    RenderThread(thread::ThreadId),
}

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}

impl Gate {
    fn pause(&self) {
        let guard = self.released.lock().unwrap();
        self.entered.store(true, Ordering::Release);
        let (guard, timeout) = self
            .wake
            .wait_timeout_while(guard, TIMEOUT, |released| !*released)
            .unwrap();
        assert!(*guard && !timeout.timed_out(), "test gate was not released");
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }

    fn entered(&self) {
        wait_for(|| self.entered.load(Ordering::Acquire));
    }
}

#[derive(Clone, Copy, Debug)]
enum PollOrdering {
    NotificationBeforePoll,
    NotificationAfterPoll,
}

struct Mixed {
    ordering: PollOrdering,
    poll_armed: AtomicBool,
    poll: Gate,
    follower_stop: Gate,
    auxiliary_join: Gate,
    follower_fault: super::Effect,
    auxiliary_fault: super::Effect,
    auxiliary_drop_panic: AtomicBool,
}

impl Mixed {
    fn new(
        ordering: PollOrdering,
        follower_fault: super::Effect,
        auxiliary_fault: super::Effect,
    ) -> Self {
        Self {
            ordering,
            poll_armed: AtomicBool::new(false),
            poll: Gate::default(),
            follower_stop: Gate::default(),
            auxiliary_join: Gate::default(),
            follower_fault,
            auxiliary_fault,
            auxiliary_drop_panic: AtomicBool::new(false),
        }
    }
}

struct Session {
    generation: u64,
    node: NodeId,
    configuration: NodeConfig,
    processing_rate: u32,
    counters: Arc<Counters>,
    ticket: RecoveryTicket,
    cancel: CancellationToken,
    written: AtomicUsize,
    nonzero: AtomicUsize,
    notifications: AtomicUsize,
}

#[derive(Default)]
struct Probe {
    opens: AtomicUsize,
    alive: AtomicUsize,
    polls: AtomicUsize,
    inject: AtomicBool,
    sessions: Mutex<Vec<Arc<Session>>>,
    events: Mutex<Vec<Event>>,
    resources: Mutex<Vec<ResourceEvent>>,
    mixed: Option<Arc<Mixed>>,
}

impl Probe {
    fn record(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    fn session(&self, index: usize) -> Arc<Session> {
        self.sessions.lock().unwrap()[index].clone()
    }
}

struct FakeNative {
    info: NativeStreamInfo,
    fault: Fault,
    number: usize,
    callbacks: u64,
    notified: bool,
    cancel: CancellationToken,
    session: Arc<Session>,
    probe: Arc<Probe>,
}

impl NativeStream for FakeNative {
    fn info(&self) -> NativeStreamInfo {
        self.info
    }

    fn start(&mut self) -> Result<(), BackendError> {
        self.probe.record(Event::Start(self.number));
        if matches!(self.fault, Fault::Start) {
            return Err(start_error());
        }
        Ok(())
    }

    fn wait(&mut self, timeout: Duration) -> Result<bool, BackendError> {
        self.callbacks += 1;
        // Arm from the test only after Running is observed. Written non-silent
        // frames prove Tone -> Engine::render -> RingSink -> Pump actually ran.
        if !matches!(self.fault, Fault::Healthy)
            && self.probe.inject.load(Ordering::Acquire)
            && self.session.written.load(Ordering::Acquire) >= MIN_RENDERED
            && self.session.nonzero.load(Ordering::Acquire) > 0
        {
            if let Some(request) = self.fault.notification() {
                self.notified = true;
                self.session.notifications.fetch_add(1, Ordering::AcqRel);
                self.probe.record(Event::Notification(self.number, request));
            }
            if matches!(self.fault, Fault::Cancel) {
                // Cancel the real host's owner token before its wait error is
                // handled; even a pending native reset must not authorize retry.
                self.cancel.cancel();
            }
            if let Fault::Combined(faults) = self.fault {
                faults.run.apply("run")?;
            }
            return Err(wait_error());
        }
        thread::park_timeout(timeout.min(Duration::from_millis(1)));
        Ok(true)
    }

    fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
        Ok(0)
    }

    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        // Partial writes exercise the actual pump's pending-frame handling.
        let count = input.len().min(31);
        let frames = &input[..count];
        assert!(frames.iter().flatten().all(|sample| sample.is_finite()));
        let nonzero = frames.iter().filter(|frame| frame[0].abs() > 1e-6).count();
        self.session.nonzero.fetch_add(nonzero, Ordering::Release);
        self.session.written.fetch_add(count, Ordering::Release);
        Ok(count)
    }

    fn stats(&self) -> NativeStreamStats {
        NativeStreamStats {
            callbacks: self.callbacks,
            written_frames: self.session.written.load(Ordering::Acquire) as u64,
            capture_dropped_frames: 7 * self.number as u64,
            render_silent_frames: 11 * self.number as u64,
            ..Default::default()
        }
    }

    fn driver_latency(&self) -> Option<NativeLatency> {
        Some(latency(self.number))
    }

    fn recovery_request(&self) -> Option<NativeRecoveryRequest> {
        if let Fault::Combined(faults) = self.fault {
            assert_ne!(
                faults.observation,
                super::Effect::Panic,
                "observation panic"
            );
        }
        self.notified.then(|| self.fault.notification()).flatten()
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.probe.record(Event::Stop(self.number));
        if let Fault::Combined(faults) = self.fault {
            return faults.stop.apply("stop");
        }
        if matches!(self.fault, Fault::Stop) {
            return Err(stop_error());
        }
        Ok(())
    }

    fn release(&mut self) -> Result<(), BackendError> {
        self.probe.record(Event::Release(self.number));
        if let Fault::Combined(faults) = self.fault {
            return faults.release.apply("release");
        }
        if matches!(self.fault, Fault::Release) {
            return Err(release_error());
        }
        Ok(())
    }
}

impl Drop for FakeNative {
    fn drop(&mut self) {
        self.probe.record(Event::Drop(self.number));
        assert_eq!(self.probe.alive.fetch_sub(1, Ordering::AcqRel), 1);
        if let Fault::Combined(faults) = self.fault {
            assert_ne!(faults.drop, super::Effect::Panic, "native Drop panic");
        }
    }
}

struct Follower {
    probe: Arc<Probe>,
    mixed: Arc<Mixed>,
}

impl NativeStream for Follower {
    fn info(&self) -> NativeStreamInfo {
        super::info()
    }
    fn start(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        thread::yield_now();
        Ok(true)
    }
    fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
        Ok(0)
    }
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        Ok(input.len())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        self.probe.record(Event::FollowerStop);
        self.mixed.follower_stop.pause();
        self.mixed.follower_fault.apply("follower stop")
    }
    fn release(&mut self) -> Result<(), BackendError> {
        self.probe.record(Event::FollowerRelease);
        Ok(())
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        self.probe.record(Event::FollowerDrop);
    }
}

struct Auxiliary {
    probe: Arc<Probe>,
    mixed: Arc<Mixed>,
    joined: bool,
}

impl AuxiliaryOwner for Auxiliary {
    fn activate(&self) -> Result<(), BackendError> {
        Ok(())
    }
    fn cancel(&self) {}
    fn running(&self) -> bool {
        true
    }
    fn finished(&self) -> bool {
        false
    }
    fn join(&mut self) -> Result<(), BackendError> {
        if self.joined {
            return Ok(());
        }
        self.joined = true;
        self.probe.record(Event::AuxiliaryJoin);
        self.mixed.auxiliary_join.pause();
        self.mixed.auxiliary_fault.apply("auxiliary join")
    }
}

impl Drop for Auxiliary {
    fn drop(&mut self) {
        self.probe.record(Event::AuxiliaryDrop);
        assert!(
            !self.mixed.auxiliary_drop_panic.load(Ordering::Acquire),
            "auxiliary Drop panic"
        );
    }
}

struct GuardedTone {
    tone: Tone,
    probe: Arc<Probe>,
    rendered: bool,
}

impl moiren_engine::boundary::RtAudioSource<f64> for GuardedTone {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        ctx: &moiren_engine::processor::ProcessContext,
        output: moiren_engine::buffer::AudioBlockMut<'_, f64>,
    ) -> moiren_engine::boundary::BoundaryReport {
        if !self.rendered {
            self.rendered = true;
            self.probe
                .record(Event::RenderThread(thread::current().id()));
        }
        self.tone.read(ctx, output)
    }
}

impl Drop for GuardedTone {
    fn drop(&mut self) {
        self.probe.record(Event::EngineDrop(thread::current().id()));
    }
}

fn latency(number: usize) -> NativeLatency {
    NativeLatency {
        input_frames: 17 * number as u32,
        output_frames: 29 * number as u32,
    }
}

fn wait_error() -> BackendError {
    BackendError::ResourceUnavailable("mock native wait interrupted".into())
}

fn start_error() -> BackendError {
    BackendError::ResourceUnavailable("mock native start rejected".into())
}

fn release_error() -> BackendError {
    BackendError::Failed("mock native release rejected".into())
}

fn stop_error() -> BackendError {
    BackendError::Failed("mock native stop rejected".into())
}

fn unsupported_error() -> BackendError {
    BackendError::Unsupported("mock native exact configuration unsupported".into())
}

struct MockBackend {
    first: Fault,
    replacement: Fault,
    probe: Arc<Probe>,
    observations: ProfessionalObservations,
    recovery: RecoveryMonitor,
}

impl EngineBackend<f64> for MockBackend {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<f64>, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        assert_eq!(request.mode, PrepareMode::Initial);
        assert!(request.reuse.is_empty());
        let mut bindings = NodeBindings::new();
        let rate = request.project.processing.engine.processing_sr as u32;
        let (node, output) = request
            .project
            .nodes
            .iter()
            .find_map(|(&node, config)| match config {
                NodeConfig::Output(output) => Some((node, output)),
                _ => None,
            })
            .expect("fixture has an output master");
        let configuration = NodeConfig::Output(output.clone());
        let ticket = self.recovery.register(
            request.owner_generation,
            vec![(node, configuration.clone())],
            rate,
        );
        let counters = Arc::new(Counters::default());
        let session = Arc::new(Session {
            generation: request.owner_generation,
            node,
            configuration,
            processing_rate: rate,
            counters: counters.clone(),
            ticket: ticket.clone(),
            cancel: cancel.clone(),
            written: AtomicUsize::new(0),
            nonzero: AtomicUsize::new(0),
            notifications: AtomicUsize::new(0),
        });
        self.probe.sessions.lock().unwrap().push(session.clone());
        let mut transport = Box::new(Transport::<f64>::new(
            request.owner_generation,
            self.observations.clone(),
        ));
        for (&node, config) in &request.project.nodes {
            if let NodeConfig::Input(input) = config {
                let InputSource::Software { name } = &input.source else {
                    panic!("fixture has only a software source");
                };
                bindings
                    .bind_source(
                        node,
                        GuardedTone {
                            tone: Tone::new(name, rate)?,
                            probe: self.probe.clone(),
                            rendered: false,
                        },
                    )
                    .unwrap();
            }
        }
        let (tx, rx) = rtrb::RingBuffer::new(4096);
        bindings
            .bind_sink(
                node,
                RingSink::<f64> {
                    tx,
                    counters: counters.clone(),
                    marker: PhantomData,
                },
            )
            .unwrap();
        let native_info = NativeStreamInfo {
            sample_rate: output.device.hardware_sample_rate.unwrap(),
            format: output.device.sample_format,
            buffer_frames: output.device.period_frames.unwrap() as u32,
            channels: output.channels,
        };
        let probe = self.probe.clone();
        let first = self.first;
        let replacement = self.replacement;
        let owner_cancel = cancel.clone();
        let (worker, actual) = owner::spawn(
            WorkerRequest {
                capture_clock: None,
                factory: Box::new(move || {
                    let number = probe.opens.fetch_add(1, Ordering::AcqRel) + 1;
                    probe.record(Event::Open(number));
                    let fault = if number == 1 { first } else { replacement };
                    if matches!(fault, Fault::Unsupported) {
                        return Err(unsupported_error());
                    }
                    assert_eq!(
                        probe.alive.fetch_add(1, Ordering::AcqRel),
                        0,
                        "replacement opened before old native instance was dropped"
                    );
                    Ok(Box::new(FakeNative {
                        info: native_info,
                        fault,
                        number,
                        callbacks: 0,
                        notified: false,
                        cancel: owner_cancel,
                        session,
                        probe,
                    }) as Box<dyn NativeStream>)
                }),
                capture: None,
                render: Some(rx),
                counters: counters.clone(),
                state: transport.state.clone(),
                slot: transport.slot.clone(),
                timeline: transport.timeline.clone(),
                engine_rate: rate,
                capacity: 4096,
                master: true,
                recovery: Some(ticket),
            },
            cancel,
        )?;
        transport.workers.push(worker);
        if let Some(mixed) = &self.probe.mixed {
            let follower = Follower {
                probe: self.probe.clone(),
                mixed: mixed.clone(),
            };
            let (worker, _) = owner::spawn(
                WorkerRequest {
                    capture_clock: None,
                    factory: Box::new(move || Ok(Box::new(follower))),
                    capture: None,
                    render: {
                        // A primed render ring models the independent WASAPI
                        // follower without adding another Engine clock owner.
                        let (mut tx, rx) = rtrb::RingBuffer::new(4096);
                        for _ in 0..1024 {
                            tx.push([0.25, 0.25]).unwrap();
                        }
                        Some(rx)
                    },
                    counters: Arc::default(),
                    state: transport.state.clone(),
                    slot: transport.slot.clone(),
                    timeline: transport.timeline.clone(),
                    engine_rate: rate,
                    capacity: 4096,
                    master: false,
                    recovery: None,
                },
                cancel,
            )?;
            transport.workers.push(worker);
            transport.auxiliaries.push(Box::new(Auxiliary {
                probe: self.probe.clone(),
                mixed: mixed.clone(),
                joined: false,
            }));
        }
        transport.sinks.push(node);
        transport.streams.push((
            ProfessionalStreamObservation {
                node,
                input: false,
                master: true,
                requested: output.device,
                actual,
                dropped_frames: 0,
                silent_frames: 0,
                native_callbacks: 0,
                native_dropped_frames: 0,
                native_silent_frames: 0,
                driver_latency: None,
            },
            counters,
        ));
        Ok(PreparedBackend {
            bindings,
            transport: Some(transport),
        })
    }

    fn poll_resource(&self) -> Option<ResourceEvent> {
        self.probe.polls.fetch_add(1, Ordering::AcqRel);
        let mixed = self
            .probe
            .mixed
            .as_ref()
            .filter(|mixed| mixed.poll_armed.swap(false, Ordering::AcqRel));
        if let Some(mixed) = mixed
            && matches!(mixed.ordering, PollOrdering::NotificationBeforePoll)
        {
            mixed.poll.pause();
        }
        let event = self.recovery.poll();
        if let Some(mixed) = mixed
            && matches!(mixed.ordering, PollOrdering::NotificationAfterPoll)
        {
            mixed.poll.pause();
        }
        let event = event?;
        self.probe.record(Event::Published(event.generation));
        self.probe.resources.lock().unwrap().push(event.clone());
        Some(event)
    }
}

fn project() -> (EngineProject, NodeId) {
    let mut project = EngineProject::default();
    project.processing.engine.processing_sr = 96_000.0;
    project.processing.engine.max_block_frames = 64;
    let source = project.graph.create_node(NodeKind::Source, 2).unwrap();
    let output = project.graph.create_node(NodeKind::Sink, 2).unwrap();
    project.nodes.insert(
        source,
        NodeConfig::Input(InputConfig {
            source: InputSource::Software {
                name: "moiren:tone:1000".into(),
            },
            channels: 2,
            device: DeviceOptions::default(),
        }),
    );
    project.nodes.insert(
        output,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
                endpoint_id: "asio:{01234567-89AB-CDEF-0123-456789ABCDEF}".into(),
                stable_id: Some("mock exact saved driver".into()),
            }),
            channels: 2,
            device: DeviceOptions {
                backend: AudioBackend::Asio,
                mode: DeviceMode::Exclusive,
                hardware_sample_rate: Some(44_100),
                sample_format: SampleFormat::Float64,
                period_frames: Some(128),
                clock_role: ClockRole::Master,
            },
        }),
    );
    project
        .graph
        .connect(
            project.graph.get_node(source).unwrap().outputs()[0].id(),
            project.graph.get_node(output).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    (project, output)
}

struct TestHost {
    host: EngineHost<f64>,
    probe: Arc<Probe>,
    observations: ProfessionalObservations,
    recovery: RecoveryMonitor,
    desired: EngineProject,
    output: NodeId,
}

impl TestHost {
    fn open(first: Fault, replacement: Fault) -> Self {
        Self::open_with_mixed(first, replacement, None)
    }

    fn open_with_mixed(first: Fault, replacement: Fault, mixed: Option<Arc<Mixed>>) -> Self {
        let (desired, output) = project();
        let probe = Arc::new(Probe {
            mixed,
            ..Default::default()
        });
        let observations = ProfessionalObservations::default();
        let recovery = RecoveryMonitor::default();
        let backend = MockBackend {
            first,
            replacement,
            probe: probe.clone(),
            observations: observations.clone(),
            recovery: recovery.clone(),
        };
        let host = EngineHost::open(
            HostConfig {
                poll_interval: POLL,
                ..Default::default()
            },
            desired.clone(),
            backend,
        )
        .unwrap();
        Self {
            host,
            probe,
            observations,
            recovery,
            desired,
            output,
        }
    }

    fn running(&self, generation_after: u64) -> HostSnapshot {
        wait_host(&self.host.observer(), |snapshot| {
            snapshot.execution == ExecutionState::Running
                && snapshot.owner_generation > generation_after
                && snapshot
                    .timeline
                    .is_some_and(|timeline| timeline.frame >= MIN_RENDERED as u64)
                && self
                    .probe
                    .sessions
                    .lock()
                    .unwrap()
                    .last()
                    .is_some_and(|session| {
                        session.generation == snapshot.owner_generation
                            && session.written.load(Ordering::Acquire) >= MIN_RENDERED
                            && session.nonzero.load(Ordering::Acquire) > 0
                    })
                && self.observations.snapshot().is_some_and(|native| {
                    native.owner_generation == snapshot.owner_generation
                        && native.running
                        && native.streams[0].native_callbacks > 0
                })
        })
    }

    fn arm(&self) {
        self.probe.inject.store(true, Ordering::Release);
    }

    fn metadata(&self, generation: u64) -> ProfessionalObservationSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let snapshot = self
                .observations
                .snapshot()
                .expect("running native session");
            assert_eq!(snapshot.owner_generation, generation);
            // A bounded seqlock read may return None during owner publication.
            // Require a coherent result, rather than interpreting contention as
            // missing or stale driver metadata.
            if snapshot.streams[0].driver_latency.is_some() {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "native latency never became readable"
            );
            thread::park_timeout(POLL);
        }
    }

    fn stable(&self, mut check: impl FnMut(&HostSnapshot)) {
        let observer = self.host.observer();
        let target = self.probe.polls.load(Ordering::Acquire) + STABLE_POLLS;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            check(&observer.snapshot());
            if self.probe.polls.load(Ordering::Acquire) >= target {
                break;
            }
            assert!(Instant::now() < deadline, "supervisor stopped polling");
            thread::park_timeout(POLL);
        }
    }

    fn blocked(&self, error: BackendError, cleanup: Option<BackendError>) -> HostSnapshot {
        let blocked = wait_host(&self.host.observer(), |snapshot| {
            matches!(&snapshot.update, UpdateState::Blocked { graph_revision: 1, reason: Diagnostic::Backend(reason) } if reason == &error)
                && snapshot.cleanup_error == cleanup
                && self.probe.alive.load(Ordering::Acquire) == 0
        });
        assert_ne!(blocked.execution, ExecutionState::Running);
        assert!(blocked.active_revision.is_none());
        assert!(blocked.fault.is_none());
        assert!(self.observations.snapshot().is_none());
        self.stable(|snapshot| {
            assert_eq!(snapshot, &blocked, "terminal state must remain blocked")
        });
        blocked
    }

    fn close(self, opens: usize) -> HostSnapshot {
        let observer = self.host.observer();
        drop(self.host);
        let snapshot = observer.snapshot();
        assert_eq!(snapshot.host, HostState::Closed);
        assert_eq!(self.probe.opens.load(Ordering::Acquire), opens);
        assert_eq!(self.probe.sessions.lock().unwrap().len(), opens);
        assert_eq!(self.probe.alive.load(Ordering::Acquire), 0);
        assert!(self.observations.snapshot().is_none());
        assert!(self.recovery.poll().is_none());
        snapshot
    }
}

fn wait_host(observer: &EngineObserver, predicate: impl Fn(&HostSnapshot) -> bool) -> HostSnapshot {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let snapshot = observer.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "host did not progress: {snapshot:?}"
        );
        thread::park_timeout(POLL);
    }
}

fn wait_for(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !predicate() {
        assert!(Instant::now() < deadline, "fixture did not reach barrier");
        thread::park_timeout(POLL);
    }
}

fn assert_teardown_before_reopen(probe: &Probe) {
    let events = probe.events.lock().unwrap();
    let position = |event| events.iter().position(|actual| actual == &event).unwrap();
    assert!(position(Event::Open(1)) < position(Event::Start(1)));
    let notification = events
        .iter()
        .position(|event| matches!(event, Event::Notification(1, _)))
        .unwrap();
    assert!(position(Event::Start(1)) < notification);
    assert!(notification < position(Event::Stop(1)));
    assert!(position(Event::Stop(1)) < position(Event::Release(1)));
    assert!(position(Event::Release(1)) < position(Event::Drop(1)));
    assert!(position(Event::Drop(1)) < position(Event::Published(1)));
    assert!(position(Event::Published(1)) < position(Event::Open(2)));
    if probe.mixed.is_some() {
        assert!(position(Event::FollowerStop) < position(Event::FollowerRelease));
        assert!(position(Event::FollowerRelease) < position(Event::FollowerDrop));
        assert!(position(Event::FollowerDrop) < position(Event::AuxiliaryJoin));
        assert!(position(Event::AuxiliaryJoin) < position(Event::AuxiliaryDrop));
        assert!(position(Event::AuxiliaryDrop) < position(Event::Published(1)));
    }
}

fn assert_exact_configuration(test: &TestHost, first: &HostSnapshot, recovered: &HostSnapshot) {
    assert!(recovered.owner_generation > first.owner_generation);
    assert_eq!(recovered.desired_revision, first.desired_revision);
    assert_eq!(recovered.active_revision, Some(1));
    assert_eq!(recovered.resource_generation, first.resource_generation + 1);
    assert_eq!(recovered.update, UpdateState::Idle);
    assert_eq!(recovered.cleanup_error, None);
    assert_eq!(recovered.fault, None);
    assert_eq!(test.host.observer().project(), test.desired);
    let sessions = test.probe.sessions.lock().unwrap();
    assert_eq!(sessions.len(), 2);
    for (session, snapshot) in sessions.iter().zip([first, recovered]) {
        assert_eq!(session.generation, snapshot.owner_generation);
        assert_eq!(session.node, test.output);
        assert_eq!(session.configuration, test.desired.nodes[&test.output]);
        assert_eq!(session.processing_rate, 96_000);
        assert!(session.written.load(Ordering::Acquire) >= MIN_RENDERED);
        assert!(session.nonzero.load(Ordering::Acquire) > 0);
    }
    let resources = test.probe.resources.lock().unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].node, test.output);
    assert_eq!(resources[0].configuration, test.desired.nodes[&test.output]);
    assert_eq!(resources[0].generation, 1);
    assert!(resources[0].available);
    let native = test.metadata(recovered.owner_generation);
    assert_eq!(native.owner_generation, recovered.owner_generation);
    assert!(native.running);
    assert_eq!(native.streams.len(), 1);
    let stream = &native.streams[0];
    let NodeConfig::Output(output) = &test.desired.nodes[&test.output] else {
        unreachable!()
    };
    assert_eq!(stream.node, test.output);
    assert!(!stream.input);
    assert!(stream.master);
    assert_eq!(stream.requested, output.device);
    assert_eq!(
        stream.actual,
        NativeStreamInfo {
            sample_rate: 44_100,
            format: SampleFormat::Float64,
            buffer_frames: 128,
            channels: 2
        }
    );
    assert_eq!(stream.driver_latency, Some(latency(2)));
    assert_eq!(stream.native_dropped_frames, 14);
    assert_eq!(stream.native_silent_frames, 22);
}

#[test]
fn reset_reopens_once_after_teardown_preserving_exact_configuration_and_metadata() {
    let test = TestHost::open(Fault::Recover(NativeRecoveryRequest::Reset), Fault::Healthy);
    let first = test.running(0);
    test.arm();
    let recovered = test.running(first.owner_generation);
    assert_exact_configuration(&test, &first, &recovered);
    assert_teardown_before_reopen(&test.probe);
    assert_eq!(
        test.probe.session(0).notifications.load(Ordering::Acquire),
        1
    );

    // Late metadata and lifecycle messages from the retired owner must not
    // change the replacement session or clear its observation registration.
    let old = test.probe.session(0);
    old.counters.publish_native(
        NativeStreamStats {
            callbacks: u64::MAX,
            capture_dropped_frames: u64::MAX,
            render_silent_frames: u64::MAX,
            ..Default::default()
        },
        Some(latency(99)),
    );
    test.observations.running(old.generation);
    test.observations.clear(old.generation);
    assert!(
        !old.ticket.request(),
        "retired recovery registration must be fenced"
    );
    let mut observed_latency = false;
    test.stable(|snapshot| {
        assert_eq!(snapshot.execution, ExecutionState::Running);
        assert_eq!(snapshot.owner_generation, recovered.owner_generation);
        assert_eq!(snapshot.cleanup_error, None);
        assert_eq!(test.probe.opens.load(Ordering::Acquire), 2);
        let native = test.observations.snapshot().unwrap();
        assert_eq!(native.owner_generation, recovered.owner_generation);
        assert!(native.running);
        assert_ne!(native.streams[0].native_callbacks, u64::MAX);
        assert_eq!(native.streams[0].native_dropped_frames, 14);
        assert_eq!(native.streams[0].native_silent_frames, 22);
        if let Some(actual) = native.streams[0].driver_latency {
            assert_eq!(actual, latency(2));
            observed_latency = true;
        }
    });
    assert!(
        observed_latency,
        "replacement latency must remain observable"
    );
    assert_eq!(test.close(2).cleanup_error, None);
}

#[test]
fn each_native_notification_uses_the_same_bounded_host_recovery_path() {
    for request in [
        NativeRecoveryRequest::Resync,
        NativeRecoveryRequest::SampleRateChanged,
        NativeRecoveryRequest::BufferSizeChanged,
    ] {
        let test = TestHost::open(Fault::Recover(request), Fault::Healthy);
        let first = test.running(0);
        test.arm();
        let recovered = test.running(first.owner_generation);
        assert_exact_configuration(&test, &first, &recovered);
        assert_teardown_before_reopen(&test.probe);
        assert!(
            test.probe
                .events
                .lock()
                .unwrap()
                .contains(&Event::Notification(1, request))
        );
        assert_eq!(test.close(2).cleanup_error, None);
    }
}

#[test]
fn repeated_notifications_exhaust_one_reopen_and_remain_terminal_without_retry_loop() {
    let test = TestHost::open(
        Fault::Recover(NativeRecoveryRequest::Reset),
        Fault::Recover(NativeRecoveryRequest::Reset),
    );
    let first = test.running(0);
    test.arm();
    let blocked = test.blocked(wait_error(), Some(wait_error()));
    assert!(blocked.owner_generation > first.owner_generation);
    assert_eq!(blocked.execution, ExecutionState::WaitingForResources);
    assert_eq!(blocked.resource_generation, 1);
    assert_eq!(test.probe.opens.load(Ordering::Acquire), 2);
    assert_eq!(test.probe.resources.lock().unwrap().len(), 1);
    for index in 0..2 {
        let session = test.probe.session(index);
        assert_eq!(session.notifications.load(Ordering::Acquire), 1);
        assert!(session.written.load(Ordering::Acquire) >= MIN_RENDERED);
        assert!(
            !session.ticket.request(),
            "same configuration must not replenish its budget"
        );
    }
    assert_teardown_before_reopen(&test.probe);
    test.stable(|snapshot| {
        assert_eq!(snapshot, &blocked);
        assert_eq!(test.probe.opens.load(Ordering::Acquire), 2);
    });
    test.close(2);
}

#[test]
fn unsupported_open_and_ordinary_start_errors_are_not_retried() {
    for (fault, error, cleanup) in [
        (Fault::Unsupported, unsupported_error(), None),
        (Fault::Start, start_error(), Some(start_error())),
    ] {
        let test = TestHost::open(fault, Fault::Healthy);
        let blocked = test.blocked(error, cleanup);
        assert_eq!(blocked.owner_generation, 1);
        assert_eq!(blocked.resource_generation, 0);
        assert!(test.probe.resources.lock().unwrap().is_empty());
        assert_eq!(test.probe.opens.load(Ordering::Acquire), 1);
        assert_eq!(test.probe.session(0).written.load(Ordering::Acquire), 0);
        test.close(1);
    }
}

#[test]
fn ordinary_wait_error_and_cancelled_reset_do_not_request_recovery() {
    for fault in [Fault::Wait, Fault::Cancel] {
        let test = TestHost::open(fault, Fault::Healthy);
        let first = test.running(0);
        test.arm();
        let blocked = test.blocked(wait_error(), Some(wait_error()));
        assert_eq!(blocked.owner_generation, first.owner_generation);
        assert_eq!(blocked.resource_generation, 0);
        assert_eq!(test.probe.opens.load(Ordering::Acquire), 1);
        assert!(test.probe.resources.lock().unwrap().is_empty());
        let session = test.probe.session(0);
        assert!(session.written.load(Ordering::Acquire) >= MIN_RENDERED);
        assert_eq!(
            session.notifications.load(Ordering::Acquire),
            usize::from(matches!(fault, Fault::Cancel))
        );
        test.close(1);
    }
}

#[test]
fn stop_or_release_error_keeps_native_notification_terminal() {
    for (fault, error) in [
        (Fault::Stop, stop_error()),
        (Fault::Release, release_error()),
    ] {
        let test = TestHost::open(fault, Fault::Healthy);
        let first = test.running(0);
        test.arm();
        let blocked = test.blocked(error.clone(), Some(error));
        assert_eq!(blocked.owner_generation, first.owner_generation);
        assert_eq!(blocked.resource_generation, 0);
        assert_eq!(test.probe.opens.load(Ordering::Acquire), 1);
        assert!(test.probe.resources.lock().unwrap().is_empty());
        assert_eq!(
            test.probe.session(0).notifications.load(Ordering::Acquire),
            1
        );
        test.close(1);
    }
}

#[test]
fn mixed_transport_recovery_waits_for_all_teardown_in_both_poll_orderings() {
    for ordering in [
        PollOrdering::NotificationBeforePoll,
        PollOrdering::NotificationAfterPoll,
    ] {
        for (follower_fault, auxiliary_fault, auxiliary_drop_panic, explicit_cancel) in [
            (super::Effect::Ok, super::Effect::Ok, false, false),
            (super::Effect::Error, super::Effect::Ok, false, false),
            (super::Effect::Ok, super::Effect::Error, false, false),
            (super::Effect::Panic, super::Effect::Error, false, false),
            (super::Effect::Error, super::Effect::Panic, false, false),
            (super::Effect::Error, super::Effect::Ok, true, false),
            (super::Effect::Ok, super::Effect::Ok, false, true),
        ] {
            let mixed = Arc::new(Mixed::new(ordering, follower_fault, auxiliary_fault));
            let test = TestHost::open_with_mixed(
                Fault::Recover(NativeRecoveryRequest::Reset),
                Fault::Healthy,
                Some(mixed.clone()),
            );
            let first = test.running(0);
            mixed
                .auxiliary_drop_panic
                .store(auxiliary_drop_panic, Ordering::Release);
            mixed.poll_armed.store(true, Ordering::Release);
            mixed.poll.entered();
            test.arm();
            mixed.follower_stop.entered();
            wait_for(|| test.probe.events.lock().unwrap().contains(&Event::Drop(1)));
            // Let the real supervisor consume either a poll before the
            // notification or a poll after it, while Stop is still blocked.
            mixed.poll.release();
            test.stable(|_| {});
            let early = test.probe.resources.lock().unwrap().len();
            mixed.follower_stop.release();
            mixed.auxiliary_join.entered();
            test.stable(|_| {});
            let before_auxiliary_join = test.probe.resources.lock().unwrap().len();
            if explicit_cancel {
                test.probe.session(0).cancel.cancel();
            }
            mixed.auxiliary_join.release();
            let panic = auxiliary_drop_panic
                || [follower_fault, auxiliary_fault].contains(&super::Effect::Panic);
            let terminal = explicit_cancel
                || panic
                || follower_fault != super::Effect::Ok
                || auxiliary_fault != super::Effect::Ok;
            if !terminal {
                test.running(first.owner_generation);
                assert_teardown_before_reopen(&test.probe);
            } else {
                let error = if panic {
                    BackendError::WorkerPanicked
                } else if follower_fault == super::Effect::Error {
                    super::Effect::Error.apply("follower stop").unwrap_err()
                } else if explicit_cancel {
                    wait_error()
                } else {
                    super::Effect::Error.apply("auxiliary join").unwrap_err()
                };
                let blocked = wait_host(&test.host.observer(), |snapshot| {
                    snapshot.cleanup_error == Some(error.clone())
                });
                assert_ne!(blocked.execution, ExecutionState::Running);
                assert_eq!(blocked.active_revision, None);
                assert_eq!(
                    blocked.fault,
                    panic.then_some(Diagnostic::Backend(BackendError::WorkerPanicked))
                );
                assert_eq!(blocked.resource_generation, 0);
                assert_eq!(test.probe.opens.load(Ordering::Acquire), 1);
                test.stable(|snapshot| assert_eq!(snapshot, &blocked));
                assert!(test.probe.resources.lock().unwrap().is_empty());
            }
            assert_eq!(early, 0, "published before follower Stop: {ordering:?}");
            assert_eq!(
                before_auxiliary_join, 0,
                "published before auxiliary Join: {ordering:?}"
            );
            let opens = if terminal { 1 } else { 2 };
            test.close(opens);
        }
    }
}

#[test]
fn combined_native_faults_latch_real_host_panic_without_recovery() {
    for faults in [
        super::Faults {
            run: super::Effect::Error,
            stop: super::Effect::Panic,
            ..Default::default()
        },
        super::Faults {
            run: super::Effect::Panic,
            release: super::Effect::Error,
            ..Default::default()
        },
        super::Faults {
            run: super::Effect::Error,
            stop: super::Effect::Error,
            release: super::Effect::Panic,
            ..Default::default()
        },
        super::Faults {
            run: super::Effect::Error,
            observation: super::Effect::Panic,
            release: super::Effect::Error,
            ..Default::default()
        },
        super::Faults {
            run: super::Effect::Error,
            release: super::Effect::Error,
            drop: super::Effect::Panic,
            ..Default::default()
        },
    ] {
        let test = TestHost::open(Fault::Combined(faults), Fault::Healthy);
        test.running(0);
        test.arm();
        let snapshot = wait_host(&test.host.observer(), |snapshot| {
            snapshot.cleanup_error == Some(BackendError::WorkerPanicked)
        });
        assert_eq!(
            snapshot.fault,
            Some(Diagnostic::Backend(BackendError::WorkerPanicked))
        );
        assert_ne!(snapshot.execution, ExecutionState::Running);
        assert_eq!(snapshot.active_revision, None);
        assert_eq!(snapshot.resource_generation, 0);
        test.stable(|current| assert_eq!(current, &snapshot));
        assert!(test.probe.resources.lock().unwrap().is_empty());
        assert!(
            test.probe
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, Event::EngineDrop(_)))
        );
        let events = test.probe.events.lock().unwrap();
        let render = events
            .iter()
            .find_map(|event| {
                if let Event::RenderThread(id) = event {
                    Some(*id)
                } else {
                    None
                }
            })
            .unwrap();
        let dropped = events
            .iter()
            .find_map(|event| {
                if let Event::EngineDrop(id) = event {
                    Some(*id)
                } else {
                    None
                }
            })
            .unwrap();
        assert_ne!(render, dropped, "Engine was destroyed on the native worker");
        drop(events);
        test.close(1);
    }
}
