//! Thread-owned WASAPI objects. Only the stop kernel handle is shared.
use super::*;
use crate::{
    catalog,
    owner::{Apartment, Mmcss, OwnedHandle, Streaming, TaskMemory},
};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle as StdHandle},
    sync::Arc,
    thread::{self, JoinHandle},
    time::Instant,
};
use windows::{
    Win32::{
        Foundation::{E_POINTER, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::*,
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
            Threading::{
                AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent, WaitForMultipleObjects,
            },
        },
    },
    core::{GUID, HSTRING, Interface},
};

fn api<T>(stage: &'static str, result: windows::core::Result<T>) -> Result<T, RenderError> {
    result.map_err(|error| RenderError::Api {
        stage,
        hresult: error.code().0,
    })
}
fn handle(value: &StdHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}
fn stop_event() -> Result<StdHandle, RenderError> {
    let raw = api("CreateEvent(stop)", unsafe {
        CreateEventW(None, true, false, None)
    })?;
    // SAFETY: CreateEvent returned a uniquely owned valid kernel handle.
    Ok(unsafe { StdHandle::from_raw_handle(raw.0) })
}

pub struct RenderSession {
    stop: Arc<StdHandle>,
    worker: Option<JoinHandle<RenderReport>>,
}
impl RenderSession {
    pub fn request_stop(&self) -> Result<(), RenderError> {
        api("SetEvent(stop)", unsafe { SetEvent(handle(&self.stop)) })
    }
    /// Waits for normal duration completion or a requested stop. The returned
    /// report is created after all stream/COM objects have been released.
    pub fn join(mut self) -> Result<RenderReport, RenderError> {
        self.worker
            .take()
            .expect("owned worker")
            .join()
            .map_err(|_| RenderError::WorkerPanicked)
    }
}
impl Drop for RenderSession {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = self.request_stop();
            let _ = worker.join();
        }
    }
}
pub fn start_render(
    options: RenderOptions,
    renderer: DemandRenderer,
) -> Result<RenderSession, RenderError> {
    options.validate()?;
    let stop = Arc::new(stop_event()?);
    let worker_stop = Arc::clone(&stop);
    let worker = thread::Builder::new()
        .name("moiren-shared-render".into())
        .spawn(move || run_owner(options, renderer, worker_stop))
        .map_err(|error| RenderError::WorkerSpawn {
            code: error.raw_os_error(),
        })?;
    Ok(RenderSession {
        stop,
        worker: Some(worker),
    })
}

#[derive(Debug, Serialize)]
pub struct RenderEndpoint {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub format_supported: Option<bool>,
    pub errors: Vec<String>,
}
/// Read-only catalog; does not choose a default endpoint or modify sessions.
pub fn list_render_endpoints() -> Result<Vec<RenderEndpoint>, RenderError> {
    let _apartment = api("CoInitializeEx(catalog)", Apartment::new())?;
    let snapshot = api("render endpoint catalog", catalog::render_snapshot())?;
    Ok(project_render_endpoints(snapshot))
}
fn project_render_endpoints(snapshot: catalog::CatalogSnapshot) -> Vec<RenderEndpoint> {
    let mut endpoints: Vec<_> = snapshot
        .endpoints
        .into_iter()
        .filter(|e| e.flow == "render")
        .map(|e| {
            let supported = e.mix_format.as_ref().map(|f| {
                let float = f.format_tag == 3
                    || f.sub_format.as_ref().is_some_and(|g| {
                        g.eq_ignore_ascii_case("00000003-0000-0010-8000-00aa00389b71")
                    });
                float
                    && f.sample_rate == SAMPLE_RATE
                    && f.channels == CHANNELS as u16
                    && f.container_bits == 32
                    && f.block_align == 8
                    && f.valid_bits.is_none_or(|bits| bits == 32)
                    && f.channel_mask.is_none_or(|mask| mask == 0 || mask == 3)
            });
            RenderEndpoint {
                endpoint_id: e.id,
                name: e.name,
                sample_rate: e.mix_format.as_ref().map(|f| f.sample_rate),
                channels: e.mix_format.as_ref().map(|f| f.channels),
                format_supported: supported,
                errors: e
                    .errors
                    .into_iter()
                    .map(|error| format!("{}: {}", error.stage, error.hresult))
                    .collect(),
            }
        })
        .collect();
    // A device that failed before its ID was read cannot be selected, but its
    // original failure must remain visible alongside usable endpoints.
    for error in snapshot.errors {
        endpoints.push(RenderEndpoint {
            endpoint_id: String::new(),
            name: None,
            sample_rate: None,
            channels: None,
            format_supported: None,
            errors: vec![format!("{}: {}", error.stage, error.hresult)],
        });
    }
    endpoints
}

