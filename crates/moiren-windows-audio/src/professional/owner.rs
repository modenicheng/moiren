//! Native workers park before activation. The master borrows Engine during the
//! unwind boundary, then returns it through a control-only handoff after Stop.
use super::{
    bridge::{FollowerServo, refill, unsupported},
    native::{NativeStream, NativeStreamInfo, StereoFrame},
    observations::{Counters, ProfessionalObservations, ProfessionalStreamObservation},
    recovery::RecoveryTicket,
    resample::StereoResampler,
};
use crate::virtual_owner::VirtualResourceSession;
use moiren_core::graph::NodeId;
use moiren_engine::{
    compiler::CompiledBindings,
    host::{
        AttachFailure, BackendError, BackendOwner, CancellationToken, OwnerReturn, OwnerStatus,
        OwnerTimeline, PreparedTransport,
    },
    runtime::Engine,
    sample::ProcessingSample,
};
use rtrb::{Consumer, Producer};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(test)]
#[path = "owner/idle_tests.rs"]
mod idle_tests;
#[cfg(test)]
#[path = "owner/lifecycle_tests.rs"]
mod lifecycle_tests;

pub(super) trait AuxiliaryOwner: Send {
    fn activate(&self) -> Result<(), BackendError>;
    fn cancel(&self);
    fn running(&self) -> bool;
    fn finished(&self) -> bool;
    fn status(&self) -> OwnerStatus {
        if self.finished() {
            OwnerStatus::Failed(BackendError::ResourceUnavailable(
                "process capture stopped".into(),
            ))
        } else if self.running() {
            OwnerStatus::Running
        } else {
            OwnerStatus::Starting
        }
    }
    fn join(&mut self) -> Result<(), BackendError>;
}

pub(super) type StreamFactory =
    Box<dyn FnOnce() -> Result<Box<dyn NativeStream>, BackendError> + Send>;

#[derive(Default)]
pub(super) struct RunState {
    stop: AtomicBool,
    active: AtomicBool,
    gate: Condvar,
    mutex: Mutex<()>,
    failure: Mutex<Option<Failure>>,
}
struct Failure {
    error: BackendError,
    recovery: Option<RecoveryTicket>,
}
impl RunState {
    pub fn cancel(&self) {
        let _guard = self.mutex.lock().unwrap_or_else(|p| p.into_inner());
        self.stop.store(true, Ordering::Release);
        self.gate.notify_all();
    }
    fn fail(&self, error: BackendError) {
        self.record_failure(error, None);
    }
    fn record_failure(&self, error: BackendError, recovery: Option<RecoveryTicket>) {
        let mut failure = self.failure.lock().unwrap_or_else(|p| p.into_inner());
        // Another stream's terminal fault must not be hidden by a recoverable
        // ASIO candidate. A panic also dominates terminal faults from other
        // workers/auxiliaries, regardless of their completion order.
        if failure.as_ref().is_none_or(|old| {
            error == BackendError::WorkerPanicked || (old.recovery.is_some() && recovery.is_none())
        }) {
            *failure = Some(Failure { error, recovery });
        }
        drop(failure);
        self.cancel();
    }
    fn await_activation(&self) -> bool {
        let guard = self.mutex.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = self
            .gate
            .wait_while(guard, |_| {
                !self.stop.load(Ordering::Acquire) && !self.active.load(Ordering::Acquire)
            })
            .unwrap_or_else(|p| p.into_inner());
        !self.stop.load(Ordering::Acquire)
    }
    fn activate(&self) -> Result<(), BackendError> {
        // Serialize the predicate change with wait_while to avoid a lost wake.
        let _guard = self.mutex.lock().unwrap_or_else(|p| p.into_inner());
        if self.stop.load(Ordering::Acquire) {
            return Err(BackendError::Cancelled);
        }
        if self.active.swap(true, Ordering::AcqRel) {
            return Err(unsupported("professional owner was already activated"));
        }
        self.gate.notify_all();
        Ok(())
    }
}

