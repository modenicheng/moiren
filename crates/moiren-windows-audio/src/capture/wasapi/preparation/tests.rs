use super::*;
use crate::{Activation, SessionDuration, capture::CaptureSource};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy)]
enum Outcome {
    Run,
    Fail,
    Panic,
    LostMetadata,
}
fn report() -> CaptureReport {
    CaptureReport::new(CaptureSource::Physical, SessionDuration::UntilStopped)
}
fn timing() -> NativeStreamTiming {
    NativeStreamTiming {
        sample_rate: 48000,
        period_frames: Some(336),
        capacity_frames: 738,
    }
}
fn input() -> PreparedInputQueue {
    PreparedInputQueue {
        capacity_frames: 8192,
        prime_frames: 1024,
        servo_reference_frames: 1024,
        low_water_frames: 482,
        high_water_frames: 7856,
        modeled_minimum_frames: Some(914),
    }
}
fn fake_prepared(
    stop: StopSignal,
    gate: ActivationGate,
    outcome: Outcome,
    exited: Arc<AtomicBool>,
) -> Result<PreparedNativeCapture, CaptureError> {
    start_native_owner(stop, move |stop, preparation, started| {
        let mut report = report();
        let result = (|| {
            match outcome {
                Outcome::Fail => return Err(CaptureError::UnsupportedFormat),
                Outcome::Panic => panic!("native capture preparation panic"),
                Outcome::LostMetadata => return Ok(CaptureStatus::Stopped),
                Outcome::Run => {}
            }
            let _ingress = preparation.prepare(timing(), 2, &stop, &mut report)?;
            if gate.wait()? == Activation::Cancelled {
                return Ok(CaptureStatus::Stopped);
            }
            report.stream_started = true;
            started.store(true, Ordering::Release);
            Ok(CaptureStatus::Completed)
        })();
        let report = preparation.finish(result, &stop, report);
        exited.store(true, Ordering::Release);
        report
    })
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
fn deferred_capture_failure_panic_and_metadata_loss_join_and_cancel_peers() {
    for (outcome, expected) in [
        (Outcome::Fail, CaptureError::UnsupportedFormat),
        (Outcome::Panic, CaptureError::WorkerPanicked),
        (Outcome::LostMetadata, CaptureError::StartupLost),
    ] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let exited = Arc::new(AtomicBool::new(false));
        assert!(
            matches!(fake_prepared(stop.clone(), gate.clone(), outcome, exited.clone()), Err(error) if error == expected)
        );
        if !matches!(outcome, Outcome::Panic) {
            assert!(exited.load(Ordering::Acquire));
        }
        assert_stopped(&stop);
        assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
    }
}

#[test]
fn deferred_capture_drop_before_attachment_joins_and_wakes_peer_gate() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let exited = Arc::new(AtomicBool::new(false));
    let prepared = fake_prepared(stop.clone(), gate.clone(), Outcome::Run, exited.clone()).unwrap();
    assert_eq!(prepared.info().timing, timing());
    assert!(!prepared.session.has_started());
    drop(prepared);
    assert!(exited.load(Ordering::Acquire));
    assert_stopped(&stop);
    assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
}

#[test]
fn deferred_capture_invalid_attachment_joins_and_cancels_peer() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let exited = Arc::new(AtomicBool::new(false));
    let prepared = fake_prepared(stop.clone(), gate, Outcome::Run, exited.clone()).unwrap();
    assert!(matches!(
        prepared.attach(PreparedInputQueue {
            prime_frames: 913,
            ..input()
        }),
        Err(CaptureError::Bridge(
            crate::clock_bridge::ClockBridgeError::InvalidConfig
        ))
    ));
    assert!(exited.load(Ordering::Acquire));
    assert_stopped(&stop);
}

#[test]
fn deferred_capture_attaches_joint_budget_and_waits_for_activation_or_stop() {
    for cancel in [false, true] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let exited = Arc::new(AtomicBool::new(false));
        let prepared =
            fake_prepared(stop.clone(), gate.clone(), Outcome::Run, exited.clone()).unwrap();
        let prepared = prepared.attach(input()).unwrap();
        assert_eq!(prepared.sample_rate, 48000);
        assert_eq!(prepared.channels, 2);
        assert!(!prepared.session.has_started());
        if cancel {
            gate.cancel().unwrap();
        } else {
            gate.activate().unwrap();
        }
        let report = prepared.session.join().unwrap();
        assert_eq!(report.stream_started, !cancel);
        assert_eq!(report.input_queue, Some(input()));
        assert!(report.failure.is_none());
        assert!(exited.load(Ordering::Acquire));
        assert_stopped(&stop);
    }
}
