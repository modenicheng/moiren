use super::*;
use crate::{Activation, SessionDuration, latency::LatencyBudgetError};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    Run,
    RunUntilStopped,
    FailBeforeMetadata,
    PanicBeforeMetadata,
    LostMetadata,
    FailAfterReceive,
    PanicAfterReceive,
    LostAttachment,
    PanicAfterActivation,
}
fn options() -> RenderOptions {
    RenderOptions {
        endpoint_id: "fake".into(),
        duration: SessionDuration::UntilStopped,
        latency_telemetry: crate::latency::TelemetryMode::Summary,
        shared_period: crate::shared_stream::SharedPeriodPolicy::default(),
        queue: RenderQueuePolicy::FullCapacity,
    }
}
fn info() -> NativeRenderInfo {
    NativeRenderInfo {
        timing: NativeStreamTiming {
            sample_rate: 48000,
            period_frames: Some(336),
            capacity_frames: 738,
        },
        shared_period: crate::shared_stream::SharedPeriodReport::default(),
    }
}
pub(crate) fn fake_prepared(
    stop: StopSignal,
    gate: ActivationGate,
    outcome: Outcome,
) -> Result<PreparedNativeRender, RenderError> {
    fake_prepared_with_info(stop, gate, outcome, info())
}
pub(crate) fn fake_prepared_with_info(
    stop: StopSignal,
    gate: ActivationGate,
    outcome: Outcome,
    metadata: NativeRenderInfo,
) -> Result<PreparedNativeRender, RenderError> {
    start_native_owner(
        stop,
        gate,
        stream::new_report(&options()),
        move |stop, gate, started, preparation| {
            let options = options();
            if matches!(outcome, Outcome::LostMetadata) {
                return NativeOwnerExit {
                    report: stream::new_report(&options),
                    renderer: None,
                    _padding: None,
                };
            }
            run_owner_with(
                &options,
                stop,
                gate,
                started,
                preparation,
                |stop, gate, started, preparation, report| {
                    match outcome {
                        Outcome::FailBeforeMetadata => return Err(RenderError::UnsupportedFormat),
                        Outcome::PanicBeforeMetadata => panic!("native preparation panic"),
                        _ => {}
                    }
                    report.sample_rate = metadata.timing.sample_rate;
                    report.endpoint_id = format!("fake-{}", metadata.timing.capacity_frames);
                    report.buffer_frames = Some(metadata.timing.capacity_frames);
                    report.engine_period_frames = metadata.timing.period_frames;
                    report.format_supported = true;
                    preparation.receive(metadata, stop, report)?;
                    match outcome {
                        Outcome::FailAfterReceive => return Err(RenderError::StagingBudget),
                        Outcome::PanicAfterReceive => panic!("native staging panic"),
                        Outcome::LostAttachment => return Ok(RenderStatus::Completed),
                        _ => {}
                    }
                    preparation
                        .attached
                        .send(Ok(()))
                        .map_err(|_| RenderError::StartupLost)?;
                    if gate.wait()? == Activation::Cancelled {
                        return Ok(RenderStatus::Stopped);
                    }
                    if matches!(outcome, Outcome::PanicAfterActivation) {
                        panic!("stream callback panic");
                    }
                    preparation
                        .renderer
                        .as_mut()
                        .unwrap()
                        .render_interleaved(&mut [0.0; 2])?;
                    started.store(true, Ordering::Release);
                    report.stream_started = true;
                    if matches!(outcome, Outcome::RunUntilStopped) {
                        unsafe {
                            windows::Win32::System::Threading::WaitForSingleObject(
                                handle(stop),
                                windows::Win32::System::Threading::INFINITE,
                            )
                        };
                        return Ok(RenderStatus::Stopped);
                    }
                    Ok(RenderStatus::Completed)
                },
            )
        },
    )
}
fn assert_stopped(stop: &StopSignal) {
    assert_eq!(
        unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(handle(&stop.event), 1000)
        },
        windows::Win32::Foundation::WAIT_OBJECT_0
    );
}