pub(super) struct EngineSlot<S: ProcessingSample> {
    value: Mutex<Option<Engine<S>>>,
}
impl<S: ProcessingSample> Default for EngineSlot<S> {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }
}
impl<S: ProcessingSample> EngineSlot<S> {
    fn take(&self) -> Option<Engine<S>> {
        self.value.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
    fn put(&self, engine: Engine<S>) {
        *self.value.lock().unwrap_or_else(|p| p.into_inner()) = Some(engine);
    }
}

#[derive(Default)]
pub(super) struct Timeline {
    sequence: AtomicU64,
    epoch: AtomicU64,
    revision: AtomicU64,
    frame: AtomicU64,
}
impl Timeline {
    fn publish<S: ProcessingSample>(&self, engine: &Engine<S>) {
        self.sequence.fetch_add(1, Ordering::SeqCst);
        self.epoch.store(engine.epoch(), Ordering::SeqCst);
        self.revision.store(engine.revision(), Ordering::SeqCst);
        self.frame.store(engine.timeline(), Ordering::SeqCst);
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    fn snapshot(&self) -> Option<OwnerTimeline> {
        let seq = self.sequence.load(Ordering::SeqCst);
        if !seq.is_multiple_of(2) {
            return None;
        }
        let snapshot = OwnerTimeline {
            epoch: self.epoch.load(Ordering::SeqCst),
            revision: self.revision.load(Ordering::SeqCst),
            frame: self.frame.load(Ordering::SeqCst),
        };
        (seq == self.sequence.load(Ordering::SeqCst)).then_some(snapshot)
    }
}

pub(super) struct Worker {
    handle: Option<JoinHandle<()>>,
    started: Arc<AtomicBool>,
}
impl Worker {
    fn join(&mut self) -> Result<(), BackendError> {
        if let Some(handle) = self.handle.take() {
            handle.join().map_err(|_| BackendError::WorkerPanicked)?;
        }
        Ok(())
    }
}

pub(super) struct WorkerRequest<S: ProcessingSample> {
    pub factory: StreamFactory,
    pub capture: Option<Producer<StereoFrame>>,
    pub render: Option<Consumer<StereoFrame>>,
    pub counters: Arc<Counters>,
    pub state: Arc<RunState>,
    pub slot: Arc<EngineSlot<S>>,
    pub timeline: Arc<Timeline>,
    pub engine_rate: u32,
    pub capacity: usize,
    pub master: bool,
    pub recovery: Option<RecoveryTicket>,
}

/// Startup allocates buffers and negotiates native metadata before returning.
pub(super) fn spawn<S: ProcessingSample>(
    request: WorkerRequest<S>,
    cancel: &CancellationToken,
) -> Result<(Worker, NativeStreamInfo), BackendError> {
    let (tx, rx) = mpsc::sync_channel(1);
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let state = request.state.clone();
    let panic_state = state.clone();
    let handle = thread::Builder::new()
        .name(if request.master {
            "moiren-professional-master".into()
        } else {
            "moiren-professional-device".into()
        })
        .spawn(move || {
            log::debug!(
                target: "moiren::audio::professional",
                "native worker started: master={}",
                request.master,
            );
            let result = catch_unwind(AssertUnwindSafe(|| {
                native_owner(request, &tx, &worker_started)
            }));
            if result.is_err() {
                let _ = tx.send(Err(BackendError::WorkerPanicked));
                panic_state.fail(BackendError::WorkerPanicked);
            }
        })
        .map_err(|e| BackendError::Failed(e.to_string()))?;
    let mut worker = Worker {
        handle: Some(handle),
        started,
    };
    loop {
        if cancel.is_cancelled() {
            state.cancel();
            let _ = worker.join();
            return Err(BackendError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(info)) => return Ok((worker, info)),
            Ok(Err(error)) => {
                state.cancel();
                let _ = worker.join();
                return Err(error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                state.cancel();
                let _ = worker.join();
                return Err(BackendError::WorkerPanicked);
            }
        }
    }
}

fn native_owner<S: ProcessingSample>(
    mut request: WorkerRequest<S>,
    sender: &mpsc::SyncSender<Result<NativeStreamInfo, BackendError>>,
    started: &AtomicBool,
) {
    let native = (request.factory)();
    let mut stream = match native {
        Ok(stream) => stream,
        Err(error) => {
            let _ = sender.send(Err(error));
            return;
        }
    };
    let info = stream.info();
    request
        .counters
        .publish_native(stream.stats(), stream.driver_latency());
    log::debug!(
        target: "moiren::audio::professional",
        "native stream negotiated: master={} rate={} format={:?} channels={} buffer_frames={}",
        request.master,
        info.sample_rate,
        info.format,
        info.channels,
        info.buffer_frames,
    );
    if !(8_000..=384_000).contains(&info.sample_rate)
        || info.buffer_frames == 0
        || info.buffer_frames > 65536
        || info.channels == 0
        || info.channels > 32
    {
        let _ = sender.send(Err(unsupported("native stream returned invalid metadata")));
        return;
    }
    let mut pump = match Pump::new(
        request.capture.take(),
        request.render.take(),
        info,
        request.engine_rate,
        request.capacity,
        request.counters.clone(),
        request.master,
    ) {
        Ok(pump) => pump,
        Err(error) => {
            let _ = sender.send(Err(error));
            return;
        }
    };
    if sender.send(Ok(info)).is_err() || !request.state.await_activation() {
        return;
    }
    let mut engine = if request.master {
        request.slot.take()
    } else {
        None
    };
    if request.master && engine.is_none() {
        request.state.fail(BackendError::Failed(
            "master activated without Engine".into(),
        ));
        return;
    }
    // Engine is outside this closure: processor/native panics cannot unwind it.
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), BackendError> {
        // Activation authorizes native start even when the master has no PCM.
        // The follower pump supplies silence without advancing Engine.
        stream.start()?;
        log::debug!(
            target: "moiren::audio::professional",
            "native stream started: master={}",
            request.master,
        );
        started.store(true, Ordering::Release);
        while !request.state.stop.load(Ordering::Acquire) {
            // Zero transfers and wait timeouts describe supply/demand, not
            // resource health. Only explicit native errors retire this owner.
            pump.cycle(&mut *stream, engine.as_mut(), &request.timeline)?;
            // Waiting belongs to the native owner, outside Engine::render.
            stream.wait(Duration::from_millis(10))?;
            request
                .counters
                .publish_native(stream.stats(), stream.driver_latency());
        }
        Ok(())
    }));
    let stopped = catch_unwind(AssertUnwindSafe(|| stream.stop()));
    // Even a panic or Stop failure returns the complete runtime before any
    // worker-local destruction. The host alone retires/drops its processors.
    if let Some(engine) = engine {
        request.slot.put(engine);
    }
    let notification = catch_unwind(AssertUnwindSafe(|| {
        request
            .counters
            .publish_native(stream.stats(), stream.driver_latency());
        stream.recovery_request()
    }));
    let recoverable = matches!(&result, Ok(Err(error)) if *error != BackendError::WorkerPanicked)
        && matches!(&stopped, Ok(Ok(())))
        && matches!(&notification, Ok(Some(_)));
    let released = catch_unwind(AssertUnwindSafe(|| stream.release()));
    // Final Drop releases the apartment/lease as well as the IASIO reference.
    // No replacement may open until both release and destruction have finished.
    let dropped = catch_unwind(AssertUnwindSafe(|| drop(stream)));
    let recovery = (recoverable && matches!(&released, Ok(Ok(()))) && dropped.is_ok())
        .then_some(request.recovery)
        .flatten();
    let mut failure = None;
    for (phase, outcome) in [
        ("run", result.unwrap_or(Err(BackendError::WorkerPanicked))),
        ("stop", stopped.unwrap_or(Err(BackendError::WorkerPanicked))),
        (
            "observation",
            notification
                .map(|_| ())
                .map_err(|_| BackendError::WorkerPanicked),
        ),
        (
            "release",
            released.unwrap_or(Err(BackendError::WorkerPanicked)),
        ),
        ("drop", dropped.map_err(|_| BackendError::WorkerPanicked)),
    ] {
        if let Err(error) = outcome {
            // Keep every concrete terminal diagnostic even when a later cleanup
            // error wins. No non-panic error may ever downgrade WorkerPanicked.
            log::error!(
                target: "moiren::audio::professional",
                "native {phase} failed: master={} error={error:?}",
                request.master,
            );
            if failure != Some(BackendError::WorkerPanicked) {
                failure = Some(error);
            }
        }
    }
    log::debug!(
        target: "moiren::audio::professional",
        "native stream stopped: master={} failure={:?}",
        request.master,
        failure,
    );
    if let Some(error) = failure {
        // This is only a candidate: another worker's Stop/Drop or auxiliary
        // Join can still make the entire transport terminal during retirement.
        request.state.record_failure(error, recovery);
    }
}

