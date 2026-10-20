//! Apartment-local native clients only; never creates a driver control owner.
use super::{
    identity,
    validation::{self, Candidate},
};
use anyhow::{Context, ensure};
use moiren_windows_audio::vad::{
    Intent, MAX_PACKET_FRAMES, StableEndpointId, discover_present_interfaces,
};
use serde::Serialize;
use std::{
    marker::PhantomData,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    rc::Rc,
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, S_OK, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
            AUDCLNT_E_BUFFER_ERROR, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioCaptureClient, IAudioClient,
            IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
            WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
            },
            Threading::{CreateEventW, WaitForSingleObject},
        },
    },
    core::{GUID, HSTRING},
};

struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn new() -> anyhow::Result<Self> {
        // This dedicated coordinator has no previous COM ownership. Both success
        // statuses require one uninitialize, after every client has been dropped.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct IdentityEvidence {
    pub root_instance: String,
    pub playback_stable_id: String,
    pub microphone_stable_id: String,
    pub playback_mmdevice_id: String,
    pub microphone_mmdevice_id: String,
    pub playback_adapter_instance: String,
    pub microphone_adapter_instance: String,
    pub render_buffer_frames: u32,
    pub capture_buffer_frames: u32,
    pub format: &'static str,
}

pub(super) struct Pair {
    render: Stream,
    capture: Stream,
    pub identity: IdentityEvidence,
    // Fields drop in declaration order: COM objects must precede the apartment.
    _apartment: Apartment,
}

impl Pair {
    pub fn prepare(ids: [StableEndpointId; 2], deadline: Instant) -> anyhow::Result<Self> {
        let apartment = Apartment::new()?;
        // Read-only SetupAPI identity resolution. No ControlClient/PcmOwner here.
        let interfaces = discover_present_interfaces()?;
        ensure!(
            interfaces.len() == 1,
            "expected one present VAD root, found {}",
            interfaces.len()
        );
        let root = identity::root_instance(&interfaces[0])?;
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let [playback, microphone] = loop {
            let (candidates, errors) = identity::candidates(&enumerator)?;
            let playback = validation::select(&candidates, ids[0], Intent::Playback, &root)?;
            let microphone = validation::select(&candidates, ids[1], Intent::Microphone, &root)?;
            if let (Some(playback), Some(microphone)) = (playback, microphone) {
                break [playback.clone(), microphone.clone()];
            }
            ensure!(
                Instant::now() < deadline,
                "owned MMDevices did not arrive; topology errors: {errors:?}"
            );
            thread::sleep(Duration::from_millis(20));
        };
        let render_device = exact_device(&enumerator, &playback, ids[0], &root)?;
        let capture_device = exact_device(&enumerator, &microphone, ids[1], &root)?;
        let render = Stream::prepare(&render_device, Intent::Playback)?;
        let capture = Stream::prepare(&capture_device, Intent::Microphone)?;
        let evidence = IdentityEvidence {
            root_instance: root,
            playback_stable_id: ids[0].to_string(),
            microphone_stable_id: ids[1].to_string(),
            playback_mmdevice_id: playback.id,
            microphone_mmdevice_id: microphone.id,
            playback_adapter_instance: playback.raw_instance,
            microphone_adapter_instance: microphone.raw_instance,
            render_buffer_frames: render.frames,
            capture_buffer_frames: capture.frames,
            format: "48000 Hz stereo Float32 exclusive event-driven, no SRC/fallback",
        };
        Ok(Self {
            render,
            capture,
            identity: evidence,
            _apartment: apartment,
        })
    }

    pub fn start(&mut self, started: &mut u32) -> anyhow::Result<()> {
        validation::start_clients(started, || self.capture.start(), || self.render.start())
    }

    pub fn set_signal(&mut self, signal: [f32; 2]) {
        self.render.pattern = signal;
    }

    pub fn pump(
        &mut self,
        observe: impl FnOnce(&[f32], bool) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.render.poll_render()?;
        self.capture.poll_capture(observe)
    }

    pub fn capture_running(&self) -> bool {
        self.capture.active
    }
    pub fn discontinuities(&self) -> u64 {
        self.capture.discontinuities
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        // Attempt both stops even if one fails; Drop repeats any failed Stop.
        let render = self.render.stop();
        let capture = self.capture.stop();
        render.and(capture)
    }
}

fn exact_device(
    enumerator: &IMMDeviceEnumerator,
    expected: &Candidate,
    stable: StableEndpointId,
    root: &str,
) -> anyhow::Result<IMMDevice> {
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(&expected.id))? };
    let actual = identity::candidate(enumerator, &device, expected.intent)?;
    ensure!(
        actual == *expected
            && validation::select(&[actual], stable, expected.intent, root)?.is_some(),
        "endpoint identity changed immediately before activation"
    );
    Ok(device)
}

fn descriptor() -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xfffe,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 384_000,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 32,
        },
        dwChannelMask: 3,
        SubFormat: GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71),
    }
}

