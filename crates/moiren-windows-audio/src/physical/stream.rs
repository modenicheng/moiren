//! Native endpoint preparation and event sampling stay on one COM owner thread.
use super::{EndpointReport, RenderSummary};
use crate::{
    catalog::{ApiFailure, FormatSnapshot},
    owner::{Apartment, Mmcss, OwnedHandle, Streaming, TaskMemory},
    stats::CaptureStats,
};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::{E_UNEXPECTED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_NOPERSIST, AudioCategory_Other,
            AudioClientProperties, IAudioCaptureClient, IAudioClient, IAudioClient2, IAudioClient3,
            IAudioClock, IAudioClock2, IAudioRenderClient, IAudioSessionControl,
            IAudioSessionControl2, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator,
            WAVEFORMATEXTENSIBLE, eCapture, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
            Threading::{AvSetMmThreadCharacteristicsW, WaitForSingleObject},
        },
    },
    core::{GUID, HSTRING, Interface, Result},
};

mod sampling;
use sampling::{SamplingWindow, capture_packets, render_silence, sample_clocks};

fn unexpected() -> windows::core::Error {
    windows::core::Error::from_hresult(E_UNEXPECTED)
}

pub(super) fn prepare_and_sample(report: &mut EndpointReport, seconds: u32) -> Result<()> {
    // Locals drop in reverse order: every service must release its COM reference
    // before this guard uninitializes the worker's apartment, even on an error.
    let _apartment = Apartment::new()?;
    report.last_stage = "GetDevice / flow";
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(&report.endpoint_id))? };
    let endpoint: IMMEndpoint = device.cast()?;
    let flow = unsafe { endpoint.GetDataFlow()? };
    report.flow = if flow == eRender {
        "render"
    } else if flow == eCapture {
        "capture"
    } else {
        return Err(unexpected());
    };
    let event = OwnedHandle::event()?;
    report.last_stage = "Activate(IAudioClient) / category";
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let client2: IAudioClient2 = client.cast()?;
    unsafe {
        client2.SetClientProperties(&AudioClientProperties {
            cbSize: size_of::<AudioClientProperties>() as u32,
            eCategory: AudioCategory_Other,
            ..Default::default()
        })?;
    }
    report.last_stage = "GetMixFormat / native f32 validation";
    let memory = TaskMemory(unsafe { client.GetMixFormat()? });
    if memory.0.is_null() {
        return Err(unexpected());
    }
    let base = unsafe { memory.0.read_unaligned() };
    let mut descriptor = FormatSnapshot::from_base(base);
    let mut float = base.wFormatTag == 3;
    if base.wFormatTag == 0xfffe && base.cbSize >= 22 {
        let extended = unsafe { memory.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
        let guid = extended.SubFormat;
        descriptor.valid_bits = Some(unsafe { extended.Samples.wValidBitsPerSample });
        descriptor.channel_mask = Some(extended.dwChannelMask);
        descriptor.sub_format = Some(format!("{guid:?}"));
        float = guid == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
    }
    report.format = Some(descriptor);
    if !float
        || base.wBitsPerSample != 32
        || base.nChannels == 0
        || base.nSamplesPerSec == 0
        || base.nBlockAlign != base.nChannels.checked_mul(4).ok_or_else(unexpected)?
    {
        // This W00 slice deliberately supports the formats needed on this machine.
        return Err(windows::core::Error::from_hresult(
            AUDCLNT_E_UNSUPPORTED_FORMAT,
        ));
    }
    report.last_stage = "Initialize / SetEventHandle";
    let session_id = unsafe { CoCreateGuid()? };
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_NOPERSIST,
            0,
            0,
            memory.0,
            Some(&session_id),
        )?;
        client.SetEventHandle(event.0)?;
    }
    // Attenuation preference applies to our render session. Capture sessions on
    // the tested drivers reject this method with AUDCLNT_E_WRONG_ENDPOINT_TYPE.
    // Never call it on QQMusic's session; category Other is set for both flows.
    if flow == eRender {
        report.last_stage = "GetService(IAudioSessionControl)";
        let session: IAudioSessionControl2 =
            unsafe { client.GetService::<IAudioSessionControl>()? }.cast()?;
        report.last_stage = "SetDuckingPreference(own render session)";
        unsafe {
            session.SetDuckingPreference(true)?;
        }
        report.own_session_ducking_opt_out = true;
    }
    report.last_stage = "GetBufferSize / latency / engine period";
    let capacity = unsafe { client.GetBufferSize()? };
    report.buffer_frames = Some(capacity);
    report.stream_latency_100ns = Some(unsafe { client.GetStreamLatency()? });
    match client.cast::<IAudioClient3>() {
        Ok(client3) => {
            let mut format_ptr = std::ptr::null_mut();
            let mut period = 0;
            match unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut format_ptr, &mut period) }
            {
                Ok(()) => {
                    let _format = TaskMemory(format_ptr);
                    report.engine_period_frames = Some(period);
                }
                Err(error) => report.errors.push(ApiFailure::new(
                    "GetCurrentSharedModeEnginePeriod (optional)",
                    error,
                )),
            }
        }
        Err(error) => report
            .errors
            .push(ApiFailure::new("IAudioClient3 (optional)", error)),
    }
    report.last_stage = "GetService(IAudioClock) / GetFrequency";
    let clock: IAudioClock = unsafe { client.GetService()? };
    let frequency = unsafe { clock.GetFrequency()? };
    if frequency == 0 {
        return Err(unexpected());
    }
    report.frequency_units_per_second = Some(frequency);
    let device_clock = match clock.cast::<IAudioClock2>() {
        Ok(clock2) => Some(clock2),
        Err(error) => {
            report
                .errors
                .push(ApiFailure::new("IAudioClock2 (optional)", error));
            None
        }
    };
    let capture: Option<IAudioCaptureClient> = if flow == eCapture {
        Some(unsafe { client.GetService()? })
    } else {
        None
    };
    let render: Option<IAudioRenderClient> = if flow == eRender {
        Some(unsafe { client.GetService()? })
    } else {
        None
    };
    let mut stats = CaptureStats::default();
    let bound = seconds as usize * 1000;
    report.clock_points = Vec::with_capacity(bound);
    if device_clock.is_some() {
        report.device_clock_points = Vec::with_capacity(bound);
    }
    if capture.is_some() {
        report.capture_packets = Vec::with_capacity(bound);
    }
    if let Some(render) = &render {
        report.render_demands = Vec::with_capacity(bound);
        report.last_stage = "render priming";
        write_silence(render, capacity)?;
        report.render = Some(RenderSummary {
            primed_frames: capacity,
            ..Default::default()
        });
    }
    let mut task_index = 0;
    let _mmcss =
        match unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut task_index) }
        {
            Ok(handle) => {
                report.mmcss_registered = true;
                Some(Mmcss(handle))
            }
            Err(error) => {
                report
                    .errors
                    .push(ApiFailure::new("MMCSS (optional)", error));
                None
            }
        };
    report.last_stage = "Start";
    let mut streaming = Streaming::start(&client)?;
    let window = SamplingWindow {
        start: Instant::now(),
        duration: Duration::from_secs(u64::from(seconds)),
        metadata_limit: bound,
    };
    let result: Result<()> = (|| {
        while window.start.elapsed() < window.duration {
            report.last_stage = "audio event wait";
            let timeout = window
                .duration
                .saturating_sub(window.start.elapsed())
                .as_millis()
                .clamp(1, 100) as u32;
            match unsafe { WaitForSingleObject(event.0, timeout) } {
                WAIT_TIMEOUT => {
                    report.timeout_wakes += 1;
                    continue;
                }
                WAIT_OBJECT_0 => report.audio_wakes += 1,
                _ => return Err(windows::core::Error::from_thread()),
            }
            sample_clocks(report, &clock, device_clock.as_ref(), &window)?;
            if let Some(capture) = &capture {
                capture_packets(report, capture, capacity, base, &mut stats, &window)?;
            }
            if let Some(render) = &render {
                render_silence(report, &client, render, capacity, &window)?;
            }
        }
        Ok(())
    })();
    report.elapsed_seconds = window.start.elapsed().as_secs_f64();
    let stop = streaming.stop();
    report.stop_succeeded = stop.is_ok();
    if capture.is_some() {
        report.capture = Some(stats.summary());
    }
    if let Err(error) = result {
        if let Err(stop_error) = stop {
            report.errors.push(ApiFailure::new("Stop", stop_error));
        }
        return Err(error);
    }
    report.last_stage = "Stop";
    stop?;
    report.last_stage = "completed";
    report.status = "completed";
    Ok(())
}

fn write_silence(render: &IAudioRenderClient, frames: u32) -> Result<()> {
    if frames == 0 {
        return Ok(());
    }
    unsafe {
        let _leased_data = render.GetBuffer(frames)?;
        // The SILENT flag makes writing into the returned pointer unnecessary.
        render.ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
    }
}
