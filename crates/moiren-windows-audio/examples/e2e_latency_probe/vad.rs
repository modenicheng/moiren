//! Native playback client and endpoint identities for the live VAD probe.
//!
//! This is deliberately example-local. The production VAD provider owns the
//! driver PCM worker; this module owns only the external WASAPI client that
//! writes into the VAD playback MMDevice.

use super::source::{PreparedSource, SourceCursor, SourceProgress, SourceProvenance};
use super::source_controls::{self, SourceControls, native::NativeControls};
use super::source_write_witness::{self, SubmitError, WriteWitness};
use super::vad_collection::PlaybackProgress;
use super::vad_period::{self, PeriodEvidence};
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
            AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE_ACTIVE, IAudioClient,
            IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
            WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0, eCapture, eRender,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize, STGM_READ,
            },
            Threading::{
                AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
                WaitForSingleObject,
            },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_request: Option<PeriodEvidence>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_witness: Option<WriteWitness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_controls: Option<SourceControls>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_mode: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stable_endpoint_id: String,
    pub mmdevice_id: String,
    pub observed_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub adapter_instance: String,
    pub native_buffer_frames: u32,
    pub packet_format: String,
    pub mmcss_registered: bool,
    pub mmcss_hresult: Option<i32>,
}

struct AudioRegistration(HANDLE);

