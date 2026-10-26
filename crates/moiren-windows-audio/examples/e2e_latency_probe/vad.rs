//! Native playback client for the live VAD qualification probe.
//!
//! This is deliberately example-local. The production VAD provider owns the
//! driver PCM worker; this module owns only the external WASAPI client that
//! writes into the VAD playback MMDevice.

use super::source::{PreparedSource, SourceCursor, SourceProgress, SourceProvenance};
use super::vad_collection::PlaybackProgress;
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
        Arc, Mutex,
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
pub struct PlaybackOutcome {
    #[serde(flatten)]
    pub info: Option<PlaybackInfo>,
    pub submitted_frames: u64,
    pub playback_source: SourceProvenance,
    pub source_progress: SourceProgress,
    pub event_timeouts: u64,
    pub elapsed_seconds: f64,
    pub started: bool,
    /// True only after the explicit native Stop call succeeded.
    pub stopped: bool,
    pub worker_joined: bool,
    /// None means no explicit observation (for example worker unwind).
    pub native_stop_attempted: Option<bool>,
    pub native_stop_succeeded: Option<bool>,
    pub failure: Option<String>,
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
    worker: Option<JoinHandle<Result<()>>>,
    latest: Arc<Mutex<WorkerState>>,
    provenance: SourceProvenance,
    activation_attempted: bool,
    unexpected_exit: AtomicBool,
    pub info: PlaybackInfo,
}

#[derive(Default)]
struct WorkerState {
    info: Option<PlaybackInfo>,
    provenance: Option<SourceProvenance>,
    progress: PlaybackProgress,
    elapsed_seconds: f64,
    native_stop_attempted: Option<bool>,
    native_stop_succeeded: Option<bool>,
}

fn stop_and_publish(
    active: bool,
    latest: &Mutex<WorkerState>,
    primary: Result<()>,
    stop: impl FnOnce() -> Result<()>,
) -> Result<()> {
    {
        let mut state = latest.lock().unwrap_or_else(|p| p.into_inner());
        state.native_stop_attempted = Some(active);
        state.native_stop_succeeded = None;
        state.progress.stopped = false;
    }
    let cleanup = if active { stop() } else { Ok(()) };
    if active {
        let mut state = latest.lock().unwrap_or_else(|p| p.into_inner());
        state.native_stop_succeeded = Some(cleanup.is_ok());
        state.progress.stopped = cleanup.is_ok();
    }
    super::soak_cleanup::combine(primary, cleanup)
}

fn submit_and_publish(
    source: &mut SourceCursor,
    latest: &Mutex<WorkerState>,
    submit: impl FnOnce(&mut SourceCursor) -> Result<()>,
) -> Result<()> {
    // The existing source cursor commits only after successful ReleaseBuffer.
    // Native IO and marker generation run outside this short publication lock.
    submit(source)?;
    latest
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .progress
        .source = source.progress.clone();
    Ok(())
}

impl PlaybackSession {
    pub fn prepare(
        stable_id: StableEndpointId,
        root_instance: String,
        source_plan: Option<PreparedSource>,
    ) -> Result<Self> {
        Self::prepare_with_abort(stable_id, root_instance, source_plan, || {}, &mut None)
    }