#[test]
fn render_abort_precedes_internal_startup_reap() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let (abort, aborted) = mpsc::channel();
    let observed = Arc::new(AtomicBool::new(false));
    let worker_observed = observed.clone();
    let result = start_native_owner_returning_with_abort(
        stop,
        gate,
        stream::new_report(&options()),
        move |_, _, _, preparation| {
            preparation
                .ready
                .send(Err(RenderError::UnsupportedFormat))
                .unwrap();
            worker_observed.store(
                aborted
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .is_ok(),
                Ordering::Release,
            );
            NativeOwnerExit {
                report: stream::new_report(&options()),
                renderer: None,
                _padding: None,
            }
        },
        || abort.send(()).unwrap(),
    );
    assert!(result.is_err());
    assert!(
        observed.load(Ordering::Acquire),
        "peer abort must precede the constructor's cleanup join"
    );
}

#[test]
fn render_abort_retains_prepare_report_and_peer_errors_on_control() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let control = thread::current().id();
    let mut calls = 0;
    let failure = start_native_owner_returning_with_abort(
        stop,
        gate,
        stream::new_report(&options()),
        |_, _, _, preparation| {
            let mut report = stream::new_report(&options());
            report.endpoint_id = "exact-partial".into();
            report.failure = Some("native failure retained".into());
            preparation
                .ready
                .send(Err(RenderError::UnsupportedFormat))
                .unwrap();
            NativeOwnerExit {
                report,
                renderer: None,
                _padding: None,
            }
        },
        || {
            assert_eq!(thread::current().id(), control);
            calls += 1;
            Err::<(), _>("recorder failed; child attempted")
        },
    )
    .err()
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(failure.error, RenderError::UnsupportedFormat);
    let report = failure.report.unwrap();
    assert_eq!(report.endpoint_id, "exact-partial");
    assert_eq!(report.failure.as_deref(), Some("native failure retained"));
    assert_eq!(failure.abort, Some(Err("recorder failed; child attempted")));
    assert_eq!(failure.stop_error, None);
    assert_eq!(failure.join_error, None);
}

#[test]
fn render_abort_lost_metadata_worker_and_callback_panics_still_reap() {
    for panic_worker in [false, true] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let failure = start_native_owner_returning_with_abort(
            stop,
            gate,
            stream::new_report(&options()),
            move |_, _, _, preparation| {
                drop(preparation);
                assert!(!panic_worker, "injected worker panic");
                NativeOwnerExit {
                    report: stream::new_report(&options()),
                    renderer: None,
                    _padding: None,
                }
            },
            || panic!("injected peer stop panic"),
        )
        .err()
        .unwrap();
        assert_eq!(failure.error, RenderError::StartupLost);
        assert_eq!(
            failure.join_error,
            panic_worker.then_some(RenderError::WorkerPanicked)
        );
        assert_eq!(failure.report.is_some(), !panic_worker);
        assert!(failure.abort_panicked);
    }
}

#[test]
fn render_abort_validation_spawn_and_control_unwind_have_no_hidden_join() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let mut invalid = options();
    invalid.endpoint_id.clear();
    let failure = prepare_render_native_with_gate_returning_with_abort(
        invalid,
        stop.clone(),
        gate.clone(),
        || 42,
    )
    .err()
    .unwrap();
    assert_eq!(failure.error, RenderError::InvalidEndpoint);
    assert_eq!(failure.abort, Some(42));
    assert!(failure.report.is_none());
    let failure = start_native_owner_with_spawn(
        stop.clone(),
        gate.clone(),
        stream::new_report(&options()),
        |_, _, _, _| panic!("must not spawn"),
        || 43,
        true,
        |_| Err(std::io::Error::from_raw_os_error(8)),
    )
    .err()
    .unwrap();
    assert_eq!(failure.error, RenderError::WorkerSpawn { code: Some(8) });
    assert_eq!(failure.abort, Some(43));
    assert!(failure.report.is_none());
    let (abort, aborted) = mpsc::channel();
    let (done, completed) = mpsc::channel();
    let worker = thread::spawn(move || {
        aborted
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        done.send(()).unwrap();
        NativeOwnerExit {
            report: stream::new_report(&options()),
            renderer: None,
            _padding: None,
        }
    });
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _guard = RenderAbortGuard {
            stop_without_owner: true,
            stop: stop.clone(),
            session: Some(RenderSession {
                stop: stop.event.clone(),
                started: Arc::new(AtomicBool::new(false)),
                worker: Some(worker),
                recovery: None,
                fallback: None,
                padding: None,
            }),
            before_join: Some(|| {
                abort.send(()).unwrap();
                panic!("peer stop panic")
            }),
        };
        panic!("control unwind");
    }));
    assert!(result.is_err());
    completed.try_recv().unwrap();
}

