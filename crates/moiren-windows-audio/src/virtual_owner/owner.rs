use super::{ClockSchedule, ClockWake, VirtualRealtimeClock, VirtualSessions};
use crate::{Activation, ActivationGate, StopSignal, capture::CaptureSession};
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
use std::{
    os::windows::io::AsRawHandle,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
};
use windows::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::WaitForSingleObject,
};

mod trace;
use trace::{ClockTrace, GapStage};

/// A prepared virtual-only clock owner. Physical-master projects never create
/// this transport; their native owner receives these same boundary bindings.
pub struct VirtualPreparedTransport {
    sinks: Vec<NodeId>,
    sessions: VirtualSessions,
    captures: Vec<CaptureSession>,
    stop: StopSignal,
    gate: ActivationGate,
    clock: Option<Box<dyn VirtualRealtimeClock>>,
    recovery_cap_periods: u64,
}

impl VirtualPreparedTransport {
    pub fn new(
        sinks: Vec<NodeId>,
        sessions: VirtualSessions,
        stop: StopSignal,
        gate: ActivationGate,
        clock: Box<dyn VirtualRealtimeClock>,
    ) -> Self {
        Self {
            sinks,
            sessions,
            captures: Vec::new(),
            stop,
            gate,
            clock: Some(clock),
            recovery_cap_periods: super::DEBT_CAP_PERIODS,
        }
    }

    /// Set only from preparation-side admission; no budget work runs on RT.
    pub fn with_recovery_cap(mut self, periods: u64) -> Result<Self, BackendError> {
        self.recovery_cap_periods = super::VirtualBufferOptions {
            recovery_cap_periods: Some(periods),
            ..Default::default()
        }
        .admitted_recovery_cap_periods()
        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        Ok(self)
    }

    pub fn with_captures(mut self, captures: Vec<CaptureSession>) -> Self {
        self.captures = captures;
        self
    }
}

impl Drop for VirtualPreparedTransport {
    fn drop(&mut self) {
        if self.clock.is_none() && self.sessions.is_empty() && self.captures.is_empty() {
            return;
        }
        let _ = self.stop.request_stop();
        self.sessions.cancel();
        for capture in self.captures.drain(..) {
            let _ = capture.join();
        }
        let _ = self.sessions.stop_and_join();
    }
}

struct Published {
    state: AtomicU8,
    sequence: AtomicU64,
    frame: AtomicU64,
    epoch: AtomicU64,
    revision: AtomicU64,
}

impl Published {
    fn new<S: ProcessingSample>(engine: &Engine<S>) -> Self {
        Self {
            state: AtomicU8::new(0),
            sequence: AtomicU64::new(0),
            frame: AtomicU64::new(engine.timeline()),
            epoch: AtomicU64::new(engine.epoch()),
            revision: AtomicU64::new(engine.revision()),
        }
    }
    fn publish<S: ProcessingSample>(&self, engine: &Engine<S>) {
        let sequence = self.sequence.load(Ordering::SeqCst);
        self.sequence
            .store(sequence.wrapping_add(1), Ordering::SeqCst);
        self.frame.store(engine.timeline(), Ordering::SeqCst);
        self.epoch.store(engine.epoch(), Ordering::SeqCst);
        self.revision.store(engine.revision(), Ordering::SeqCst);
        self.sequence
            .store(sequence.wrapping_add(2), Ordering::SeqCst);
    }
    fn snapshot(&self) -> Option<OwnerTimeline> {
        let sequence = self.sequence.load(Ordering::SeqCst);
        if !sequence.is_multiple_of(2) {
            return None;
        }
        let result = OwnerTimeline {
            frame: self.frame.load(Ordering::SeqCst),
            epoch: self.epoch.load(Ordering::SeqCst),
            revision: self.revision.load(Ordering::SeqCst),
        };
        (sequence == self.sequence.load(Ordering::SeqCst)).then_some(result)
    }
}

struct Exit<S: ProcessingSample> {
    engine: Engine<S>,
    error: Option<BackendError>,
    clock_trace: ClockTrace,
    _clock: Box<dyn VirtualRealtimeClock>,
    _timeline_resets: Vec<Arc<AtomicU64>>,
}

