//! Native playback client for the live VAD qualification probe.
//!
//! This is deliberately example-local. The production VAD provider owns the
//! driver PCM worker; this module owns only the external WASAPI client that
//! writes into the VAD playback MMDevice.

use super::source::{PreparedSource, SourceCursor, SourceProgress, SourceProvenance};
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::vad::{
    Channels, DeviceInterface, Intent, MAX_PACKET_FRAMES, SampleFormat, StableEndpointId,
    discover_present_interfaces, encode_interleaved,
};
use serde::Serialize;
use std::{
    marker::PhantomData,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_Get_Device_ID_Size, CM_Get_Device_IDW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL,
                CM_Locate_DevNodeW, CONFIGRET, CR_SUCCESS, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
                SP_DEVINFO_DATA, SetupDiCreateDeviceInfoList, SetupDiDestroyDeviceInfoList,
                SetupDiGetDeviceInterfaceDetailW, SetupDiOpenDeviceInterfaceW,
            },
            FunctionDiscovery::PKEY_Device_FriendlyName,
            FunctionDiscovery::PKEY_Device_InstanceId,
        },
        Foundation::{
            ERROR_INSUFFICIENT_BUFFER, HANDLE, S_OK, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED,
            AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE_ACTIVE,
            IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
            WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0, eRender,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize, STGM_READ,
            },
            Threading::{CreateEventW, WaitForSingleObject},
            Variant::VT_LPWSTR,
        },
    },
    core::{GUID, HSTRING, PWSTR},
};

const CONTROL_START_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize)]
pub struct PlaybackReport {
    pub stable_endpoint_id: String,
    pub mmdevice_id: String,
    pub observed_name: String,
    pub adapter_instance: String,
    pub native_buffer_frames: u32,
    pub packet_format: String,
    pub submitted_frames: u64,
    pub playback_source: SourceProvenance,
    pub source_progress: SourceProgress,
    pub event_timeouts: u64,
    pub elapsed_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackInfo {
    pub stable_endpoint_id: String,
    pub mmdevice_id: String,
    pub observed_name: String,
    pub adapter_instance: String,
    pub native_buffer_frames: u32,
    pub packet_format: String,
}

pub struct PlaybackSession {
    stop: Arc<AtomicBool>,
    activate: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<PlaybackReport>>>,
    pub info: PlaybackInfo,
}

impl PlaybackSession {
    /// Prepare the native stream without starting its clock. The VAD provider
    /// must prepare its worker against this stable stream generation before
    /// the gate is opened; otherwise the worker treats native stream creation
    /// as a recovery and closes the newly admitted gate.
    pub fn prepare(
        stable_id: StableEndpointId,
        root_instance: String,
        source_plan: Option<PreparedSource>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let activate = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_activate = activate.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("moiren-vad-native-playback".into())
            .spawn(move || {
                let result = run_worker(
                    stable_id,
                    &root_instance,
                    source_plan,
                    worker_stop,
                    worker_activate,
                    &ready_tx,
                );
                if let Err(error) = &result {
                    let _ = ready_tx.send(Err(format!("{error:#}")));
                }
                result
            })
            .context("spawn VAD native playback worker")?;
        let info = match ready_rx.recv_timeout(CONTROL_START_TIMEOUT) {
            Ok(Ok(info)) => info,
            Ok(Err(error)) => {
                let _ = worker.join();
                return Err(anyhow::anyhow!(error));
            }
            Err(error) => {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                return Err(anyhow::anyhow!("VAD native playback startup lost: {error}"));
            }
        };
        Ok(Self {
            stop,
            activate,
            worker: Some(worker),
            info,
        })
    }

    pub fn activate(&self) {
        self.activate.store(true, Ordering::Release);
    }

