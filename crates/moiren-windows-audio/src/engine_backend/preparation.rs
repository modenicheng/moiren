use super::{
    NativeObservations, NativeStreamKind, NativeStreamObservation, WindowsBackendConfig,
    owner::WindowsTransport,
    resources::{ResourceSelector, Resources},
    selectors,
};
use crate::{
    ActivationGate, SessionDuration, StopSignal,
    capture::{CaptureOptions, prepare_capture_native_with_gate},
    clock_bridge::{ClockBridgeConfig, capture_bridge},
    latency::prepare_render_queue,
    process_loopback::{
        ProcessLoopbackOptions, inspect_process, prepare_process_capture_with_gate,
    },
    render::{RenderOptions, prepare_render_native_with_gate},
    shared_stream::SharedPeriodPolicy,
};
use moiren_engine::{
    boundary::{CaptureIntent, ClockRole, audio_bridge},
    compiler::NodeBindings,
    host::{
        BackendError, CancellationToken, EngineBackend, NodeConfig, PrepareMode, PrepareRequest,
        PreparedBackend, ResourceEvent,
    },
    node::{DeviceOptions, InputSource, OutputTarget},
};
use std::sync::{Arc, Mutex};

mod errors;
mod group;
mod tone;
pub(super) use errors::{capture_error, render_error};
use group::PreparationGroup;
use tone::Tone;

pub(super) type Diagnostics = Arc<Mutex<Option<String>>>;

pub struct WindowsBackend {
    config: WindowsBackendConfig,
    pub(super) resources: Arc<Resources>,
    pub(super) diagnostics: Diagnostics,
    observations: NativeObservations,
}

impl WindowsBackend {
    pub fn new(config: WindowsBackendConfig) -> Result<Self, BackendError> {
        config.validate()?;
        Ok(Self {
            config,
            resources: Arc::new(Resources::new(config)?),
            diagnostics: Arc::new(Mutex::new(None)),
            observations: NativeObservations::default(),
        })
    }

