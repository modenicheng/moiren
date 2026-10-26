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
