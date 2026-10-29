use super::renderer::DemandRenderer;
use crate::live::render_request::PhysicalRenderRequest;
use anyhow::{Context, Result, anyhow, ensure};
use moiren_engine::{
    audio_config::SampleFormat,
    node::{DeviceMode, DeviceOptions},
};
use moiren_windows_audio::{
    Activation, ActivationGate, SessionDuration, StopSignal,
    latency::{
        CallbackObservation, DeviceClockObservation, LatencyRecorder, LatencySnapshot,
        NativeStreamTiming, PreparedRenderQueue, QpcClock, RenderQueuePolicy, TelemetryMode,
        render::{
            RenderCallback, RenderClockSample, RenderDiagnostics, RenderDiagnosticsSnapshot,
            RenderPaddingObserver,
        },
        shared_period::SharedPeriodReport,
    },
    native::{NativeStream, NativeStreamStats, StereoFrame},
    wasapi::{WasapiOptions, WasapiStream},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HANDLE,
    System::Threading::{AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW},
};

pub(in crate::live) struct RenderOptions {
    pub queue: RenderQueuePolicy,
    pub latency_telemetry: TelemetryMode,
    pub endpoint_id: String,
    pub duration: SessionDuration,
    pub physical_render_request: PhysicalRenderRequest,
}

impl RenderOptions {
    pub(in crate::live) fn from_live(
        options: &crate::live::LiveOptions,
        queue: RenderQueuePolicy,
        duration: SessionDuration,
    ) -> Self {
        Self {
            queue,
            latency_telemetry: options.telemetry,
            endpoint_id: options.output_endpoint.clone(),
            duration,
            physical_render_request: options.physical_render_request,
        }
    }

    fn device_options(&self) -> DeviceOptions {
        self.physical_render_request.apply(DeviceOptions {
            mode: DeviceMode::Shared,
            sample_format: SampleFormat::Float32,
            ..DeviceOptions::default()
        })
    }
}