impl<S: ProcessingSample> PreparedTransport<S> for VirtualPreparedTransport {
    fn attach(
        mut self: Box<Self>,
        mut engine: Engine<S>,
        bindings: &CompiledBindings,
        cancel: CancellationToken,
    ) -> Result<Box<dyn BackendOwner<S>>, Box<AttachFailure<S>>> {
        let validated = (|| {
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            self.gate
                .validate_stop(&self.stop)
                .map_err(|error| BackendError::Failed(error.to_string()))?;
            let processing_rate = engine.config().processing_sr;
            if !processing_rate.is_finite()
                || processing_rate.fract() != 0.0
                || !(8_000.0..=384_000.0).contains(&processing_rate)
                || engine.config().max_block_frames == 0
            {
                return Err(BackendError::Unsupported(
                    "virtual owner requires an integral Engine rate in 8..384kHz".into(),
                ));
            }
            let protected = self
                .sinks
                .iter()
                .map(|&node| bindings.node(node))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| BackendError::Unsupported("virtual sink bindings changed".into()))?;
            if protected.is_empty()
                || protected
                    .iter()
                    .enumerate()
                    .any(|(index, sink)| protected[..index].contains(sink))
                || engine
                    .plan_snapshot()
                    .sink_processors()
                    .any(|sink| !protected.contains(&sink))
            {
                return Err(BackendError::Unsupported(
                    "virtual owner must protect every graph sink".into(),
                ));
            }
            for sink in protected {
                engine
                    .require_sink_reuse(sink)
                    .map_err(|error| BackendError::Unsupported(error.to_string()))?;
            }
            let stop = self.stop.clone();
            cancel.on_cancel(move || {
                let _ = stop.request_stop();
            })?;
            Ok(())
        })();
        if let Err(error) = validated {
            return Err(Box::new(AttachFailure { engine, error }));
        }
        let published = Arc::new(Published::new(&engine));
        // This slot is used only for the one startup handoff. It keeps the
        // Engine on the caller when thread creation fails, with no RT locks.
        let slot = Arc::new(Mutex::new(Some(engine)));
        let worker_slot = slot.clone();
        let worker_published = published.clone();
        let gate = self.gate.clone();
        let stop = self.stop.clone();
        let timeline_resets = self.sessions.timeline_resets();
        let mut clock = self.clock.take().expect("prepared virtual clock");
        let recovery_cap_periods = self.recovery_cap_periods;
        let mut clock_trace = ClockTrace::default();
        let worker = thread::Builder::new()
            .name("moiren-virtual-owner".into())
            .spawn(move || {
                // Match the physical paths' scheduling class: this thread
                // paces Engine renders against audio deadlines. Registration
                // failure is non-fatal.
                let _mmcss = crate::owner::Mmcss::audio();
                let mut engine = worker_slot
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .take()
                    .expect("single Engine handoff");
                let error = match catch_unwind(AssertUnwindSafe(|| {
                    match gate
                        .wait()
                        .map_err(|error| BackendError::Failed(error.to_string()))?
                    {
                        Activation::Cancelled => return Ok(()),
                        Activation::Start => {}
                    }
                    run_with_recovery_cap(
                        &mut engine,
                        &mut *clock,
                        &worker_published,
                        &timeline_resets,
                        &stop,
                        &mut clock_trace,
                        recovery_cap_periods,
                    )
                })) {
                    Ok(result) => result.err(),
                    Err(_) => Some(BackendError::WorkerPanicked),
                };
                worker_published.publish(&engine);
                worker_published
                    .state
                    .store(if error.is_some() { 3 } else { 2 }, Ordering::Release);
                let _ = stop.request_stop();
                // The Engine, its live/retired processors, and errors return to the
                // control joiner; no graph resources are reclaimed by the callback.
                Exit {
                    engine,
                    error,
                    clock_trace,
                    _clock: clock,
                    _timeline_resets: timeline_resets,
                }
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                return Err(Box::new(AttachFailure {
                    engine: slot
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take()
                        .expect("failed spawn retained Engine"),
                    error: BackendError::Failed(error.to_string()),
                }));
            }
        };
        Ok(Box::new(VirtualOwner {
            worker: Some(worker),
            sessions: std::mem::take(&mut self.sessions),
            captures: std::mem::take(&mut self.captures),
            stop: self.stop.clone(),
            gate: self.gate.clone(),
            published,
            cancel,
        }))
    }
}

