//! Native service/bridge lifecycle shared by physical and virtual capture.
use super::{Startup, Wake, api, handle, packet::transfer_packet, wait_target};
use crate::{
    capture::{CaptureError, CaptureReport, CaptureSource, CaptureStatus},
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    owner::{Mmcss, Streaming},
};
use std::{
    os::windows::io::OwnedHandle,
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HANDLE,
    Media::Audio::{IAudioCaptureClient, IAudioClient},
    System::Threading::{AvSetMmThreadCharacteristicsW, SetEvent},
};

pub(crate) struct StreamInput<'a> {
    pub client: &'a IAudioClient,
    pub audio: HANDLE,
    pub target: Option<HANDLE>,
    pub stop: &'a OwnedHandle,
    pub duration: Duration,
    pub sample_rate: u32,
    pub channels: usize,
}
pub(crate) fn run(
    input: StreamInput<'_>,
    sender: &SyncSender<Result<Startup, CaptureError>>,
    report: &mut CaptureReport,
) -> Result<CaptureStatus, CaptureError> {
    let capacity = api("GetBufferSize(capture)", unsafe {
        input.client.GetBufferSize()
    })?;
    if capacity == 0 || u64::from(capacity) * input.channels as u64 * 4 > 8 * 1024 * 1024 {
        return Err(CaptureError::BufferBudget);
    }
    report.buffer_frames = Some(capacity);
    report.sample_rate = Some(input.sample_rate);
    report.channels = Some(input.channels);
    let capture: IAudioCaptureClient =
        api("GetService(capture)", unsafe { input.client.GetService() })?;
    let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig {
        input_sample_rate: input.sample_rate,
        input_channels: input.channels,
        detect_position_gaps: report.source == CaptureSource::Physical,
        ..ClockBridgeConfig::default()
    })?;
    // Check stop/target again after preparation, before Start or publishing source.
    match wait_target(handle(input.stop), input.audio, input.target, 0)? {
        Wake::Stop => return Err(CaptureError::Cancelled),
        Wake::TargetExited => return Err(CaptureError::TargetExited),
        Wake::Audio | Wake::Timeout => {}
    }
    let mut index = 0;
    let _mmcss =
        match unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut index) } {
            Ok(value) => {
                report.mmcss_registered = true;
                Some(Mmcss(value))
            }
            Err(error) => {
                report.mmcss_hresult = Some(error.code().0);
                None
            }
        };
    let mut streaming = api("Start(capture)", Streaming::start(input.client))?;
    report.stream_started = true;
    let started = Instant::now();
    let result = (|| {
        sender
            .send(Ok(Startup {
                source,
                observer,
                sample_rate: input.sample_rate,
                channels: input.channels,
            }))
            .map_err(|_| CaptureError::StartupLost)?;
        while started.elapsed() < input.duration {
            let remaining = input.duration.saturating_sub(started.elapsed());
            match wait_target(
                handle(input.stop),
                input.audio,
                input.target,
                remaining.as_millis().clamp(1, 100) as u32,
            )? {
                Wake::Stop => return Ok(CaptureStatus::Stopped),
                Wake::TargetExited => return Ok(CaptureStatus::TargetExited),
                Wake::Timeout => {
                    report.timeout_wakes += 1;
                    continue;
                }
                Wake::Audio => report.audio_wakes += 1,
            }
            // Stop and process-exit checks also bound a continuously busy producer.
            while started.elapsed() < input.duration {
                match wait_target(handle(input.stop), input.audio, input.target, 0)? {
                    Wake::Stop => return Ok(CaptureStatus::Stopped),
                    Wake::TargetExited => return Ok(CaptureStatus::TargetExited),
                    Wake::Audio | Wake::Timeout => {}
                }
                if api("GetNextPacketSize(capture)", unsafe {
                    capture.GetNextPacketSize()
                })? == 0
                {
                    break;
                }
                if transfer_packet(&capture, capacity, &mut ingress)? == 0 {
                    break;
                }
                report.packets += 1;
            }
        }
        Ok(CaptureStatus::Completed)
    })();
    report.elapsed_seconds = started.elapsed().as_secs_f64();
    // Publication is over even if native Stop or COM cleanup blocks. Consumers
    // must observe retirement before they can exhaust the final queued packet.
    ingress.finish();
    // Wake peer before stopping the native client, including target exit.
    let peer_stop = api("SetEvent(capture peer stop)", unsafe {
        SetEvent(handle(input.stop))
    });
    let stopped = streaming.stop();
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
    report
}
