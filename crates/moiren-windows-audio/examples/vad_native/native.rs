//! One explicitly identified exclusive WASAPI client, owned on the probe thread.
use crate::{
    NativeMode,
    period::PeriodEvidence,
    validation::{self, Observation, PositionProgress},
};
use anyhow::{Context, ensure};
use moiren_windows_audio::vad::{
    Channels, Intent, MAX_PACKET_FRAMES, SampleFormat, decode_interleaved, encode_interleaved,
};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{HANDLE, S_OK, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
            AUDCLNT_E_BUFFER_ERROR, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioCaptureClient, IAudioClient, IAudioClock,
            IAudioRenderClient, IMMDevice, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
            WAVEFORMATEXTENSIBLE_0,
        },
        System::{
            Com::CLSCTX_ALL,
            Threading::{CreateEventW, WaitForSingleObject},
        },
    },
    core::{GUID, Interface},
};

fn capture_packet_available(result: windows::core::Result<()>) -> anyhow::Result<bool> {
    match result {
        Ok(()) => Ok(true),
        // Exclusive capture documents this as no packet for this processing
        // pass. The case still needs real matching frames before its deadline.
        Err(error) if error.code() == AUDCLNT_E_BUFFER_ERROR => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn stream_flags(mode: NativeMode) -> u32 {
    match mode {
        NativeMode::Event => AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        NativeMode::Timer => 0,
    }
}

enum Service {
    Render(IAudioRenderClient),
    Capture(IAudioCaptureClient),
}

pub(super) struct Stream {
    client: IAudioClient,
    event: Option<OwnedHandle>,
    clock: Option<IAudioClock>,
    service: Service,
    active: bool,
    pub frames: u32,
    format: SampleFormat,
    channels: Channels,
    encoded: Vec<u8>,
    decoded: Vec<f32>,
    pub discontinuities: u64,
    pub position: PositionProgress,
}

pub(super) fn descriptor(format: SampleFormat, channels: Channels) -> WAVEFORMATEXTENSIBLE {
    let bits = (format.bytes_per_sample() * 8) as u16;
    let align = channels as u16 * bits / 8;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xfffe,
            nChannels: channels as u16,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * u32::from(align),
            nBlockAlign: align,
            wBitsPerSample: bits,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: bits,
        },
        dwChannelMask: if channels == Channels::Mono { 4 } else { 3 },
        SubFormat: GUID::from_u128(match format {
            SampleFormat::Pcm16 => 0x00000001_0000_0010_8000_00aa00389b71,
            SampleFormat::Float32 => 0x00000003_0000_0010_8000_00aa00389b71,
        }),
    }
}

