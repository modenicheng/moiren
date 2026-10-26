//! All COM objects stay on the worker; preparation precedes the event loop.
use super::{RenderOwnerExit, Wake, api, format::mix_format, handle, packet::submit, wait};
use crate::latency::{CallbackObservation, LatencyRecorder, clock::QpcClock};
use crate::negotiation::{OpenedSharedClient, SharedOpenStages, open_shared_client};
use crate::render::SampleRenderer;
use crate::{Activation, ActivationGate};
use crate::{
    owner::{Apartment, Mmcss, OwnedHandle, Streaming},
    render::{
        CHANNELS, DemandRenderer, RenderError, RenderOptions, RenderReport, RenderStats,
        RenderStatus, SAMPLE_RATE, writable_frames,
    },
};
use std::{
    os::windows::io::OwnedHandle as StdHandle,
    panic::{AssertUnwindSafe, catch_unwind},
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
            IAudioClient, IAudioClock, IAudioRenderClient, IAudioSessionControl,
            IAudioSessionControl2,
        },
        System::Threading::SetEvent,
    },
    core::Interface,
};

const OPEN_STAGES: SharedOpenStages = SharedOpenStages::render();

// Optional metadata cannot prevent an otherwise supported stream from starting.
fn record_optional_properties(client: &IAudioClient, report: &mut RenderReport) {
    report.engine_period_frames = report.shared_period.engine_period_frames;
    report.period_hresult = report.shared_period.actual_period_hresult;
    let ducking_result = unsafe { client.GetService::<IAudioSessionControl>() }
        .and_then(|s| s.cast::<IAudioSessionControl2>())
        .and_then(|session| unsafe { session.SetDuckingPreference(true) });
    match ducking_result {
        Ok(()) => report.own_session_ducking_opt_out = true,
        Err(error) => report.ducking_hresult = Some(error.code().0),
    }
}

