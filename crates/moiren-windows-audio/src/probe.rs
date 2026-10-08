//! Opt-in, statistics-only Process Loopback probe; it never opens a render stream.
use crate::{
    catalog::{self, ApiFailure, CatalogSnapshot, FormatSnapshot},
    owner::{Apartment, Mmcss, OwnedHandle, PacketLease, Process, ProcessIdentity, Streaming},
    stats::{CaptureStats, CaptureSummary, SILENT, analyze_f32},
};
use serde::Serialize;
use std::{
    mem::{ManuallyDrop, size_of},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::{E_UNEXPECTED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::{Audio::*, Multimedia::WAVE_FORMAT_IEEE_FLOAT},
        System::{
            Com::{
                BLOB, IAgileObject, IAgileObject_Impl,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
            },
            Threading::{AvSetMmThreadCharacteristicsW, WaitForSingleObject},
            Variant::VT_BLOB,
        },
    },
    core::{HRESULT, Interface, Ref, Result, implement},
};

#[derive(Serialize)]
pub struct PacketRecord {
    pub arrival_ms: f64,
    pub frames: u32,
    pub flags: u32,
    pub device_position_frames: u64,
    pub qpc_100ns: u64,
    pub rms: f64,
    pub peak: f64,
}

#[derive(Serialize)]
pub struct CaptureReport {
    pub target: Option<ProcessIdentity>,
    pub mode: &'static str,
    pub status: &'static str,
    pub last_stage: &'static str,
    pub requested_seconds: u32,
    pub elapsed_seconds: f64,
    pub capture_stream_format: FormatSnapshot,
    pub windows_auto_conversion: bool,
    pub buffer_frames: Option<u32>,
    pub mmcss_registered: bool,
    pub stop_succeeded: bool,
    pub summary: CaptureSummary,
    pub metadata_dropped: u64,
    pub packets: Vec<PacketRecord>,
    pub errors: Vec<ApiFailure>,
}

#[derive(Serialize)]
pub struct ProbeReport {
    pub schema_version: u32,
    pub started_unix_ms: u128,
    pub before: CatalogSnapshot,
    pub capture: Option<CaptureReport>,
    pub after: Option<CatalogSnapshot>,
    pub observed_state_changes: Vec<String>,
    pub errors: Vec<ApiFailure>,
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Completion {
    completed: Arc<AtomicBool>,
    // Keep activation data alive even if the owner times out before completion.
    _params: Arc<AUDIOCLIENT_ACTIVATION_PARAMS>,
}
impl IAgileObject_Impl for Completion_Impl {}
impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> Result<()> {
        // No panic, lock, COM interface transfer, or last-reference release in the callback.
        self.completed.store(true, Ordering::Release);
        Ok(())
    }
}

fn activation_params(pid: u32) -> Arc<AUDIOCLIENT_ACTIVATION_PARAMS> {
    Arc::new(AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    })
}

fn borrowed_activation_variant(
    params: &Arc<AUDIOCLIENT_ACTIVATION_PARAMS>,
) -> ManuallyDrop<PROPVARIANT> {
    // windows 0.62 PROPVARIANT implements Drop (PropVariantClear). Suppress the
    // OUTER destructor too: this BLOB borrows Rust memory, not CoTaskMem memory.
    ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: Arc::as_ptr(params).cast_mut().cast(),
                    },
                },
                ..Default::default()
            }),
        },
    })
}