#[derive(Clone, Copy)]
pub(in crate::live) struct NativeRenderInfo {
    pub timing: NativeStreamTiming,
    pub queue: PreparedRenderQueue,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RenderStatus {
    Completed,
    Stopped,
    Failed,
}

#[derive(Default, Serialize)]
pub(in crate::live) struct RenderStats {
    primed_frames: u32,
    submitted_frames: u64,
    processed_frames: u64,
    dsp_blocks: u64,
    dsp_segments: u64,
    audio_wakes: u64,
    timeout_wakes: u64,
    zero_demand_wakes: u64,
    bridge_shortfall_frames: u64,
    nonzero_samples: u64,
    peak_amplitude: f64,
}

#[derive(Serialize)]
pub(in crate::live) struct RenderReport {
    schema_version: u32,
    endpoint_id: String,
    status: RenderStatus,
    requested_seconds: Option<f64>,
    elapsed_seconds: f64,
    sample_rate: Option<u32>,
    channels: Option<usize>,
    timing: Option<NativeStreamTiming>,
    shared_period: Option<SharedPeriodReport>,
    physical_render_request: PhysicalRenderRequest,
    requested_queue: RenderQueuePolicy,
    queue: Option<PreparedRenderQueue>,
    stream_started: bool,
    stop_succeeded: bool,
    mmcss_registered: bool,
    mmcss_hresult: Option<i32>,
    stats: RenderStats,
    native_stats: Option<NativeStreamStats>,
    latency: LatencySnapshot,
    diagnostics: RenderDiagnosticsSnapshot,
    clock_frequency: Option<u64>,
    clock_frequency_hresult: Option<i32>,
    // NativeStream has no ducking contract; absence is distinct from success.
    own_session_ducking_opt_out: Option<bool>,
    primary_failure: Option<String>,
    stop_error: Option<String>,
    release_error: Option<String>,
    failure: Option<String>,
    terminal_callback: Option<CallbackEvidence>,
}

#[derive(Clone, Copy, Serialize)]
struct CallbackEvidence {
    wake_qpc_100ns: Option<u64>,
    completed_qpc_100ns: Option<u64>,
    demand_frames: Option<u32>,
    padding_before_frames: Option<u32>,
    submitted_frames_before: u64,
    device_clock: Option<DeviceClockObservation>,
    bridge_before: Option<moiren_windows_audio::clock_bridge::BridgeSnapshot>,
    bridge_after: Option<moiren_windows_audio::clock_bridge::BridgeSnapshot>,
}

fn complete_callback(
    evidence: CallbackEvidence,
    submitted: Result<usize>,
    telemetry: &mut LatencyRecorder,
    diagnostics: &mut RenderDiagnostics,
) -> Result<usize> {
    let written = submitted.as_ref().copied().unwrap_or(0);
    if let Some(demand) = evidence.demand_frames {
        telemetry.record_with_device_clock(
            CallbackObservation {
                wake_qpc_100ns: evidence.wake_qpc_100ns,
                completed_qpc_100ns: evidence.completed_qpc_100ns,
                demand_frames: u64::from(demand),
                padding_before_frames: evidence.padding_before_frames,
                submitted_frames: written as u64,
            },
            evidence.device_clock,
        );
        if let Some(padding_before_frames) = evidence.padding_before_frames {
            diagnostics.record(RenderCallback {
                wake_qpc_100ns: evidence.wake_qpc_100ns,
                demand_frames: demand,
                padding_before_frames,
                submitted_frames_before: evidence.submitted_frames_before,
                native_clock: evidence.device_clock.map(|clock| RenderClockSample {
                    position_units: clock.position_units,
                    qpc_100ns: clock.qpc_100ns,
                    hresult: clock.hresult,
                }),
                bridge_before: evidence.bridge_before,
                bridge_after: evidence.bridge_after,
                failed: submitted.is_err(),
                ..RenderCallback::default()
            });
        }
    }
    submitted
}

pub(in crate::live) struct RenderSession {
    stop: StopSignal,
    started: Arc<AtomicBool>,
    worker: Option<JoinHandle<OwnerExit>>,
}
pub(in crate::live) struct PreparedNativeRender {
    session: Option<RenderSession>,
    renderer: Option<SyncSender<DemandRenderer>>,
    info: NativeRenderInfo,
    padding: Option<RenderPaddingObserver>,
    queue: RenderQueuePolicy,
}
pub(in crate::live) struct RenderOwnerExit {
    pub report: RenderReport,
    pub renderer: DemandRenderer,
}
struct OwnerExit {
    report: RenderReport,
    renderer: Option<DemandRenderer>,
    // Only the control joiner destroys graph resources and prepared buffers.
    _staging: Vec<StereoFrame>,
    _scratch: Vec<f32>,
}
pub(in crate::live) struct RenderPreparationFailure<A> {
    pub error: anyhow::Error,
    pub stop_error: Option<anyhow::Error>,
    pub join_error: Option<anyhow::Error>,
    pub report: Option<RenderReport>,
    pub abort: Option<A>,
    pub abort_panicked: bool,
}
pub(in crate::live) struct RenderAttachFailure<A> {
    pub error: anyhow::Error,
    pub renderer: DemandRenderer,
    pub stop_error: Option<anyhow::Error>,
    pub join_error: Option<anyhow::Error>,
    pub report: Option<RenderReport>,
    pub abort: Option<A>,
    pub abort_panicked: bool,
}

fn failure_evidence<A: Serialize>(
    stage: &str,
    error: &anyhow::Error,
    stop_error: &Option<anyhow::Error>,
    join_error: &Option<anyhow::Error>,
    abort: &Option<A>,
    abort_panicked: bool,
) -> Value {
    json!({
        "stage": stage,
        "error": format!("{error:#}"),
        "stop_error": stop_error.as_ref().map(|error| format!("{error:#}")),
        "join_error": join_error.as_ref().map(|error| format!("{error:#}")),
        "abort": abort,
        "abort_panicked": abort_panicked,
    })
}
impl<A: Serialize> RenderPreparationFailure<A> {
    pub(in crate::live) fn evidence(&self, stage: &str) -> Value {
        failure_evidence(
            stage,
            &self.error,
            &self.stop_error,
            &self.join_error,
            &self.abort,
            self.abort_panicked,
        )
    }
}
impl<A: Serialize> RenderAttachFailure<A> {
    pub(in crate::live) fn evidence(&self, stage: &str) -> Value {
        failure_evidence(
            stage,
            &self.error,
            &self.stop_error,
            &self.join_error,
            &self.abort,
            self.abort_panicked,
        )
    }
}

fn abort<A>(before_join: &mut impl FnMut() -> A) -> (Option<A>, bool) {
    match catch_unwind(AssertUnwindSafe(before_join)) {
        Ok(value) => (Some(value), false),
        Err(_) => (None, true),
    }
}

pub(in crate::live) fn prepare_render_native_with_gate_returning_with_abort<A>(
    options: RenderOptions,
    stop: StopSignal,
    gate: ActivationGate,
    mut before_join: impl FnMut() -> A,
) -> std::result::Result<PreparedNativeRender, Box<RenderPreparationFailure<A>>> {
    let mut session = None;
    let result = (|| -> Result<PreparedNativeRender> {
        ensure!(
            !options.endpoint_id.trim().is_empty() && !options.endpoint_id.contains('\0'),
            "render requires an explicit endpoint"
        );
        options.queue.validate()?;
        options.physical_render_request.validate(options.queue)?;
        ensure!(
            options.duration.validate().is_ok(),
            "invalid render duration"
        );
        let queue = options.queue;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (renderer_tx, renderer_rx) = mpsc::sync_channel(1);
        let started = Arc::new(AtomicBool::new(false));
        let worker_started = started.clone();
        let worker = thread::Builder::new()
            .name("e2e-native-render".into())
            .spawn(move || owner(options, gate, worker_started, ready_tx, renderer_rx))
            .context("spawn native render owner")?;
        session = Some(RenderSession {
            stop: stop.clone(),
            started,
            worker: Some(worker),
        });
        let (info, padding) = ready_rx
            .recv()
            .context("render owner lost before preparation")?
            .map_err(|error| anyhow!(error))?;
        Ok(PreparedNativeRender {
            session: session.take(),
            renderer: Some(renderer_tx),
            info,
            padding,
            queue,
        })
    })();
    match result {
        Ok(prepared) => Ok(prepared),
        Err(error) => {
            // Source/recorder peers receive cancellation before a native join,
            // even if they are stalled in their own initialization.
            let (abort, abort_panicked) = abort(&mut before_join);
            let stop_error = stop.request_stop().err().map(Into::into);
            let mut report = None;
            let mut join_error = None;
            if let Some(mut session) = session {
                match session.join_exit() {
                    Ok(exit) => report = Some(exit.report),
                    Err(error) => join_error = Some(error),
                }
            }
            Err(Box::new(RenderPreparationFailure {
                error,
                stop_error,
                join_error,
                report,
                abort,
                abort_panicked,
            }))
        }
    }
}

impl PreparedNativeRender {
    pub(in crate::live) fn info(&self) -> NativeRenderInfo {
        self.info
    }
    pub(in crate::live) fn padding_observer(&self) -> Option<RenderPaddingObserver> {
        self.padding.clone()
    }
    pub(in crate::live) fn attach_returning_with_abort<A>(
        mut self,
        renderer: DemandRenderer,
        queue: RenderQueuePolicy,
        mut before_join: impl FnMut() -> A,
    ) -> std::result::Result<RenderSession, Box<RenderAttachFailure<A>>> {
        let failure = if queue != self.queue {
            (
                anyhow!("render queue changed after native preparation"),
                renderer,
            )
        } else {
            match self
                .renderer
                .take()
                .expect("prepared renderer sender")
                .send(renderer)
            {
                Ok(()) => return Ok(self.session.take().expect("prepared native session")),
                Err(error) => (
                    anyhow!("native owner stopped before graph attachment"),
                    error.0,
                ),
            }
        };
        let (abort, abort_panicked) = abort(&mut before_join);
        self.renderer.take();
        let mut session = self.session.take().expect("prepared native session");
        let stop_error = session.stop.request_stop().err().map(Into::into);
        let (report, join_error) = match session.join_exit() {
            Ok(exit) => (Some(exit.report), None),
            Err(error) => (None, Some(error)),
        };
        Err(Box::new(RenderAttachFailure {
            error: failure.0,
            renderer: failure.1,
            stop_error,
            join_error,
            report,
            abort,
            abort_panicked,
        }))
    }
    pub(in crate::live) fn cancel_retaining_report(mut self) -> (Option<RenderReport>, Result<()>) {
        // The parked owner cannot finish until attachment is disconnected.
        // Stop failure must not suppress its join or discard its final report.
        self.renderer.take();
        let mut session = self.session.take().expect("prepared native session");
        let stopped = session
            .stop
            .request_stop()
            .context("cancel prepared render");
        let joined = session.join_exit().map(|exit| exit.report);
        retain_cancelled_report(stopped, joined)
    }
}

fn retain_cancelled_report<T>(stopped: Result<()>, joined: Result<T>) -> (Option<T>, Result<()>) {
    let (report, joined) = match joined {
        Ok(report) => (Some(report), Ok(())),
        Err(error) => (None, Err(error)),
    };
    (report, crate::live::soak_cleanup::combine(stopped, joined))
}

#[cfg(test)]
impl PreparedNativeRender {
    /// Exercise real attachment/cancellation ownership without opening COM/audio.
    pub(in crate::live) fn parked_fixture(
        info: NativeRenderInfo,
        stop: StopSignal,
        requested_queue: RenderQueuePolicy,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<DemandRenderer>(1);
        let report = RenderReport {
            schema_version: 3,
            endpoint_id: "parked-no-native-owner".into(),
            status: RenderStatus::Stopped,
            requested_seconds: None,
            elapsed_seconds: 0.0,
            sample_rate: Some(info.timing.sample_rate),
            channels: Some(2),
            timing: Some(info.timing),
            shared_period: None,
            physical_render_request: PhysicalRenderRequest::default(),
            requested_queue,
            queue: Some(info.queue),
            stream_started: false,
            stop_succeeded: true,
            mmcss_registered: false,
            mmcss_hresult: None,
            stats: RenderStats::default(),
            native_stats: None,
            latency: LatencyRecorder::new(TelemetryMode::Disabled).snapshot(),
            diagnostics: RenderDiagnosticsSnapshot::default(),
            clock_frequency: None,
            clock_frequency_hresult: None,
            own_session_ducking_opt_out: None,
            primary_failure: None,
            stop_error: None,
            release_error: None,
            failure: None,
            terminal_callback: None,
        };
        let worker = thread::spawn(move || {
            let renderer = match receiver.recv_timeout(Duration::from_secs(2)) {
                Ok(renderer) => Some(renderer),
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("parked owner joined before attachment was disconnected")
                }
            };
            OwnerExit {
                report,
                renderer,
                _staging: Vec::new(),
                _scratch: Vec::new(),
            }
        });
        Self {
            session: Some(RenderSession {
                stop,
                started: Arc::new(AtomicBool::new(false)),
                worker: Some(worker),
            }),
            renderer: Some(sender),
            info,
            padding: Some(RenderPaddingObserver::new()),
            queue: requested_queue,
        }
    }
}

