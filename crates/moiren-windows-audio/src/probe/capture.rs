//! Loopback capture prepares its metadata storage before entering streaming.
use super::{CaptureReport, PacketRecord, activation::activate};
use crate::{
    catalog::ApiFailure,
    owner::{Mmcss, OwnedHandle, PacketLease, Process, Streaming},
    stats::{CaptureStats, SILENT, analyze_f32},
};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::{E_UNEXPECTED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::{
            Audio::{
                AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
                AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, WAVEFORMATEX,
            },
            Multimedia::WAVE_FORMAT_IEEE_FLOAT,
        },
        System::Threading::{AvSetMmThreadCharacteristicsW, WaitForSingleObject},
    },
    core::Result,
};

pub(super) fn capture(process: &Process, report: &mut CaptureReport) -> Result<()> {
    report.last_stage = "CreateEventW";
    let event = OwnedHandle::event()?;
    report.last_stage = "target process liveness";
    if process.exited()? {
        report.status = "target_exited";
        return Ok(());
    }
    report.last_stage = "ActivateAudioInterfaceAsync / completion / GetActivateResult";
    let client = activate(process.identity.pid)?;
    let format = capture_format();
    unsafe {
        report.last_stage = "IAudioClient::Initialize";
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            0,
            0,
            &format,
            None,
        )?;
        report.last_stage = "SetEventHandle";
        client.SetEventHandle(event.0)?;
    }
    report.last_stage = "GetBufferSize";
    let buffer_frames = unsafe { client.GetBufferSize()? };
    report.buffer_frames = Some(buffer_frames);
    report.last_stage = "GetService(IAudioCaptureClient)";
    let service: IAudioCaptureClient = unsafe { client.GetService()? };
    // Allocated before streaming; if the bound is reached, count lost metadata.
    report.packets = Vec::with_capacity(report.requested_seconds as usize * 1000);
    let mut stats = CaptureStats::default();
    let mut task_index = 0;
    let mmcss =
        unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut task_index) };
    let _mmcss = match mmcss {
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
    let start = Instant::now();
    let duration = Duration::from_secs(u64::from(report.requested_seconds));
    let result: Result<()> = (|| {
        while start.elapsed() < duration {
            report.last_stage = "target process liveness";
            if process.exited()? {
                report.status = "target_exited";
                break;
            }
            let timeout = duration
                .saturating_sub(start.elapsed())
                .as_millis()
                .clamp(1, 100) as u32;
            report.last_stage = "audio event wait";
            match unsafe { WaitForSingleObject(event.0, timeout) } {
                WAIT_TIMEOUT => continue,
                WAIT_OBJECT_0 => {}
                _ => return Err(windows::core::Error::from_thread()),
            }
            while start.elapsed() < duration {
                report.last_stage = "GetNextPacketSize";
                if unsafe { service.GetNextPacketSize()? } == 0 {
                    break;
                }
                report.last_stage = "target process liveness";
                if process.exited()? {
                    report.status = "target_exited";
                    return Ok(());
                }
                let (mut data, mut frames, mut flags, mut position, mut qpc) =
                    (std::ptr::null_mut(), 0, 0, 0, 0);
                report.last_stage = "GetBuffer";
                unsafe {
                    service.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        Some(&mut position),
                        Some(&mut qpc),
                    )?;
                }
                if frames == 0 {
                    break;
                }
                report.last_stage = "packet validation / statistics";
                let lease = PacketLease {
                    client: &service,
                    frames,
                    released: false,
                };
                if frames > buffer_frames {
                    return Err(windows::core::Error::from_hresult(E_UNEXPECTED));
                }
                let silent = flags & SILENT != 0;
                if !silent && data.is_null() {
                    return Err(windows::core::Error::from_hresult(E_UNEXPECTED));
                }
                let bytes = if silent {
                    &[]
                } else {
                    // Bound by GetBuffer's lease, returned frames, and the initialized f32 format.
                    unsafe {
                        std::slice::from_raw_parts(
                            data,
                            frames as usize * usize::from(format.nBlockAlign),
                        )
                    }
                };
                let metrics = analyze_f32(bytes, frames, format.nChannels, silent)
                    .map_err(|_| windows::core::Error::from_hresult(E_UNEXPECTED))?;
                if report.packets.len() < report.packets.capacity() {
                    report.packets.push(PacketRecord {
                        arrival_ms: start.elapsed().as_secs_f64() * 1000.0,
                        frames,
                        flags,
                        device_position_frames: position,
                        qpc_100ns: qpc,
                        rms: metrics.rms(),
                        peak: metrics.peak,
                    });
                } else {
                    report.metadata_dropped += 1;
                }
                stats.observe(frames, flags, position, qpc, metrics);
                report.last_stage = "ReleaseBuffer";
                lease.release()?;
            }
        }
        Ok(())
    })();
    report.elapsed_seconds = start.elapsed().as_secs_f64();
    let stop = streaming.stop();
    report.stop_succeeded = stop.is_ok();
    if let Err(error) = stop {
        report.errors.push(ApiFailure::new("Stop", error));
    }
    report.summary = stats.summary();
    result?;
    if !report.stop_succeeded {
        report.status = "api_failed";
        report.last_stage = "Stop";
        return Ok(());
    }
    report.last_stage = "capture completed / Stop attempted";
    if report.status != "target_exited" {
        report.status = if report.summary.signal_packets > 0 {
            "completed_with_signal"
        } else {
            "completed_no_signal"
        };
    }
    Ok(())
}

pub(super) fn capture_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 48000 * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}
