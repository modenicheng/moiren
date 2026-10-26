use super::*;
use crate::{Activation, SessionDuration, capture::CaptureSource};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Run,
    Fail,
    Panic,
    LostMetadata,
}
fn report() -> CaptureReport {
    CaptureReport::new(CaptureSource::Physical, SessionDuration::UntilStopped)
}

#[test]
fn configured_process_preparation_changes_real_priming_and_reports_the_attached_queue() {
    use crate::{
        clock_bridge::CapturePacket,
        latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
        process_loopback::ProcessBridgeOptions,
    };
    let options = ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 8192,
            target_frames: 1024,
            jitter_frames: 96,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 480,
            max_gap_frames: 480,
            source: DeliveryTimingSource::Declared,
        }),
        ..Default::default()
    };
    let stop = StopSignal::new().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let preparation = CapturePreparation::Configured {
        sender,
        config: options.clock_config().unwrap(),
    };
    let mut report = CaptureReport::new(
        CaptureSource::ProcessLoopback,
        SessionDuration::UntilStopped,
    );
    let mut ingress = preparation
        .prepare(
            NativeStreamTiming {
                sample_rate: 48_000,
                period_frames: None,
                capacity_frames: 480,
            },
            2,
            &stop.event,
            &mut report,
            None,
        )
        .unwrap();
    let startup = receiver.recv().unwrap().unwrap();
    let mut source = startup.source;
    assert_eq!(source.prepared_input(), Some(options.prepare().unwrap()));
    assert_eq!(report.input_queue, source.prepared_input());
    assert_eq!(source.prepared_input().unwrap().capacity_frames, 8192);
    // Three 480-frame publications cross 1024 but do not reach the old 2048
    // threshold. Passing this requires the actual reader to use the new target.
    let pcm: Vec<u8> = (0..480)
        .flat_map(|_| [0.25f32, -0.5].into_iter().flat_map(f32::to_le_bytes))
        .collect();
    for index in 0..3 {
        ingress
            .push_packet(
                &pcm,
                CapturePacket {
                    frames: 480,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 1 + index * 100_000,
                },
            )
            .unwrap();
    }
    let mut output = [0.0; 2];
    assert_eq!(
        source
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(output, [0.25, -0.5]);
    assert_eq!(startup.observer.snapshot().discontinuities, 0);
    assert_eq!(report.shared_period.actual_period_frames, None);
}

