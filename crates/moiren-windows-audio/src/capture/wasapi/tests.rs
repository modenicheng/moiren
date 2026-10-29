use super::*;

#[test]
fn process_preparation_rejects_foreign_gate_stop_before_spawning() {
    use crate::{GateError, SessionDuration};
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(StopSignal::new().unwrap()).unwrap();
    let expected = CaptureError::Gate(GateError::Api {
        stage: "ActivationGate stop identity",
        hresult: 0x80070057u32 as i32,
    });
    let process = crate::process_loopback::prepare_process_capture_with_gate(
        crate::process_loopback::ProcessLoopbackOptions {
            artifact: None,
            bridge: Default::default(),
            latency_telemetry: crate::latency::TelemetryMode::Summary,
            target: crate::process_loopback::ProcessIdentity {
                pid: u32::MAX,
                creation_time_100ns: 1,
                executable_name: "unopened.exe".into(),
            },
            duration: SessionDuration::UntilStopped,
        },
        stop,
        gate,
    );
    assert!(matches!(process, Err(error) if error == expected));
}

#[test]
fn target_exit_wakes_without_audio_and_stop_wins_all_signals() {
    let stop = crate::StopSignal::new().unwrap();
    let target = crate::owner::OwnedHandle::event().unwrap();
    let audio = crate::owner::OwnedHandle::event().unwrap();
    unsafe {
        SetEvent(target.0).unwrap();
    }
    assert_eq!(
        wait_target(handle(&stop.event), audio.0, Some(target.0), 1000).unwrap(),
        Wake::TargetExited
    );
    // Keep target signaled alongside audio; target wins the packet wake.
    unsafe {
        SetEvent(target.0).unwrap();
        SetEvent(audio.0).unwrap();
    }
    assert_eq!(
        wait_target(handle(&stop.event), audio.0, Some(target.0), 0).unwrap(),
        Wake::TargetExited
    );
    stop.request_stop().unwrap();
    assert_eq!(
        wait_target(handle(&stop.event), audio.0, Some(target.0), 0).unwrap(),
        Wake::Stop
    );
}
use super::packet::transfer_packet;
use crate::{
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    owner::OwnedHandle,
    stats::SILENT,
};
use std::{
    cell::{RefCell, UnsafeCell},
    rc::Rc,
};
use windows::{
    Win32::Media::Audio::*,
    core::{Result as WinResult, implement},
};

#[test]
fn capture_stop_is_responsive_without_audio_and_has_priority() {
    let stop = StopSignal::new().unwrap().event;
    let audio = OwnedHandle::event().unwrap();
    assert_eq!(wait(handle(&stop), audio.0, 0).unwrap(), Wake::Timeout);
    unsafe {
        SetEvent(handle(&stop)).unwrap();
        SetEvent(audio.0).unwrap();
    }
    assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
}

#[test]
fn linked_render_and_capture_stop_without_waiting_for_worker_cleanup() {
    let session = CaptureSession {
        started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        stop: StopSignal::new().unwrap().event,
        worker: None,
    };
    let peer = session.stop_signal();
    let audio = OwnedHandle::event().unwrap();
    peer.request_stop().unwrap();
    assert_eq!(
        wait(handle(&session.stop), audio.0, 100).unwrap(),
        Wake::Stop
    );
}

