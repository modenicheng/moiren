use moiren_app::service::*;
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    catalog_failure: AtomicBool,
    join_error: AtomicBool,
    finish_during_poll: AtomicBool,
    next_session: AtomicUsize,
    session_events: Mutex<Vec<(usize, &'static str, thread::ThreadId)>>,
    saw_bounded_reap: AtomicBool,
    mismatch_timeline: AtomicBool,
    completed: AtomicBool,
    target_exited: AtomicBool,
    controls: AtomicBool,
    prefill: AtomicBool,
    pump: AtomicBool,
    blocked: Mutex<bool>,
    wake: Condvar,
    preparing: AtomicBool,
    started: AtomicBool,
    failed: AtomicBool,
    activation_error: AtomicBool,
    events: Mutex<Vec<(&'static str, thread::ThreadId)>>,
}
impl State {
    fn event(&self, event: &'static str) {
        self.events
            .lock()
            .unwrap()
            .push((event, thread::current().id()));
    }
    fn unblock(&self) {
        *self.blocked.lock().unwrap() = false;
        self.wake.notify_all();
    }
}
struct Fake(Arc<State>);
struct ToneControl {
    renderer: moiren_windows_audio::render::DemandRenderer,
    control: ControlPort,
    bindings: CompiledBindings,
    gain: moiren_core::graph::NodeId,
    pan: moiren_core::graph::NodeId,
}
struct Session(Arc<State>, Option<ToneControl>, usize);
impl Session {
    fn record(&self, event: &'static str) {
        self.0.event(event);
        self.0
            .session_events
            .lock()
            .unwrap()
            .push((self.2, event, thread::current().id()));
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.record("drop");
    }
}
impl BackendAdapter for Fake {
    fn prepare(
        &self,
        _: SessionSpec,
        _: Cancellation,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        self.0.preparing.store(true, Ordering::Release);
        self.0.event("prepare");
        let mut blocked = self.0.blocked.lock().unwrap();
        while *blocked {
            blocked = self.0.wake.wait(blocked).unwrap();
        }
        let tone = if self.0.controls.load(Ordering::Acquire) {
            let mut tone =
                moiren_app::tone::prepare_tone(moiren_app::tone::ToneConfig::default()).unwrap();
            if self.0.prefill.load(Ordering::Acquire) {
                for id in 10000..10063 {
                    use moiren_core::protocol::*;
                    assert_eq!(
                        tone.compiled
                            .control
                            .submit(
                                ParameterRequest {
                                    request_id: id,
                                    plan_revision: 1,
                                    timeline_epoch: 1,
                                    target: ParameterKey {
                                        processor: tone
                                            .compiled
                                            .bindings
                                            .node(tone.gain_node)
                                            .unwrap(),
                                        parameter: moiren_engine::processor::Gain::LEVEL
                                    },
                                    at: ApplyAt::NextBlock,
                                    value: ParamValue::Float(0.05),
                                    ramp_frames: 0
                                },
                                0
                            )
                            .code,
                        ReplyCode::Accepted
                    );
                }
            }
            Some(ToneControl {
                renderer: moiren_windows_audio::render::DemandRenderer::new(
                    tone.compiled.engine,
                    tone.output,
                )
                .unwrap(),
                control: tone.compiled.control,
                bindings: tone.compiled.bindings,
                gain: tone.gain_node,
                pan: tone.pan_node,
            })
        } else {
            None
        };
        Ok(Box::new(Session(
            self.0.clone(),
            tone,
            self.0.next_session.fetch_add(1, Ordering::AcqRel),
        )))
    }
    fn catalog(&self) -> Result<DeviceCatalog, BackendError> {
        if self.0.catalog_failure.load(Ordering::Acquire) {
            return Err(BackendError::new("catalog", "fake enumeration failure"));
        }
        Ok(DeviceCatalog {
            inputs: vec![DeviceRow {
                endpoint_id: "input-id".into(),
                name: "input".into(),
            }],
            outputs: vec![DeviceRow {
                endpoint_id: "output-id".into(),
                name: "output".into(),
            }],
            processes: vec![ProcessRow {
                pid: 123,
                creation_time_100ns: 456,
                name: "process".into(),
            }],
            default_input_endpoint_id: Some("input-id".into()),
            default_output_endpoint_id: Some("output-id".into()),
            ..DeviceCatalog::default()
        })
    }
}
impl PreparedSession for Session {
    fn activate(&self) -> Result<(), BackendError> {
        self.record("activate");
        if self.0.activation_error.load(Ordering::Acquire) {
            Err(BackendError::new("activate", "fake failure"))
        } else {
            Ok(())
        }
    }
    fn into_active(self: Box<Self>) -> Box<dyn ActiveSession> {
        self
    }
    fn cancel(&self) -> Result<(), BackendError> {
        self.record("cancel");
        Ok(())
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
        self.record("join");
        Ok(SessionReport::stopped())
    }
}
impl ActiveSession for Session {
    fn request_stop(&self) -> Result<(), BackendError> {
        self.record("stop");
        Ok(())
    }
    fn is_finished(&self) -> bool {
        self.0.failed.load(Ordering::Acquire)
            || self.0.completed.load(Ordering::Acquire)
            || self.0.target_exited.load(Ordering::Acquire)
    }
    fn poll_started(&self) -> StartedState {
        if self.0.finish_during_poll.load(Ordering::Acquire) {
            self.0.completed.store(true, Ordering::Release);
        }
        if self.is_finished() {
            StartedState::Failed
        } else if self.0.started.load(Ordering::Acquire) {
            StartedState::Running
        } else {
            StartedState::Starting
        }
    }
    fn control(&mut self) -> Option<(&mut ControlPort, &CompiledBindings)> {
        let tone = self.1.as_mut()?;
        if self.0.pump.load(Ordering::Acquire) {
            tone.renderer.render_interleaved(&mut [0.; 128]).unwrap();
        }
        Some((&mut tone.control, &tone.bindings))
    }
    fn timeline(&self) -> Option<moiren_windows_audio::render::TimelineSnapshot> {
        self.1.as_ref().map(|t| {
            let mut snapshot = t.renderer.timeline_observer().snapshot();
            if self.0.mismatch_timeline.load(Ordering::Acquire) {
                snapshot.epoch += 1;
            }
            snapshot
        })
    }
    fn gain_pan_nodes(&self) -> Option<(moiren_core::graph::NodeId, moiren_core::graph::NodeId)> {
        self.1.as_ref().map(|t| (t.gain, t.pan))
    }
    fn join(mut self: Box<Self>) -> Result<SessionReport, BackendError> {
        self.record("join");
        let mut report = SessionReport::stopped();
        if self.0.failed.load(Ordering::Acquire) {
            report.status = SessionEnd::Failed;
        }
        if self.0.completed.load(Ordering::Acquire) {
            report.status = SessionEnd::Completed;
        }
        if self.0.target_exited.load(Ordering::Acquire) {
            report.status = SessionEnd::TargetExited;
        }
        if let Some(tone) = self.1.as_mut() {
            loop {
                while let Some(reply) = tone.control.poll_applied() {
                    if reply.request_id < 10000 {
                        report.control_replies.push(reply);
                    }
                }
                if tone.renderer.retire_controls() == 0 {
                    break;
                }
            }
            while let Some(reply) = tone.control.poll_applied() {
                if reply.request_id < 10000 {
                    report.control_replies.push(reply);
                }
            }
        }
        if self.0.join_error.load(Ordering::Acquire) {
            Err(BackendError::new("join", "irrecoverable output owner"))
        } else {
            Ok(report)
        }
    }
}
fn spec() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone { frequency_hz: 440. },
        output_endpoint_id: "fake-output".into(),
        limit: RunLimit::UntilStopped,
        gain: 0.05,
        pan: 0.,
        max_block_frames: 256,
    }
}
fn wait_until(mut check: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(Instant::now() < until, "timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn phase(handle: &AppHandle, expected: SessionPhase) {
    wait_until(|| handle.try_snapshot().is_some_and(|s| s.phase == expected));
}
struct HarnessDriver {
    inner: BackendDriver,
    state: Arc<State>,
    saturation: Option<Arc<Saturation>>,
    filled: bool,
}
#[derive(Default)]
struct Saturation {
    blocked: Mutex<bool>,
    wake: Condvar,
}
impl Saturation {
    fn unblock(&self) {
        *self.blocked.lock().unwrap() = false;
        self.wake.notify_all();
    }
}
impl ServiceDriver for HarnessDriver {
    fn request(&mut self, c: &mut ServiceContext, p: &mut JobPool, r: AppRequest) {
        self.inner.request(c, p, r);
    }
    fn begin_exit(&mut self, c: &mut ServiceContext, p: &mut JobPool) {
        self.inner.begin_exit(c, p);
    }
    fn is_drained(&self) -> bool {
        self.inner.is_drained()
    }
    fn poll(&mut self, c: &mut ServiceContext, p: &mut JobPool, b: RoundBudget) {
        let before = c.control.accepted_pending;
        self.inner.poll(c, p, b);
        if before > c.control.accepted_pending {
            assert!(before - c.control.accepted_pending <= 32);
        }
        if before == 64 && c.control.accepted_pending == 32 {
            self.state.saw_bounded_reap.store(true, Ordering::Release);
            assert!(c.core.owner_generation().is_some());
            assert_ne!(c.core.phase(), SessionPhase::Exited);
        }
        if !self.filled
            && self.state.preparing.load(Ordering::Acquire)
            && let Some(saturation) = self.saturation.as_ref()
        {
            let gate = saturation.clone();
            p.try_submit_owned(
                JobPriority::Cleanup,
                Box::new(move || {
                    let mut blocked = gate.blocked.lock().unwrap();
                    while *blocked {
                        blocked = gate.wake.wait(blocked).unwrap();
                    }
                }),
            )
            .unwrap_or_else(|_| panic!("free cleanup lane"));
            for _ in 0..15 {
                c.try_defer(JobPriority::Cleanup, Box::new(|| {}))
                    .unwrap_or_else(|_| panic!("reserved remaining slots"));
            }
            self.filled = true;
            self.state.event("saturated");
        }
    }
}
fn runtime(state: &Arc<State>) -> ServiceRuntime {
    ServiceRuntime::spawn_with_driver(HarnessDriver {
        inner: BackendDriver::new(Arc::new(Fake(state.clone()))),
        state: state.clone(),
        saturation: None,
        filled: false,
    })
    .unwrap()
}
fn exit(runtime: ServiceRuntime) {
    let h = runtime.handle();
    h.request_exit();
    phase(&h, SessionPhase::Exited);
    runtime.join().unwrap();
}

#[test]
fn stale_prepared_result_never_activates_and_drops_on_cleanup_worker() {
    let state = Arc::new(State::default());
    *state.blocked.lock().unwrap() = true;
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    wait_until(|| state.preparing.load(Ordering::Acquire));
    h.try_request(AppRequest::Stop).unwrap();
    phase(&h, SessionPhase::Idle);
    state.unblock();
    wait_until(|| {
        state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == "drop")
    });
    exit(rt);
    let events = state.events.lock().unwrap();
    assert!(!events.iter().any(|(e, _)| *e == "activate"));
    let prepare = events.iter().find(|(e, _)| *e == "prepare").unwrap().1;
    let join = events.iter().find(|(e, _)| *e == "join").unwrap().1;
    let drop = events.iter().find(|(e, _)| *e == "drop").unwrap().1;
    assert_eq!(join, drop);
    assert_ne!(prepare, drop);
    assert_ne!(thread::current().id(), drop);
}
#[test]
fn running_requires_owner_acknowledgement_and_owner_failure_stops_peer() {
    let state = Arc::new(State::default());
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    wait_until(|| {
        state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == "activate")
    });
    phase(&h, SessionPhase::Starting);
    state.started.store(true, Ordering::Release);
    phase(&h, SessionPhase::Running);
    state.failed.store(true, Ordering::Release);
    phase(&h, SessionPhase::Failed);
    exit(rt);
    assert!(
        state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == "stop")
    );
}
#[test]
fn activation_error_cleans_prepared_ownership_on_worker() {
    let state = Arc::new(State::default());
    state.activation_error.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Failed);
    exit(rt);
    let events = state.events.lock().unwrap();
    let activate = events.iter().find(|(e, _)| *e == "activate").unwrap().1;
    let drop = events.iter().find(|(e, _)| *e == "drop").unwrap().1;
    assert_ne!(activate, drop);
    assert!(events.iter().any(|(e, _)| *e == "cancel"));
}
#[test]
fn exit_waits_for_blocked_preparation_and_disposal() {
    let state = Arc::new(State::default());
    *state.blocked.lock().unwrap() = true;
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    wait_until(|| state.preparing.load(Ordering::Acquire));
    h.request_exit();
    phase(&h, SessionPhase::Exiting);
    state.unblock();
    phase(&h, SessionPhase::Exited);
    rt.join().unwrap();
    let events = state.events.lock().unwrap();
    assert!(!events.iter().any(|(e, _)| *e == "activate"));
    assert!(events.iter().any(|(e, _)| *e == "drop"));
}