fn validate_format(
    base: WAVEFORMATEX,
    float: bool,
    valid_bits: Option<u16>,
    mask: Option<u32>,
) -> Result<(), RenderError> {
    if !float
        || base.nSamplesPerSec != SAMPLE_RATE
        || base.nChannels != CHANNELS as u16
        || base.wBitsPerSample != 32
        || base.nBlockAlign != 8
        || base.nAvgBytesPerSec != SAMPLE_RATE * 8
        || valid_bits.is_some_and(|bits| bits != 32)
        || mask.is_some_and(|mask| mask != 0 && mask != 3)
    {
        return Err(RenderError::UnsupportedFormat);
    }
    Ok(())
}

struct RenderLease<'a> {
    client: &'a IAudioRenderClient,
    active: bool,
}
impl RenderLease<'_> {
    fn release(mut self, frames: u32) -> Result<(), RenderError> {
        self.active = false;
        api("ReleaseBuffer(render)", unsafe {
            self.client.ReleaseBuffer(frames, 0)
        })
    }
}
impl Drop for RenderLease<'_> {
    fn drop(&mut self) {
        if self.active {
            // Shared mode cancellation: no part of an abandoned packet is used.
            let _ = unsafe { self.client.ReleaseBuffer(0, 0) };
        }
    }
}
fn submit(client: &IAudioRenderClient, samples: &[f32]) -> Result<(), RenderError> {
    if samples.is_empty() {
        return Ok(());
    }
    if !samples.len().is_multiple_of(CHANNELS) {
        return Err(RenderError::InvalidSamples);
    }
    let frames =
        u32::try_from(samples.len() / CHANNELS).map_err(|_| RenderError::InvalidSamples)?;
    let data = api("GetBuffer(render)", unsafe { client.GetBuffer(frames) })?;
    let lease = RenderLease {
        client,
        active: true,
    };
    if data.is_null() {
        return Err(RenderError::Api {
            stage: "null render buffer",
            hresult: E_POINTER.0,
        });
    }
    // SAFETY: A validated native f32 stereo WASAPI lease provides frames * 8
    // writable bytes on this owner thread. Source is separately owned staging;
    // neither pointer escapes and the full packet is released below.
    unsafe {
        std::ptr::copy_nonoverlapping(
            samples.as_ptr().cast::<u8>(),
            data,
            std::mem::size_of_val(samples),
        );
    }
    lease.release(frames)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    Stop,
    Audio,
    Timeout,
}
fn wait(stop: HANDLE, audio: HANDLE, timeout_ms: u32) -> Result<Wake, RenderError> {
    match unsafe { WaitForMultipleObjects(&[stop, audio], false, timeout_ms) } {
        event if event == WAIT_OBJECT_0 => Ok(Wake::Stop),
        event if event.0 == WAIT_OBJECT_0.0 + 1 => Ok(Wake::Audio),
        WAIT_TIMEOUT => Ok(Wake::Timeout),
        _ => Err(RenderError::Api {
            stage: "WaitForMultipleObjects",
            hresult: windows::core::Error::from_thread().code().0,
        }),
    }
}

fn produce(
    renderer: &mut DemandRenderer,
    samples: &mut [f32],
    stats: &mut RenderStats,
) -> Result<(), RenderError> {
    let before = renderer.counters();
    let result = renderer.render_interleaved(samples);
    let after = renderer.counters();
    stats.dsp_blocks += after.blocks - before.blocks;
    stats.dsp_segments += after.segments - before.segments;
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            if let RenderError::IncompleteTransfer { expected, actual } = error {
                stats.bridge_shortfall_frames += (expected - actual) as u64;
            }
            Err(error)
        }
    }
}
fn record_submit(samples: &[f32], stats: &mut RenderStats) {
    stats.submitted_frames += (samples.len() / CHANNELS) as u64;
    for &sample in samples {
        stats.nonzero_samples += u64::from(sample != 0.0);
        stats.peak_amplitude = stats.peak_amplitude.max(sample.abs());
    }
}