impl Stream {
    pub fn prepare(
        device: &IMMDevice,
        intent: Intent,
        format: SampleFormat,
        channels: Channels,
        pattern: [f32; 2],
        mode: NativeMode,
        period_evidence: &mut PeriodEvidence,
    ) -> anyhow::Result<Self> {
        let wave = descriptor(format, channels);
        // SAFETY: complete descriptor, apartment-local COM objects, no reference
        // escapes Initialize. No fallback format or Windows SRC is requested.
        let mut client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let supported =
            unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &wave.Format, None) };
        supported
            .ok()
            .context("IsFormatSupported(exact exclusive format)")?;
        ensure!(supported == S_OK, "exclusive format was not an exact match");
        let (mut default_hns, mut minimum_hns) = (0, 0);
        unsafe { client.GetDevicePeriod(Some(&mut default_hns), Some(&mut minimum_hns))? };
        let mut period = period_evidence.select(default_hns, minimum_hns)?;
        let initialize = |client: &IAudioClient, period| unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                stream_flags(mode),
                period,
                period,
                &wave.Format,
                None,
            )
        };
        let initial = initialize(&client, period);
        if initial
            .as_ref()
            .is_err_and(|e| e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
        {
            let frames = unsafe { client.GetBufferSize()? };
            ensure!(
                (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
                "alignment buffer out of bounds: {frames}"
            );
            period = (u64::from(frames) * 10_000_000).div_ceil(48_000) as i64;
            // Record the attempted retry before fresh activation can fail. This
            // request is not a measurement of the eventual native cadence.
            period_evidence.record_alignment_retry(period);
            // Failed Initialize needs a fresh client, not a second Initialize.
            drop(client);
            client = unsafe { device.Activate(CLSCTX_ALL, None)? };
            initialize(&client, period).context("Initialize(aligned exclusive)")?;
        } else {
            initial.context("Initialize(exclusive)")?;
        }
        let frames = unsafe { client.GetBufferSize()? };
        ensure!(
            (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
            "native buffer out of bounds: {frames}"
        );
        let event = if mode == NativeMode::Event {
            // SAFETY: auto-reset, unnamed, non-inheritable event transferred once.
            let event = unsafe { CreateEventW(None, false, false, None)? };
            let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
            unsafe { client.SetEventHandle(HANDLE(event.as_raw_handle()))? };
            Some(event)
        } else {
            None // Timer mode never creates or registers an event handle.
        };
        let clock: Option<IAudioClock> = if mode == NativeMode::Timer {
            Some(unsafe { client.GetService() }.context("GetService(IAudioClock, timer)")?)
        } else {
            None
        };
        let mut position = PositionProgress::default();
        if let Some(clock) = &clock {
            position.frequency = unsafe { clock.GetFrequency() }.context("GetFrequency(timer)")?;
            ensure!(
                position.frequency > 0,
                "native timer clock frequency is zero"
            );
        }
        let service = match intent {
            Intent::Playback => Service::Render(unsafe { client.GetService()? }),
            Intent::Microphone => Service::Capture(unsafe { client.GetService()? }),
        };
        let mut result = Self {
            client,
            event,
            clock,
            service,
            active: false,
            frames,
            format,
            channels,
            encoded: vec![0; frames as usize * channels as usize * format.bytes_per_sample()],
            decoded: vec![0.0; frames as usize * 2],
            discontinuities: 0,
            position,
        };
        encode_interleaved(
            &pattern.repeat(frames as usize),
            &mut result.encoded,
            format,
            channels,
        )
        .map_err(|error| anyhow::anyhow!("native pattern encoding: {error:?}"))?;
        Ok(result)
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        if let Service::Render(service) = &self.service {
            // Gate admission clears native storage. Prime only after that
            // transaction, immediately before Start, or timer mode consumes
            // an erased first buffer despite having PCM reads already queued.
            self.render(service, self.frames)?;
        }
        // SAFETY: fully initialized client; Drop stops on every later error.
        unsafe { self.client.Start() }.context("Start(exclusive)")?;
        self.active = true;
        Ok(())
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        if self.active {
            unsafe { self.client.Stop() }.context("Stop(exclusive)")?;
            self.active = false;
        }
        Ok(())
    }

    pub fn buffer_duration(&self) -> Duration {
        Duration::from_nanos((u64::from(self.frames) * 1_000_000_000).div_ceil(48_000))
    }

    pub fn poll(&mut self, expected: [f32; 2], seen: &mut Observation) -> anyhow::Result<()> {
        // Probe polling is control-side and is not a production timing path.
        if let Some(event) = &self.event {
            match unsafe { WaitForSingleObject(HANDLE(event.as_raw_handle()), 0) } {
                WAIT_TIMEOUT => return Ok(()),
                WAIT_OBJECT_0 => {}
                WAIT_FAILED => {
                    return Err(windows::core::Error::from_thread()).context("native event wait");
                }
                status => anyhow::bail!("unexpected native event result {}", status.0),
            }
        } else {
            self.sample_position()?;
        }
        let available = if self.event.is_none() {
            let padding = unsafe { self.client.GetCurrentPadding() }
                .context("GetCurrentPadding(exclusive timer)")?;
            // Capture's next packet can grow after this snapshot; the leased
            // GetBuffer frame count below, not this value, bounds the copy.
            match &self.service {
                Service::Render(_) => validation::timer_render_frames(self.frames, padding)?,
                Service::Capture(_) => {
                    ensure!(
                        padding <= self.frames,
                        "capture padding exceeds native capacity"
                    );
                    padding
                }
            }
        } else {
            self.frames
        };
        if available == 0 {
            return Ok(());
        }
        match &self.service {
            Service::Render(service) => self.render(service, available),
            Service::Capture(service) => {
                let mut pointer = std::ptr::null_mut();
                let (mut frames, mut flags) = (0, 0);
                // SAFETY: initialized capture client, writable packet outputs.
                // S_BUFFER_EMPTY is a successful call with zero frames.
                let available = capture_packet_available(unsafe {
                    service.GetBuffer(&mut pointer, &mut frames, &mut flags, None, None)
                })
                .context("GetBuffer(exclusive capture)")?;
                if !available || frames == 0 {
                    return Ok(());
                }
                let result = (|| -> anyhow::Result<()> {
                    ensure!(
                        frames <= self.frames,
                        "capture packet exceeds allocated buffer"
                    );
                    let decoded = &mut self.decoded[..frames as usize * 2];
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                        decoded.fill(0.0);
                    } else {
                        ensure!(!pointer.is_null(), "capture packet is null");
                        let length = frames as usize
                            * self.channels as usize
                            * self.format.bytes_per_sample();
                        // SAFETY: WASAPI owns this validated frame count until
                        // ReleaseBuffer below. No borrowed sample escapes it.
                        let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
                        decode_interleaved(bytes, decoded, self.format, self.channels).map_err(
                            |error| anyhow::anyhow!("native capture decoding: {error:?}"),
                        )?;
                    }
                    if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                        self.discontinuities += 1;
                    }
                    seen.observe(decoded, expected)
                })();
                // Return the lease even when PCM validation fails.
                let released = unsafe { service.ReleaseBuffer(frames) };
                result?;
                released.context("ReleaseBuffer(capture)")?;
                Ok(())
            }
        }
    }

    fn sample_position(&mut self) -> anyhow::Result<()> {
        let clock = self
            .clock
            .as_ref()
            .context("timer clock was not prepared")?;
        let (mut position, mut qpc) = (0, 0);
        // Preserve S_FALSE: low-accuracy samples cannot prove clock progress.
        let status = unsafe {
            (Interface::vtable(clock).GetPosition)(
                Interface::as_raw(clock),
                &mut position,
                &mut qpc,
            )
        };
        status.ok().context("GetPosition(exclusive timer)")?;
        if status == S_OK {
            self.position.observe(position, qpc)?;
        }
        Ok(())
    }

    fn render(&self, service: &IAudioRenderClient, frames: u32) -> anyhow::Result<()> {
        ensure!(
            frames > 0 && frames <= self.frames,
            "invalid render frame count"
        );
        // Event mode submits the whole buffer; timer mode only the free prefix.
        // One bounded transfer per poll, never a catch-up loop after a stall.
        let bytes = frames as usize * self.channels as usize * self.format.bytes_per_sample();
        let pointer = unsafe { service.GetBuffer(frames)? };
        ensure!(!pointer.is_null(), "render packet is null");
        // SAFETY: GetBuffer granted exactly frames and the descriptor has the
        // checked block alignment. Initialized bytes are copied before release.
        unsafe {
            std::ptr::copy_nonoverlapping(self.encoded.as_ptr(), pointer, bytes);
            service.ReleaseBuffer(frames, 0)?;
        }
        Ok(())
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::HRESULT;

    #[test]
    fn timer_initialization_never_requests_an_event_callback() {
        assert_eq!(
            stream_flags(NativeMode::Event),
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        );
        assert_eq!(stream_flags(NativeMode::Timer), 0);
    }

    #[test]
    fn exclusive_descriptors_match_exact_native_shapes() {
        for format in [SampleFormat::Pcm16, SampleFormat::Float32] {
            for channels in [Channels::Mono, Channels::Stereo] {
                let descriptor = descriptor(format, channels);
                let wave = descriptor.Format;
                assert_eq!({ wave.nSamplesPerSec }, 48_000);
                assert_eq!(
                    usize::from(wave.nBlockAlign),
                    channels as usize * format.bytes_per_sample()
                );
                assert_eq!(
                    { descriptor.dwChannelMask },
                    if channels == Channels::Mono { 4 } else { 3 }
                );
                assert_eq!(unsafe { descriptor.Samples.wValidBitsPerSample }, {
                    wave.wBitsPerSample
                });
            }
        }
    }

    #[test]
    fn exclusive_capture_buffer_error_is_one_empty_poll() {
        assert!(
            !capture_packet_available(Err(windows::core::Error::from_hresult(
                AUDCLNT_E_BUFFER_ERROR,
            )))
            .unwrap()
        );
        assert!(capture_packet_available(Ok(())).unwrap());
        assert!(
            capture_packet_available(Err(windows::core::Error::from_hresult(
                HRESULT::from_win32(5),
            )))
            .is_err()
        );
    }
}
