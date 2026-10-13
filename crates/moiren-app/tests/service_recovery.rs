use moiren_app::{
    monitor::{
        MonitorFaults, MonitorInitializationApi, MonitorInputBudget, MonitorLatencyConfig,
        MonitorLatencyProfile, MonitorObservation, MonitorPreparedDiagnostics,
        MonitorRecoveryPolicy, MonitorRenderBudget, MonitorStreamDiagnostics,
    },
    service::*,
};
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    events: Mutex<Vec<(usize, &'static str)>>,
    preparations: Mutex<Vec<(SessionSpec, MonitorLatencyConfig, u32)>>,
    faults: AtomicBool,
    counter: AtomicU64,
    fade_ready: AtomicBool,
    join_failure: AtomicBool,
    prepare_failure: AtomicBool,
    delay_start: AtomicBool,
    hide_diagnostics: AtomicBool,
    input_fill: AtomicU64,
    join_blocked: Mutex<bool>,
    wake: Condvar,
}
impl State {
    fn record(&self, id: usize, event: &'static str) {
        self.events.lock().unwrap().push((id, event));
    }
    fn release(&self) {
        *self.join_blocked.lock().unwrap() = false;
        self.wake.notify_all();
    }
}
struct Fake(Arc<State>);
struct Session {
    state: Arc<State>,
    id: usize,
    stopped: AtomicBool,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.state.record(self.id, "drop");
    }
}
impl Fake {
    fn prepare_session(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
        latency: MonitorLatencyConfig,
        fade_in_frames: u32,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        assert!(!cancellation.is_cancelled());
        let mut preparations = self.0.preparations.lock().unwrap();
        let id = preparations.len();
        preparations.push((spec, latency, fade_in_frames));
        self.0.record(id, "prepare");
        if id > 0 && self.0.prepare_failure.load(Ordering::Acquire) {
            return Err(BackendError::new("prepare", "injected replacement failure"));
        }
        Ok(Box::new(Session {
            state: self.0.clone(),
            id,
            stopped: AtomicBool::new(false),
        }))
    }
}
impl BackendAdapter for Fake {
    fn prepare(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        self.prepare_session(spec, cancellation, MonitorLatencyConfig::default(), 0)
    }
    fn prepare_with_monitor_latency(
        &self,
        spec: SessionSpec,
        cancellation: Cancellation,
        _: &mut SignalPreparation,
        latency: MonitorLatencyConfig,
        fade_in_frames: u32,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        self.prepare_session(spec, cancellation, latency, fade_in_frames)
    }
    fn catalog(&self) -> Result<DeviceCatalog, BackendError> {
        Ok(DeviceCatalog::default())
    }
}
impl PreparedSession for Session {
    fn activate(&self) -> Result<(), BackendError> {
        self.state.record(self.id, "activate");
        Ok(())
    }
    fn into_active(self: Box<Self>) -> Box<dyn ActiveSession> {
        self
    }
    fn cancel(&self) -> Result<(), BackendError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
        ActiveSession::join(self)
    }
}
impl ActiveSession for Session {
    fn request_stop(&self) -> Result<(), BackendError> {
        self.state.record(self.id, "stop");
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
    fn is_finished(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    fn poll_started(&self) -> StartedState {
        if self.state.delay_start.load(Ordering::Acquire) {
            StartedState::Starting
        } else {
            StartedState::Running
        }
    }
    fn control(&mut self) -> Option<(&mut ControlPort, &CompiledBindings)> {
        None
    }
    fn monitor_faults(&self) -> Option<MonitorFaults> {
        let tick = self.state.counter.fetch_add(1, Ordering::Relaxed);
        Some(MonitorFaults {
            output_frames: tick * 480,
            underrun_frames: if self.state.faults.load(Ordering::Acquire) {
                tick
            } else {
                0
            },
            ..MonitorFaults::default()
        })
    }
    fn monitor_observation(&self) -> Option<MonitorObservation> {
        if self.state.hide_diagnostics.load(Ordering::Acquire) {
            return None;
        }
        let fallback = self.state.preparations.lock().unwrap()[self.id].1.profile
            == MonitorLatencyProfile::Compatible;
        let stream = MonitorStreamDiagnostics {
            sample_rate: 48_000,
            actual_period_frames: Some(if fallback { 480 } else { 288 }),
            capacity_frames: 1056,
            requested_period_frames: Some(if fallback { 480 } else { 240 }),
            initialization_api: Some(MonitorInitializationApi::Client3),
            period_fallback: None,
        };
        let target = if fallback { 2048 } else { 1024 };
        Some(MonitorObservation {
            prepared: MonitorPreparedDiagnostics {
                input: MonitorStreamDiagnostics {
                    actual_period_frames: None,
                    ..stream
                },
                output: stream,
                input_queue: MonitorInputBudget {
                    capacity_frames: 8192,
                    prime_frames: target,
                    servo_reference_frames: target,
                    low_water_frames: 2,
                    high_water_frames: 8191,
                    modeled_minimum_frames: None,
                },
                render_queue: MonitorRenderBudget {
                    capacity_frames: 1056,
                    target_frames: if fallback { 1056 } else { 480 },
                    deadline_budget_frames: if fallback { None } else { Some(432) },
                },
            },
            processed_frames: self.state.counter.load(Ordering::Relaxed) * 480,
            input_fill_frames: self.state.input_fill.load(Ordering::Acquire),
            correction_ppm: 0.0,
        })
    }
    fn request_monitor_fade_out(&self, _: u32) -> Result<(), BackendError> {
        self.state.record(self.id, "fade");
        Ok(())
    }
    fn monitor_fade_out_submitted(&self) -> bool {
        self.state.fade_ready.load(Ordering::Acquire)
    }
    fn monitor_queue_drain_duration(&self) -> Duration {
        Duration::from_millis(15)
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
        self.state.record(self.id, "join");
        let mut blocked = self.state.join_blocked.lock().unwrap();
        while *blocked {
            blocked = self.state.wake.wait(blocked).unwrap();
        }
        self.state.record(self.id, "reaped");
        if self.id == 0 && self.state.join_failure.load(Ordering::Acquire) {
            Err(BackendError::new("join", "injected cleanup failure"))
        } else {
            Ok(SessionReport::stopped())
        }
    }
}
fn policy() -> MonitorRecoveryPolicy {
    MonitorRecoveryPolicy {
        startup_grace: Duration::from_millis(20),
        window: Duration::from_millis(20),
        max_observation_gap: Duration::from_secs(1),
        consecutive_windows: 2,
        fade_frames: 240,
        fade_timeout: Duration::from_millis(200),
    }
}
fn spec() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Physical {
            endpoint_id: "fake-input".into(),
        },
        output_endpoint_id: "fake-output".into(),
        limit: RunLimit::UntilStopped,
        gain: 0.05,
        pan: 0.0,
        max_block_frames: 256,
    }
}
fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn runtime(state: &Arc<State>, recovery: Option<MonitorRecoveryPolicy>) -> ServiceRuntime {
    ServiceRuntime::spawn_with_driver(
        BackendDriver::new(Arc::new(Fake(state.clone())))
            .with_monitor_latency(MonitorLatencyConfig::balanced(), recovery)
            .unwrap(),
    )
    .unwrap()
}
fn shutdown(runtime: ServiceRuntime) {
    runtime.handle().request_exit();
    runtime.join().unwrap();
}

