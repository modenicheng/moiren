//! One explicitly identified WASAPI client, owned on the probe thread.
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
            AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_E_BUFFER_ERROR,
            AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_SHAREMODE, AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioCaptureClient,
            IAudioClient, IAudioClock, IAudioRenderClient, IMMDevice, WAVEFORMATEX,
            WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
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
        NativeMode::Event | NativeMode::SharedCapture => AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        NativeMode::Timer => 0,
    }
}

fn initialization_parameters(mode: NativeMode, period: i64) -> (AUDCLNT_SHAREMODE, u32, i64, i64) {
    if mode == NativeMode::SharedCapture {
        (AUDCLNT_SHAREMODE_SHARED, stream_flags(mode), 0, 0)
    } else {
        (
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            stream_flags(mode),
            period,
            period,
        )
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(super) struct CaptureDiagnostics {
    pub packets: u64,
    pub frames: u64,
    pub first_flags: Option<u32>,
    pub flags_or: u32,
    pub silent_flag_packets: u64,
    pub silent_flag_frames: u64,
    pub discontinuity_packets: u64,
    pub timestamp_error_packets: u64,
    pub samples: u64,
    pub nonzero_samples: u64,
    pub nonfinite_samples: u64,
    pub peak_abs: f32,
    pub first_samples: [f32; 16],
    pub first_sample_count: usize,
    pub first_nonzero_frame: Option<[f32; 2]>,
}

impl CaptureDiagnostics {
    fn packet(&mut self, frames: u32, flags: u32) {
        self.packets += 1;
        self.frames += u64::from(frames);
        self.first_flags.get_or_insert(flags);
        self.flags_or |= flags;
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            self.silent_flag_packets += 1;
            self.silent_flag_frames += u64::from(frames);
        }
        self.discontinuity_packets +=
            u64::from(flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0);
        self.timestamp_error_packets +=
            u64::from(flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0);
    }

    fn samples(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.samples += 1;
            if !sample.is_finite() {
                self.nonfinite_samples += 1;
                continue;
            }
            self.nonzero_samples += u64::from(sample != 0.0);
            self.peak_abs = self.peak_abs.max(sample.abs());
            if self.first_sample_count < self.first_samples.len() {
                self.first_samples[self.first_sample_count] = sample;
                self.first_sample_count += 1;
            }
        }
        if self.first_nonzero_frame.is_none() {
            self.first_nonzero_frame = samples
                .as_chunks::<2>()
                .0
                .iter()
                .find(|frame| {
                    frame.iter().all(|sample| sample.is_finite())
                        && frame.iter().any(|sample| *sample != 0.0)
                })
                .copied();
        }
    }
}