#[test]
fn gain_accepted_pan_full_retains_only_unsent_pan_and_terminalizes_on_exit() {
    use moiren_core::protocol::ReplyCode;
    let state = Arc::new(State::default());
    state.controls.store(true, Ordering::Release);
    state.prefill.store(true, Ordering::Release);
    state.started.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    h.try_request(AppRequest::SetGainPan {
        gain: 0.2,
        pan: 0.3,
    })
    .unwrap();
    wait_until(|| {
        h.try_snapshot().is_some_and(|s| {
            s.control.accepted_pending == 1 && s.control.pending_gain_pan == Some((0.2, 0.3))
        })
    });
    state.pump.store(true, Ordering::Release);
    wait_until(|| {
        h.try_snapshot().is_some_and(|s| {
            s.control.pending_gain_pan.is_none()
                && s.control.last_result.is_some_and(|r| {
                    r.request_id == 2
                        && matches!(r.code, ReplyCode::Applied | ReplyCode::AppliedLate)
                })
        })
    });
    exit(rt);
}
#[test]
fn ledger_is_bounded_and_stop_completes_every_accepted_request() {
    let state = Arc::new(State::default());
    state.controls.store(true, Ordering::Release);
    state.started.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    for n in 1..=32 {
        h.try_request(AppRequest::SetGainPan {
            gain: n as f64 / 100.,
            pan: 0.,
        })
        .unwrap();
        wait_until(|| {
            h.try_snapshot()
                .is_some_and(|s| s.control.accepted_pending == n * 2)
        });
    }
    h.try_request(AppRequest::SetGainPan {
        gain: 0.6,
        pan: 0.5,
    })
    .unwrap();
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.control.pending_gain_pan == Some((0.6, 0.5)))
    });
    assert_eq!(h.try_snapshot().unwrap().control.accepted_pending, 64);
    h.request_exit();
    phase(&h, SessionPhase::Exited);
    let snapshot = h.try_snapshot().unwrap();
    assert_eq!(snapshot.control.accepted_pending, 0);
    assert!(snapshot.control.pending_gain_pan.is_none());
    rt.join().unwrap();
    assert!(state.saw_bounded_reap.load(Ordering::Acquire));
}

