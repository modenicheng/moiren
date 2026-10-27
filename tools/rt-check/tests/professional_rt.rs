//! Isolated harness: the library unit-test binary already installs an allocator.
//! Compile the actual bridge/owner files here to measure their private RT paths.
//! Run: cargo test --manifest-path tools/rt-check/Cargo.toml
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    audio_config::SampleFormat,
    boundary::{BoundaryReport, RtAudioSource},
    buffer::{AudioBlockMut, BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
    compiler::{CompileConfig, CompiledGraph, NodeBindings, compile},
    host::{BackendError, CancellationToken, OwnerStatus, PreparedTransport},
    processor::ProcessContext,
    runtime::EngineConfig,
    sample::ProcessingSample,
};
#[cfg(windows)]
use moiren_windows_audio::clock_bridge;
use moiren_windows_audio::latency;
#[cfg(windows)]
use moiren_windows_audio::process_loopback;
use moiren_windows_audio::professional::native;
use moiren_windows_audio::virtual_owner;
use std::{
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::ThreadId,
    time::{Duration, Instant},
};
#[cfg(windows)]
#[path = "../../../crates/moiren-windows-audio/src/professional/asio_rt_tests.rs"]
mod asio_rt;
#[path = "../../../crates/moiren-windows-audio/src/professional/bridge.rs"]
mod bridge;
#[path = "../../../crates/moiren-windows-audio/src/codec.rs"]
mod codec;
#[path = "../../../crates/moiren-windows-audio/src/professional/observations.rs"]
mod observations;
#[path = "../../../crates/moiren-windows-audio/src/professional/owner.rs"]
mod professional_owner;
#[cfg(windows)]
#[path = "../../../crates/moiren-windows-audio/src/owner.rs"]
mod windows_owner;
// Mirror the library's shared native helpers without replacing the workers
// whose private realtime paths this harness measures.
mod owner {
    pub(crate) use super::professional_owner::*;
    #[cfg(windows)]
    pub(crate) use super::windows_owner::Mmcss;
}
#[path = "../../../crates/moiren-windows-audio/src/professional/recovery.rs"]
mod recovery;
#[path = "../../../crates/moiren-windows-audio/src/professional/recovery_host_tests.rs"]
mod recovery_host_tests;
#[path = "../../../crates/moiren-windows-audio/src/resample.rs"]
mod resample;
#[path = "../../../crates/moiren-windows-audio/src/servo.rs"]
mod servo;
use bridge::{RingSink, RingSource, Tone};
use native::{NativeStream, NativeStreamInfo, StereoFrame};
use observations::{Counters, ProfessionalObservations};
use owner::{Pump, Timeline, Transport, WorkerRequest};

// The counting allocator is the single shared copy under
// `tests/common/alloc_guard.rs`; this separate-crate harness pulls it in with
// `#[path]` just like the shadow-compiled modules above.
#[path = "../../../crates/moiren-windows-audio/tests/common/alloc_guard.rs"]
mod alloc_guard;

fn measured<T>(f: impl FnOnce() -> T) -> T {
    alloc_guard::assert_allocates_nothing(f)
}

