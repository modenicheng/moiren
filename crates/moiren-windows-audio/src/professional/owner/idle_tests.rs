//! Exercise the real native owner without a device or a wall-clock audio run.
use super::*;
use crate::native::NativeStreamStats;
use moiren_core::graph::LogicalGraph;
use moiren_engine::{
    audio_config::SampleFormat,
    compiler::{CompileConfig, NodeBindings, compile},
    runtime::EngineConfig,
};
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

const WATCHDOG: Duration = Duration::from_secs(5);
// Miri checks ownership across the same transitions with fewer repeated cycles.
const IDLE_CYCLES: usize = if cfg!(miri) { 8 } else { 512 };

#[derive(Clone, Copy)]
enum Transfer {
    EmptyCapture,
    Backpressure,
    Silence,
    InvalidCapture,
    InvalidRender,
    WaitError,
}

#[derive(Default)]
struct Probe {
    starts: AtomicUsize,
    stops: AtomicUsize,
    releases: AtomicUsize,
    drops: AtomicUsize,
    waits: AtomicUsize,
    reads: AtomicUsize,
    writes: AtomicUsize,
    nonzero_frames: AtomicUsize,
    accept_writes: AtomicBool,
    cross_stall_deadline: AtomicBool,
    reached: (Mutex<bool>, Condvar),
    resume: (Mutex<bool>, Condvar),
}

impl Probe {
    fn wait_for_idle(&self) -> bool {
        let (reached, _) = self
            .reached
            .1
            .wait_timeout_while(self.reached.0.lock().unwrap(), WATCHDOG, |reached| {
                !*reached
            })
            .unwrap();
        *reached
    }

    fn resume(&self) {
        *self.resume.0.lock().unwrap() = true;
        self.resume.1.notify_all();
    }
}

struct IdleNative {
    probe: Arc<Probe>,
    transfer: Transfer,
}

impl NativeStream for IdleNative {
    fn info(&self) -> NativeStreamInfo {
        NativeStreamInfo {
            sample_rate: 48_000,
            format: SampleFormat::Float32,
            buffer_frames: 128,
            channels: 2,
        }
    }

    fn start(&mut self) -> Result<(), BackendError> {
        self.probe.starts.fetch_add(1, Ordering::Release);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.probe.stops.fetch_add(1, Ordering::Release);
        Ok(())
    }

    fn release(&mut self) -> Result<(), BackendError> {
        self.probe.releases.fetch_add(1, Ordering::Release);
        Ok(())
    }

    fn read(&mut self, output: &mut [StereoFrame]) -> Result<usize, BackendError> {
        self.probe.reads.fetch_add(1, Ordering::Release);
        Ok(if matches!(self.transfer, Transfer::InvalidCapture) {
            output.len() + 1
        } else {
            0
        })
    }

    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        self.probe.writes.fetch_add(1, Ordering::Release);
        assert!(input.iter().flatten().all(|sample| sample.is_finite()));
        let count = match self.transfer {
            Transfer::InvalidRender => input.len() + 1,
            Transfer::Silence => input.len(),
            Transfer::Backpressure if self.probe.accept_writes.load(Ordering::Acquire) => {
                input.len()
            }
            _ => 0,
        };
        if count <= input.len() {
            self.probe.nonzero_frames.fetch_add(
                input[..count]
                    .iter()
                    .filter(|frame| **frame != [0.0; 2])
                    .count(),
                Ordering::Release,
            );
        }
        Ok(count)
    }

    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        if matches!(self.transfer, Transfer::WaitError) {
            return Err(BackendError::ResourceUnavailable(
                "explicit native wait failure".into(),
            ));
        }
        let waits = self.probe.waits.fetch_add(1, Ordering::Release) + 1;
        if waits == 1 && self.probe.cross_stall_deadline.load(Ordering::Acquire) {
            // One dedicated regression crosses the old Instant watchdog.
            // Other idle cases use bounded iterations and do not sleep.
            thread::sleep(Duration::from_millis(3100));
        }
        if waits == IDLE_CYCLES {
            *self.probe.reached.0.lock().unwrap() = true;
            self.probe.reached.1.notify_all();
            let guard = self.probe.resume.0.lock().unwrap();
            // A test watchdog only; this does not implement stream liveness.
            let (guard, _) = self
                .probe
                .resume
                .1
                .wait_timeout_while(guard, WATCHDOG, |resume| !*resume)
                .unwrap();
            drop(guard);
        }
        Ok(waits.is_multiple_of(2))
    }

    fn stats(&self) -> NativeStreamStats {
        let waits = self.probe.waits.load(Ordering::Acquire) as u64;
        NativeStreamStats {
            wait_wakes: waits / 2,
            wait_timeouts: waits.div_ceil(2),
            zero_writes: if matches!(self.transfer, Transfer::Backpressure) {
                self.probe.writes.load(Ordering::Acquire) as u64
            } else {
                0
            },
            ..Default::default()
        }
    }
}