pub(super) struct Pump {
    capture: Option<Producer<StereoFrame>>,
    render: Option<Consumer<StereoFrame>>,
    src: Option<StereoResampler>,
    input: Box<[StereoFrame]>,
    output: Box<[StereoFrame]>,
    pending: usize,
    offset: usize,
    counters: Arc<Counters>,
    master: bool,
    servo: FollowerServo,
    max_engine_frames: usize,
}
impl Pump {
    pub(super) fn new(
        capture: Option<Producer<StereoFrame>>,
        render: Option<Consumer<StereoFrame>>,
        info: NativeStreamInfo,
        engine_rate: u32,
        capacity: usize,
        counters: Arc<Counters>,
        master: bool,
    ) -> Result<Self, BackendError> {
        let src = if render.is_some() {
            Some(
                StereoResampler::new(engine_rate, info.sample_rate, capacity)
                    .map_err(unsupported)?,
            )
        } else {
            None
        };
        // The servo targets two native periods in engine-rate frames; this
        // watermark is a drift target, never a condition for native start.
        let period_frames = (u64::from(info.buffer_frames) * u64::from(engine_rate)
            / u64::from(info.sample_rate)) as usize;
        Ok(Self {
            capture,
            render,
            src,
            input: vec![[0.0; 2]; info.buffer_frames as usize].into_boxed_slice(),
            output: vec![[0.0; 2]; info.buffer_frames as usize].into_boxed_slice(),
            pending: 0,
            offset: 0,
            counters,
            master,
            servo: FollowerServo::new(FollowerServo::start_target(period_frames, capacity)),
            max_engine_frames: (u64::from(info.buffer_frames) * u64::from(engine_rate))
                .div_ceil(u64::from(info.sample_rate)) as usize
                + 64,
        })
    }
    pub(super) fn cycle<S: ProcessingSample>(
        &mut self,
        native: &mut dyn NativeStream,
        mut engine: Option<&mut Engine<S>>,
        timeline: &Timeline,
    ) -> Result<bool, BackendError> {
        let mut progress = false;
        if let Some(tx) = &mut self.capture {
            // Snapshot-sized work: a perpetually busy capture cannot starve output.
            for _ in 0..4 {
                let frames = native.read(&mut self.input)?;
                if frames > self.input.len() {
                    return Err(unsupported("native capture exceeded its destination"));
                }
                if frames == 0 {
                    break;
                }
                progress = true;
                let accepted = frames.min(tx.slots());
                for &frame in &self.input[..accepted] {
                    let _ = tx.push(frame);
                }
                self.counters
                    .dropped
                    .fetch_add((frames - accepted) as u64, Ordering::Relaxed);
            }
        }
        if let (Some(rx), Some(src)) = (&mut self.render, &mut self.src) {
            if self.pending == self.offset {
                self.pending = 0;
                self.offset = 0;
                let mut rendered = 0usize;
                if !self.master {
                    let queued = rx.slots() + src.queued_frames();
                    src.set_correction_ppm(self.servo.observe(queued));
                }
                while self.pending < self.output.len() {
                    refill(src, rx);
                    let count = src.produce(&mut self.output[self.pending..]);
                    self.pending += count;
                    if self.pending == self.output.len() {
                        break;
                    }
                    if self.master {
                        let engine = engine
                            .as_mut()
                            .ok_or_else(|| unsupported("master has no Engine"))?;
                        if rendered > self.max_engine_frames + engine.config().max_block_frames {
                            return Err(unsupported(
                                "Engine did not produce its prepared master sink",
                            ));
                        }
                        engine
                            .render(engine.config().max_block_frames)
                            .map_err(|e| BackendError::Failed(e.to_string()))?;
                        rendered += engine.config().max_block_frames;
                        timeline.publish(engine);
                    } else {
                        // Followers run only from queued master output and never
                        // advance Engine's timeline to mask a queue shortfall.
                        let missing = self.output.len() - self.pending;
                        self.output[self.pending..].fill([0.0; 2]);
                        self.counters
                            .silent
                            .fetch_add(missing as u64, Ordering::Relaxed);
                        self.pending = self.output.len();
                    }
                }
            }
            let written = native.write(&self.output[self.offset..self.pending])?;
            if written > self.pending - self.offset {
                return Err(unsupported("native render consumed beyond its source"));
            }
            self.offset += written;
            progress |= written > 0;
        }
        Ok(progress)
    }
}