#[test]
fn resampler_reset_and_input_planning_are_allocation_free_without_stale_audio() {
    for input_rate in [44_100, 48_000, 88_200, 96_000] {
        for output_rate in [44_100, 48_000, 88_200, 96_000] {
            let capacity = 512;
            assert!(resample::StereoResampler::storage_bytes(capacity).unwrap() > capacity * 16);
            assert!(resample::StereoResampler::storage_bytes(usize::MAX).is_none());
            let mut src =
                resample::StereoResampler::new(input_rate, output_rate, capacity).unwrap();
            let mut output = [[0.0; 2]; 128];
            measured(|| {
                let needed = src.input_frames_needed(output.len()).unwrap();
                for _ in 0..needed {
                    assert!(src.push_frame([0.75, -0.25]));
                }
                assert_eq!(src.produce(&mut output), output.len());
                assert!(output.iter().any(|frame| frame[0] > 0.5));
                src.set_correction_ppm(2000.0);
                src.reset();

                assert_eq!(src.input_frames_needed(0), Some(0));
                let needed = src.input_frames_needed(output.len()).unwrap();
                assert!(needed <= src.free_frames());
                for _ in 0..needed - 1 {
                    assert!(src.push_frame([0.0; 2]));
                }
                let first = src.produce(&mut output);
                assert!(first < output.len());
                assert_eq!(src.input_frames_needed(output.len() - first), Some(1));
                assert!(src.push_frame([0.0; 2]));
                let rest = src.produce(&mut output[first..]);
                assert_eq!(first + rest, output.len());
                assert!(output.iter().all(|frame| *frame == [0.0; 2]));
            });
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Failure {
    None,
    Start,
    Wait,
    Write,
    Stop,
    PanicWrite,
    PanicStop,
    Recover,
    RecoverStop,
    RecoverRelease,
    Combined(Faults),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Effect {
    #[default]
    Ok,
    Error,
    Panic,
}
impl Effect {
    fn apply(self, phase: &str) -> Result<(), BackendError> {
        match self {
            Self::Ok => Ok(()),
            Self::Error => Err(BackendError::Failed(format!("injected {phase} failure"))),
            Self::Panic => panic!("injected {phase} panic"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Faults {
    run: Effect,
    stop: Effect,
    observation: Effect,
    release: Effect,
    drop: Effect,
}
#[derive(Default)]
struct Probe {
    starts: AtomicUsize,
    stops: AtomicUsize,
    written: AtomicUsize,
    releases: AtomicUsize,
    drops: AtomicUsize,
}
struct FakeNative {
    info: NativeStreamInfo,
    failure: Failure,
    probe: Arc<Probe>,
    partial: usize,
}
impl NativeStream for FakeNative {
    fn info(&self) -> NativeStreamInfo {
        self.info
    }
    fn start(&mut self) -> Result<(), BackendError> {
        self.probe.starts.fetch_add(1, Ordering::Relaxed);
        if matches!(self.failure, Failure::Start) {
            return Err(injected());
        }
        Ok(())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        self.probe.stops.fetch_add(1, Ordering::Relaxed);
        if let Failure::Combined(faults) = self.failure {
            return faults.stop.apply("stop");
        }
        assert!(
            !matches!(self.failure, Failure::PanicStop),
            "fake Stop panic"
        );
        if matches!(self.failure, Failure::Stop | Failure::RecoverStop) {
            return Err(injected());
        }
        Ok(())
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        if let Failure::Combined(faults) = self.failure {
            faults.run.apply("run")?;
        }
        if matches!(
            self.failure,
            Failure::Wait | Failure::Recover | Failure::RecoverStop | Failure::RecoverRelease
        ) {
            return Err(injected());
        }
        std::thread::yield_now();
        Ok(true)
    }
    fn recovery_request(&self) -> Option<native::NativeRecoveryRequest> {
        if let Failure::Combined(faults) = self.failure {
            // Observation itself can panic; Result errors are only meaningful
            // for run/stop/release, which have fallible native contracts.
            assert_ne!(faults.observation, Effect::Panic, "observation panic");
            return Some(native::NativeRecoveryRequest::Reset);
        }
        matches!(
            self.failure,
            Failure::Recover | Failure::RecoverStop | Failure::RecoverRelease
        )
        .then_some(native::NativeRecoveryRequest::Reset)
    }
    fn release(&mut self) -> Result<(), BackendError> {
        self.probe.releases.fetch_add(1, Ordering::Relaxed);
        if let Failure::Combined(faults) = self.failure {
            return faults.release.apply("release");
        }
        if matches!(self.failure, Failure::RecoverRelease) {
            Err(injected())
        } else {
            Ok(())
        }
    }
    fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
        Ok(0)
    }
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        assert!(
            !matches!(self.failure, Failure::PanicWrite),
            "fake native write panic"
        );
        if matches!(self.failure, Failure::Write) {
            return Err(injected());
        }
        let count = input.len().min(self.partial);
        self.probe.written.fetch_add(count, Ordering::Relaxed);
        Ok(count)
    }
}
impl Drop for FakeNative {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::Relaxed);
        if let Failure::Combined(faults) = self.failure {
            assert_ne!(faults.drop, Effect::Panic, "native Drop panic");
        }
    }
}
fn injected() -> BackendError {
    BackendError::ResourceUnavailable("injected native failure".into())
}
fn info() -> NativeStreamInfo {
    NativeStreamInfo {
        sample_rate: 44100,
        format: SampleFormat::Float64,
        buffer_frames: 128,
        channels: 2,
    }
}

struct GuardedSource<S: ProcessingSample> {
    tone: Tone,
    panic: bool,
    dropped: mpsc::Sender<ThreadId>,
    marker: PhantomData<S>,
}
impl<S: ProcessingSample> Drop for GuardedSource<S> {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}
impl<S: ProcessingSample> RtAudioSource<S> for GuardedSource<S> {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, ctx: &ProcessContext, output: AudioBlockMut<'_, S>) -> BoundaryReport {
        assert!(!self.panic, "fake DSP panic");
        self.tone.read(ctx, output)
    }
}

fn compiled<S: ProcessingSample>(
    sink: RingSink<S>,
    panic: bool,
) -> (
    CompiledGraph<S>,
    moiren_core::graph::NodeId,
    mpsc::Receiver<ThreadId>,
) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let output = graph.create_node(NodeKind::Sink, 2).unwrap();
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            graph.get_node(output).unwrap().inputs()[0].id(),
            SendParams::default(),
        )
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            GuardedSource::<S> {
                tone: Tone::new("moiren:tone:1000", 96000).unwrap(),
                panic,
                dropped: tx,
                marker: PhantomData,
            },
        )
        .unwrap();
    bindings.bind_sink(output, sink).unwrap();
    let mut compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 96000.0,
                max_block_frames: 64,
                max_events_per_block: 8,
            },
            audio_byte_budget: 1024 * 1024,
            plan_revision: 11,
            timeline_epoch: 7,
            control_capacity: 8,
            control_horizon_frames: 96000,
        },
    )
    .unwrap();
    let _plans = compiled.engine.enable_plan_switching(1).unwrap();
    (compiled, output, rx)
}