impl Drop for IdleNative {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::Release);
    }
}

struct Fixture {
    worker: Worker,
    state: Arc<RunState>,
    timeline: Arc<Timeline>,
    counters: Arc<Counters>,
    slot: Arc<EngineSlot<f32>>,
    probe: Arc<Probe>,
    // Keep the other ring end alive, while deliberately providing no content.
    _capture: Option<Consumer<StereoFrame>>,
    _render: Option<Producer<StereoFrame>>,
}

impl Fixture {
    fn new(transfer: Transfer) -> Self {
        Self::with_master(transfer, false)
    }

    fn with_master(transfer: Transfer, master: bool) -> Self {
        let capture = matches!(
            transfer,
            Transfer::EmptyCapture | Transfer::InvalidCapture | Transfer::WaitError
        );
        let (producer, consumer) = rtrb::RingBuffer::new(1024);
        let (capture_tx, capture_rx, render_tx, render_rx) = if capture {
            (Some(producer), Some(consumer), None, None)
        } else {
            (None, None, Some(producer), Some(consumer))
        };
        let probe = Arc::new(Probe::default());
        let stream_probe = probe.clone();
        let state = Arc::new(RunState::default());
        let timeline = Arc::new(Timeline::default());
        let counters = Arc::new(Counters::default());
        let slot = Arc::new(EngineSlot::default());
        if master {
            let compiled = compile(
                &LogicalGraph::new(),
                NodeBindings::new(),
                CompileConfig {
                    engine: EngineConfig {
                        processing_sr: 48_000.0,
                        max_block_frames: 64,
                        max_events_per_block: 8,
                    },
                    audio_byte_budget: 4096,
                    plan_revision: 1,
                    timeline_epoch: 1,
                    control_capacity: 8,
                    control_horizon_frames: 48_000,
                },
            )
            .unwrap();
            slot.put(compiled.engine);
        }
        let (worker, _) = spawn(
            WorkerRequest::<f32> {
                factory: Box::new(move || {
                    Ok(Box::new(IdleNative {
                        probe: stream_probe,
                        transfer,
                    }))
                }),
                capture: capture_tx,
                capture_clock: capture.then(|| Arc::new(CaptureWriteClock::new())),
                render: render_rx,
                counters: counters.clone(),
                state: state.clone(),
                slot: slot.clone(),
                timeline: timeline.clone(),
                engine_rate: 48_000,
                capacity: 1024,
                master,
                recovery: None,
            },
            &CancellationToken::default(),
        )
        .unwrap();
        Self {
            worker,
            state,
            timeline,
            counters,
            slot,
            probe,
            _capture: capture_rx,
            _render: render_tx,
        }
    }

    fn finish(&mut self) {
        self.state.cancel();
        self.probe.resume();
        self.worker.join().unwrap();
    }
}

#[test]
fn master_survives_idle_capture_and_returns_its_unadvanced_engine() {
    let mut fixture = Fixture::with_master(Transfer::EmptyCapture, true);
    fixture.state.activate().unwrap();
    let reached = fixture.probe.wait_for_idle();
    let healthy = !fixture.state.stop.load(Ordering::Acquire)
        && fixture.state.failure.lock().unwrap().is_none();
    fixture.finish();
    assert!(reached, "master failed while capture had no PCM");
    assert!(healthy);
    assert!(fixture.worker.started.load(Ordering::Acquire));
    assert_eq!(fixture.slot.take().unwrap().timeline(), 0);
    assert_eq!(fixture.probe.stops.load(Ordering::Acquire), 1);
    assert_eq!(fixture.probe.releases.load(Ordering::Acquire), 1);
    assert_eq!(fixture.probe.drops.load(Ordering::Acquire), 1);
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.finish();
    }
}

#[test]
fn follower_starts_without_master_content_and_supplies_silence() {
    let mut fixture = Fixture::new(Transfer::Silence);
    fixture.state.activate().unwrap();
    let reached = fixture.probe.wait_for_idle();
    fixture.finish();
    assert!(
        reached,
        "follower waited for master PCM before native start"
    );
    assert_eq!(fixture.probe.starts.load(Ordering::Acquire), 1);
    assert!(fixture.worker.started.load(Ordering::Acquire));
    assert!(fixture.counters.silent.load(Ordering::Acquire) > 0);
    assert!(fixture.probe.writes.load(Ordering::Acquire) >= IDLE_CYCLES);
    assert_eq!(fixture.probe.nonzero_frames.load(Ordering::Acquire), 0);
    assert!(fixture.state.failure.lock().unwrap().is_none());
    assert_eq!(fixture.timeline.snapshot().unwrap().frame, 0);
}