#[test]
fn default_physical_driver_publishes_native_diagnostics_without_a_profile_override() {
    let state = Arc::new(State::default());
    let runtime =
        ServiceRuntime::spawn_with_driver(BackendDriver::new(Arc::new(Fake(state.clone()))))
            .unwrap();
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.monitor_diagnostics.is_some())
    });
    let running = handle.try_snapshot().unwrap();
    assert!(running.monitor_latency.is_none());
    assert_eq!(
        running
            .monitor_diagnostics
            .unwrap()
            .observation
            .prepared
            .input_queue
            .prime_frames,
        2048
    );
    shutdown(runtime);
}

#[test]
fn non_physical_sources_cannot_publish_physical_monitor_diagnostics() {
    for source in [
        InputSelection::Tone {
            frequency_hz: 440.0,
        },
        InputSelection::Process {
            pid: 123,
            creation_time_100ns: 456,
        },
    ] {
        let state = Arc::new(State::default());
        let runtime = runtime(&state, None);
        let handle = runtime.handle();
        let mut session = spec();
        session.source = source;
        handle.try_request(AppRequest::Start(session)).unwrap();
        wait(|| {
            handle
                .try_snapshot()
                .is_some_and(|snapshot| snapshot.phase == SessionPhase::Running)
        });
        let running = handle.try_snapshot().unwrap();
        assert!(running.monitor_latency.is_none());
        assert!(running.monitor_diagnostics.is_none());
        shutdown(runtime);
    }
}