#[test]
fn codecs_and_all_rate_pairs_allocate_and_free_nothing_during_transfer() {
    let input = [[0.125, -0.25]; 128];
    for format in [
        SampleFormat::Pcm16,
        SampleFormat::Pcm24,
        SampleFormat::Pcm32,
        SampleFormat::Float32,
        SampleFormat::Float64,
    ] {
        let codec = codec::SampleCodec::new(format, 2).unwrap();
        let mut bytes = vec![0; codec.frame_bytes() * input.len()];
        let mut decoded = [[0.0; 2]; 128];
        measured(|| {
            for _ in 0..16 {
                codec.encode(&input, &mut bytes).unwrap();
                codec.decode(&bytes, &mut decoded).unwrap();
            }
        });
        assert!((decoded[0][0] - 0.125).abs() < 1e-6);
    }
    for from in [44100, 48000, 88200, 96000] {
        for to in [44100, 48000, 88200, 96000] {
            let mut src = resample::StereoResampler::new(from, to, 1024).unwrap();
            let mut output = [[0.0; 2]; 512];
            let count = measured(|| {
                let mut count = 0;
                for _ in 0..32 {
                    assert_eq!(src.push(&input), input.len());
                    src.set_correction_ppm(500.0);
                    count += src.produce(&mut output);
                }
                count
            });
            assert!(count > 1000);
        }
    }
}

fn measure_renderer<S: ProcessingSample>() {
    let counters = Arc::new(Counters::default());
    let (tx, rx) = rtrb::RingBuffer::new(4096);
    let (mut compiled, _, released) = compiled(
        RingSink::<S> {
            tx,
            counters: counters.clone(),
            marker: PhantomData,
        },
        false,
    );
    let mut pump = Pump::new(None, None, Some(rx), info(), 96000, 4096, counters, true).unwrap();
    let probe = Arc::new(Probe::default());
    let mut native = FakeNative {
        info: info(),
        failure: Failure::None,
        probe: probe.clone(),
        partial: 17,
    };
    let timeline = Timeline::default();
    measured(|| {
        for _ in 0..256 {
            pump.cycle(&mut native, Some(&mut compiled.engine), &timeline)
                .unwrap();
        }
    });
    assert_eq!(probe.written.load(Ordering::Relaxed), 32 * 128);
    assert!(compiled.engine.timeline() > 4096);
    assert!(released.try_recv().is_err());
    drop(compiled);
    assert_eq!(released.recv().unwrap(), std::thread::current().id());
}
#[test]
fn master_renderer_and_graph_sink_are_allocation_free_in_both_precisions_with_partial_writes() {
    measure_renderer::<f32>();
    measure_renderer::<f64>();
}

