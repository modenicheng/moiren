//! Shared/Exclusive WASAPI streams. All COM services and packet leases remain
//! on the creating worker, including alignment retry and final Release.
use super::{
    bridge::unsupported,
    codec::SampleCodec,
    native::{NativeStream, NativeStreamInfo, NativeStreamStats, StereoFrame},
};
use crate::negotiation::{
    EnginePeriodicityPolicy, EnginePeriods, LegacySharedInit, SUBFORMAT_FLOAT, SUBFORMAT_PCM,
    StageError, client3_rejects_format, friendly_name, initialize_legacy_shared, parse_descriptor,
    shared_engine_periods,
};
use crate::owner::{Apartment, LeaseFallback, OwnedHandle, PacketLease, TaskMemory};
use moiren_engine::{
    audio_config::{BUFFER_SIZES, SampleFormat},
    host::BackendError,
    node::{DefaultDeviceRole, DeviceMode, DeviceOptions, EndpointSelector},
};
use std::{ptr, time::Duration};
use windows::{
    Win32::{
        Foundation::{S_OK, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::*,
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance},
            Threading::WaitForSingleObject,
        },
    },
    core::{HSTRING, Interface},
};

#[derive(Debug, Clone)]
pub struct WasapiOptions {
    pub endpoint_id: String,
    pub sample_rate: u32,
    pub device: DeviceOptions,
    pub input: bool,
}

/// D4: professional's PERIODICITY_LOCKED policy — an explicit Shared period
/// is exact-or-fail, so the HRESULT surfaces instead of retrying with the
/// engine's current period. The legacy shared stack retries exactly once;
/// see `negotiation::EnginePeriodicityPolicy`. The assertion keeps this
/// stack pinned to the documented policy at compile time.
const PERIODICITY_POLICY: EnginePeriodicityPolicy = EnginePeriodicityPolicy::ExactPeriod;
const _: () = assert!(matches!(
    PERIODICITY_POLICY,
    EnginePeriodicityPolicy::ExactPeriod
));

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
    // Declared last so every COM interface releases before CoUninitialize.
    _apartment: Apartment,
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
        let event = api("CreateEvent", OwnedHandle::event())?;
        let mut client: IAudioClient = api("Activate(IAudioClient)", unsafe {
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
        let channels = if options.device.mode == DeviceMode::Shared {
            usize::from(base.nChannels)
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
        let exclusive = options.device.mode == DeviceMode::Exclusive;
        if exclusive && !supported(&client, &format.Format)? {
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
                if supported(&client, &candidate.Format)? {
                    selected = Some(candidate);
                    break;
                }
            }
            format = selected
                .ok_or_else(|| unsupported("no supported Exclusive format at requested rate"))?;
        }
        let (sample_format, codec) = codec_for(&format)?;
        let rate = format.Format.nSamplesPerSec;
        if exclusive {
            let (mut default_period, mut minimum_period) = (0, 0);
            api("GetDevicePeriod", unsafe {
                client.GetDevicePeriod(Some(&mut default_period), Some(&mut minimum_period))
            })?;
            let duration = options
                .device
                .period_frames
                .map_or(default_period, |frames| {
                    frames_duration(frames as u32, rate)
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
            let initialized = initialize_exclusive(&client, &format.Format, duration);
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
                    initialize_exclusive(&client, &format.Format, frames_duration(aligned, rate)),
                )?;
            } else {
                api("Initialize(Exclusive)", initialized)?;
            }
        } else if let Some(period) = options.device.period_frames {
            // Explicit requests are exact, not rounded to a convenient driver
            // period. Legacy conversion cannot promise an exact event period.
            let period = period as u32;
            if shared_conversion {
                log::warn!(
                    target: "moiren::audio::wasapi",
                    "WASAPI Shared explicit period uses compatibility initialization because the requested client format differs from the endpoint mix: endpoint={:?} requested_rate={} requested_format={:?} requested_frames={}",
                    options.endpoint_id,
                    rate,
                    sample_format,
                    period,
                );
                initialize_shared_legacy(
                    &client,
                    &format.Format,
                    "Initialize(Shared compatibility)",
                )?;
            } else {
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
                        initialize_shared_legacy(
                            &client,
                            &format.Format,
                            "Initialize(Shared compatibility)",
                        )?;
                        None
                    }
                    Err(error) => return api("IAudioClient3(exact period)", Err(error)),
                };
                if let Some(client3) = client3 {
                    // SAFETY: format is the locally owned selected descriptor,
                    // valid for this query.
                    let capabilities = unsafe {
                        shared_engine_periods(&client3, std::ptr::from_ref(&format.Format))
                    };
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
                                    "requested Shared period {period} is outside {minimum}..{maximum} or not a multiple of {fundamental}"
                                )));
                            }
                            // D4: an explicit Shared period is a hard contract
                            // here (out-of-range periods are rejected above,
                            // never rounded), so PERIODICITY_LOCKED surfaces
                            // instead of retrying with the engine period
                            // (PERIODICITY_POLICY; the legacy shared stack
                            // retries exactly once).
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
                                &client,
                                &format.Format,
                                "Initialize(Shared compatibility)",
                            )?;
                        }
                        Err(error) => {
                            return api("GetSharedModeEnginePeriod", Err(error));
                        }
                    }
                }
            }
        } else {
            initialize_shared_legacy(&client, &format.Format, "Initialize(Shared)")?;
        }
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
                format: sample_format,
                buffer_frames: capacity,
                channels: usize::from(format.Format.nChannels),
            },
            codec,
            mode: options.device.mode,
            started: false,
            render_ready: true,
            captured: vec![[0.0; 2]; capacity as usize].into_boxed_slice(),
            captured_offset: 0,
            captured_frames: 0,
            stats: NativeStreamStats::default(),
            _apartment: apartment,
        })
    }
}