enum Service {
    Render(IAudioRenderClient),
    Capture(IAudioCaptureClient),
}
struct Stream {
    client: IAudioClient,
    event: OwnedHandle,
    service: Service,
    active: bool,
    frames: u32,
    pattern: [f32; 2],
    discontinuities: u64,
    decoded: Vec<f32>,
}
impl Stream {
    fn prepare(device: &IMMDevice, intent: Intent) -> anyhow::Result<Self> {
        let wave = descriptor();
        let mut client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let supported =
            unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &wave.Format, None) };
        supported.ok().context("exact exclusive Float32 format")?;
        ensure!(supported == S_OK, "native format was not an exact match");
        let mut period = 0;
        unsafe {
            client.GetDevicePeriod(Some(&mut period), None)?;
        }
        ensure!(
            (1..=1_000_000).contains(&period),
            "invalid native period {period}"
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
        let initial = initialize(&client, period);
        if initial
            .as_ref()
            .is_err_and(|error| error.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
        {
            let frames = unsafe { client.GetBufferSize()? };
            ensure!(
                (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
                "alignment buffer too large"
            );
            period = (u64::from(frames) * 10_000_000).div_ceil(48_000) as i64;
            drop(client);
            client = unsafe { device.Activate(CLSCTX_ALL, None)? };
            initialize(&client, period).context("Initialize(aligned exclusive event)")?;
        } else {
            initial.context("Initialize(exclusive event)")?;
        }
        let frames = unsafe { client.GetBufferSize()? };
        ensure!(
            (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
            "native buffer too large"
        );
        // Own this auto-reset event exactly once, for at least the client lifetime.
        let event =
            unsafe { OwnedHandle::from_raw_handle(CreateEventW(None, false, false, None)?.0) };
        unsafe {
            client.SetEventHandle(HANDLE(event.as_raw_handle()))?;
        }
        let service = match intent {
            Intent::Playback => Service::Render(unsafe { client.GetService()? }),
            Intent::Microphone => Service::Capture(unsafe { client.GetService()? }),
        };
        let mut stream = Self {
            client,
            event,
            service,
            active: false,
            frames,
            pattern: validation::SIGNAL,
            discontinuities: 0,
            decoded: vec![0.0; frames as usize * 2],
        };
        if let Service::Render(render) = &stream.service {
            stream.render(render)?;
        }
        // Initialize does not start a client. Stage2 reports starts separately.
        stream.active = false;
        Ok(stream)
    }
    fn start(&mut self) -> anyhow::Result<()> {
        unsafe { self.client.Start() }.context("Start(exclusive event)")?;
        self.active = true;
        Ok(())
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        if self.active {
            unsafe { self.client.Stop() }.context("Stop(native client)")?;
            self.active = false;
        }
        Ok(())
    }
    fn event_ready(&self) -> anyhow::Result<bool> {
        match unsafe { WaitForSingleObject(HANDLE(self.event.as_raw_handle()), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(windows::core::Error::from_thread()).context("native event wait"),
            status => anyhow::bail!("unexpected event wait {}", status.0),
        }
    }
    fn poll_render(&self) -> anyhow::Result<()> {
        if !self.event_ready()? {
            return Ok(());
        }
        let Service::Render(render) = &self.service else {
            anyhow::bail!("not a render client");
        };
        self.render(render)
    }
    fn render(&self, render: &IAudioRenderClient) -> anyhow::Result<()> {
        let pointer = unsafe { render.GetBuffer(self.frames)? };
        let result = (|| -> anyhow::Result<()> {
            ensure!(!pointer.is_null(), "null render packet");
            // Float32 bytes are copied without assuming WASAPI pointer alignment.
            for frame in 0..self.frames as usize {
                for (channel, sample) in self.pattern.iter().enumerate() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            sample.to_le_bytes().as_ptr(),
                            pointer.add(frame * 8 + channel * 4),
                            4,
                        );
                    }
                }
            }
            Ok(())
        })();
        let released = unsafe { render.ReleaseBuffer(self.frames, 0) };
        result?;
        released.context("ReleaseBuffer(render)")
    }
    fn poll_capture(
        &mut self,
        observe: impl FnOnce(&[f32], bool) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if !self.event_ready()? {
            return Ok(());
        }
        let Service::Capture(capture) = &self.service else {
            anyhow::bail!("not a capture client");
        };
        let (mut pointer, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
        match unsafe { capture.GetBuffer(&mut pointer, &mut frames, &mut flags, None, None) } {
            Err(error) if error.code() == AUDCLNT_E_BUFFER_ERROR => return Ok(()),
            result => result.context("GetBuffer(capture)")?,
        }
        if frames == 0 {
            return Ok(());
        }
        let result = (|| -> anyhow::Result<()> {
            ensure!(frames <= self.frames, "capture lease exceeds capacity");
            let decoded = &mut self.decoded[..frames as usize * 2];
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                decoded.fill(0.0);
            } else {
                ensure!(!pointer.is_null(), "null capture packet");
                // Copy before ReleaseBuffer; no native lease escapes this poll.
                let bytes = unsafe { std::slice::from_raw_parts(pointer, frames as usize * 8) };
                for (sample, bytes) in decoded.iter_mut().zip(bytes.as_chunks::<4>().0) {
                    *sample = f32::from_le_bytes(*bytes);
                }
            }
            let discontinuity = flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0;
            if discontinuity {
                self.discontinuities += 1;
            }
            observe(decoded, discontinuity)
        })();
        let released = unsafe { capture.ReleaseBuffer(frames) };
        result?;
        released.context("ReleaseBuffer(capture)")
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