#[test]
fn render_abort_legacy_validation_and_spawn_do_not_signal_stop() {
    let stop = StopSignal::new().unwrap();
    let foreign = ActivationGate::new(StopSignal::new().unwrap()).unwrap();
    assert!(prepare_render_native_with_gate(options(), stop.clone(), foreign).is_err());
    assert_eq!(
        unsafe { windows::Win32::System::Threading::WaitForSingleObject(handle(&stop.event), 0) },
        windows::Win32::Foundation::WAIT_TIMEOUT
    );
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let failure = start_native_owner_with_spawn(
        stop.clone(),
        gate,
        stream::new_report(&options()),
        |_, _, _, _| panic!("no owner"),
        || (),
        false,
        |_| Err(std::io::Error::from_raw_os_error(8)),
    )
    .err()
    .unwrap();
    assert!(matches!(failure.error, RenderError::WorkerSpawn { .. }));
    assert_eq!(
        unsafe { windows::Win32::System::Threading::WaitForSingleObject(handle(&stop.event), 0) },
        windows::Win32::Foundation::WAIT_TIMEOUT
    );
}

#[test]
fn render_abort_attachment_guard_control_unwind_reaps_and_recovers_once() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let (abort, aborted) = mpsc::channel();
    let prepared = start_native_owner(
        stop.clone(),
        gate,
        stream::new_report(&options()),
        move |stop, _, _, mut preparation| {
            preparation.ready.send(Ok(info())).unwrap();
            let (handoff, _) = preparation.receiver.receive(&stop).unwrap().unwrap();
            preparation.renderer = handoff.lock().unwrap().take();
            aborted
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            panic!("owner panic after peer stop");
        },
    )
    .unwrap();
    let (dropped, released) = mpsc::channel();
    let control = thread::current().id();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let PreparedNativeRender {
            session,
            renderer: sender,
            ..
        } = prepared;
        let guard = RenderAbortGuard {
            stop,
            session: Some(session),
            before_join: Some(|| {
                abort.send(()).unwrap();
                panic!("peer-stop panic")
            }),
            stop_without_owner: true,
        };
        let retained = guard
            .session
            .as_ref()
            .unwrap()
            .recovery
            .as_ref()
            .unwrap()
            .clone();
        *retained.lock().unwrap() = Some(NativeRenderer::Master(Box::new(
            super::super::tests::probe_renderer(dropped),
        )));
        sender
            .send((
                retained,
                prepare_render_queue(info().timing, RenderQueuePolicy::FullCapacity).unwrap(),
            ))
            .ok()
            .unwrap();
        panic!("attachment control unwind with owned renderer");
    }));
    assert!(result.is_err());
    assert_eq!(released.try_recv().unwrap(), control);
    assert!(released.try_recv().is_err());
}

