//! Shared/Exclusive WASAPI streams. All COM services and packet leases remain
//! on the creating worker, including alignment retry and final Release.
use crate::bridge::unsupported;
use crate::capture_packet::borrow_capture_packet;
use crate::codec::SampleCodec;
use crate::latency::{
    NativeStreamTiming, PreparedRenderQueue, RenderQueuePolicy, prepare_render_queue,
    render::{RenderPaddingObserver, RenderPaddingOwner, RenderPaddingSample},
    shared_period::{
        SharedFallbackReason, SharedFormatInfo, SharedInitializationApi,
        SharedInitializationFailure, SharedInitializationStage, SharedPeriodCaps,
        SharedPeriodReport,
    },
};
use crate::native::{
    ExclusivePeriodEvidence, ExclusivePeriodEvidenceHandle, NativeDeviceClockObservation,
    NativeStream, NativeStreamInfo, NativeStreamStats, StereoFrame, observe_exclusive_initialize,
};
use crate::negotiation::{
    EnginePeriods, SUBFORMAT_FLOAT, SUBFORMAT_PCM, StageError, aligned_shared_buffer_duration,
    client3_rejects_format, exclusive_format_supported, exclusive_frames_duration, friendly_name,
    initialize_exclusive, initialize_legacy_shared, initialize_legacy_shared_with_duration,
    parse_descriptor, shared_engine_periods, shared_initialize_retryable,
};
use crate::owner::{Apartment, LeaseFallback, OwnedHandle, PacketLease, TaskMemory};
use moiren_engine::{
    audio_config::{BUFFER_SIZES, SampleFormat},
    host::BackendError,
    node::{DefaultDeviceRole, DeviceMode, DeviceOptions, EndpointSelector},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{ptr, time::Duration};
use windows::{
    Win32::{
        Foundation::{PROPERTYKEY, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{IMMNotificationClient_Impl, *},
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, IAgileObject, IAgileObject_Impl},
            Threading::WaitForSingleObject,
        },
    },
    core::{HSTRING, Interface, PCWSTR, implement},
};

#[derive(Debug, Clone)]
pub struct WasapiOptions {
    pub endpoint_id: String,
    pub sample_rate: u32,
    pub device: DeviceOptions,
    pub input: bool,
    /// Opt-in queried minimum request plus preparation-only receipt. Requires
    /// input Exclusive mode without an explicit period; never used by read().
    pub exclusive_period_evidence: Option<ExclusivePeriodEvidenceHandle>,
}

pub(crate) fn native_period_request_supported(device: DeviceOptions, input: bool) -> bool {
    // Diagnostic-only exact render request; never widen the app's presets or
    // admit this period for Shared, capture, ASIO or a default library build.
    if cfg!(feature = "vad-probe")
        && !input
        && device.backend == moiren_engine::audio_config::AudioBackend::Wasapi
        && device.mode == DeviceMode::Exclusive
        && device.period_frames == Some(144)
    {
        return true;
    }
    device
        .period_frames
        .is_none_or(|frames| (64..=4096).contains(&frames) && frames.is_power_of_two())
}

fn capture_packet_frames(
    mode: DeviceMode,
    next_packet: impl FnOnce() -> windows::core::Result<u32>,
    acquire: impl FnOnce() -> windows::core::Result<u32>,
) -> Result<u32, BackendError> {
    // GetNextPacketSize is Shared-only. Exclusive capture polls GetBuffer
    // directly; BUFFER_ERROR means wait for the next processing pass.
    if mode == DeviceMode::Shared && api("GetNextPacketSize", next_packet())? == 0 {
        return Ok(0);
    }
    match acquire() {
        Err(error) if mode == DeviceMode::Exclusive && error.code() == AUDCLNT_E_BUFFER_ERROR => {
            Ok(0)
        }
        result => api("GetBuffer(capture)", result),
    }
}

fn api<T>(stage: &'static str, result: windows::core::Result<T>) -> Result<T, BackendError> {
    result.map_err(|error| {
        let failure = StageError::new(stage, error);
        let message = format!(
            "{}: HRESULT 0x{:08X}",
            failure.stage, failure.hresult as u32
        );
        let code = failure.hresult;
        if code == AUDCLNT_E_UNSUPPORTED_FORMAT.0
            || code == AUDCLNT_E_INVALID_DEVICE_PERIOD.0
            || code == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED.0
            || code == AUDCLNT_E_BUFFER_SIZE_ERROR.0
        {
            BackendError::Unsupported(message)
        } else if code == AUDCLNT_E_DEVICE_IN_USE.0 {
            BackendError::ResourceUnavailable(format!(
                "{message}; the endpoint is already owned by another exclusive stream"
            ))
        } else {
            BackendError::ResourceUnavailable(message)
        }
    })
}

pub(super) fn resolve_endpoint(
    selector: &EndpointSelector,
    input: bool,
) -> Result<String, BackendError> {
    selector.validate().map_err(unsupported)?;
    let _apartment = api("CoInitializeEx(resolve)", Apartment::new())?;
    let enumerator: IMMDeviceEnumerator = api("MMDeviceEnumerator", unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    })?;
    let flow = if input { eCapture } else { eRender };
    let device = api(
        "resolve endpoint",
        match selector {
            EndpointSelector::PinnedEndpoint { endpoint_id, .. } => {
                if endpoint_id.contains('\0') {
                    return Err(unsupported("endpoint ID contains NUL"));
                }
                unsafe { enumerator.GetDevice(&HSTRING::from(endpoint_id)) }
            }
            EndpointSelector::FollowDefault(role) => unsafe {
                enumerator.GetDefaultAudioEndpoint(
                    flow,
                    match role {
                        DefaultDeviceRole::Console => eConsole,
                        DefaultDeviceRole::Multimedia => eMultimedia,
                        DefaultDeviceRole::Communications => eCommunications,
                    },
                )
            },
        },
    )?;
    let endpoint: IMMEndpoint = api("IMMEndpoint", device.cast())?;
    if api("GetState", unsafe { device.GetState() })? != DEVICE_STATE_ACTIVE
        || api("GetDataFlow", unsafe { endpoint.GetDataFlow() })? != flow
    {
        return Err(BackendError::ResourceUnavailable(
            "endpoint is inactive or has the wrong direction".into(),
        ));
    }
    let id = api("GetId", unsafe { device.GetId() })?;
    api("endpoint ID", crate::owner::take_string(id))
}

pub struct WasapiStream {
    capture: Option<IAudioCaptureClient>,
    render: Option<IAudioRenderClient>,
    device_clock: Option<PreparedDeviceClock>,
    capture_evidence: Option<crate::capture::PcmEvidenceWriter>,
    client: IAudioClient,
    event: OwnedHandle,
    info: NativeStreamInfo,
    timing: NativeStreamTiming,
    shared_period: Option<SharedPeriodReport>,
    exclusive_period: Option<ExclusivePeriodEvidence>,
    render_queue: Option<PreparedRenderQueue>,
    padding_observer: Option<RenderPaddingObserver>,
    padding_owner: Option<RenderPaddingOwner>,
    codec: SampleCodec,
    mode: DeviceMode,
    started: bool,
    render_ready: bool,
    captured: Box<[StereoFrame]>,
    captured_offset: usize,
    captured_frames: usize,
    stats: NativeStreamStats,
    notifications: EndpointNotificationRegistration,
    // Declared last so every COM interface releases before CoUninitialize.
    _apartment: Apartment,
}

/// Endpoint notifications are control events, not a PCM-progress watchdog.
/// Callbacks only latch invalidation; the native owner observes it on its
/// existing bounded wait and reports the concrete endpoint state change.
struct EndpointNotificationRegistration {
    enumerator: IMMDeviceEnumerator,
    callback: IMMNotificationClient,
    invalidated: Arc<AtomicBool>,
    registered: bool,
}

#[implement(IMMNotificationClient, IAgileObject)]
struct EndpointNotifications {
    endpoint_id: Box<[u16]>,
    invalidated: Arc<AtomicBool>,
}

impl IAgileObject_Impl for EndpointNotifications_Impl {}

impl EndpointNotifications {
    fn matches(&self, endpoint_id: &PCWSTR) -> bool {
        if endpoint_id.0.is_null() {
            return false;
        }
        for (index, expected) in self.endpoint_id.iter().enumerate() {
            // Stop at the callback's terminator so a shorter unrelated ID
            // never causes reads beyond the borrowed string allocation.
            let actual = unsafe { endpoint_id.0.add(index).read() };
            if actual != *expected {
                return false;
            }
            if *expected == 0 {
                return true;
            }
        }
        false
    }

    fn invalidate_if_matching(&self, endpoint_id: &PCWSTR) {
        if self.matches(endpoint_id) {
            self.invalidated.store(true, Ordering::Release);
        }
    }
}

