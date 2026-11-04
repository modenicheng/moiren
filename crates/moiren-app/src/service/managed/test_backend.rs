#![allow(dead_code, unused_imports)]
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use moiren_core::graph::{NodeId, NodeKind, SendParams};
use moiren_engine::{
    boundary::{BoundaryReport, ConstantSource, RtAudioSink},
    buffer::AudioBlock,
    compiler::{CompiledBindings, NodeBindings},
    host::*,
    node::{DeviceOptions, InputConfig, InputSource, OutputConfig, OutputTarget},
    processor::ProcessContext,
    runtime::Engine,
    sample::ProcessingSample,
};

#[derive(Default)]
pub struct State {
    pub prepares: AtomicUsize,
    pub starts: AtomicUsize,
    pub attaches: AtomicUsize,
    pub hold_attach: AtomicBool,
    pub stops: AtomicUsize,
    pub sinks: AtomicUsize,
    pub sources: AtomicUsize,
    pub last_sample: AtomicU64,
    pub unavailable: AtomicBool,
    pub block_update: AtomicBool,
    pub ignore_update_cancel: AtomicBool,
    pub panic_prepare: AtomicBool,
    pub fail_update: AtomicBool,
    pub fail_owner: AtomicBool,
    pub pause_audio: AtomicBool,
    /// Opt-in wall-clock delivery benchmark; normal lifecycle tests keep 8-frame callbacks.
    pub timed_meter_frames: AtomicUsize,
    pub render_history: Mutex<VecDeque<(u64, Instant)>>,
    pub fail_stop: AtomicBool,
    pub events: Mutex<VecDeque<ResourceEvent>>,
    pub drops: Mutex<Vec<String>>,
}
#[derive(Clone, Default)]
pub struct FakeBackend(pub Arc<State>);
impl<S: ProcessingSample> EngineBackend<S> for FakeBackend {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<S>, BackendError> {
        self.0.prepares.fetch_add(1, Ordering::SeqCst);
        if self.0.panic_prepare.swap(false, Ordering::SeqCst) {
            panic!("injected prepare panic");
        }
        if self.0.unavailable.load(Ordering::SeqCst) {
            return Err(BackendError::ResourceUnavailable(
                "selected endpoint".into(),
            ));
        }
        if request.mode == PrepareMode::Update {
            while self.0.block_update.load(Ordering::SeqCst) {
                if self.0.ignore_update_cancel.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(2));
                } else if cancel.wait_cancelled(Duration::from_millis(2)) {
                    return Err(BackendError::Cancelled);
                }
            }
            if self.0.fail_update.load(Ordering::SeqCst) {
                return Err(BackendError::Unsupported("injected update failure".into()));
            }
        }
        let mut bindings = NodeBindings::new();
        for (&node, config) in &request.project.nodes {
            if request.reuse.iter().any(|&(reused, _)| reused == node) {
                continue;
            }
            match config {
                NodeConfig::Input(config) => {
                    self.0.sources.fetch_add(1, Ordering::SeqCst);
                    bindings
                        .bind_source(
                            node,
                            ConstantSource {
                                channels: config.channels,
                                value: 1.0,
                            },
                        )
                        .unwrap();
                }
                NodeConfig::Output(config) => {
                    self.0.sinks.fetch_add(1, Ordering::SeqCst);
                    bindings
                        .bind_sink(
                            node,
                            Sink {
                                state: Arc::clone(&self.0),
                                channels: config.channels,
                            },
                        )
                        .unwrap();
                }
                _ => {}
            }
        }
        Ok(PreparedBackend {
            bindings,
            transport: (request.mode == PrepareMode::Initial)
                .then(|| Box::new(Transport(Arc::clone(&self.0))) as Box<dyn PreparedTransport<S>>),
        })
    }
    fn poll_resource(&self) -> Option<ResourceEvent> {
        self.0.events.lock().unwrap().pop_front()
    }
}
struct Sink {
    state: Arc<State>,
    channels: usize,
}
impl<S: ProcessingSample> RtAudioSink<S> for Sink {
    fn channel_count(&self) -> usize {
        self.channels
    }
    fn write(&mut self, _: &ProcessContext, input: AudioBlock<'_, S>) -> BoundaryReport {
        self.state
            .last_sample
            .store(input.channel(0)[0].to_f64().to_bits(), Ordering::SeqCst);
        BoundaryReport {
            transferred_frames: input.frames(),
            ..Default::default()
        }
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        self.state
            .drops
            .lock()
            .unwrap()
            .push(thread::current().name().unwrap_or("unnamed").to_string());
    }
}
struct Transport(Arc<State>);
impl<S: ProcessingSample> PreparedTransport<S> for Transport {
    fn attach(
        self: Box<Self>,
        mut engine: Engine<S>,
        _: &CompiledBindings,
        cancel: CancellationToken,
    ) -> Result<Box<dyn BackendOwner<S>>, Box<AttachFailure<S>>> {
        let state = Arc::clone(&self.0);
        let status = Arc::new(Mutex::new(OwnerStatus::Starting));
        let timeline = Arc::new(Mutex::new(None));
        let driver_status = Arc::clone(&status);
        let driver_timeline = Arc::clone(&timeline);
        let driver_cancel = cancel.clone();
        let activated = Arc::new(AtomicBool::new(false));
        let driver_activated = Arc::clone(&activated);
        self.0.attaches.fetch_add(1, Ordering::SeqCst);
        let thread = thread::Builder::new()
            .name("fake-audio-owner".into())
            .spawn(move || {
                while !driver_activated.load(Ordering::SeqCst) && !driver_cancel.is_cancelled() {
                    driver_cancel.wait_cancelled(Duration::from_millis(1));
                }
                let mut error = None;
                while !driver_cancel.is_cancelled() {
                    if state.fail_owner.load(Ordering::SeqCst) {
                        error = Some(BackendError::ResourceUnavailable(
                            "owner disconnected".into(),
                        ));
                        break;
                    }
                    if state.pause_audio.load(Ordering::SeqCst) {
                        driver_cancel.wait_cancelled(Duration::from_millis(1));
                        continue;
                    }
                    let timed_frames = state.timed_meter_frames.load(Ordering::Relaxed);
                    let began = Instant::now();
                    match catch_unwind(AssertUnwindSafe(|| engine.render(timed_frames.max(8)))) {
                        Ok(Ok(_)) => {
                            if timed_frames > 0 {
                                // Test-backend instrumentation only, outside the render call.
                                let mut history = state.render_history.lock().unwrap();
                                if history.len() == 2048 {
                                    history.pop_front();
                                }
                                history.push_back((engine.timeline(), began));
                            }
                            *driver_timeline.lock().unwrap() = Some(OwnerTimeline {
                                epoch: engine.epoch(),
                                revision: engine.revision(),
                                frame: engine.timeline(),
                            });
                            *driver_status.lock().unwrap() = OwnerStatus::Running;
                        }
                        Ok(Err(failure)) => {
                            error = Some(BackendError::Failed(failure.to_string()));
                            break;
                        }
                        Err(_) => {
                            error = Some(BackendError::WorkerPanicked);
                            break;
                        }
                    }
                    if timed_frames > 0 {
                        thread::sleep(Duration::from_secs_f64(timed_frames as f64 / 48_000.));
                    } else {
                        driver_cancel.wait_cancelled(Duration::from_millis(1));
                    }
                }
                if let Some(error) = &error {
                    *driver_status.lock().unwrap() = OwnerStatus::Failed(error.clone());
                }
                OwnerReturn { engine, error }
            })
            .unwrap();
        while self.0.hold_attach.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(2));
        }
        Ok(Box::new(Owner {
            state: Arc::clone(&self.0),
            thread,
            status,
            timeline,
            cancel,
            activated,
        }))
    }
}
struct Owner<S: ProcessingSample> {
    state: Arc<State>,
    thread: JoinHandle<OwnerReturn<S>>,
    status: Arc<Mutex<OwnerStatus>>,
    timeline: Arc<Mutex<Option<OwnerTimeline>>>,
    cancel: CancellationToken,
    activated: Arc<AtomicBool>,
}
impl<S: ProcessingSample> BackendOwner<S> for Owner<S> {
    fn activate(&self) -> Result<(), BackendError> {
        if self.cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        self.activated
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| BackendError::Failed("owner already activated".into()))?;
        self.state.starts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn cancel(&self) {
        self.cancel.cancel();
    }
    fn status(&self) -> OwnerStatus {
        self.status.lock().unwrap().clone()
    }
    fn timeline(&self) -> Option<OwnerTimeline> {
        *self.timeline.lock().unwrap()
    }
    fn stop_and_join(self: Box<Self>) -> OwnerReturn<S> {
        self.cancel.cancel();
        let mut result = self.thread.join().unwrap();
        if self.state.fail_stop.load(Ordering::SeqCst) {
            result.error = Some(BackendError::Failed("injected stop failure".into()));
        }
        self.state.stops.fetch_add(1, Ordering::SeqCst);
        result
    }
}
