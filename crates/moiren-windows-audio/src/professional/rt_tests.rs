//! Isolated harness: the library unit-test binary already installs an allocator.
//! Compile the actual bridge/owner files here to measure their private RT paths.
//! Run: cargo test --manifest-path crates/moiren-windows-audio/src/professional/rt-check/Cargo.toml
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
use moiren_windows_audio::professional::{codec, native, resample};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::ThreadId,
    time::{Duration, Instant},
};
#[path = "bridge.rs"]
mod bridge;
#[path = "observations.rs"]
mod observations;
#[path = "owner.rs"]
mod owner;
use bridge::{RingSink, RingSource, Tone};
use native::{NativeStream, NativeStreamInfo, StereoFrame};
use observations::{Counters, ProfessionalObservations};
use owner::{Pump, Timeline, Transport, WorkerRequest};

struct Allocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
        }
        // SAFETY: preserve the GlobalAlloc caller's layout contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let _ = FREES.try_with(|n| n.set(n.get() + 1));
        }
        // SAFETY: allocations are forwarded to the same System allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn measured<T>(f: impl FnOnce() -> T) -> T {
    ALLOCS.with(|n| n.set(0));
    FREES.with(|n| n.set(0));
    TRACK.with(|n| n.set(true));
    let value = f();
    TRACK.with(|n| n.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "RT allocated");
    assert_eq!(FREES.with(Cell::get), 0, "RT deallocated");
    value
}

#[derive(Clone, Copy)]
enum Failure {
    None,
    Start,
    Wait,
    Write,
    Stop,
    PanicWrite,
    PanicStop,
}
#[derive(Default)]
struct Probe {
    starts: AtomicUsize,
    stops: AtomicUsize,
    written: AtomicUsize,
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
        assert!(
            !matches!(self.failure, Failure::PanicStop),
            "fake Stop panic"
        );
        if matches!(self.failure, Failure::Stop) {
            return Err(injected());
        }
        Ok(())
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        if matches!(self.failure, Failure::Wait) {
            return Err(injected());
        }
        std::thread::yield_now();
        Ok(true)
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
    let mut pump = Pump::new(None, Some(rx), info(), 96000, 4096, counters, true).unwrap();
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
    let mut source = RingSource::<S>::new(rx, 44100, 96000, 128, 4096, counters.clone()).unwrap();
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
            render: Some(rx),
            counters,
            state: transport.state.clone(),
            slot: transport.slot.clone(),
            timeline: transport.timeline.clone(),
            engine_rate: 96000,
            capacity: 4096,
            master: true,
        },
        &CancellationToken::default(),
    )
    .unwrap();
    transport.workers.push(worker);
    transport.sinks.push(output);
    (transport, compiled, released, probe)
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