impl IMMNotificationClient_Impl for EndpointNotifications_Impl {
    fn OnDeviceStateChanged(
        &self,
        endpoint_id: &PCWSTR,
        state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        if state != DEVICE_STATE_ACTIVE {
            self.invalidate_if_matching(endpoint_id);
        }
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, endpoint_id: &PCWSTR) -> windows::core::Result<()> {
        self.invalidate_if_matching(endpoint_id);
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _: EDataFlow,
        _: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

impl EndpointNotificationRegistration {
    fn register(enumerator: IMMDeviceEnumerator, endpoint_id: &str) -> Result<Self, BackendError> {
        let invalidated = Arc::new(AtomicBool::new(false));
        let callback: IMMNotificationClient = EndpointNotifications {
            endpoint_id: endpoint_id.encode_utf16().chain([0]).collect(),
            invalidated: invalidated.clone(),
        }
        .into();
        api("RegisterEndpointNotificationCallback", unsafe {
            enumerator.RegisterEndpointNotificationCallback(&callback)
        })?;
        Ok(Self {
            enumerator,
            callback,
            invalidated,
            registered: true,
        })
    }

    fn check(&self) -> Result<(), BackendError> {
        if self.invalidated.load(Ordering::Acquire) {
            Err(BackendError::ResourceUnavailable(
                "selected WASAPI endpoint was removed or became inactive".into(),
            ))
        } else {
            Ok(())
        }
    }

    fn unregister(&mut self) -> Result<(), BackendError> {
        if self.registered {
            api("UnregisterEndpointNotificationCallback", unsafe {
                self.enumerator
                    .UnregisterEndpointNotificationCallback(&self.callback)
            })?;
            self.registered = false;
        }
        Ok(())
    }
}

impl Drop for EndpointNotificationRegistration {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

impl WasapiStream {
    pub fn exclusive_period(&self) -> Option<ExclusivePeriodEvidence> {
        self.exclusive_period
    }
    pub fn open(options: WasapiOptions) -> Result<Self, BackendError> {
        let requested_mode = options.device.mode;
        let requested_period = options.device.period_frames;
        let requested_rate = options.sample_rate;
        let requested_format = options.device.sample_format;
        let direction = if options.input { "input" } else { "output" };
        let endpoint_id = options.endpoint_id.clone();
        let result = Self::open_inner(options);
        if let Err(error) = &result {
            log::error!(
                target: "moiren::audio::wasapi",
                "WASAPI stream open failed: endpoint={endpoint_id:?} direction={direction} mode={requested_mode:?} requested_sample_rate={requested_rate} requested_format={requested_format:?} requested_period_frames={requested_period:?} error={error}"
            );
        }
        result
    }

    /// Result of the format-selection phase of `open_inner`: the descriptor that
    /// later phases negotiate, its decoded codec pair, and whether Shared mode
    /// will convert between the client format and the endpoint mix.
    fn open_inner(options: WasapiOptions) -> Result<Self, BackendError> {
        if options.exclusive_period_evidence.is_some()
            && (options.device.mode != DeviceMode::Exclusive || !options.input)
        {
            return Err(unsupported(
                "Exclusive period evidence requires an input Exclusive WASAPI stream",
            ));
        }
        if options.exclusive_period_evidence.is_some() && options.device.period_frames.is_some() {
            return Err(unsupported(
                "Exclusive minimum-period evidence cannot combine with an explicit period",
            ));
        }
        if options.endpoint_id.trim().is_empty()
            || options.endpoint_id.contains('\0')
            || !(8_000..=384_000).contains(&options.sample_rate)
            || !native_period_request_supported(options.device, options.input)
        {
            return Err(unsupported(
                "invalid WASAPI endpoint, rate or buffer request",
            ));
        }
        let apartment = api("CoInitializeEx(WASAPI)", Apartment::new())?;
        let enumerator: IMMDeviceEnumerator = api("MMDeviceEnumerator", unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        })?;
        let device = api("GetDevice", unsafe {
            enumerator.GetDevice(&HSTRING::from(&options.endpoint_id))
        })?;
        let endpoint_name = friendly_name(&device)
            .ok()
            .unwrap_or_else(|| "<unknown>".into());
        let endpoint: IMMEndpoint = api("IMMEndpoint", device.cast())?;
        if api("GetDataFlow", unsafe { endpoint.GetDataFlow() })?
            != if options.input { eCapture } else { eRender }
        {
            return Err(unsupported("WASAPI endpoint direction does not match node"));
        }
        let notifications =
            EndpointNotificationRegistration::register(enumerator.clone(), &options.endpoint_id)?;
        // Close the race where the endpoint becomes inactive just before
        // notification registration begins; subsequent changes are latched by
        // the registered callback.
        if api("GetState(after notification registration)", unsafe {
            device.GetState()
        })? != DEVICE_STATE_ACTIVE
        {
            return Err(BackendError::ResourceUnavailable(
                "selected WASAPI endpoint became inactive during stream preparation".into(),
            ));
        }
        let event = api("CreateEvent", OwnedHandle::event())?;
        let client: IAudioClient = api("Activate(IAudioClient)", unsafe {
            device.Activate(CLSCTX_ALL, None)
        })?;
        let memory = TaskMemory(api("GetMixFormat", unsafe { client.GetMixFormat() })?);
        if memory.0.is_null() {
            return Err(unsupported("device returned a null mix format"));
        }
        // GetMixFormat owns a complete base+cbSize allocation. Read packed fields
        // by value; no reference to an unaligned native field escapes.
        let base = unsafe { memory.0.read_unaligned() };
        let mix = descriptor(memory.0)?;
        let mix_info = shared_format_info(&mix);
        let mix_rate = base.nSamplesPerSec;
        let mix_channels = base.nChannels;
        let mix_container_bits = base.wBitsPerSample;
        let mix_block_align = base.nBlockAlign;
        log::debug!(
            target: "moiren::audio::wasapi",
            "WASAPI device capability: endpoint={:?} name={:?} direction={} mode={:?} mix_rate={} mix_format={:?} mix_channels={} mix_container_bits={} mix_block_align={} default_period_100ns_pending",
            options.endpoint_id,
            endpoint_name,
            if options.input { "input" } else { "output" },
            options.device.mode,
            mix_rate,
            codec_for(&mix)?.0,
            mix_channels,
            mix_container_bits,
            mix_block_align,
        );
        let exclusive = options.device.mode == DeviceMode::Exclusive;
        let selected = select_format(
            &options,
            &client,
            mix,
            mix_channels,
            &endpoint_name,
            exclusive,
        )?;
        let SelectedFormat {
            format,
            sample_format,
            codec,
            shared_conversion: _,
        } = &selected;
        let rate = format.Format.nSamplesPerSec;
        let mut period_report = (!exclusive).then(|| SharedPeriodReport {
            mix_format: Some(mix_info),
            client_format: Some(shared_format_info(format)),
            windows_auto_conversion: selected.shared_conversion,
            desired_period_ms: options
                .device
                .period_frames
                .map(|frames| frames as f64 * 1000.0 / f64::from(rate)),
            requested_period_frames: options.device.period_frames.map(|frames| frames as u32),
            ..Default::default()
        });
        let exclusive_period_handle = options.exclusive_period_evidence.clone();
        let client = if exclusive {
            negotiate_exclusive(
                &options,
                &device,
                client,
                &selected,
                &endpoint_name,
                exclusive_period_handle.as_ref(),
            )?
        } else if let Some(period_frames) = options.device.period_frames {
            negotiate_shared_exact(
                &options,
                &client,
                &selected,
                &endpoint_name,
                period_frames,
                period_report.as_mut().expect("Shared period report"),
            )?;
            client
        } else {
            initialize_shared_legacy(
                &client,
                &format.Format,
                "Initialize(Shared)",
                period_report.as_mut().expect("Shared period report"),
            )?;
            period_report
                .as_mut()
                .expect("Shared period report")
                .initialization_api = Some(SharedInitializationApi::LegacyClient);
            client
        };
        if let Some(report) = period_report.as_mut() {
            record_current_shared_period(&client, rate, report);
        }
        api("SetEventHandle", unsafe { client.SetEventHandle(event.0) })?;
        let capacity_result = unsafe { client.GetBufferSize() };
        if let Some(handle) = exclusive_period_handle.as_ref() {
            let mut evidence = handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            evidence.record_actual_hresult(match &capacity_result {
                Ok(_) => 0,
                Err(error) => error.code().0,
            });
        }
        let capacity = api("GetBufferSize", capacity_result)?;
        if let Some(handle) = exclusive_period_handle.as_ref() {
            handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .record_actual(capacity);
        }
        if capacity == 0 || capacity > 65536 {
            return Err(unsupported(
                "native WASAPI buffer exceeds the prepared frame limit",
            ));
        }
        if exclusive
            && options
                .device
                .period_frames
                .is_some_and(|requested| requested != capacity as usize)
        {
            return Err(unsupported(format!(
                "driver returned {capacity} frames instead of requested Exclusive buffer"
            )));
        }
        let negotiated_channels = format.Format.nChannels;
        log::info!(
            target: "moiren::audio::wasapi",
            "WASAPI stream opened: endpoint={:?} name={:?} direction={} mode={:?} rate={} format={:?} channels={} buffer_frames={} requested_period_frames={:?}",
            options.endpoint_id,
            endpoint_name,
            if options.input { "input" } else { "output" },
            options.device.mode,
            rate,
            sample_format,
            negotiated_channels,
            capacity,
            options.device.period_frames,
        );
        let capture = if options.input {
            Some(api("IAudioCaptureClient", unsafe { client.GetService() })?)
        } else {
            None
        };
        let render = if options.input {
            None
        } else {
            Some(api("IAudioRenderClient", unsafe { client.GetService() })?)
        };
        // Diagnostic service discovery and frequency query stay in preparation.
        // Neither a missing service nor a bad clock disables native audio.
        let device_clock = if options.input {
            None
        } else {
            // Clock tracing applies to render timelines; capture timestamps
            // come from GetBuffer and remain a separate raw evidence source.
            unsafe { client.GetService::<IAudioClock>() }
                .ok()
                .map(PreparedDeviceClock::new)
        };
        let timing = NativeStreamTiming {
            sample_rate: rate,
            // Exclusive event callbacks consume exactly the negotiated packet.
            // Shared capacity and the requested period are not observations.
            period_frames: if exclusive {
                Some(capacity)
            } else {
                period_report
                    .as_ref()
                    .and_then(|report| report.actual_period_frames)
            },
            capacity_frames: capacity,
        };
        let render_queue = prepare_wasapi_render_queue(
            timing,
            options.input,
            options.device.mode,
            RenderQueuePolicy::FullCapacity,
        )?;
        let padding_observer = (!options.input && !exclusive).then(RenderPaddingObserver::new);
        let padding_owner = padding_observer
            .as_ref()
            .map(RenderPaddingObserver::new_owner);
        Ok(Self {
            capture,
            render,
            device_clock,
            capture_evidence: None,
            client,
            event,
            info: NativeStreamInfo {
                sample_rate: rate,
                format: *sample_format,
                buffer_frames: capacity,
                channels: usize::from(format.Format.nChannels),
            },
            timing,
            shared_period: period_report,
            exclusive_period: exclusive_period_handle.map(|handle| {
                handle
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .to_owned()
            }),
            render_queue,
            padding_observer,
            padding_owner,
            codec: *codec,
            mode: options.device.mode,
            started: false,
            render_ready: true,
            captured: vec![[0.0; 2]; capacity as usize].into_boxed_slice(),
            captured_offset: 0,
            captured_frames: 0,
            stats: NativeStreamStats::default(),
            notifications,
            _apartment: apartment,
        })
    }
}

struct PreparedDeviceClock {
    clock: IAudioClock,
    frequency_hz: Option<u64>,
    frequency_hresult: Option<i32>,
}

impl PreparedDeviceClock {
    fn new(clock: IAudioClock) -> Self {
        let (frequency_hz, frequency_hresult) = match unsafe { clock.GetFrequency() } {
            // A successful zero result cannot normalize position. Preserve
            // unknown units without fabricating an API failure or sample rate.
            Ok(frequency) => ((frequency != 0).then_some(frequency), None),
            Err(error) => (None, Some(error.code().0)),
        };
        Self {
            clock,
            frequency_hz,
            frequency_hresult,
        }
    }

    fn sample(&self) -> NativeDeviceClockObservation {
        let mut sample = crate::latency::DeviceClockObservation::default();
        // The Result-returning wrapper erases S_FALSE. Calling the same COM
        // method through its vtable preserves the native accuracy/error code.
        // SAFETY: this owner retains the service and supplies live scalars.
        sample.hresult = unsafe {
            (Interface::vtable(&self.clock).GetPosition)(
                Interface::as_raw(&self.clock),
                &mut sample.position_units,
                &mut sample.qpc_100ns,
            )
        }
        .0;
        NativeDeviceClockObservation {
            frequency_hz: self.frequency_hz,
            frequency_hresult: self.frequency_hresult,
            sample,
        }
    }
}

fn shared_format_info(format: &WAVEFORMATEXTENSIBLE) -> SharedFormatInfo {
    let extended = format.Format.wFormatTag == crate::negotiation::EXTENSIBLE_TAG;
    // GUID comparison borrows both operands; copy the packed field first.
    let sub_format = format.SubFormat;
    SharedFormatInfo {
        sample_rate: format.Format.nSamplesPerSec,
        channels: format.Format.nChannels,
        container_bits: format.Format.wBitsPerSample,
        // SAFETY: parsed/synthesized descriptors initialize this union member.
        valid_bits: if extended {
            // SAFETY: the extensible tag guarantees this union member is valid.
            Some(unsafe { format.Samples.wValidBitsPerSample })
        } else {
            None
        },
        channel_mask: extended.then_some(format.dwChannelMask),
        floating_point: sub_format == SUBFORMAT_FLOAT,
    }
}

fn record_shared_period(
    frames: u32,
    engine_rate: u32,
    client_rate: u32,
    report: &mut SharedPeriodReport,
) -> windows::core::Result<()> {
    use windows::Win32::Foundation::E_INVALIDARG;
    if frames == 0 || engine_rate == 0 || client_rate == 0 {
        return Err(E_INVALIDARG.into());
    }
    let normalized = (u64::from(frames) * u64::from(client_rate)).div_ceil(u64::from(engine_rate));
    let normalized =
        u32::try_from(normalized).map_err(|_| windows::core::Error::from_hresult(E_INVALIDARG))?;
    report.engine_period_frames = Some(frames);
    report.engine_sample_rate = Some(engine_rate);
    report.actual_period_frames = Some(normalized);
    Ok(())
}

fn record_current_shared_period(
    client: &IAudioClient,
    client_rate: u32,
    report: &mut SharedPeriodReport,
) {
    let result = client.cast::<IAudioClient3>().and_then(|client3| {
        let mut pointer = ptr::null_mut();
        let mut period = 0;
        let result = unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut pointer, &mut period) };
        // Retain the COM allocation until after the descriptor read, including
        // failure cleanup. A copied pointer does not retain TaskMemory's owner.
        let memory = TaskMemory(pointer);
        result?;
        if memory.0.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        // SAFETY: this call returned a complete owned WAVEFORMATEX descriptor.
        let format = unsafe { memory.0.read_unaligned() };
        record_shared_period(period, format.nSamplesPerSec, client_rate, report)
    });
    if let Err(error) = result {
        report.actual_period_hresult = Some(error.code().0);
    }
}