    pub fn stop(mut self) -> Result<PlaybackReport> {
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .expect("VAD playback worker")
            .join()
            .map_err(|_| anyhow::anyhow!("VAD native playback worker panicked"))?
    }
}

impl Drop for PlaybackSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Apartment(PhantomData<std::rc::Rc<()>>);

impl Apartment {
    fn new() -> Result<Self> {
        // SAFETY: this worker has no prior COM ownership and balances the call.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: exactly one successful CoInitializeEx belongs to this guard.
        unsafe { CoUninitialize() };
    }
}

struct DeviceSet(HDEVINFO);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns the SetupAPI set.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

struct TaskString(PWSTR);

impl Drop for TaskString {
    fn drop(&mut self) {
        // SAFETY: GetId allocates with the COM task allocator.
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

fn take_string(pointer: PWSTR) -> Result<String> {
    let owned = TaskString(pointer);
    ensure!(!owned.0.is_null(), "COM returned a null string");
    // SAFETY: the successful COM call returned a NUL-terminated UTF-16 string.
    Ok(String::from_utf16(unsafe { owned.0.as_wide() })?)
}

fn config(result: CONFIGRET, stage: &str) -> Result<()> {
    ensure!(result == CR_SUCCESS, "{stage}: CONFIGRET={}", result.0);
    Ok(())
}

fn instance(node: u32) -> Result<String> {
    let mut length = 0;
    // SAFETY: obtains the size for a live Configuration Manager node.
    config(
        unsafe { CM_Get_Device_ID_Size(&mut length, node, 0) },
        "CM_Get_Device_ID_Size",
    )?;
    ensure!(
        (1..=4096).contains(&length),
        "PnP identity length out of bounds"
    );
    let mut buffer = vec![0_u16; length as usize + 1];
    // SAFETY: the buffer includes the required terminating NUL.
    config(
        unsafe { CM_Get_Device_IDW(node, &mut buffer, 0) },
        "CM_Get_Device_IDW",
    )?;
    ensure!(buffer[length as usize] == 0, "unterminated PnP identity");
    Ok(String::from_utf16(&buffer[..length as usize])?)
}

fn root_instance(interface: &DeviceInterface) -> Result<String> {
    let path = interface.display_path();
    ensure!(
        path.is_ascii(),
        "control interface path is not lossless ASCII"
    );
    // SAFETY: creates an empty, owned SetupAPI device-info set.
    let set = DeviceSet(unsafe { SetupDiCreateDeviceInfoList(None, None)? });
    let mut data = SP_DEVICE_INTERFACE_DATA {
        cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
        ..Default::default()
    };
    // SAFETY: the set, path and output structure are live and initialized.
    unsafe { SetupDiOpenDeviceInterfaceW(set.0, &HSTRING::from(path), 0, Some(&mut data))? };
    let mut info = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    let result =
        unsafe { SetupDiGetDeviceInterfaceDetailW(set.0, &data, None, 0, None, Some(&mut info)) };
    ensure!(
        result.is_err_and(|error| error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult()),
        "unexpected SetupDiGetDeviceInterfaceDetailW result"
    );
    instance(info.DevInst)
}

fn candidate(
    enumerator: &IMMDeviceEnumerator,
    device: &IMMDevice,
    intent: Intent,
) -> Result<(String, String, String, String, Intent)> {
    // SAFETY: all COM objects remain in this worker apartment and topology is
    // used only to obtain the opaque adapter identity.
    unsafe {
        let id = take_string(device.GetId()?)?;
        let endpoint_properties = device.OpenPropertyStore(STGM_READ)?;
        let endpoint_name = endpoint_properties.GetValue(&PKEY_Device_FriendlyName)?;
        ensure!(
            endpoint_name.Anonymous.Anonymous.vt == VT_LPWSTR,
            "VAD endpoint lacks a friendly name"
        );
        let endpoint_name_pointer = endpoint_name.Anonymous.Anonymous.Anonymous.pwszVal;
        ensure!(
            !endpoint_name_pointer.is_null(),
            "VAD endpoint returned a null friendly name"
        );
        let observed_name = String::from_utf16(endpoint_name_pointer.as_wide())?;
        let topology: windows::Win32::Media::Audio::IDeviceTopology =
            device.Activate(CLSCTX_ALL, None)?;
        ensure!(
            topology.GetConnectorCount()? == 1,
            "endpoint has no unique connector"
        );
        let adapter_id = take_string(topology.GetConnector(0)?.GetDeviceIdConnectedTo()?)?;
        let adapter = enumerator.GetDevice(&HSTRING::from(adapter_id))?;
        let properties = adapter.OpenPropertyStore(STGM_READ)?;
        let property = properties.GetValue(&PKEY_Device_InstanceId)?;
        ensure!(
            property.Anonymous.Anonymous.vt == VT_LPWSTR,
            "adapter lacks a string PnP identity"
        );
        let pointer = property.Anonymous.Anonymous.Anonymous.pwszVal;
        ensure!(!pointer.is_null(), "adapter returned a null PnP identity");
        let raw_instance = String::from_utf16(pointer.as_wide())?;
        let mut node = 0;
        config(
            CM_Locate_DevNodeW(
                &mut node,
                &HSTRING::from(&raw_instance),
                CM_LOCATE_DEVNODE_NORMAL,
            ),
            "CM_Locate_DevNodeW",
        )?;
        let mut parent = 0;
        config(CM_Get_Parent(&mut parent, node, 0), "CM_Get_Parent")?;
        Ok((id, observed_name, raw_instance, instance(parent)?, intent))
    }
}

fn resolve_playback(
    enumerator: &IMMDeviceEnumerator,
    stable_id: StableEndpointId,
    root: &str,
) -> Result<(IMMDevice, String, String, String)> {
    let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
    let wanted = stable_id.to_string();
    let mut match_count = 0;
    let mut selected = None;
    for index in 0..unsafe { devices.GetCount()? } {
        let device = unsafe { devices.Item(index)? };
        let (id, observed_name, raw, parent, intent) =
            candidate(enumerator, &device, Intent::Playback)?;
        let owned = raw.rsplit_once('\\').is_some_and(|(hardware, suffix)| {
            hardware.eq_ignore_ascii_case(r"MOIREN\VAD_RAW")
                && suffix
                    .rsplit('&')
                    .next()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&wanted))
        });
        if owned && intent == Intent::Playback && parent.eq_ignore_ascii_case(root) {
            match_count += 1;
            selected = Some((device, id, observed_name, raw));
        }
    }
    ensure!(
        match_count == 1,
        "expected one VAD playback MMDevice for {wanted}, found {match_count}"
    );
    Ok(selected.expect("one owned VAD playback device"))
}

fn wait_playback(
    enumerator: &IMMDeviceEnumerator,
    stable_id: StableEndpointId,
    root: &str,
) -> Result<(IMMDevice, String, String, String)> {
    let deadline = Instant::now() + CONTROL_START_TIMEOUT;
    let mut last_error = match resolve_playback(enumerator, stable_id, root) {
        Ok(device) => return Ok(device),
        Err(error) => error,
    };
    loop {
        ensure!(
            Instant::now() < deadline,
            "VAD playback MMDevice was not published: {:#}",
            last_error
        );
        thread::sleep(Duration::from_millis(100));
        match resolve_playback(enumerator, stable_id, root) {
            Ok(device) => return Ok(device),
            Err(error) => last_error = error,
        }
    }
}

fn descriptor(format: SampleFormat) -> WAVEFORMATEXTENSIBLE {
    let bits = (format.bytes_per_sample() * 8) as u16;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xfffe,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 2 * format.bytes_per_sample() as u32,
            nBlockAlign: 2 * format.bytes_per_sample() as u16,
            wBitsPerSample: bits,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: bits,
        },
        dwChannelMask: 3,
        SubFormat: GUID::from_u128(match format {
            SampleFormat::Pcm16 => 0x00000001_0000_0010_8000_00aa00389b71,
            SampleFormat::Float32 => 0x00000003_0000_0010_8000_00aa00389b71,
        }),
    }
}