#[test]
fn native_diagnostics_remain_unpublished_until_both_owners_are_running() {
    let state = Arc::new(State::default());
    state.delay_start.store(true, Ordering::Release);
    let runtime = runtime(&state, None);
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| state.events.lock().unwrap().contains(&(0, "activate")));
    let starting = handle.try_snapshot().unwrap();
    assert_eq!(starting.phase, SessionPhase::Starting);
    assert!(starting.monitor_diagnostics.is_none());
    state.delay_start.store(false, Ordering::Release);
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.monitor_diagnostics.is_some())
    });
    let running = handle.try_snapshot().unwrap();
    assert_eq!(running.phase, SessionPhase::Running);
    let diagnostics = running.monitor_diagnostics.unwrap();
    assert_eq!(diagnostics.generation, running.generation);
    let prepared = diagnostics.observation.prepared;
    assert_eq!(prepared.output.requested_period_frames, Some(240));
    assert_eq!(prepared.output.actual_period_frames, Some(288));
    assert_eq!(prepared.input.actual_period_frames, None);
    shutdown(runtime);
}

#[test]
fn running_diagnostics_follow_fill_and_clear_before_cleanup_finishes() {
    struct Unblock(Arc<State>);
    impl Drop for Unblock {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    let state = Arc::new(State::default());
    state.input_fill.store(512, Ordering::Release);
    let runtime = runtime(&state, None);
    let _unblock = Unblock(state.clone());
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.monitor_diagnostics.is_some())
    });
    let earlier = handle.try_snapshot().unwrap().monitor_diagnostics.unwrap();
    assert_eq!(earlier.observation.input_fill_frames, 512);
    state.input_fill.store(1536, Ordering::Release);
    wait(|| {
        handle.try_snapshot().is_some_and(|snapshot| {
            snapshot
                .monitor_diagnostics
                .is_some_and(|diagnostics| diagnostics.observation.input_fill_frames == 1536)
        })
    });
    assert_eq!(earlier.observation.input_fill_frames, 512);
    *state.join_blocked.lock().unwrap() = true;
    handle.try_request(AppRequest::Stop).unwrap();
    wait(|| state.events.lock().unwrap().contains(&(0, "join")));
    let stopping = handle.try_snapshot().unwrap();
    assert_eq!(stopping.phase, SessionPhase::Stopping);
    assert_eq!(stopping.generation, SessionGeneration(2));
    assert!(stopping.monitor_diagnostics.is_none());
    state.release();
    shutdown(runtime);
}

#[test]
fn a_selected_policy_does_not_invent_missing_native_diagnostics() {
    let state = Arc::new(State::default());
    state.hide_diagnostics.store(true, Ordering::Release);
    let runtime = runtime(&state, None);
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let running = handle.try_snapshot().unwrap();
    assert_eq!(
        running.monitor_latency.as_ref().unwrap().selected,
        MonitorLatencyConfig::balanced()
    );
    assert!(running.monitor_diagnostics.is_none());
    shutdown(runtime);
}