#[test]
fn finite_completion_and_target_exit_consume_intent_without_restarting() {
    for target in [false, true] {
        let state = Arc::new(State::default());
        state.started.store(true, Ordering::Release);
        let rt = runtime(&state);
        let h = rt.handle();
        h.try_request(AppRequest::Start(spec())).unwrap();
        phase(&h, SessionPhase::Running);
        if target {
            state.target_exited.store(true, Ordering::Release);
        } else {
            state.completed.store(true, Ordering::Release);
        }
        phase(&h, SessionPhase::Idle);
        let snapshot = h.try_snapshot().unwrap();
        assert_eq!(snapshot.generation, SessionGeneration(1));
        assert!(snapshot.desired.is_none());
        exit(rt);
        assert_eq!(
            state
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|(e, _)| *e == "prepare")
                .count(),
            1
        );
    }
}
#[test]
fn catalog_keeps_endpoint_defaults_and_exact_process_identity() {
    let state = Arc::new(State::default());
    let rt = runtime(&state);
    let h = rt.handle();
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.catalog.default_output_endpoint_id.as_deref() == Some("output-id"))
    });
    let snapshot = h.try_snapshot().unwrap();
    assert_eq!(
        snapshot.catalog.default_input_endpoint_id.as_deref(),
        Some("input-id")
    );
    assert_eq!(snapshot.catalog.processes[0].pid, 123);
    assert_eq!(snapshot.catalog.processes[0].creation_time_100ns, 456);
    exit(rt);
}
#[cfg(windows)]
#[test]
fn native_adapter_rejects_invalid_selection_before_spawning_owners() {
    let backend = WindowsBackend::default();
    let mut invalid = spec();
    invalid.output_endpoint_id = String::new();
    assert!(
        backend
            .prepare(invalid, Cancellation::new().unwrap())
            .is_err()
    );
}