#[implement(IAudioCaptureClient)]
struct FakeCapture {
    data: Rc<UnsafeCell<[f32; 4]>>,
    releases: Rc<RefCell<Vec<u32>>>,
    frames: u32,
    flags: u32,
    null_buffer: bool,
    fail_release: bool,
}
impl IAudioCaptureClient_Impl for FakeCapture_Impl {
    fn GetBuffer(
        &self,
        data: *mut *mut u8,
        frames: *mut u32,
        flags: *mut u32,
        position: *mut u64,
        qpc: *mut u64,
    ) -> WinResult<()> {
        // SAFETY: transfer_packet provides valid outputs on this test thread.
        unsafe {
            *data = if self.null_buffer {
                std::ptr::null_mut()
            } else {
                self.data.get().cast()
            };
            *frames = self.frames;
            *flags = self.flags;
            *position = 0;
            *qpc = 1;
        }
        Ok(())
    }
    fn ReleaseBuffer(&self, frames: u32) -> WinResult<()> {
        self.releases.borrow_mut().push(frames);
        if self.fail_release {
            Err(windows::Win32::Foundation::E_NOINTERFACE.into())
        } else {
            Ok(())
        }
    }
    fn GetNextPacketSize(&self) -> WinResult<u32> {
        Ok(self.frames)
    }
}

#[test]
fn capture_lease_releases_complete_packet_on_success_silence_and_validation_failure() {
    for (frames, flags, null_buffer, fails) in [
        (2, 0, false, false),
        (2, SILENT, true, false),
        (2, 0, true, true),
        (3, 0, false, true),
        (0, 0, true, false),
    ] {
        let releases = Rc::new(RefCell::new(Vec::new()));
        let client: IAudioCaptureClient = FakeCapture {
            data: Rc::new(UnsafeCell::new([0.25, -0.25, 0.5, -0.5])),
            releases: Rc::clone(&releases),
            frames,
            flags,
            null_buffer,
            fail_release: false,
        }
        .into();
        let (mut input, _source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
        assert_eq!(
            transfer_packet(&client, 2, &mut input, None, None).is_err(),
            fails
        );
        assert_eq!(
            *releases.borrow(),
            if frames == 0 { vec![] } else { vec![frames] }
        );
        assert_eq!(
            observer.snapshot().captured_frames,
            if fails { 0 } else { frames as u64 }
        );
    }
}

#[test]
fn published_packet_evidence_survives_release_failure_without_healing_native_error() {
    let releases = Rc::new(RefCell::new(Vec::with_capacity(1)));
    let client: IAudioCaptureClient = FakeCapture {
        data: Rc::new(UnsafeCell::new([0.25, -0.25, 0.5, -0.5])),
        releases: releases.clone(),
        frames: 2,
        flags: 0,
        null_buffer: false,
        fail_release: true,
    }
    .into();
    let (mut input, _source, observer) = crate::clock_bridge::capture_bridge_with_clock(
        ClockBridgeConfig::default(),
        crate::latency::QpcClock::new(),
    )
    .unwrap();
    let error = transfer_packet(&client, 2, &mut input, None, None)
        .err()
        .unwrap();
    assert_eq!(
        error,
        CaptureError::Api {
            stage: "ReleaseBuffer(capture)",
            hresult: windows::Win32::Foundation::E_NOINTERFACE.0,
        }
    );
    assert_eq!(releases.borrow().as_slice(), [2]);
    let packets = observer.packet_history().unwrap();
    assert_eq!(packets.total_events, 1);
    assert_eq!(packets.offered_frames, 2);
    assert_eq!(packets.published_frames, 2);
    let sample = packets.events[0].sample;
    assert!(sample.qpc_before_100ns.unwrap() <= sample.qpc_after_100ns.unwrap());
    assert!(
        sample.qpc_after_100ns.unwrap() > 1,
        "GetBuffer sample stamp is not the commit clock"
    );
    assert!(
        observer.delivery_events().unwrap().events.is_empty(),
        "no successfully released drain"
    );
}

#[test]
fn capture_lease_copies_raw_evidence_before_release_without_allocating() {
    use crate::capture::{PcmEvidenceConfig, pcm_evidence_stream};
    for (frames, flags, null_buffer, fails) in [
        (2, 0, false, false),
        (2, SILENT, true, false),
        (2, 0, true, true),
        (3, 0, false, true),
        (0, 0, true, false),
    ] {
        let samples = [0.25, -0.25, f32::from_bits(0x7fc0_1234), f32::NEG_INFINITY];
        let data = Rc::new(UnsafeCell::new(samples));
        // The fake ReleaseBuffer log has its capacity before the guarded lease.
        let releases = Rc::new(RefCell::new(Vec::with_capacity(1)));
        let client: IAudioCaptureClient = FakeCapture {
            data: data.clone(),
            releases: releases.clone(),
            frames,
            flags,
            null_buffer,
            fail_release: false,
        }
        .into();
        let (mut ingress, _source, _) = capture_bridge(ClockBridgeConfig::default()).unwrap();
        let (mut writer, mut reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 2,
                detect_position_gaps: true,
            },
            48_000,
            2,
        )
        .unwrap();
        let (result, counts) = allocation_guard::count_allocations(|| {
            transfer_packet(&client, 2, &mut ingress, None, Some(&mut writer))
        });
        assert_eq!(counts, (0, 0));
        assert_eq!(result.is_err(), fails);
        let expected_release = [frames];
        assert_eq!(
            releases.borrow().as_slice(),
            if frames == 0 {
                &[][..]
            } else {
                &expected_release[..]
            }
        );
        // The lease has ended; replacing native memory must not change evidence.
        unsafe { *data.get() = [99.0; 4] };
        if fails || frames == 0 {
            assert!(reader.pop().is_none());
            assert_eq!(writer.snapshot().packets, 0);
        } else {
            for offset in 0..2 {
                let frame = reader.pop().unwrap();
                assert_eq!(frame.packet_frame_offset, offset as u32);
                assert_eq!(frame.qpc_100ns, 1);
                for channel in 0..2 {
                    let expected = if flags & SILENT != 0 {
                        0.0
                    } else {
                        samples[offset * 2 + channel]
                    };
                    assert_eq!(frame.samples[channel].to_bits(), expected.to_bits());
                }
            }
            assert!(reader.pop().is_none());
        }
    }
}