impl Drop for PreparedNativeRender {
    fn drop(&mut self) {
        // Disconnect attachment before RenderSession::drop joins a parked owner.
        self.renderer.take();
        self.session.take();
    }
}
impl RenderSession {
    pub(in crate::live) fn into_session(self) -> Self {
        self
    }
    pub(in crate::live) fn has_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
    pub(in crate::live) fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
    fn join_exit(&mut self) -> Result<OwnerExit> {
        self.worker
            .take()
            .context("render owner already joined")?
            .join()
            .map_err(|_| anyhow!("render owner panicked outside its streaming unwind boundary"))
    }
    pub(in crate::live) fn join(mut self) -> Result<RenderReport> {
        Ok(self.join_exit()?.report)
    }
    pub(in crate::live) fn join_with_renderer(mut self) -> Result<RenderOwnerExit> {
        let exit = self.join_exit()?;
        Ok(RenderOwnerExit {
            report: exit.report,
            renderer: exit
                .renderer
                .context("attached renderer missing after join")?,
        })
    }
}
impl Drop for RenderSession {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.stop.request_stop();
            let _ = self.join_exit();
        }
    }
}

struct Mmcss(HANDLE);
impl Drop for Mmcss {
    fn drop(&mut self) {
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}
type PreparedRender = (NativeRenderInfo, Option<RenderPaddingObserver>);
type Ready = std::result::Result<PreparedRender, String>;

impl RenderReport {
    fn before_open(options: &RenderOptions) -> Self {
        Self {
            schema_version: 3,
            endpoint_id: options.endpoint_id.clone(),
            status: RenderStatus::Failed,
            requested_seconds: options.duration.requested_seconds(),
            elapsed_seconds: 0.0,
            sample_rate: None,
            channels: None,
            timing: None,
            shared_period: None,
            physical_render_request: options.physical_render_request,
            requested_queue: options.queue,
            queue: None,
            stream_started: false,
            stop_succeeded: false,
            mmcss_registered: false,
            mmcss_hresult: None,
            stats: RenderStats::default(),
            native_stats: None,
            latency: LatencyRecorder::new(options.latency_telemetry).snapshot(),
            diagnostics: RenderDiagnosticsSnapshot::default(),
            clock_frequency: None,
            clock_frequency_hresult: None,
            own_session_ducking_opt_out: None,
            primary_failure: None,
            stop_error: None,
            release_error: None,
            failure: None,
            terminal_callback: None,
        }
    }
}

fn prepare_render_stream(
    options: &RenderOptions,
    report: &mut RenderReport,
    stream: &mut dyn NativeStream,
) -> Result<PreparedRender> {
    let info = stream.info();
    let timing = stream.prepared_timing();
    report.sample_rate = Some(info.sample_rate);
    report.channels = Some(info.channels);
    report.timing = Some(timing);
    report.shared_period = stream.shared_period();
    options
        .physical_render_request
        .validate_observed(timing, report.shared_period)?;
    if let Some(clock) = stream.device_clock_observation() {
        report.clock_frequency = clock.frequency_hz;
        report.clock_frequency_hresult = clock.frequency_hresult;
    }
    // Retain negotiated timing and the attempted policy if the native
    // adapter refuses its deadline budget before any audio is started.
    stream.configure_render_queue(options.queue)?;
    ensure!(
        info.sample_rate == 48_000 && info.channels == 2 && info.buffer_frames > 0,
        "probe render requires negotiated 48k stereo"
    );
    let queue = stream
        .render_queue()
        .context("native render queue unavailable")?;
    let padding = stream.render_padding_observer();
    ensure!(
        padding.is_some() || options.device_options().mode == DeviceMode::Exclusive,
        "native Shared padding observer unavailable"
    );
    report.queue = Some(queue);
    Ok((NativeRenderInfo { timing, queue }, padding))
}

fn owner(
    options: RenderOptions,
    gate: ActivationGate,
    acknowledged: Arc<AtomicBool>,
    ready: SyncSender<Ready>,
    attachment: mpsc::Receiver<DemandRenderer>,
) -> OwnerExit {
    let mut ready = Some(ready);
    let mut stream = None;
    let mut renderer = None;
    let mut staging = Vec::new();
    let mut scratch = Vec::new();
    let mut telemetry = LatencyRecorder::new(options.latency_telemetry);
    let mut diagnostics = None;
    let mut report = RenderReport::before_open(&options);
    // Keep graph and scratch outside this unwind scope. A callback failure
    // stops/releases native services here and returns DSP ownership to control.
    let run = catch_unwind(AssertUnwindSafe(|| -> Result<RenderStatus> {
        stream = Some(WasapiStream::open(WasapiOptions {
            endpoint_id: options.endpoint_id.clone(),
            sample_rate: 48_000,
            device: options.device_options(),
            input: false,
        })?);
        let stream = stream.as_mut().expect("opened native stream");
        let (prepared, padding) = prepare_render_stream(&options, &mut report, stream)?;
        let timing = prepared.timing;
        let queue = prepared.queue;
        let info = stream.info();
        let frames = usize::try_from(info.buffer_frames)?;
        ensure!(
            frames <= (8 * 1024 * 1024) / size_of::<StereoFrame>(),
            "native staging exceeds the probe budget"
        );
        staging.resize(frames, [0.0; 2]);
        let wake_budget =
            (u64::from(queue.target_frames) * 10_000_000).div_ceil(u64::from(info.sample_rate));
        diagnostics = Some(RenderDiagnostics::new(
            options.latency_telemetry,
            wake_budget,
        ));
        ready
            .take()
            .expect("startup sender")
            .send(Ok((NativeRenderInfo { timing, queue }, padding.clone())))
            .map_err(|_| anyhow!("native preparation receiver lost"))?;
        let Ok(attached) = attachment.recv() else {
            return Ok(RenderStatus::Stopped);
        };
        renderer = Some(attached);
        let renderer = renderer.as_mut().expect("attached graph");
        scratch.resize(renderer.scratch_samples(), 0.0);
        let clock = QpcClock::prepare(options.latency_telemetry);
        if gate.wait()? == Activation::Cancelled {
            return Ok(RenderStatus::Stopped);
        }
        let mut index = 0;
        let mut mmcss_index = 0;
        let _mmcss = match unsafe {
            AvSetMmThreadCharacteristicsW(windows::core::w!("Audio"), &mut mmcss_index)
        } {
            Ok(handle) => {
                report.mmcss_registered = true;
                Some(Mmcss(handle))
            }
            Err(error) => {
                report.mmcss_hresult = Some(error.code().0);
                None
            }
        };
        // Prime with graph audio, exactly as the supervised path renders later.
        let mut pending = 0;
        let prime_wake = clock.and_then(|clock| clock.now());
        let prime_demand = stream.render_demand();
        let mut prime_evidence = CallbackEvidence {
            wake_qpc_100ns: prime_wake,
            completed_qpc_100ns: None,
            demand_frames: prime_demand
                .as_ref()
                .ok()
                .and_then(|frames| u32::try_from(*frames).ok()),
            padding_before_frames: padding
                .as_ref()
                .and_then(|observer| observer.try_snapshot())
                .map(|sample| sample.padding_frames),
            submitted_frames_before: report.stats.submitted_frames,
            device_clock: stream.device_clock_observation().map(|clock| clock.sample),
            bridge_before: renderer.bridge_snapshot(),
            bridge_after: None,
        };
        let prime = (|| -> Result<usize> {
            pending = prime_demand?;
            ensure!(
                pending <= staging.len(),
                "native prime demand exceeds staging"
            );
            renderer.render(&mut staging[..pending], &mut scratch)?;
            submit_pending(
                stream,
                &staging,
                &mut index,
                pending,
                pending,
                &mut report.stats,
            )
        })();
        prime_evidence.completed_qpc_100ns = clock.and_then(|clock| clock.now());
        prime_evidence.bridge_after = renderer.bridge_snapshot();
        if prime.is_err() {
            report.terminal_callback = Some(prime_evidence);
        }
        let written = complete_callback(
            prime_evidence,
            prime,
            &mut telemetry,
            diagnostics.as_mut().expect("prepared diagnostics"),
        )?;
        report.stats.primed_frames = written as u32;
        stream.start()?;
        report.stream_started = true;
        acknowledged.store(true, Ordering::Release);
        let started = Instant::now();
        loop {
            report.elapsed_seconds = started.elapsed().as_secs_f64();
            if gate.wait()? == Activation::Cancelled {
                return Ok(RenderStatus::Stopped);
            }
            let remaining = options.duration.remaining(started.elapsed());
            if remaining == Some(Duration::ZERO) {
                return Ok(RenderStatus::Completed);
            }
            let timeout = remaining
                .unwrap_or(Duration::from_millis(50))
                .min(Duration::from_millis(50));
            if !stream.wait(timeout)? {
                report.stats.timeout_wakes += 1;
                continue;
            }
            report.stats.audio_wakes += 1;
            let wake = clock.and_then(|clock| clock.now());
            let demand = stream.render_demand();
            let mut evidence = CallbackEvidence {
                wake_qpc_100ns: wake,
                completed_qpc_100ns: None,
                demand_frames: demand
                    .as_ref()
                    .ok()
                    .and_then(|frames| u32::try_from(*frames).ok()),
                padding_before_frames: if demand.is_ok() {
                    padding
                        .as_ref()
                        .and_then(|observer| observer.try_snapshot())
                        .map(|sample| sample.padding_frames)
                } else {
                    None
                },
                submitted_frames_before: report.stats.submitted_frames,
                device_clock: stream.device_clock_observation().map(|clock| clock.sample),
                bridge_before: renderer.bridge_snapshot(),
                bridge_after: None,
            };
            let submitted = (|| -> Result<usize> {
                let demand = demand?;
                ensure!(
                    demand <= staging.len(),
                    "native demand exceeds prepared staging"
                );
                if demand == 0 {
                    report.stats.zero_demand_wakes += 1;
                }
                if index == pending {
                    index = 0;
                    pending = demand;
                    renderer.render(&mut staging[..pending], &mut scratch)?;
                }
                submit_pending(
                    stream,
                    &staging,
                    &mut index,
                    pending,
                    demand,
                    &mut report.stats,
                )
            })();
            evidence.completed_qpc_100ns = clock.and_then(|clock| clock.now());
            evidence.bridge_after = renderer.bridge_snapshot();
            if submitted.is_err() {
                report.terminal_callback = Some(evidence);
            }
            complete_callback(
                evidence,
                submitted,
                &mut telemetry,
                diagnostics.as_mut().expect("prepared diagnostics"),
            )?;
        }
    }));
    match run {
        Ok(Ok(status)) => report.status = status,
        Ok(Err(error)) => report.primary_failure = Some(format!("{error:#}")),
        Err(_) => report.primary_failure = Some("native render owner panicked".into()),
    }
    report.failure = report.primary_failure.clone();
    if let Some(stream) = &mut stream {
        // All native teardown stays on the opening COM owner, even on failure.
        if let Err(error) = stream.stop() {
            report.stop_error = Some(error.to_string());
            append_failure(&mut report, format!("native Stop: {error}"));
        } else {
            report.stop_succeeded = true;
        }
        report.native_stats = Some(stream.stats());
        if let Err(error) = stream.release() {
            report.release_error = Some(error.to_string());
            append_failure(&mut report, format!("native release: {error}"));
        }
    }
    if let Some(renderer) = &renderer {
        let counters = renderer.counters();
        report.stats.processed_frames = counters.frames;
        report.stats.dsp_blocks = counters.blocks;
        report.stats.dsp_segments = counters.segments;
        report.stats.bridge_shortfall_frames = counters.shortfall_frames;
    }
    report.latency = telemetry.snapshot();
    if let Some(diagnostics) = diagnostics {
        report.diagnostics = diagnostics.finish();
    }
    if let Some(ready) = ready {
        let _ = ready.send(Err(report
            .failure
            .clone()
            .unwrap_or_else(|| "render preparation stopped".into())));
    }
    OwnerExit {
        report,
        renderer,
        _staging: staging,
        _scratch: scratch,
    }
}

fn append_failure(report: &mut RenderReport, failure: String) {
    report.status = RenderStatus::Failed;
    if let Some(old) = &mut report.failure {
        old.push_str("; ");
        old.push_str(&failure);
    } else {
        report.failure = Some(failure);
    }
}

fn record_samples(frames: &[StereoFrame], stats: &mut RenderStats) {
    stats.submitted_frames += frames.len() as u64;
    for sample in frames.iter().flatten() {
        stats.nonzero_samples += u64::from(*sample != 0.0);
        stats.peak_amplitude = stats.peak_amplitude.max(sample.abs());
    }
}

fn submit_pending(
    stream: &mut dyn NativeStream,
    staging: &[StereoFrame],
    index: &mut usize,
    pending: usize,
    demand: usize,
    stats: &mut RenderStats,
) -> Result<usize> {
    // Preserve the unwritten suffix across short writes. Never render it
    // again, skip graph frames, or exceed the adapter's current demand.
    let end = *index + (pending - *index).min(demand);
    if end == *index {
        return Ok(0);
    }
    let written = stream.write(&staging[*index..end])?;
    ensure!(
        written <= end - *index,
        "native write exceeded requested audio"
    );
    record_samples(&staging[*index..*index + written], stats);
    *index += written;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_engine::host::BackendError;
    use moiren_windows_audio::native::NativeStreamInfo;

    fn physical_options(mode: DeviceMode) -> RenderOptions {
        RenderOptions {
            queue: RenderQueuePolicy::FullCapacity,
            latency_telemetry: TelemetryMode::Disabled,
            endpoint_id: "selected render".into(),
            duration: SessionDuration::UntilStopped,
            physical_render_request: PhysicalRenderRequest {
                mode: Some(mode),
                period_frames: Some(128),
            },
        }
    }

    struct PreparedFixture {
        timing: NativeStreamTiming,
        shared: Option<SharedPeriodReport>,
        padding: Option<RenderPaddingObserver>,
        configured: bool,
    }

    impl NativeStream for PreparedFixture {
        fn info(&self) -> NativeStreamInfo {
            NativeStreamInfo {
                sample_rate: self.timing.sample_rate,
                format: SampleFormat::Float32,
                buffer_frames: self.timing.capacity_frames,
                channels: 2,
            }
        }
        fn prepared_timing(&self) -> NativeStreamTiming {
            self.timing
        }
        fn shared_period(&self) -> Option<SharedPeriodReport> {
            self.shared
        }
        fn configure_render_queue(
            &mut self,
            _: RenderQueuePolicy,
        ) -> std::result::Result<(), BackendError> {
            self.configured = true;
            Ok(())
        }
        fn render_queue(&self) -> Option<PreparedRenderQueue> {
            self.configured.then_some(PreparedRenderQueue {
                capacity_frames: self.timing.capacity_frames,
                target_frames: self.timing.capacity_frames,
                deadline_budget_frames: None,
            })
        }
        fn render_padding_observer(&self) -> Option<RenderPaddingObserver> {
            self.padding.clone()
        }
        fn start(&mut self) -> std::result::Result<(), BackendError> {
            panic!("preparation must not Start")
        }
        fn stop(&mut self) -> std::result::Result<(), BackendError> {
            unreachable!()
        }
        fn wait(&mut self, _: Duration) -> std::result::Result<bool, BackendError> {
            unreachable!()
        }
        fn read(&mut self, _: &mut [StereoFrame]) -> std::result::Result<usize, BackendError> {
            unreachable!()
        }
        fn write(&mut self, _: &[StereoFrame]) -> std::result::Result<usize, BackendError> {
            panic!("preparation must not write")
        }
    }

    fn prepared_fixture(mode: DeviceMode) -> PreparedFixture {
        PreparedFixture {
            timing: NativeStreamTiming {
                sample_rate: 48_000,
                period_frames: Some(128),
                capacity_frames: 128,
            },
            shared: (mode == DeviceMode::Shared).then_some(SharedPeriodReport {
                requested_period_frames: Some(128),
                actual_period_frames: Some(128),
                initialization_api: Some(
                    moiren_windows_audio::latency::shared_period::SharedInitializationApi::Client3,
                ),
                ..Default::default()
            }),
            padding: (mode == DeviceMode::Shared).then(RenderPaddingObserver::new),
            configured: false,
        }
    }

    #[test]
    fn physical_render_owner_maps_live_request_for_both_process_paths_without_default_drift() {
        let live = crate::live::LiveOptions {
            output_endpoint: "exact selected endpoint".into(),
            physical_render_request: PhysicalRenderRequest {
                mode: Some(DeviceMode::Exclusive),
                period_frames: Some(128),
            },
            ..Default::default()
        };
        for duration in [
            SessionDuration::UntilStopped,
            SessionDuration::For(Duration::from_secs(2)),
        ] {
            let options =
                RenderOptions::from_live(&live, RenderQueuePolicy::FullCapacity, duration);
            assert_eq!(
                options.physical_render_request,
                live.physical_render_request
            );
            assert_eq!(options.endpoint_id, live.output_endpoint);
            assert_eq!(options.duration, duration);
            assert_eq!(
                options.device_options(),
                DeviceOptions {
                    mode: DeviceMode::Exclusive,
                    period_frames: Some(128),
                    sample_format: SampleFormat::Float32,
                    ..Default::default()
                }
            );
        }
        let omitted = RenderOptions::from_live(
            &crate::live::LiveOptions::default(),
            RenderQueuePolicy::FullCapacity,
            SessionDuration::UntilStopped,
        );
        assert_eq!(
            omitted.device_options(),
            DeviceOptions {
                mode: DeviceMode::Shared,
                sample_format: SampleFormat::Float32,
                ..Default::default()
            }
        );
    }

    #[test]
    fn physical_render_owner_exclusive_admits_without_fabricating_padding() {
        let options = physical_options(DeviceMode::Exclusive);
        let mut report = RenderReport::before_open(&options);
        let mut stream = prepared_fixture(DeviceMode::Exclusive);
        let (info, padding) = prepare_render_stream(&options, &mut report, &mut stream).unwrap();
        assert!(padding.is_none());
        assert_eq!(info.timing, stream.timing);
        assert_eq!(info.queue.target_frames, 128);
        assert!(stream.configured);
        assert!(!report.stream_started);
        assert!(report.shared_period.is_none());
    }

    #[test]
    fn physical_render_owner_refusal_keeps_requested_and_negotiated_evidence_before_queue_or_start()
    {
        use moiren_windows_audio::latency::shared_period::SharedFallbackReason;
        let options = physical_options(DeviceMode::Shared);
        let mut report = RenderReport::before_open(&options);
        assert_eq!(
            report.physical_render_request,
            options.physical_render_request
        );
        assert!(report.timing.is_none());
        let mut stream = prepared_fixture(DeviceMode::Shared);
        stream.shared.as_mut().unwrap().fallback_reason =
            Some(SharedFallbackReason::WindowsFormatConversion);
        assert!(prepare_render_stream(&options, &mut report, &mut stream).is_err());
        assert_eq!(report.timing, Some(stream.timing));
        assert_eq!(report.shared_period, stream.shared);
        assert!(!stream.configured);
        assert!(!report.stream_started);
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["physical_render_request"]["period_frames"], 128);
        assert_eq!(
            value["shared_period"]["fallback_reason"],
            "windows_format_conversion"
        );
    }

