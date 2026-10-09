//! Same-thread COM resources and restoration for the owned experiment.
use super::Samples;
use anyhow::{Result, bail};
use serde::Serialize;
use std::{
    process::Child,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioCaptureClient,
            IAudioClient, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
            ISimpleAudioVolume, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
        },
    },
    core::{GUID, Interface},
};
pub(super) struct Apartment;
impl Apartment {
    pub(super) fn new() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct MixMemory(*mut WAVEFORMATEX);
impl Drop for MixMemory {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}
pub(super) struct OwnedGenerator(pub(super) Child);
impl Drop for OwnedGenerator {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) struct EndpointTap {
    service: IAudioCaptureClient,
    client: IAudioClient,
    capacity: u32,
}
impl EndpointTap {
    pub(super) fn new(device: &IMMDevice) -> Result<Self> {
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let memory = MixMemory(unsafe { client.GetMixFormat()? });
        // GetMixFormat owns a complete descriptor; inspect its extension
        // only after validating the advertised size and native layout.
        let format = unsafe { &*memory.0 };
        let float = format.wFormatTag == 3
            || (format.wFormatTag == 0xfffe
                && format.cbSize >= 22
                && unsafe {
                    let ext = &*memory.0.cast::<WAVEFORMATEXTENSIBLE>();
                    let sub_format = ext.SubFormat;
                    sub_format == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71)
                        && ext.Samples.wValidBitsPerSample == 32
                });
        if !float
            || format.nSamplesPerSec != 48_000
            || format.nChannels != 2
            || format.wBitsPerSample != 32
            || format.nBlockAlign != 8
        {
            bail!("probe requires a native 48 kHz stereo f32 endpoint");
        }
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                0,
                0,
                memory.0,
                None,
            )?
        };
        let capacity = unsafe { client.GetBufferSize()? };
        let service = unsafe { client.GetService()? };
        unsafe { client.Start()? };
        Ok(Self {
            service,
            client,
            capacity,
        })
    }
    pub(super) fn drain(&self, samples: &mut Samples) -> Result<()> {
        while unsafe { self.service.GetNextPacketSize()? } != 0 {
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
            unsafe {
                self.service
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?
            };
            // Every GetBuffer has a release, including validation failures.
            let lease = CaptureLease {
                service: &self.service,
                frames,
            };
            if frames > self.capacity || (flags & 2 == 0 && data.is_null()) {
                bail!("invalid endpoint loopback packet");
            }
            if flags & 2 != 0 {
                for _ in 0..u64::from(frames) * 2 {
                    samples.add(0.0);
                }
            } else {
                // Read byte chunks so driver-buffer alignment is irrelevant.
                let bytes = unsafe { std::slice::from_raw_parts(data, frames as usize * 8) };
                for bytes in bytes.as_chunks::<4>().0 {
                    samples.add(f64::from(f32::from_le_bytes(*bytes)));
                }
            }
            drop(lease);
        }
        Ok(())
    }
}
impl Drop for EndpointTap {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}
struct CaptureLease<'a> {
    service: &'a IAudioCaptureClient,
    frames: u32,
}
impl Drop for CaptureLease<'_> {
    fn drop(&mut self) {
        let _ = unsafe { self.service.ReleaseBuffer(self.frames) };
    }
}

#[derive(Serialize)]
pub(super) struct Restoration {
    volume: f32,
    muted: bool,
    pub(super) verified: bool,
}
pub(super) struct SessionGuard {
    volume: ISimpleAudioVolume,
    pub(super) before_volume: f32,
    pub(super) before_mute: bool,
    restored: bool,
}
impl SessionGuard {
    pub(super) fn find(device: &IMMDevice, child: &mut OwnedGenerator) -> Result<Self> {
        let start = Instant::now();
        loop {
            if child.0.try_wait()?.is_some() {
                bail!("owned generator exited during startup");
            }
            let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
            let sessions = unsafe { manager.GetSessionEnumerator()? };
            for index in 0..unsafe { sessions.GetCount()? } {
                let session: IAudioSessionControl2 =
                    unsafe { sessions.GetSession(index)? }.cast()?;
                let mut pid = 0;
                let status = unsafe {
                    (Interface::vtable(&session).GetProcessId)(
                        Interface::as_raw(&session),
                        &mut pid,
                    )
                };
                // A successful non-S_OK status can mean a shared session.
                // Modify only the still-live owned child's private session.
                if status.0 == 0 && pid == child.0.id() {
                    let volume: ISimpleAudioVolume = session.cast()?;
                    let before_volume = unsafe { volume.GetMasterVolume()? };
                    let before_mute = unsafe { volume.GetMute()? }.as_bool();
                    return Ok(Self {
                        volume,
                        before_volume,
                        before_mute,
                        restored: false,
                    });
                }
            }
            if start.elapsed() > Duration::from_secs(5) {
                bail!("owned audio session not found");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    pub(super) fn set(&self, volume: f32, muted: bool) -> Result<()> {
        unsafe { self.volume.SetMasterVolume(volume, std::ptr::null())? };
        unsafe { self.volume.SetMute(muted, std::ptr::null())? };
        Ok(())
    }
    pub(super) fn restore(&mut self) -> Result<Restoration> {
        // Attempt both settings even when the first API fails. Read back
        // both values before claiming restoration succeeded.
        let volume_result = unsafe {
            self.volume
                .SetMasterVolume(self.before_volume, std::ptr::null())
        };
        let mute_result = unsafe { self.volume.SetMute(self.before_mute, std::ptr::null()) };
        let volume = unsafe { self.volume.GetMasterVolume() };
        let muted = unsafe { self.volume.GetMute() };
        volume_result?;
        mute_result?;
        let volume = volume?;
        let muted = muted?.as_bool();
        self.restored = (volume - self.before_volume).abs() <= 1e-6 && muted == self.before_mute;
        Ok(Restoration {
            volume,
            muted,
            verified: self.restored,
        })
    }
}
impl Drop for SessionGuard {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore();
        }
    }
}