#[test]
fn prepared_capture_publishes_bridge_before_gate_and_cancel_never_runs() {
    use crate::{Activation, ActivationGate, SessionDuration};
    use std::sync::atomic::{AtomicUsize, Ordering};
    for cancel in [false, true] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let worker_gate = gate.clone();
        let runs = Arc::new(AtomicUsize::new(0));
        let ran = runs.clone();
        let prepared = start_owner(stop, move |stop, sender, started| {
            let (_, source, observer) = capture_bridge(ClockBridgeConfig::default()).unwrap();
            let mut report = crate::capture::CaptureReport::new(
                crate::capture::CaptureSource::Physical,
                SessionDuration::UntilStopped,
            );
            report.shared_period.requested_period_frames = Some(240);
            report.shared_period.actual_period_frames = Some(336);
            let result = sender
                .send(Ok(Startup {
                    source,
                    observer,
                    sample_rate: 48000,
                    channels: 2,
                }))
                .map_err(|_| CaptureError::StartupLost)
                .and_then(|()| worker_gate.wait().map_err(CaptureError::from))
                .map(|activation| {
                    if activation == Activation::Start {
                        ran.fetch_add(1, Ordering::SeqCst);
                        started.store(true, Ordering::Release);
                        report.stream_started = true;
                        crate::capture::CaptureStatus::Completed
                    } else {
                        crate::capture::CaptureStatus::Stopped
                    }
                });
            stream::finish(result, &stop, &sender, report)
        })
        .unwrap();
        assert_eq!(prepared.sample_rate, 48000);
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert!(!prepared.session.has_started());
        if cancel {
            gate.cancel().unwrap();
        } else {
            gate.activate().unwrap();
        }
        let report = prepared.session.join().unwrap();
        assert_eq!(report.stream_started, !cancel);
        assert_eq!(runs.load(Ordering::SeqCst), usize::from(!cancel));
        assert_eq!(report.schema_version, 10);
        assert_eq!(report.requested_seconds, None);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["schema_version"], 10);
        assert_eq!(json["delivery"]["packets"], 0);
        assert!(json["delivery_bound"].is_null());
        assert_eq!(json["shared_period"]["requested_period_frames"], 240);
        assert_eq!(json["shared_period"]["actual_period_frames"], 336);
        assert!(json["requested_seconds"].is_null());
    }
}