#[test]
fn fallback_waits_for_fade_and_reaping_then_prepares_compatible_once() {
    let state = Arc::new(State::default());
    state.faults.store(true, Ordering::Release);
    *state.join_blocked.lock().unwrap() = true;
    let runtime = runtime(&state, Some(policy()));
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| state.events.lock().unwrap().contains(&(0, "fade")));
    assert!(!state.events.lock().unwrap().contains(&(0, "stop")));
    handle
        .try_request(AppRequest::SetGainPan {
            gain: 0.3,
            pan: -0.2,
        })
        .unwrap();
    state.fade_ready.store(true, Ordering::Release);
    wait(|| state.events.lock().unwrap().contains(&(0, "join")));
    assert_eq!(state.preparations.lock().unwrap().len(), 1);
    assert!(handle.try_snapshot().unwrap().monitor_diagnostics.is_none());
    state.release();
    wait(|| {
        handle.try_snapshot().is_some_and(|snapshot| {
            snapshot.generation == SessionGeneration(2) && snapshot.phase == SessionPhase::Running
        })
    });
    let snapshot = handle.try_snapshot().unwrap();
    let diagnostics = snapshot.monitor_diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics.generation, SessionGeneration(2));
    assert_eq!(
        diagnostics.observation.prepared.input_queue.prime_frames,
        2048
    );
    assert_eq!(
        diagnostics.observation.prepared.render_queue.target_frames,
        1056
    );
    assert_eq!(
        diagnostics.observation.prepared.output.actual_period_frames,
        Some(480)
    );
    let status = snapshot.monitor_latency.as_ref().unwrap();
    assert_eq!(status.requested.profile, MonitorLatencyProfile::Balanced);
    assert_eq!(status.selected, MonitorLatencyConfig::compatible_fallback());
    assert_eq!(
        status.fallback.as_ref().unwrap().from_generation,
        SessionGeneration(1)
    );
    assert!(!status.fallback.as_ref().unwrap().fade_timed_out);
    let preparations = state.preparations.lock().unwrap();
    assert_eq!(preparations.len(), 2);
    assert_eq!(preparations[1].0.gain, 0.3);
    assert_eq!(preparations[1].0.pan, -0.2);
    assert_eq!(
        preparations[1].1,
        MonitorLatencyConfig::compatible_fallback()
    );
    assert_eq!(preparations[1].2, 240);
    drop(preparations);
    let events = state.events.lock().unwrap();
    assert!(
        events
            .iter()
            .position(|event| *event == (0, "drop"))
            .unwrap()
            < events
                .iter()
                .position(|event| *event == (1, "prepare"))
                .unwrap()
    );
    drop(events);
    shutdown(runtime);
    assert_eq!(state.preparations.lock().unwrap().len(), 2);
}

#[test]
fn stop_during_fade_revokes_replacement() {
    let state = Arc::new(State::default());
    state.faults.store(true, Ordering::Release);
    let runtime = runtime(&state, Some(policy()));
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| state.events.lock().unwrap().contains(&(0, "fade")));
    handle.try_request(AppRequest::Stop).unwrap();
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.phase == SessionPhase::Idle)
    });
    shutdown(runtime);
    assert_eq!(state.preparations.lock().unwrap().len(), 1);
}

#[test]
fn stalled_fade_times_out_without_hanging_the_control_service() {
    let state = Arc::new(State::default());
    state.faults.store(true, Ordering::Release);
    let runtime = runtime(&state, Some(policy()));
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| {
        handle.try_snapshot().is_some_and(|snapshot| {
            snapshot.generation == SessionGeneration(2) && snapshot.phase == SessionPhase::Running
        })
    });
    assert!(
        handle
            .try_snapshot()
            .unwrap()
            .monitor_latency
            .as_ref()
            .unwrap()
            .fallback
            .as_ref()
            .unwrap()
            .fade_timed_out
    );
    shutdown(runtime);
}