pub(super) struct Transport<S: ProcessingSample> {
    pub workers: Vec<Worker>,
    pub auxiliaries: Vec<Box<dyn AuxiliaryOwner>>,
    pub state: Arc<RunState>,
    pub slot: Arc<EngineSlot<S>>,
    pub timeline: Arc<Timeline>,
    pub sinks: Vec<NodeId>,
    pub observations: ProfessionalObservations,
    pub streams: Vec<(ProfessionalStreamObservation, Arc<Counters>)>,
    pub generation: u64,
}
impl<S: ProcessingSample> Transport<S> {
    pub fn new(generation: u64, observations: ProfessionalObservations) -> Self {
        Self {
            workers: Vec::new(),
            auxiliaries: Vec::new(),
            state: Arc::default(),
            slot: Arc::default(),
            timeline: Arc::default(),
            sinks: Vec::new(),
            observations,
            streams: Vec::new(),
            generation,
        }
    }
    /// Reuse native capture retirement without attaching or creating an Engine.
    pub fn into_follower(
        self: Box<Self>,
        cancellation: CancellationToken,
    ) -> Result<Box<dyn VirtualResourceSession>, BackendError> {
        if !self.sinks.is_empty() {
            return Err(unsupported(
                "render transports cannot become capture followers",
            ));
        }
        let state = self.state.clone();
        cancellation.on_cancel(move || state.cancel())?;
        Ok(Box::new(FollowerOwner {
            transport: self,
            cancellation,
        }))
    }
    fn activate(&self) -> Result<(), BackendError> {
        for auxiliary in &self.auxiliaries {
            if let Err(error) = auxiliary.activate() {
                self.cancel();
                return Err(error);
            }
        }
        if let Err(error) = self.state.activate() {
            // Cancellation can win after auxiliary admission but before the
            // native start gate. Roll back every admitted resource immediately.
            self.cancel();
            return Err(error);
        }
        Ok(())
    }
    fn cancel(&self) {
        self.state.cancel();
        for auxiliary in &self.auxiliaries {
            if catch_unwind(AssertUnwindSafe(|| auxiliary.cancel())).is_err() {
                self.state.fail(BackendError::WorkerPanicked);
            }
        }
        self.observations.clear(self.generation);
    }
    fn failure(&self) -> Option<BackendError> {
        self.state
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|failure| failure.error.clone())
    }
    fn status(&self) -> OwnerStatus {
        if self.failure().is_some() {
            self.cancel();
            return OwnerStatus::Failed(self.failure().expect("failure retained until join"));
        }
        let mut starting = false;
        for auxiliary in &self.auxiliaries {
            match auxiliary.status() {
                OwnerStatus::Failed(error) => {
                    self.state.fail(error);
                    self.cancel();
                    return OwnerStatus::Failed(
                        self.failure().expect("auxiliary failure retained"),
                    );
                }
                OwnerStatus::Starting => starting = true,
                OwnerStatus::Running => {}
            }
        }
        if !starting
            && self
                .workers
                .iter()
                .all(|w| w.started.load(Ordering::Acquire))
            && !self.state.stop.load(Ordering::Acquire)
        {
            self.observations.running(self.generation);
            OwnerStatus::Running
        } else {
            OwnerStatus::Starting
        }
    }
    fn take_error(
        &self,
        cancellation: &CancellationToken,
        suppress_recovered: bool,
    ) -> Option<BackendError> {
        let failure = self
            .state
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        // Only the complete stop/release/drop/auxiliary aggregate can spend a
        // recovery ticket. A user cancellation fences authorization.
        failure.and_then(|failure| {
            let authorized = !cancellation.is_cancelled()
                && failure.recovery.is_some_and(|ticket| ticket.request());
            (!(authorized && suppress_recovered)).then_some(failure.error)
        })
    }
    fn cleanup(&mut self) {
        log::debug!(
            target: "moiren::audio::professional",
            "transport cleanup started: generation={} workers={} auxiliaries={}",
            self.generation,
            self.workers.len(),
            self.auxiliaries.len(),
        );
        self.state.cancel();
        for auxiliary in &self.auxiliaries {
            if catch_unwind(AssertUnwindSafe(|| auxiliary.cancel())).is_err() {
                self.state.fail(BackendError::WorkerPanicked);
            }
        }
        for worker in &mut self.workers {
            if let Err(error) = worker.join() {
                self.state.fail(error);
            }
        }
        for mut auxiliary in self.auxiliaries.drain(..) {
            if let Err(error) = catch_unwind(AssertUnwindSafe(|| auxiliary.join()))
                .unwrap_or(Err(BackendError::WorkerPanicked))
            {
                self.state.fail(error);
            }
            // Auxiliaries may own native leases too. Their destruction must be
            // included in authorization, not deferred to Transport::drop.
            if catch_unwind(AssertUnwindSafe(|| drop(auxiliary))).is_err() {
                self.state.fail(BackendError::WorkerPanicked);
            }
        }
        self.observations.clear(self.generation);
        log::debug!(
            target: "moiren::audio::professional",
            "transport cleanup finished: generation={}",
            self.generation,
        );
    }
}
impl<S: ProcessingSample> Drop for Transport<S> {
    fn drop(&mut self) {
        self.cleanup();
    }
}
impl<S: ProcessingSample> PreparedTransport<S> for Transport<S> {
    fn attach(
        mut self: Box<Self>,
        mut engine: Engine<S>,
        bindings: &CompiledBindings,
        cancel: CancellationToken,
    ) -> Result<Box<dyn BackendOwner<S>>, Box<AttachFailure<S>>> {
        let validate = || -> Result<(), BackendError> {
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            for node in &self.sinks {
                if bindings.node(*node).is_none() {
                    return Err(unsupported("compiled transport sink is missing"));
                }
            }
            Ok(())
        };
        if let Err(error) = validate() {
            return Err(Box::new(AttachFailure { engine, error }));
        }
        if engine.plan_switching_enabled() {
            for node in &self.sinks {
                if let Err(error) = engine.require_sink_reuse(bindings.node(*node).unwrap()) {
                    return Err(Box::new(AttachFailure {
                        engine,
                        error: unsupported(error),
                    }));
                }
            }
        }
        let state = self.state.clone();
        if let Err(error) = cancel.on_cancel(move || state.cancel()) {
            return Err(Box::new(AttachFailure { engine, error }));
        }
        self.timeline.publish(&engine);
        self.slot.put(engine);
        self.observations
            .publish(self.generation, std::mem::take(&mut self.streams));
        Ok(Box::new(Owner {
            transport: self,
            cancellation: cancel,
        }))
    }
}