// The library test binary needs its own allocator; the retired render worker
// used to provide it. Integration tests compile this same scaffold separately.
#[path = "../../../tests/common/alloc_guard.rs"]
mod allocation_guard;

#[test]
fn capture_preparation_failure_joins_owner_before_returning() {
    let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let owner_exited = exited.clone();
    let result = start_owner(StopSignal::new().unwrap(), move |stop, sender, _| {
        let report = crate::capture::CaptureReport::new(
            crate::capture::CaptureSource::Physical,
            crate::SessionDuration::For(std::time::Duration::from_secs(10)),
        );
        let report = stream::finish(Err(CaptureError::UnsupportedFormat), &stop, &sender, report);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["schema_version"], 10);
        assert_eq!(json["requested_seconds"], 10.0);
        owner_exited.store(true, std::sync::atomic::Ordering::Release);
        report
    });
    assert!(matches!(result, Err(CaptureError::UnsupportedFormat)));
    assert!(exited.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn capture_abort_precedes_internal_startup_reap() {
    let (abort, aborted) = mpsc::channel();
    let observed = Arc::new(AtomicBool::new(false));
    let worker_observed = observed.clone();
    let result = start_owner_returning_with_abort(
        StopSignal::new().unwrap(),
        move |stop, sender, _| {
            let report = crate::capture::CaptureReport::new(
                crate::capture::CaptureSource::ProcessLoopback,
                crate::SessionDuration::UntilStopped,
            );
            let report =
                stream::finish(Err(CaptureError::UnsupportedFormat), &stop, &sender, report);
            worker_observed.store(
                aborted
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .is_ok(),
                Ordering::Release,
            );
            report
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
fn capture_abort_retains_primary_report_and_control_borrow() {
    let control = thread::current().id();
    let mut calls = 0;
    let failure = start_owner_returning_with_abort(
        StopSignal::new().unwrap(),
        |stop, sender, _| {
            let mut report = crate::capture::CaptureReport::new(
                crate::capture::CaptureSource::ProcessLoopback,
                crate::SessionDuration::UntilStopped,
            );
            report.activation_seconds = Some(0.125);
            stream::finish(Err(CaptureError::TargetExited), &stop, &sender, report)
        },
        || {
            assert_eq!(thread::current().id(), control);
            calls += 1;
            Err::<(), _>("recorder stop failed; child stop attempted")
        },
    )
    .err()
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(failure.error, CaptureError::TargetExited);
    let report = failure.report.unwrap();
    assert_eq!(report.activation_seconds, Some(0.125));
    assert_eq!(
        report.failure.as_deref(),
        Some(CaptureError::TargetExited.to_string().as_str())
    );
    assert_eq!(
        failure.abort,
        Some(Err("recorder stop failed; child stop attempted"))
    );
    assert!(!failure.abort_panicked);
    assert_eq!(failure.stop_error, None);
    assert_eq!(failure.join_error, None);
}

#[test]
fn capture_abort_panic_still_reaps_lost_handshake_and_worker_panic() {
    for panic_worker in [false, true] {
        let failure = start_owner_returning_with_abort(
            StopSignal::new().unwrap(),
            move |_, sender, _| {
                drop(sender);
                assert!(!panic_worker, "injected worker panic");
                crate::capture::CaptureReport::new(
                    crate::capture::CaptureSource::ProcessLoopback,
                    crate::SessionDuration::UntilStopped,
                )
            },
            || panic!("injected peer-stop panic"),
        )
        .err()
        .unwrap();
        assert_eq!(failure.error, CaptureError::StartupLost);
        assert_eq!(
            failure.join_error,
            panic_worker.then_some(CaptureError::WorkerPanicked)
        );
        assert_eq!(failure.report.is_some(), !panic_worker);
        assert!(failure.abort_panicked);
        assert!(failure.abort.is_none());
    }
}

#[test]
fn capture_abort_validation_and_spawn_failure_without_owner() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let mut calls = 0;
    let failure = crate::process_loopback::prepare_process_capture_with_gate_returning_with_abort(
        crate::process_loopback::ProcessLoopbackOptions {
            artifact: None,
            bridge: Default::default(),
            latency_telemetry: crate::latency::TelemetryMode::Summary,
            target: crate::process_loopback::ProcessIdentity {
                pid: 0,
                creation_time_100ns: 0,
                executable_name: String::new(),
            },
            duration: crate::SessionDuration::UntilStopped,
        },
        stop,
        gate,
        || {
            calls += 1;
            7
        },
    )
    .err()
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(failure.error, CaptureError::InvalidProcess);
    assert!(failure.report.is_none());
    assert_eq!(failure.abort, Some(7));
    let failure = start_owner_with_spawn(
        StopSignal::new().unwrap(),
        |_, _, _| panic!("must not spawn"),
        || 8,
        true,
        |_| Err(std::io::Error::from_raw_os_error(8)),
    )
    .err()
    .unwrap();
    assert_eq!(failure.error, CaptureError::WorkerSpawn { code: Some(8) });
    assert!(failure.report.is_none());
    assert_eq!(failure.abort, Some(8));
    assert_eq!(failure.join_error, None);
}

#[test]
fn capture_abort_guard_runs_before_reap_during_control_unwind() {
    let (abort, aborted) = mpsc::channel();
    let (done, completed) = mpsc::channel();
    let worker = thread::spawn(move || {
        aborted
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        done.send(()).unwrap();
        crate::capture::CaptureReport::new(
            crate::capture::CaptureSource::ProcessLoopback,
            crate::SessionDuration::UntilStopped,
        )
    });
    let stop = StopSignal::new().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = CaptureAbortGuard {
            stop_without_owner: true,
            stop: stop.clone(),
            session: Some(CaptureSession {
                stop: stop.event.clone(),
                started: Arc::new(AtomicBool::new(false)),
                worker: Some(worker),
            }),
            before_join: Some(|| {
                abort.send(()).unwrap();
                panic!("peer callback panicked too")
            }),
        };
        panic!("injected control unwind");
    }));
    assert!(result.is_err());
    completed.try_recv().unwrap();
}