#[test]
fn render_abort_attachment_failures_return_renderer_once_and_preserve_native_failure() {
    for (outcome, expected) in [
        (Outcome::FailAfterReceive, RenderError::StagingBudget),
        (Outcome::PanicAfterReceive, RenderError::WorkerPanicked),
        (Outcome::LostAttachment, RenderError::StartupLost),
    ] {
        for panic_abort in [false, true] {
            let stop = StopSignal::new().unwrap();
            let gate = ActivationGate::new(stop.clone()).unwrap();
            let native = fake_prepared(stop.clone(), gate, outcome).unwrap();
            let (dropped, released) = mpsc::channel();
            let control = thread::current().id();
            let mut calls = 0;
            let failure = native
                .attach_returning_with_abort(
                    super::super::tests::probe_renderer(dropped),
                    RenderQueuePolicy::FullCapacity,
                    || {
                        calls += 1;
                        assert_eq!(thread::current().id(), control);
                        assert_stopped(&stop);
                        assert!(!panic_abort, "peer stop panic");
                        Err::<(), _>("both peer stops attempted")
                    },
                )
                .err()
                .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(failure.error, expected);
            assert_eq!(failure.abort_panicked, panic_abort);
            assert_eq!(failure.abort.is_some(), !panic_abort);
            let report = failure.report.as_ref().unwrap();
            assert_eq!(report.endpoint_id, "fake-738");
            if !matches!(outcome, Outcome::LostAttachment) {
                assert_eq!(
                    report.failure.as_deref(),
                    Some(expected.to_string().as_str())
                );
            }
            assert!(released.try_recv().is_err());
            drop(failure.renderer);
            assert_eq!(released.try_recv().unwrap(), control);
            assert!(released.try_recv().is_err());
        }
    }
}

#[test]
fn render_abort_invalid_queue_and_closed_sender_abort_before_owner_reap() {
    for invalid_queue in [true, false] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let (abort, aborted) = mpsc::channel();
        let (closed, is_closed) = mpsc::channel();
        let prepared = start_native_owner(
            stop.clone(),
            gate,
            stream::new_report(&options()),
            move |_, _, _, preparation| {
                preparation.ready.send(Ok(info())).unwrap();
                if !invalid_queue {
                    preparation.receiver.close_and_take();
                }
                closed.send(()).unwrap();
                aborted
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                NativeOwnerExit {
                    report: stream::new_report(&options()),
                    renderer: None,
                    _padding: None,
                }
            },
        )
        .unwrap();
        is_closed
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (dropped, released) = mpsc::channel();
        let queue = if invalid_queue {
            RenderQueuePolicy::Target {
                frames: 1,
                wake_margin_frames: 96,
                callback_budget_frames: 48,
            }
        } else {
            RenderQueuePolicy::FullCapacity
        };
        let failure = prepared
            .attach_returning_with_abort(
                super::super::tests::probe_renderer(dropped),
                queue,
                || abort.send(()).unwrap(),
            )
            .err()
            .unwrap();
        if invalid_queue {
            assert!(matches!(failure.error, RenderError::LatencyBudget(_)));
        } else {
            assert!(matches!(
                failure.error,
                RenderError::Api {
                    stage: "Send prepared renderer",
                    ..
                }
            ));
        }
        assert!(failure.report.is_some());
        assert_eq!(failure.join_error, None);
        drop(failure.renderer);
        assert_eq!(released.try_recv().unwrap(), thread::current().id());
    }
}

#[test]
fn render_abort_fallback_retains_worker_panic_and_renderer() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let prepared = start_native_owner(
        stop.clone(),
        gate,
        stream::new_report(&options()),
        |stop, _, _, mut preparation| {
            preparation.ready.send(Ok(info())).unwrap();
            let (retained, _) = preparation.receiver.receive(&stop).unwrap().unwrap();
            preparation.renderer = retained.lock().unwrap().take();
            panic!("outside normal native unwind scope");
        },
    )
    .unwrap();
    let (dropped, released) = mpsc::channel();
    let failure = prepared
        .attach_returning_with_abort(
            super::super::tests::probe_renderer(dropped),
            RenderQueuePolicy::FullCapacity,
            || (),
        )
        .err()
        .unwrap();
    assert_eq!(failure.error, RenderError::StartupLost);
    assert_eq!(failure.join_error, Some(RenderError::WorkerPanicked));
    assert_eq!(
        failure.report.as_ref().unwrap().failure.as_deref(),
        Some(RenderError::WorkerPanicked.to_string().as_str())
    );
    assert!(released.try_recv().is_err());
    drop(failure.renderer);
    assert_eq!(released.try_recv().unwrap(), thread::current().id());
}