fn activate(pid: u32) -> Result<IAudioClient> {
    let params = activation_params(pid);
    let completed = Arc::new(AtomicBool::new(false));
    let handler: IActivateAudioInterfaceCompletionHandler = Completion {
        completed: completed.clone(),
        _params: params.clone(),
    }
    .into();
    let variant = borrowed_activation_variant(&params);
    // The variant borrows Arc-owned data: never PropVariantClear this borrowed BLOB.
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*variant),
            &handler,
        )?
    };
    let start = Instant::now();
    while !completed.load(Ordering::Acquire) {
        if start.elapsed() >= Duration::from_secs(10) {
            return Err(windows::core::Error::from_hresult(HRESULT::from_win32(
                1460,
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut status = E_UNEXPECTED;
    let mut interface = None;
    unsafe {
        operation.GetActivateResult(&mut status, &mut interface)?;
    }
    status.ok()?;
    interface
        .ok_or_else(|| windows::core::Error::from_hresult(E_UNEXPECTED))?
        .cast()
}

fn capture(process: &Process, report: &mut CaptureReport) -> Result<()> {
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

fn capture_format() -> WAVEFORMATEX {
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

/// None only enumerates. An explicit PID opts into capture of that process tree.
pub fn run(pid: Option<u32>, seconds: u32) -> anyhow::Result<ProbeReport> {
    if !(1..=600).contains(&seconds) {
        anyhow::bail!("--seconds must be within 1..=600, got {seconds}");
    }
    if pid == Some(0) {
        anyhow::bail!("pid 0 (system idle process) cannot be a capture target");
    }
    let _apartment = Apartment::new()?;
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis());
    let before = catalog::snapshot()?;
    let mut result = ProbeReport {
        schema_version: 1,
        started_unix_ms,
        before,
        capture: None,
        after: None,
        observed_state_changes: Vec::new(),
        errors: Vec::new(),
    };
    if let Some(pid) = pid {
        let mut report = CaptureReport {
            target: None,
            mode: "include_target_process_tree",
            status: "initializing",
            last_stage: "target process identity",
            requested_seconds: seconds,
            elapsed_seconds: 0.0,
            capture_stream_format: FormatSnapshot::from_base(capture_format()),
            windows_auto_conversion: true,
            buffer_frames: None,
            mmcss_registered: false,
            stop_succeeded: false,
            summary: CaptureSummary::default(),
            metadata_dropped: 0,
            packets: Vec::new(),
            errors: Vec::new(),
        };
        match Process::open(pid) {
            Ok(process) => {
                if let Err(error) = capture(&process, &mut report) {
                    report.status = "api_failed";
                    report
                        .errors
                        .push(ApiFailure::new(report.last_stage, error));
                }
                report.target = Some(process.identity);
            }
            Err(error) => {
                report.status = "api_failed";
                report
                    .errors
                    .push(ApiFailure::new("target process identity", error));
            }
        }
        result.capture = Some(report);
        match catalog::snapshot() {
            Ok(after) => {
                result.observed_state_changes = catalog::changes(&result.before, &after, pid);
                result.after = Some(after);
            }
            Err(error) => result
                .errors
                .push(ApiFailure::new("post-capture snapshot", error)),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_blob_drop_does_not_free_rust_activation_data() {
        let params = activation_params(123);
        {
            let variant = borrowed_activation_variant(&params);
            let blob = unsafe { &variant.Anonymous.Anonymous.Anonymous.blob };
            assert_eq!(blob.pBlobData, Arc::as_ptr(&params).cast_mut().cast());
            assert_eq!(
                blob.cbSize as usize,
                size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()
            );
        }
        assert_eq!(
            unsafe { params.Anonymous.ProcessLoopbackParams.TargetProcessId },
            123
        );
        assert_eq!(Arc::strong_count(&params), 1);
    }

    #[test]
    fn agile_completion_keeps_parameters_alive_until_last_reference() {
        let params = activation_params(123);
        let completed = Arc::new(AtomicBool::new(false));
        let handler: IActivateAudioInterfaceCompletionHandler = Completion {
            completed: completed.clone(),
            _params: params.clone(),
        }
        .into();
        let agile = handler.cast::<IAgileObject>().unwrap();
        assert_eq!(Arc::strong_count(&params), 2);
        unsafe {
            handler
                .ActivateCompleted(None::<&IActivateAudioInterfaceAsyncOperation>)
                .unwrap();
        }
        assert!(completed.load(Ordering::Acquire));
        drop(handler);
        assert_eq!(Arc::strong_count(&params), 2);
        drop(agile);
        assert_eq!(Arc::strong_count(&params), 1);
    }
}
