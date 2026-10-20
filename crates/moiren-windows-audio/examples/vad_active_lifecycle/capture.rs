//! Apartment-local exclusive capture. No data is interpreted as PCM evidence.

use anyhow::{Context, ensure};
use moiren_windows_audio::vad::MAX_PACKET_FRAMES;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::{
    Foundation::{HANDLE, S_OK, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Media::Audio::{
        AUDCLNT_E_BUFFER_ERROR, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_SHAREMODE_EXCLUSIVE,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioCaptureClient, IAudioClient, IMMDevice,
        WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
    },
    System::{
        Com::CLSCTX_ALL,
        Threading::{CreateEventW, WaitForSingleObject},
    },
};
use windows::core::GUID;

pub(super) struct Capture {
    client: IAudioClient,
    service: IAudioCaptureClient,
    event: OwnedHandle,
    running: bool,
    pub frames: u32,
}

fn descriptor() -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xfffe,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 192_000,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 16,
        },
        dwChannelMask: 3,
        SubFormat: GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71),
    }
}

impl Capture {
    pub fn prepare(device: &IMMDevice) -> anyhow::Result<Self> {
        let wave = descriptor();
        // SAFETY: exact initialized format and apartment-local COM ownership.
        let mut client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let supported =
            unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &wave.Format, None) };
        supported.ok().context("IsFormatSupported(stereo PCM16)")?;
        ensure!(
            supported == S_OK,
            "exclusive capture format not an exact match"
        );
        let mut period = 0;
        unsafe { client.GetDevicePeriod(Some(&mut period), None)? };
        ensure!(
            (1..=1_000_000).contains(&period),
            "capture period out of bounds"
        );
        period = period.max(100_000);
        let initialize = |client: &IAudioClient, period| unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                period,
                period,
                &wave.Format,
                None,
            )
        };
        let initialized = initialize(&client, period);
        if initialized
            .as_ref()
            .is_err_and(|e| e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
        {
            let frames = unsafe { client.GetBufferSize()? };
            ensure!(
                (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
                "alignment capacity invalid"
            );
            period = (u64::from(frames) * 10_000_000).div_ceil(48_000) as i64;
            // Failed Initialize requires a fresh client, never a second init.
            drop(client);
            client = unsafe { device.Activate(CLSCTX_ALL, None)? };
            initialize(&client, period).context("Initialize(aligned capture)")?;
        } else {
            initialized.context("Initialize(exclusive capture)")?;
        }
        let frames = unsafe { client.GetBufferSize()? };
        ensure!(
            (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
            "capture capacity invalid"
        );
        // SAFETY: transfer successful unnamed non-inheritable event exactly once.
        let event = unsafe { CreateEventW(None, false, false, None)? };
        let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
        unsafe { client.SetEventHandle(HANDLE(event.as_raw_handle()))? };
        let service = unsafe { client.GetService()? };
        Ok(Self {
            client,
            service,
            event,
            running: false,
            frames,
        })
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        unsafe { self.client.Start() }.context("Start(exclusive capture)")?;
        self.running = true;
        Ok(())
    }

    pub fn poll_discard(&self) -> anyhow::Result<u32> {
        // Never retain a packet lease over sleep, remove, or a control request.
        match unsafe { WaitForSingleObject(HANDLE(self.event.as_raw_handle()), 0) } {
            WAIT_TIMEOUT => return Ok(0),
            WAIT_OBJECT_0 => {}
            WAIT_FAILED => {
                return Err(windows::core::Error::from_thread()).context("capture event");
            }
            status => anyhow::bail!("unexpected capture wait result {}", status.0),
        }
        let mut pointer = std::ptr::null_mut();
        let (mut frames, mut flags) = (0, 0);
        let captured = unsafe {
            self.service
                .GetBuffer(&mut pointer, &mut frames, &mut flags, None, None)
        };
        match captured {
            Err(error) if error.code() == AUDCLNT_E_BUFFER_ERROR => return Ok(0),
            Err(error) => return Err(error).context("GetBuffer(active capture)"),
            Ok(()) if frames == 0 => return Ok(0),
            Ok(()) => {}
        }
        // The payload is deliberately not read: stage 1 measures lifetime only.
        let released = unsafe { self.service.ReleaseBuffer(frames) };
        ensure!(
            frames <= self.frames,
            "capture packet exceeds native capacity"
        );
        released.context("ReleaseBuffer(active capture)")?;
        Ok(frames)
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        if self.running {
            unsafe { self.client.Stop() }.context("Stop(old capture client)")?;
            self.running = false;
        }
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_one_is_exact_stereo_48khz_pcm16() {
        let wave = descriptor();
        assert_eq!({ wave.Format.nChannels }, 2);
        assert_eq!({ wave.Format.nSamplesPerSec }, 48_000);
        assert_eq!({ wave.Format.wBitsPerSample }, 16);
        assert_eq!({ wave.Format.nBlockAlign }, 4);
        assert_eq!({ wave.dwChannelMask }, 3);
    }
}