impl NativeStream for WasapiStream {
    fn info(&self) -> NativeStreamInfo {
        self.info
    }
    fn start(&mut self) -> Result<(), BackendError> {
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
        match unsafe {
            WaitForSingleObject(
                self.event.0,
                timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32,
            )
        } {
            WAIT_OBJECT_0 => {
                self.render_ready = true;
                self.stats.wait_wakes += 1;
                Ok(true)
            }
            WAIT_TIMEOUT => {
                self.stats.wait_timeouts += 1;
                Ok(false)
            }
            _ => api(
                "WaitForSingleObject",
                Err(windows::core::Error::from_thread()),
            ),
        }
    }
    fn read(&mut self, output: &mut [StereoFrame]) -> Result<usize, BackendError> {
        let Some(capture) = &self.capture else {
            return Ok(0);
        };
        if output.is_empty() {
            return Ok(0);
        }
        if self.captured_frames == self.captured_offset {
            if api("GetNextPacketSize", unsafe { capture.GetNextPacketSize() })? == 0 {
                return Ok(0);
            }
            let (mut data, mut frames, mut flags) = (ptr::null_mut(), 0, 0);
            api("GetBuffer(capture)", unsafe {
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)
            })?;
            let lease = PacketLease::new(capture, frames, LeaseFallback::CapturePacket);
            if frames > self.info.buffer_frames {
                return Err(unsupported("capture packet exceeds negotiated buffer"));
            }
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            if silent {
                self.captured[..frames as usize].fill([0.0; 2]);
            } else if frames > 0 {
                if data.is_null() {
                    return Err(unsupported("capture packet has a null data pointer"));
                }
                // The packet lease supplies exactly frames of the negotiated
                // descriptor. Copy before ReleaseBuffer, retaining partial reads.
                let bytes = unsafe {
                    std::slice::from_raw_parts(data, frames as usize * self.codec.frame_bytes())
                };
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
}
impl Drop for WasapiStream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn frames_duration(frames: u32, rate: u32) -> i64 {
    ((u64::from(frames) * 10_000_000 + u64::from(rate) / 2) / u64::from(rate)) as i64
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

fn initialize_exclusive(
    client: &IAudioClient,
    format: &WAVEFORMATEX,
    duration: i64,
) -> windows::core::Result<()> {
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            duration,
            duration,
            format,
            None,
        )
    }
}

fn initialize_shared_legacy(
    client: &IAudioClient,
    format: &WAVEFORMATEX,
    stage: &'static str,
) -> Result<(), BackendError> {
    // D4: professional keeps Windows' default persisted-session behavior
    // (no NOPERSIST) and always converts between its fixed client format and
    // the endpoint mix; the legacy shared stack passes its own combination.
    // SAFETY: format is the caller's live descriptor for this call.
    api(stage, unsafe {
        initialize_legacy_shared(
            client,
            std::ptr::from_ref(format),
            None,
            LegacySharedInit {
                auto_convert: true,
                nopersist: false,
            },
        )
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
    let hr = unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, format, None) };
    if hr == AUDCLNT_E_UNSUPPORTED_FORMAT {
        return Ok(false);
    }
    api("IsFormatSupported(Exclusive)", hr.ok())?;
    Ok(hr == S_OK)
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
    use super::{frames_milliseconds, minimum_exclusive_frames, next_exclusive_preset};

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
