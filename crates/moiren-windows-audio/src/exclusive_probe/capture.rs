//! The same process client remains active across occupation and release.
use super::WindowReport;
use crate::{
    owner::{OwnedHandle, PacketLease, Process},
    process_loopback::{activation::activate, capture_format},
    stats::{CaptureStats, SILENT, analyze_f32},
};
use anyhow::Context;
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    Media::Audio::{
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient,
    },
    System::Threading::WaitForSingleObject,
};

pub(super) struct Loopback {
    service: IAudioCaptureClient,
    pub client: IAudioClient,
    event: OwnedHandle,
    capacity: u32,
}

impl Loopback {
    pub fn prepare(process: &Process) -> anyhow::Result<Self> {
        let client = activate(process.identity.pid, || {
            if process
                .exited()
                .map_err(|error| crate::capture::CaptureError::Api {
                    stage: "target liveness",
                    hresult: error.code().0,
                })?
            {
                return Err(crate::capture::CaptureError::TargetExited);
            }
            Ok(())
        })?;
        let event = OwnedHandle::event()?;
        unsafe {
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    0,
                    0,
                    &capture_format(),
                    None,
                )
                .context("Initialize(process loopback)")?;
            client.SetEventHandle(event.0)?;
            let capacity = client.GetBufferSize()?;
            let service = client.GetService()?;
            Ok(Self {
                service,
                client,
                event,
                capacity,
            })
        }
    }

    pub fn sample(
        &self,
        process: &Process,
        phase: &'static str,
        settle: Duration,
        duration: Duration,
        mut check_peer: impl FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<WindowReport> {
        let mut stats = CaptureStats::default();
        let mut signal_frames = 0;
        let start = Instant::now();
        let deadline = settle + duration;
        while start.elapsed() < deadline {
            anyhow::ensure!(!process.exited()?, "target exited during {phase}");
            check_peer()?;
            match unsafe { WaitForSingleObject(self.event.0, 50) } {
                WAIT_TIMEOUT => continue,
                WAIT_OBJECT_0 => {}
                _ => return Err(windows::core::Error::from_thread().into()),
            }
            while start.elapsed() < deadline {
                if unsafe { self.service.GetNextPacketSize() }.context("GetNextPacketSize")? == 0 {
                    break;
                }
                let (mut data, mut frames, mut flags, mut position, mut qpc) =
                    (std::ptr::null_mut(), 0, 0, 0, 0);
                unsafe {
                    self.service.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        Some(&mut position),
                        Some(&mut qpc),
                    )
                }
                .context("GetBuffer(process loopback)")?;
                let lease = PacketLease {
                    client: &self.service,
                    frames,
                    released: false,
                };
                anyhow::ensure!(frames <= self.capacity, "capture packet exceeds capacity");
                let silent = flags & SILENT != 0;
                anyhow::ensure!(
                    silent || frames == 0 || !data.is_null(),
                    "null capture packet"
                );
                let bytes = if silent || frames == 0 {
                    &[]
                } else {
                    // GetBuffer lease owns exactly frames of initialized stereo f32.
                    unsafe { std::slice::from_raw_parts(data, frames as usize * 8) }
                };
                let metrics = analyze_f32(bytes, frames, 2, silent)?;
                if start.elapsed() >= settle {
                    // An amplitude floor excludes denormals/noise from evidence.
                    if metrics.peak > 1e-5 {
                        signal_frames += u64::from(frames);
                    }
                    stats.observe(frames, flags, position, qpc, metrics);
                }
                lease.release().context("ReleaseBuffer(process loopback)")?;
            }
        }
        anyhow::ensure!(!process.exited()?, "target exited during {phase}");
        check_peer()?;
        Ok(WindowReport {
            phase,
            elapsed_seconds: start.elapsed().saturating_sub(settle).as_secs_f64(),
            signal_frames,
            summary: stats.summary(),
        })
    }
}