    #[test]
    fn cancelled_report_survives_stop_failure() {
        let (report, result) =
            retain_cancelled_report(Err(anyhow!("injected stop failure")), Ok(42));
        assert_eq!(report, Some(42));
        assert!(format!("{:#}", result.unwrap_err()).contains("injected stop failure"));
    }

    #[test]
    fn cancelled_report_keeps_stop_and_join_failures() {
        let (report, result) = retain_cancelled_report::<u64>(
            Err(anyhow!("injected stop failure").context("cancel prepared render")),
            Err(anyhow!("injected join failure").context("join prepared render")),
        );
        assert_eq!(report, None);
        assert_eq!(
            format!("{:#}", result.unwrap_err()),
            "cancel prepared render: injected stop failure; join prepared render: injected join failure"
        );
    }

    #[test]
    fn cancelled_report_preserves_join_failure_without_inventing_report() {
        let (report, result) =
            retain_cancelled_report::<u64>(Ok(()), Err(anyhow!("injected join failure")));
        assert_eq!(report, None);
        assert_eq!(result.unwrap_err().to_string(), "injected join failure");
    }

    #[test]
    fn cancelled_report_preserves_successful_report() {
        let (report, result) = retain_cancelled_report(Ok(()), Ok(42));
        assert_eq!(report, Some(42));
        result.unwrap();
    }