#[test]
fn capture_abort_legacy_validation_and_no_owner_spawn_leave_stop_untouched() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(StopSignal::new().unwrap()).unwrap();
    let options = crate::process_loopback::ProcessLoopbackOptions {
        artifact: None,
        bridge: Default::default(),
        latency_telemetry: crate::latency::TelemetryMode::Summary,
        target: crate::process_loopback::ProcessIdentity {
            pid: 1,
            creation_time_100ns: 1,
            executable_name: "unopened".into(),
        },
        duration: crate::SessionDuration::UntilStopped,
    };
    assert!(
        crate::process_loopback::prepare_process_capture_with_gate(options, stop.clone(), gate)
            .is_err()
    );
    assert_eq!(
        unsafe { windows::Win32::System::Threading::WaitForSingleObject(handle(&stop.event), 0) },
        windows::Win32::Foundation::WAIT_TIMEOUT
    );
    let failure = start_owner_with_spawn(
        stop.clone(),
        |_, _, _| panic!("no owner"),
        || (),
        false,
        |_| Err(std::io::Error::from_raw_os_error(8)),
    )
    .err()
    .unwrap();
    assert!(matches!(failure.error, CaptureError::WorkerSpawn { .. }));
    assert_eq!(
        unsafe { windows::Win32::System::Threading::WaitForSingleObject(handle(&stop.event), 0) },
        windows::Win32::Foundation::WAIT_TIMEOUT
    );
}