    pub(super) fn prepare_with_abort(
        stable_id: StableEndpointId,
        root_instance: String,
        source_plan: Option<PreparedSource>,
        before_join: impl FnMut(),
        partial: &mut Option<PlaybackOutcome>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let activate = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(WorkerState::default()));
        let provenance = SourceProvenance::new(source_plan.as_ref());
        let worker_stop = stop.clone();
        let worker_activate = activate.clone();
        let worker_latest = latest.clone();
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
                    worker_latest,
                );
                // Ready may already occupy the one-slot channel after a startup
                // timeout. A final error publication must never block the join.
                if let Err(error) = &result {
                    let _ = ready_tx.try_send(Err(format!("{error:#}")));
                }
                result
            })
            .context("spawn VAD native playback worker")?;
        let mut guard = StartupGuard {
            worker: Some(worker),
            stop: stop.clone(),
            latest: latest.clone(),
            provenance: provenance.clone(),
            before_join,
            partial,
        };
        let info = match ready_rx.recv_timeout(CONTROL_START_TIMEOUT) {
            Ok(Ok(info)) => info,
            Ok(Err(error)) => return Err(anyhow::anyhow!(error)),
            Err(error) => anyhow::bail!("VAD native playback startup lost: {error}"),
        };
        let worker = guard.worker.take();
        Ok(Self {
            stop,
            activate,
            worker,
            latest,
            provenance,
            activation_attempted: false,
            unexpected_exit: AtomicBool::new(false),
            info,
        })
    }

    pub fn activate(&mut self) -> Result<()> {
        ensure!(
            !self.activation_attempted && !self.stop.load(Ordering::Acquire),
            "source activation already attempted or stopping"
        );
        self.activation_attempted = true;
        self.check()?;
        self.activate.store(true, Ordering::Release);
        Ok(())
    }
    pub fn request_stop(&self) {
        if !self.stop.load(Ordering::Acquire)
            && self
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
        {
            self.unexpected_exit.store(true, Ordering::Release);
        }
        self.stop.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<Option<PlaybackProgress>> {
        let worker = self
            .worker
            .as_ref()
            .context("source worker missing or finished")?;
        ensure!(
            self.stop.load(Ordering::Acquire) || !worker.is_finished(),
            "source worker exited before requested stop"
        );
        match self.latest.try_lock() {
            Ok(state) => Ok(Some(state.progress.clone())),
            Err(std::sync::TryLockError::WouldBlock) => Ok(None),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                anyhow::bail!("source progress publication poisoned")
            }
        }
    }
    pub fn finish(&mut self) -> Result<PlaybackOutcome> {
        self.request_stop();
        let worker = self
            .worker
            .take()
            .context("source worker missing or already joined")?;
        let mut outcome = join_outcome(worker, &self.latest, self.provenance.clone());
        if self.unexpected_exit.load(Ordering::Acquire) {
            super::vad_collection::append_failure(
                &mut outcome.failure,
                "source worker exited before requested stop",
            );
        }
        Ok(outcome)
    }
    pub fn stop(mut self) -> Result<PlaybackOutcome> {
        self.request_stop();
        self.finish()
    }
}

struct StartupGuard<'a, F: FnMut()> {
    worker: Option<JoinHandle<Result<()>>>,
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<WorkerState>>,
    provenance: SourceProvenance,
    before_join: F,
    partial: &'a mut Option<PlaybackOutcome>,
}

impl<F: FnMut()> Drop for StartupGuard<'_, F> {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            // The recorder already exists while this source is half prepared.
            // Broadcast its stop before the first possible blocking join.
            let aborted =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut self.before_join));
            self.stop.store(true, Ordering::Release);
            let mut outcome = join_outcome(worker, &self.latest, self.provenance.clone());
            if aborted.is_err() {
                super::vad_collection::append_failure(
                    &mut outcome.failure,
                    "source startup abort callback panicked",
                );
            }
            *self.partial = Some(outcome);
        }
    }
}

fn join_outcome(
    worker: JoinHandle<Result<()>>,
    latest: &Mutex<WorkerState>,
    provenance: SourceProvenance,
) -> PlaybackOutcome {
    let message = match worker.join() {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("{error:#}")),
        Err(_) => Some("VAD native playback worker panicked".into()),
    };
    let mut failure = None;
    if let Some(message) = message {
        super::vad_collection::append_failure(&mut failure, &message);
    }
    if latest.is_poisoned() {
        super::vad_collection::append_failure(&mut failure, "source progress publication poisoned");
    }
    let state = latest.lock().unwrap_or_else(|poison| poison.into_inner());
    PlaybackOutcome {
        info: state.info.clone(),
        submitted_frames: state.progress.source.native_submitted_frames,
        playback_source: state.provenance.clone().unwrap_or(provenance),
        source_progress: state.progress.source.clone(),
        event_timeouts: state.progress.event_timeouts,
        elapsed_seconds: state.elapsed_seconds,
        started: state.progress.started,
        stopped: state.progress.stopped,
        worker_joined: true,
        native_stop_attempted: state.native_stop_attempted,
        native_stop_succeeded: state.native_stop_succeeded,
        failure,
    }
}

