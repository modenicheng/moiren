//! All COM objects stay on the worker; preparation precedes the event loop.
use super::{
    RenderOwnerExit, Wake, api, format::mix_format, handle, packet::submit, ready_and_wait, wait,
};
use crate::{Activation, ActivationGate};
use crate::{
    owner::{Apartment, Mmcss, OwnedHandle, Streaming, TaskMemory},
    render::{
        CHANNELS, DemandRenderer, RenderError, RenderOptions, RenderReport, RenderStats,
        RenderStatus, SAMPLE_RATE, writable_frames,
    },
};
use std::{
    os::windows::io::OwnedHandle as StdHandle,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_NOPERSIST, AudioCategory_Other, AudioClientProperties,
            IAudioClient, IAudioClient2, IAudioClient3, IAudioRenderClient, IAudioSessionControl,
            IAudioSessionControl2, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
            Threading::{AvSetMmThreadCharacteristicsW, SetEvent},
        },
    },
    core::{HSTRING, Interface},
};

// Optional metadata cannot prevent an otherwise supported stream from starting.
fn record_optional_properties(client: &IAudioClient, report: &mut RenderReport) {
    match client.cast::<IAudioClient3>() {
        Ok(client3) => {
            let mut ptr = std::ptr::null_mut();
            let mut period = 0;
            let period_result =
                unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut ptr, &mut period) };
            let _format = TaskMemory(ptr);
            match period_result {
                Ok(()) => report.engine_period_frames = Some(period),
                Err(error) => report.period_hresult = Some(error.code().0),
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

pub(super) fn owner(
    options: &RenderOptions,
    renderer: &mut DemandRenderer,
    stop: &StdHandle,
    report: &mut RenderReport,
    gate: &ActivationGate,
    ready: &SyncSender<Result<(), RenderError>>,
    acknowledged: &AtomicBool,
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
    let memory = mix_format(&client, report)?;
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
    record_optional_properties(&client, report);
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
    if ready_and_wait(ready, gate)? == Activation::Cancelled {
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
    acknowledged.store(true, Ordering::Release);
    let started = Instant::now();
    let result = (|| {
        while options.duration.remaining(started.elapsed()) != Some(Duration::ZERO) {
            let timeout_ms = options.duration.timeout_ms(started.elapsed());
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
    let peer_stop = api("SetEvent(render peer stop)", unsafe {
        SetEvent(handle(stop))
    });
    let stopped = streaming.stop();
    report.stop_succeeded = stopped.is_ok();
    if let Err(error) = &stopped {
        report.stop_hresult = Some(error.code().0);
    }
    // Preserve a streaming failure and separately record any Stop failure.
    match result {
        Err(error) => Err(error),
        Ok(status) => {
            peer_stop?;
            api("Stop", stopped)?;
            Ok(status)
        }
    }
}

pub(super) fn run_owner_with(
    options: RenderOptions,
    mut renderer: DemandRenderer,
    stop: Arc<StdHandle>,
    gate: ActivationGate,
    ready: SyncSender<Result<(), RenderError>>,
    acknowledged: Arc<AtomicBool>,
    run: impl FnOnce(
        &RenderOptions,
        &mut DemandRenderer,
        &StdHandle,
        &mut RenderReport,
        &ActivationGate,
        &SyncSender<Result<(), RenderError>>,
        &AtomicBool,
    ) -> Result<RenderStatus, RenderError>,
) -> RenderOwnerExit {
    let initial_frame = renderer.timeline();
    let mut report = RenderReport {
        schema_version: 2,
        endpoint_id: options.endpoint_id.clone(),
        status: RenderStatus::Failed,
        requested_seconds: options.duration.requested_seconds(),
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
    // formatting. Engine/ring ownership returns to the joining cleanup worker.
    let result = run(
        &options,
        &mut renderer,
        &stop,
        &mut report,
        &gate,
        &ready,
        &acknowledged,
    );
    let _ = unsafe { SetEvent(handle(&stop)) };
    match result {
        Ok(status) => report.status = status,
        Err(error) => {
            report.failure = Some(error.to_string());
            let _ = ready.try_send(Err(error));
        }
    }
    report.stats.processed_frames = renderer.timeline().saturating_sub(initial_frame);
    RenderOwnerExit { report, renderer }
}
