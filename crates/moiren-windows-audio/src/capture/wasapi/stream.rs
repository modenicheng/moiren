//! Native preparation, streaming and destruction run on one COM owner thread.
use super::{Startup, Wake, api, format::mix_format, handle, packet::transfer_packet, wait};
use crate::{
    capture::{CaptureError, CaptureOptions, CaptureReport, CaptureStatus},
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    owner::{Apartment, Mmcss, OwnedHandle, Streaming},
};
use std::{
    os::windows::io::OwnedHandle as StdHandle,
    sync::{Arc, mpsc::SyncSender},
    time::Instant,
};
use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_NOPERSIST, AudioCategory_Other, AudioClientProperties,
            IAudioCaptureClient, IAudioClient, IAudioClient2, IMMDeviceEnumerator, IMMEndpoint,
            MMDeviceEnumerator, eCapture,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
            Threading::{AvSetMmThreadCharacteristicsW, SetEvent},
        },
    },
    core::{HSTRING, Interface},
};

fn owner(
    options: &CaptureOptions,
    stop: &StdHandle,
    sender: &SyncSender<Result<Startup, CaptureError>>,
    report: &mut CaptureReport,
) -> Result<CaptureStatus, CaptureError> {
    // Reverse declaration order releases stream/services/client before apartment.
    let _apartment = api("CoInitializeEx(capture)", Apartment::new())?;
    let enumerator: IMMDeviceEnumerator = api("CoCreateInstance(capture)", unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    })?;
    let device = api("GetDevice(capture pinned)", unsafe {
        enumerator.GetDevice(&HSTRING::from(&options.endpoint_id))
    })?;
    let endpoint: IMMEndpoint = api("IMMEndpoint(capture)", device.cast())?;
    if api("GetDataFlow(capture)", unsafe { endpoint.GetDataFlow() })? != eCapture {
        return Err(CaptureError::InvalidEndpoint);
    }
    let event = api("CreateEvent(capture audio)", OwnedHandle::event())?;
    let client: IAudioClient = api("Activate(capture)", unsafe {
        device.Activate(CLSCTX_ALL, None)
    })?;
    let client2: IAudioClient2 = api("IAudioClient2(capture)", client.cast())?;
    api("SetClientProperties(capture)", unsafe {
        client2.SetClientProperties(&AudioClientProperties {
            cbSize: size_of::<AudioClientProperties>() as u32,
            eCategory: AudioCategory_Other,
            ..Default::default()
        })
    })?;
    let memory = mix_format(&client)?;
    // SAFETY: mix_format validated the owned packed native structure.
    let base = unsafe { memory.0.read_unaligned() };
    report.sample_rate = Some(base.nSamplesPerSec);
    report.channels = Some(usize::from(base.nChannels));
    let guid = api("CoCreateGuid(capture)", unsafe { CoCreateGuid() })?;
    api("Initialize(capture Shared)", unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_NOPERSIST,
            0,
            0,
            memory.0,
            Some(&guid),
        )
    })?;
    api("SetEventHandle(capture)", unsafe {
        client.SetEventHandle(event.0)
    })?;
    let capacity = api("GetBufferSize(capture)", unsafe { client.GetBufferSize() })?;
    if capacity == 0 || u64::from(capacity) * u64::from(base.nBlockAlign) > 8 * 1024 * 1024 {
        return Err(CaptureError::BufferBudget);
    }
    report.buffer_frames = Some(capacity);
    let capture: IAudioCaptureClient = api("GetService(capture)", unsafe { client.GetService() })?;
    let (mut ingress, source, observer) = capture_bridge(ClockBridgeConfig {
        input_sample_rate: base.nSamplesPerSec,
        input_channels: usize::from(base.nChannels),
        ..ClockBridgeConfig::default()
    })?;
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
    let mut streaming = api("Start(capture)", Streaming::start(&client))?;
    report.stream_started = true;
    let started = Instant::now();
    let result = (|| {
        sender
            .send(Ok(Startup {
                source,
                observer,
                sample_rate: base.nSamplesPerSec,
                channels: usize::from(base.nChannels),
            }))
            .map_err(|_| CaptureError::StartupLost)?;
        while started.elapsed() < options.duration {
            let remaining = options.duration.saturating_sub(started.elapsed());
            match wait(
                handle(stop),
                event.0,
                remaining.as_millis().clamp(1, 100) as u32,
            )? {
                Wake::Stop => return Ok(CaptureStatus::Stopped),
                Wake::Timeout => {
                    report.timeout_wakes += 1;
                    continue;
                }
                Wake::Audio => report.audio_wakes += 1,
            }
            // Check cancellation between packets so a busy producer cannot keep
            // an owner draining indefinitely, including when Render has failed.
            while started.elapsed() < options.duration {
                if wait(handle(stop), event.0, 0)? == Wake::Stop {
                    return Ok(CaptureStatus::Stopped);
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
    // Wake linked Render before retiring the producer or releasing COM. A
    // control-side JoinHandle can remain unfinished throughout native cleanup.
    let peer_stop = api("SetEvent(capture peer stop)", unsafe {
        SetEvent(handle(stop))
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
pub(super) fn run_owner(
    options: CaptureOptions,
    stop: Arc<StdHandle>,
    sender: SyncSender<Result<Startup, CaptureError>>,
) -> CaptureReport {
    let mut report = CaptureReport {
        schema_version: 1,
        endpoint_id: options.endpoint_id.clone(),
        status: CaptureStatus::Failed,
        requested_seconds: options.duration.as_secs_f64(),
        elapsed_seconds: 0.0,
        sample_rate: None,
        channels: None,
        buffer_frames: None,
        stream_started: false,
        stop_succeeded: false,
        stop_hresult: None,
        mmcss_registered: false,
        mmcss_hresult: None,
        audio_wakes: 0,
        timeout_wakes: 0,
        packets: 0,
        failure: None,
    };
    // Formatting and startup-error delivery happen after native objects drop.
    let result = owner(&options, &stop, &sender, &mut report);
    // Initialization failures also cancel a linked peer, if any.
    let _ = unsafe { SetEvent(handle(&stop)) };
    match result {
        Ok(status) => report.status = status,
        Err(error) => {
            report.failure = Some(error.to_string());
            let _ = sender.try_send(Err(error));
        }
    }
    report
}