impl Drop for PlaybackSession {
    fn drop(&mut self) {
        self.request_stop();
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
            // Explicit worker retirement records Stop errors. This fallback
            // cannot establish a successful Stop acknowledgement in a report.
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
    latest: Arc<Mutex<WorkerState>>,
) -> Result<()> {
    let _apartment = Apartment::new()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let (device, mmdevice_id, observed_name, adapter_instance) =
        wait_playback(&enumerator, stable_id, root)?;
    let mut stream = Stream::prepare(&device)?;
    let mut source = SourceCursor::new(source_plan);
    latest.lock().unwrap_or_else(|p| p.into_inner()).provenance = Some(source.provenance());
    let info = PlaybackInfo {
        stable_endpoint_id: stable_id.to_string(),
        mmdevice_id,
        observed_name,
        adapter_instance,
        native_buffer_frames: stream.frames,
        packet_format: format!("{:?}", stream.format),
    };
    let mut started = None;
    let result = (|| -> Result<()> {
        latest.lock().unwrap_or_else(|p| p.into_inner()).info = Some(info.clone());
        submit_and_publish(&mut source, &latest, |source| {
            stream.fill_and_submit(source, true)
        })?;
        ready_tx
            .send(Ok(info))
            .map_err(|_| anyhow::anyhow!("VAD playback startup receiver closed"))?;
        while !activate.load(Ordering::Acquire) {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(1));
        }
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        stream.start()?;
        started = Some(Instant::now());
        latest
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .progress
            .started = true;
        while !stop.load(Ordering::Acquire) {
            match unsafe { WaitForSingleObject(HANDLE(stream.event.as_raw_handle()), 100) } {
                WAIT_OBJECT_0 => {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    submit_and_publish(&mut source, &latest, |source| {
                        stream.fill_and_submit(source, false)
                    })?;
                    let mut state = latest.lock().unwrap_or_else(|p| p.into_inner());
                    state.elapsed_seconds = started.map_or(0.0, |t| t.elapsed().as_secs_f64());
                }
                WAIT_TIMEOUT => {
                    latest
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .progress
                        .event_timeouts += 1
                }
                WAIT_FAILED => return Err(windows::core::Error::from_thread().into()),
                status => anyhow::bail!("unexpected VAD playback wait result {}", status.0),
            }
        }
        Ok(())
    })();
    let stopped = stop_and_publish(stream.active, &latest, result, || unsafe {
        stream.client.Stop().map_err(anyhow::Error::from)
    });
    {
        let mut state = latest.lock().unwrap_or_else(|p| p.into_inner());
        if state.native_stop_succeeded == Some(true) {
            stream.active = false;
        }
        state.elapsed_seconds = started.map_or(0.0, |t| t.elapsed().as_secs_f64());
    }
    stopped
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_native_stop_ack_is_distinct_from_join_and_retains_both_errors() {
        let latest = Mutex::new(WorkerState::default());
        latest
            .lock()
            .unwrap()
            .progress
            .source
            .acknowledged_source_frames = 37;
        let result = stop_and_publish(
            true,
            &latest,
            Err(anyhow::anyhow!("primary PCM error")),
            || {
                assert!(latest.try_lock().is_ok());
                anyhow::bail!("native Stop HRESULT failure")
            },
        );
        let text = format!("{:#}", result.unwrap_err());
        assert!(text.contains("primary PCM error") && text.contains("native Stop HRESULT failure"));
        let state = latest.lock().unwrap();
        assert_eq!(state.native_stop_attempted, Some(true));
        assert_eq!(state.native_stop_succeeded, Some(false));
        assert!(!state.progress.stopped);
        assert_eq!(state.progress.source.acknowledged_source_frames, 37);
        drop(state);
        stop_and_publish(false, &latest, Ok(()), || {
            panic!("unstarted client must not Stop")
        })
        .unwrap();
        assert_eq!(latest.lock().unwrap().native_stop_attempted, Some(false));
        assert_eq!(latest.lock().unwrap().native_stop_succeeded, None);
        assert!(!latest.lock().unwrap().progress.stopped);
        stop_and_publish(true, &latest, Ok(()), || Ok(())).unwrap();
        assert!(latest.lock().unwrap().progress.stopped);
        assert_eq!(latest.lock().unwrap().native_stop_succeeded, Some(true));
    }
    #[test]
    fn playback_submission_publication_is_silent_preroll_and_success_only() {
        let plan = super::super::source::SourceSchedule {
            schema_version: 1,
            schedule_id: "publication".into(),
            waveform_version: super::super::source::WAVEFORM_VERSION.into(),
            sample_rate: 48_000,
            channels: 2,
            marker_frames: 32,
            marker_amplitude: 0.1,
            total_source_frames: 64,
            starts: vec![0],
        }
        .prepare(1)
        .unwrap();
        let mut cursor = SourceCursor::new(Some(plan));
        let latest = Mutex::new(WorkerState::default());
        let mut samples = [0f32; 128];
        submit_and_publish(&mut cursor, &latest, |source| {
            source.submit(&mut samples, true, |pcm| {
                assert!(pcm.iter().all(|sample| *sample == 0.0));
                assert!(
                    latest.try_lock().is_ok(),
                    "publication lock held across native IO"
                );
                Ok(())
            })
        })
        .unwrap();
        let preroll = latest.lock().unwrap().progress.source.clone();
        assert_eq!(preroll.native_preroll_frames, 64);
        assert_eq!(preroll.acknowledged_source_frames, 0);
        assert!(preroll.origin_submission_frame.is_none());
        assert!(
            submit_and_publish(&mut cursor, &latest, |source| source.submit(
                &mut samples,
                false,
                |_| { anyhow::bail!("failed ReleaseBuffer") }
            ))
            .is_err()
        );
        assert_eq!(latest.lock().unwrap().progress.source, preroll);
        submit_and_publish(&mut cursor, &latest, |source| {
            source.submit(&mut samples, false, |_| Ok(()))
        })
        .unwrap();
        let submitted = latest.lock().unwrap().progress.source.clone();
        assert_eq!(submitted.origin_submission_frame, Some(64));
        assert_eq!(submitted.acknowledged_source_frames, 64);
        assert!(submitted.schedule_submitted_complete);
        submit_and_publish(&mut cursor, &latest, |source| {
            source.submit(&mut samples, false, |pcm| {
                assert!(pcm.iter().all(|sample| *sample == 0.0));
                Ok(())
            })
        })
        .unwrap();
        let tail = latest.lock().unwrap();
        assert_eq!(tail.progress.source.acknowledged_source_frames, 64);
        assert_eq!(tail.progress.source.native_submitted_frames, 192);
    }
    fn fake_session(worker: impl FnOnce() -> Result<()> + Send + 'static) -> PlaybackSession {
        PlaybackSession {
            stop: Arc::new(AtomicBool::new(false)),
            activate: Arc::new(AtomicBool::new(false)),
            worker: Some(thread::spawn(worker)),
            latest: Arc::new(Mutex::new(WorkerState::default())),
            provenance: SourceProvenance::new(None),
            activation_attempted: false,
            unexpected_exit: AtomicBool::new(false),
            info: PlaybackInfo {
                stable_endpoint_id: "test".into(),
                mmdevice_id: "test".into(),
                observed_name: "test".into(),
                adapter_instance: "test".into(),
                native_buffer_frames: 480,
                packet_format: "Float32".into(),
            },
        }
    }
    #[test]
    fn playback_activation_is_single_use_and_stop_before_start_is_rejected() {
        let (initial_tx, rx) = mpsc::channel();
        let mut session = fake_session(move || {
            rx.recv().unwrap();
            Ok(())
        });
        let tx = initial_tx;
        session.activate().unwrap();
        assert!(session.activate().is_err());
        session.request_stop();
        tx.send(()).unwrap();
        session.finish().unwrap();
        let mut stopped = fake_session(|| Ok(()));
        stopped.request_stop();
        assert!(stopped.activate().is_err());
    }
    #[test]
    fn playback_contention_is_unavailable_and_error_or_panic_keeps_partial_snapshot() {
        for panic in [false, true] {
            let (initial_tx, rx) = mpsc::channel();
            let mut session = fake_session(move || {
                let _ = rx.recv();
                if panic {
                    panic!("injected source panic");
                }
                anyhow::bail!("injected failed ReleaseBuffer")
            });
            let tx = initial_tx;
            {
                let mut state = session.latest.lock().unwrap();
                state.progress.started = true;
                state.progress.source.acknowledged_source_frames = 37;
                state.progress.source.origin_submission_frame = Some(480);
                assert!(session.check().unwrap().is_none());
            }
            session.request_stop();
            tx.send(()).unwrap();
            let outcome = session.finish().unwrap();
            assert_eq!(outcome.source_progress.acknowledged_source_frames, 37);
            assert!(outcome.started && outcome.failure.is_some());
            assert!(session.finish().is_err());
        }
    }
    #[test]
    fn playback_startup_abort_requests_peer_stop_before_first_join_and_retains_partial() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_events = events.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                thread::yield_now();
            }
            worker_events.lock().unwrap().push("source_join");
            anyhow::bail!("startup error")
        });
        let peer_events = events.clone();
        let mut partial = None;
        {
            let _guard = StartupGuard {
                worker: Some(worker),
                stop,
                latest: Arc::new(Mutex::new(WorkerState::default())),
                provenance: SourceProvenance::new(None),
                before_join: move || {
                    peer_events.lock().unwrap().push("recorder_stop");
                },
                partial: &mut partial,
            };
        }
        assert_eq!(*events.lock().unwrap(), ["recorder_stop", "source_join"]);
        assert!(partial.unwrap().failure.unwrap().contains("startup error"));
    }

    #[test]
    fn playback_unrequested_clean_exit_is_failure_even_after_submission_complete() {
        let mut session = fake_session(|| Ok(()));
        while !session.worker.as_ref().unwrap().is_finished() {
            thread::yield_now();
        }
        session
            .latest
            .lock()
            .unwrap()
            .progress
            .source
            .schedule_submitted_complete = true;
        assert!(session.check().is_err());
        let outcome = session.finish().unwrap();
        assert!(
            outcome
                .failure
                .as_deref()
                .is_some_and(|failure| failure.contains("before requested stop"))
        );
    }

    #[test]
    fn playback_startup_unwind_and_abort_panic_still_join_and_retain_snapshot() {
        let mut partial = None;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let latest = Arc::new(Mutex::new(WorkerState::default()));
        latest.lock().unwrap().progress.source.native_preroll_frames = 480;
        let joined = Arc::new(AtomicBool::new(false));
        let worker_joined = joined.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                thread::yield_now();
            }
            worker_joined.store(true, Ordering::Release);
            Ok(())
        });
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = StartupGuard {
                worker: Some(worker),
                stop,
                latest,
                provenance: SourceProvenance::new(None),
                before_join: || panic!("injected stop callback panic"),
                partial: &mut partial,
            };
            panic!("injected startup unwind");
        }));
        assert!(unwind.is_err() && joined.load(Ordering::Acquire));
        let outcome = partial.unwrap();
        assert_eq!(outcome.source_progress.native_preroll_frames, 480);
        assert!(outcome.failure.unwrap().contains("abort callback panicked"));
    }
}