    #[test]
    fn cancelled_report_disconnects_real_parked_owner_before_join() {
        let timing = NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(48),
            capacity_frames: 512,
        };
        let queue = PreparedRenderQueue {
            capacity_frames: 512,
            target_frames: 256,
            deadline_budget_frames: Some(96),
        };
        let prepared = PreparedNativeRender::parked_fixture(
            NativeRenderInfo { timing, queue },
            StopSignal::new().unwrap(),
            RenderQueuePolicy::FullCapacity,
        );
        let (report, result) = prepared.cancel_retaining_report();
        result.unwrap();
        let report = report.expect("parked owner report must survive cancellation");
        assert_eq!(report.endpoint_id, "parked-no-native-owner");
        assert_eq!(report.queue, Some(queue));
        assert!(!report.stream_started);
    }

    struct ShortWrites {
        received: Vec<StereoFrame>,
        maximum: usize,
    }
    impl NativeStream for ShortWrites {
        fn info(&self) -> NativeStreamInfo {
            NativeStreamInfo {
                sample_rate: 48_000,
                format: SampleFormat::Float32,
                buffer_frames: 8,
                channels: 2,
            }
        }
        fn start(&mut self) -> std::result::Result<(), BackendError> {
            unreachable!()
        }
        fn stop(&mut self) -> std::result::Result<(), BackendError> {
            unreachable!()
        }
        fn wait(&mut self, _: Duration) -> std::result::Result<bool, BackendError> {
            unreachable!()
        }
        fn read(&mut self, _: &mut [StereoFrame]) -> std::result::Result<usize, BackendError> {
            unreachable!()
        }
        fn write(&mut self, input: &[StereoFrame]) -> std::result::Result<usize, BackendError> {
            let count = input.len().min(self.maximum);
            self.received.extend_from_slice(&input[..count]);
            Ok(count)
        }
    }

    #[test]
    fn short_native_writes_preserve_audio_order_and_respect_each_new_demand() {
        let frames = [[1.0; 2], [2.0; 2], [3.0; 2], [4.0; 2], [5.0; 2]];
        let mut stream = ShortWrites {
            received: Vec::new(),
            maximum: 2,
        };
        let mut index = 0;
        let mut stats = RenderStats::default();
        for (demand, expected_written) in [(5, 2), (1, 1), (0, 0), (5, 2)] {
            assert_eq!(
                submit_pending(
                    &mut stream,
                    &frames,
                    &mut index,
                    frames.len(),
                    demand,
                    &mut stats
                )
                .unwrap(),
                expected_written
            );
        }
        assert_eq!(stream.received, frames);
        assert_eq!(index, frames.len());
        assert_eq!(stats.submitted_frames, frames.len() as u64);
        assert_eq!(stats.nonzero_samples, 10);
    }

    #[test]
    fn native_backpressure_does_not_consume_pending_graph_audio() {
        let frames = [[1.0; 2]; 4];
        let mut stream = ShortWrites {
            received: Vec::new(),
            maximum: 0,
        };
        let mut index = 0;
        let mut stats = RenderStats::default();
        assert_eq!(
            submit_pending(&mut stream, &frames, &mut index, 4, 4, &mut stats).unwrap(),
            0
        );
        assert_eq!(index, 0);
        stream.maximum = 4;
        assert_eq!(
            submit_pending(&mut stream, &frames, &mut index, 4, 4, &mut stats).unwrap(),
            4
        );
        assert_eq!(stream.received, frames);
    }
}