struct Stream {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: OwnedHandle,
    frames: u32,
    format: SampleFormat,
    encoded: Vec<u8>,
    samples: Vec<f32>,
    active: bool,
}

impl Stream {
    fn prepare(device: &IMMDevice) -> Result<Self> {
        let mut selected = None;
        for format in [SampleFormat::Float32, SampleFormat::Pcm16] {
            let wave = descriptor(format);
            // SAFETY: exact format descriptor and apartment-local device.
            let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
            let supported = unsafe {
                client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &wave.Format, None)
            };
            if supported.is_ok() && supported == S_OK {
                selected = Some((client, wave, format));
                break;
            }
        }
        let (mut client, wave, format) =
            selected.context("VAD MMDevice has no exact Float32/PCM16 format")?;
        let mut period = 0;
        unsafe { client.GetDevicePeriod(Some(&mut period), None)? };
        ensure!(
            (1_000..=1_000_000).contains(&period),
            "native period out of bounds: {period}"
        );
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
        let result = initialize(&client, period);
        if result
            .as_ref()
            .is_err_and(|error| error.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
        {
            let frames = unsafe { client.GetBufferSize()? };
            ensure!(
                (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
                "aligned buffer out of bounds"
            );
            period = (u64::from(frames) * 10_000_000).div_ceil(48_000) as i64;
            drop(client);
            client = unsafe { device.Activate(CLSCTX_ALL, None)? };
            initialize(&client, period).context("Initialize aligned VAD playback")?;
        } else {
            result.context("Initialize exclusive VAD playback")?;
        }
        let frames = unsafe { client.GetBufferSize()? };
        ensure!(
            (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
            "native buffer out of bounds"
        );
        // SAFETY: the event is uniquely owned and transferred into OwnedHandle.
        let event =
            unsafe { OwnedHandle::from_raw_handle(CreateEventW(None, false, false, None)?.0) };
        unsafe { client.SetEventHandle(HANDLE(event.as_raw_handle()))? };
        let render = unsafe { client.GetService()? };
        Ok(Self {
            client,
            render,
            event,
            frames,
            format,
            encoded: vec![0; frames as usize * 2 * format.bytes_per_sample()],
            samples: vec![0.0; frames as usize * 2],
            active: false,
        })
    }

    fn start(&mut self) -> Result<()> {
        // SAFETY: Initialize and event handle setup completed successfully.
        unsafe { self.client.Start()? };
        self.active = true;
        Ok(())
    }

    fn fill_and_submit(&mut self, source: &mut SourceCursor, preroll: bool) -> Result<()> {
        let silent_preroll = preroll && source.has_schedule();
        source.submit(&mut self.samples, preroll, |samples| {
            encode_interleaved(samples, &mut self.encoded, self.format, Channels::Stereo)
                .map_err(|error| anyhow::anyhow!("encode native VAD PCM: {error:?}"))?;
            let pointer = unsafe { self.render.GetBuffer(self.frames)? };
            ensure!(!pointer.is_null(), "VAD render buffer is null");
            // SAFETY: WASAPI granted exactly self.frames of the checked format.
            // Only successful ReleaseBuffer commits the portable source cursor.
            unsafe {
                std::ptr::copy_nonoverlapping(self.encoded.as_ptr(), pointer, self.encoded.len());
                self.render.ReleaseBuffer(
                    self.frames,
                    if silent_preroll {
                        AUDCLNT_BUFFERFLAGS_SILENT.0 as u32
                    } else {
                        0
                    },
                )?;
            }
            Ok(())
        })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: the client remains owned and initialized until Drop.
            let _ = unsafe { self.client.Stop() };
        }
    }
}

fn run_worker(
    stable_id: StableEndpointId,
    root: &str,
    source_plan: Option<PreparedSource>,
    stop: Arc<AtomicBool>,
    activate: Arc<AtomicBool>,
    ready_tx: &mpsc::SyncSender<std::result::Result<PlaybackInfo, String>>,
) -> Result<PlaybackReport> {
    let _apartment = Apartment::new()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let (device, mmdevice_id, observed_name, adapter_instance) =
        wait_playback(&enumerator, stable_id, root)?;
    let mut stream = Stream::prepare(&device)?;
    let mut source = SourceCursor::new(source_plan);
    stream.fill_and_submit(&mut source, true)?;
    let info = PlaybackInfo {
        stable_endpoint_id: stable_id.to_string(),
        mmdevice_id: mmdevice_id.clone(),
        observed_name: observed_name.clone(),
        adapter_instance: adapter_instance.clone(),
        native_buffer_frames: stream.frames,
        packet_format: format!("{:?}", stream.format),
    };
    ready_tx
        .send(Ok(info.clone()))
        .map_err(|_| anyhow::anyhow!("VAD playback startup receiver closed"))?;
    while !activate.load(Ordering::Acquire) {
        if stop.load(Ordering::Acquire) {
            return Ok(PlaybackReport {
                stable_endpoint_id: stable_id.to_string(),
                mmdevice_id,
                observed_name,
                adapter_instance,
                native_buffer_frames: stream.frames,
                packet_format: format!("{:?}", stream.format),
                submitted_frames: source.progress.native_submitted_frames,
                playback_source: source.provenance(),
                source_progress: source.progress.clone(),
                event_timeouts: 0,
                elapsed_seconds: 0.0,
            });
        }
        thread::sleep(Duration::from_millis(1));
    }
    stream.start()?;
    let started = Instant::now();
    let mut event_timeouts = 0;
    // PlaybackSession owns the stop/join fence, including its Drop path. Keep
    // the native stream alive until control has sampled the running provider.
    while !stop.load(Ordering::Acquire) {
        match unsafe { WaitForSingleObject(HANDLE(stream.event.as_raw_handle()), 100) } {
            WAIT_OBJECT_0 => {
                stream.fill_and_submit(&mut source, false)?;
            }
            WAIT_TIMEOUT => event_timeouts += 1,
            WAIT_FAILED => return Err(windows::core::Error::from_thread().into()),
            status => anyhow::bail!("unexpected VAD playback wait result {}", status.0),
        }
    }
    Ok(PlaybackReport {
        stable_endpoint_id: stable_id.to_string(),
        mmdevice_id,
        observed_name,
        adapter_instance,
        native_buffer_frames: stream.frames,
        packet_format: format!("{:?}", stream.format),
        submitted_frames: source.progress.native_submitted_frames,
        playback_source: source.provenance(),
        source_progress: source.progress.clone(),
        event_timeouts,
        elapsed_seconds: started.elapsed().as_secs_f64(),
    })
}

pub fn discover_root() -> Result<(DeviceInterface, String)> {
    let interfaces = discover_present_interfaces()?;
    ensure!(
        interfaces.len() == 1,
        "expected one present VAD control interface, found {}",
        interfaces.len()
    );
    let interface = interfaces.into_iter().next().expect("one interface");
    let root = root_instance(&interface)?;
    Ok((interface, root))
}