fn owner(
    options: &RenderOptions,
    renderer: &mut DemandRenderer,
    stop: &StdHandle,
    report: &mut RenderReport,
) -> Result<RenderStatus, RenderError> {
    // Declaration order keeps all COM services/client and event handles alive
    // through Stop, then releases them before CoUninitialize on this thread.
    let _apartment = api("CoInitializeEx(render)", Apartment::new())?;
    let enumerator: IMMDeviceEnumerator = api("CoCreateInstance(MMDeviceEnumerator)", unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    })?;
    let device = api("GetDevice(pinned)", unsafe {
        enumerator.GetDevice(&HSTRING::from(&options.endpoint_id))
    })?;
    let endpoint: IMMEndpoint = api("IMMEndpoint", device.cast())?;
    if api("GetDataFlow", unsafe { endpoint.GetDataFlow() })? != eRender {
        return Err(RenderError::InvalidEndpoint);
    }
    let event = api("CreateEvent(audio)", OwnedHandle::event())?;
    let client: IAudioClient = api("Activate(IAudioClient)", unsafe {
        device.Activate(CLSCTX_ALL, None)
    })?;
    let client2: IAudioClient2 = api("IAudioClient2", client.cast())?;
    api("SetClientProperties", unsafe {
        client2.SetClientProperties(&AudioClientProperties {
            cbSize: size_of::<AudioClientProperties>() as u32,
            eCategory: AudioCategory_Other,
            ..Default::default()
        })
    })?;
    let memory = TaskMemory(api("GetMixFormat", unsafe { client.GetMixFormat() })?);
    if memory.0.is_null() {
        return Err(RenderError::UnsupportedFormat);
    }
    // SAFETY: GetMixFormat returns an owned WAVEFORMATEX allocation, with cbSize
    // describing the trailing extension; TaskMemory releases it after use.
    let base = unsafe { memory.0.read_unaligned() };
    report.native_mix_sample_rate = Some(base.nSamplesPerSec);
    report.native_mix_channels = Some(base.nChannels);
    report.native_mix_container_bits = Some(base.wBitsPerSample);
    let (float, valid_bits, mask) = if base.wFormatTag == 0xfffe && base.cbSize >= 22 {
        let extended = unsafe { memory.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
        let sub_format = extended.SubFormat;
        (
            sub_format == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71),
            Some(unsafe { extended.Samples.wValidBitsPerSample }),
            Some(extended.dwChannelMask),
        )
    } else {
        (base.wFormatTag == 3, None, None)
    };
    validate_format(base, float, valid_bits, mask)?;
    report.format_supported = true;
    let session_guid = api("CoCreateGuid(session)", unsafe { CoCreateGuid() })?;
    api("Initialize(Shared)", unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_NOPERSIST,
            0,
            0,
            memory.0,
            Some(&session_guid),
        )
    })?;
    api("SetEventHandle", unsafe { client.SetEventHandle(event.0) })?;
    let capacity = api("GetBufferSize", unsafe { client.GetBufferSize() })?;
    writable_frames(capacity, 0)?;
    report.buffer_frames = Some(capacity);
    report.stream_latency_100ns = Some(api("GetStreamLatency", unsafe {
        client.GetStreamLatency()
    })?);
    match client.cast::<IAudioClient3>() {
        Ok(client3) => {
            let mut ptr = std::ptr::null_mut();
            let mut period = 0;
            let period_result =
                unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut ptr, &mut period) };
            let _format = TaskMemory(ptr);
            if period_result.is_ok() {
                report.engine_period_frames = Some(period);
            } else if let Err(error) = period_result {
                report.period_hresult = Some(error.code().0);
            }
        }
        Err(error) => report.period_hresult = Some(error.code().0),
    }
    let ducking_result = unsafe { client.GetService::<IAudioSessionControl>() }
        .and_then(|s| s.cast::<IAudioSessionControl2>())
        .and_then(|session| unsafe { session.SetDuckingPreference(true) });
    match ducking_result {
        Ok(()) => report.own_session_ducking_opt_out = true,
        Err(error) => report.ducking_hresult = Some(error.code().0),
    }
    let render: IAudioRenderClient = api("GetService(IAudioRenderClient)", unsafe {
        client.GetService()
    })?;
    // Native capacity is driver supplied: cap staging to the explicit 8 MiB
    // preparation budget before allocating; failure never enters streaming.
    let sample_count = (capacity as usize)
        .checked_mul(CHANNELS)
        .filter(|&n| n <= 8 * 1024 * 1024 / size_of::<f32>())
        .ok_or(RenderError::StagingBudget)?;
    let mut staging = vec![0.0f32; sample_count];
    if wait(handle(stop), event.0, 0)? == Wake::Stop {
        return Ok(RenderStatus::Stopped);
    }
    produce(renderer, &mut staging, &mut report.stats)?;
    submit(&render, &staging)?;
    report.stats.primed_frames = capacity;
    record_submit(&staging, &mut report.stats);
    let mut task_index = 0;
    let _mmcss =
        match unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut task_index) }
        {
            Ok(value) => {
                report.mmcss_registered = true;
                Some(Mmcss(value))
            }
            Err(error) => {
                report.mmcss_hresult = Some(error.code().0);
                None
            }
        };
    let mut streaming = api("Start", Streaming::start(&client))?;
    report.stream_started = true;
    let started = Instant::now();
    let result = (|| {
        while started.elapsed() < options.duration {
            let remaining = options.duration.saturating_sub(started.elapsed());
            let timeout_ms = remaining.as_millis().clamp(1, 100) as u32;
            match wait(handle(stop), event.0, timeout_ms)? {
                Wake::Stop => return Ok(RenderStatus::Stopped),
                Wake::Timeout => {
                    report.stats.timeout_wakes += 1;
                    continue;
                }
                Wake::Audio => report.stats.audio_wakes += 1,
            }
            let padding = api("GetCurrentPadding", unsafe { client.GetCurrentPadding() })?;
            let demand = writable_frames(capacity, padding)?;
            if demand == 0 {
                report.stats.zero_demand_wakes += 1;
                continue;
            }
            if padding == 0 {
                report.stats.empty_padding_wakes += 1;
            }
            report.stats.min_writable_frames = Some(
                report
                    .stats
                    .min_writable_frames
                    .map_or(demand, |n| n.min(demand)),
            );
            report.stats.max_writable_frames = report.stats.max_writable_frames.max(demand);
            let samples = &mut staging[..demand as usize * CHANNELS];
            produce(renderer, samples, &mut report.stats)?;
            submit(&render, samples)?;
            record_submit(samples, &mut report.stats);
        }
        Ok(RenderStatus::Completed)
    })();
    report.elapsed_seconds = started.elapsed().as_secs_f64();
    let stopped = streaming.stop();
    report.stop_succeeded = stopped.is_ok();
    if let Err(error) = &stopped {
        report.stop_hresult = Some(error.code().0);
    }
    // Preserve a streaming failure and separately record any Stop failure.
    match result {
        Err(error) => Err(error),
        Ok(status) => {
            api("Stop", stopped)?;
            Ok(status)
        }
    }
}