fn shared_render_demand(
    timing: NativeStreamTiming,
    queue: PreparedRenderQueue,
    owner: Option<&mut RenderPaddingOwner>,
    padding: u32,
) -> Result<usize, BackendError> {
    let demand = queue.demand(padding).map_err(unsupported)?;
    if let Some(owner) = owner {
        // Publish valid observations even when the target is already full.
        // The control reader never sees capacity presented as a period.
        owner.publish(RenderPaddingSample {
            padding_frames: padding,
            capacity_frames: timing.capacity_frames,
            sample_rate: Some(timing.sample_rate),
            period_frames: timing.period_frames,
        });
    }
    Ok(demand as usize)
}

fn record_capture_evidence(
    writer: Option<&mut crate::capture::PcmEvidenceWriter>,
    bytes: &[u8],
    frames: u32,
    flags: u32,
    device_position_frames: u64,
    qpc_100ns: u64,
) {
    if let Some(writer) = writer {
        let _ = writer.record_packet(
            bytes,
            crate::capture::PcmEvidencePacket {
                frames,
                flags,
                device_position_frames,
                qpc_100ns,
            },
        );
    }
}

fn prepare_wasapi_render_queue(
    timing: NativeStreamTiming,
    input: bool,
    mode: DeviceMode,
    policy: RenderQueuePolicy,
) -> Result<Option<PreparedRenderQueue>, BackendError> {
    if input {
        return match policy {
            RenderQueuePolicy::FullCapacity => Ok(None),
            RenderQueuePolicy::Target { .. } => {
                Err(unsupported("capture streams do not have a render queue"))
            }
        };
    }
    if mode == DeviceMode::Exclusive && policy != RenderQueuePolicy::FullCapacity {
        return Err(unsupported(
            "Exclusive render requires complete native packets",
        ));
    }
    prepare_render_queue(timing, policy)
        .map(Some)
        .map_err(unsupported)
}

struct SelectedFormat {
    format: WAVEFORMATEXTENSIBLE,
    sample_format: SampleFormat,
    codec: SampleCodec,
    shared_conversion: bool,
}

/// Phase 1: choose the client format descriptor from the requested options
/// and the device's mix format, including the Exclusive fallback matrix.
fn select_format(
    options: &WasapiOptions,
    client: &IAudioClient,
    mix: WAVEFORMATEXTENSIBLE,
    mix_channels: u16,
    endpoint_name: &str,
    exclusive: bool,
) -> Result<SelectedFormat, BackendError> {
    let channels = if options.device.mode == DeviceMode::Shared {
        usize::from(mix_channels)
    } else {
        2
    };
    let mut format = if options.device.sample_format == SampleFormat::Native {
        let mut mix = mix;
        mix.Format.nSamplesPerSec = options.sample_rate;
        mix.Format.nAvgBytesPerSec = options
            .sample_rate
            .checked_mul(u32::from(mix.Format.nBlockAlign))
            .ok_or_else(|| unsupported("native format size overflow"))?;
        mix
    } else {
        wave_format(options.device.sample_format, options.sample_rate, channels)?
    };
    let shared_conversion =
        options.device.mode == DeviceMode::Shared && !same_format(&format, &mix);
    if shared_conversion {
        let mix_rate = mix.Format.nSamplesPerSec;
        let client_rate = format.Format.nSamplesPerSec;
        log::info!(
            target: "moiren::audio::wasapi",
            "WASAPI Shared format conversion selected: endpoint={:?} name={:?} mix_rate={} mix_format={:?} client_rate={} client_format={:?}",
            options.endpoint_id,
            endpoint_name,
            mix_rate,
            codec_for(&mix)?.0,
            client_rate,
            codec_for(&format)?.0,
        );
    }
    if exclusive && !supported(client, &format.Format)? {
        if options.device.sample_format != SampleFormat::Native {
            return Err(unsupported(
                "requested Exclusive rate/sample format is not supported",
            ));
        }
        let mut selected = None;
        for candidate in [
            SampleFormat::Float32,
            SampleFormat::Pcm32,
            SampleFormat::Pcm24,
            SampleFormat::Pcm16,
            SampleFormat::Float64,
        ] {
            let candidate = wave_format(candidate, options.sample_rate, 2)?;
            if supported(client, &candidate.Format)? {
                selected = Some(candidate);
                break;
            }
        }
        format = selected
            .ok_or_else(|| unsupported("no supported Exclusive format at requested rate"))?;
    }
    let (sample_format, codec) = codec_for(&format)?;
    Ok(SelectedFormat {
        format,
        sample_format,
        codec,
        shared_conversion,
    })
}