#[test]
fn capture_abort_process_panic_report_does_not_convert_legacy_join_to_success() {
    for (after_ready, retaining_join) in [(false, false), (true, false), (true, true)] {
        let result = start_owner_returning_with_abort(
            StopSignal::new().unwrap(),
            move |stop, sender, _| {
                let report = crate::capture::CaptureReport::new(
                    crate::capture::CaptureSource::ProcessLoopback,
                    crate::SessionDuration::UntilStopped,
                );
                finish_owner_retaining_report(report, &stop, &sender, |report| {
                    report.activation_seconds = Some(0.375);
                    if after_ready {
                        report.stream_started = true;
                        report.packets = 7;
                        let (_, source, observer) =
                            crate::clock_bridge::capture_bridge(Default::default()).unwrap();
                        sender
                            .send(Ok(Startup {
                                source,
                                observer,
                                sample_rate: 48_000,
                                channels: 2,
                            }))
                            .ok()
                            .unwrap();
                    }
                    panic!("process owner panic");
                })
            },
            || (),
        );
        if after_ready {
            let prepared = result.unwrap();
            if retaining_join {
                let (report, error) = prepared.session.join_retaining_report();
                assert_eq!(error, Some(CaptureError::WorkerPanicked));
                let report = report.expect("post-ready panic report must be retained");
                assert_eq!(report.activation_seconds, Some(0.375));
                assert!(report.stream_started);
                assert_eq!(report.packets, 7);
                assert_eq!(
                    report.failure.as_deref(),
                    Some(CaptureError::WorkerPanicked.to_string().as_str())
                );
            } else {
                assert!(matches!(
                    prepared.session.join(),
                    Err(CaptureError::WorkerPanicked)
                ));
            }
        } else {
            let failure = result.err().unwrap();
            assert_eq!(failure.error, CaptureError::WorkerPanicked);
            assert_eq!(failure.join_error, Some(CaptureError::WorkerPanicked));
            let report = failure.report.unwrap();
            assert_eq!(report.activation_seconds, Some(0.375));
            assert_eq!(
                report.failure.as_deref(),
                Some(CaptureError::WorkerPanicked.to_string().as_str())
            );
        }
    }
}

#[test]
fn capture_join_retaining_report_preserves_successful_owner_report() {
    let prepared = start_owner_returning_with_abort(
        StopSignal::new().unwrap(),
        |_, sender, _| {
            let (_, source, observer) = capture_bridge(Default::default()).unwrap();
            sender
                .send(Ok(Startup {
                    source,
                    observer,
                    sample_rate: 48_000,
                    channels: 2,
                }))
                .ok()
                .unwrap();
            let mut report = CaptureReport::new(
                crate::capture::CaptureSource::ProcessLoopback,
                crate::SessionDuration::UntilStopped,
            );
            report.activation_seconds = Some(0.625);
            report.packets = 11;
            report.status = CaptureStatus::Stopped;
            report.stop_succeeded = true;
            report
        },
        || panic!("successful preparation must not abort"),
    )
    .unwrap();
    let (report, error) = prepared.session.join_retaining_report();
    assert_eq!(error, None);
    let report = report.expect("successful owner report must be retained");
    assert_eq!(report.activation_seconds, Some(0.625));
    assert_eq!(report.packets, 11);
    assert_eq!(report.status, CaptureStatus::Stopped);
    assert!(report.stop_succeeded);
    assert!(report.failure.is_none());
}