fn produce(
    renderer: &mut impl SampleRenderer,
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

/// Fields captured once per wake, before the demand decision, that both the
/// zero-demand and the render/submit paths report identically.
struct WakeTrace {
    wake_qpc_100ns: Option<u64>,
    padding_before_frames: u32,
    submitted_frames_before: u64,
    native_clock: Option<super::super::RenderClockSample>,
    bridge_before: Option<crate::clock_bridge::BridgeSnapshot>,
}

/// What the wake did between the demand decision and the recording call.
enum CallbackOutcome {
    /// Demand was zero: no render or submit was attempted.
    ZeroDemand,
    /// A render/submit of `demand` frames ran; `failed` preserves its error.
    Submitted { demand: u32, failed: bool },
}

/// One wake, one record: the stats wake counters, the trace diagnostics event
/// and the latency telemetry observation converge here so the zero-demand and
/// the render/submit paths can never drift apart.
#[inline]
fn record_callback(
    report: &mut RenderReport,
    telemetry: &mut LatencyRecorder,
    clock: Option<&QpcClock>,
    wake: &WakeTrace,
    outcome: CallbackOutcome,
    bridge_after: Option<crate::clock_bridge::BridgeSnapshot>,
) {
    let (demand_frames, submitted_frames, failed) = match outcome {
        CallbackOutcome::ZeroDemand => (0, 0, false),
        CallbackOutcome::Submitted { demand, failed } => {
            (demand, if failed { 0 } else { u64::from(demand) }, failed)
        }
    };
    if demand_frames == 0 {
        report.stats.zero_demand_wakes += 1;
    } else if wake.padding_before_frames == 0 {
        report.stats.empty_padding_wakes += 1;
    }
    report.record_diagnostics(super::super::RenderCallback {
        wake_qpc_100ns: wake.wake_qpc_100ns,
        padding_before_frames: wake.padding_before_frames,
        demand_frames,
        submitted_frames_before: wake.submitted_frames_before,
        native_clock: wake.native_clock,
        failed,
        bridge_before: wake.bridge_before,
        bridge_after,
        ..Default::default()
    });
    telemetry.record(CallbackObservation {
        wake_qpc_100ns: wake.wake_qpc_100ns,
        completed_qpc_100ns: clock.and_then(QpcClock::now),
        demand_frames: u64::from(demand_frames),
        padding_before_frames: Some(wake.padding_before_frames),
        submitted_frames,
    });
}

pub(super) struct NativeRender {
    render: IAudioRenderClient,
    client: IAudioClient,
    event: OwnedHandle,
    capacity: u32,
    clock: Option<IAudioClock>,
}

fn clock_sample(clock: &IAudioClock) -> super::super::RenderClockSample {
    let mut sample = super::super::RenderClockSample::default();
    // The service is prepared and retained on this owner before Start. Keep
    // S_FALSE instead of losing its low-accuracy warning in Result<()>.
    sample.hresult = unsafe {
        (Interface::vtable(clock).GetPosition)(
            Interface::as_raw(clock),
            &mut sample.position_units,
            &mut sample.qpc_100ns,
        )
    }
    .0;
    sample
}
impl NativeRender {
    pub(super) fn timing(&self, report: &RenderReport) -> crate::latency::NativeStreamTiming {
        crate::latency::NativeStreamTiming {
            sample_rate: SAMPLE_RATE,
            period_frames: report.shared_period.actual_period_frames,
            capacity_frames: self.capacity,
        }
    }
}
pub(super) fn with_native<ResultValue>(
    options: &RenderOptions,
    stop: &StdHandle,
    report: &mut RenderReport,
    prepared: impl FnOnce(&NativeRender, &mut RenderReport) -> Result<ResultValue, RenderError>,
) -> Result<ResultValue, RenderError> {
    if wait(handle(stop), handle(stop), 0)? == Wake::Stop {
        return Err(RenderError::Cancelled);
    }
    // Declaration order keeps all COM services/client and event handles alive
    // through Stop, then releases them before CoUninitialize on this thread.
    let _apartment = api("CoInitializeEx(render)", Apartment::new())?;
    let (opened, ()) = open_shared_client(
        OPEN_STAGES,
        &options.endpoint_id,
        false,
        report,
        |report, memory| -> Result<_, RenderError> {
            mix_format(memory, report).map(|format| (format, ()))
        },
        |report, client, format, guid| -> Result<(), RenderError> {
            crate::shared_stream::initialize_shared(
                client,
                format,
                options.shared_period,
                guid,
                &mut report.shared_period,
            )
            .map_err(|error| RenderError::Api {
                stage: error.stage,
                hresult: error.error.code().0,
            })?;
            report.format_supported = true;
            Ok(())
        },
    )?;
    let OpenedSharedClient { client, event, .. } = opened;
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
    let clock = if options.latency_telemetry == crate::latency::TelemetryMode::Trace {
        match unsafe { client.GetService::<IAudioClock>() }.and_then(|clock| {
            let frequency = unsafe { clock.GetFrequency()? };
            if frequency == 0 {
                return Err(windows::Win32::Foundation::E_INVALIDARG.into());
            }
            report.clock_frequency = Some(frequency);
            Ok(clock)
        }) {
            Ok(clock) => Some(clock),
            Err(error) => {
                report.clock_hresult = Some(error.code().0);
                None
            }
        }
    } else {
        None
    };
    prepared(
        &NativeRender {
            render,
            client,
            event,
            capacity,
            clock,
        },
        report,
    )
}
pub(super) struct StreamContext<'a> {
    pub options: &'a RenderOptions,
    pub stop: &'a StdHandle,
    pub gate: &'a ActivationGate,
    pub acknowledged: &'a AtomicBool,
    pub padding: Option<&'a mut crate::render::RenderPaddingOwner>,
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
    with_native(options, stop, report, |native, report| {
        report.queue = Some(crate::latency::prepare_render_queue(
            native.timing(report),
            options.queue,
        )?);
        run_stream(
            native,
            renderer,
            report,
            StreamContext {
                options,
                stop,
                gate,
                acknowledged,
                padding: None,
            },
            || ready.send(Ok(())).map_err(|_| RenderError::StartupLost),
        )
    })
}
pub(super) fn run_stream(
    native: &NativeRender,
    renderer: &mut impl SampleRenderer,
    report: &mut RenderReport,
    context: StreamContext<'_>,
    ready: impl FnOnce() -> Result<(), RenderError>,
) -> Result<RenderStatus, RenderError> {
    let StreamContext {
        options,
        stop,
        gate,
        acknowledged,
        padding,
    } = context;
    // Borrow, never own the Arc on the callback stack. Every return/unwind
    // invalidates the sample; the capability itself goes back to the joiner.
    struct InvalidatePadding<'a>(Option<&'a mut crate::render::RenderPaddingOwner>);
    impl Drop for InvalidatePadding<'_> {
        fn drop(&mut self) {
            if let Some(padding) = self.0.as_mut() {
                padding.invalidate();
            }
        }
    }
    let mut padding_observation = InvalidatePadding(padding);
    let NativeRender {
        client,
        render,
        event,
        capacity,
        clock: native_clock,
    } = native;
    let capacity = *capacity;
    let queue = report.queue.ok_or(RenderError::StartupLost)?;
    report.prepare_diagnostics(queue.target_frames);
    // Native capacity is driver supplied: cap staging to the explicit 8 MiB
    // preparation budget before allocating; failure never enters streaming.
    let sample_count = (capacity as usize)
        .checked_mul(CHANNELS)
        .filter(|&n| n <= 8 * 1024 * 1024 / size_of::<f32>())
        .ok_or(RenderError::StagingBudget)?;
    let mut staging = vec![0.0f32; sample_count];
    let mut telemetry = LatencyRecorder::new(options.latency_telemetry);
    let trace = options.latency_telemetry == crate::latency::TelemetryMode::Trace;
    let clock = QpcClock::prepare(options.latency_telemetry);
    ready()?;
    if gate.wait()? == Activation::Cancelled {
        return Ok(RenderStatus::Stopped);
    }
    let prefill = &mut staging[..queue.target_frames as usize * CHANNELS];
    produce(renderer, prefill, &mut report.stats)?;
    submit(render, prefill)?;
    renderer.acknowledge_submission();
    report.stats.primed_frames = queue.target_frames;
    record_submit(prefill, &mut report.stats);
    let _mmcss = match Mmcss::audio_detailed() {
        Ok(mmcss) => {
            report.mmcss_registered = true;
            Some(mmcss)
        }
        Err(error) => {
            report.mmcss_hresult = Some(error.code().0);
            None
        }
    };
    let mut streaming = api("Start", Streaming::start(client))?;
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
            let wake_qpc_100ns = clock.as_ref().and_then(QpcClock::now);
            let padding = api("GetCurrentPadding", unsafe { client.GetCurrentPadding() })?;
            writable_frames(capacity, padding)?;
            if let Some(owner) = padding_observation.0.as_mut() {
                owner.publish(crate::render::RenderPaddingSample {
                    padding_frames: padding,
                    capacity_frames: capacity,
                    sample_rate: Some(SAMPLE_RATE),
                    period_frames: report.shared_period.actual_period_frames,
                });
            }
            let wake = WakeTrace {
                wake_qpc_100ns,
                padding_before_frames: padding,
                submitted_frames_before: report.stats.submitted_frames,
                native_clock: trace
                    .then(|| native_clock.as_ref().map(clock_sample))
                    .flatten(),
                bridge_before: trace.then(|| renderer.bridge_snapshot()).flatten(),
            };
            let writable = writable_frames(capacity, padding)?;
            let demand = queue.demand(padding)?;
            if writable > 0 {
                report.stats.min_writable_frames = Some(
                    report
                        .stats
                        .min_writable_frames
                        .map_or(writable, |frames| frames.min(writable)),
                );
                report.stats.max_writable_frames = report.stats.max_writable_frames.max(writable);
            }
            if demand == 0 {
                record_callback(
                    report,
                    &mut telemetry,
                    clock.as_ref(),
                    &wake,
                    CallbackOutcome::ZeroDemand,
                    wake.bridge_before,
                );
                continue;
            }
            let samples = &mut staging[..demand as usize * CHANNELS];
            let result = produce(renderer, samples, &mut report.stats)
                .and_then(|()| submit(render, samples));
            if result.is_ok() {
                renderer.acknowledge_submission();
                record_submit(samples, &mut report.stats);
            }
            // Preserve the terminal wake and post-render bridge state before
            // propagating the original error. Failed transfers are not submissions.
            record_callback(
                report,
                &mut telemetry,
                clock.as_ref(),
                &wake,
                CallbackOutcome::Submitted {
                    demand,
                    failed: result.is_err(),
                },
                trace.then(|| renderer.bridge_snapshot()).flatten(),
            );
            result?;
        }
        Ok(RenderStatus::Completed)
    })();
    report.elapsed_seconds = started.elapsed().as_secs_f64();
    let peer_stop = api("SetEvent(render peer stop)", unsafe {
        SetEvent(handle(stop))
    });
    let stopped = streaming.stop();
    report.latency = telemetry.snapshot();
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
    let mut report = new_report(&options);
    // Both owned allocations survive a callback panic and return to the joiner
    // after the borrowed native objects and Streaming guard have unwound.
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(
            &options,
            &mut renderer,
            &stop,
            &mut report,
            &gate,
            &ready,
            &acknowledged,
        )
    }))
    .unwrap_or(Err(RenderError::WorkerPanicked));
    report.finish_diagnostics();
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
pub(super) fn new_report(options: &RenderOptions) -> RenderReport {
    RenderReport {
        schema_version: 8,
        queue: None,
        latency: LatencyRecorder::new(options.latency_telemetry).snapshot(),
        diagnostics: super::super::RenderDiagnosticsSnapshot {
            enabled: options.latency_telemetry == crate::latency::TelemetryMode::Trace,
            ..Default::default()
        },
        diagnostics_recorder: Some(super::super::RenderDiagnostics::new(
            options.latency_telemetry,
            0,
        )),
        shared_period: crate::shared_stream::SharedPeriodReport::default(),
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
        clock_frequency: None,
        clock_hresult: None,
        mmcss_registered: false,
        mmcss_hresult: None,
        own_session_ducking_opt_out: false,
        ducking_hresult: None,
        stop_succeeded: false,
        stream_started: false,
        stop_hresult: None,
        stats: RenderStats::default(),
        failure: None,
    }
}

