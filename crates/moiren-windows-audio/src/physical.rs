//! Explicit endpoint probes: native f32 capture and Shared silent render.
use crate::{
    catalog::{self, ApiFailure, CatalogSnapshot, FormatSnapshot},
    clock::{
        ClockPoint, ClockSummary, DeviceClockSummary, analyze_clock, analyze_device_clock,
        relative_ppm,
    },
    owner::{Apartment, Mmcss, OwnedHandle, PacketLease, Streaming, TaskMemory},
    probe::PacketRecord,
    stats::{CaptureStats, CaptureSummary, SILENT, TIMESTAMP_ERROR, analyze_f32},
};
use serde::Serialize;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::{
    Win32::{
        Foundation::{E_UNEXPECTED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::*,
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
            Threading::{AvSetMmThreadCharacteristicsW, WaitForSingleObject},
        },
    },
    core::{GUID, HSTRING, Interface, Result},
};

#[derive(Serialize)]
pub struct DemandRecord {
    pub arrival_ms: f64,
    pub padding_frames: u32,
    pub writable_frames: u32,
}

#[derive(Default, Serialize)]
pub struct RenderSummary {
    pub primed_frames: u32,
    pub submitted_frames: u64,
    pub zero_demand_wakes: u64,
    /// An empty buffer observation is a diagnostic, not proof of an underrun.
    pub empty_padding_wakes: u64,
    pub min_writable_frames: Option<u32>,
    pub max_writable_frames: u32,
}

#[derive(Serialize)]
pub struct EndpointReport {
    pub endpoint_id: String,
    pub name: Option<String>,
    pub flow: &'static str,
    pub status: &'static str,
    pub last_stage: &'static str,
    pub elapsed_seconds: f64,
    pub format: Option<FormatSnapshot>,
    pub buffer_frames: Option<u32>,
    pub stream_latency_100ns: Option<i64>,
    pub engine_period_frames: Option<u32>,
    pub category: &'static str,
    pub own_session_ducking_opt_out: bool,
    pub mmcss_registered: bool,
    pub stop_succeeded: bool,
    pub audio_wakes: u64,
    pub timeout_wakes: u64,
    pub frequency_units_per_second: Option<u64>,
    pub clock: ClockSummary,
    /// Capture frame/QPC pairs, normalized by stream sample rate, not GetFrequency.
    pub capture_packet_clock: Option<ClockSummary>,
    pub comparison_clock_source: &'static str,
    pub clock_points: Vec<ClockPoint>,
    /// Optional IAudioClock2 raw positions are device frames, not client bytes.
    pub device_clock: Option<DeviceClockSummary>,
    pub device_clock_points: Vec<ClockPoint>,
    pub capture: Option<CaptureSummary>,
    pub capture_packets: Vec<PacketRecord>,
    pub render: Option<RenderSummary>,
    pub render_demands: Vec<DemandRecord>,
    pub metadata_dropped: u64,
    pub errors: Vec<ApiFailure>,
}

#[derive(Serialize)]
pub struct RelativeClock {
    pub left_endpoint: String,
    pub right_endpoint: String,
    pub left_faster_ppm: Option<f64>,
}

#[derive(Serialize)]
pub struct PhysicalReport {
    pub schema_version: u32,
    pub started_unix_ms: u128,
    pub requested_seconds: u32,
    pub before: CatalogSnapshot,
    pub endpoints: Vec<EndpointReport>,
    pub common_qpc_window_100ns: Option<(u64, u64)>,
    pub relative_clocks: Vec<RelativeClock>,
    pub after: Option<CatalogSnapshot>,
    pub observed_state_changes: Vec<String>,
    pub errors: Vec<ApiFailure>,
}

fn unexpected() -> windows::core::Error {
    windows::core::Error::from_hresult(E_UNEXPECTED)
}