fn measure_capture<S: ProcessingSample>() {
    let counters = Arc::new(Counters::default());
    let (mut tx, rx) = rtrb::RingBuffer::new(4096);
    let mut source = RingSource::<S>::new(
        rx,
        44100,
        96000,
        128,
        4096,
        counters.clone(),
        std::sync::Arc::new(bridge::CaptureWriteClock::new()),
    )
    .unwrap();
    let mut arena = BufferArena::<S>::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 64,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    let ctx = ProcessContext {
        timeline_epoch: 1,
        timeline_start: 0,
        frames: 64,
        processing_sr: 96000.0,
    };
    let reports = measured(|| {
        let mut real = 0;
        for _ in 0..32 {
            for _ in 0..128 {
                tx.push([0.25, -0.25]).unwrap();
            }
            arena
                .with_io(&io, 0..64, |io| {
                    if let ProcessIo::Separate { mut outputs, .. } = io {
                        real += source
                            .read(&ctx, outputs.get_mut(0).unwrap())
                            .transferred_frames;
                    }
                })
                .unwrap();
        }
        real
    });
    assert!(reports > 0);
    assert!(counters.silent.load(Ordering::Relaxed) > 0);
}

#[test]
fn observation_snapshot_exposes_native_metadata_fields() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Source, 2).unwrap();
    let observation = observations::ProfessionalStreamObservation {
        node,
        input: true,
        master: false,
        requested: moiren_engine::node::DeviceOptions::default(),
        actual: info(),
        dropped_frames: 3,
        silent_frames: 5,
        native_callbacks: 0,
        native_dropped_frames: 0,
        native_silent_frames: 0,
        driver_latency: None,
    };
    assert_eq!(observation.node, node);
    assert!(observation.input);
    assert!(!observation.master);
    assert_eq!(
        observation.requested,
        moiren_engine::node::DeviceOptions::default()
    );
    assert_eq!(observation.actual.buffer_frames, 128);
    assert_eq!(observation.dropped_frames + observation.silent_frames, 8);
    let snapshot = observations::ProfessionalObservationSnapshot {
        owner_generation: 1,
        running: false,
        streams: vec![observation],
    };
    assert_eq!(snapshot.streams.len(), 1);
    assert_eq!(snapshot.owner_generation, 1);
    assert!(!snapshot.running);
}

#[test]
fn capture_queue_and_servo_are_allocation_free_with_startup_silence() {
    measure_capture::<f32>();
    measure_capture::<f64>();
}

fn transport<S: ProcessingSample>(
    failure: Failure,
    panic: bool,
) -> (
    Box<Transport<S>>,
    CompiledGraph<S>,
    mpsc::Receiver<ThreadId>,
    Arc<Probe>,
) {
    transport_with_recovery(failure, panic, None)
}