#[test]
fn cleanup_credit_survives_full_deferred_storage_and_busy_cleanup_lane() {
    let state = Arc::new(State::default());
    *state.blocked.lock().unwrap() = true;
    let saturation = Arc::new(Saturation::default());
    *saturation.blocked.lock().unwrap() = true;
    let rt = ServiceRuntime::spawn_with_driver(HarnessDriver {
        inner: BackendDriver::new(Arc::new(Fake(state.clone()))),
        state: state.clone(),
        saturation: Some(saturation.clone()),
        filled: false,
    })
    .unwrap();
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    wait_until(|| {
        state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == "saturated")
    });
    h.try_request(AppRequest::Stop).unwrap();
    phase(&h, SessionPhase::Idle);
    state.unblock();
    h.request_exit();
    phase(&h, SessionPhase::Exiting);
    assert!(
        !state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == "drop")
    );
    saturation.unblock();
    phase(&h, SessionPhase::Exited);
    rt.join().unwrap();
    let events = state.session_events.lock().unwrap();
    let joins: Vec<_> = events.iter().filter(|(_, e, _)| *e == "join").collect();
    let drops: Vec<_> = events.iter().filter(|(_, e, _)| *e == "drop").collect();
    assert_eq!(joins.len(), 1);
    assert_eq!(drops.len(), 1);
    assert_eq!(joins[0].0, drops[0].0);
    assert_eq!(joins[0].2, drops[0].2);
}
#[test]
fn frequency_replacement_reaps_old_identity_before_preparing_new_owner() {
    let state = Arc::new(State::default());
    state.started.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    let mut replacement = spec();
    replacement.source = InputSelection::Tone { frequency_hz: 880. };
    h.try_request(AppRequest::Start(replacement)).unwrap();
    wait_until(|| {
        h.try_snapshot().is_some_and(|s| {
            s.generation == SessionGeneration(2) && s.phase == SessionPhase::Running
        })
    });
    exit(rt);
    let events = state.session_events.lock().unwrap();
    let old_drop = events
        .iter()
        .position(|(id, e, _)| *id == 0 && *e == "drop")
        .unwrap();
    let new_activate = events
        .iter()
        .position(|(id, e, _)| *id == 1 && *e == "activate")
        .unwrap();
    assert!(old_drop < new_activate);
    for id in [0, 1] {
        let join = events
            .iter()
            .find(|(i, e, _)| *i == id && *e == "join")
            .unwrap()
            .2;
        let drop = events
            .iter()
            .find(|(i, e, _)| *i == id && *e == "drop")
            .unwrap()
            .2;
        assert_eq!(join, drop);
    }
}
#[test]
fn parameters_wait_for_epoch_matched_session_timeline() {
    let state = Arc::new(State::default());
    state.controls.store(true, Ordering::Release);
    state.started.store(true, Ordering::Release);
    state.mismatch_timeline.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    h.try_request(AppRequest::SetGainPan {
        gain: 0.2,
        pan: 0.3,
    })
    .unwrap();
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.control.pending_gain_pan == Some((0.2, 0.3)))
    });
    assert_eq!(h.try_snapshot().unwrap().control.accepted_pending, 0);
    h.try_request(AppRequest::SetGainPan {
        gain: 0.4,
        pan: -0.6,
    })
    .unwrap();
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.control.pending_gain_pan == Some((0.4, -0.6)))
    });
    assert_eq!(h.try_snapshot().unwrap().control.accepted_pending, 0);
    state.mismatch_timeline.store(false, Ordering::Release);
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.control.accepted_pending == 2)
    });
    assert_eq!(
        h.try_snapshot()
            .unwrap()
            .control
            .last_result
            .unwrap()
            .request_id,
        2
    );
    exit(rt);
}