fn prepare_and_sample(report: &mut EndpointReport, seconds: u32) -> Result<()> {
    let _apartment = Apartment::new()?;
    report.last_stage = "GetDevice / flow";
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(&report.endpoint_id))? };
    let endpoint: IMMEndpoint = device.cast()?;
    let flow = unsafe { endpoint.GetDataFlow()? };
    report.flow = if flow == eRender {
        "render"
    } else if flow == eCapture {
        "capture"
    } else {
        return Err(unexpected());
    };
    let event = OwnedHandle::event()?;
    report.last_stage = "Activate(IAudioClient) / category";
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let client2: IAudioClient2 = client.cast()?;
    unsafe {
        client2.SetClientProperties(&AudioClientProperties {
            cbSize: size_of::<AudioClientProperties>() as u32,
            eCategory: AudioCategory_Other,
            ..Default::default()
        })?;
    }
    report.last_stage = "GetMixFormat / native f32 validation";
    let memory = TaskMemory(unsafe { client.GetMixFormat()? });
    if memory.0.is_null() {
        return Err(unexpected());
    }
    let base = unsafe { memory.0.read_unaligned() };
    let mut descriptor = FormatSnapshot::from_base(base);
    let mut float = base.wFormatTag == 3;
    if base.wFormatTag == 0xfffe && base.cbSize >= 22 {
        let extended = unsafe { memory.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
        let guid = extended.SubFormat;
        descriptor.valid_bits = Some(unsafe { extended.Samples.wValidBitsPerSample });
        descriptor.channel_mask = Some(extended.dwChannelMask);
        descriptor.sub_format = Some(format!("{guid:?}"));
        float = guid == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
    }
    report.format = Some(descriptor);
    if !float
        || base.wBitsPerSample != 32
        || base.nChannels == 0
        || base.nSamplesPerSec == 0
        || base.nBlockAlign != base.nChannels.checked_mul(4).ok_or_else(unexpected)?
    {
        // This W00 slice deliberately supports the formats needed on this machine.
        return Err(windows::core::Error::from_hresult(
            AUDCLNT_E_UNSUPPORTED_FORMAT,
        ));
    }
    report.last_stage = "Initialize / SetEventHandle";
    let session_id = unsafe { CoCreateGuid()? };
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_NOPERSIST,
            0,
            0,
            memory.0,
            Some(&session_id),
        )?;
        client.SetEventHandle(event.0)?;
    }
    // Attenuation preference applies to our render session. Capture sessions on
    // the tested drivers reject this method with AUDCLNT_E_WRONG_ENDPOINT_TYPE.
    // Never call it on QQMusic's session; category Other is set for both flows.
    if flow == eRender {
        report.last_stage = "GetService(IAudioSessionControl)";
        let session: IAudioSessionControl2 =
            unsafe { client.GetService::<IAudioSessionControl>()? }.cast()?;
        report.last_stage = "SetDuckingPreference(own render session)";
        unsafe {
            session.SetDuckingPreference(true)?;
        }
        report.own_session_ducking_opt_out = true;
    }
    report.last_stage = "GetBufferSize / latency / engine period";
    let capacity = unsafe { client.GetBufferSize()? };
    report.buffer_frames = Some(capacity);
    report.stream_latency_100ns = Some(unsafe { client.GetStreamLatency()? });
    match client.cast::<IAudioClient3>() {
        Ok(client3) => {
            let mut format_ptr = std::ptr::null_mut();
            let mut period = 0;
            match unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut format_ptr, &mut period) }
            {
                Ok(()) => {
                    let _format = TaskMemory(format_ptr);
                    report.engine_period_frames = Some(period);
                }
                Err(error) => report.errors.push(ApiFailure::new(
                    "GetCurrentSharedModeEnginePeriod (optional)",
                    error,
                )),
            }
        }
        Err(error) => report
            .errors
            .push(ApiFailure::new("IAudioClient3 (optional)", error)),
    }
    report.last_stage = "GetService(IAudioClock) / GetFrequency";
    let clock: IAudioClock = unsafe { client.GetService()? };
    let frequency = unsafe { clock.GetFrequency()? };
    if frequency == 0 {
        return Err(unexpected());
    }
    report.frequency_units_per_second = Some(frequency);
    let device_clock = match clock.cast::<IAudioClock2>() {
        Ok(clock2) => Some(clock2),
        Err(error) => {
            report
                .errors
                .push(ApiFailure::new("IAudioClock2 (optional)", error));
            None
        }
    };
    let capture: Option<IAudioCaptureClient> = if flow == eCapture {
        Some(unsafe { client.GetService()? })
    } else {
        None
    };
    let render: Option<IAudioRenderClient> = if flow == eRender {
        Some(unsafe { client.GetService()? })
    } else {
        None
    };
    let mut stats = CaptureStats::default();
    let bound = seconds as usize * 1000;
    report.clock_points = Vec::with_capacity(bound);
    if device_clock.is_some() {
        report.device_clock_points = Vec::with_capacity(bound);
    }
    if capture.is_some() {
        report.capture_packets = Vec::with_capacity(bound);
    }
    if let Some(render) = &render {
        report.render_demands = Vec::with_capacity(bound);
        report.last_stage = "render priming";
        write_silence(render, capacity)?;
        report.render = Some(RenderSummary {
            primed_frames: capacity,
            ..Default::default()
        });
    }
    let mut task_index = 0;
    let _mmcss =
        match unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut task_index) }
        {
            Ok(handle) => {
                report.mmcss_registered = true;
                Some(Mmcss(handle))
            }
            Err(error) => {
                report
                    .errors
                    .push(ApiFailure::new("MMCSS (optional)", error));
                None
            }
        };
    report.last_stage = "Start";
    let mut streaming = Streaming::start(&client)?;
    let start = Instant::now();
    let duration = Duration::from_secs(u64::from(seconds));
    let result: Result<()> = (|| {
        while start.elapsed() < duration {
            report.last_stage = "audio event wait";
            let timeout = duration
                .saturating_sub(start.elapsed())
                .as_millis()
                .clamp(1, 100) as u32;
            match unsafe { WaitForSingleObject(event.0, timeout) } {
                WAIT_TIMEOUT => {
                    report.timeout_wakes += 1;
                    continue;
                }
                WAIT_OBJECT_0 => report.audio_wakes += 1,
                _ => return Err(windows::core::Error::from_thread()),
            }
            report.last_stage = "IAudioClock::GetPosition (before buffer processing)";
            let (mut position, mut qpc) = (0, 0);
            // Preserve S_FALSE, which the convenience binding converts into Ok(()).
            let hr = unsafe {
                (Interface::vtable(&clock).GetPosition)(
                    Interface::as_raw(&clock),
                    &mut position,
                    &mut qpc,
                )
            };
            if report.clock_points.len() < bound {
                report.clock_points.push(ClockPoint {
                    arrival_ms: start.elapsed().as_secs_f64() * 1000.0,
                    position_units: position,
                    qpc_100ns: qpc,
                    hresult: hr.0,
                });
            } else {
                report.metadata_dropped += 1;
            }
            hr.ok()?;
            if let Some(device_clock) = &device_clock {
                let (mut position, mut qpc) = (0, 0);
                let hr = unsafe {
                    (Interface::vtable(device_clock).GetDevicePosition)(
                        Interface::as_raw(device_clock),
                        &mut position,
                        &mut qpc,
                    )
                };
                if report.device_clock_points.len() < bound {
                    report.device_clock_points.push(ClockPoint {
                        arrival_ms: start.elapsed().as_secs_f64() * 1000.0,
                        position_units: position,
                        qpc_100ns: qpc,
                        hresult: hr.0,
                    });
                } else {
                    report.metadata_dropped += 1;
                }
                // Optional hardware-clock failures remain in the raw trace and
                // diagnostics; they do not suppress an otherwise working stream.
            }
            if let Some(capture) = &capture {
                while start.elapsed() < duration {
                    report.last_stage = "GetNextPacketSize";
                    if unsafe { capture.GetNextPacketSize()? } == 0 {
                        break;
                    }
                    let (mut data, mut frames, mut flags, mut position, mut qpc) =
                        (std::ptr::null_mut(), 0, 0, 0, 0);
                    report.last_stage = "GetBuffer";
                    unsafe {
                        capture.GetBuffer(
                            &mut data,
                            &mut frames,
                            &mut flags,
                            Some(&mut position),
                            Some(&mut qpc),
                        )?;
                    }
                    if frames == 0 {
                        break;
                    }
                    let lease = PacketLease {
                        client: capture,
                        frames,
                        released: false,
                    };
                    report.last_stage = "capture packet validation";
                    if frames > capacity || (flags & SILENT == 0 && data.is_null()) {
                        return Err(unexpected());
                    }
                    let bytes = if flags & SILENT != 0 {
                        &[]
                    } else {
                        unsafe {
                            std::slice::from_raw_parts(
                                data,
                                frames as usize * usize::from(base.nBlockAlign),
                            )
                        }
                    };
                    let metrics = analyze_f32(bytes, frames, base.nChannels, flags & SILENT != 0)
                        .map_err(|_| unexpected())?;
                    if report.capture_packets.len() < bound {
                        report.capture_packets.push(PacketRecord {
                            arrival_ms: start.elapsed().as_secs_f64() * 1000.0,
                            frames,
                            flags,
                            device_position_frames: position,
                            qpc_100ns: qpc,
                            rms: metrics.rms(),
                            peak: metrics.peak,
                        });
                    } else {
                        report.metadata_dropped += 1;
                    }
                    stats.observe(frames, flags, position, qpc, metrics);
                    report.last_stage = "ReleaseBuffer(capture)";
                    lease.release()?;
                }
            }
            if let Some(render) = &render {
                report.last_stage = "GetCurrentPadding";
                let padding = unsafe { client.GetCurrentPadding()? };
                let writable = capacity.checked_sub(padding).ok_or_else(unexpected)?;
                let summary = report.render.as_mut().ok_or_else(unexpected)?;
                summary.zero_demand_wakes += u64::from(writable == 0);
                summary.empty_padding_wakes += u64::from(padding == 0);
                summary.min_writable_frames = Some(
                    summary
                        .min_writable_frames
                        .map_or(writable, |n| n.min(writable)),
                );
                summary.max_writable_frames = summary.max_writable_frames.max(writable);
                if writable != 0 {
                    report.last_stage = "GetBuffer / ReleaseBuffer(render silence)";
                    write_silence(render, writable)?;
                    summary.submitted_frames += u64::from(writable);
                }
                if report.render_demands.len() < bound {
                    report.render_demands.push(DemandRecord {
                        arrival_ms: start.elapsed().as_secs_f64() * 1000.0,
                        padding_frames: padding,
                        writable_frames: writable,
                    });
                } else {
                    report.metadata_dropped += 1;
                }
            }
        }
        Ok(())
    })();
    report.elapsed_seconds = start.elapsed().as_secs_f64();
    let stop = streaming.stop();
    report.stop_succeeded = stop.is_ok();
    if capture.is_some() {
        report.capture = Some(stats.summary());
    }
    if let Err(error) = result {
        if let Err(stop_error) = stop {
            report.errors.push(ApiFailure::new("Stop", stop_error));
        }
        return Err(error);
    }
    report.last_stage = "Stop";
    stop?;
    report.last_stage = "completed";
    report.status = "completed";
    Ok(())
}