fn transport_with_recovery<S: ProcessingSample>(
    failure: Failure,
    panic: bool,
    recovery: Option<recovery::RecoveryTicket>,
) -> (
    Box<Transport<S>>,
    CompiledGraph<S>,
    mpsc::Receiver<ThreadId>,
    Arc<Probe>,
) {
    let observations = ProfessionalObservations::default();
    let mut transport = Box::new(Transport::<S>::new(3, observations));
    let counters = Arc::new(Counters::default());
    let (tx, rx) = rtrb::RingBuffer::new(4096);
    let (compiled, output, released) = compiled(
        RingSink::<S> {
            tx,
            counters: counters.clone(),
            marker: PhantomData,
        },
        panic,
    );
    let probe = Arc::new(Probe::default());
    let worker_probe = probe.clone();
    let (worker, _) = owner::spawn(
        WorkerRequest {
            factory: Box::new(move || {
                Ok(Box::new(FakeNative {
                    info: info(),
                    failure,
                    probe: worker_probe,
                    partial: 17,
                }))
            }),
            capture: None,
            capture_clock: None,
            render: Some(rx),
            counters,
            state: transport.state.clone(),
            slot: transport.slot.clone(),
            timeline: transport.timeline.clone(),
            engine_rate: 96000,
            capacity: 4096,
            master: true,
            recovery,
        },
        &CancellationToken::default(),
    )
    .unwrap();
    transport.workers.push(worker);
    transport.sinks.push(output);
    (transport, compiled, released, probe)
}

#[test]
fn recoverable_notifications_are_published_only_after_owner_cleanup() {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Sink, 2).unwrap();
    let config = moiren_engine::host::NodeConfig::Output(
        moiren_windows_audio::engine_backend::physical_output(
            moiren_engine::node::EndpointSelector::FollowDefault(
                moiren_engine::node::DefaultDeviceRole::Multimedia,
            ),
        ),
    );
    for failure in [
        Failure::Recover,
        Failure::RecoverStop,
        Failure::RecoverRelease,
    ] {
        let monitor = recovery::RecoveryMonitor::default();
        let ticket = monitor.register(3, vec![(node, config.clone())], 96000);
        let (transport, compiled, released, probe) =
            transport_with_recovery::<f64>(failure, false, Some(ticket));
        let owner = transport
            .attach(
                compiled.engine,
                &compiled.bindings,
                CancellationToken::default(),
            )
            .unwrap_or_else(|_| panic!("attach"));
        owner.activate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut failed = false;
        while Instant::now() < deadline {
            assert!(monitor.poll().is_none(), "worker must not publish recovery");
            if matches!(owner.status(), OwnerStatus::Failed(_)) {
                failed = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            failed,
            "notification must report Failed to trigger retirement"
        );
        assert!(monitor.poll().is_none());
        assert_eq!(probe.stops.load(Ordering::Acquire), 1);
        assert_eq!(probe.releases.load(Ordering::Acquire), 1);
        assert_eq!(probe.drops.load(Ordering::Acquire), 1);
        // Real EngineHost retirement calls cancel even for internal failures.
        // That wakeup is not explicit cancellation of the owner's token.
        owner.cancel();
        let returned = owner.stop_and_join();
        if matches!(failure, Failure::Recover) {
            let event = monitor.poll().expect("cleanup should schedule one reopen");
            assert_eq!(event.node, node);
            assert_eq!(event.configuration, config);
            assert!(event.available);
        } else {
            assert!(monitor.poll().is_none(), "teardown error must be terminal");
        }
        assert_eq!(
            returned.error.is_none(),
            matches!(failure, Failure::Recover)
        );
        assert!(released.try_recv().is_err());
        drop(returned);
        assert_eq!(released.recv().unwrap(), std::thread::current().id());
    }
}

#[test]
fn deferred_recovery_respects_explicit_cancellation_and_rejected_tickets() {
    for cancel_after_failure in [false, true] {
        for exhaust_ticket in [false, true] {
            let mut graph = LogicalGraph::new();
            let node = graph.create_node(NodeKind::Sink, 2).unwrap();
            let config = moiren_engine::host::NodeConfig::Output(
                moiren_windows_audio::engine_backend::physical_output(
                    moiren_engine::node::EndpointSelector::FollowDefault(
                        moiren_engine::node::DefaultDeviceRole::Multimedia,
                    ),
                ),
            );
            let monitor = recovery::RecoveryMonitor::default();
            let ticket = monitor.register(3, vec![(node, config)], 96000);
            if exhaust_ticket {
                assert!(ticket.request());
                assert!(monitor.poll().is_some());
            }
            let (transport, compiled, released, _) =
                transport_with_recovery::<f64>(Failure::Recover, false, Some(ticket));
            let cancel = CancellationToken::default();
            let owner = transport
                .attach(compiled.engine, &compiled.bindings, cancel.clone())
                .unwrap_or_else(|_| panic!("attach"));
            owner.activate().unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !matches!(owner.status(), OwnerStatus::Failed(_)) {
                assert!(Instant::now() < deadline, "candidate not reported");
                std::thread::yield_now();
            }
            assert!(monitor.poll().is_none());
            if cancel_after_failure {
                cancel.cancel();
            }
            owner.cancel();
            let returned = owner.stop_and_join();
            let authorized = !cancel_after_failure && !exhaust_ticket;
            assert_eq!(returned.error, (!authorized).then(injected));
            assert_eq!(monitor.poll().is_some(), authorized);
            assert!(released.try_recv().is_err());
            drop(returned);
            assert_eq!(
                released.recv_timeout(Duration::from_secs(2)).unwrap(),
                std::thread::current().id()
            );
        }
    }
}