#[test]
fn cleanup_and_replacement_prepare_failures_are_terminal_without_retry() {
    for cleanup_failure in [false, true] {
        let state = Arc::new(State::default());
        state.faults.store(true, Ordering::Release);
        state.fade_ready.store(true, Ordering::Release);
        state.join_failure.store(cleanup_failure, Ordering::Release);
        state
            .prepare_failure
            .store(!cleanup_failure, Ordering::Release);
        let runtime = runtime(&state, Some(policy()));
        let handle = runtime.handle();
        handle.try_request(AppRequest::Start(spec())).unwrap();
        wait(|| {
            handle
                .try_snapshot()
                .is_some_and(|snapshot| snapshot.phase == SessionPhase::Failed)
        });
        assert!(handle.try_snapshot().unwrap().error.is_some());
        assert!(handle.try_snapshot().unwrap().monitor_diagnostics.is_none());
        shutdown(runtime);
        assert_eq!(
            state.preparations.lock().unwrap().len(),
            if cleanup_failure { 1 } else { 2 }
        );
    }
}

#[test]
fn recovery_is_opt_in_even_when_faults_continue() {
    let state = Arc::new(State::default());
    state.faults.store(true, Ordering::Release);
    let runtime = runtime(&state, None);
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    thread::sleep(Duration::from_millis(150));
    assert_eq!(state.preparations.lock().unwrap().len(), 1);
    assert!(!state.events.lock().unwrap().contains(&(0, "fade")));
    shutdown(runtime);
}

#[test]
fn newer_start_during_cleanup_supersedes_fallback_and_restores_requested_policy() {
    let state = Arc::new(State::default());
    state.faults.store(true, Ordering::Release);
    state.fade_ready.store(true, Ordering::Release);
    *state.join_blocked.lock().unwrap() = true;
    let runtime = runtime(&state, Some(policy()));
    let handle = runtime.handle();
    handle.try_request(AppRequest::Start(spec())).unwrap();
    wait(|| state.events.lock().unwrap().contains(&(0, "join")));
    state.faults.store(false, Ordering::Release);
    state.join_failure.store(true, Ordering::Release);
    let mut replacement = spec();
    replacement.gain = 0.4;
    handle.try_request(AppRequest::Start(replacement)).unwrap();
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.generation == SessionGeneration(3))
    });
    assert!(handle.try_snapshot().unwrap().monitor_diagnostics.is_none());
    state.release();
    wait(|| {
        handle
            .try_snapshot()
            .is_some_and(|snapshot| snapshot.phase == SessionPhase::Running)
    });
    let status = handle.try_snapshot().unwrap();
    assert_eq!(
        status.monitor_latency.as_ref().unwrap().selected,
        MonitorLatencyConfig::balanced()
    );
    assert!(status.monitor_latency.as_ref().unwrap().fallback.is_none());
    let diagnostics = status.monitor_diagnostics.unwrap();
    assert_eq!(diagnostics.generation, SessionGeneration(3));
    assert_eq!(
        diagnostics.observation.prepared.input_queue.prime_frames,
        1024
    );
    assert_eq!(
        diagnostics.observation.prepared.render_queue.target_frames,
        480
    );
    let preparations = state.preparations.lock().unwrap();
    assert_eq!(preparations.len(), 2);
    assert_eq!(preparations[1].0.gain, 0.4);
    assert_eq!(preparations[1].2, 0);
    drop(preparations);
    shutdown(runtime);
}

#[test]
fn exit_during_fade_revokes_replacement() {
    let state = Arc::new(State::default());
    state.faults.store(true, Ordering::Release);
    let runtime = runtime(&state, Some(policy()));
    let handle = runtime.handle();
    runtime
        .handle()
        .try_request(AppRequest::Start(spec()))
        .unwrap();
    wait(|| state.events.lock().unwrap().contains(&(0, "fade")));
    shutdown(runtime);
    let exited = handle.try_snapshot().unwrap();
    assert_eq!(exited.phase, SessionPhase::Exited);
    assert!(exited.monitor_diagnostics.is_none());
    assert_eq!(state.preparations.lock().unwrap().len(), 1);
}