#[test]
fn render_abort_success_prepare_and_attach_never_call_callback() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let prepared = start_native_owner_returning_with_abort(
        stop.clone(),
        gate,
        stream::new_report(&options()),
        |stop, gate, _, preparation| {
            preparation.ready.send(Ok(info())).unwrap();
            let _attachment = preparation.receiver.receive(&stop).unwrap();
            preparation.attached.send(Ok(())).unwrap();
            assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
            NativeOwnerExit {
                report: stream::new_report(&options()),
                renderer: None,
                _padding: None,
            }
        },
        || panic!("successful prepare must not abort"),
    )
    .unwrap();
    let (dropped, released) = mpsc::channel();
    let attached = prepared
        .attach_returning_with_abort(
            super::super::tests::probe_renderer(dropped),
            RenderQueuePolicy::FullCapacity,
            || panic!("successful attach must not abort"),
        )
        .ok()
        .unwrap();
    attached.request_stop().unwrap();
    attached.into_session().join().unwrap();
    assert_eq!(released.try_recv().unwrap(), thread::current().id());
}

#[test]
fn render_abort_legacy_explicit_error_and_closed_handshake_panic_precedence() {
    for explicit in [false, true] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let result = start_native_owner(
            stop,
            gate,
            stream::new_report(&options()),
            move |_, _, _, preparation| {
                if explicit {
                    preparation
                        .ready
                        .send(Err(RenderError::StartupLost))
                        .unwrap();
                }
                drop(preparation);
                panic!("join panic");
            },
        );
        let expected = if explicit {
            RenderError::StartupLost
        } else {
            RenderError::WorkerPanicked
        };
        assert!(matches!(result, Err(error) if error == expected));
    }
}

#[test]
fn render_abort_stop_failure_join_panic_and_peer_error_are_independent() {
    use std::os::windows::io::FromRawHandle;
    let mutex =
        unsafe { windows::Win32::System::Threading::CreateMutexW(None, false, None) }.unwrap();
    let stop = StopSignal {
        event: Arc::new(unsafe { OwnedHandle::from_raw_handle(mutex.0) }),
    };
    let actual_stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(actual_stop).unwrap();
    let failure = start_native_owner_returning_with_abort(
        stop,
        gate,
        stream::new_report(&options()),
        |_, _, _, preparation| {
            preparation
                .ready
                .send(Err(RenderError::UnsupportedFormat))
                .unwrap();
            panic!("join panic after primary failure");
        },
        || Err::<(), _>("both peer stops attempted"),
    )
    .err()
    .unwrap();
    assert_eq!(failure.error, RenderError::UnsupportedFormat);
    assert!(matches!(
        failure.stop_error,
        Some(RenderError::Api {
            stage: "SetEvent(stop)",
            ..
        })
    ));
    assert_eq!(failure.join_error, Some(RenderError::WorkerPanicked));
    assert_eq!(failure.abort, Some(Err("both peer stops attempted")));
    assert!(failure.report.is_none());
}

#[test]
fn render_abort_lost_metadata_and_attachment_reply_signal_before_reap() {
    for attach in [false, true] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let (abort, aborted) = mpsc::channel();
        let (done, completed) = mpsc::channel();
        let before_join = || {
            abort.send(()).unwrap();
            panic!("peer callback panic after stops")
        };
        let owner = move |stop: Arc<OwnedHandle>,
                          _: ActivationGate,
                          _: Arc<AtomicBool>,
                          mut preparation: RenderPreparation| {
            if attach {
                preparation.ready.send(Ok(info())).unwrap();
                let (handoff, _) = preparation.receiver.receive(&stop).unwrap().unwrap();
                preparation.renderer = handoff.lock().unwrap().take();
            }
            let (unused_ready, _) = mpsc::sync_channel(1);
            let (unused_attached, _) = mpsc::sync_channel(1);
            drop(std::mem::replace(&mut preparation.ready, unused_ready));
            drop(std::mem::replace(
                &mut preparation.attached,
                unused_attached,
            ));
            aborted
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            done.send(()).unwrap();
            let renderer = preparation.renderer.take();
            NativeOwnerExit {
                report: stream::new_report(&options()),
                renderer,
                _padding: None,
            }
        };
        if attach {
            let prepared =
                start_native_owner(stop, gate, stream::new_report(&options()), owner).unwrap();
            let (dropped, released) = mpsc::channel();
            let failure = prepared
                .attach_returning_with_abort(
                    super::super::tests::probe_renderer(dropped),
                    RenderQueuePolicy::FullCapacity,
                    before_join,
                )
                .err()
                .unwrap();
            assert_eq!(failure.error, RenderError::StartupLost);
            assert!(failure.abort_panicked);
            assert!(failure.report.is_some());
            drop(failure.renderer);
            assert_eq!(released.try_recv().unwrap(), thread::current().id());
        } else {
            let failure = start_native_owner_returning_with_abort(
                stop,
                gate,
                stream::new_report(&options()),
                owner,
                before_join,
            )
            .err()
            .unwrap();
            assert_eq!(failure.error, RenderError::StartupLost);
            assert!(failure.abort_panicked);
            assert!(failure.report.is_some());
        }
        completed.try_recv().unwrap();
    }
}