/// Phase 2 (Exclusive): device-period precheck and Initialize with the
/// alignment retry. Consumes the audio client because a
/// BUFFER_SIZE_NOT_ALIGNED failure requires activating a fresh one; the
/// (possibly realigned) client is returned for service setup.
fn negotiate_exclusive(
    options: &WasapiOptions,
    device: &IMMDevice,
    mut client: IAudioClient,
    selected: &SelectedFormat,
    endpoint_name: &str,
    evidence_handle: Option<&ExclusivePeriodEvidenceHandle>,
) -> Result<IAudioClient, BackendError> {
    let format = &selected.format;
    let sample_format = selected.sample_format;
    let rate = format.Format.nSamplesPerSec;
    let (mut default_period, mut minimum_period) = (0, 0);
    let period_result =
        unsafe { client.GetDevicePeriod(Some(&mut default_period), Some(&mut minimum_period)) };
    if let Some(handle) = evidence_handle {
        let mut evidence = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        evidence.record_query_hresult(match &period_result {
            Ok(()) => 0,
            Err(error) => error.code().0,
        });
        if period_result.is_ok() {
            evidence.record_query(default_period, minimum_period);
        }
    }
    api("GetDevicePeriod", period_result)?;
    let duration = exclusive_request_duration(options, default_period, minimum_period, rate)?;
    if let Some(requested_frames) = options.device.period_frames
        && let Some(minimum_frames) = minimum_exclusive_frames(minimum_period, rate)
        && u64::try_from(requested_frames)
            .is_ok_and(|requested| requested < u64::from(minimum_frames))
    {
        let next_preset = next_exclusive_preset(minimum_frames);
        let suggestion = next_preset.map_or_else(
            || "no configured buffer preset is large enough".to_owned(),
            |frames| format!("choose {frames} frames or larger"),
        );
        log::warn!(
            target: "moiren::audio::wasapi",
            "WASAPI exclusive period rejected before Initialize: endpoint={:?} name={:?} rate={} requested_frames={} minimum_frames={} next_preset={:?}",
            options.endpoint_id,
            endpoint_name,
            rate,
            requested_frames,
            minimum_frames,
            next_preset,
        );
        return Err(unsupported(format!(
            "requested Exclusive buffer {requested_frames} frames ({:.2} ms) is below the device minimum {minimum_frames} frames ({:.2} ms) at {rate} Hz; {suggestion}",
            frames_milliseconds(requested_frames as u32, rate),
            frames_milliseconds(minimum_frames, rate),
        )));
    }
    let selected_channels = format.Format.nChannels;
    let selected_bits = format.Format.wBitsPerSample;
    log::debug!(
        target: "moiren::audio::wasapi",
        "WASAPI initialize request: endpoint={:?} name={:?} direction={} requested_rate={} selected_format={:?} channels={} bits={} period_frames={:?} duration_100ns={} device_default_period_100ns={} device_minimum_period_100ns={}",
        options.endpoint_id,
        endpoint_name,
        if options.input { "input" } else { "output" },
        rate,
        sample_format,
        selected_channels,
        selected_bits,
        options.device.period_frames,
        duration,
        default_period,
        minimum_period,
    );
    let initialized = observe_exclusive_initialize(
        evidence_handle,
        duration,
        false,
        // SAFETY: `format.Format` remains part of `selected` for this call.
        |actual| unsafe { initialize_exclusive(&client, &format.Format, actual) },
        |error| error.code().0,
    );
    if initialized
        .as_ref()
        .is_err_and(|e| e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
    {
        if let Some(handle) = evidence_handle {
            handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .begin_alignment();
        }
        let aligned_result = unsafe { client.GetBufferSize() };
        if let Some(handle) = evidence_handle {
            let mut evidence = handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            evidence.record_alignment_hresult(match &aligned_result {
                Ok(_) => 0,
                Err(error) => error.code().0,
            });
        }
        let aligned = api("GetBufferSize(alignment)", aligned_result)?;
        if let Some(handle) = evidence_handle {
            handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .record_alignment(aligned);
        }
        if evidence_handle.is_some() && !(1..=65536).contains(&aligned) {
            return Err(unsupported(
                "exclusive alignment query exceeds the prepared frame limit",
            ));
        }
        if options
            .device
            .period_frames
            .is_some_and(|requested| requested != aligned as usize)
        {
            return Err(unsupported(format!(
                "requested Exclusive buffer requires {aligned} aligned frames"
            )));
        }
        // Failed initialization requires a fresh audio client.
        drop(client);
        client = api("Activate(aligned client)", unsafe {
            device.Activate(CLSCTX_ALL, None)
        })?;
        let retry = observe_exclusive_initialize(
            evidence_handle,
            exclusive_frames_duration(aligned, rate),
            true,
            // SAFETY: the same selected format outlives this fresh-client call.
            |actual| unsafe { initialize_exclusive(&client, &format.Format, actual) },
            |error| error.code().0,
        );
        api(
            "Initialize(Exclusive aligned)",
            // The shared negotiation helper owns the raw COM call; this map
            // keeps the professional BackendError stage and policy.
            retry,
        )?;
    } else {
        api("Initialize(Exclusive)", initialized)?;
    }
    Ok(client)
}

fn exclusive_request_duration(
    options: &WasapiOptions,
    default_hns: i64,
    minimum_hns: i64,
    rate: u32,
) -> Result<i64, BackendError> {
    if options.exclusive_period_evidence.is_some() {
        if !(1_000..=1_000_000).contains(&minimum_hns) {
            return Err(unsupported(
                "exclusive minimum period query returned an invalid period",
            ));
        }
        Ok(minimum_hns)
    } else {
        Ok(options.device.period_frames.map_or(default_hns, |frames| {
            exclusive_frames_duration(frames as u32, rate)
        }))
    }
}

/// Phase 2 (Shared, explicit period): exact period negotiation through
/// IAudioClient3's engine periods, falling back to compatibility
/// initialization on the kernel rejection matrix (D4 policy).
fn negotiate_shared_exact(
    options: &WasapiOptions,
    client: &IAudioClient,
    selected: &SelectedFormat,
    endpoint_name: &str,
    period_frames: usize,
    report: &mut SharedPeriodReport,
) -> Result<(), BackendError> {
    let format = &selected.format;
    let sample_format = selected.sample_format;
    let rate = format.Format.nSamplesPerSec;
    // Explicit requests are exact, not rounded to a convenient driver
    // period. Legacy conversion cannot promise an exact event period.
    let period = period_frames as u32;
    if selected.shared_conversion {
        log::warn!(
            target: "moiren::audio::wasapi",
            "WASAPI Shared explicit period uses compatibility initialization because the requested client format differs from the endpoint mix: endpoint={:?} requested_rate={} requested_format={:?} requested_frames={}",
            options.endpoint_id,
            rate,
            sample_format,
            period,
        );
        initialize_shared_legacy(
            client,
            &format.Format,
            "Initialize(Shared compatibility)",
            report,
        )?;
        report.initialization_api = Some(SharedInitializationApi::LegacyClient);
        report.fallback_reason = Some(SharedFallbackReason::WindowsFormatConversion);
        return Ok(());
    }
    let client3: Option<IAudioClient3> = match client.cast() {
        Ok(client3) => Some(client3),
        // The professional stack applies the kernel rejection
        // matrix to this interface query as well; the legacy
        // shared stack falls back on E_NOINTERFACE alone.
        Err(error) if client3_rejects_format(error.code()) => {
            report.client3_failure = Some(SharedInitializationFailure {
                stage: SharedInitializationStage::InterfaceQuery,
                hresult: error.code().0,
            });
            report.fallback_reason = Some(SharedFallbackReason::Client3Unavailable);
            log::warn!(
                target: "moiren::audio::wasapi",
                "WASAPI Client3 unavailable for exact Shared period; using compatibility initialization: endpoint={:?} hresult=0x{:08X} requested_frames={}",
                options.endpoint_id,
                error.code().0 as u32,
                period,
            );
            initialize_shared_legacy(
                client,
                &format.Format,
                "Initialize(Shared compatibility)",
                report,
            )?;
            report.initialization_api = Some(SharedInitializationApi::LegacyClient);
            None
        }
        Err(error) => return api("IAudioClient3(exact period)", Err(error)),
    };
    if let Some(client3) = client3 {
        // SAFETY: format is the locally owned selected descriptor,
        // valid for this query.
        let capabilities =
            unsafe { shared_engine_periods(&client3, std::ptr::from_ref(&format.Format)) };
        match capabilities {
            Ok(periods) => {
                report.caps = Some(SharedPeriodCaps {
                    default_frames: periods.default_frames,
                    fundamental_frames: periods.fundamental_frames,
                    min_frames: periods.min_frames,
                    max_frames: periods.max_frames,
                });
                let EnginePeriods {
                    default_frames: default,
                    fundamental_frames: fundamental,
                    min_frames: minimum,
                    max_frames: maximum,
                } = periods;
                log::info!(
                    target: "moiren::audio::wasapi",
                    "WASAPI shared period capabilities: endpoint={:?} name={:?} direction={} rate={} default_frames={} fundamental_frames={} minimum_frames={} maximum_frames={} requested_frames={}",
                    options.endpoint_id,
                    endpoint_name,
                    if options.input { "input" } else { "output" },
                    rate,
                    default,
                    fundamental,
                    minimum,
                    maximum,
                    period,
                );
                if fundamental == 0
                    || period < minimum
                    || period > maximum
                    || !period.is_multiple_of(fundamental)
                {
                    return Err(unsupported(format!(
                        "requested Shared period {period} is outside {minimum}..={maximum} or not a multiple of {fundamental}"
                    )));
                }
                // An explicit Shared period is a hard contract: out-of-range
                // periods were rejected above (never rounded), so
                // PERIODICITY_LOCKED surfaces instead of retrying with the
                // engine's current period.
                api("InitializeSharedAudioStream", unsafe {
                    client3.InitializeSharedAudioStream(
                        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                        period,
                        &format.Format,
                        None,
                    )
                })?;
                report.initialization_api = Some(SharedInitializationApi::Client3);
            }
            Err(error) if client3_rejects_format(error.code()) => {
                report.client3_failure = Some(SharedInitializationFailure {
                    stage: SharedInitializationStage::Capabilities,
                    hresult: error.code().0,
                });
                report.fallback_reason = Some(SharedFallbackReason::CapabilityQueryUnsupported);
                log::warn!(
                    target: "moiren::audio::wasapi",
                    "WASAPI Client3 rejected the Shared format; using compatibility initialization: endpoint={:?} hresult=0x{:08X} requested_frames={}",
                    options.endpoint_id,
                    error.code().0 as u32,
                    period,
                );
                initialize_shared_legacy(
                    client,
                    &format.Format,
                    "Initialize(Shared compatibility)",
                    report,
                )?;
                report.initialization_api = Some(SharedInitializationApi::LegacyClient);
            }
            Err(error) => {
                return api("GetSharedModeEnginePeriod", Err(error));
            }
        }
    }
    Ok(())
}

impl NativeStream for WasapiStream {
    fn info(&self) -> NativeStreamInfo {
        self.info
    }
    fn prepared_timing(&self) -> NativeStreamTiming {
        self.timing
    }
    fn shared_period(&self) -> Option<SharedPeriodReport> {
        self.shared_period
    }
    fn render_queue(&self) -> Option<PreparedRenderQueue> {
        self.render_queue
    }
    fn render_padding_observer(&self) -> Option<RenderPaddingObserver> {
        self.padding_observer.clone()
    }
    fn device_clock_observation(&self) -> Option<NativeDeviceClockObservation> {
        self.device_clock.as_ref().map(PreparedDeviceClock::sample)
    }
    fn attach_capture_evidence(
        &mut self,
        writer: crate::capture::PcmEvidenceWriter,
    ) -> Result<(), Box<(BackendError, crate::capture::PcmEvidenceWriter)>> {
        if self.started || self.capture.is_none() {
            return Err(Box::new((
                unsupported("capture evidence must be attached before Start to a capture stream"),
                writer,
            )));
        }
        if self.info.format != SampleFormat::Float32 || !(1..=2).contains(&self.info.channels) {
            return Err(Box::new((
                unsupported("raw capture evidence requires negotiated Float32 mono/stereo"),
                writer,
            )));
        }
        let summary = writer.snapshot();
        if summary.sample_rate != self.info.sample_rate || summary.channels != self.info.channels {
            return Err(Box::new((
                unsupported("capture evidence format does not match the native client"),
                writer,
            )));
        }
        if self.capture_evidence.is_some() {
            return Err(Box::new((
                unsupported("capture evidence is already attached"),
                writer,
            )));
        }
        self.capture_evidence = Some(writer);
        Ok(())
    }
    fn take_capture_evidence(&mut self) -> Option<crate::capture::PcmEvidenceWriter> {
        self.capture_evidence.take().map(|mut writer| {
            writer.close();
            writer
        })
    }
    fn configure_render_queue(&mut self, policy: RenderQueuePolicy) -> Result<(), BackendError> {
        if self.started {
            return Err(unsupported(
                "render queue must be configured before native Start",
            ));
        }
        self.render_queue =
            prepare_wasapi_render_queue(self.timing, self.capture.is_some(), self.mode, policy)?;
        Ok(())
    }
    fn render_demand(&mut self) -> Result<usize, BackendError> {
        self.notifications.check()?;
        if self.render.is_none() {
            return Ok(0);
        }
        if self.mode == DeviceMode::Exclusive {
            return Ok(if self.render_ready {
                self.info.buffer_frames as usize
            } else {
                0
            });
        }
        let padding = api("GetCurrentPadding", unsafe {
            self.client.GetCurrentPadding()
        })?;
        let queue = self.render_queue.expect("prepared render queue");
        shared_render_demand(self.timing, queue, self.padding_owner.as_mut(), padding)
    }
    fn start(&mut self) -> Result<(), BackendError> {
        self.notifications.check()?;
        if !self.started {
            // Stop invalidates the old generation. Restart is preparation,
            // before the native client can issue another audio callback.
            if let Some(observer) = &self.padding_observer {
                self.padding_owner = Some(observer.new_owner());
            }
            api("Start", unsafe { self.client.Start() })?;
            self.started = true;
        }
        Ok(())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        if let Some(owner) = self.padding_owner.as_mut() {
            owner.invalidate();
        }
        if self.started {
            api("Stop", unsafe { self.client.Stop() })?;
            self.started = false;
        }
        Ok(())
    }
    fn wait(&mut self, timeout: Duration) -> Result<bool, BackendError> {
        self.notifications.check()?;
        let ready = match unsafe {
            WaitForSingleObject(
                self.event.0,
                timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32,
            )
        } {
            WAIT_OBJECT_0 => {
                self.render_ready = true;
                self.stats.wait_wakes += 1;
                true
            }
            WAIT_TIMEOUT => {
                self.stats.wait_timeouts += 1;
                false
            }
            _ => {
                return api(
                    "WaitForSingleObject",
                    Err(windows::core::Error::from_thread()),
                );
            }
        };
        self.notifications.check()?;
        Ok(ready)
    }
    fn read(&mut self, output: &mut [StereoFrame]) -> Result<usize, BackendError> {
        self.notifications.check()?;
        let Some(capture) = &self.capture else {
            return Ok(0);
        };
        if output.is_empty() {
            return Ok(0);
        }
        if self.captured_frames == self.captured_offset {
            let (mut data, mut frames, mut flags, mut position, mut qpc) =
                (ptr::null_mut(), 0, 0, 0, 0);
            let available = capture_packet_frames(
                self.mode,
                || unsafe { capture.GetNextPacketSize() },
                || unsafe {
                    capture.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        Some(&mut position),
                        Some(&mut qpc),
                    )?;
                    Ok(frames)
                },
            )?;
            if available == 0 {
                // No packet was acquired. GetBuffer permits omitting ReleaseBuffer
                // for zero frames; errors likewise do not create a packet lease.
                return Ok(0);
            }
            let lease = PacketLease::new(capture, frames, LeaseFallback::CapturePacket);
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            self.stats.capture_discontinuities +=
                u64::from(flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0);
            self.stats.capture_timestamp_errors +=
                u64::from(flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0);
            if silent {
                self.stats.capture_silent_frames += u64::from(frames);
            }
            // SAFETY: data is borrowed from the live GetBuffer lease until
            // the ReleaseBuffer call below.
            let bytes = unsafe {
                borrow_capture_packet(
                    data,
                    frames,
                    self.info.buffer_frames,
                    silent,
                    self.codec.frame_bytes(),
                )
            }
            .map_err(|error| unsupported(format!("invalid capture packet: {error:?}")))?;
            // Evidence saturation is reported by the bounded writer but never
            // stalls or aborts native capture. Copy while the lease is valid.
            record_capture_evidence(
                self.capture_evidence.as_mut(),
                bytes,
                frames,
                flags,
                position,
                qpc,
            );
            if silent {
                self.captured[..frames as usize].fill([0.0; 2]);
            } else {
                self.codec
                    .decode(bytes, &mut self.captured)
                    .map_err(unsupported)?;
            }
            api("ReleaseBuffer(capture)", lease.release())?;
            self.captured_frames = frames as usize;
            self.captured_offset = 0;
        }
        let count = output
            .len()
            .min(self.captured_frames - self.captured_offset);
        output[..count]
            .copy_from_slice(&self.captured[self.captured_offset..self.captured_offset + count]);
        self.captured_offset += count;
        Ok(count)
    }
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        // Re-query after graph processing so a split Pump submission never
        // exceeds the target even if native consumption changes the padding.
        let demand = self.render_demand()?;
        let Some(render) = &self.render else {
            return Ok(0);
        };
        let count = if self.mode == DeviceMode::Exclusive {
            if demand == 0 || input.len() < self.info.buffer_frames as usize {
                return Ok(0);
            }
            self.info.buffer_frames as usize
        } else {
            input.len().min(demand)
        };
        if count == 0 {
            self.stats.zero_writes += 1;
            return Ok(0);
        }
        let data = api("GetBuffer(render)", unsafe {
            render.GetBuffer(count as u32)
        })?;
        let lease = PacketLease::new(render, count as u32, LeaseFallback::RenderSilent);
        if data.is_null() {
            return Err(unsupported("render packet has a null data pointer"));
        }
        // A successful GetBuffer reserves this complete writable frame region.
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(data, count * self.codec.frame_bytes()) };
        self.codec
            .encode(&input[..count], bytes)
            .map_err(unsupported)?;
        api("ReleaseBuffer(render)", lease.release())?;
        self.render_ready = false;
        self.stats.write_calls += 1;
        self.stats.written_frames += count as u64;
        Ok(count)
    }
    fn stats(&self) -> NativeStreamStats {
        NativeStreamStats {
            capture_quality_available: self.capture.is_some(),
            ..self.stats
        }
    }
    fn release(&mut self) -> Result<(), BackendError> {
        if let Some(owner) = self.padding_owner.as_mut() {
            owner.invalidate();
        }
        if let Some(writer) = self.capture_evidence.as_mut() {
            writer.close();
        }
        self.notifications.unregister()
    }
}
impl Drop for WasapiStream {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = self.notifications.unregister();
    }
}