#[test]
fn native_completion_racing_start_poll_uses_join_report() {
    let state = Arc::new(State::default());
    state.started.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    state.finish_during_poll.store(true, Ordering::Release);
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| matches!(s.phase, SessionPhase::Idle | SessionPhase::Failed))
    });
    let observed = h.try_snapshot().unwrap().phase;
    exit(rt);
    assert_eq!(observed, SessionPhase::Idle);
}

#[test]
fn unrecoverable_join_terminalizes_accepted_ledger_only_after_worker_join() {
    use moiren_core::protocol::ReplyCode;
    let state = Arc::new(State::default());
    state.controls.store(true, Ordering::Release);
    state.started.store(true, Ordering::Release);
    state.join_error.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    h.try_request(AppRequest::SetGainPan {
        gain: 0.2,
        pan: 0.3,
    })
    .unwrap();
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.control.accepted_pending == 2)
    });
    h.try_request(AppRequest::Stop).unwrap();
    // An explicit stop advances the current generation, so its older owner's
    // cleanup failure cannot overwrite the newer desired Idle state.
    phase(&h, SessionPhase::Idle);
    let snapshot = h.try_snapshot().unwrap();
    assert_eq!(snapshot.control.accepted_pending, 0);
    assert_eq!(
        snapshot.control.last_result.unwrap().code,
        ReplyCode::StaleRevision
    );
    assert!(
        state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == "join")
    );
    exit(rt);
}