#[cfg(test)]
pub(super) mod diagnostic_tests {
    use super::*;
    use crate::{
        clock_bridge::BridgeSnapshot,
        latency::{NativeStreamTiming, RenderQueuePolicy, TelemetryMode, prepare_render_queue},
        render::{DemandCounters, DemandReport, RENDER_EVENT_CAPACITY, RenderEvent},
    };
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
        sync::atomic::AtomicUsize,
    };
    use windows::{
        Win32::{Foundation::E_FAIL, Media::Audio::*},
        core::{GUID, HRESULT, Result as WinResult, implement},
    };

    // Deliberately independent of the shared counting allocator in
    // `tests/common/alloc_guard.rs`: that guard counts per thread, while this
    // probe registers one specific RenderEvent allocation by pointer and
    // attributes its frees to the worker versus the joiner thread through
    // atomics — the library's unit-test binary also cannot share a
    // `tests/common` module, and only one `#[global_allocator]` may exist.
    struct EventAllocator;
    thread_local! {
        static TRACK_EVENTS: Cell<bool> = const { Cell::new(false) };
    }
    static EVENT_POINTER: AtomicUsize = AtomicUsize::new(0);
    static WORKER_FREES: AtomicUsize = AtomicUsize::new(0);
    static JOINER_FREES: AtomicUsize = AtomicUsize::new(0);

    unsafe impl GlobalAlloc for EventAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: Forward the caller's allocation contract unchanged.
            let pointer = unsafe { System.alloc(layout) };
            if layout.size() == size_of::<RenderEvent>() * RENDER_EVENT_CAPACITY
                && TRACK_EVENTS.try_with(Cell::get).unwrap_or(false)
            {
                let _ = EVENT_POINTER.compare_exchange(
                    0,
                    pointer as usize,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }
            pointer
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            if pointer as usize == EVENT_POINTER.load(Ordering::SeqCst) {
                if TRACK_EVENTS.try_with(Cell::get).unwrap_or(false) {
                    WORKER_FREES.fetch_add(1, Ordering::SeqCst);
                } else {
                    JOINER_FREES.fetch_add(1, Ordering::SeqCst);
                }
            }
            // SAFETY: This allocator forwards all allocations to System.
            unsafe { System.dealloc(pointer, layout) };
        }
    }
    #[global_allocator]
    static ALLOCATOR: EventAllocator = EventAllocator;

    pub(crate) fn track_events() {
        EVENT_POINTER.store(0, Ordering::SeqCst);
        WORKER_FREES.store(0, Ordering::SeqCst);
        JOINER_FREES.store(0, Ordering::SeqCst);
        TRACK_EVENTS.with(|track| track.set(true));
    }
    pub(crate) fn stop_tracking_events() {
        TRACK_EVENTS.with(|track| track.set(false));
    }
    pub(crate) fn event_allocation() -> (usize, usize, usize) {
        (
            EVENT_POINTER.load(Ordering::SeqCst),
            WORKER_FREES.load(Ordering::SeqCst),
            JOINER_FREES.load(Ordering::SeqCst),
        )
    }

    #[derive(Clone, Copy, Default)]
    pub(crate) enum Failure {
        #[default]
        None,
        Render,
        GetBuffer,
        ReleaseBuffer,
        Start,
        Cancel,
    }
    #[implement(IAudioClock)]
    struct DiagnosticClock {
        hresult: i32,
    }
    impl IAudioClock_Impl for DiagnosticClock_Impl {
        fn GetFrequency(&self) -> WinResult<u64> {
            Ok(384_000)
        }
        fn GetPosition(&self, position: *mut u64, qpc: *mut u64) -> WinResult<()> {
            // SAFETY: clock_sample supplies valid owner-local output scalars.
            unsafe {
                *position = 1234;
                *qpc = 5678;
            }
            if self.hresult == 0 {
                Ok(())
            } else {
                Err(windows::core::Error::from_hresult(HRESULT(self.hresult)))
            }
        }
        fn GetCharacteristics(&self) -> WinResult<u32> {
            unreachable!()
        }
    }
    #[implement(IAudioClient)]
    struct DiagnosticClient {
        audio: windows::Win32::Foundation::HANDLE,
        padding: u32,
        fail_start: bool,
    }
    impl IAudioClient_Impl for DiagnosticClient_Impl {
        fn Initialize(
            &self,
            _: AUDCLNT_SHAREMODE,
            _: u32,
            _: i64,
            _: i64,
            _: *const WAVEFORMATEX,
            _: *const GUID,
        ) -> WinResult<()> {
            unreachable!()
        }
        fn GetBufferSize(&self) -> WinResult<u32> {
            unreachable!()
        }
        fn GetStreamLatency(&self) -> WinResult<i64> {
            unreachable!()
        }
        fn GetCurrentPadding(&self) -> WinResult<u32> {
            // Rearm the fake wake for the next callback without device access.
            unsafe { SetEvent(self.audio)? };
            Ok(self.padding)
        }
        fn IsFormatSupported(
            &self,
            _: AUDCLNT_SHAREMODE,
            _: *const WAVEFORMATEX,
            _: *mut *mut WAVEFORMATEX,
        ) -> HRESULT {
            unreachable!()
        }
        fn GetMixFormat(&self) -> WinResult<*mut WAVEFORMATEX> {
            unreachable!()
        }
        fn GetDevicePeriod(&self, _: *mut i64, _: *mut i64) -> WinResult<()> {
            unreachable!()
        }
        fn Start(&self) -> WinResult<()> {
            if self.fail_start {
                return Err(E_FAIL.into());
            }
            unsafe { SetEvent(self.audio) }
        }
        fn Stop(&self) -> WinResult<()> {
            Ok(())
        }
        fn Reset(&self) -> WinResult<()> {
            unreachable!()
        }
        fn SetEventHandle(&self, _: windows::Win32::Foundation::HANDLE) -> WinResult<()> {
            unreachable!()
        }
        fn GetService(&self, _: *const GUID, _: *mut *mut std::ffi::c_void) -> WinResult<()> {
            Err(windows::Win32::Foundation::E_NOINTERFACE.into())
        }
    }
    #[implement(IAudioRenderClient)]
    struct DiagnosticRender {
        data: std::cell::UnsafeCell<[f32; 8]>,
        submissions: Cell<usize>,
        failure: Failure,
    }
    impl IAudioRenderClient_Impl for DiagnosticRender_Impl {
        fn GetBuffer(&self, frames: u32) -> WinResult<*mut u8> {
            assert!(frames <= 4);
            self.submissions.set(self.submissions.get() + 1);
            if self.submissions.get() == 2 && matches!(self.failure, Failure::GetBuffer) {
                return Err(E_FAIL.into());
            }
            Ok(self.data.get().cast())
        }
        fn ReleaseBuffer(&self, _: u32, _: u32) -> WinResult<()> {
            if self.submissions.get() == 2 && matches!(self.failure, Failure::ReleaseBuffer) {
                return Err(E_FAIL.into());
            }
            Ok(())
        }
    }
    pub(crate) fn options() -> RenderOptions {
        RenderOptions {
            queue: RenderQueuePolicy::FullCapacity,
            latency_telemetry: TelemetryMode::Trace,
            shared_period: Default::default(),
            endpoint_id: "fake-diagnostic-render".into(),
            duration: crate::SessionDuration::For(Duration::from_secs(1)),
        }
    }
    #[test]
    fn converted_render_keeps_raw_period_but_admits_queues_in_client_frames() {
        let mut report = new_report(&options());
        report.shared_period.engine_period_frames = Some(960);
        report.shared_period.engine_sample_rate = Some(96_000);
        report.shared_period.actual_period_frames = Some(480);
        let event = OwnedHandle::event().unwrap();
        let native = NativeRender {
            client: DiagnosticClient {
                audio: event.0,
                padding: 0,
                fail_start: false,
            }
            .into(),
            render: DiagnosticRender {
                data: std::cell::UnsafeCell::new([0.0; 8]),
                submissions: Cell::new(0),
                failure: Failure::None,
            }
            .into(),
            event,
            capacity: 960,
            clock: None,
        };
        record_optional_properties(&native.client, &mut report);
        assert_eq!(report.engine_period_frames, Some(960));
        assert_eq!(native.timing(&report).sample_rate, 48_000);
        assert_eq!(native.timing(&report).period_frames, Some(480));
    }
    pub(crate) fn run(
        options: &RenderOptions,
        report: &mut RenderReport,
        renderer: &mut impl SampleRenderer,
        failure: Failure,
        padding: u32,
    ) -> Result<RenderStatus, RenderError> {
        run_with_padding(
            options,
            report,
            renderer,
            failure,
            padding,
            &crate::render::RenderPaddingObserver::new(),
        )
    }
    fn run_with_padding(
        options: &RenderOptions,
        report: &mut RenderReport,
        renderer: &mut impl SampleRenderer,
        failure: Failure,
        padding: u32,
        observer: &crate::render::RenderPaddingObserver,
    ) -> Result<RenderStatus, RenderError> {
        let stop = crate::StopSignal::new().unwrap();
        let gate = ActivationGate::new(stop.clone()).unwrap();
        if matches!(failure, Failure::Cancel) {
            gate.cancel().unwrap();
        } else {
            gate.activate().unwrap();
        }
        let event = OwnedHandle::event().unwrap();
        let native = NativeRender {
            client: DiagnosticClient {
                audio: event.0,
                padding,
                fail_start: matches!(failure, Failure::Start),
            }
            .into(),
            render: DiagnosticRender {
                data: std::cell::UnsafeCell::new([0.0; 8]),
                submissions: Cell::new(0),
                failure,
            }
            .into(),
            event,
            capacity: 4,
            clock: Some(DiagnosticClock { hresult: 0 }.into()),
        };
        report.clock_frequency = Some(384_000);
        report.queue = Some(
            prepare_render_queue(
                NativeStreamTiming {
                    sample_rate: SAMPLE_RATE,
                    period_frames: Some(2),
                    capacity_frames: 4,
                },
                options.queue,
            )
            .unwrap(),
        );
        let result = run_stream(
            &native,
            renderer,
            report,
            StreamContext {
                options,
                stop: &stop.event,
                gate: &gate,
                acknowledged: &AtomicBool::new(false),
                padding: Some(&mut observer.new_owner()),
            },
            || Ok(()),
        );
        report.finish_diagnostics();
        result
    }
    struct DiagnosticRenderer {
        renders: usize,
        acknowledged: Cell<usize>,
        failure: Failure,
        bridge: Option<BridgeSnapshot>,
    }

    #[test]
    fn native_padding_is_published_before_render_even_with_trace_disabled() {
        struct InspectPadding(crate::render::RenderPaddingObserver, usize);
        impl SampleRenderer for InspectPadding {
            fn timeline(&self) -> u64 {
                0
            }
            fn counters(&self) -> DemandCounters {
                DemandCounters::default()
            }
            fn render_interleaved(&mut self, _: &mut [f32]) -> Result<DemandReport, RenderError> {
                self.1 += 1;
                if self.1 > 1 {
                    let sample = self
                        .0
                        .try_snapshot()
                        .expect("padding missing in native callback");
                    assert_eq!(sample.padding_frames, 1);
                    assert_eq!(sample.capacity_frames, 4);
                    assert_eq!(sample.sample_rate, Some(SAMPLE_RATE));
                    assert_eq!(sample.period_frames, Some(2));
                    return Err(RenderError::InvalidSamples);
                }
                assert!(self.0.try_snapshot().is_none());
                Ok(DemandReport::default())
            }
            fn acknowledge_submission(&self) {}
        }
        let mut options = options();
        options.latency_telemetry = TelemetryMode::Disabled;
        let mut report = new_report(&options);
        report.shared_period.actual_period_frames = Some(2);
        let observer = crate::render::RenderPaddingObserver::new();
        let mut renderer = InspectPadding(observer.clone(), 0);
        assert_eq!(
            run_with_padding(
                &options,
                &mut report,
                &mut renderer,
                Failure::None,
                1,
                &observer
            ),
            Err(RenderError::InvalidSamples)
        );
        assert!(
            observer.try_snapshot().is_none(),
            "stopped stream retained stale padding"
        );
        assert_eq!(report.stats.empty_padding_wakes, 0);
    }
    impl SampleRenderer for DiagnosticRenderer {
        fn timeline(&self) -> u64 {
            0
        }
        fn counters(&self) -> DemandCounters {
            DemandCounters::default()
        }
        fn render_interleaved(&mut self, _: &mut [f32]) -> Result<DemandReport, RenderError> {
            self.renders += 1;
            if let Some(bridge) = &mut self.bridge {
                bridge.fill_frames -= 4;
                bridge.backlog_recoveries += 1;
                bridge.underrun_frames += 2;
            }
            if self.renders == 2 && matches!(self.failure, Failure::Render) {
                return Err(RenderError::InvalidSamples);
            }
            Ok(DemandReport::default())
        }
        fn acknowledge_submission(&self) {
            self.acknowledged.set(self.acknowledged.get() + 1);
        }
        fn bridge_snapshot(&self) -> Option<BridgeSnapshot> {
            self.bridge
        }
    }

    fn assert_terminal_error(failure: Failure, expected: RenderError) {
        let options = options();
        let mut report = new_report(&options);
        let mut renderer = DiagnosticRenderer {
            renders: 0,
            acknowledged: Cell::new(0),
            failure,
            bridge: Some(BridgeSnapshot {
                fill_frames: 100,
                ..Default::default()
            }),
        };
        assert_eq!(
            run(&options, &mut report, &mut renderer, failure, 0),
            Err(expected)
        );
        assert_eq!(renderer.acknowledged.get(), 1, "only prefill was submitted");
        assert_eq!(report.stats.submitted_frames, 4);
        assert_eq!(report.stats.empty_padding_wakes, 1);
        assert_eq!(
            report.diagnostics.events.len(),
            1,
            "terminal callback is missing"
        );
        let event = &report.diagnostics.events[0];
        assert!(event.callback.wake_qpc_100ns.is_some());
        assert_eq!(event.callback.padding_before_frames, 0);
        assert_eq!(event.callback.demand_frames, 4);
        assert_eq!(event.callback.bridge_before.unwrap().fill_frames, 96);
        assert_eq!(event.callback.bridge_after.unwrap().fill_frames, 92);
        assert!(event.bridge_recovered);
        assert!(event.bridge_underrun);
        assert!(event.empty_padding);
        let latency = report.latency.trace.records().last().unwrap();
        assert_eq!(latency.demand_frames, 4);
        assert_eq!(latency.submitted_frames, 0);
    }

    #[test]
    fn terminal_render_error_retains_callback_and_original_error() {
        assert_terminal_error(Failure::Render, RenderError::InvalidSamples);
    }

    #[test]
    fn terminal_get_buffer_error_retains_callback_and_original_error() {
        assert_terminal_error(
            Failure::GetBuffer,
            RenderError::Api {
                stage: "GetBuffer(render)",
                hresult: E_FAIL.0,
            },
        );
    }

    #[test]
    fn terminal_release_buffer_error_retains_callback_and_original_error() {
        assert_terminal_error(
            Failure::ReleaseBuffer,
            RenderError::Api {
                stage: "ReleaseBuffer(render)",
                hresult: E_FAIL.0,
            },
        );
    }

    #[test]
    fn terminal_submit_error_with_nonempty_padding_is_an_event_without_bridge() {
        let options = options();
        let mut report = new_report(&options);
        let mut renderer = DiagnosticRenderer {
            renders: 0,
            acknowledged: Cell::new(0),
            failure: Failure::GetBuffer,
            bridge: None,
        };
        assert!(run(&options, &mut report, &mut renderer, Failure::GetBuffer, 2).is_err());
        assert_eq!(report.diagnostics.events.len(), 1);
        let json = serde_json::to_value(report.diagnostics.events[0]).unwrap();
        assert_eq!(json["callback"]["failed"], true);
        assert_eq!(json["callback"]["padding_before_frames"], 2);
        assert_eq!(json["callback"]["demand_frames"], 2);
        assert_eq!(report.stats.submitted_frames, 4);
    }

    #[test]
    fn trace_correlates_device_clock_padding_and_successful_submissions() {
        let options = options();
        let mut report = new_report(&options);
        let mut renderer = DiagnosticRenderer {
            renders: 0,
            acknowledged: Cell::new(0),
            failure: Failure::Render,
            bridge: None,
        };
        assert!(run(&options, &mut report, &mut renderer, Failure::Render, 0).is_err());
        let json = serde_json::to_value(&report).unwrap();
        let callback = &json["diagnostics"]["events"][0]["callback"];
        assert_eq!(callback["submitted_frames_before"], 4);
        assert_eq!(callback["native_clock"]["position_units"], 1234);
        assert_eq!(callback["native_clock"]["qpc_100ns"], 5678);
        assert_eq!(callback["native_clock"]["hresult"], 0);
        assert_eq!(json["clock_frequency"], 384_000);
        assert_eq!(json["clock_hresult"], serde_json::Value::Null);
    }

    #[test]
    fn raw_clock_sample_keeps_low_accuracy_and_failure_hresult() {
        for hresult in [1, E_FAIL.0] {
            let clock: IAudioClock = DiagnosticClock { hresult }.into();
            let sample = clock_sample(&clock);
            assert_eq!(sample.hresult, hresult);
            if hresult == 1 {
                assert_eq!(sample.position_units, 1234);
                assert_eq!(sample.qpc_100ns, 5678);
            }
        }
    }
}
