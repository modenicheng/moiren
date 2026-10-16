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
    assert_eq!(json["schema_version"], 7);
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
