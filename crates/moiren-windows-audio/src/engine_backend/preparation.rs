use super::{
    WindowsBackendConfig,
    owner::WindowsTransport,
    resources::{ResourceSelector, Resources},
    selectors,
};
use crate::{
    ActivationGate, SessionDuration, StopSignal,
    capture::{CaptureError, CaptureOptions, CaptureSession, prepare_capture_native_with_gate},
    latency::prepare_latency_budget,
    process_loopback::{
        ProcessLoopbackOptions, inspect_process, prepare_process_capture_with_gate,
    },
    render::{RenderError, RenderOptions, prepare_render_native_with_gate},
    shared_stream::SharedPeriodPolicy,
};
use moiren_engine::{
    boundary::{BoundaryReport, CaptureIntent, ClockRole, RtAudioSource, audio_bridge},
    buffer::AudioBlockMut,
    compiler::NodeBindings,
    host::{
        BackendError, CancellationToken, EngineBackend, NodeConfig, PrepareMode, PrepareRequest,
        PreparedBackend, ResourceEvent,
    },
    node::{DeviceMode, DeviceOptions, InputSource, OutputTarget},
    processor::ProcessContext,
};
use std::sync::{Arc, Mutex};

pub(super) type Diagnostics = Arc<Mutex<Option<String>>>;

pub struct WindowsBackend {
    config: WindowsBackendConfig,
    pub(super) resources: Arc<Resources>,
    pub(super) diagnostics: Diagnostics,
}

impl WindowsBackend {
    pub fn new(config: WindowsBackendConfig) -> Result<Self, BackendError> {
        config.validate()?;
        Ok(Self {
            config,
            resources: Arc::new(Resources::new(config)?),
            diagnostics: Arc::new(Mutex::new(None)),
        })
    }