fn run_owner(
    options: RenderOptions,
    mut renderer: DemandRenderer,
    stop: Arc<StdHandle>,
) -> RenderReport {
    let initial_frame = renderer.timeline();
    let mut report = RenderReport {
        schema_version: 1,
        endpoint_id: options.endpoint_id.clone(),
        status: RenderStatus::Failed,
        requested_seconds: options.duration.as_secs_f64(),
        elapsed_seconds: 0.0,
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        native_mix_sample_rate: None,
        native_mix_channels: None,
        native_mix_container_bits: None,
        format_supported: false,
        buffer_frames: None,
        engine_period_frames: None,
        period_hresult: None,
        stream_latency_100ns: None,
        mmcss_registered: false,
        mmcss_hresult: None,
        own_session_ducking_opt_out: false,
        ducking_hresult: None,
        stop_succeeded: false,
        stream_started: false,
        stop_hresult: None,
        stats: RenderStats::default(),
        failure: None,
    };
    // owner() has stopped and released services/client/apartment before any
    // formatting. Engine/ring ownership is dropped here outside streaming.
    match owner(&options, &mut renderer, &stop, &mut report) {
        Ok(status) => report.status = status,
        Err(error) => report.failure = Some(error.to_string()),
    }
    report.stats.processed_frames = renderer.timeline().saturating_sub(initial_frame);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{RefCell, UnsafeCell},
        rc::Rc,
    };
    use windows::core::{Result as WinResult, implement};

    #[test]
    fn usable_render_choices_survive_missing_capture_and_local_errors() {
        let mut mix_format = catalog::FormatSnapshot::from_base(WAVEFORMATEX {
            wFormatTag: 3,
            nChannels: 2,
            nSamplesPerSec: 48000,
            nAvgBytesPerSec: 384000,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        });
        mix_format.channel_mask = Some(3);
        let endpoints = project_render_endpoints(catalog::CatalogSnapshot {
            defaults: vec![catalog::DefaultEndpoint {
                flow: "capture",
                role: "console",
                id: None,
            }],
            endpoints: vec![catalog::EndpointSnapshot {
                id: "speakers".into(),
                name: Some("Usable speakers".into()),
                flow: "render",
                mix_format: Some(mix_format),
                default_period_100ns: None,
                minimum_period_100ns: None,
                volume_scalar: None,
                muted: None,
                sessions: Vec::new(),
                errors: vec![catalog::ApiFailure {
                    stage: "session enumeration".into(),
                    hresult: "0x80070005".into(),
                }],
            }],
            errors: vec![catalog::ApiFailure {
                stage: "missing capture default".into(),
                hresult: "0x80070490".into(),
            }],
        });
        assert_eq!(endpoints[0].endpoint_id, "speakers");
        assert_eq!(endpoints[0].format_supported, Some(true));
        assert!(endpoints[0].errors[0].contains("0x80070005"));
        assert_eq!(endpoints[1].endpoint_id, "");
        assert!(endpoints[1].errors[0].contains("0x80070490"));
    }

    #[test]
    fn stop_wakes_without_any_audio_event_and_takes_priority() {
        let stop = stop_event().unwrap();
        let audio = OwnedHandle::event().unwrap();
        assert_eq!(wait(handle(&stop), audio.0, 0).unwrap(), Wake::Timeout);
        unsafe {
            SetEvent(handle(&stop)).unwrap();
        }
        assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
        unsafe {
            SetEvent(audio.0).unwrap();
        }
        assert_eq!(wait(handle(&stop), audio.0, 100).unwrap(), Wake::Stop);
    }
    #[test]
    fn native_format_validation_rejects_conversion_and_bad_layouts() {
        let base = WAVEFORMATEX {
            wFormatTag: 3,
            nChannels: 2,
            nSamplesPerSec: 48000,
            nAvgBytesPerSec: 384000,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        assert!(validate_format(base, true, None, None).is_ok());
        assert!(validate_format(base, true, Some(32), Some(3)).is_ok());
        for bad in [
            WAVEFORMATEX {
                nSamplesPerSec: 44100,
                ..base
            },
            WAVEFORMATEX {
                nChannels: 1,
                ..base
            },
            WAVEFORMATEX {
                nBlockAlign: 4,
                ..base
            },
            WAVEFORMATEX {
                wBitsPerSample: 24,
                ..base
            },
            WAVEFORMATEX {
                nAvgBytesPerSec: 192000,
                ..base
            },
        ] {
            assert_eq!(
                validate_format(bad, true, None, None),
                Err(RenderError::UnsupportedFormat)
            );
        }
        assert_eq!(
            validate_format(base, false, None, None),
            Err(RenderError::UnsupportedFormat)
        );
        assert_eq!(
            validate_format(base, true, Some(24), None),
            Err(RenderError::UnsupportedFormat)
        );
        assert_eq!(
            validate_format(base, true, Some(32), Some(12)),
            Err(RenderError::UnsupportedFormat)
        );
    }
    #[implement(IAudioRenderClient)]
    struct FakeRender {
        data: Rc<UnsafeCell<[f32; 4]>>,
        calls: Rc<RefCell<Vec<(u32, u32)>>>,
        null_buffer: bool,
    }
    impl IAudioRenderClient_Impl for FakeRender_Impl {
        fn GetBuffer(&self, frames: u32) -> WinResult<*mut u8> {
            assert_eq!(frames, 2);
            self.calls.borrow_mut().push((frames, u32::MAX));
            Ok(if self.null_buffer {
                std::ptr::null_mut()
            } else {
                self.data.get().cast()
            })
        }
        fn ReleaseBuffer(&self, frames: u32, flags: u32) -> WinResult<()> {
            self.calls.borrow_mut().push((frames, flags));
            Ok(())
        }
    }
    #[test]
    fn render_leases_copy_full_pcm_or_cancel_without_submitting_stale_data() {
        for null_buffer in [false, true] {
            let data = Rc::new(UnsafeCell::new([99.0; 4]));
            let calls = Rc::new(RefCell::new(Vec::new()));
            let client: IAudioRenderClient = FakeRender {
                data: Rc::clone(&data),
                calls: Rc::clone(&calls),
                null_buffer,
            }
            .into();
            assert!(submit(&client, &[]).is_ok());
            assert!(calls.borrow().is_empty());
            let result = submit(&client, &[0.1, -0.1, 0.2, -0.2]);
            assert_eq!(result.is_err(), null_buffer);
            assert_eq!(
                *calls.borrow(),
                [(2, u32::MAX), (if null_buffer { 0 } else { 2 }, 0)]
            );
            // SAFETY: Single-threaded test reads after the buffer lease ended.
            assert_eq!(
                unsafe { *data.get() },
                if null_buffer {
                    [99.0; 4]
                } else {
                    [0.1, -0.1, 0.2, -0.2]
                }
            );
        }
    }
}
