//! Exercise production preparation without a native device or a second Engine.
use super::super::{Group, NativeDevices, ProfessionalBackend};
use super::{request, virtual_project};
use crate::professional::owner::StreamFactory;
use crate::virtual_owner::{
    ClockWake, PreparedVirtualMicrophone, PreparedVirtualPlayback, VirtualBoundaryProvider,
    VirtualBoundaryRequest, VirtualClockFactory, VirtualRealtimeClock, VirtualResourceSession,
};
use moiren_core::graph::NodeKind;
use moiren_engine::{
    boundary::{AudioReader, AudioWriter, audio_bridge},
    compiler::compile,
    host::{BackendError, CancellationToken, OwnerStatus},
    node::{DeviceOptions, EndpointSelector},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

const RATE: usize = 48_000;
const GRAPH: usize = 256;
const PACKET: usize = 480;
const CAPACITY: usize = 4_800;
const SETTLE: usize = 2 * RATE;
const PHASE_STEP: usize = 6 * RATE;
const END: usize = 10 * RATE;
// Sample ticks, not wall time: independently arriving 10 ms packets have at
// most 1 ms of jitter, followed by an additional persistent 64/96-sample delay.
const JITTER: [usize; 8] = [0, 48, 16, 32, 0, 32, 48, 16];

#[derive(Default)]
struct PacketProvider {
    playback: Mutex<Option<AudioWriter<f32>>>,
    microphone: Mutex<Option<AudioReader<f32>>>,
}

impl PacketProvider {
    fn check_request(request: VirtualBoundaryRequest) {
        assert_eq!(request.engine_sample_rate, RATE as u32);
        assert_eq!(request.max_block_frames, GRAPH);
        assert_eq!(request.channels, 2);
    }
}

impl VirtualBoundaryProvider for PacketProvider {
    fn prepare_playback(
        &self,
        request: VirtualBoundaryRequest,
        _: &CancellationToken,
    ) -> Result<PreparedVirtualPlayback, BackendError> {
        Self::check_request(request);
        let (writer, reader) = audio_bridge(2, CAPACITY, request.audio_byte_budget).unwrap();
        assert!(self.playback.lock().unwrap().replace(writer).is_none());
        Ok(PreparedVirtualPlayback {
            carrier: reader,
            reset_epoch: Arc::new(AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            session: Box::new(UnstartedSession),
        })
    }

    fn prepare_microphone(
        &self,
        request: VirtualBoundaryRequest,
        _: &CancellationToken,
    ) -> Result<PreparedVirtualMicrophone, BackendError> {
        Self::check_request(request);
        let (writer, reader) = audio_bridge(2, CAPACITY, request.audio_byte_budget).unwrap();
        assert!(self.microphone.lock().unwrap().replace(reader).is_none());
        Ok(PreparedVirtualMicrophone {
            carrier: writer,
            reset_epoch: Arc::new(AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            session: Box::new(UnstartedSession),
        })
    }
}

struct UnstartedSession;
impl VirtualResourceSession for UnstartedSession {
    fn activate(&self) -> Result<(), BackendError> {
        panic!("packet trace must not activate transport workers");
    }
    fn cancel(&self) {}
    fn status(&self) -> OwnerStatus {
        OwnerStatus::Starting
    }
    fn stop_and_join(self: Box<Self>) -> Result<(), BackendError> {
        Ok(())
    }
}

struct NoNativeDevices;
impl NativeDevices for NoNativeDevices {
    fn resolve(
        &self,
        _: &EndpointSelector,
        _: DeviceOptions,
        _: bool,
    ) -> Result<String, BackendError> {
        panic!("virtual-only preparation must not resolve native devices");
    }
    fn factory(&self, _: &Group, _: u32) -> Result<StreamFactory, BackendError> {
        panic!("virtual-only preparation must not open native devices");
    }
}

#[derive(Default)]
struct UnusedClockFactory(AtomicUsize);
impl VirtualClockFactory for UnusedClockFactory {
    fn prepare(&self, _: crate::StopSignal) -> Result<Box<dyn VirtualRealtimeClock>, BackendError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(UnusedClock))
    }
}
struct UnusedClock;
impl VirtualRealtimeClock for UnusedClock {
    fn frequency(&self) -> u64 {
        RATE as u64
    }
    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        panic!("one manually rendered Engine owns the sample timeline");
    }
    fn wait_ticks(&mut self, _: u64) -> Result<ClockWake, BackendError> {
        panic!("deterministic packet trace must not sleep");
    }
}

#[derive(Debug, Default)]
struct Content {
    silent_frames: usize,
    invalid_frames: usize,
    flat_blocks: usize,
}
impl Content {
    fn observe(&mut self, samples: &[f32]) {
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        let mut silent = 0;
        for &[left, right] in samples.as_chunks::<2>().0 {
            if left.abs() <= 0.0001 && right.abs() <= 0.0001 {
                silent += 1;
                continue;
            }
            // A nonconstant, nonzero stereo tone permits frame-level silence
            // detection after FIR/SRC without assuming an exact fixed latency.
            if !left.is_finite()
                || !right.is_finite()
                || !(0.20..0.55).contains(&left)
                || (right + 0.25 + 0.5 * left).abs() > 0.0001
            {
                self.invalid_frames += 1;
            }
            low = low.min(left);
            high = high.max(left);
        }
        self.silent_frames += silent;
        if silent == 0 && high - low < 0.20 {
            self.flat_blocks += 1;
        }
    }
}