#[test]
fn finite_completion_clears_only_current_unaccepted_slider_intent() {
    let state = Arc::new(State::default());
    state.controls.store(true, Ordering::Release);
    state.started.store(true, Ordering::Release);
    state.mismatch_timeline.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    let mut finite = spec();
    finite.limit = RunLimit::Seconds(1);
    h.try_request(AppRequest::Start(finite)).unwrap();
    phase(&h, SessionPhase::Running);
    h.try_request(AppRequest::SetGainPan {
        gain: 0.2,
        pan: 0.3,
    })
    .unwrap();
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.control.pending_gain_pan == Some((0.2, 0.3)))
    });
    assert_eq!(h.try_snapshot().unwrap().control.accepted_pending, 0);
    state.completed.store(true, Ordering::Release);
    phase(&h, SessionPhase::Idle);
    let pending = h.try_snapshot().unwrap().control.pending_gain_pan;
    exit(rt);
    assert_eq!(pending, None);
}

#[test]
fn older_completion_preserves_new_generation_unaccepted_slider_intent() {
    let state = Arc::new(State::default());
    state.controls.store(true, Ordering::Release);
    state.started.store(true, Ordering::Release);
    state.mismatch_timeline.store(true, Ordering::Release);
    let saturation = Arc::new(Saturation::default());
    *saturation.blocked.lock().unwrap() = true;
    let rt = ServiceRuntime::spawn_with_driver(HarnessDriver {
        inner: BackendDriver::new(Arc::new(Fake(state.clone()))),
        state: state.clone(),
        saturation: Some(saturation.clone()),
        filled: false,
    })
    .unwrap();
    let h = rt.handle();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    wait_until(|| {
        state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(event, _)| *event == "saturated")
    });
    state.completed.store(true, Ordering::Release);
    phase(&h, SessionPhase::Stopping);
    h.try_request(AppRequest::Start(spec())).unwrap();
    h.try_request(AppRequest::SetGainPan {
        gain: 0.4,
        pan: -0.6,
    })
    .unwrap();
    wait_until(|| {
        h.try_snapshot().is_some_and(|s| {
            s.generation == SessionGeneration(2) && s.control.pending_gain_pan == Some((0.4, -0.6))
        })
    });
    state.completed.store(false, Ordering::Release);
    saturation.unblock();
    phase(&h, SessionPhase::Running);
    let pending = h.try_snapshot().unwrap().control.pending_gain_pan;
    exit(rt);
    assert_eq!(pending, Some((0.4, -0.6)));
}