fn frames_milliseconds(frames: u32, rate: u32) -> f64 {
    f64::from(frames) * 1000.0 / f64::from(rate)
}

fn minimum_exclusive_frames(period_100ns: i64, rate: u32) -> Option<u32> {
    if period_100ns <= 0 || rate == 0 {
        return None;
    }
    let frames = (u128::try_from(period_100ns).ok()? * u128::from(rate)).div_ceil(10_000_000);
    u32::try_from(frames).ok()
}

fn next_exclusive_preset(minimum_frames: u32) -> Option<u32> {
    BUFFER_SIZES
        .into_iter()
        .find(|&frames| frames >= minimum_frames)
}

fn initialize_shared_legacy(
    client: &IAudioClient,
    format: &WAVEFORMATEX,
    stage: &'static str,
    report: &mut SharedPeriodReport,
) -> Result<(), BackendError> {
    // Keep the professional stack's default persisted-session behavior and
    // conversion between its fixed client format and the endpoint mix.
    // SAFETY: format is the caller's live descriptor for both calls.
    let init_default =
        || unsafe { initialize_legacy_shared(client, std::ptr::from_ref(format), None) };
    let init_with_duration = |duration: i64| unsafe {
        initialize_legacy_shared_with_duration(client, std::ptr::from_ref(format), None, duration)
    };
    let initial = match init_default() {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if !shared_initialize_retryable(initial.code()) {
        return api(stage, Err(initial));
    }
    // The engine can reject the 0/0 default durations on the AUTOCONVERTPCM
    // path (observed as raw 0x8889000A). Retry exactly once with an explicit
    // buffer duration of two engine periods; a failed shared Initialize left
    // this client un-initialized, so it stays reusable (unlike the exclusive
    // alignment flow, which must activate a fresh client).
    let (mut default_period, mut minimum_period) = (0, 0);
    let aligned = match unsafe {
        client.GetDevicePeriod(Some(&mut default_period), Some(&mut minimum_period))
    } {
        Ok(()) => aligned_shared_buffer_duration(default_period)
            .or_else(|| aligned_shared_buffer_duration(minimum_period)),
        Err(error) => {
            log::warn!(
                target: "moiren::audio::wasapi",
                "WASAPI Shared aligned-duration retry skipped; GetDevicePeriod failed: hresult=0x{:08X} initial_hresult=0x{:08X}",
                error.code().0 as u32,
                initial.code().0 as u32,
            );
            None
        }
    };
    let Some(duration) = aligned else {
        return api(stage, Err(initial));
    };
    log::warn!(
        target: "moiren::audio::wasapi",
        "WASAPI Shared compatibility initialization returned hresult=0x{:08X}; retrying once with an aligned buffer duration of {duration} 100ns units",
        initial.code().0 as u32,
    );
    match init_with_duration(duration) {
        Ok(()) => {
            report.fallback_reason = Some(SharedFallbackReason::AlignedDurationRetry);
            Ok(())
        }
        // Retain the original rejection as the surfaced diagnosis; the retry
        // HRESULT stays in the log line above for evidence.
        Err(_retry) => api(stage, Err(initial)),
    }
}

fn same_format(left: &WAVEFORMATEXTENSIBLE, right: &WAVEFORMATEXTENSIBLE) -> bool {
    let left_base = left.Format;
    let right_base = right.Format;
    let left_valid = unsafe { left.Samples.wValidBitsPerSample };
    let right_valid = unsafe { right.Samples.wValidBitsPerSample };
    let left_mask = left.dwChannelMask;
    let right_mask = right.dwChannelMask;
    let left_sub_format = left.SubFormat;
    let right_sub_format = right.SubFormat;
    left_base.wFormatTag == right_base.wFormatTag
        && left_base.nChannels == right_base.nChannels
        && left_base.nSamplesPerSec == right_base.nSamplesPerSec
        && left_base.nAvgBytesPerSec == right_base.nAvgBytesPerSec
        && left_base.nBlockAlign == right_base.nBlockAlign
        && left_base.wBitsPerSample == right_base.wBitsPerSample
        && left_base.cbSize == right_base.cbSize
        && left_valid == right_valid
        && left_mask == right_mask
        && left_sub_format == right_sub_format
}

fn supported(client: &IAudioClient, format: &WAVEFORMATEX) -> Result<bool, BackendError> {
    // Preserve the professional error mapping around the shared raw query.
    let result = exclusive_format_supported(client, std::ptr::from_ref(format));
    match result {
        Ok(supported) => Ok(supported),
        Err(error) => api("IsFormatSupported(Exclusive)", Err(error)),
    }
}
fn wave_format(
    format: SampleFormat,
    rate: u32,
    channels: usize,
) -> Result<WAVEFORMATEXTENSIBLE, BackendError> {
    let bits = match format {
        SampleFormat::Pcm16 => 16,
        SampleFormat::Pcm24 => 24,
        SampleFormat::Pcm32 | SampleFormat::Float32 => 32,
        SampleFormat::Float64 => 64,
        SampleFormat::Native => return Err(unsupported("Native format needs a device descriptor")),
    };
    if !(1..=32).contains(&channels) {
        return Err(unsupported("unsupported native channel layout"));
    }
    let float = matches!(format, SampleFormat::Float32 | SampleFormat::Float64);
    let align = channels as u16 * (bits / 8);
    Ok(WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xfffe,
            nChannels: channels as u16,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * u32::from(align),
            nBlockAlign: align,
            wBitsPerSample: bits,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: bits,
        },
        dwChannelMask: if channels == 1 {
            4
        } else if channels == 2 {
            3
        } else {
            0
        },
        SubFormat: if float {
            SUBFORMAT_FLOAT
        } else {
            SUBFORMAT_PCM
        },
    })
}
fn descriptor(pointer: *const WAVEFORMATEX) -> Result<WAVEFORMATEXTENSIBLE, BackendError> {
    // SAFETY: GetMixFormat owns a complete base+cbSize allocation. The
    // kernel parse copies packed fields by value; no reference to an
    // unaligned native field escapes.
    let result = unsafe { parse_descriptor(pointer) }
        .map_err(|_| unsupported("unsupported native mix descriptor"))?
        .into_extensible();
    codec_for(&result)?;
    Ok(result)
}
fn codec_for(format: &WAVEFORMATEXTENSIBLE) -> Result<(SampleFormat, SampleCodec), BackendError> {
    let base = format.Format;
    let valid = if base.wFormatTag == 0xfffe {
        unsafe { format.Samples.wValidBitsPerSample }
    } else {
        base.wBitsPerSample
    };
    let valid = if valid == 0 {
        base.wBitsPerSample
    } else {
        valid
    };
    let sub_format = format.SubFormat;
    let sample = if sub_format == SUBFORMAT_FLOAT {
        match base.wBitsPerSample {
            32 => SampleFormat::Float32,
            64 => SampleFormat::Float64,
            _ => return Err(unsupported("unsupported floating-point descriptor")),
        }
    } else if sub_format == SUBFORMAT_PCM {
        match valid {
            16 if base.wBitsPerSample == 16 => SampleFormat::Pcm16,
            24 => SampleFormat::Pcm24,
            _ if base.wBitsPerSample == 32 => SampleFormat::Pcm32,
            _ => return Err(unsupported("unsupported PCM descriptor")),
        }
    } else {
        return Err(unsupported("non-PCM device format"));
    };
    let codec = SampleCodec::with_layout(
        sample,
        usize::from(base.nChannels),
        base.wBitsPerSample,
        valid,
    )
    .map_err(unsupported)?;
    if base.nSamplesPerSec == 0
        || codec.frame_bytes() != usize::from(base.nBlockAlign)
        || u64::from(base.nAvgBytesPerSec)
            != u64::from(base.nSamplesPerSec) * u64::from(base.nBlockAlign)
    {
        return Err(unsupported("inconsistent native format sizes"));
    }
    Ok((sample, codec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use windows::core::PCWSTR;

    #[test]
    fn native_explicit_render_144_requires_exclusive_probe_and_not_capture_or_asio() {
        let device = DeviceOptions {
            mode: DeviceMode::Exclusive,
            period_frames: Some(144),
            ..Default::default()
        };
        assert_eq!(
            native_period_request_supported(device, false),
            cfg!(feature = "vad-probe")
        );
        assert!(!native_period_request_supported(device, true));
        assert!(!native_period_request_supported(
            DeviceOptions {
                mode: DeviceMode::Shared,
                ..device
            },
            false
        ));
        assert!(!native_period_request_supported(
            DeviceOptions {
                backend: moiren_engine::audio_config::AudioBackend::Asio,
                ..device
            },
            false
        ));
        for period in [0, 143, 145, 160, 192, 480, 8192] {
            assert!(!native_period_request_supported(
                DeviceOptions {
                    period_frames: Some(period),
                    ..device
                },
                false
            ));
        }
        assert_eq!(
            exclusive_request_duration(
                &WasapiOptions {
                    endpoint_id: "not-opened".into(),
                    sample_rate: 48_000,
                    device,
                    input: false,
                    exclusive_period_evidence: None,
                },
                100_000,
                30_000,
                48_000
            )
            .unwrap(),
            30_000
        );
    }

    fn minimum_period_evidence() -> ExclusivePeriodEvidenceHandle {
        Arc::new(std::sync::Mutex::new(ExclusivePeriodEvidence {
            qualification: "unqualified-request-only",
            requested_minimum: true,
            default_hns: None,
            minimum_hns: None,
            query_hresult: None,
            initial_request_hns: None,
            final_request_hns: None,
            alignment_retry: false,
            alignment_query_hresult: None,
            aligned_buffer_frames: None,
            first_initialize_hresult: None,
            retry_initialize_hresult: None,
            actual_buffer_frames: None,
            actual_buffer_hresult: None,
        }))
    }

    #[test]
    fn exclusive_minimum_selection_preserves_default_and_refuses_invalid_query() {
        let mut options = WasapiOptions {
            endpoint_id: "not-opened".into(),
            sample_rate: 48_000,
            device: DeviceOptions {
                mode: DeviceMode::Exclusive,
                ..Default::default()
            },
            input: true,
            exclusive_period_evidence: None,
        };
        assert_eq!(
            exclusive_request_duration(&options, 100_000, 30_000, 48_000).unwrap(),
            100_000
        );
        options.device.period_frames = Some(256);
        assert_eq!(
            exclusive_request_duration(&options, 100_000, 30_000, 48_000).unwrap(),
            53_333
        );
        options.device.period_frames = None;
        let handle = minimum_period_evidence();
        options.exclusive_period_evidence = Some(handle.clone());
        assert_eq!(
            exclusive_request_duration(&options, 100_000, 30_000, 48_000).unwrap(),
            30_000
        );
        for invalid in [i64::MIN, 0, 999, 1_000_001] {
            handle.lock().unwrap().record_query(100_000, invalid);
            assert!(exclusive_request_duration(&options, 100_000, invalid, 48_000).is_err());
            let evidence = handle.lock().unwrap();
            assert_eq!(evidence.initial_request_hns, None);
            assert_eq!(evidence.final_request_hns, None);
            assert_eq!(evidence.first_initialize_hresult, None);
        }
    }

    #[test]
    fn exclusive_minimum_refuses_shared_output_and_explicit_period_before_com() {
        for (input, mode, period, expected) in [
            (true, DeviceMode::Shared, None, "input Exclusive"),
            (false, DeviceMode::Exclusive, None, "input Exclusive"),
            (true, DeviceMode::Exclusive, Some(256), "explicit period"),
        ] {
            let handle = minimum_period_evidence();
            let options = WasapiOptions {
                endpoint_id: "not-opened".into(),
                sample_rate: 48_000,
                device: DeviceOptions {
                    mode,
                    period_frames: period,
                    ..Default::default()
                },
                input,
                exclusive_period_evidence: Some(handle.clone()),
            };
            match WasapiStream::open_inner(options) {
                Err(error) => assert!(error.to_string().contains(expected)),
                Ok(_) => panic!("invalid request must refuse before native activation"),
            }
            let evidence = handle.lock().unwrap();
            assert_eq!(evidence.query_hresult, None);
            assert_eq!(evidence.first_initialize_hresult, None);
        }
    }

    #[test]
    fn exclusive_minimum_receipt_is_omitted_by_default_and_preserves_partial_failure() {
        use crate::{
            SessionDuration,
            capture::{CaptureReport, CaptureSource},
        };
        let mut report = CaptureReport::new(CaptureSource::Physical, SessionDuration::UntilStopped);
        let default = serde_json::to_value(&report).unwrap();
        assert!(default.get("exclusive_period").is_none());
        assert_eq!(default["schema_version"], 12);
        let handle = minimum_period_evidence();
        handle.lock().unwrap().record_query_hresult(-1);
        report.exclusive_period = Some(*handle.lock().unwrap());
        report.failure = Some("preserved preparation failure".into());
        let partial = serde_json::to_value(&report).unwrap();
        assert_eq!(partial["failure"], "preserved preparation failure");
        assert_eq!(partial["exclusive_period"]["query_hresult"], -1);
        assert!(partial["exclusive_period"]["minimum_hns"].is_null());
        assert!(partial["exclusive_period"]["initial_request_hns"].is_null());
    }

    #[test]
    fn exclusive_capture_skips_shared_only_packet_query() {
        let frames = capture_packet_frames(
            DeviceMode::Exclusive,
            || {
                Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_NOTIMPL,
                ))
            },
            || Ok(480),
        )
        .unwrap();
        assert_eq!(frames, 480);
    }

    #[test]
    fn exclusive_capture_empty_pass_never_becomes_signal_or_fatal_error() {
        assert_eq!(
            capture_packet_frames(
                DeviceMode::Exclusive,
                || Ok(480),
                || Err(windows::core::Error::from_hresult(AUDCLNT_E_BUFFER_ERROR)),
            )
            .unwrap(),
            0
        );
        for mode in [DeviceMode::Shared, DeviceMode::Exclusive] {
            assert_eq!(
                capture_packet_frames(mode, || Ok(480), || Ok(0)).unwrap(),
                0
            );
        }
    }

    #[test]
    fn capture_packet_polling_preserves_shared_admission_and_real_errors() {
        use windows::Win32::Media::Audio::AUDCLNT_E_OUT_OF_ORDER;

        assert_eq!(
            capture_packet_frames(DeviceMode::Shared, || Ok(0), || panic!("no packet lease"))
                .unwrap(),
            0
        );
        assert_eq!(
            capture_packet_frames(DeviceMode::Shared, || Ok(480), || Ok(480)).unwrap(),
            480
        );
        assert!(
            capture_packet_frames(
                DeviceMode::Shared,
                || Err(windows::core::Error::from_hresult(
                    AUDCLNT_E_DEVICE_INVALIDATED
                )),
                || panic!("query failure must be retained"),
            )
            .is_err()
        );
        for (mode, error) in [
            (DeviceMode::Shared, AUDCLNT_E_BUFFER_ERROR),
            (DeviceMode::Shared, AUDCLNT_E_DEVICE_INVALIDATED),
            (DeviceMode::Exclusive, AUDCLNT_E_DEVICE_INVALIDATED),
            (DeviceMode::Exclusive, AUDCLNT_E_OUT_OF_ORDER),
        ] {
            assert!(
                capture_packet_frames(
                    mode,
                    || Ok(480),
                    || Err(windows::core::Error::from_hresult(error)),
                )
                .is_err()
            );
        }
    }

    #[implement(IAudioClock)]
    struct DiagnosticClock {
        frequency: Result<u64, i32>,
        frequency_calls: std::rc::Rc<std::cell::Cell<usize>>,
        hresult: i32,
    }

    impl IAudioClock_Impl for DiagnosticClock_Impl {
        fn GetFrequency(&self) -> windows::core::Result<u64> {
            self.frequency_calls.set(self.frequency_calls.get() + 1);
            self.frequency
                .map_err(|code| windows::core::Error::from_hresult(windows::core::HRESULT(code)))
        }
        fn GetPosition(&self, position: *mut u64, qpc: *mut u64) -> windows::core::Result<()> {
            // SAFETY: PreparedDeviceClock::sample supplies live output scalars.
            unsafe {
                *position = 1234;
                *qpc = 5678;
            }
            if self.hresult == 0 {
                Ok(())
            } else {
                Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    self.hresult,
                )))
            }
        }
        fn GetCharacteristics(&self) -> windows::core::Result<u32> {
            unreachable!()
        }
    }

    #[test]
    fn raw_device_clock_keeps_frequency_units_qpc_and_low_accuracy_or_failure_hresult() {
        use windows::Win32::Foundation::E_FAIL;
        for hresult in [0, 1, E_FAIL.0] {
            let calls = std::rc::Rc::new(std::cell::Cell::new(0));
            let clock: IAudioClock = DiagnosticClock {
                frequency: Ok(384_000),
                frequency_calls: calls.clone(),
                hresult,
            }
            .into();
            let clock = PreparedDeviceClock::new(clock);
            assert_eq!(calls.get(), 1);
            for _ in 0..3 {
                let observation = clock.sample();
                assert_eq!(observation.frequency_hz, Some(384_000));
                assert_eq!(observation.frequency_hresult, None);
                assert_eq!(observation.sample.position_units, 1234);
                assert_eq!(observation.sample.qpc_100ns, 5678);
                assert_eq!(observation.sample.hresult, hresult);
            }
            assert_eq!(calls.get(), 1, "position sampling must not query frequency");
        }
    }

    #[test]
    fn raw_device_clock_never_invents_frequency_when_query_fails_or_returns_zero() {
        use windows::Win32::Foundation::E_FAIL;
        for frequency in [Err(E_FAIL.0), Ok(0)] {
            let clock: IAudioClock = DiagnosticClock {
                frequency,
                frequency_calls: std::rc::Rc::new(std::cell::Cell::new(0)),
                hresult: 1,
            }
            .into();
            let observation = PreparedDeviceClock::new(clock).sample();
            assert_eq!(observation.frequency_hz, None);
            assert_eq!(observation.frequency_hresult, frequency.err());
            assert_eq!(observation.sample.position_units, 1234);
            assert_eq!(observation.sample.qpc_100ns, 5678);
            assert_eq!(observation.sample.hresult, 1);
        }
    }

    #[test]
    fn actual_shared_period_is_normalized_instead_of_using_native_capacity() {
        for (frames, engine_rate, expected) in [(960, 96_000, 480), (128, 44_100, 140)] {
            let mut report = SharedPeriodReport::default();
            record_shared_period(frames, engine_rate, 48_000, &mut report).unwrap();
            assert_eq!(report.engine_period_frames, Some(frames));
            assert_eq!(report.engine_sample_rate, Some(engine_rate));
            assert_eq!(report.actual_period_frames, Some(expected));
        }
        for (frames, engine_rate, client_rate) in [
            (0, 48_000, 48_000),
            (480, 0, 48_000),
            (480, 48_000, 0),
            (u32::MAX, 1, u32::MAX),
        ] {
            let mut report = SharedPeriodReport::default();
            assert!(record_shared_period(frames, engine_rate, client_rate, &mut report).is_err());
            assert_eq!(report.actual_period_frames, None);
            assert_eq!(report.engine_period_frames, None);
        }
    }

    fn timing() -> NativeStreamTiming {
        NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(240),
            capacity_frames: 960,
        }
    }

    fn target() -> RenderQueuePolicy {
        RenderQueuePolicy::Target {
            frames: 480,
            wake_margin_frames: 120,
            callback_budget_frames: 120,
        }
    }

    #[test]
    fn target_queue_requires_actual_shared_timing_and_rejects_capture_and_exclusive() {
        let timing = timing();
        let queue = prepare_wasapi_render_queue(timing, false, DeviceMode::Shared, target())
            .unwrap()
            .unwrap();
        assert_eq!(queue.capacity_frames, 960);
        assert_eq!(queue.target_frames, 480);
        assert_eq!(queue.deadline_budget_frames, Some(480));
        assert!(prepare_wasapi_render_queue(timing, true, DeviceMode::Shared, target()).is_err());
        assert!(
            prepare_wasapi_render_queue(timing, false, DeviceMode::Exclusive, target()).is_err()
        );
        let unknown = NativeStreamTiming {
            period_frames: None,
            ..timing
        };
        let full = prepare_wasapi_render_queue(
            unknown,
            false,
            DeviceMode::Shared,
            RenderQueuePolicy::FullCapacity,
        )
        .unwrap()
        .unwrap();
        assert_eq!(full.target_frames, 960);
        assert_eq!(full.deadline_budget_frames, None);
        assert!(prepare_wasapi_render_queue(unknown, false, DeviceMode::Shared, target()).is_err());
        for frames in [479, 961] {
            let policy = RenderQueuePolicy::Target {
                frames,
                wake_margin_frames: 120,
                callback_budget_frames: 120,
            };
            assert!(
                prepare_wasapi_render_queue(timing, false, DeviceMode::Shared, policy).is_err()
            );
        }
    }

    #[test]
    fn live_padding_publishes_at_zero_demand_and_preserves_unknown_period() {
        let observer = RenderPaddingObserver::new();
        let mut owner = observer.new_owner();
        let queue = prepare_render_queue(timing(), target()).unwrap();
        for (padding, expected) in [(0, 480), (240, 240), (480, 0), (960, 0)] {
            assert_eq!(
                shared_render_demand(timing(), queue, Some(&mut owner), padding).unwrap(),
                expected
            );
            let sample = observer.try_snapshot().unwrap();
            assert_eq!(sample.padding_frames, padding);
            assert_eq!(sample.capacity_frames, 960);
            assert_eq!(sample.period_frames, Some(240));
        }
        assert!(shared_render_demand(timing(), queue, Some(&mut owner), 961).is_err());
        let unknown = NativeStreamTiming {
            period_frames: None,
            ..timing()
        };
        let queue = prepare_render_queue(unknown, RenderQueuePolicy::FullCapacity).unwrap();
        assert_eq!(
            shared_render_demand(unknown, queue, Some(&mut owner), 0).unwrap(),
            960
        );
        assert_eq!(observer.try_snapshot().unwrap().period_frames, None);
    }

    #[test]
    fn raw_capture_evidence_preserves_pcm_bits_flags_and_native_timestamps() {
        use crate::{
            capture::{PcmEvidenceConfig, pcm_evidence_stream},
            stats::TIMESTAMP_ERROR,
        };
        let (mut writer, mut reader) = pcm_evidence_stream(
            PcmEvidenceConfig {
                capacity_frames: 4,
                detect_position_gaps: true,
            },
            48_000,
            2,
        )
        .unwrap();
        let pcm = [
            f32::from_bits(0x3f80_0001),
            f32::NEG_INFINITY,
            f32::from_bits(0x7fc0_1234),
            -0.0,
        ];
        let bytes: Vec<u8> = pcm.into_iter().flat_map(f32::to_le_bytes).collect();
        record_capture_evidence(
            Some(&mut writer),
            &bytes,
            2,
            TIMESTAMP_ERROR,
            0x1234_5678_9abc,
            0x8765_4321,
        );
        for expected in pcm.as_chunks::<2>().0 {
            let frame = reader.pop().unwrap();
            assert_eq!(frame.samples[0].to_bits(), expected[0].to_bits());
            assert_eq!(frame.samples[1].to_bits(), expected[1].to_bits());
            assert_eq!(frame.packet_sequence, 1);
            assert_eq!(frame.packet_frames, 2);
            assert_eq!(frame.flags, TIMESTAMP_ERROR);
            assert_eq!(frame.device_position_frames, 0x1234_5678_9abc);
            assert_eq!(frame.qpc_100ns, 0x8765_4321);
        }
        let summary = writer.snapshot();
        assert_eq!(summary.timestamp_error_packets, 1);
        assert_eq!(summary.stored_frames, 2);
    }

    #[test]
    fn endpoint_notifications_match_only_the_selected_identity() {
        let selected = EndpointNotifications {
            endpoint_id: "selected-endpoint".encode_utf16().chain([0]).collect(),
            invalidated: Arc::new(AtomicBool::new(false)),
        };
        let selected_id: Vec<_> = "selected-endpoint\0".encode_utf16().collect();
        let other_id: Vec<_> = "other-endpoint\0".encode_utf16().collect();
        selected.invalidate_if_matching(&PCWSTR(selected_id.as_ptr()));
        assert!(selected.invalidated.load(Ordering::Acquire));

        selected.invalidated.store(false, Ordering::Release);
        selected.invalidate_if_matching(&PCWSTR(other_id.as_ptr()));
        assert!(!selected.invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn shorter_endpoint_id_stops_comparison_at_its_terminator() {
        let selected = EndpointNotifications {
            endpoint_id: "selected-endpoint".encode_utf16().chain([0]).collect(),
            invalidated: Arc::new(AtomicBool::new(false)),
        };
        let shorter_id: Vec<_> = "selected\0".encode_utf16().collect();
        selected.invalidate_if_matching(&PCWSTR(shorter_id.as_ptr()));
        assert!(!selected.invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn null_endpoint_identity_is_ignored() {
        let selected = EndpointNotifications {
            endpoint_id: "selected-endpoint".encode_utf16().chain([0]).collect(),
            invalidated: Arc::new(AtomicBool::new(false)),
        };
        selected.invalidate_if_matching(&PCWSTR::null());
        assert!(!selected.invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn minimum_period_rounds_up_to_a_complete_frame() {
        assert_eq!(minimum_exclusive_frames(30_000, 96_000), Some(288));
        assert_eq!(minimum_exclusive_frames(1, 44_100), Some(1));
        assert_eq!(minimum_exclusive_frames(0, 48_000), None);
        assert_eq!(minimum_exclusive_frames(-1, 48_000), None);
    }

    #[test]
    fn next_exclusive_preset_is_the_smallest_supported_choice() {
        assert_eq!(next_exclusive_preset(64), Some(64));
        assert_eq!(next_exclusive_preset(288), Some(512));
        assert_eq!(next_exclusive_preset(4097), None);
    }

    #[test]
    fn frame_duration_is_reported_in_milliseconds() {
        assert!((frames_milliseconds(288, 96_000) - 3.0).abs() < f64::EPSILON);
    }
}