#[test]
fn wait_error_with_stop_panic_keeps_worker_panicked() {
    assert_combined_faults(
        Faults {
            run: Effect::Error,
            stop: Effect::Panic,
            ..Default::default()
        },
        false,
    );
}

#[test]
fn native_panic_with_release_error_keeps_worker_panicked() {
    assert_combined_faults(
        Faults {
            run: Effect::Panic,
            release: Effect::Error,
            ..Default::default()
        },
        false,
    );
}

#[test]
fn dsp_panic_with_release_error_keeps_worker_panicked() {
    assert_combined_faults(
        Faults {
            release: Effect::Error,
            ..Default::default()
        },
        true,
    );
}

struct PanicCancelAuxiliary {
    finished: bool,
}

impl owner::AuxiliaryOwner for PanicCancelAuxiliary {
    fn activate(&self) -> Result<(), BackendError> {
        Ok(())
    }
    fn cancel(&self) {
        panic!("injected auxiliary cancel panic");
    }
    fn running(&self) -> bool {
        true
    }
    fn finished(&self) -> bool {
        self.finished
    }
    fn join(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
}

#[test]
fn native_failure_and_auxiliary_cancel_panic_do_not_deadlock_status() {
    for failure in [Failure::Wait, Failure::Recover] {
        assert_cancel_panic(failure, false);
    }
}

#[test]
fn finished_auxiliary_cancel_panic_keeps_panic_dominance() {
    assert_cancel_panic(Failure::None, true);
}

fn assert_cancel_panic(failure: Failure, finished: bool) {
    let (mut transport, compiled, released, probe) = transport::<f64>(failure, false);
    transport
        .auxiliaries
        .push(Box::new(PanicCancelAuxiliary { finished }));
    let owner = transport
        .attach(
            compiled.engine,
            &compiled.bindings,
            CancellationToken::default(),
        )
        .unwrap_or_else(|_| panic!("attach"));
    owner.activate().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !finished && probe.drops.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline, "native failure was not reached");
        std::thread::yield_now();
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let control = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            let status = owner.status();
            if matches!(status, OwnerStatus::Failed(_)) {
                break status;
            }
            assert!(Instant::now() < deadline, "native failure was not recorded");
            std::thread::yield_now();
        };
        sender
            .send((status, owner.stop_and_join()))
            .unwrap_or_else(|_| panic!("test receiver dropped"));
    });
    let (status, returned) = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("status/cleanup deadlocked after auxiliary cancel panic");
    control.join().unwrap();
    assert_eq!(status, OwnerStatus::Failed(BackendError::WorkerPanicked));
    assert_eq!(returned.error, Some(BackendError::WorkerPanicked));
    assert!(released.try_recv().is_err());
    drop(returned);
    assert_eq!(
        released.recv_timeout(Duration::from_secs(2)).unwrap(),
        std::thread::current().id()
    );
}