#[derive(Debug)]
struct Trace {
    phase: usize,
    delay: usize,
    startup_silent_frames: usize,
    steady: Content,
    shifted: Content,
}

fn trace(phase: usize, delay: usize) -> Trace {
    let mut project = virtual_project(2, RATE as u32);
    project.processing.engine.max_block_frames = GRAPH;
    let provider = Arc::new(PacketProvider::default());
    let clock = Arc::new(UnusedClockFactory::default());
    let backend = ProfessionalBackend::<f32>::new()
        .unwrap()
        .with_virtual_provider(provider.clone())
        .with_virtual_clock_factory(clock.clone());
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let mut prepared = backend
        .prepare_with_native(
            request(&project, &host),
            &CancellationToken::default(),
            &NoNativeDevices,
        )
        .unwrap();
    assert_eq!(clock.0.load(Ordering::Relaxed), 1);
    assert!(prepared.transport.is_some());

    // bind_sink rejects duplicates. Fan out the SAME prepared playback source
    // to a fresh raw sink instead: no boundary is reconstructed or replaced.
    // The original microphone stays bound but is not the content oracle; its
    // independent startup/underflow behavior cannot mask playback starvation.
    let source = *project.nodes.keys().next().unwrap();
    let diagnostic = project.graph.create_node(NodeKind::Sink, 2).unwrap();
    project
        .graph
        .connect(
            project.graph.get_node(source).unwrap().outputs()[0].id(),
            project.graph.get_node(diagnostic).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    let (writer, mut output) =
        audio_bridge(2, GRAPH, project.processing.audio_byte_budget).unwrap();
    prepared.bindings.bind_sink(diagnostic, writer).unwrap();
    let mut engine = compile(
        &project.graph,
        prepared.bindings,
        project.processing.compile_config(1, 1),
    )
    .unwrap()
    .engine;
    let mut playback = provider.playback.lock().unwrap().take().unwrap();
    let mut microphone = provider.microphone.lock().unwrap().take().unwrap();
    assert_eq!(playback.capacity_frames(), CAPACITY);
    assert_eq!(microphone.capacity_frames(), CAPACITY);
    let mut packet = [0.0; PACKET * 2];
    let mut block = [0.0; GRAPH * 2];
    let mut ignored_microphone = [0.0; GRAPH * 2];
    let mut next_packet = 0;
    let mut result = Trace {
        phase,
        delay,
        startup_silent_frames: 0,
        steady: Content::default(),
        shifted: Content::default(),
    };
    for now in (0..END).step_by(GRAPH) {
        loop {
            let native_tick = next_packet * PACKET;
            let arrival = native_tick
                + phase
                + JITTER[next_packet % JITTER.len()]
                + if native_tick >= PHASE_STEP { delay } else { 0 };
            if arrival > now {
                break;
            }
            for (index, frame) in packet.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let angle = std::f64::consts::TAU * (native_tick + index) as f64 / GRAPH as f64;
                let left = 0.375 + 0.125 * angle.sin() as f32;
                *frame = [left, -0.25 - 0.5 * left];
            }
            assert!(playback.write_interleaved_exact(&packet).unwrap());
            next_packet += 1;
        }
        engine.render(GRAPH).unwrap();
        let report = output.read_interleaved(&mut block).unwrap();
        assert_eq!(
            report.transferred_frames, GRAPH,
            "raw diagnostic sink cannot underrun"
        );
        // Drain without scoring microphone PCM, preventing its queue pressure
        // from becoming a second, unrelated cause of a failed source trace.
        microphone
            .read_interleaved(&mut ignored_microphone)
            .unwrap();
        if now < SETTLE {
            result.startup_silent_frames += block
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|frame| frame[0].abs() <= 0.0001 && frame[1].abs() <= 0.0001)
                .count();
        } else if now < PHASE_STEP {
            result.steady.observe(&block);
        } else {
            result.shifted.observe(&block);
        }
    }
    assert_eq!(engine.timeline(), END as u64);
    // Never attach the prepared transport: exactly one Engine was rendered,
    // without workers, wall-clock waits, a driver, or a native stream.
    drop(prepared.transport);
    result
}

#[test]
fn virtual_only_prepared_playback_survives_480_packet_256_graph_phase_steps() {
    let traces: Vec<_> = [(0, 64), (0, 96), (63, 64), (63, 96)]
        .into_iter()
        .map(|(phase, delay)| trace(phase, delay))
        .collect();
    for result in &traces {
        eprintln!(
            "virtual-only 480/256 phase={} delay={} settle=2s startup_silent={} steady={:?} shifted={:?}",
            result.phase, result.delay, result.startup_silent_frames, result.steady, result.shifted,
        );
    }
    for result in &traces {
        for content in [&result.steady, &result.shifted] {
            assert_eq!(
                content.silent_frames, 0,
                "post-settle starvation: {result:?}"
            );
            assert_eq!(
                content.invalid_frames, 0,
                "stereo PCM corrupted: {result:?}"
            );
            assert_eq!(
                content.flat_blocks, 0,
                "tone replaced by flat PCM: {result:?}"
            );
        }
    }
}
