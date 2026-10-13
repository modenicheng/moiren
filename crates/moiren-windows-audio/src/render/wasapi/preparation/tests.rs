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
                    preparation.receive(info(), stop, report)?;
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
    let recovery = Arc::new(Mutex::new(Some(renderer)));
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