/// Late wakes repay missed periods from the bounded debt ledger in
/// [`ClockSchedule`]: unrendered periods stay due and this loop renders one
/// block per immediate redue, so the engine timeline keeps pace with the
/// wall clock pacing the native side and the boundary carriers never
/// accumulate a permanent level bias — the defect behind the combined-soak
/// silent-packet failure. Debt beyond the cap is clamped: a single clamped
/// wake is a bounded, counted loss. Clamping on consecutive wakes means the
/// owner cannot keep pace at all (sustained CPU starvation), which is a real
/// stall and escalates to the full reset handshake.
const STALL_CLAMPED_WAKE_STREAK: u32 = 3;

#[cfg(test)]
fn run<S: ProcessingSample>(
    engine: &mut Engine<S>,
    clock: &mut dyn VirtualRealtimeClock,
    published: &Published,
    timeline_resets: &[Arc<AtomicU64>],
    stop: &StopSignal,
    clock_trace: &mut ClockTrace,
) -> Result<(), BackendError> {
    run_with_recovery_cap(
        engine,
        clock,
        published,
        timeline_resets,
        stop,
        clock_trace,
        super::DEBT_CAP_PERIODS,
    )
}

fn run_with_recovery_cap<S: ProcessingSample>(
    engine: &mut Engine<S>,
    clock: &mut dyn VirtualRealtimeClock,
    published: &Published,
    timeline_resets: &[Arc<AtomicU64>],
    stop: &StopSignal,
    clock_trace: &mut ClockTrace,
    recovery_cap_periods: u64,
) -> Result<(), BackendError> {
    let origin = clock.now_ticks()?;
    let frequency = clock.frequency();
    clock_trace.start(origin, frequency);
    let sample_rate = engine.config().processing_sr as u64;
    let mut schedule = ClockSchedule::with_recovery_cap(
        frequency,
        engine.config().max_block_frames,
        sample_rate,
        origin,
        recovery_cap_periods,
    )
    .map_err(|error| BackendError::Failed(error.to_string()))?;
    published.state.store(1, Ordering::Release);
    let mut clamped_streak = 0_u32;
    let mut burst_open = false;
    loop {
        // A persistently late clock may never enter wait_ticks. Poll the same
        // prepared stop event without blocking so overload cannot prevent join.
        if stop_requested(stop)? {
            return Ok(());
        }
        let now = clock.now_ticks()?;
        let remaining = schedule
            .remaining_ticks(now)
            .map_err(|error| BackendError::Failed(error.to_string()))?;
        if remaining > 0 {
            if clock.wait_ticks(remaining)? == ClockWake::Stopped {
                return Ok(());
            }
            continue;
        }
        let Some(tick) = schedule
            .due(now)
            .map_err(|error| BackendError::Failed(error.to_string()))?
        else {
            continue;
        };
        // One trace event per late burst: the redue loop drains the backlog
        // without waiting, so only the burst's first wake is the news.
        if tick.backlog_periods > 1 {
            if !burst_open {
                clock_trace.record(
                    now,
                    engine.timeline(),
                    tick.backlog_periods - 1,
                    GapStage::Wake,
                );
                burst_open = true;
            }
        } else {
            burst_open = false;
        }
        if tick.clamped_periods > 0 {
            clock_trace.record(
                now,
                engine.timeline(),
                tick.clamped_periods,
                GapStage::Clamp,
            );
            clamped_streak += 1;
            if clamped_streak >= STALL_CLAMPED_WAKE_STREAK {
                reset_timeline(timeline_resets);
                clamped_streak = 0;
            }
        } else {
            clamped_streak = 0;
        }
        // Exactly one Engine executes one bounded block per due wake, repaying
        // at most the capped backlog. External microphone client counts are
        // deliberately absent from this decision.
        let rendered = engine.render(tick.frames);
        published.publish(engine);
        rendered.map_err(|error| BackendError::Failed(error.to_string()))?;
    }
}

