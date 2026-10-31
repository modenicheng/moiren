//! Shared/Exclusive WASAPI streams. All COM services and packet leases remain
//! on the creating worker, including alignment retry and final Release.
use crate::bridge::unsupported;
use crate::capture_packet::borrow_capture_packet;
use crate::codec::SampleCodec;
use crate::native::{NativeStream, NativeStreamInfo, NativeStreamStats, StereoFrame};
use crate::negotiation::{
    EnginePeriods, SUBFORMAT_FLOAT, SUBFORMAT_PCM, StageError, client3_rejects_format,
    exclusive_format_supported, exclusive_frames_duration, friendly_name, initialize_exclusive,
    initialize_legacy_shared, parse_descriptor, shared_engine_periods,
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
    client: IAudioClient,
    event: OwnedHandle,
    info: NativeStreamInfo,
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
        if options.endpoint_id.trim().is_empty()
            || options.endpoint_id.contains('\0')
            || !(8_000..=384_000).contains(&options.sample_rate)
            || options
                .device
                .period_frames
                .is_some_and(|f| !(64..=4096).contains(&f) || !f.is_power_of_two())
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
        let client = if exclusive {
            negotiate_exclusive(&options, &device, client, &selected, &endpoint_name)?
        } else if let Some(period_frames) = options.device.period_frames {
            negotiate_shared_exact(&options, &client, &selected, &endpoint_name, period_frames)?;
            client
        } else {
            initialize_shared_legacy(&client, &format.Format, "Initialize(Shared)")?;
            client
        };
        api("SetEventHandle", unsafe { client.SetEventHandle(event.0) })?;
        let capacity = api("GetBufferSize", unsafe { client.GetBufferSize() })?;
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
        Ok(Self {
            capture,
            render,
            client,
            event,
            info: NativeStreamInfo {
                sample_rate: rate,
                format: *sample_format,
                buffer_frames: capacity,
                channels: usize::from(format.Format.nChannels),
            },
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
) -> Result<IAudioClient, BackendError> {
    let format = &selected.format;
    let sample_format = selected.sample_format;
    let rate = format.Format.nSamplesPerSec;
    let (mut default_period, mut minimum_period) = (0, 0);
    api("GetDevicePeriod", unsafe {
        client.GetDevicePeriod(Some(&mut default_period), Some(&mut minimum_period))
    })?;
    let duration = options
        .device
        .period_frames
        .map_or(default_period, |frames| {
            exclusive_frames_duration(frames as u32, rate)
        });
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
    // SAFETY: `format.Format` remains part of `selected` for this call.
    let initialized = unsafe { initialize_exclusive(&client, &format.Format, duration) };
    if initialized
        .as_ref()
        .is_err_and(|e| e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
    {
        let aligned = api("GetBufferSize(alignment)", unsafe {
            client.GetBufferSize()
        })?;
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
        api(
            "Initialize(Exclusive aligned)",
            // The shared negotiation helper owns the raw COM call; this map
            // keeps the professional BackendError stage and policy.
            unsafe {
                initialize_exclusive(
                    &client,
                    &format.Format,
                    exclusive_frames_duration(aligned, rate),
                )
            },
        )?;
    } else {
        api("Initialize(Exclusive)", initialized)?;
    }
    Ok(client)
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
        initialize_shared_legacy(client, &format.Format, "Initialize(Shared compatibility)")?;
        return Ok(());
    }
    let client3: Option<IAudioClient3> = match client.cast() {
        Ok(client3) => Some(client3),
        // The professional stack applies the kernel rejection
        // matrix to this interface query as well; the legacy
        // shared stack falls back on E_NOINTERFACE alone.
        Err(error) if client3_rejects_format(error.code()) => {
            log::warn!(
                target: "moiren::audio::wasapi",
                "WASAPI Client3 unavailable for exact Shared period; using compatibility initialization: endpoint={:?} hresult=0x{:08X} requested_frames={}",
                options.endpoint_id,
                error.code().0 as u32,
                period,
            );
            initialize_shared_legacy(client, &format.Format, "Initialize(Shared compatibility)")?;
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
            }
            Err(error) if client3_rejects_format(error.code()) => {
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
                )?;
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
    fn start(&mut self) -> Result<(), BackendError> {
        self.notifications.check()?;
        if !self.started {
            api("Start", unsafe { self.client.Start() })?;
            self.started = true;
        }
        Ok(())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
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
            let (mut data, mut frames, mut flags) = (ptr::null_mut(), 0, 0);
            let available = capture_packet_frames(
                self.mode,
                || unsafe { capture.GetNextPacketSize() },
                || unsafe {
                    capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
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
        self.notifications.check()?;
        let Some(render) = &self.render else {
            return Ok(0);
        };
        let count = if self.mode == DeviceMode::Exclusive {
            if !self.render_ready || input.len() < self.info.buffer_frames as usize {
                return Ok(0);
            }
            self.info.buffer_frames as usize
        } else {
            let padding = api("GetCurrentPadding", unsafe {
                self.client.GetCurrentPadding()
            })?;
            if padding > self.info.buffer_frames {
                return Err(unsupported("render padding exceeds native capacity"));
            }
            input
                .len()
                .min((self.info.buffer_frames - padding) as usize)
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
        self.stats
    }
    fn release(&mut self) -> Result<(), BackendError> {
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
) -> Result<(), BackendError> {
    // Keeps Windows' default persisted-session behavior (no NOPERSIST) and
    // always converts between the fixed client format and the endpoint mix.
    // SAFETY: format is the caller's live descriptor for this call.
    api(stage, unsafe {
        initialize_legacy_shared(client, std::ptr::from_ref(format), None)
    })
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
    use super::{
        DeviceMode, EndpointNotifications, capture_packet_frames, frames_milliseconds,
        minimum_exclusive_frames, next_exclusive_preset,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use windows::core::PCWSTR;

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
                || {
                    Err(windows::core::Error::from_hresult(
                        windows::Win32::Media::Audio::AUDCLNT_E_BUFFER_ERROR,
                    ))
                },
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
        use windows::Win32::Media::Audio::{
            AUDCLNT_E_BUFFER_ERROR, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_OUT_OF_ORDER,
        };

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