impl Drop for AudioRegistration {
    fn drop(&mut self) {
        // Registration and retirement both run on this source owner thread.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}

fn record_audio_registration<T>(
    result: std::result::Result<T, i32>,
    info: &mut PlaybackInfo,
) -> Option<T> {
    info.mmcss_registered = result.is_ok();
    info.mmcss_hresult = result.as_ref().err().copied();
    result.ok()
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct MicrophoneIdentity {
    pub stable_endpoint_id: String,
    pub mmdevice_id: String,
    pub observed_name: String,
    pub adapter_instance: String,
    pub root_instance: String,
}

/// Resolve the exact owned capture endpoint without retaining COM or control IO.
/// The measurement owner must separately initialize and check the pinned ID.
pub(super) fn resolve_microphone_identity(
    stable_id: StableEndpointId,
    root: &str,
) -> Result<MicrophoneIdentity> {
    let _apartment = Apartment::new()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let (device, selected) = resolve_endpoint(&enumerator, stable_id, root, Intent::Microphone)?;
    // Re-read the selected object before returning only owned identity strings.
    // This is not a lease: the later measurement open still checks its identity.
    ensure!(
        unsafe { device.GetState()? } == DEVICE_STATE_ACTIVE,
        "selected VAD microphone is no longer active"
    );
    let current = candidate(&enumerator, &device, Intent::Microphone)?;
    ensure!(
        current == selected && endpoint_matches(&current, stable_id, root, Intent::Microphone),
        "selected VAD microphone identity changed during resolution"
    );
    Ok(MicrophoneIdentity {
        stable_endpoint_id: stable_id.to_string(),
        mmdevice_id: current.mmdevice_id,
        observed_name: current.observed_name,
        adapter_instance: current.adapter_instance,
        root_instance: current.root_instance,
    })
}

/// Wait only for initial MMDevice publication within the caller's setup budget.
/// Later identity rechecks must use `resolve_microphone_identity` directly.
pub(super) fn wait_microphone_identity(
    stable_id: StableEndpointId,
    root: &str,
    deadline: Instant,
) -> Result<MicrophoneIdentity> {
    wait_microphone_publication(
        deadline,
        || resolve_microphone_identity(stable_id, root),
        Instant::now,
        thread::sleep,
    )
}

fn wait_microphone_publication<T>(
    deadline: Instant,
    mut resolve: impl FnMut() -> Result<T>,
    mut now: impl FnMut() -> Instant,
    mut pause: impl FnMut(Duration),
) -> Result<T> {
    let timeout = |last_absence: Option<anyhow::Error>| {
        last_absence
            .unwrap_or_else(|| anyhow::anyhow!("VAD microphone publication not observed"))
            .context("VAD microphone MMDevice publication deadline exceeded")
    };
    let mut last_absence = None;
    loop {
        if now() >= deadline {
            return Err(timeout(last_absence));
        }
        let result = resolve();
        let completed = now();
        match result {
            Ok(identity) => {
                // A synchronous native query cannot be interrupted. Do not
                // accept its result if it returned outside the setup budget.
                return if completed < deadline {
                    Ok(identity)
                } else {
                    Err(timeout(last_absence))
                };
            }
            Err(error) => {
                if error.downcast_ref::<EndpointNotPublished>().is_none() {
                    return Err(error);
                }
                last_absence = Some(error);
            }
        }
        if completed >= deadline {
            return Err(timeout(last_absence));
        }
        let before_pause = now();
        if before_pause >= deadline {
            return Err(timeout(last_absence));
        }
        pause((deadline - before_pause).min(Duration::from_millis(100)));
        if now() >= deadline {
            return Err(timeout(last_absence));
        }
    }
}

enum PlaybackTarget {
    Vad {
        stable_id: StableEndpointId,
        root: String,
        minimum_period: bool,
    },
    Shared {
        endpoint: String,
        witness: bool,
    },
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
    period_request: Option<PeriodEvidence>,
    provenance: Option<SourceProvenance>,
    progress: PlaybackProgress,
    elapsed_seconds: f64,
    native_stop_attempted: Option<bool>,
    native_stop_succeeded: Option<bool>,
    write_witness: Option<WriteWitness>,
    source_controls: Option<SourceControls>,
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
        minimum_period: bool,
        partial: &mut Option<PlaybackOutcome>,
    ) -> Result<Self> {
        Self::prepare_with_abort(
            stable_id,
            root_instance,
            source_plan,
            minimum_period,
            || {},
            partial,
        )
    }

    pub(super) fn prepare_with_abort(
        stable_id: StableEndpointId,
        root_instance: String,
        source_plan: Option<PreparedSource>,
        minimum_period: bool,
        before_join: impl FnMut(),
        partial: &mut Option<PlaybackOutcome>,
    ) -> Result<Self> {
        Self::prepare_target(
            PlaybackTarget::Vad {
                stable_id,
                root: root_instance,
                minimum_period,
            },
            source_plan,
            before_join,
            partial,
        )
    }

    pub(super) fn prepare_shared(
        endpoint: String,
        plan: PreparedSource,
        witness: bool,
        partial: &mut Option<PlaybackOutcome>,
    ) -> Result<Self> {
        ensure!(
            !endpoint.trim().is_empty()
                && !endpoint.contains('\0')
                && endpoint.encode_utf16().count() <= 4096,
            "explicit shared endpoint required"
        );
        Self::prepare_target(
            PlaybackTarget::Shared { endpoint, witness },
            Some(plan),
            || {},
            partial,
        )
    }

    fn prepare_target(
        target: PlaybackTarget,
        source_plan: Option<PreparedSource>,
        before_join: impl FnMut(),
        partial: &mut Option<PlaybackOutcome>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let activate = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(WorkerState {
            period_request: matches!(
                target,
                PlaybackTarget::Vad {
                    minimum_period: true,
                    ..
                }
            )
            .then(PeriodEvidence::new),
            ..WorkerState::default()
        }));
        let provenance = SourceProvenance::new(source_plan.as_ref());
        let worker_stop = stop.clone();
        let worker_activate = activate.clone();
        let worker_latest = latest.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("moiren-vad-native-playback".into())
            .spawn(move || {
                let result = run_worker(
                    target,
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
        period_request: state.period_request,
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
        write_witness: state.write_witness.clone(),
        source_controls: state.source_controls.clone(),
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
) -> Result<EndpointCandidate> {
    // SAFETY: all COM objects remain in this worker apartment and topology is
    // used only to obtain the opaque adapter identity.
    unsafe {
        let endpoint: windows::Win32::Media::Audio::IMMEndpoint =
            windows::core::Interface::cast(device)?;
        let expected_flow = match intent {
            Intent::Playback => eRender,
            Intent::Microphone => eCapture,
        };
        ensure!(
            endpoint.GetDataFlow()? == expected_flow,
            "VAD endpoint direction does not match {intent:?}"
        );
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
        Ok(EndpointCandidate {
            mmdevice_id: id,
            observed_name,
            adapter_instance: raw_instance,
            root_instance: instance(parent)?,
            intent,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EndpointCandidate {
    mmdevice_id: String,
    observed_name: String,
    adapter_instance: String,
    root_instance: String,
    intent: Intent,
}

#[derive(Debug, thiserror::Error)]
#[error("expected one VAD {intent:?} MMDevice for {stable_id}, found 0")]
struct EndpointNotPublished {
    stable_id: StableEndpointId,
    intent: Intent,
}

fn endpoint_matches(
    candidate: &EndpointCandidate,
    stable_id: StableEndpointId,
    root: &str,
    intent: Intent,
) -> bool {
    let wanted = stable_id.to_string();
    candidate
        .adapter_instance
        .rsplit_once('\\')
        .is_some_and(|(hardware, suffix)| {
            hardware.eq_ignore_ascii_case(r"MOIREN\VAD_RAW")
                && suffix
                    .rsplit('&')
                    .next()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&wanted))
        })
        && candidate.intent == intent
        && candidate.root_instance.eq_ignore_ascii_case(root)
}

fn select_endpoint<T>(
    candidates: impl IntoIterator<Item = Result<(T, EndpointCandidate)>>,
    stable_id: StableEndpointId,
    root: &str,
    intent: Intent,
) -> Result<(T, EndpointCandidate)> {
    let mut selected = None;
    let mut match_count = 0;
    for candidate in candidates {
        let (device, candidate) = candidate?;
        if endpoint_matches(&candidate, stable_id, root, intent) {
            match_count += 1;
            selected = Some((device, candidate));
        }
    }
    if match_count == 0 {
        return Err(EndpointNotPublished { stable_id, intent }.into());
    }
    ensure!(
        match_count == 1,
        "expected one VAD {intent:?} MMDevice for {stable_id}, found {match_count}"
    );
    Ok(selected.expect("one owned VAD endpoint"))
}

fn resolve_playback(
    enumerator: &IMMDeviceEnumerator,
    stable_id: StableEndpointId,
    root: &str,
) -> Result<(IMMDevice, String, String, String)> {
    let (device, selected) = resolve_endpoint(enumerator, stable_id, root, Intent::Playback)?;
    Ok((
        device,
        selected.mmdevice_id,
        selected.observed_name,
        selected.adapter_instance,
    ))
}

fn resolve_endpoint(
    enumerator: &IMMDeviceEnumerator,
    stable_id: StableEndpointId,
    root: &str,
    intent: Intent,
) -> Result<(IMMDevice, EndpointCandidate)> {
    let flow = match intent {
        Intent::Playback => eRender,
        Intent::Microphone => eCapture,
    };
    let devices = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
    let count = unsafe { devices.GetCount()? };
    select_endpoint(
        (0..count).map(|index| {
            let device = unsafe { devices.Item(index)? };
            let candidate = candidate(enumerator, &device, intent)?;
            Ok((device, candidate))
        }),
        stable_id,
        root,
        intent,
    )
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

fn available_frames(capacity: u32, padding: u32) -> Result<u32> {
    ensure!(
        (1..=MAX_PACKET_FRAMES * 2).contains(&capacity),
        "shared native buffer out of bounds"
    );
    capacity
        .checked_sub(padding)
        .context("shared padding exceeds native buffer")
}

fn submit_packet(
    source: &mut SourceCursor,
    samples: &mut [f32],
    frames: u32,
    preroll: bool,
    release: impl FnOnce(&[f32]) -> Result<()>,
) -> Result<()> {
    if frames == 0 {
        return Ok(());
    }
    let packet = samples
        .get_mut(..frames as usize * 2)
        .context("source packet exceeds preallocation")?;
    source.submit(packet, preroll, release)
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
    shared: bool,
    witness: Option<WriteWitness>,
    witness_clock: Option<moiren_windows_audio::latency::QpcClock>,
}

fn prepare_alignment_retry(
    latest: &Mutex<WorkerState>,
    buffer_size: impl FnOnce() -> Result<u32>,
) -> Result<i64> {
    if let Some(evidence) = latest
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .period_request
        .as_mut()
    {
        // The alignment error already requires recovery. If its size query
        // fails, retain that intent and the last known request unchanged.
        evidence.alignment_retry = true;
    }
    let frames = buffer_size()?;
    ensure!(
        (1..=MAX_PACKET_FRAMES * 2).contains(&frames),
        "aligned buffer out of bounds"
    );
    let period = (u64::from(frames) * 10_000_000).div_ceil(48_000) as i64;
    if let Some(evidence) = latest
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .period_request
        .as_mut()
    {
        // Publish before fresh activation/initialization can fail.
        evidence.record_alignment_retry(period);
    }
    Ok(period)
}

impl Stream {
    fn prepare(device: &IMMDevice, latest: &Mutex<WorkerState>) -> Result<Self> {
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
        let mut evidence = latest
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .period_request;
        let (mut default_hns, mut minimum_hns) = (0, 0);
        // Keep the legacy query shape unless explicitly requesting a minimum.
        // This query is evidence of a request, not the actual packet cadence.
        unsafe {
            client.GetDevicePeriod(
                Some(&mut default_hns),
                evidence.as_ref().map(|_| &mut minimum_hns as *mut i64),
            )?
        };
        let selection = vad_period::select_period(default_hns, minimum_hns, evidence.as_mut());
        latest
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .period_request = evidence;
        let mut period = selection?;
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
            period = prepare_alignment_retry(latest, || {
                // SAFETY: apartment-local client after the alignment error.
                Ok(unsafe { client.GetBufferSize()? })
            })?;
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
            shared: false,
            witness: None,
            witness_clock: None,
        })
    }

    fn prepare_shared(device: &IMMDevice, witness: bool) -> Result<Self> {
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let pointer = unsafe { client.GetMixFormat()? };
        struct Mix(*mut WAVEFORMATEX);
        impl Drop for Mix {
            fn drop(&mut self) {
                unsafe { CoTaskMemFree(Some(self.0.cast())) };
            }
        }
        let owned = Mix(pointer);
        ensure!(!owned.0.is_null(), "null shared mix format");
        let wave = unsafe { &*owned.0 };
        ensure!(
            wave.nSamplesPerSec == 48_000 && wave.nChannels == 2,
            "shared source requires exact 48 kHz stereo mix format"
        );
        let format = match (wave.wFormatTag, wave.wBitsPerSample) {
            (1, 16) => SampleFormat::Pcm16,
            (3, 32) => SampleFormat::Float32,
            (0xfffe, bits) if wave.cbSize == 22 => {
                let ext = unsafe { &*owned.0.cast::<WAVEFORMATEXTENSIBLE>() };
                ensure!(
                    ext.dwChannelMask == 3 && unsafe { ext.Samples.wValidBitsPerSample } == bits,
                    "unsupported shared channel mask or valid bits"
                );
                let pcm = descriptor(SampleFormat::Pcm16).SubFormat;
                let float = descriptor(SampleFormat::Float32).SubFormat;
                match (ext.SubFormat, bits) {
                    (id, 16) if id == pcm => SampleFormat::Pcm16,
                    (id, 32) if id == float => SampleFormat::Float32,
                    _ => anyhow::bail!("unsupported shared mix subtype"),
                }
            }
            _ => anyhow::bail!("unsupported shared source mix format"),
        };
        ensure!(
            wave.nBlockAlign as usize == 2 * format.bytes_per_sample()
                && wave.nAvgBytesPerSec == 48_000 * wave.nBlockAlign as u32,
            "invalid shared mix alignment"
        );
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                0,
                0,
                owned.0,
                None,
            )?
        };
        let frames = unsafe { client.GetBufferSize()? };
        available_frames(frames, 0)?;
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
            shared: true,
            witness: witness.then(WriteWitness::default),
            witness_clock: witness
                .then(moiren_windows_audio::latency::QpcClock::new)
                .flatten(),
        })
    }

    fn start(&mut self) -> Result<()> {
        // SAFETY: Initialize and event handle setup completed successfully.
        unsafe { self.client.Start()? };
        self.active = true;
        Ok(())
    }

    fn fill_and_submit(&mut self, source: &mut SourceCursor, preroll: bool) -> Result<()> {
        let frames = if self.shared {
            available_frames(self.frames, unsafe { self.client.GetCurrentPadding()? })?
        } else {
            self.frames
        };
        if frames == 0 {
            return Ok(());
        }
        let bytes = frames as usize * 2 * self.format.bytes_per_sample();
        let silent_preroll = preroll && source.has_schedule();
        let cursor_before = self
            .witness
            .as_ref()
            .filter(|w| !w.attempted)
            .map(|_| source.progress.clone());
        submit_packet(source, &mut self.samples, frames, preroll, |samples| {
            encode_interleaved(
                samples,
                &mut self.encoded[..bytes],
                self.format,
                Channels::Stereo,
            )
            .map_err(|error| anyhow::anyhow!("encode native VAD PCM: {error:?}"))?;
            let flags = if silent_preroll {
                AUDCLNT_BUFFERFLAGS_SILENT.0 as u32
            } else {
                0
            };
            let mut packet =
                self.witness
                    .as_ref()
                    .zip(cursor_before.as_ref())
                    .and_then(|(witness, cursor)| {
                        witness.prepare(self.format, samples, &self.encoded[..bytes], flags, cursor)
                    });
            if let Some(packet) = packet.as_mut() {
                packet.qpc_frequency_hz = self
                    .witness_clock
                    .as_ref()
                    .and_then(|clock| clock.frequency_hz());
            }
            let pointer = unsafe { self.render.GetBuffer(frames)? };
            // SAFETY: WASAPI granted exactly frames of the checked format.
            // Only successful ReleaseBuffer commits the portable source cursor.
            let result = unsafe {
                source_write_witness::copy_release(
                    &self.encoded[..bytes],
                    pointer,
                    self.witness.as_mut().zip(packet),
                    || {
                        self.witness_clock
                            .as_ref()
                            .and_then(|clock| clock.now_ticks())
                    },
                    || self.render.ReleaseBuffer(frames, flags),
                    |error| error.code().0,
                )
            };
            match result {
                Ok(()) => Ok(()),
                Err(SubmitError::Release(error)) => Err(error.into()),
                Err(SubmitError::NullBuffer) => anyhow::bail!("VAD render buffer is null"),
            }
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
    target: PlaybackTarget,
    source_plan: Option<PreparedSource>,
    stop: Arc<AtomicBool>,
    activate: Arc<AtomicBool>,
    ready_tx: &mpsc::SyncSender<std::result::Result<PlaybackInfo, String>>,
    latest: Arc<Mutex<WorkerState>>,
) -> Result<()> {
    let _apartment = Apartment::new()?;
    let witness = matches!(&target, PlaybackTarget::Shared { witness: true, .. });
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let (device, mmdevice_id, observed_name, adapter_instance, stable_endpoint_id, shared) =
        match target {
            PlaybackTarget::Vad {
                stable_id, root, ..
            } => {
                let (device, id, name, adapter) = wait_playback(&enumerator, stable_id, &root)?;
                (device, id, name, adapter, stable_id.to_string(), false)
            }
            PlaybackTarget::Shared { endpoint, .. } => {
                let device = unsafe { enumerator.GetDevice(&HSTRING::from(&endpoint))? };
                let id = take_string(unsafe { device.GetId()? })?;
                ensure!(id == endpoint, "shared MMDevice identity mismatch");
                let flow: windows::Win32::Media::Audio::IMMEndpoint =
                    windows::core::Interface::cast(&device)?;
                ensure!(
                    unsafe { flow.GetDataFlow()? } == eRender,
                    "shared source endpoint is not render"
                );
                let properties = unsafe { device.OpenPropertyStore(STGM_READ)? };
                let property = unsafe { properties.GetValue(&PKEY_Device_FriendlyName)? };
                let name = if property.vt() == VT_LPWSTR {
                    unsafe { property.Anonymous.Anonymous.Anonymous.pwszVal.to_string()? }
                } else {
                    anyhow::bail!("shared source name is not a string")
                };
                ensure!(
                    name.encode_utf16().count() <= 4096,
                    "shared source name exceeds receipt budget"
                );
                (device, id, name, String::new(), String::new(), true)
            }
        };
    let mut stream = if shared {
        Stream::prepare_shared(&device, witness)?
    } else {
        Stream::prepare(&device, &latest)?
    };
    let mut source = SourceCursor::new(source_plan);
    let mut controls = source_controls::prepare_if(witness, || {
        NativeControls::prepare(
            &stream.client,
            &device,
            &mmdevice_id,
            stream.witness_clock.as_ref(),
            || stop.load(Ordering::Acquire),
        )
    });
    if let Some(controls) = controls.as_ref() {
        latest
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .source_controls = Some(controls.evidence.clone());
    }
    latest.lock().unwrap_or_else(|p| p.into_inner()).provenance = Some(source.provenance());
    let mut info = PlaybackInfo {
        source_mode: shared.then(|| "shared_explicit_mmdevice".into()),
        stable_endpoint_id,
        mmdevice_id,
        observed_name,
        adapter_instance,
        native_buffer_frames: stream.frames,
        packet_format: format!("{:?}", stream.format),
        mmcss_registered: false,
        mmcss_hresult: None,
    };
    // A normal-priority test source can starve before the graph while capture
    // and render owners keep serving. Match their Audio task and retain the
    // actual result; successful ReleaseBuffer alone cannot prove supply timing.
    let mut task_index = 0;
    let registration =
        unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut task_index) }
            .map(AudioRegistration)
            .map_err(|error| error.code().0);
    let _audio_registration = record_audio_registration(registration, &mut info);
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
                    if let Some(controls) = controls.as_mut().filter(|controls| {
                        controls.evidence.snapshot.is_none()
                            && stream
                                .witness
                                .as_ref()
                                .is_some_and(|w| w.confirmed.is_some())
                    }) {
                        // Successful nonzero Release has committed source progress.
                        // No lease/mutex is held during getters; they may perturb
                        // servicing and cannot be assigned a hard COM timeout.
                        controls.evidence.snapshot = Some(source_controls::observe(
                            controls,
                            source.progress.clone(),
                            || {
                                stream
                                    .witness_clock
                                    .as_ref()
                                    .and_then(|clock| clock.now_ticks())
                            },
                            || stop.load(Ordering::Acquire),
                        ));
                        latest
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .source_controls = Some(controls.evidence.clone());
                    }
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
        state.write_witness = stream.witness.clone();
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
    #[test]
    fn source_controls_join_keeps_observation_and_original_audio_error() {
        use super::super::source_controls::{
            ControlSnapshot, Preparation, Reading, SourceControls,
        };
        let latest = Mutex::new(WorkerState {
            source_controls: Some(SourceControls {
                qualified: false,
                mmdevice_id: "exact owned playback".into(),
                process_id: 42,
                process_creation_100ns: Reading::observed(0, 123),
                qpc_frequency_hz: Some(10_000_000),
                preparation: Preparation {
                    session_control_hresult: Some(0),
                    session_volume_hresult: Some(-123),
                    endpoint_volume_hresult: Some(0),
                },
                snapshot: Some(ControlSnapshot {
                    phase: "after_first_nonzero_release",
                    client_start_succeeded: true,
                    source_progress: SourceProgress {
                        submitted_nonzero_frames: 480,
                        ..Default::default()
                    },
                    before_qpc: Some(100),
                    after_qpc: Some(101),
                    stop_requested: false,
                    session_state: Reading::observed(0, 1),
                    session_volume: Reading::default(),
                    session_mute: Reading::default(),
                    endpoint_volume: Reading::observed(0, 0.5),
                    endpoint_mute: Reading::observed(0, false),
                }),
            }),
            ..WorkerState::default()
        });
        let worker = thread::spawn(|| anyhow::bail!("original later render failure"));
        let outcome = join_outcome(worker, &latest, SourceProvenance::new(None));
        assert!(
            outcome
                .failure
                .as_deref()
                .unwrap()
                .contains("original later render failure")
        );
        let json = serde_json::to_value(outcome).unwrap();
        assert_eq!(
            json["source_controls"]["preparation"]["session_volume_hresult"],
            -123
        );
        assert_eq!(json["source_controls"]["snapshot"]["before_qpc"], 100);
        assert!(json["source_controls"]["snapshot"]["session_volume"]["value"].is_null());
        assert!(json["source_controls"]["snapshot"]["session_volume"]["hresult"].is_null());
        assert_eq!(
            json["source_controls"]["snapshot"]["endpoint_mute"]["value"],
            false
        );
        assert_eq!(json["source_controls"]["qualified"], false);
    }

    #[test]
    fn shared_source_padding_bounds_the_packet_and_rejects_invalid_padding() {
        assert_eq!(super::available_frames(480, 352).unwrap(), 128);
        assert_eq!(super::available_frames(480, 480).unwrap(), 0);
        assert!(super::available_frames(480, 481).is_err());
        assert!(super::available_frames(0, 0).is_err());
    }

    #[test]
    fn shared_source_packet_slices_to_padding_and_failed_release_does_not_advance() {
        let plan = super::super::source::SourceSchedule {
            schema_version: 1,
            schedule_id: "padding".into(),
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
        let mut source = SourceCursor::new(Some(plan));
        let mut samples = [9.0; 960];
        let frames = available_frames(480, 448).unwrap();
        submit_packet(&mut source, &mut samples, frames, true, |pcm| {
            assert_eq!(
                pcm.len(),
                64,
                "native ReleaseBuffer must use available frames only"
            );
            assert!(pcm.iter().all(|sample| *sample == 0.0));
            Ok(())
        })
        .unwrap();
        let before = source.progress.clone();
        assert_eq!(before.native_preroll_frames, 32);
        assert_eq!(samples[64], 9.0, "unused capacity must not be generated");
        assert!(
            submit_packet(&mut source, &mut samples, frames, false, |_| {
                anyhow::bail!("ReleaseBuffer failed")
            })
            .is_err()
        );
        assert_eq!(source.progress, before);
        submit_packet(&mut source, &mut samples, frames, false, |_| Ok(())).unwrap();
        assert_eq!(source.progress.acknowledged_source_frames, 32);
        assert_eq!(source.progress.origin_submission_frame, Some(32));
        let before = source.progress.clone();
        submit_packet(&mut source, &mut samples, 0, false, |_| {
            panic!("full buffer must skip release")
        })
        .unwrap();
        assert_eq!(source.progress, before);
    }
    use super::*;

    fn identity_candidate(stable_id: StableEndpointId, intent: Intent) -> EndpointCandidate {
        EndpointCandidate {
            mmdevice_id: "opaque-mmdevice-id".into(),
            observed_name: "same friendly name for all siblings".into(),
            adapter_instance: format!(r"MOIREN\VAD_RAW\6&opaque&{stable_id}"),
            root_instance: r"ROOT\MOIRENVAD\0001".into(),
            intent,
        }
    }

    #[test]
    fn endpoint_identity_selects_exact_microphone_and_preserves_playback() {
        let stable_id = StableEndpointId::new(0xabcd, 0xef01);
        for intent in [Intent::Playback, Intent::Microphone] {
            let mut candidate = identity_candidate(stable_id, intent);
            candidate.adapter_instance = candidate.adapter_instance.to_ascii_uppercase();
            let root = candidate.root_instance.to_ascii_lowercase();
            let (selected, identity) =
                select_endpoint([Ok((7, candidate))], stable_id, &root, intent).unwrap();
            assert_eq!(selected, 7);
            assert_eq!(identity.mmdevice_id, "opaque-mmdevice-id");
            assert_eq!(
                identity.observed_name,
                "same friendly name for all siblings"
            );
            assert_eq!(identity.intent, intent);
        }
    }

    #[test]
    fn endpoint_identity_rejects_wrong_root_id_intent_hardware_and_sibling_suffix() {
        let stable_id = StableEndpointId::new(0xabcd, 0xef01);
        for intent in [Intent::Playback, Intent::Microphone] {
            let wanted = identity_candidate(stable_id, intent);
            let mut wrong_root = wanted.clone();
            wrong_root.root_instance.push('2');
            let wrong_id = identity_candidate(StableEndpointId::new(0xabce, 0xef01), intent);
            let mut wrong_intent = wanted.clone();
            wrong_intent.intent = if intent == Intent::Playback {
                Intent::Microphone
            } else {
                Intent::Playback
            };
            let mut wrong_hardware = wanted.clone();
            wrong_hardware.adapter_instance = format!(r"OTHER\VAD_RAW\6&opaque&{stable_id}");
            let mut suffix_sibling = wanted.clone();
            suffix_sibling.adapter_instance.push('0');
            let mut prefix_sibling = wanted.clone();
            prefix_sibling.adapter_instance = format!(r"MOIREN\VAD_RAW\6&opaque&0{stable_id}");
            for candidate in [
                wrong_root,
                wrong_id,
                wrong_intent,
                wrong_hardware,
                suffix_sibling,
                prefix_sibling,
            ] {
                assert!(
                    select_endpoint(
                        [Ok((1, candidate))],
                        stable_id,
                        &wanted.root_instance,
                        intent
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn endpoint_identity_does_not_select_same_name_sibling() {
        let stable_id = StableEndpointId::new(0xabcd, 0xef01);
        let wanted = identity_candidate(stable_id, Intent::Microphone);
        let mut sibling = wanted.clone();
        sibling.intent = Intent::Playback;
        let mut other_root = wanted.clone();
        other_root.root_instance = r"ROOT\MOIRENVAD\0002".into();
        let (selected, identity) = select_endpoint(
            [Ok((1, sibling)), Ok((2, wanted)), Ok((3, other_root))],
            stable_id,
            r"ROOT\MOIRENVAD\0001",
            Intent::Microphone,
        )
        .unwrap();
        assert_eq!(selected, 2);
        assert_eq!(identity.root_instance, r"ROOT\MOIRENVAD\0001");
    }

    #[test]
    fn endpoint_identity_rejects_duplicate_exact_matches() {
        let stable_id = StableEndpointId::new(0xabcd, 0xef01);
        for intent in [Intent::Playback, Intent::Microphone] {
            let wanted = identity_candidate(stable_id, intent);
            let error = select_endpoint(
                [Ok((1, wanted.clone())), Ok((2, wanted))],
                stable_id,
                r"ROOT\MOIRENVAD\0001",
                intent,
            )
            .unwrap_err();
            assert!(error.to_string().contains("found 2"));
        }
    }

    #[test]
    fn endpoint_identity_does_not_hide_enumeration_failure_after_a_match() {
        let stable_id = StableEndpointId::new(0xabcd, 0xef01);
        let error = select_endpoint(
            [
                Ok((1, identity_candidate(stable_id, Intent::Playback))),
                Err(anyhow::anyhow!("candidate metadata unavailable")),
            ],
            stable_id,
            r"ROOT\MOIRENVAD\0001",
            Intent::Playback,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "candidate metadata unavailable");
    }

    fn unpublished_microphone() -> anyhow::Error {
        select_endpoint::<()>(
            std::iter::empty(),
            StableEndpointId::new(0xabcd, 0xef01),
            r"ROOT\MOIRENVAD\0001",
            Intent::Microphone,
        )
        .unwrap_err()
    }

    #[test]
    fn microphone_publication_wait_retries_typed_absence_then_returns_exact_identity() {
        let start = Instant::now();
        let clock = std::cell::Cell::new(start);
        let mut calls = 0;
        let mut delays = Vec::new();
        let selected = wait_microphone_publication(
            start + Duration::from_secs(1),
            || {
                calls += 1;
                if calls < 3 {
                    return Err(unpublished_microphone().context("enumerate capture endpoints"));
                }
                Ok(identity_candidate(
                    StableEndpointId::new(0xabcd, 0xef01),
                    Intent::Microphone,
                ))
            },
            || clock.get(),
            |delay| {
                delays.push(delay);
                clock.set(clock.get() + delay);
            },
        )
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(delays, [Duration::from_millis(100); 2]);
        assert_eq!(selected.mmdevice_id, "opaque-mmdevice-id");
        assert_eq!(selected.intent, Intent::Microphone);
    }

    #[test]
    fn microphone_publication_wait_does_not_query_an_expired_deadline() {
        let deadline = Instant::now();
        let mut calls = 0;
        let result = wait_microphone_publication(
            deadline,
            || {
                calls += 1;
                Ok(())
            },
            || deadline,
            |_| panic!("expired setup must not sleep"),
        );
        assert!(result.is_err());
        assert_eq!(calls, 0);
    }

    #[test]
    fn microphone_publication_wait_refuses_success_returned_at_or_after_deadline() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(100);
        for completion in [deadline, deadline + Duration::from_millis(1)] {
            let clock = std::cell::Cell::new(start);
            let result = wait_microphone_publication(
                deadline,
                || {
                    clock.set(completion);
                    Ok(())
                },
                || clock.get(),
                |_| panic!("late success must not sleep"),
            );
            assert!(result.unwrap_err().to_string().contains("deadline"));
        }
    }

    #[test]
    fn microphone_publication_wait_caps_delays_and_never_restarts_setup_budget() {
        let start = Instant::now();
        let clock = std::cell::Cell::new(start);
        let mut calls = 0;
        let mut delays = Vec::new();
        let result = wait_microphone_publication::<()>(
            start + Duration::from_millis(230),
            || {
                calls += 1;
                Err(unpublished_microphone())
            },
            || clock.get(),
            |delay| {
                delays.push(delay);
                clock.set(clock.get() + delay);
            },
        );
        let error = result.unwrap_err();
        assert!(error.to_string().contains("deadline"));
        assert!(error.downcast_ref::<EndpointNotPublished>().is_some());
        assert_eq!(calls, 3);
        assert_eq!(delays, [100, 100, 30].map(Duration::from_millis));
    }

    #[test]
    fn microphone_publication_wait_checks_deadline_after_slow_query_and_delay() {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(50);
        for slow_query in [false, true] {
            let clock = std::cell::Cell::new(start);
            let mut calls = 0;
            let mut delays = 0;
            let result = wait_microphone_publication::<()>(
                deadline,
                || {
                    calls += 1;
                    if slow_query {
                        clock.set(deadline + Duration::from_millis(1));
                    }
                    Err(unpublished_microphone())
                },
                || clock.get(),
                |delay| {
                    assert_eq!(delay, Duration::from_millis(50));
                    delays += 1;
                    clock.set(deadline + Duration::from_millis(1));
                },
            );
            assert!(result.unwrap_err().to_string().contains("deadline"));
            assert_eq!(calls, 1);
            assert_eq!(delays, usize::from(!slow_query));
        }
    }

    #[test]
    fn microphone_publication_wait_never_retries_ambiguous_identity_or_io_errors() {
        let stable_id = StableEndpointId::new(0xabcd, 0xef01);
        let wanted = identity_candidate(stable_id, Intent::Microphone);
        let ambiguous = select_endpoint(
            [Ok((1, wanted.clone())), Ok((2, wanted))],
            stable_id,
            r"ROOT\MOIRENVAD\0001",
            Intent::Microphone,
        )
        .unwrap_err();
        for error in [
            ambiguous,
            anyhow::anyhow!("selected VAD microphone identity changed during resolution"),
            anyhow::anyhow!("GetState failed: found 0 devices is not a typed absence"),
        ] {
            let message = error.to_string();
            let mut error = Some(error);
            let mut calls = 0;
            let start = Instant::now();
            let result = wait_microphone_publication::<()>(
                start + Duration::from_secs(1),
                || {
                    calls += 1;
                    Err(error.take().expect("hard failure must not be retried"))
                },
                || start,
                |_| panic!("hard failure must not sleep"),
            );
            assert_eq!(result.unwrap_err().to_string(), message);
            assert_eq!(calls, 1);
        }
    }

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
                source_mode: None,
                stable_endpoint_id: "test".into(),
                mmdevice_id: "test".into(),
                observed_name: "test".into(),
                adapter_instance: "test".into(),
                native_buffer_frames: 480,
                packet_format: "Float32".into(),
                mmcss_registered: false,
                mmcss_hresult: None,
            },
        }
    }
    #[test]
    fn playback_source_reports_audio_scheduler_registration_separately_from_started() {
        let session = fake_session(|| Ok(()));
        let info = serde_json::to_value(&session.info).unwrap();
        assert_eq!(info["mmcss_registered"], false);
        assert_eq!(info["mmcss_hresult"], serde_json::Value::Null);
    }
    #[test]
    fn playback_source_retains_scheduler_failure_and_retires_successful_registration() {
        use std::cell::Cell;
        struct Registration<'a>(&'a Cell<bool>);
        impl Drop for Registration<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let mut session = fake_session(|| Ok(()));
        let retired = Cell::new(false);
        let registration = record_audio_registration(Ok(Registration(&retired)), &mut session.info);
        assert!(session.info.mmcss_registered);
        assert!(session.info.mmcss_hresult.is_none());
        assert!(!retired.get());
        drop(registration);
        assert!(retired.get());
        let registration =
            record_audio_registration::<Registration<'_>>(Err(-123), &mut session.info);
        assert!(registration.is_none());
        assert!(!session.info.mmcss_registered);
        assert_eq!(session.info.mmcss_hresult, Some(-123));
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

    #[test]
    fn minimum_period_join_retains_request_evidence_after_preparation_failure() {
        for alignment_retry in [false, true] {
            let mut evidence = PeriodEvidence::new();
            if alignment_retry {
                vad_period::select_period(100_000, 10_000, Some(&mut evidence)).unwrap();
                evidence.record_alignment_retry(106_667);
            } else {
                assert!(vad_period::select_period(100_000, 0, Some(&mut evidence)).is_err());
            }
            let latest = Mutex::new(WorkerState {
                period_request: Some(evidence),
                ..WorkerState::default()
            });
            let worker = thread::spawn(|| anyhow::bail!("injected Initialize failure"));
            let outcome = join_outcome(worker, &latest, SourceProvenance::new(None));
            let json = serde_json::to_value(outcome).unwrap();
            assert_eq!(json["period_request"]["default_hns"], 100_000);
            assert_eq!(
                json["period_request"]["qualification"],
                "unqualified-request-only"
            );
            if alignment_retry {
                assert_eq!(json["period_request"]["initial_request_hns"], 10_000);
                assert_eq!(json["period_request"]["final_request_hns"], 106_667);
                assert_eq!(json["period_request"]["alignment_retry"], true);
            } else {
                assert_eq!(json["period_request"]["minimum_hns"], 0);
                assert!(json["period_request"]["initial_request_hns"].is_null());
                assert!(json["period_request"]["final_request_hns"].is_null());
            }
            assert!(json["failure"].as_str().unwrap().contains("Initialize"));
            assert_eq!(json["worker_joined"], true);
            assert_eq!(json["started"], false);
            assert!(json.get("native_buffer_frames").is_none());
            assert!(json.get("actual_period_hns").is_none());
        }
    }

    #[test]
    fn minimum_period_legacy_outcome_omits_request_evidence() {
        let outcome = join_outcome(
            thread::spawn(|| Ok(())),
            &Mutex::new(WorkerState::default()),
            SourceProvenance::new(None),
        );
        assert!(
            serde_json::to_value(outcome)
                .unwrap()
                .get("period_request")
                .is_none()
        );
    }

    #[test]
    fn minimum_period_failed_alignment_query_retains_recovery_intent() {
        for frames in [None, Some(0), Some(MAX_PACKET_FRAMES * 2 + 1)] {
            let mut evidence = PeriodEvidence::new();
            vad_period::select_period(100_000, 10_000, Some(&mut evidence)).unwrap();
            let latest = Mutex::new(WorkerState {
                period_request: Some(evidence),
                ..WorkerState::default()
            });
            let request = prepare_alignment_retry(&latest, || {
                let state = latest.try_lock().expect("native query runs without a lock");
                assert!(
                    state.period_request.unwrap().alignment_retry,
                    "retain alignment recovery intent before querying buffer size"
                );
                drop(state);
                frames.context("injected GetBufferSize failure")
            });
            assert!(request.is_err());
            let outcome = join_outcome(
                thread::spawn(move || request.map(|_| ())),
                &latest,
                SourceProvenance::new(None),
            );
            let json = serde_json::to_value(outcome).unwrap();
            assert_eq!(json["period_request"]["alignment_retry"], true);
            assert_eq!(json["period_request"]["initial_request_hns"], 10_000);
            assert_eq!(json["period_request"]["final_request_hns"], 10_000);
            assert!(json["failure"].is_string());
            assert!(json.get("native_buffer_frames").is_none());
        }
    }

    #[test]
    fn minimum_period_alignment_retry_preserves_legacy_buffer_bounds() {
        for minimum in [false, true] {
            for frames in [1, 48, 512, MAX_PACKET_FRAMES * 2] {
                let mut evidence = minimum.then(PeriodEvidence::new);
                vad_period::select_period(100_000, 10_000, evidence.as_mut()).unwrap();
                let latest = Mutex::new(WorkerState {
                    period_request: evidence,
                    ..WorkerState::default()
                });
                let period = prepare_alignment_retry(&latest, || {
                    assert!(latest.try_lock().is_ok());
                    Ok(frames)
                })
                .unwrap();
                let expected = (u64::from(frames) * 10_000_000).div_ceil(48_000) as i64;
                assert_eq!(period, expected);
                let state = latest.lock().unwrap();
                if minimum {
                    let evidence = state.period_request.unwrap();
                    assert!(evidence.alignment_retry);
                    assert_eq!(evidence.initial_request_hns, Some(10_000));
                    assert_eq!(evidence.final_request_hns, Some(expected));
                } else {
                    assert!(state.period_request.is_none());
                }
            }
        }
    }
}