#[test]
fn prepared_padding_observer_stays_unknown_and_is_invalidated_after_native_exit() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let prepared = fake_prepared(stop.clone(), gate, Outcome::Run).unwrap();
    let padding = prepared.padding_observer();
    assert_eq!(padding.try_snapshot(), None);
    stop.request_stop().unwrap();
    prepared.cancel_with_report().unwrap();
    assert_eq!(padding.try_snapshot(), None);
}

#[test]
fn prepared_cancellation_retains_stop_join_and_pre_activation_report() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let prepared = fake_prepared(stop, gate, Outcome::Run).unwrap();

    let cancellation = prepared.cancel_with_report_returning();

    assert_eq!(cancellation.stop_error, None);
    assert_eq!(cancellation.join_error, None);
    let report = cancellation.report.expect("joined owner report");
    assert_eq!(report.status, RenderStatus::Failed);
    assert!(!report.stream_started);
    assert!(!report.stop_succeeded);
    assert_eq!(
        report.failure.as_deref(),
        Some("render preparation was cancelled")
    );
}

#[test]
fn deferred_render_failure_panic_and_metadata_loss_cancel_peers_before_returning() {
    for (outcome, expected) in [
        (Outcome::FailBeforeMetadata, RenderError::UnsupportedFormat),
        (Outcome::PanicBeforeMetadata, RenderError::WorkerPanicked),
        (Outcome::LostMetadata, RenderError::StartupLost),
    ] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        assert!(
            matches!(fake_prepared(stop.clone(), gate.clone(), outcome), Err(error) if error == expected)
        );
        assert_stopped(&stop);
        assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
    }
}

#[test]
fn deferred_render_cancel_before_attachment_never_owns_or_runs_an_engine() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let prepared = fake_prepared(stop.clone(), gate.clone(), Outcome::Run).unwrap();
    assert_eq!(prepared.info().timing, info().timing);
    assert!(!prepared.session.has_started());
    stop.request_stop().unwrap();
    let exit = prepared.session.join_owner().unwrap();
    assert!(exit.renderer.is_none());
    assert_eq!(
        exit.report.failure.as_deref(),
        Some(RenderError::Cancelled.to_string().as_str())
    );
    assert_eq!(exit.report.stats.processed_frames, 0);
    assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
}

#[test]
fn deferred_render_attachment_failure_returns_engine_to_control_thread() {
    for (outcome, expected) in [
        (Outcome::FailAfterReceive, RenderError::StagingBudget),
        (Outcome::PanicAfterReceive, RenderError::WorkerPanicked),
        (Outcome::LostAttachment, RenderError::StartupLost),
    ] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let prepared = fake_prepared(stop.clone(), gate, outcome).unwrap();
        let (dropped, released) = mpsc::channel();
        let renderer = super::super::tests::probe_renderer(dropped);
        assert!(
            matches!(prepared.attach(renderer, RenderQueuePolicy::FullCapacity), Err(error) if error == expected)
        );
        assert_eq!(
            released
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            thread::current().id()
        );
        assert_stopped(&stop);
    }
}