struct Owner<S: ProcessingSample> {
    transport: Box<Transport<S>>,
    cancellation: CancellationToken,
}
struct FollowerOwner<S: ProcessingSample> {
    transport: Box<Transport<S>>,
    cancellation: CancellationToken,
}
impl<S: ProcessingSample> VirtualResourceSession for FollowerOwner<S> {
    fn activate(&self) -> Result<(), BackendError> {
        if self.cancellation.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        self.transport
            .observations
            .publish(self.transport.generation, self.transport.streams.clone());
        self.transport.activate()
    }
    fn cancel(&self) {
        self.transport.cancel();
    }
    fn status(&self) -> OwnerStatus {
        self.transport.status()
    }
    fn stop_and_join(mut self: Box<Self>) -> Result<(), BackendError> {
        self.transport.cleanup();
        // The virtual owner must receive the actual capture error even if a
        // native recovery event was authorized after successful retirement.
        self.transport
            .take_error(&self.cancellation, false)
            .map_or(Ok(()), Err)
    }
}
impl<S: ProcessingSample> BackendOwner<S> for Owner<S> {
    fn activate(&self) -> Result<(), BackendError> {
        if self.cancellation.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        self.transport.activate()
    }
    fn cancel(&self) {
        self.transport.cancel();
    }
    fn status(&self) -> OwnerStatus {
        self.transport.status()
    }
    fn timeline(&self) -> Option<OwnerTimeline> {
        self.transport.timeline.snapshot()
    }
    fn stop_and_join(mut self: Box<Self>) -> OwnerReturn<S> {
        self.transport.cleanup();
        let engine = self
            .transport
            .slot
            .take()
            .expect("Engine is retained outside native unwind boundaries");
        let error = self.transport.take_error(&self.cancellation, true);
        OwnerReturn { engine, error }
    }
}