#[test]
fn catalog_failure_is_visible_and_refresh_preserves_running_session_and_last_rows() {
    let state = Arc::new(State::default());
    state.catalog_failure.store(true, Ordering::Release);
    state.started.store(true, Ordering::Release);
    let rt = runtime(&state);
    let h = rt.handle();
    wait_until(|| {
        h.try_snapshot().is_some_and(|s| {
            s.catalog_error.as_deref() == Some("catalog: fake enumeration failure")
        })
    });
    let initial = h.try_snapshot().unwrap();
    assert_eq!(initial.phase, SessionPhase::Idle);
    assert!(initial.catalog.outputs.is_empty());
    assert!(initial.error.is_none());
    state.catalog_failure.store(false, Ordering::Release);
    h.try_request(AppRequest::RefreshCatalog).unwrap();
    wait_until(|| {
        h.try_snapshot()
            .is_some_and(|s| s.catalog_error.is_none() && !s.catalog.outputs.is_empty())
    });
    let catalog = h.try_snapshot().unwrap().catalog.clone();
    h.try_request(AppRequest::Start(spec())).unwrap();
    phase(&h, SessionPhase::Running);
    state.catalog_failure.store(true, Ordering::Release);
    h.try_request(AppRequest::RefreshCatalog).unwrap();
    wait_until(|| h.try_snapshot().is_some_and(|s| s.catalog_error.is_some()));
    let failed = h.try_snapshot().unwrap();
    assert!(Arc::ptr_eq(&catalog, &failed.catalog));
    assert_eq!(failed.phase, SessionPhase::Running);
    assert!(failed.error.is_none());
    state.catalog_failure.store(false, Ordering::Release);
    h.try_request(AppRequest::RefreshCatalog).unwrap();
    wait_until(|| h.try_snapshot().is_some_and(|s| s.catalog_error.is_none()));
    assert_eq!(h.try_snapshot().unwrap().phase, SessionPhase::Running);
    exit(rt);
}