#[test]
fn follower_resumes_pcm_after_backpressure_without_restart_or_graph_execution() {
    let mut fixture = Fixture::new(Transfer::Backpressure);
    fixture.state.activate().unwrap();
    assert!(fixture.probe.wait_for_idle());
    assert_eq!(fixture.probe.nonzero_frames.load(Ordering::Acquire), 0);
    let producer = fixture._render.as_mut().unwrap();
    for _ in 0..producer.slots() {
        producer.push([0.25, -0.125]).unwrap();
    }
    fixture.probe.accept_writes.store(true, Ordering::Release);
    fixture.probe.resume();
    let deadline = Instant::now() + WATCHDOG;
    while fixture.probe.nonzero_frames.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
        thread::yield_now();
    }
    fixture.finish();
    assert!(fixture.probe.nonzero_frames.load(Ordering::Acquire) > 0);
    assert_eq!(fixture.probe.starts.load(Ordering::Acquire), 1);
    assert_eq!(fixture.timeline.snapshot().unwrap().frame, 0);
    assert!(fixture.state.failure.lock().unwrap().is_none());
}

#[test]
fn idle_capture_and_render_backpressure_do_not_fail_or_advance_the_graph() {
    for transfer in [Transfer::EmptyCapture, Transfer::Backpressure] {
        let mut fixture = Fixture::new(transfer);
        fixture.state.activate().unwrap();
        let reached = fixture.probe.wait_for_idle();
        // Inspect before cancelling: a naturally completed idle worker would
        // otherwise be indistinguishable from a successful retirement.
        let healthy = !fixture.state.stop.load(Ordering::Acquire)
            && fixture.state.failure.lock().unwrap().is_none();
        fixture.finish();
        assert!(reached, "idle stream failed or never started");
        assert!(healthy, "empty queues or backpressure terminated the owner");
        assert_eq!(fixture.timeline.snapshot().unwrap().frame, 0);
        assert_eq!(fixture.probe.stops.load(Ordering::Acquire), 1);
        assert_eq!(fixture.probe.releases.load(Ordering::Acquire), 1);
        assert_eq!(fixture.probe.drops.load(Ordering::Acquire), 1);
    }
}

#[test]
fn master_and_follower_idle_survive_the_former_stall_deadline() {
    let mut master = Fixture::with_master(Transfer::EmptyCapture, true);
    let mut follower = Fixture::new(Transfer::EmptyCapture);
    for fixture in [&master, &follower] {
        fixture
            .probe
            .cross_stall_deadline
            .store(true, Ordering::Release);
        fixture.state.activate().unwrap();
    }
    let reached = master.probe.wait_for_idle() && follower.probe.wait_for_idle();
    let healthy = [&master, &follower].into_iter().all(|fixture| {
        !fixture.state.stop.load(Ordering::Acquire)
            && fixture.state.failure.lock().unwrap().is_none()
    });
    master.finish();
    follower.finish();
    assert!(
        reached,
        "an idle owner retired after the former three seconds"
    );
    assert!(healthy);
    assert_eq!(master.slot.take().unwrap().timeline(), 0);
}

#[test]
fn empty_follower_can_be_cancelled_before_activation_without_native_start() {
    let mut fixture = Fixture::new(Transfer::Silence);
    fixture.finish();
    assert_eq!(fixture.probe.starts.load(Ordering::Acquire), 0);
    assert_eq!(fixture.probe.drops.load(Ordering::Acquire), 1);
    assert!(fixture.state.failure.lock().unwrap().is_none());
}

#[test]
fn explicit_native_errors_and_invalid_transfer_lengths_still_fail() {
    for transfer in [
        Transfer::WaitError,
        Transfer::InvalidCapture,
        Transfer::InvalidRender,
    ] {
        let mut fixture = Fixture::new(transfer);
        fixture.state.activate().unwrap();
        let deadline = Instant::now() + WATCHDOG;
        while !fixture.state.stop.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        let failed = fixture.state.failure.lock().unwrap().is_some();
        fixture.finish();
        assert!(failed, "native error or invalid length was ignored");
        assert_eq!(fixture.probe.stops.load(Ordering::Acquire), 1);
        assert_eq!(fixture.probe.releases.load(Ordering::Acquire), 1);
        assert_eq!(fixture.probe.drops.load(Ordering::Acquire), 1);
    }
}