pub(super) fn observe_capture_samples(
    metrics: Option<&mut CaptureDiagnostics>,
    decoded: &[f32],
    expected: [f32; 2],
    seen: &mut Observation,
) -> anyhow::Result<()> {
    // Retain the actual packet even when its samples fail pattern validation.
    if let Some(metrics) = metrics {
        metrics.samples(decoded);
    }
    seen.observe(decoded, expected)
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
    mode: NativeMode,
    pub capture_diagnostics: Option<CaptureDiagnostics>,
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
        ensure!(
            mode != NativeMode::SharedCapture
                || (intent == Intent::Microphone
                    && channels == Channels::Stereo
                    && format == SampleFormat::Pcm16),
            "SharedCapture requires owned stereo PCM16 microphone"
        );
        // SAFETY: complete descriptor, apartment-local COM objects, no reference
        // escapes Initialize. No fallback format or Windows SRC is requested.
        let mut client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        if mode == NativeMode::SharedCapture {
            ensure!(
                !period_evidence.minimum_period,
                "SharedCapture refuses minimum period"
            );
            let (share, flags, duration, periodicity) = initialization_parameters(mode, 0);
            period_evidence.initial_request_hns = Some(duration);
            period_evidence.final_request_hns = Some(duration);
            // Exact client PCM16 descriptor; no AUTOCONVERTPCM, closest format,
            // alignment retry or Exclusive fallback is requested.
            unsafe { client.Initialize(share, flags, duration, periodicity, &wave.Format, None) }
                .context("Initialize(exact shared PCM16 capture)")?;
        } else {
            let supported = unsafe {
                client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &wave.Format, None)
            };
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
        }
        let frames = unsafe { client.GetBufferSize()? };
        ensure!(
            (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
            "native buffer out of bounds: {frames}"
        );
        let event = if mode != NativeMode::Timer {
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
            mode,
            capture_diagnostics: (mode == NativeMode::SharedCapture)
                .then(CaptureDiagnostics::default),
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
        unsafe { self.client.Start() }.with_context(|| {
            if self.mode == NativeMode::SharedCapture {
                "Start(shared capture)"
            } else {
                "Start(exclusive)"
            }
        })?;
        self.active = true;
        Ok(())
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        if self.active {
            unsafe { self.client.Stop() }.with_context(|| {
                if self.mode == NativeMode::SharedCapture {
                    "Stop(shared capture)"
                } else {
                    "Stop(exclusive)"
                }
            })?;
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
                WAIT_TIMEOUT if self.mode != NativeMode::SharedCapture => return Ok(()),
                WAIT_TIMEOUT => {}
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
                let available = if self.mode == NativeMode::SharedCapture {
                    // One packet per poll bounds the drain; all three lease
                    // operations remain on this same probe-owner thread.
                    if unsafe { service.GetNextPacketSize() }
                        .context("GetNextPacketSize(shared capture)")?
                        == 0
                    {
                        return Ok(());
                    }
                    unsafe { service.GetBuffer(&mut pointer, &mut frames, &mut flags, None, None) }
                        .context("GetBuffer(shared capture)")?;
                    true
                } else {
                    capture_packet_available(unsafe {
                        service.GetBuffer(&mut pointer, &mut frames, &mut flags, None, None)
                    })
                    .context("GetBuffer(exclusive capture)")?
                };
                if !available || frames == 0 {
                    return Ok(());
                }
                if let Some(metrics) = &mut self.capture_diagnostics {
                    metrics.packet(frames, flags);
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
                    observe_capture_samples(
                        self.capture_diagnostics.as_mut(),
                        decoded,
                        expected,
                        seen,
                    )
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
    fn shared_capture_initialization_is_exact_event_zero_duration_and_periodicity() {
        let (share, flags, duration, periodicity) =
            initialization_parameters(NativeMode::SharedCapture, 100_000);
        assert_eq!(
            share,
            windows::Win32::Media::Audio::AUDCLNT_SHAREMODE_SHARED
        );
        assert_eq!(flags, AUDCLNT_STREAMFLAGS_EVENTCALLBACK);
        assert_eq!((duration, periodicity), (0, 0));
        for mode in [NativeMode::Event, NativeMode::Timer] {
            let (share, flags, duration, periodicity) = initialization_parameters(mode, 100_000);
            assert_eq!(share, AUDCLNT_SHAREMODE_EXCLUSIVE);
            assert_eq!(flags, stream_flags(mode));
            assert_eq!((duration, periodicity), (100_000, 100_000));
        }
    }

    #[test]
    fn shared_capture_metrics_preserve_flags_samples_and_bounded_prefix() {
        let mut metrics = CaptureDiagnostics::default();
        metrics.packet(480, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
        metrics.samples(&[0.0; 960]);
        metrics.packet(2, AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 | 4);
        metrics.samples(&[0.25, -0.5, 0.0, f32::NAN]);
        assert_eq!(metrics.packets, 2);
        assert_eq!(metrics.frames, 482);
        assert_eq!(
            metrics.first_flags,
            Some(AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
        );
        assert_eq!(metrics.silent_flag_packets, 1);
        assert_eq!(metrics.silent_flag_frames, 480);
        assert_eq!(metrics.discontinuity_packets, 1);
        assert_eq!(metrics.timestamp_error_packets, 1);
        assert_eq!(metrics.samples, 964);
        assert_eq!(metrics.nonzero_samples, 2);
        assert_eq!(metrics.nonfinite_samples, 1);
        assert_eq!(metrics.peak_abs, 0.5);
        assert_eq!(metrics.first_sample_count, 16);
        assert_eq!(metrics.first_samples, [0.0; 16]);
        assert!(serde_json::to_value(metrics).is_ok());
    }

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