#[test]
fn compatible_process_bridge_retains_legacy_fifo_without_running_backlog_recovery() {
    let options = crate::process_loopback::ProcessBridgeOptions::default();
    let config = options.clock_config().unwrap();
    assert_eq!(config.capacity_frames, 8192);
    assert_eq!(config.target_fill_frames, 2048);
    assert_eq!(config.prepared_input, None);
    assert!(!config.detect_position_gaps);
    let (_, source, _) = crate::clock_bridge::capture_bridge(config).unwrap();
    assert_eq!(source.prepared_input(), None);
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

#[test]
fn converted_capture_uses_client_packets_without_inferring_native_position_gaps() {
    use crate::{
        clock_bridge::CapturePacket,
        shared_stream::{SharedFormatInfo, SharedPeriodReport},
        stats::{DISCONTINUITY, SILENT},
    };
    for deferred in [false, true] {
        let stop = StopSignal::new().unwrap();
        let mut report = report();
        report.shared_period = SharedPeriodReport {
            mix_format: Some(SharedFormatInfo {
                sample_rate: 96_000,
                channels: 2,
                container_bits: 32,
                valid_bits: None,
                channel_mask: None,
                floating_point: true,
            }),
            windows_auto_conversion: true,
            ..Default::default()
        };
        let native_timing = NativeStreamTiming {
            sample_rate: 96_000,
            ..timing()
        };
        assert!(client_position_clock(native_timing, report.shared_period));
        let (mut ingress, _source, observer) = if deferred {
            let (tx, rx) = mpsc::sync_channel(1);
            let prepared = start_native_owner(stop, move |stop, preparation, _| {
                let ingress = preparation
                    .prepare(timing(), 2, &stop, &mut report, None)
                    .unwrap();
                // Return the producer only in this test so both bridge setup paths
                // can exercise identical packets without touching a real device.
                tx.send(ingress).unwrap();
                preparation.finish(Ok(CaptureStatus::Stopped), &stop, report)
            })
            .unwrap();
            let capture = prepared.attach(input()).unwrap();
            let ingress = rx.recv().unwrap();
            capture.session.join().unwrap();
            (ingress, capture.source, capture.observer)
        } else {
            let (sender, receiver) = mpsc::sync_channel(1);
            let ingress = CapturePreparation::Immediate(sender)
                .prepare(timing(), 2, &stop.event, &mut report, None)
                .unwrap();
            let startup = receiver.recv().unwrap().unwrap();
            (ingress, startup.source, startup.observer)
        };
        // Client frames advance by 480, native positions by 960. Their ratio
        // must not create a discontinuity on every otherwise continuous packet.
        for index in 0..3 {
            ingress
                .push_packet(
                    &[],
                    CapturePacket {
                        frames: 480,
                        flags: SILENT,
                        device_position_frames: index * 960,
                        qpc_100ns: 1_000_000 + index * 100_000,
                    },
                )
                .unwrap();
        }
        assert_eq!(observer.snapshot().discontinuities, 0);
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 480,
                    flags: SILENT | DISCONTINUITY,
                    device_position_frames: 2880,
                    qpc_100ns: 1_300_000,
                },
            )
            .unwrap();
        assert_eq!(observer.snapshot().discontinuities, 1);
    }
}
pub(super) fn fake_prepared(
    stop: StopSignal,
    gate: ActivationGate,
    outcome: Outcome,
    exited: Arc<AtomicBool>,
) -> Result<PreparedNativeCapture, CaptureError> {
    start_native_owner(stop, move |stop, preparation, started| {
        let mut report = report();
        report.sample_rate = Some(timing().sample_rate);
        report.endpoint_id = Some("fake-capture".into());
        report.channels = Some(2);
        report.buffer_frames = Some(timing().capacity_frames);
        let result = (|| {
            match outcome {
                Outcome::Fail => return Err(CaptureError::UnsupportedFormat),
                Outcome::Panic => panic!("native capture preparation panic"),
                Outcome::LostMetadata => return Ok(CaptureStatus::Stopped),
                Outcome::Run => {}
            }
            let _ingress = preparation.prepare(timing(), 2, &stop, &mut report, None)?;
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
fn deferred_capture_adapts_native_frames_to_the_requested_engine_rate() {
    for engine_rate in [44_100_u32, 48_000, 88_200, 96_000] {
        let stop = StopSignal::new().unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let prepared = start_native_owner(stop, move |stop, preparation, _| {
            let mut report = report();
            let ingress = preparation
                .prepare(timing(), 2, &stop, &mut report, None)
                .unwrap();
            sender.send(ingress).unwrap();
            preparation.finish(Ok(CaptureStatus::Stopped), &stop, report)
        })
        .unwrap();
        let mut capture = prepared
            .attach_returning(input(), engine_rate)
            .unwrap_or_else(|failure| panic!("{}", failure.error));
        let mut ingress = receiver.recv().unwrap();
        capture.session.join().unwrap();
        let pcm: Vec<_> = (0..2048)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .flat_map(f32::to_le_bytes)
            .collect();
        ingress
            .push_packet(
                &pcm,
                crate::clock_bridge::CapturePacket {
                    frames: 2048,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 1,
                },
            )
            .unwrap();
        let mut output = [f32::NAN; 32];
        assert_eq!(
            capture
                .source
                .read_interleaved(&mut output)
                .unwrap()
                .transferred_frames,
            16
        );
        let step = 48_000.0 / engine_rate as f32;
        for frame in 1..16 {
            assert!((output[frame * 2] - output[0] - frame as f32 * step).abs() < 0.001);
            assert_eq!(output[frame * 2 + 1], -output[frame * 2]);
        }
    }
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

fn evidence_config() -> PcmEvidenceConfig {
    PcmEvidenceConfig {
        capacity_frames: 4,
        detect_position_gaps: true,
    }
}

fn fake_evidence_prepared(
    stop: StopSignal,
    gate: ActivationGate,
    channels: usize,
    mix_rate: Option<u32>,
    exited: Arc<AtomicBool>,
    fail_after_attachment: bool,
) -> PreparedNativeCapture {
    start_native_owner(stop, move |stop, preparation, started| {
        let native_timing = NativeStreamTiming {
            sample_rate: 44_100,
            ..timing()
        };
        let mut report = report();
        report.sample_rate = Some(native_timing.sample_rate);
        report.channels = Some(channels);
        report.shared_period.mix_format =
            mix_rate.map(|sample_rate| crate::shared_stream::SharedFormatInfo {
                sample_rate,
                channels: channels as u16,
                container_bits: 32,
                valid_bits: None,
                channel_mask: None,
                floating_point: true,
            });
        report.shared_period.windows_auto_conversion =
            mix_rate.is_some_and(|rate| rate != native_timing.sample_rate);
        let result = (|| {
            let _ingress =
                preparation.prepare(native_timing, channels, &stop, &mut report, None)?;
            if gate.wait()? == Activation::Cancelled {
                return Ok(CaptureStatus::Stopped);
            }
            report.stream_started = true;
            started.store(true, Ordering::Release);
            let writer = report
                .evidence_recorder
                .as_mut()
                .expect("real deferred attachment must transfer the evidence writer");
            let mut pcm = [0; 8];
            pcm[..4].copy_from_slice(&0.25_f32.to_le_bytes());
            pcm[4..].copy_from_slice(&(-0.5_f32).to_le_bytes());
            for (index, position) in [10, 14].into_iter().enumerate() {
                assert_eq!(
                    writer.record_packet(
                        &pcm[..channels * 4],
                        crate::capture::PcmEvidencePacket {
                            frames: 1,
                            flags: 0,
                            device_position_frames: position,
                            qpc_100ns: 1000 + index as u64 * 100,
                        },
                    )?,
                    1
                );
            }
            if fail_after_attachment {
                Err(CaptureError::InvalidPacket)
            } else {
                Ok(CaptureStatus::Completed)
            }
        })();
        let report = preparation.finish(result, &stop, report);
        exited.store(true, Ordering::Release);
        report
    })
    .unwrap()
}

fn assert_closed_report_writer(report: &mut CaptureReport) {
    assert_eq!(
        report
            .evidence_recorder
            .as_mut()
            .expect("join must return the still-owned recorder, not only its summary")
            .record_packet(
                &[],
                crate::capture::PcmEvidencePacket {
                    frames: 1,
                    flags: crate::stats::SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 1,
                },
            ),
        Err(crate::capture::PcmEvidenceError::Closed)
    );
}

#[test]
fn deferred_capture_evidence_negotiates_mono_stereo_and_frame_units_on_start_or_cancel() {
    for channels in [1, 2] {
        // Unknown/equal mix clocks retain explicit opt-in; SRC frame-unit
        // mismatch must revoke it even when the caller requested detection.
        for (mix_rate, compatible_units) in
            [(None, true), (Some(44_100), true), (Some(88_200), false)]
        {
            for detect_position_gaps in [false, true] {
                for cancel in [false, true] {
                    let stop = StopSignal::new().unwrap();
                    let gate = ActivationGate::new(stop.clone()).unwrap();
                    let exited = Arc::new(AtomicBool::new(false));
                    let prepared = fake_evidence_prepared(
                        stop.clone(),
                        gate.clone(),
                        channels,
                        mix_rate,
                        exited.clone(),
                        false,
                    );
                    assert_eq!(prepared.info().timing.sample_rate, 44_100);
                    assert_eq!(prepared.info().channels, channels);
                    let (capture, mut reader) = prepared
                        .attach_with_evidence(
                            input(),
                            PcmEvidenceConfig {
                                detect_position_gaps,
                                ..evidence_config()
                            },
                        )
                        .unwrap();
                    assert_eq!(capture.sample_rate, 44_100);
                    assert_eq!(capture.channels, channels);
                    assert_eq!(capture.source.prepared_input(), Some(input()));
                    assert!(!capture.session.has_started());
                    assert!(reader.pop().is_none());
                    assert!(!reader.is_finished());
                    if cancel {
                        gate.cancel().unwrap();
                    } else {
                        gate.activate().unwrap();
                    }
                    let mut report = capture.session.join().unwrap();
                    assert!(exited.load(Ordering::Acquire));
                    assert_stopped(&stop);
                    assert_eq!(report.stream_started, !cancel);
                    assert_eq!(
                        report.status,
                        if cancel {
                            CaptureStatus::Stopped
                        } else {
                            CaptureStatus::Completed
                        }
                    );
                    assert!(report.failure.is_none());
                    assert_eq!(report.sample_rate, Some(44_100));
                    assert_eq!(report.channels, Some(channels));
                    assert_eq!(report.input_queue, Some(input()));
                    let metadata = report.evidence_metadata.unwrap();
                    assert_eq!(metadata.sample_rate, 44_100);
                    assert_eq!(metadata.channels, channels);
                    assert_eq!(metadata.capacity_frames, 4);
                    assert_eq!(
                        metadata.detect_position_gaps,
                        detect_position_gaps && compatible_units
                    );
                    assert_eq!(metadata.packets, if cancel { 0 } else { 2 });
                    assert_eq!(metadata.observed_frames, if cancel { 0 } else { 2 });
                    assert_eq!(metadata.stored_frames, if cancel { 0 } else { 2 });
                    assert_eq!(metadata.dropped_frames, 0);
                    let infer_gap = !cancel && detect_position_gaps && compatible_units;
                    assert_eq!(metadata.gap_events, if infer_gap { 1 } else { 0 });
                    assert_eq!(metadata.gap_frames, if infer_gap { 3 } else { 0 });
                    assert_closed_report_writer(&mut report);
                    assert_eq!(reader.is_finished(), cancel);
                    if !cancel {
                        for (sequence, position, qpc) in [(1, 10, 1000), (2, 14, 1100)] {
                            let frame = reader.pop().unwrap();
                            assert_eq!(frame.samples[0].to_bits(), 0.25_f32.to_bits());
                            assert_eq!(
                                frame.samples[1].to_bits(),
                                if channels == 2 {
                                    (-0.5_f32).to_bits()
                                } else {
                                    0
                                }
                            );
                            assert_eq!(frame.packet_sequence, sequence);
                            assert_eq!(frame.packet_frame_offset, 0);
                            assert_eq!(frame.packet_frames, 1);
                            assert_eq!(frame.device_position_frames, position);
                            assert_eq!(frame.qpc_100ns, qpc);
                        }
                    }
                    assert!(reader.pop().is_none());
                    assert!(
                        reader.is_finished(),
                        "closure must not require dropping the report"
                    );
                    assert!(report.evidence_recorder.is_some());
                }
            }
        }
    }
}

#[test]
fn deferred_capture_evidence_invalid_config_or_queue_joins_and_cancels_owner() {
    use crate::capture::PcmEvidenceError;

    for (capacity_frames, queue, expected) in [
        (
            0,
            input(),
            CaptureError::PcmEvidence(PcmEvidenceError::InvalidConfig),
        ),
        (
            usize::MAX,
            input(),
            CaptureError::PcmEvidence(PcmEvidenceError::StorageBudget),
        ),
        (
            4,
            PreparedInputQueue {
                prime_frames: 913,
                ..input()
            },
            CaptureError::Bridge(crate::clock_bridge::ClockBridgeError::InvalidConfig),
        ),
    ] {
        let stop = StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        let exited = Arc::new(AtomicBool::new(false));
        let prepared =
            fake_evidence_prepared(stop.clone(), gate.clone(), 2, None, exited.clone(), false);
        let started = prepared.session.started.clone();
        assert_eq!(
            prepared
                .attach_with_evidence(
                    queue,
                    PcmEvidenceConfig {
                        capacity_frames,
                        ..evidence_config()
                    },
                )
                .err()
                .expect("invalid attachment must fail"),
            expected
        );
        assert!(
            exited.load(Ordering::Acquire),
            "failure must join, not detach the owner"
        );
        assert!(!started.load(Ordering::Acquire));
        assert_stopped(&stop);
        assert_eq!(gate.wait().unwrap(), Activation::Cancelled);
    }
}

#[test]
fn deferred_capture_evidence_invalid_queue_keeps_untransferred_writer_on_control() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let exited = Arc::new(AtomicBool::new(false));
    let prepared = fake_evidence_prepared(stop.clone(), gate, 1, None, exited.clone(), false);
    let (writer, reader) = pcm_evidence_stream(evidence_config(), 44_100, 1).unwrap();
    let handoff = Arc::new(Mutex::new(Some(writer)));
    let failure = prepared
        .attach_with_handoff(
            PreparedInputQueue {
                prime_frames: 913,
                ..input()
            },
            crate::NATIVE_SAMPLE_RATE,
            Some(handoff.clone()),
        )
        .err()
        .expect("invalid queue must fail before handoff");
    assert_eq!(
        failure.error,
        CaptureError::Bridge(crate::clock_bridge::ClockBridgeError::InvalidConfig)
    );
    let report = failure
        .report
        .expect("queue rejection must retain the joined native report");
    assert!(exited.load(Ordering::Acquire));
    assert_stopped(&stop);
    assert!(!report.stream_started);
    assert!(report.evidence_recorder.is_none());
    assert!(report.evidence_metadata.is_none());
    assert_eq!(Arc::strong_count(&handoff), 1);
    assert!(handoff.lock().unwrap().is_some());
    assert!(
        !reader.is_finished(),
        "untransferred writer still belongs to control"
    );
    drop(report);
    assert!(!reader.is_finished());
    drop(handoff);
    assert!(reader.is_finished());
}

#[test]
fn deferred_capture_evidence_finish_retains_queued_writer_through_failure_or_cancel_join() {
    for fail in [false, true] {
        let stop = StopSignal::new().unwrap();
        let (sender, receiver) = preparation::channel().unwrap();
        let (ready, _metadata) = mpsc::sync_channel(1);
        let (attached, _attachment) = mpsc::sync_channel(1);
        let (mut writer, mut reader) = pcm_evidence_stream(evidence_config(), 48_000, 1).unwrap();
        writer
            .record_packet(
                &0.25_f32.to_le_bytes(),
                crate::capture::PcmEvidencePacket {
                    frames: 1,
                    flags: 0,
                    device_position_frames: 10,
                    qpc_100ns: 1000,
                },
            )
            .unwrap();
        let handoff = Arc::new(Mutex::new(Some(writer)));
        let (ingress, _source, _observer) = capture_bridge_with_clock(
            ClockBridgeConfig::with_engine_input(48_000, 48_000, 1, input()),
            None,
        )
        .unwrap();
        sender
            .send((ingress, input(), Some(handoff.clone())))
            .unwrap();
        stop.request_stop().unwrap();
        assert!(
            receiver.receive(&stop.event).unwrap().is_none(),
            "stop must beat the queued attachment"
        );
        let preparation = CapturePreparation::Deferred {
            ready,
            bridge: receiver,
            attached,
        };
        let worker_stop = stop.event.clone();
        let worker = thread::spawn(move || {
            preparation.finish(
                if fail {
                    Err(CaptureError::InvalidPacket)
                } else {
                    Ok(CaptureStatus::Stopped)
                },
                &worker_stop,
                report(),
            )
        });
        let session = CaptureSession {
            started: Arc::new(AtomicBool::new(false)),
            stop: stop.event.clone(),
            worker: Some(worker),
        };
        let mut report = session.join().unwrap();
        assert_stopped(&stop);
        assert!(
            handoff.lock().unwrap().is_none(),
            "finish must recover the queued writer into the report"
        );
        assert!(!report.stream_started);
        assert!(report.input_queue.is_none());
        assert_eq!(
            report.status,
            if fail {
                CaptureStatus::Failed
            } else {
                CaptureStatus::Stopped
            }
        );
        assert_eq!(
            report.failure,
            fail.then(|| CaptureError::InvalidPacket.to_string())
        );
        assert_eq!(report.evidence_metadata.unwrap().stored_frames, 1);
        assert_closed_report_writer(&mut report);
        assert!(
            !reader.is_finished(),
            "closed queued evidence must remain drainable"
        );
        assert_eq!(
            reader.pop().unwrap().samples[0].to_bits(),
            0.25_f32.to_bits()
        );
        assert!(reader.pop().is_none());
        assert!(reader.is_finished());
        assert!(report.evidence_recorder.is_some());
    }
}

#[test]
fn deferred_capture_evidence_closed_handoff_keeps_late_writer_on_control_after_join() {
    let stop = StopSignal::new().unwrap();
    let (finished, completion) = mpsc::sync_channel(1);
    let prepared = start_native_owner(stop.clone(), move |stop, preparation, _| {
        let CapturePreparation::Deferred { ready, .. } = &preparation else {
            unreachable!()
        };
        ready
            .send(Ok(NativeCaptureInfo {
                timing: timing(),
                channels: 2,
                shared_period: Default::default(),
                age_clock: None,
            }))
            .unwrap();
        let report = preparation.finish(Err(CaptureError::InvalidPacket), &stop, report());
        finished.send(()).unwrap();
        report
    })
    .unwrap();
    // Deterministically close the owner mailbox before attempting a late send.
    completion
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    let (writer, reader) = pcm_evidence_stream(evidence_config(), 48_000, 2).unwrap();
    let handoff = Arc::new(Mutex::new(Some(writer)));
    let failure = prepared
        .attach_with_handoff(input(), 48_000, Some(handoff.clone()))
        .err()
        .expect("closed native mailbox must reject the late attachment");
    assert_eq!(
        failure.error,
        CaptureError::Api {
            stage: "Send capture bridge",
            hresult: windows::Win32::Foundation::E_ABORT.0,
        }
    );
    let report = failure
        .report
        .expect("late handoff failure must join and retain the native report");
    assert_stopped(&stop);
    assert_eq!(
        report.failure,
        Some(CaptureError::InvalidPacket.to_string())
    );
    assert!(report.evidence_recorder.is_none());
    assert!(report.evidence_metadata.is_none());
    assert_eq!(Arc::strong_count(&handoff), 1);
    assert!(handoff.lock().unwrap().is_some());
    assert!(!reader.is_finished());
    drop(report);
    assert!(
        !reader.is_finished(),
        "the failed send must not drop the control-owned writer"
    );
    drop(handoff);
    assert!(reader.is_finished());
}

#[test]
fn deferred_capture_evidence_attached_owner_failure_closes_reader_and_returns_recorder() {
    let stop = StopSignal::new().unwrap();
    let gate = ActivationGate::new(stop.clone()).unwrap();
    let exited = Arc::new(AtomicBool::new(false));
    let prepared =
        fake_evidence_prepared(stop.clone(), gate.clone(), 2, None, exited.clone(), true);
    let (capture, mut reader) = prepared
        .attach_with_evidence(input(), evidence_config())
        .unwrap();
    assert!(!capture.session.has_started());
    assert!(!reader.is_finished());
    gate.activate().unwrap();
    let mut report = capture.session.join().unwrap();
    assert!(exited.load(Ordering::Acquire));
    assert_stopped(&stop);
    assert!(report.stream_started);
    assert_eq!(report.status, CaptureStatus::Failed);
    assert_eq!(
        report.failure,
        Some(CaptureError::InvalidPacket.to_string())
    );
    assert_eq!(report.input_queue, Some(input()));
    let metadata = report.evidence_metadata.unwrap();
    assert_eq!(metadata.channels, 2);
    assert_eq!(metadata.sample_rate, 44_100);
    assert_eq!(metadata.stored_frames, 2);
    assert_closed_report_writer(&mut report);
    assert!(
        !reader.is_finished(),
        "failure must close without discarding pending frames"
    );
    assert_eq!(reader.pop().unwrap().packet_sequence, 1);
    assert_eq!(reader.pop().unwrap().packet_sequence, 2);
    assert!(reader.pop().is_none());
    assert!(
        reader.is_finished(),
        "failure closure must not depend on dropping the report"
    );
    assert!(report.evidence_recorder.is_some());
}