#[test]
fn capture_join_retaining_report_does_not_fabricate_report_for_unrelated_panic() {
    let prepared = start_owner_returning_with_abort(
        StopSignal::new().unwrap(),
        |_, sender, _| {
            let (_, source, observer) = capture_bridge(Default::default()).unwrap();
            sender
                .send(Ok(Startup {
                    source,
                    observer,
                    sample_rate: 48_000,
                    channels: 2,
                }))
                .ok()
                .unwrap();
            std::panic::panic_any(42_u64);
        },
        || panic!("post-ready panic must not become preparation failure"),
    )
    .unwrap();
    let (report, error) = prepared.session.join_retaining_report();
    assert!(report.is_none());
    assert_eq!(error, Some(CaptureError::WorkerPanicked));
}

#[test]
fn capture_abort_success_never_calls_callback_and_legacy_panic_precedence_is_retained() {
    let prepared = start_owner_returning_with_abort(
        StopSignal::new().unwrap(),
        |_, sender, _| {
            let (ingress, source, observer) =
                crate::clock_bridge::capture_bridge(Default::default()).unwrap();
            sender
                .send(Ok(Startup {
                    source,
                    observer,
                    sample_rate: 48_000,
                    channels: 2,
                }))
                .ok()
                .unwrap();
            drop(ingress);
            crate::capture::CaptureReport::new(
                crate::capture::CaptureSource::ProcessLoopback,
                crate::SessionDuration::UntilStopped,
            )
        },
        || panic!("success must not abort"),
    );
    let prepared = prepared.unwrap();
    prepared.session.join().unwrap();
    let legacy = start_owner(StopSignal::new().unwrap(), |_, _, _| {
        panic!("legacy worker panic")
    });
    assert!(matches!(legacy, Err(CaptureError::WorkerPanicked)));
    let legacy = start_owner(StopSignal::new().unwrap(), |_, sender, _| {
        sender.send(Err(CaptureError::StartupLost)).unwrap();
        panic!("explicit startup error must win over later join panic");
    });
    assert!(matches!(legacy, Err(CaptureError::StartupLost)));
}

#[test]
fn capture_abort_stop_failure_and_join_panic_do_not_replace_primary() {
    use std::os::windows::io::FromRawHandle;
    // A real mutex handle is owned and valid, but cannot be signaled by SetEvent.
    let mutex =
        unsafe { windows::Win32::System::Threading::CreateMutexW(None, false, None) }.unwrap();
    let stop = StopSignal {
        event: Arc::new(unsafe { StdHandle::from_raw_handle(mutex.0) }),
    };
    let failure = start_owner_returning_with_abort(
        stop,
        |_, sender, _| {
            sender.send(Err(CaptureError::TargetExited)).unwrap();
            panic!("join panic after primary error");
        },
        || Err::<(), _>("both peers attempted"),
    )
    .err()
    .unwrap();
    assert_eq!(failure.error, CaptureError::TargetExited);
    assert!(matches!(
        failure.stop_error,
        Some(CaptureError::Api {
            stage: "SetEvent(capture stop)",
            ..
        })
    ));
    assert_eq!(failure.join_error, Some(CaptureError::WorkerPanicked));
    assert_eq!(failure.abort, Some(Err("both peers attempted")));
    assert!(failure.report.is_none());
}

#[test]
fn capture_abort_lost_handshake_callback_precedes_reap_even_when_callback_panics() {
    let (abort, aborted) = mpsc::channel();
    let (done, completed) = mpsc::channel();
    let failure = start_owner_returning_with_abort(
        StopSignal::new().unwrap(),
        move |_, sender, _| {
            drop(sender);
            aborted
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            done.send(()).unwrap();
            crate::capture::CaptureReport::new(
                crate::capture::CaptureSource::ProcessLoopback,
                crate::SessionDuration::UntilStopped,
            )
        },
        || {
            abort.send(()).unwrap();
            panic!("abort panic after signaling peers")
        },
    )
    .err()
    .unwrap();
    assert_eq!(failure.error, CaptureError::StartupLost);
    assert!(failure.abort_panicked);
    assert!(failure.report.is_some());
    completed.try_recv().unwrap();
}