#[test]
fn deferred_render_invalid_budget_cancels_owner_before_handoff() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let prepared = fake_prepared(stop.clone(), gate, Outcome::Run).unwrap();
    let (dropped, released) = mpsc::channel();
    let renderer = super::super::tests::probe_renderer(dropped);
    assert!(matches!(
        prepared.attach(
            renderer,
            RenderQueuePolicy::Target {
                frames: 479,
                wake_margin_frames: 96,
                callback_budget_frames: 48,
            }
        ),
        Err(RenderError::LatencyBudget(
            LatencyBudgetError::InsufficientRenderBudget
        ))
    ));
    assert_eq!(
        released
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap(),
        thread::current().id()
    );
    assert_stopped(&stop);
}

#[test]
fn deferred_render_waits_for_gate_and_returns_engine_after_cancel_start_or_panic() {
    for (cancel, outcome) in [
        (true, Outcome::Run),
        (false, Outcome::Run),
        (false, Outcome::PanicAfterActivation),
    ] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let prepared = fake_prepared(stop.clone(), gate, outcome).unwrap();
        let (dropped, released) = mpsc::channel();
        let renderer = super::super::tests::probe_renderer(dropped);
        let prepared = prepared
            .attach(renderer, RenderQueuePolicy::FullCapacity)
            .unwrap();
        assert!(!prepared.has_started());
        assert!(released.try_recv().is_err());
        if cancel {
            prepared.request_stop().unwrap();
        } else {
            prepared.activate().unwrap();
        }
        let exit = prepared.into_session().join_with_renderer().unwrap();
        let ran = !cancel && matches!(outcome, Outcome::Run);
        assert_eq!(exit.renderer.timeline(), u64::from(ran));
        assert_eq!(exit.report.stats.processed_frames, u64::from(ran));
        assert_eq!(exit.report.stream_started, ran);
        if matches!(outcome, Outcome::PanicAfterActivation) {
            assert_eq!(exit.report.status, RenderStatus::Failed);
            assert_eq!(
                exit.report.failure.as_deref(),
                Some(RenderError::WorkerPanicked.to_string().as_str())
            );
        }
        assert!(released.try_recv().is_err());
        drop(exit.renderer);
        assert_eq!(
            released
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            thread::current().id()
        );
        assert_stopped(&stop);
    }
}

#[test]
fn cancelled_queued_renderer_returns_to_joiner_without_counting_its_prior_timeline() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let (sender, receiver) = preparation::channel().unwrap();
    let (ready, _metadata) = mpsc::sync_channel(1);
    let (attached, _attachment) = mpsc::sync_channel(1);
    let (dropped, released) = mpsc::channel();
    let mut renderer = super::super::tests::probe_renderer(dropped);
    renderer.render_interleaved(&mut [0.0; 2]).unwrap();
    let queue = prepare_render_queue(info().timing, RenderQueuePolicy::FullCapacity).unwrap();
    let recovery = Arc::new(Mutex::new(Some(NativeRenderer::Master(Box::new(renderer)))));
    sender.send((recovery.clone(), queue)).unwrap();
    stop.request_stop().unwrap();
    let worker_recovery = recovery.clone();
    let worker = thread::spawn(move || {
        run_owner_with(
            &options(),
            stop.event,
            gate,
            Arc::new(AtomicBool::new(false)),
            RenderPreparation {
                ready,
                attached,
                receiver,
                renderer: None,
                initial_frame: 0,
                recovery: worker_recovery,
                padding: None,
            },
            |stop, _, _, preparation, report| {
                preparation.receive(info(), stop, report)?;
                Ok(RenderStatus::Completed)
            },
        )
    });
    let exit = worker.join().unwrap();
    let renderer = recovery.lock().unwrap().take().unwrap();
    assert_eq!(renderer.timeline(), 1);
    assert_eq!(exit.report.stats.processed_frames, 0);
    assert!(released.try_recv().is_err());
    drop(renderer);
    assert_eq!(
        released
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap(),
        thread::current().id()
    );
}