#[test]
fn combined_native_and_dsp_faults_keep_panic_dominance_and_control_side_engine_drop() {
    for dsp_panic in [false, true] {
        for run in [Effect::Ok, Effect::Error, Effect::Panic] {
            for stop in [Effect::Ok, Effect::Error, Effect::Panic] {
                for observation in [Effect::Ok, Effect::Panic] {
                    for release in [Effect::Ok, Effect::Error, Effect::Panic] {
                        for native_drop in [Effect::Ok, Effect::Panic] {
                            assert_combined_faults(
                                Faults {
                                    run,
                                    stop,
                                    observation,
                                    release,
                                    drop: native_drop,
                                },
                                dsp_panic,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn assert_combined_faults(faults: Faults, dsp_panic: bool) {
    let mut graph = LogicalGraph::new();
    let node = graph.create_node(NodeKind::Sink, 2).unwrap();
    let config = moiren_engine::host::NodeConfig::Output(
        moiren_windows_audio::engine_backend::physical_output(
            moiren_engine::node::EndpointSelector::FollowDefault(
                moiren_engine::node::DefaultDeviceRole::Multimedia,
            ),
        ),
    );
    let monitor = recovery::RecoveryMonitor::default();
    let ticket = monitor.register(3, vec![(node, config)], 96000);
    let (transport, compiled, released, probe) =
        transport_with_recovery::<f64>(Failure::Combined(faults), dsp_panic, Some(ticket));
    let owner = transport
        .attach(
            compiled.engine,
            &compiled.bindings,
            CancellationToken::default(),
        )
        .unwrap_or_else(|_| panic!("attach"));
    owner.activate().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    if !dsp_panic && faults.run == Effect::Ok {
        while probe.written.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < deadline, "no render progress: {faults:?}");
            std::thread::yield_now();
        }
        owner.cancel();
    }
    let panic = dsp_panic
        || [
            faults.run,
            faults.stop,
            faults.observation,
            faults.release,
            faults.drop,
        ]
        .contains(&Effect::Panic);
    let expected = if panic {
        Some(BackendError::WorkerPanicked)
    } else if faults.release == Effect::Error {
        Some(Effect::Error.apply("release").unwrap_err())
    } else if faults.stop == Effect::Error {
        Some(Effect::Error.apply("stop").unwrap_err())
    } else if faults.run == Effect::Error {
        Some(Effect::Error.apply("run").unwrap_err())
    } else {
        None
    };
    if let Some(error) = &expected {
        loop {
            if let OwnerStatus::Failed(actual) = owner.status() {
                assert_eq!(&actual, error, "{faults:?}, DSP panic={dsp_panic}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "failure not reported: {faults:?}"
            );
            std::thread::yield_now();
        }
    }
    assert!(monitor.poll().is_none(), "worker must never publish");
    assert!(released.try_recv().is_err(), "worker dropped Engine");
    owner.cancel();
    let returned = owner.stop_and_join();
    let recoverable = !panic
        && faults.run == Effect::Error
        && faults.stop == Effect::Ok
        && faults.release == Effect::Ok;
    assert_eq!(returned.error, if recoverable { None } else { expected });
    assert_eq!(monitor.poll().is_some(), recoverable);
    assert_eq!(probe.stops.load(Ordering::Acquire), 1);
    assert_eq!(probe.releases.load(Ordering::Acquire), 1);
    assert_eq!(probe.drops.load(Ordering::Acquire), 1);
    assert!(released.try_recv().is_err());
    drop(returned);
    assert_eq!(
        released.recv_timeout(Duration::from_secs(2)).unwrap(),
        std::thread::current().id()
    );
}

#[test]
fn owner_waits_for_activation_and_returns_engine_without_worker_destruction() {
    #[cfg(windows)]
    let _ = std::mem::size_of::<bridge::ProcessSource<f32>>();
    let (professional_transport, compiled, released, probe) =
        transport::<f64>(Failure::None, false);
    let observations = professional_transport.observations.clone();
    let owner = professional_transport
        .attach(
            compiled.engine,
            &compiled.bindings,
            CancellationToken::default(),
        )
        .unwrap_or_else(|_| panic!("attach failed"));
    assert_eq!(owner.status(), OwnerStatus::Starting);
    assert_eq!(probe.starts.load(Ordering::Relaxed), 0);
    assert!(!observations.snapshot().unwrap().running);
    owner.activate().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while owner.timeline().is_none_or(|t| t.frame == 0) && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(owner.status(), OwnerStatus::Running);
    assert!(observations.snapshot().unwrap().running);
    assert!(released.try_recv().is_err());
    let returned = owner.stop_and_join();
    assert!(returned.error.is_none());
    assert!(returned.engine.timeline() > 0);
    assert_eq!(probe.stops.load(Ordering::Relaxed), 1);
    assert!(released.try_recv().is_err());
    assert!(observations.snapshot().is_none());
    drop(returned);
    assert_eq!(released.recv().unwrap(), std::thread::current().id());
}

#[test]
fn processor_native_and_stop_panics_or_errors_always_return_engine_to_control() {
    for (failure, panic) in [
        (Failure::Start, false),
        (Failure::Wait, false),
        (Failure::Write, false),
        (Failure::Stop, false),
        (Failure::PanicWrite, false),
        (Failure::PanicStop, false),
        (Failure::None, true),
    ] {
        let (professional_transport, compiled, released, probe) = transport::<f32>(failure, panic);
        let owner = professional_transport
            .attach(
                compiled.engine,
                &compiled.bindings,
                CancellationToken::default(),
            )
            .unwrap_or_else(|_| panic!("attach failed"));
        owner.activate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(owner.status(), OwnerStatus::Failed(_)) && Instant::now() < deadline {
            if matches!(failure, Failure::Stop | Failure::PanicStop)
                && probe.written.load(Ordering::Relaxed) > 0
            {
                break;
            }
            std::thread::yield_now();
        }
        assert!(released.try_recv().is_err());
        let returned = owner.stop_and_join();
        assert!(returned.error.is_some());
        if panic || matches!(failure, Failure::PanicWrite | Failure::PanicStop) {
            assert_eq!(returned.error, Some(BackendError::WorkerPanicked));
        }
        assert_eq!(probe.stops.load(Ordering::Relaxed), 1);
        assert!(released.try_recv().is_err());
        drop(returned);
        assert_eq!(released.recv().unwrap(), std::thread::current().id());
    }
}

#[test]
fn cancellation_before_activation_and_attach_failure_keep_unrendered_engine() {
    let (professional_transport, compiled, released, probe) =
        transport::<f64>(Failure::None, false);
    let cancel = CancellationToken::default();
    let owner = professional_transport
        .attach(compiled.engine, &compiled.bindings, cancel.clone())
        .unwrap_or_else(|_| panic!("attach failed"));
    cancel.cancel();
    assert_eq!(owner.activate(), Err(BackendError::Cancelled));
    let returned = owner.stop_and_join();
    assert_eq!(returned.engine.timeline(), 0);
    assert_eq!(probe.starts.load(Ordering::Relaxed), 0);
    assert!(released.try_recv().is_err());
    drop(returned);
    assert_eq!(released.recv().unwrap(), std::thread::current().id());

    let (professional_transport, compiled, released, probe) =
        transport::<f32>(Failure::None, false);
    let cancel = CancellationToken::default();
    cancel.cancel();
    let failure = match professional_transport.attach(compiled.engine, &compiled.bindings, cancel) {
        Ok(_) => panic!("cancelled attach succeeded"),
        Err(failure) => failure,
    };
    assert_eq!(failure.error, BackendError::Cancelled);
    assert_eq!(failure.engine.timeline(), 0);
    assert_eq!(probe.starts.load(Ordering::Relaxed), 0);
    assert!(released.try_recv().is_err());
    drop(failure);
    assert_eq!(released.recv().unwrap(), std::thread::current().id());
}

#[test]
fn tone_nyquist_and_float32_finite_overflow_regressions() {
    assert!(Tone::new("moiren:tone:23000", 48000).is_ok());
    assert!(Tone::new("moiren:tone:40000", 96000).is_ok());
    assert!(Tone::new("moiren:tone:23000", 44100).is_err());
    let codec = codec::SampleCodec::new(SampleFormat::Float32, 2).unwrap();
    let mut bytes = [0; 8];
    codec.encode(&[[f64::MAX, -f64::MAX]], &mut bytes).unwrap();
    let mut decoded = [[0.0; 2]];
    codec.decode(&bytes, &mut decoded).unwrap();
    assert_eq!(decoded, [[f32::MAX as f64, -(f32::MAX as f64)]]);
}