    pub fn observations(&self) -> NativeObservations {
        self.observations.clone()
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
        super::validate_native_project(request.project)?;
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
        let stop = StopSignal::new().map_err(|error| BackendError::Failed(error.to_string()))?;
        let cancellation_stop = stop.clone();
        cancel.on_cancel(move || {
            let _ = cancellation_stop.request_stop();
        })?;
        let gate = ActivationGate::new(stop.clone())
            .map_err(|error| BackendError::Failed(error.to_string()))?;
        let mut group = PreparationGroup::new(stop.clone(), self.diagnostics.clone());
        let mut streams = Vec::new();
        let mut bridges = Vec::new();
        let mut endpoints = Vec::new();
        let mut master_index = 0;
        for (&node, config) in &request.project.nodes {
            let NodeConfig::Output(output) = config else {
                continue;
            };
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            let OutputTarget::Physical(endpoint) = &output.target else {
                unreachable!("validated native output");
            };
            let endpoint_id = self.resources.resolve(
                node,
                ResourceSelector::Endpoint {
                    endpoint: endpoint.clone(),
                    capture: false,
                },
            )?;
            if endpoints.contains(&endpoint_id) {
                return Err(BackendError::Unsupported(
                    "multiple output nodes resolved to the same native endpoint".into(),
                ));
            }
            endpoints.push(endpoint_id.clone());
            let native = prepare_render_native_with_gate(
                RenderOptions {
                    endpoint_id,
                    duration: SessionDuration::UntilStopped,
                    shared_period: self.period(output.device),
                    queue: self.config.render_queue,
                    latency_telemetry: self.config.telemetry,
                },
                stop.clone(),
                gate.clone(),
            )
            .map_err(render_error)?;
            let (info, render_queue) = group.admit_output(
                node,
                native,
                self.config.latency_profile,
                self.config.render_queue,
            )?;
            let master = output.device.clock_role == ClockRole::Master;
            streams.push(NativeStreamObservation {
                node,
                kind: if master {
                    NativeStreamKind::MasterOutput
                } else {
                    NativeStreamKind::FollowerOutput
                },
                timing: Some(info.timing),
                shared_period: Some(info.shared_period),
                render_queue: Some(render_queue),
                input_queue: None,
            });
            if master {
                master_index = group.prepared.len() - 1;
            }
        }
        let (sink, native, queue) = group.prepared.swap_remove(master_index);
        let output_timing = native.info().timing;
        // Keep the master owner in the cancellation group while capture and
        // follower preparation can still fail.
        group.prepared.push((sink, native, queue));
        let mut capture_endpoints = Vec::new();
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
                    if capture_endpoints.contains(&endpoint_id) {
                        return Err(BackendError::Unsupported(
                            "multiple input nodes resolved to the same native endpoint".into(),
                        ));
                    }
                    capture_endpoints.push(endpoint_id.clone());
                    let native_capture = prepare_capture_native_with_gate(
                        CaptureOptions {
                            endpoint_id,
                            duration: SessionDuration::UntilStopped,
                            shared_period: self.period(input.device),
                            latency_telemetry: self.config.telemetry,
                        },
                        stop.clone(),
                        gate.clone(),
                    )
                    .map_err(capture_error)?;
                    let (capture, info, input_queue) = group.admit_capture(
                        native_capture,
                        input.device.hardware_sample_rate,
                        self.config.latency_profile,
                        output_timing,
                        self.config.latency_budget,
                    )?;
                    streams.push(NativeStreamObservation {
                        node,
                        kind: NativeStreamKind::PhysicalInput,
                        timing: Some(info.timing),
                        shared_period: Some(info.shared_period),
                        render_queue: None,
                        input_queue: Some(input_queue),
                    });
                    bridges.push((node, capture.observer));
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
                    streams.push(NativeStreamObservation {
                        node,
                        kind: NativeStreamKind::ProcessInput,
                        timing: None,
                        shared_period: None,
                        render_queue: None,
                        input_queue: Some(crate::latency::PreparedInputQueue {
                            capacity_frames: 8192,
                            prime_frames: 2048,
                            servo_reference_frames: 2048,
                            low_water_frames: 2,
                            high_water_frames: 8191,
                            modeled_minimum_frames: None,
                        }),
                    });
                    bridges.push((node, capture.observer));
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
        let mut sinks = vec![sink];
        // All followers consume their own graph sink. The capture bridge's
        // continuous fill servo corrects their independent native clock drift;
        // none receives or executes another Engine.
        while group.prepared.len() > 1 {
            let node = group.prepared[0].0;
            let info = group.prepared[0].1.info();
            let input = group.check(
                self.config
                    .latency_profile
                    .follower_input(
                        output_timing,
                        info.timing,
                        self.config.latency_budget,
                        prepare_render_queue(output_timing, queue)
                            .map_err(|error| BackendError::Unsupported(error.to_string()))?,
                    )
                    .map_err(|error| BackendError::Unsupported(error.to_string())),
            )?;
            let mut config = ClockBridgeConfig::with_prepared_input(48_000, 2, input);
            config.byte_budget = request.project.processing.audio_byte_budget;
            let (writer, source, observer) = group.check(
                capture_bridge(config)
                    .map_err(|error| BackendError::Unsupported(error.to_string())),
            )?;
            group.check(
                bindings
                    .bind_sink(node, writer)
                    .map_err(|error| BackendError::Unsupported(error.to_string())),
            )?;
            sinks.push(node);
            bridges.push((node, observer));
            if let Some(stream) = streams.iter_mut().find(|stream| stream.node == node) {
                stream.input_queue = Some(input);
            }
            // Remove the prepared owner only for its report-returning handoff;
            // all budget, bridge and binding checks above retain group ownership.
            let (_, follower, follower_queue) = group.prepared.remove(0);
            let session = match follower.attach_follower(source, follower_queue) {
                Ok(prepared) => prepared.into_session(),
                Err(failure) => {
                    if let Some(report) = failure.report {
                        group.failed_renders.push(report);
                    }
                    return group.check(Err(render_error(failure.error)));
                }
            };
            group.followers.push(session);
        }
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let (sink, native, queue) = group.prepared.pop().expect("master prepared last");
        let captures = std::mem::take(&mut group.captures);
        let followers = std::mem::take(&mut group.followers);
        Ok(PreparedBackend {
            bindings,
            transport: Some(Box::new(WindowsTransport {
                native: Some(native),
                captures,
                followers,
                output: Some(reader),
                sink,
                sinks,
                stop,
                gate,
                generation: request.owner_generation,
                queue,
                diagnostics: self.diagnostics.clone(),
                resources: self.resources.clone(),
                observations: self.observations.clone(),
                streams,
                bridges,
            })),
        })
    }
    fn period(&self, device: DeviceOptions) -> SharedPeriodPolicy {
        let mut period = super::profile::device_period(device);
        if period.desired_period_ms.is_none() {
            period.desired_period_ms = self.config.latency_profile.desired_period_ms();
        }
        period
    }
}

impl EngineBackend<f32> for WindowsBackend {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<f32>, BackendError> {
        let result = self.prepare_inner(request, cancel);
        if let Err(error) = &result {
            let mut diagnostics = self
                .diagnostics
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if let Some(report) = diagnostics
                .as_ref()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                && report["failure"] == "native preparation did not complete"
            {
                let mut report = report;
                report["failure"] = serde_json::Value::String(error.to_string());
                *diagnostics = serde_json::to_string(&report).ok();
            }
        }
        if matches!(result, Err(BackendError::ResourceUnavailable(_))) {
            self.resources.retry();
        }
        result
    }
    fn poll_resource(&self) -> Option<ResourceEvent> {
        self.resources.poll()
    }
}