#[test]
fn trace_windows_and_event_allocation_survive_native_callback_unwind_to_joiner() {
    use stream::diagnostic_tests as diagnostic;
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let (sender, receiver) = preparation::channel().unwrap();
    let (ready, _metadata) = mpsc::sync_channel(1);
    let (attached, _attachment) = mpsc::sync_channel(1);
    let recovery = Arc::new(Mutex::new(None));
    let worker_recovery = recovery.clone();
    let (dropped, released) = mpsc::channel();
    let renderer = super::super::tests::probe_renderer_with_panic(dropped, Some(4));
    let worker = thread::spawn(move || {
        let _sender = sender;
        diagnostic::track_events();
        let exit = run_owner_with(
            &diagnostic::options(),
            stop.event,
            gate,
            Arc::new(AtomicBool::new(false)),
            RenderPreparation {
                ready,
                attached,
                receiver,
                renderer: Some(NativeRenderer::Master(Box::new(renderer))),
                initial_frame: 0,
                recovery: worker_recovery,
                padding: None,
            },
            |_, _, _, preparation, report| {
                diagnostic::run(
                    &diagnostic::options(),
                    report,
                    preparation.renderer.as_mut().unwrap(),
                    diagnostic::Failure::None,
                    0,
                )
            },
        );
        diagnostic::stop_tracking_events();
        exit
    });
    let exit = worker.join().unwrap();
    assert!(released.try_recv().is_err());
    drop(recovery.lock().unwrap().take());
    assert_eq!(released.recv().unwrap(), thread::current().id());
    assert_eq!(
        exit.report.failure.as_deref(),
        Some(RenderError::WorkerPanicked.to_string().as_str())
    );
    assert!(
        exit.report.diagnostics.enabled,
        "Trace was lost across unwind"
    );
    assert_eq!(exit.report.diagnostics.wake_budget_100ns, 834);
    assert_eq!(exit.report.diagnostics.events.len(), 2);
    assert_eq!(
        exit.report.diagnostics.events[1].previous_callbacks.len(),
        1
    );
    let (pointer, worker_frees, joiner_frees) = diagnostic::event_allocation();
    assert_ne!(pointer, 0, "the event allocation was not observed");
    assert_eq!(exit.report.diagnostics.events.as_ptr() as usize, pointer);
    assert_eq!(worker_frees, 0, "native unwind freed the event allocation");
    assert_eq!(joiner_frees, 0);
    let json = serde_json::to_value(&exit.report).unwrap();
    assert_eq!(json["schema_version"], 10);
    assert!(json.get("diagnostics_recorder").is_none());
    drop(exit);
    assert_eq!(diagnostic::event_allocation(), (pointer, 0, 1));
}

#[test]
fn trace_metadata_survives_native_cancel_and_start_failure() {
    use stream::diagnostic_tests as diagnostic;
    for (failure, expected) in [
        (diagnostic::Failure::Cancel, Ok(RenderStatus::Stopped)),
        (
            diagnostic::Failure::Start,
            Err(RenderError::Api {
                stage: "Start",
                hresult: windows::Win32::Foundation::E_FAIL.0,
            }),
        ),
    ] {
        let options = diagnostic::options();
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let (_sender, receiver) = preparation::channel().unwrap();
        let (ready, _) = mpsc::sync_channel(1);
        let (attached, _) = mpsc::sync_channel(1);
        let (dropped, _released) = mpsc::channel();
        let renderer = super::super::tests::probe_renderer(dropped);
        let exit = run_owner_with(
            &options,
            stop.event,
            gate,
            Arc::new(AtomicBool::new(false)),
            RenderPreparation {
                ready,
                attached,
                receiver,
                renderer: Some(NativeRenderer::Master(Box::new(renderer))),
                initial_frame: 0,
                recovery: Arc::new(Mutex::new(None)),
                padding: None,
            },
            |_, _, _, preparation, report| {
                let actual = diagnostic::run(
                    &options,
                    report,
                    preparation.renderer.as_mut().unwrap(),
                    failure,
                    0,
                );
                assert_eq!(actual, expected);
                actual
            },
        );
        assert!(exit.report.diagnostics.enabled);
        assert_eq!(exit.report.diagnostics.wake_budget_100ns, 834);
        assert!(exit.report.diagnostics.events.is_empty());
        assert!(!exit.report.stream_started);
    }
}
