//! Native service/bridge lifecycle shared by physical and virtual capture.
#[cfg(test)]
#[path = "stream_live_tests.rs"]
mod live_tests;
use super::{
    Startup, Wake, api, handle, packet::transfer_packet, preparation::CapturePreparation,
    wait_target,
};
use crate::{
    Activation, ActivationGate, SessionDuration,
    capture::{CaptureBufferSizeQuery, CaptureError, CaptureReport, CaptureStatus},
    latency::{
        CallbackObservation, InputDeliveryRecorder, LatencyRecorder, TelemetryMode, clock::QpcClock,
    },
    owner::{Mmcss, Streaming},
};
use std::{
    os::windows::io::OwnedHandle,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, S_OK},
        Media::Audio::{IAudioCaptureClient, IAudioClient},
        System::Threading::SetEvent,
    },
    core::Interface,
};

pub(crate) struct StreamInput<'a> {
    pub client: &'a IAudioClient,
    pub audio: HANDLE,
    pub target: Option<HANDLE>,
    pub stop: &'a OwnedHandle,
    pub duration: SessionDuration,
    pub latency_telemetry: TelemetryMode,
    pub gate: &'a ActivationGate,
    pub started: &'a AtomicBool,
    pub sample_rate: u32,
    pub channels: usize,
    pub artifact: Option<crate::capture::PcmArtifactConfig>,
}
pub(crate) fn run(
    input: StreamInput<'_>,
    sender: &SyncSender<Result<Startup, CaptureError>>,
    report: &mut CaptureReport,
    bridge: crate::clock_bridge::ClockBridgeConfig,
) -> Result<CaptureStatus, CaptureError> {
    run_prepared(
        input,
        &CapturePreparation::Configured {
            sender: sender.clone(),
            config: bridge,
        },
        report,
    )
}
pub(super) fn run_prepared(
    input: StreamInput<'_>,
    preparation: &CapturePreparation,
    report: &mut CaptureReport,
) -> Result<CaptureStatus, CaptureError> {
    // The generated binding hides nonnegative HRESULTs and zeroes its output.
    // Call the same ABI directly once so a refused Process client preserves
    // both the status and the output instead of looking like a budget choice.
    // The poison is deliberately outside the unchanged 8 MiB safety bound.
    let initial_frames = u32::MAX;
    let mut capacity = initial_frames;
    // SAFETY: this live owner-local client supplies its IAudioClient vtable;
    // capacity is a valid UINT32 output for the duration of this one call.
    let status =
        unsafe { (input.client.vtable().GetBufferSize)(input.client.as_raw(), &mut capacity) };
    let query = CaptureBufferSizeQuery {
        hresult: status.0,
        initial_frames,
        raw_frames: capacity,
        output_changed: capacity != initial_frames,
        raw_bytes: u64::try_from(input.channels)
            .ok()
            .and_then(|channels| channels.checked_mul(4))
            .and_then(|frame_bytes| u64::from(capacity).checked_mul(frame_bytes)),
    };
    report.buffer_size_query = Some(query);
    if status.is_err() {
        return Err(CaptureError::Api {
            stage: "GetBufferSize(capture)",
            hresult: status.0,
        });
    }
    if status != S_OK
        || !query.output_changed
        || capacity == 0
        || query
            .raw_bytes
            .is_none_or(|bytes| bytes == 0 || bytes > 8 * 1024 * 1024)
    {
        return Err(CaptureError::InvalidBufferSize {
            hresult: query.hresult,
            raw_frames: query.raw_frames,
            raw_bytes: query.raw_bytes,
            output_changed: query.output_changed,
        });
    }
    report.buffer_frames = Some(capacity);
    report.stream_latency_100ns = unsafe { input.client.GetStreamLatency() }.ok();
    let mut telemetry = LatencyRecorder::new(input.latency_telemetry);
    // Scalar live observations need QPC even when trace collection is disabled.
    // Preparing frequency stays off RT; the recorder's selected mode is intact.
    let clock = QpcClock::prepare(TelemetryMode::Summary);
    let mut delivery = InputDeliveryRecorder::new(report.delivery_bound)?;
    report.sample_rate = Some(input.sample_rate);
    report.channels = Some(input.channels);
    let capture: IAudioCaptureClient =
        api("GetService(capture)", unsafe { input.client.GetService() })?;
    // Check stop/target again after preparation, before Start or publishing source.
    match wait_target(handle(input.stop), input.audio, input.target, 0)? {
        Wake::Stop => return Err(CaptureError::Cancelled),
        Wake::TargetExited => return Err(CaptureError::TargetExited),
        Wake::Audio | Wake::Timeout => {}
    }
    // Allocate bounded evidence before publishing the startup bridge. A
    // rejected artifact configuration must never look like a live capture to
    // the control side.
    report.prepare_artifact(input.artifact, input.sample_rate, input.channels)?;
    let mut ingress = preparation.prepare(
        crate::latency::NativeStreamTiming {
            sample_rate: input.sample_rate,
            period_frames: report.shared_period.actual_period_frames,
            capacity_frames: capacity,
        },
        input.channels,
        report,
        clock,
    )?;
    let activation = input.gate.wait_with_target(input.target)?;
    // A process can exit during preparation. Stop still wins over that exit.
    match wait_target(handle(input.stop), input.audio, input.target, 0)? {
        Wake::Stop => return Ok(CaptureStatus::Stopped),
        Wake::TargetExited => return Ok(CaptureStatus::TargetExited),
        Wake::Audio | Wake::Timeout => {}
    }
    if activation == Activation::Cancelled {
        return Ok(CaptureStatus::Stopped);
    }
    let _mmcss = match Mmcss::audio_detailed() {
        Ok(mmcss) => {
            report.mmcss_registered = true;
            Some(mmcss)
        }
        Err(error) => {
            report.mmcss_hresult = Some(error.code().0);
            None
        }
    };
    let mut streaming = api("Start(capture)", Streaming::start(input.client))?;
    report.stream_started = true;
    input.started.store(true, Ordering::Release);
    let started = Instant::now();
    let result = (|| {
        while input.duration.remaining(started.elapsed()) != Some(Duration::ZERO) {
            match wait_target(
                handle(input.stop),
                input.audio,
                input.target,
                input.duration.timeout_ms(started.elapsed()),
            )? {
                Wake::Stop => return Ok(CaptureStatus::Stopped),
                Wake::TargetExited => return Ok(CaptureStatus::TargetExited),
                Wake::Timeout => {
                    report.timeout_wakes += 1;
                    continue;
                }
                Wake::Audio => report.audio_wakes += 1,
            }
            let wake_qpc_100ns = clock.as_ref().and_then(QpcClock::now);
            let mut native_frames = 0u64;
            let mut accepted_frames = 0u64;
            // Stop and process-exit checks also bound a continuously busy producer.
            // A terminating drain may already have released packets. Publish
            // them before propagating its outcome; partial service bursts do
            // not establish a steady native period.
            let drained = (|| -> Result<Option<CaptureStatus>, CaptureError> {
                while input.duration.remaining(started.elapsed()) != Some(Duration::ZERO) {
                    match wait_target(handle(input.stop), input.audio, input.target, 0)? {
                        Wake::Stop => return Ok(Some(CaptureStatus::Stopped)),
                        Wake::TargetExited => return Ok(Some(CaptureStatus::TargetExited)),
                        Wake::Audio | Wake::Timeout => {}
                    }
                    if api("GetNextPacketSize(capture)", unsafe {
                        capture.GetNextPacketSize()
                    })? == 0
                    {
                        break;
                    }
                    let transfer = transfer_packet(
                        &capture,
                        capacity,
                        &mut ingress,
                        report.artifact_recorder.as_mut(),
                        report.evidence_recorder.as_mut(),
                    )?;
                    if transfer.native_frames == 0 {
                        break;
                    }
                    delivery.packet(
                        transfer.native_frames,
                        clock.as_ref().and_then(QpcClock::now),
                    );
                    native_frames = native_frames.saturating_add(u64::from(transfer.native_frames));
                    accepted_frames =
                        accepted_frames.saturating_add(transfer.accepted_frames as u64);
                    report.packets += 1;
                    ingress.publish_native_delivery(delivery.snapshot());
                }
                Ok(None)
            })();
            let completed_qpc_100ns = clock.as_ref().and_then(QpcClock::now);
            delivery.callback_burst(native_frames);
            ingress.record_native_delivery_event(native_frames, completed_qpc_100ns);
            ingress.publish_native_delivery(delivery.snapshot());
            telemetry.record(CallbackObservation {
                wake_qpc_100ns,
                completed_qpc_100ns,
                demand_frames: native_frames,
                padding_before_frames: None,
                submitted_frames: accepted_frames,
            });
            if let Some(status) = drained? {
                return Ok(status);
            }
        }
        Ok(CaptureStatus::Completed)
    })();
    report.elapsed_seconds = started.elapsed().as_secs_f64();
    report.delivery = delivery.snapshot();
    // Wake peer before retiring producer or releasing COM, including target exit.
    let peer_stop = api("SetEvent(capture peer stop)", unsafe {
        SetEvent(handle(input.stop))
    });
    let stopped = streaming.stop();
    report.latency = telemetry.snapshot();
    report.stop_succeeded = stopped.is_ok();
    report.stop_hresult = stopped.as_ref().err().map(|e| e.code().0);
    match result {
        Err(error) => Err(error),
        Ok(status) => {
            peer_stop?;
            api("Stop(capture)", stopped)?;
            Ok(status)
        }
    }
}
pub(crate) fn finish(
    result: Result<CaptureStatus, CaptureError>,
    stop: &OwnedHandle,
    sender: &SyncSender<Result<Startup, CaptureError>>,
    mut report: CaptureReport,
) -> CaptureReport {
    // Called after owner-local COM has dropped; setup failures cancel peers too.
    let _ = unsafe { SetEvent(handle(stop)) };
    match result {
        Ok(status) => report.status = status,
        Err(error) => {
            report.failure = Some(error.to_string());
            let _ = sender.try_send(Err(error));
        }
    }
    report.finish_artifact();
    report
}
