//! Native endpoint preparation and event sampling stay on one COM owner thread.
use super::{EndpointReport, RenderSummary};
use crate::{
    catalog::{ApiFailure, FormatSnapshot},
    negotiation::{
        ParsedDescriptor, SUBFORMAT_FLOAT, endpoint_data_flow, mix_format, open_endpoint_device,
        parse_descriptor,
    },
    stats::CaptureStats,
    win32_raii::{Apartment, Mmcss, OwnedHandle, Streaming, current_shared_mode_engine_period},
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
            IAudioSessionControl2, eCapture, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid},
            Threading::WaitForSingleObject,
        },
    },
    core::{Interface, Result},
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
    let device = open_endpoint_device(&report.endpoint_id)?;
    let flow = endpoint_data_flow(&device)?;
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
    let memory = mix_format(&client)?;
    let base = unsafe { memory.0.read_unaligned() };
    let mut descriptor = FormatSnapshot::from_base(base);
    // SAFETY: mix_format guarantees the complete base+cbSize allocation;
    // the kernel parse copies packed fields by value.
    let parsed = unsafe { parse_descriptor(memory.0) }
        .map_err(|_| windows::core::Error::from_hresult(AUDCLNT_E_UNSUPPORTED_FORMAT))?;
    let float = match parsed {
        ParsedDescriptor::Float { .. } => true,
        ParsedDescriptor::Extensible(extended) => {
            // Packed struct: copy fields by value; no reference escapes.
            let sub_format = extended.SubFormat;
            let valid_bits = unsafe { extended.Samples.wValidBitsPerSample };
            descriptor.valid_bits = Some(valid_bits);
            descriptor.channel_mask = Some(extended.dwChannelMask);
            descriptor.sub_format = Some(format!("{sub_format:?}"));
            sub_format == SUBFORMAT_FLOAT
        }
        ParsedDescriptor::Pcm { .. } => false,
    };
    report.format = Some(descriptor);
    if !float
        || base.wBitsPerSample != 32
        || base.nChannels == 0
        || base.nSamplesPerSec == 0
        || base.nBlockAlign != base.nChannels.checked_mul(4).ok_or_else(unexpected)?
    {
        // This hardware probe deliberately supports native f32 only.
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
    // Never call it on other processes' sessions; category Other is set for
    // both flows.
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
        Ok(client3) => match current_shared_mode_engine_period(&client3) {
            Ok((period, _format)) => report.engine_period_frames = Some(period),
            Err(error) => report.errors.push(ApiFailure::new(
                "GetCurrentSharedModeEnginePeriod (optional)",
                error,
            )),
        },
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
    let _mmcss = match Mmcss::audio_detailed() {
        Ok(mmcss) => {
            report.mmcss_registered = true;
            Some(mmcss)
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