    pub fn last_native_report(&self) -> Option<String> {
        self.diagnostics
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    fn prepare_inner(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<f32>, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        validate_project(&request)?;
        if matches!(self.config.latency_budget, crate::latency::LatencyBudgetPolicy::Custom { .. })
            && request.project.nodes.values().any(|config| matches!(config, NodeConfig::Input(input) if matches!(input.source, InputSource::Application { .. })))
        { return Err(BackendError::Unsupported("custom input budgets require physical capture negotiation".into())); }
        self.resources
            .track(request.project, request.resource_generation)?;
        let mut bindings = NodeBindings::new();
        if request.mode == PrepareMode::Update {
            for (&node, config) in &request.project.nodes {
                if request.reuse.iter().any(|&(reused, _)| reused == node) {
                    continue;
                }
                match config {
                    NodeConfig::Input(input) => match &input.source {
                        InputSource::Software { name } => bindings
                            .bind_source(node, Tone::new(selectors::tone(name)?))
                            .map_err(|error| BackendError::Unsupported(error.to_string()))?,
                        _ => {
                            return Err(BackendError::Unsupported(
                                "native input changes require owner replacement".into(),
                            ));
                        }
                    },
                    NodeConfig::Output(_) => {
                        return Err(BackendError::Unsupported(
                            "native output changes require owner replacement".into(),
                        ));
                    }
                    _ => {}
                }
            }
            return Ok(PreparedBackend {
                bindings,
                transport: None,
            });
        }
        let (sink, output_config) = request
            .project
            .nodes
            .iter()
            .find_map(|(&node, config)| match config {
                NodeConfig::Output(output) => Some((node, output)),
                _ => None,
            })
            .ok_or_else(|| BackendError::Unsupported("native project requires an output".into()))?;
        let OutputTarget::Physical(endpoint) = &output_config.target else {
            return Err(BackendError::Unsupported(
                "software outputs have no Windows render transport".into(),
            ));
        };
        let endpoint_id = self.resources.resolve(
            sink,
            ResourceSelector::Endpoint {
                endpoint: endpoint.clone(),
                capture: false,
            },
        )?;
        let stop = StopSignal::new().map_err(|error| BackendError::Failed(error.to_string()))?;
        let cancellation_stop = stop.clone();
        cancel.on_cancel(move || {
            let _ = cancellation_stop.request_stop();
        })?;
        let gate = ActivationGate::new(stop.clone())
            .map_err(|error| BackendError::Failed(error.to_string()))?;
        let native = prepare_render_native_with_gate(
            RenderOptions {
                endpoint_id,
                duration: SessionDuration::UntilStopped,
                shared_period: period(output_config.device),
                queue: self.config.render_queue,
                latency_telemetry: self.config.telemetry,
            },
            stop.clone(),
            gate.clone(),
        )
        .map_err(render_error)?;
        let output_timing = native.info().timing;
        let mut group = PreparationGroup {
            stop: stop.clone(),
            captures: Vec::new(),
        };
        for (&node, config) in &request.project.nodes {
            let NodeConfig::Input(input) = config else {
                continue;
            };
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            match &input.source {
                InputSource::Software { name } => bindings
                    .bind_source(node, Tone::new(selectors::tone(name)?))
                    .map_err(|error| BackendError::Unsupported(error.to_string()))?,
                InputSource::Physical(endpoint) => {
                    let endpoint_id = self.resources.resolve(
                        node,
                        ResourceSelector::Endpoint {
                            endpoint: endpoint.clone(),
                            capture: true,
                        },
                    )?;
                    let native_capture = prepare_capture_native_with_gate(
                        CaptureOptions {
                            endpoint_id,
                            duration: SessionDuration::UntilStopped,
                            shared_period: period(input.device),
                            latency_telemetry: self.config.telemetry,
                        },
                        stop.clone(),
                        gate.clone(),
                    )
                    .map_err(capture_error)?;
                    let info = native_capture.info();
                    if input
                        .device
                        .hardware_sample_rate
                        .is_some_and(|rate| rate != info.timing.sample_rate)
                    {
                        return Err(BackendError::Unsupported(
                            "capture mix rate differs from requested hardware rate".into(),
                        ));
                    }
                    let budget = prepare_latency_budget(
                        info.timing,
                        output_timing,
                        self.config.latency_budget,
                    )
                    .map_err(|error| BackendError::Unsupported(error.to_string()))?;
                    let capture = native_capture.attach(budget.input).map_err(capture_error)?;
                    group.captures.push(capture.session);
                    bindings
                        .bind_source(node, capture.source)
                        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
                }
                InputSource::Application { executable, intent } => {
                    if *intent != CaptureIntent::PassiveTap {
                        return Err(BackendError::Unsupported(
                            "process capture supports PassiveTap only".into(),
                        ));
                    }
                    let (pid, creation) = selectors::process(executable)?;
                    self.resources
                        .resolve(node, ResourceSelector::Process { pid, creation })?;
                    let target = inspect_process(pid).map_err(capture_error)?;
                    if target.creation_time_100ns != creation {
                        return Err(BackendError::ResourceUnavailable(
                            "selected process identity changed".into(),
                        ));
                    }
                    let capture = prepare_process_capture_with_gate(
                        ProcessLoopbackOptions {
                            target,
                            duration: SessionDuration::UntilStopped,
                            latency_telemetry: self.config.telemetry,
                        },
                        stop.clone(),
                        gate.clone(),
                    )
                    .map_err(capture_error)?;
                    group.captures.push(capture.session);
                    bindings
                        .bind_source(node, capture.source)
                        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
                }
            }
        }
        let block = request.project.processing.engine.max_block_frames;
        let (writer, reader) =
            audio_bridge::<f32>(2, block, request.project.processing.audio_byte_budget)
                .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        bindings
            .bind_sink(sink, writer)
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let captures = std::mem::take(&mut group.captures);
        Ok(PreparedBackend {
            bindings,
            transport: Some(Box::new(WindowsTransport {
                native: Some(native),
                captures,
                output: Some(reader),
                sink,
                stop,
                gate,
                generation: request.owner_generation,
                queue: self.config.render_queue,
                diagnostics: self.diagnostics.clone(),
                resources: self.resources.clone(),
            })),
        })
    }
}

impl EngineBackend<f32> for WindowsBackend {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<f32>, BackendError> {
        let result = self.prepare_inner(request, cancel);
        if matches!(result, Err(BackendError::ResourceUnavailable(_))) {
            self.resources.retry();
        }
        result
    }
    fn poll_resource(&self) -> Option<ResourceEvent> {
        self.resources.poll()
    }
}

struct PreparationGroup {
    stop: StopSignal,
    captures: Vec<CaptureSession>,
}
impl Drop for PreparationGroup {
    fn drop(&mut self) {
        if !self.captures.is_empty() {
            let _ = self.stop.request_stop();
        }
    }
}

fn validate_project(request: &PrepareRequest<'_>) -> Result<(), BackendError> {
    let project = request.project;
    if project.processing.engine.processing_sr != 48_000.0
        || project
            .nodes
            .values()
            .filter(|node| matches!(node, NodeConfig::Output(_)))
            .count()
            != 1
    {
        return Err(BackendError::Unsupported(
            "Windows owner requires 48kHz and one stereo output".into(),
        ));
    }
    for config in project.nodes.values() {
        let (channels, device) = match config {
            NodeConfig::Input(input) => (input.channels, input.device),
            NodeConfig::Output(output) => (output.channels, output.device),
            _ => continue,
        };
        if channels != 2 || device.mode != DeviceMode::Shared {
            return Err(BackendError::Unsupported(
                "Windows owner supports Shared stereo graph IO".into(),
            ));
        }
        period(device)
            .validate()
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        if matches!(config, NodeConfig::Input(_)) && device.clock_role != ClockRole::Follower
            || matches!(config, NodeConfig::Output(_)) && device.clock_role != ClockRole::Master
        {
            return Err(BackendError::Unsupported(
                "render is the master clock and capture inputs must follow it".into(),
            ));
        }
        match config {
            NodeConfig::Input(input) => match &input.source {
                InputSource::Software { name } => {
                    selectors::tone(name)?;
                    if device.hardware_sample_rate.is_some() || device.period_frames.is_some() {
                        return Err(BackendError::Unsupported(
                            "software tone has no hardware rate or device period".into(),
                        ));
                    }
                }
                InputSource::Application { executable, intent } => {
                    selectors::process(executable)?;
                    if *intent != CaptureIntent::PassiveTap
                        || device.period_frames.is_some()
                        || device
                            .hardware_sample_rate
                            .is_some_and(|rate| rate != 48_000)
                    {
                        return Err(BackendError::Unsupported(
                            "process capture uses a fixed 48kHz passive Shared stream".into(),
                        ));
                    }
                }
                InputSource::Physical(_) => {
                    if device
                        .hardware_sample_rate
                        .is_some_and(|rate| ![44_100, 48_000].contains(&rate))
                    {
                        return Err(BackendError::Unsupported(
                            "capture supports 44.1/48kHz native f32".into(),
                        ));
                    }
                }
            },
            NodeConfig::Output(output)
                if !matches!(output.target, OutputTarget::Physical(_))
                    || device
                        .hardware_sample_rate
                        .is_some_and(|rate| rate != 48_000) =>
            {
                return Err(BackendError::Unsupported(
                    "output requires physical 48kHz stereo f32".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn period(device: DeviceOptions) -> SharedPeriodPolicy {
    SharedPeriodPolicy {
        desired_period_ms: device.period_frames.map(|frames| {
            frames as f64 * 1000.0 / f64::from(device.hardware_sample_rate.unwrap_or(48_000))
        }),
    }
}

pub(super) fn render_error(error: RenderError) -> BackendError {
    match error {
        RenderError::Cancelled => BackendError::Cancelled,
        RenderError::Api { .. } => BackendError::ResourceUnavailable(error.to_string()),
        RenderError::WorkerPanicked => BackendError::WorkerPanicked,
        _ => BackendError::Unsupported(error.to_string()),
    }
}
pub(super) fn capture_error(error: CaptureError) -> BackendError {
    match error {
        CaptureError::Cancelled => BackendError::Cancelled,
        CaptureError::Api { .. }
        | CaptureError::TargetExited
        | CaptureError::ProcessIdentityChanged
        | CaptureError::ActivationTimeout => BackendError::ResourceUnavailable(error.to_string()),
        CaptureError::WorkerPanicked => BackendError::WorkerPanicked,
        _ => BackendError::Unsupported(error.to_string()),
    }
}

struct Tone {
    phase: f64,
    frequency: f64,
}
impl Tone {
    fn new(frequency: f64) -> Self {
        Self {
            phase: 0.0,
            frequency,
        }
    }
}
impl RtAudioSource<f32> for Tone {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(
        &mut self,
        context: &ProcessContext,
        mut output: AudioBlockMut<'_, f32>,
    ) -> BoundaryReport {
        let advance = std::f64::consts::TAU * self.frequency / context.processing_sr;
        for frame in 0..context.frames {
            let value = self.phase.sin() as f32;
            output.channel_mut(0)[frame] = value;
            output.channel_mut(1)[frame] = value;
            self.phase = (self.phase + advance) % std::f64::consts::TAU;
        }
        BoundaryReport {
            transferred_frames: context.frames,
            ..BoundaryReport::default()
        }
    }
}