fn stop_requested(stop: &StopSignal) -> Result<bool, BackendError> {
    // SAFETY: the owned manual-reset event remains alive for this nonblocking
    // query; no handle or pointer escapes the call.
    match unsafe { WaitForSingleObject(HANDLE(stop.event.as_raw_handle()), 0) } {
        value if value == WAIT_OBJECT_0 => Ok(true),
        value if value == WAIT_TIMEOUT => Ok(false),
        _ => Err(BackendError::ResourceUnavailable(format!(
            "virtual owner stop query: {}",
            windows::core::Error::from_thread()
        ))),
    }
}

fn reset_timeline(epochs: &[Arc<AtomicU64>]) {
    for epoch in epochs {
        epoch.fetch_add(1, Ordering::AcqRel);
    }
}

struct VirtualOwner<S: ProcessingSample> {
    worker: Option<JoinHandle<Exit<S>>>,
    sessions: VirtualSessions,
    captures: Vec<CaptureSession>,
    stop: StopSignal,
    gate: ActivationGate,
    published: Arc<Published>,
    cancel: CancellationToken,
}

impl<S: ProcessingSample> BackendOwner<S> for VirtualOwner<S> {
    fn activate(&self) -> Result<(), BackendError> {
        if self.cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        if let Err(error) = self.gate.activate() {
            self.sessions.cancel();
            let _ = self.stop.request_stop();
            return Err(BackendError::Failed(error.to_string()));
        }
        // The graph gate is the admission barrier. Start transport workers
        // only after the Engine has accepted the current prepared graph, so a
        // worker cannot open a driver gate before graph activation is known.
        if let Err(error) = self.sessions.activate() {
            let _ = self.stop.request_stop();
            return Err(error);
        }
        Ok(())
    }
    fn cancel(&self) {
        self.sessions.cancel();
        let _ = self.stop.request_stop();
    }
    fn status(&self) -> OwnerStatus {
        if let OwnerStatus::Failed(error) = self.sessions.status() {
            self.cancel();
            return OwnerStatus::Failed(error);
        }
        if self.captures.iter().any(CaptureSession::is_finished) {
            self.cancel();
            return OwnerStatus::Failed(BackendError::ResourceUnavailable(
                "virtual owner capture stopped".into(),
            ));
        }
        match self.published.state.load(Ordering::Acquire) {
            0 => OwnerStatus::Starting,
            1 if self.sessions.status() == OwnerStatus::Running
                && self.captures.iter().all(CaptureSession::has_started) =>
            {
                OwnerStatus::Running
            }
            1 => OwnerStatus::Starting,
            _ => OwnerStatus::Failed(BackendError::ResourceUnavailable(
                "virtual realtime owner stopped; no fallback clock".into(),
            )),
        }
    }
    fn timeline(&self) -> Option<OwnerTimeline> {
        self.published.snapshot()
    }
    fn stop_and_join(mut self: Box<Self>) -> OwnerReturn<S> {
        self.cancel();
        let exit = self
            .worker
            .take()
            .expect("single owner join")
            .join()
            .expect("virtual worker catches unwind while retaining Engine");
        exit.clock_trace.emit();
        let mut error = exit.error;
        for capture in self.captures.drain(..) {
            if let Err(failure) = capture.join() {
                error.get_or_insert_with(|| BackendError::Failed(failure.to_string()));
            }
        }
        if let Err(failure) = self.sessions.stop_and_join() {
            error.get_or_insert(failure);
        }
        OwnerReturn {
            engine: exit.engine,
            error,
        }
    }
}

impl<S: ProcessingSample> Drop for VirtualOwner<S> {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            // BackendOwner's normal path returns the Engine explicitly. A
            // defensive drop still joins before control-side reclamation.
            if let Ok(exit) = worker.join() {
                exit.clock_trace.emit();
            }
        }
        for capture in self.captures.drain(..) {
            let _ = capture.join();
        }
    }
}

#[cfg(test)]
mod tests;