fn write_silence(render: &IAudioRenderClient, frames: u32) -> Result<()> {
    if frames == 0 {
        return Ok(());
    }
    unsafe {
        let _leased_data = render.GetBuffer(frames)?;
        // The SILENT flag makes writing into the returned pointer unnecessary.
        render.ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
    }
}

/// All endpoints are explicit. No name matching or default switching occurs here.
pub fn run(
    ids: Vec<String>,
    seconds: u32,
    observe_pid: Option<u32>,
) -> anyhow::Result<PhysicalReport> {
    if ids.is_empty()
        || ids.len() > 8
        || !(1..=600).contains(&seconds)
        || ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id))
    {
        anyhow::bail!(
            "expected 1..=8 unique endpoint IDs and --seconds within 1..=600, got {} IDs and {seconds}s",
            ids.len()
        );
    }
    let _apartment = Apartment::new()?;
    let before = catalog::snapshot()?;
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |t| t.as_millis());
    let mut threads = Vec::new();
    for id in ids {
        let name = before
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == id)
            .and_then(|endpoint| endpoint.name.clone());
        threads.push(std::thread::spawn(move || {
            let mut report = EndpointReport {
                endpoint_id: id,
                name,
                flow: "unknown",
                status: "initializing",
                last_stage: "CoInitializeEx",
                elapsed_seconds: 0.0,
                format: None,
                buffer_frames: None,
                stream_latency_100ns: None,
                engine_period_frames: None,
                category: "Other",
                own_session_ducking_opt_out: false,
                mmcss_registered: false,
                stop_succeeded: false,
                audio_wakes: 0,
                timeout_wakes: 0,
                frequency_units_per_second: None,
                clock: ClockSummary::default(),
                capture_packet_clock: None,
                comparison_clock_source: "unavailable",
                clock_points: Vec::new(),
                device_clock: None,
                device_clock_points: Vec::new(),
                capture: None,
                capture_packets: Vec::new(),
                render: None,
                render_demands: Vec::new(),
                metadata_dropped: 0,
                errors: Vec::new(),
            };
            if let Err(error) = prepare_and_sample(&mut report, seconds) {
                report.status = "api_failed";
                report
                    .errors
                    .push(ApiFailure::new(report.last_stage, error));
            }
            report
        }));
    }
    let mut endpoints = Vec::new();
    let mut worker_panicked = false;
    for thread in threads {
        match thread.join() {
            Ok(endpoint) => endpoints.push(endpoint),
            Err(_) => worker_panicked = true,
        }
    }
    if worker_panicked {
        // Join every owner before returning; dropping a JoinHandle detaches it.
        anyhow::bail!("an endpoint worker thread panicked during sampling");
    }
    // Packet timestamps refer to the first captured frame. Preserve the separate
    // IAudioClock observations, including stale or inconsistent capture QPC pairs.
    // These post-stop vectors contain metadata only.
    let comparison_points: Vec<Vec<ClockPoint>> = endpoints
        .iter()
        .map(|endpoint| {
            if endpoint.flow == "capture" {
                endpoint
                    .capture_packets
                    .iter()
                    .map(|packet| ClockPoint {
                        arrival_ms: packet.arrival_ms,
                        position_units: packet.device_position_frames,
                        qpc_100ns: packet.qpc_100ns,
                        hresult: if packet.flags & TIMESTAMP_ERROR == 0 {
                            0
                        } else {
                            E_UNEXPECTED.0
                        },
                    })
                    .collect()
            } else {
                endpoint.clock_points.clone()
            }
        })
        .collect();
    // Exclude the first second and fit all endpoints over the same correlated QPC interval.
    let usable = |p: &&ClockPoint| p.hresult == 0 && p.qpc_100ns > 0 && p.arrival_ms >= 1000.0;
    let starts: Vec<_> = comparison_points
        .iter()
        .filter_map(|points| points.iter().find(usable).map(|p| p.qpc_100ns))
        .collect();
    let ends: Vec<_> = comparison_points
        .iter()
        .filter_map(|points| points.iter().rev().find(usable).map(|p| p.qpc_100ns))
        .collect();
    let window = starts
        .iter()
        .max()
        .zip(ends.iter().min())
        .and_then(|(&start, &end)| (start < end).then_some((start, end)));
    for (endpoint, points) in endpoints.iter_mut().zip(&comparison_points) {
        if !endpoint.device_clock_points.is_empty() {
            let mut summary = analyze_device_clock(&endpoint.device_clock_points, None);
            if summary.diagnostics.regressions == 0
                && summary.diagnostics.inconsistent_qpc_reads == 0
                && let Some(window) = window
            {
                summary.fit = analyze_device_clock(&endpoint.device_clock_points, Some(window)).fit;
            }
            endpoint.device_clock = Some(summary);
        }
        endpoint.clock = analyze_clock(
            &endpoint.clock_points,
            endpoint.frequency_units_per_second.unwrap_or(0),
            None,
        );
        if endpoint.clock.regressions == 0
            && endpoint.clock.inconsistent_qpc_reads == 0
            && let Some(window) = window
        {
            endpoint.clock.fit = analyze_clock(
                &endpoint.clock_points,
                endpoint.frequency_units_per_second.unwrap_or(0),
                Some(window),
            )
            .fit;
        }
        if endpoint.flow == "capture" {
            let frequency = endpoint
                .format
                .as_ref()
                .map_or(0, |f| u64::from(f.sample_rate));
            let mut packet_clock = analyze_clock(points, frequency, None);
            if packet_clock.regressions == 0
                && packet_clock.inconsistent_qpc_reads == 0
                && let Some(window) = window
            {
                packet_clock.fit = analyze_clock(points, frequency, Some(window)).fit;
            }
            if endpoint
                .capture
                .as_ref()
                .is_some_and(|capture| capture.later_discontinuities > 0)
            {
                packet_clock.fit = None;
            }
            endpoint.capture_packet_clock = Some(packet_clock);
            endpoint.comparison_clock_source = "capture_packet_frames / sample_rate";
        } else if endpoint.flow == "render" {
            endpoint.comparison_clock_source = "IAudioClock_position / GetFrequency";
        }
    }
    let mut relative_clocks = Vec::new();
    for (i, left) in endpoints.iter().enumerate() {
        for right in &endpoints[i + 1..] {
            let left_clock = left.capture_packet_clock.as_ref().unwrap_or(&left.clock);
            let right_clock = right.capture_packet_clock.as_ref().unwrap_or(&right.clock);
            let ppm = window.and_then(|_| {
                left_clock
                    .fit
                    .as_ref()
                    .zip(right_clock.fit.as_ref())
                    .and_then(|(l, r)| relative_ppm(l.rate_ratio, r.rate_ratio))
            });
            relative_clocks.push(RelativeClock {
                left_endpoint: left.endpoint_id.clone(),
                right_endpoint: right.endpoint_id.clone(),
                left_faster_ppm: ppm,
            });
        }
    }
    let mut result = PhysicalReport {
        schema_version: 1,
        started_unix_ms,
        requested_seconds: seconds,
        before,
        endpoints,
        common_qpc_window_100ns: window,
        relative_clocks,
        after: None,
        observed_state_changes: Vec::new(),
        errors: Vec::new(),
    };
    match catalog::snapshot() {
        Ok(after) => {
            result.observed_state_changes =
                catalog::changes(&result.before, &after, observe_pid.unwrap_or(0));
            result.after = Some(after);
        }
        Err(error) => result
            .errors
            .push(ApiFailure::new("post-probe snapshot", error)),
    }
    Ok(result)
}
