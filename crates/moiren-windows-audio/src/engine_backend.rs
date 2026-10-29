//! Resolve intent on control, then negotiate every native stream on its owner.
mod native_observations;
pub(crate) mod observations;
#[cfg(windows)]
mod owner;
mod process_admission;
#[cfg(any(windows, test))]
mod recovery;

pub use native_observations::{
    NativeBoundaryProbe, NativeBridgeObservation, NativeObservationReadHandle,
    NativeObservationSnapshot, NativeObservations, NativeProcessDeliveryObservation,
    NativeProcessTraceCollector, NativeProcessWindowCandidate, NativeRenderPaddingObservation,
    NativeStreamKind, NativeStreamObservation, PreparedNativeProcessWindow,
};
#[cfg(windows)]
pub use observations::{
    ProfessionalObservationReadHandle, ProfessionalObservationSnapshot, ProfessionalObservations,
    ProfessionalStreamObservation,
};

use crate::asio::{AsioOptions, AsioStream};
use crate::bridge::{CaptureWriteClock, ProcessSource, RingSink, RingSource, Tone, unsupported};
use crate::latency::{LatencyBudgetPolicy, NativeStreamTiming, RenderQueuePolicy, TelemetryMode};
use crate::native::{NativeStreamInfo, canonical_asio_driver_id};
use crate::virtual_owner::{
    ProjectClock, QpcClockFactory, VirtualBoundaryProvider, VirtualClockFactory,
    VirtualPreparedTransport,
};
use crate::wasapi::{WasapiOptions, WasapiStream, resolve_endpoint};
use moiren_core::graph::NodeId;
use moiren_engine::{
    audio_config::{AudioBackend, SAMPLE_RATES, SampleFormat},
    boundary::{CaptureIntent, ClockRole},
    compiler::NodeBindings,
    host::{
        BackendError, CancellationToken, EngineBackend, NodeConfig, PrepareMode, PrepareRequest,
        PreparedBackend,
    },
    node::{DeviceMode, DeviceOptions, EndpointSelector, InputSource, OutputTarget},
    sample::ProcessingSample,
};
use observations::Counters;
use owner::{AuxiliaryOwner, StreamFactory, Transport, WorkerRequest, spawn};
use recovery::RecoveryMonitor;
use rtrb::RingBuffer;
use std::{marker::PhantomData, sync::Arc};

#[cfg(test)]
#[path = "engine_backend/tests.rs"]
mod virtual_tests;

mod validation;
mod virtual_io;
use validation::validate;

pub struct ProfessionalBackend<S: ProcessingSample> {
    observations: ProfessionalObservations,
    native_observations: NativeObservations,
    recovery: RecoveryMonitor,
    virtual_provider: Option<Arc<dyn VirtualBoundaryProvider>>,
    virtual_clock: Arc<dyn VirtualClockFactory>,
    process_bridge: crate::process_loopback::ProcessBridgeOptions,
    measured_process_bridges: Vec<process_admission::MeasuredBridgeRequest>,
    virtual_buffer: crate::virtual_owner::VirtualBufferOptions,
    render_queue: RenderQueuePolicy,
    latency_budget: LatencyBudgetPolicy,
    telemetry: TelemetryMode,
    marker: PhantomData<S>,
}
impl<S: ProcessingSample> ProfessionalBackend<S> {
    pub fn new() -> Result<Self, BackendError> {
        Ok(Self {
            observations: ProfessionalObservations::default(),
            native_observations: NativeObservations::default(),
            recovery: RecoveryMonitor::default(),
            virtual_provider: None,
            virtual_clock: Arc::new(QpcClockFactory),
            process_bridge: Default::default(),
            measured_process_bridges: Vec::new(),
            virtual_buffer: Default::default(),
            render_queue: RenderQueuePolicy::default(),
            latency_budget: LatencyBudgetPolicy::default(),
            telemetry: TelemetryMode::Summary,
            marker: PhantomData,
        })
    }
    pub fn observations(&self) -> ProfessionalObservations {
        self.observations.clone()
    }
    pub fn native_observations(&self) -> NativeObservations {
        self.native_observations.clone()
    }
    /// Enable virtual resource preparation without changing physical settings.
    pub fn with_virtual_provider(mut self, provider: Arc<dyn VirtualBoundaryProvider>) -> Self {
        self.virtual_provider = Some(provider);
        self
    }
    /// Select the clock used only when there are no physical render outputs.
    pub fn with_virtual_clock_factory(mut self, clock: Arc<dyn VirtualClockFactory>) -> Self {
        self.virtual_clock = clock;
        self
    }
    pub fn with_process_bridge(
        mut self,
        bridge: crate::process_loopback::ProcessBridgeOptions,
    ) -> Result<Self, BackendError> {
        bridge.prepare().map_err(unsupported)?;
        self.process_bridge = bridge;
        Ok(self)
    }
    pub fn with_virtual_buffer(
        mut self,
        buffer: crate::virtual_owner::VirtualBufferOptions,
    ) -> Result<Self, BackendError> {
        buffer.validate().map_err(unsupported)?;
        self.virtual_buffer = buffer;
        Ok(self)
    }
    pub fn with_render_queue(mut self, policy: RenderQueuePolicy) -> Result<Self, BackendError> {
        policy.validate().map_err(unsupported)?;
        self.render_queue = policy;
        Ok(self)
    }
    pub fn with_latency_budget(
        mut self,
        policy: LatencyBudgetPolicy,
    ) -> Result<Self, BackendError> {
        if let LatencyBudgetPolicy::Custom {
            input_capacity_frames,
            input_target_frames,
            render,
            ..
        } = policy
        {
            if !(4..=262_144).contains(&input_capacity_frames)
                || !(2..input_capacity_frames).contains(&input_target_frames)
            {
                return Err(unsupported(
                    "custom physical input queue exceeds the bounded frame range",
                ));
            }
            render.validate().map_err(unsupported)?;
            self.render_queue = render;
        }
        self.latency_budget = policy;
        Ok(self)
    }
    pub fn with_telemetry(mut self, mode: TelemetryMode) -> Self {
        self.telemetry = mode;
        self
    }
}

#[derive(Clone)]
struct Direction {
    node: NodeId,
    device: DeviceOptions,
}
struct Group {
    id: String,
    backend: AudioBackend,
    input: Option<Direction>,
    output: Option<Direction>,
}
impl Group {
    fn master(&self) -> bool {
        self.output
            .as_ref()
            .is_some_and(|d| d.device.clock_role == ClockRole::Master)
    }
    fn factory(&self, processing_rate: u32) -> Result<StreamFactory, BackendError> {
        match self.backend {
            AudioBackend::Wasapi => {
                let direction = self.input.as_ref().or(self.output.as_ref()).unwrap();
                let options = WasapiOptions {
                    endpoint_id: self.id.clone(),
                    sample_rate: direction
                        .device
                        .hardware_sample_rate
                        .unwrap_or(processing_rate),
                    device: direction.device,
                    input: self.input.is_some(),
                };
                Ok(Box::new(move || Ok(Box::new(WasapiStream::open(options)?))))
            }
            AudioBackend::Asio => {
                let (rate, buffer_frames) = asio_session_options(
                    self.input.as_ref().map(|d| d.device),
                    self.output.as_ref().map(|d| d.device),
                    processing_rate,
                )?;
                let options = AsioOptions {
                    driver_id: self.id.clone(),
                    sample_rate: rate,
                    buffer_frames,
                    input: self.input.is_some(),
                    output: self.output.is_some(),
                };
                Ok(Box::new(move || Ok(Box::new(AsioStream::open(options)?))))
            }
        }
    }
}

fn asio_session_options(
    input: Option<DeviceOptions>,
    output: Option<DeviceOptions>,
    processing_rate: u32,
) -> Result<(u32, u32), BackendError> {
    let rate = input
        .or(output)
        .unwrap()
        .hardware_sample_rate
        .unwrap_or(processing_rate);
    let mut buffer = None;
    for device in input.into_iter().chain(output) {
        if device.hardware_sample_rate.unwrap_or(processing_rate) != rate {
            return Err(unsupported(
                "one ASIO duplex session requires one sample rate",
            ));
        }
        if let Some(frames) = device.period_frames {
            if buffer.is_some_and(|b| b != frames) {
                return Err(unsupported(
                    "one ASIO duplex session requires one buffer size",
                ));
            }
            buffer = Some(frames);
        }
    }
    // Zero is the ASIO adapter's driver-preferred sentinel. One explicit side
    // selects the common duplex period; two explicit sides must agree.
    Ok((rate, buffer.unwrap_or(0) as u32))
}

trait NativeDevices {
    fn resolve(
        &self,
        selector: &EndpointSelector,
        device: DeviceOptions,
        input: bool,
    ) -> Result<String, BackendError>;
    fn factory(&self, group: &Group, rate: u32) -> Result<StreamFactory, BackendError>;
}

struct SystemNativeDevices;
impl NativeDevices for SystemNativeDevices {
    fn resolve(
        &self,
        selector: &EndpointSelector,
        device: DeviceOptions,
        input: bool,
    ) -> Result<String, BackendError> {
        match device.backend {
            AudioBackend::Wasapi => resolve_endpoint(selector, input),
            AudioBackend::Asio => match selector {
                EndpointSelector::PinnedEndpoint { endpoint_id, .. } => {
                    canonical_asio_driver_id(endpoint_id)
                }
                _ => Err(unsupported(
                    "ASIO requires an explicitly selected driver CLSID",
                )),
            },
        }
    }
    fn factory(&self, group: &Group, rate: u32) -> Result<StreamFactory, BackendError> {
        group.factory(rate)
    }
}

impl<S: ProcessingSample> EngineBackend<S> for ProfessionalBackend<S> {
    fn poll_resource(&self) -> Option<moiren_engine::host::ResourceEvent> {
        self.recovery.poll().or_else(|| {
            self.virtual_provider
                .as_ref()
                .and_then(|p| p.poll_resource())
        })
    }
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<S>, BackendError> {
        self.prepare_with_native(request, cancel, &SystemNativeDevices)
    }
}

impl<S: ProcessingSample> ProfessionalBackend<S> {
    fn prepare_with_native(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
        native: &dyn NativeDevices,
    ) -> Result<PreparedBackend<S>, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        if let LatencyBudgetPolicy::Custom { render, .. } = self.latency_budget
            && render != self.render_queue
        {
            return Err(unsupported(
                "latency budget and renderer must use the same render queue policy",
            ));
        }
        let (rate, clock) = validate(&request, self.virtual_provider.is_some())?;
        self.validate_measured_sources(request.project)?;
        let mut bindings = NodeBindings::new();
        if request.mode == PrepareMode::Update {
            for (&node, config) in &request.project.nodes {
                if request.reuse.iter().any(|&(reused, _)| reused == node) {
                    continue;
                }
                match config {
                    NodeConfig::Input(input) => match &input.source {
                        InputSource::Software { name } => bindings
                            .bind_source(node, Tone::new(name, rate)?)
                            .map_err(unsupported)?,
                        _ => {
                            return Err(unsupported(
                                "native IO changes require a stopped professional transport",
                            ));
                        }
                    },
                    NodeConfig::Output(_) => {
                        return Err(unsupported(
                            "native IO changes require a stopped professional transport",
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
        let io_count = request
            .project
            .nodes
            .values()
            .filter(|c| matches!(c, NodeConfig::Input(_) | NodeConfig::Output(_)))
            .count();
        let capacity = (request.project.processing.engine.max_block_frames * 8).max(16384);
        let capture_capacity = match self.latency_budget {
            LatencyBudgetPolicy::Custom {
                input_capacity_frames,
                ..
            } => usize::try_from(input_capacity_frames).map_err(unsupported)?,
            LatencyBudgetPolicy::Compatible => capacity,
        };
        let mut prepared_bytes = io_count
            .checked_mul(capacity.max(capture_capacity) * 32 + 131072)
            .ok_or_else(|| unsupported("native queue budget overflow"))?;
        for (&node, config) in &request.project.nodes {
            let extra = match config {
                NodeConfig::Input(input)
                    if matches!(input.source, InputSource::Application { .. }) =>
                {
                    let InputSource::Application { executable, .. } = &input.source else {
                        unreachable!()
                    };
                    // Resolve the selected policy for storage accounting now;
                    // admission runs again after the master's actual demand
                    // is negotiated. Never budget a measured queue as legacy.
                    self.process_bridge_for(node, process_selector(executable)?, rate, 1)?
                        .clock_config()
                        .map_err(unsupported)?
                        .storage_bytes()
                        .map_err(unsupported)?
                }
                NodeConfig::Input(input)
                    if matches!(input.source, InputSource::VirtualPlayback { .. }) =>
                {
                    virtual_extra_bytes(self.virtual_buffer, capacity, input.channels)?
                }
                NodeConfig::Output(output)
                    if matches!(output.target, OutputTarget::VirtualMicrophone { .. }) =>
                {
                    virtual_extra_bytes(self.virtual_buffer, capacity, output.channels)?
                }
                _ => 0,
            };
            prepared_bytes = prepared_bytes
                .checked_add(extra)
                .ok_or_else(|| unsupported("native queue budget overflow"))?;
        }
        if prepared_bytes > request.project.processing.audio_byte_budget {
            return Err(unsupported(
                "professional native queues/SRC exceed the audio preparation budget",
            ));
        }
        let mut transport = Box::new(Transport::<S>::new(
            request.owner_generation,
            self.observations.clone(),
        ));
        transport.native_observations = self.native_observations.clone();
        let block_demand = u32::try_from(request.project.processing.engine.max_block_frames)
            .map_err(|_| unsupported("virtual block demand overflow"))?;
        let recovery_cap_periods = self
            .virtual_buffer
            .admitted_recovery_cap_periods()
            .map_err(unsupported)?;
        let virtual_demand = if clock == ProjectClock::VirtualRealtime {
            self.virtual_buffer
                .prepare_recovery(rate, block_demand)
                .map_err(unsupported)?
                .engine_burst_frames
        } else {
            // Replaced with the physical master's negotiated whole wake below.
            block_demand
                .checked_mul(recovery_cap_periods as u32)
                .ok_or_else(|| unsupported("virtual burst demand overflow"))?
        };
        // Compatible resources keep their existing preparation/cleanup order.
        // Explicit targets wait for the physical master's negotiated demand.
        let virtual_prepared = if self.virtual_buffer == Default::default() {
            Some(virtual_io::prepare(
                &request,
                self.virtual_provider.as_deref(),
                rate,
                virtual_demand,
                self.virtual_buffer,
                &mut bindings,
                cancel,
            )?)
        } else {
            None
        };

        let mut groups: Vec<Group> = Vec::new();
        let mut processes = Vec::new();
        let mut process_nodes = Vec::new();
        for (&node, config) in &request.project.nodes {
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            let (selector, input, device) = match config {
                NodeConfig::Input(input) => match &input.source {
                    InputSource::Software { name } => {
                        bindings
                            .bind_source(node, Tone::new(name, rate)?)
                            .map_err(unsupported)?;
                        continue;
                    }
                    InputSource::Application { executable, intent } => {
                        if *intent != CaptureIntent::PassiveTap
                            || input.device.mode != DeviceMode::Shared
                            || input
                                .device
                                .hardware_sample_rate
                                .is_some_and(|r| r != 48000 && r != rate)
                            || input.device.period_frames.is_some()
                            || !matches!(
                                input.device.sample_format,
                                SampleFormat::Native | SampleFormat::Float32
                            )
                        {
                            return Err(unsupported(
                                "process loopback is a default-buffer Shared 48 kHz f32 stream",
                            ));
                        }
                        let identity = process_selector(executable)?;
                        if processes.contains(&identity) {
                            return Err(unsupported("duplicate process capture ownership"));
                        }
                        processes.push(identity);
                        process_nodes.push((node, input.device, identity));
                        continue;
                    }
                    InputSource::Physical(selector) => (selector, true, input.device),
                    InputSource::VirtualPlayback { .. } => continue,
                },
                NodeConfig::Output(output) => match &output.target {
                    OutputTarget::Physical(selector) => (selector, false, output.device),
                    OutputTarget::VirtualMicrophone { .. } => continue,
                    _ => unreachable!("validated output target"),
                },
                _ => continue,
            };
            let id = native.resolve(selector, device, input)?;
            let endpoint_id_for_log = id.clone();
            let direction = Direction { node, device };
            if let Some(group) = groups.iter_mut().find(|g| {
                g.id == id
                    && g.backend == device.backend
                    && (device.backend == AudioBackend::Asio || g.input.is_some() == input)
            }) {
                if device.backend != AudioBackend::Asio
                    || if input {
                        group.input.is_some()
                    } else {
                        group.output.is_some()
                    }
                {
                    return Err(unsupported("duplicate native endpoint ownership"));
                }
                if input {
                    group.input = Some(direction);
                } else {
                    group.output = Some(direction);
                }
            } else {
                groups.push(Group {
                    id,
                    backend: device.backend,
                    input: if input { Some(direction.clone()) } else { None },
                    output: if input { None } else { Some(direction) },
                });
            }
            log::debug!(
                target: "moiren::audio::professional",
                "resolved native device request: node={:?} backend={} endpoint={:?} direction={} settings={:?}",
                node,
                device.backend.title(),
                endpoint_id_for_log,
                if input { "input" } else { "output" },
                device,
            );
        }
        if groups
            .iter()
            .filter(|g| g.backend == AudioBackend::Asio)
            .count()
            > 1
        {
            return Err(unsupported(
                "direct ASIO hosts one driver session at a time",
            ));
        }
        let asio_configurations: Vec<_> = groups
            .iter()
            .filter(|group| group.backend == AudioBackend::Asio)
            .flat_map(|group| group.input.iter().chain(&group.output))
            .map(|direction| {
                (
                    direction.node,
                    request.project.nodes[&direction.node].clone(),
                )
            })
            .collect();
        let recovery = self
            .recovery
            .register(request.owner_generation, asio_configurations, rate);
        let mut process_demand = virtual_demand;
        // Custom input admission depends on the master's actual negotiated
        // period, so open that group before any physical capture followers.
        groups.sort_by_key(|group| !group.master());
        let mut master_timing: Option<NativeStreamTiming> = None;
        for group in groups {
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            let counters = Arc::new(Counters::default());
            let (capture_tx, capture_rx, capture_clock) = if group.input.is_some() {
                let (tx, rx) = RingBuffer::new(capture_capacity);
                (Some(tx), Some(rx), Some(Arc::new(CaptureWriteClock::new())))
            } else {
                (None, None, None)
            };
            let (render_tx, render_rx) = if group.output.is_some() {
                let (tx, rx) = RingBuffer::new(capacity);
                (Some(tx), Some(rx))
            } else {
                (None, None)
            };
            let metadata = Arc::new(std::sync::Mutex::new(None));
            let owner_metadata = metadata.clone();
            let factory = native.factory(&group, rate)?;
            let queue = self.render_queue;
            let renders = group.output.is_some();
            let worker_request = WorkerRequest {
                factory: Box::new(move || {
                    let mut stream = factory()?;
                    if renders {
                        stream.configure_render_queue(queue)?;
                    }
                    *owner_metadata.lock().unwrap_or_else(|p| p.into_inner()) = Some((
                        stream.prepared_timing(),
                        stream.shared_period(),
                        stream.render_queue(),
                        stream.render_padding_observer(),
                    ));
                    Ok(stream)
                }),
                capture: capture_tx,
                capture_clock: capture_clock.clone(),
                render: render_rx,
                counters: counters.clone(),
                state: transport.state.clone(),
                slot: transport.slot.clone(),
                timeline: transport.timeline.clone(),
                engine_rate: rate,
                capacity,
                master: group.master(),
                recovery: (group.backend == AudioBackend::Asio).then(|| recovery.clone()),
            };
            log::info!(
                target: "moiren::audio::professional",
                "opening native device group: backend={} endpoint={:?} input={} output={} processing_rate={} input_settings={:?} output_settings={:?} master={}",
                group.backend.title(),
                group.id,
                group.input.is_some(),
                group.output.is_some(),
                rate,
                group.input.as_ref().map(|direction| direction.device),
                group.output.as_ref().map(|direction| direction.device),
                group.master(),
            );
            let (worker, info) = spawn(worker_request, cancel)?;
            transport.workers.push(worker);
            let (timing, shared_period, render_queue, padding) = metadata
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .expect("native preparation publishes metadata before its handshake");
            if group.master() {
                master_timing = Some(timing);
                // Pump's bound includes SRC lookahead and graph rounding on
                // its last render. Budget the whole wake, never one segment.
                let demand = render_queue.map_or(info.buffer_frames, |queue| queue.target_frames);
                process_demand = u32::try_from(
                    (u64::from(demand) * u64::from(rate))
                        .div_ceil(u64::from(info.sample_rate))
                        .checked_add(64)
                        .and_then(|frames| frames.checked_add(u64::from(block_demand) * 2))
                        .ok_or_else(|| unsupported("process demand overflow"))?,
                )
                .map_err(|_| unsupported("process demand overflow"))?;
            }
            log::info!(
                target: "moiren::audio::professional",
                "native device group opened: backend={} endpoint={:?} actual_rate={} actual_format={:?} actual_channels={} actual_buffer_frames={}",
                group.backend.title(),
                group.id,
                info.sample_rate,
                info.format,
                info.channels,
                info.buffer_frames,
            );
            for direction in group.input.iter().chain(&group.output) {
                if direction.device.sample_format != SampleFormat::Native
                    && direction.device.sample_format != info.format
                {
                    return Err(unsupported(
                        "native driver encoding does not match requested sample format",
                    ));
                }
                if direction.device.hardware_sample_rate.unwrap_or(rate) != info.sample_rate {
                    return Err(unsupported(
                        "native driver rate does not match requested sample rate",
                    ));
                }
                if group.backend == AudioBackend::Asio
                    && direction
                        .device
                        .period_frames
                        .is_some_and(|frames| frames != info.buffer_frames as usize)
                {
                    return Err(unsupported(
                        "native driver buffer does not match the exact requested frame count",
                    ));
                }
            }
            if let Some(input) = &group.input {
                let queue = if matches!(self.latency_budget, LatencyBudgetPolicy::Custom { .. }) {
                    let prepared = crate::latency::prepare_latency_budget(
                        timing,
                        master_timing.ok_or_else(|| {
                            unsupported("custom physical input budgets require a physical master")
                        })?,
                        self.latency_budget,
                    )
                    .map_err(unsupported)?;
                    // The renamed backend uses a FIR at the graph boundary.
                    // Admit its whole wake, including block rounding, instead
                    // of reusing the old linear bridge's two-frame reserve.
                    let LatencyBudgetPolicy::Custom {
                        input_jitter_frames,
                        ..
                    } = self.latency_budget
                    else {
                        unreachable!()
                    };
                    Some(
                        crate::latency::transport::prepare_transport_input(
                            crate::latency::transport::TransportInputPolicy::Target {
                                capacity_frames: prepared.input.capacity_frames,
                                target_frames: prepared.input.prime_frames,
                                jitter_frames: input_jitter_frames,
                            },
                            Some(crate::latency::transport::InputDeliveryTiming {
                                sample_rate: timing.sample_rate,
                                max_packet_frames: timing.capacity_frames,
                                max_gap_frames: timing
                                    .period_frames
                                    .expect("custom budget requires actual period"),
                                source: crate::latency::transport::DeliveryTimingSource::Declared,
                            }),
                            rate,
                            process_demand,
                            32,
                        )
                        .map_err(unsupported)?,
                    )
                } else {
                    None
                };
                let source = RingSource::<S>::new(
                    capture_rx.unwrap(),
                    info.sample_rate,
                    rate,
                    info.buffer_frames as usize,
                    capture_capacity,
                    counters.clone(),
                    capture_clock.unwrap(),
                )?;
                let source = if let Some(queue) = queue {
                    source.with_input_queue(queue)
                } else {
                    source
                };
                bindings
                    .bind_source(input.node, source)
                    .map_err(unsupported)?;
                transport.native_streams.push(NativeStreamObservation {
                    node: input.node,
                    kind: NativeStreamKind::PhysicalInput,
                    timing: Some(timing),
                    shared_period,
                    render_queue: None,
                    input_queue: queue,
                    virtual_budget: None,
                });
                transport
                    .streams
                    .push((observation(input, true, false, info), counters.clone()));
            }
            if let Some(output) = &group.output {
                transport.native_streams.push(NativeStreamObservation {
                    node: output.node,
                    kind: if group.master() {
                        NativeStreamKind::MasterOutput
                    } else {
                        NativeStreamKind::FollowerOutput
                    },
                    timing: Some(timing),
                    shared_period,
                    render_queue,
                    input_queue: None,
                    virtual_budget: None,
                });
                if let Some(padding) = padding {
                    transport.native_renders.push((output.node, padding));
                }
                bindings
                    .bind_sink(
                        output.node,
                        RingSink::<S> {
                            tx: render_tx.unwrap(),
                            counters: counters.clone(),
                            marker: PhantomData,
                        },
                    )
                    .map_err(unsupported)?;
                transport.sinks.push(output.node);
                transport
                    .streams
                    .push((observation(output, false, group.master(), info), counters));
            }
        }
        let virtual_io::PreparedVirtualIo {
            sessions: mut virtual_sessions,
            sinks: virtual_sinks,
            observations: virtual_observations,
            probes: virtual_probes,
        } = match virtual_prepared {
            Some(prepared) => prepared,
            None => virtual_io::prepare(
                &request,
                self.virtual_provider.as_deref(),
                rate,
                process_demand,
                self.virtual_buffer,
                &mut bindings,
                cancel,
            )?,
        };
        for (stream, _) in &virtual_observations {
            let virtual_budget = self
                .virtual_buffer
                .prepare_budget(rate, block_demand, process_demand)
                .map_err(unsupported)?;
            transport.native_streams.push(NativeStreamObservation {
                node: stream.node,
                kind: if stream.input {
                    NativeStreamKind::VirtualPlayback
                } else {
                    NativeStreamKind::VirtualMicrophone
                },
                timing: None,
                shared_period: None,
                render_queue: None,
                input_queue: virtual_budget.queue,
                virtual_budget: Some(virtual_budget),
            });
        }
        transport.native_probes.extend(virtual_probes);
        transport.streams.extend(virtual_observations);
        if matches!(clock, ProjectClock::Physical(_)) {
            transport.sinks.extend(&virtual_sinks);
            if !virtual_sessions.is_empty() {
                transport
                    .auxiliaries
                    .push(Box::new(virtual_io::VirtualAuxiliary(std::mem::take(
                        &mut virtual_sessions,
                    ))));
            }
        }
        for (node, device, identity) in process_nodes {
            prepare_process(
                node,
                device,
                identity,
                self.process_bridge_for(node, identity, rate, process_demand)?,
                self.measured_source_generation(node),
                self.telemetry,
                &mut bindings,
                &mut transport,
                cancel,
            )?;
        }
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        if clock == ProjectClock::VirtualRealtime {
            // Captures keep their existing native owner threads and retirement
            // protocol. Only the QPC owner receives and advances the Engine.
            // The retired transport keeps publishing and fencing the stream
            // observations even when it owns no worker or auxiliary, so
            // virtual-only projects stay visible in the diagnostics snapshot.
            virtual_sessions.push(transport.into_follower(cancel.clone())?);
            let stop = crate::StopSignal::new().map_err(unsupported)?;
            let gate = crate::ActivationGate::new(stop.clone()).map_err(unsupported)?;
            let clock = self.virtual_clock.prepare(stop.clone())?;
            if cancel.is_cancelled() {
                return Err(BackendError::Cancelled);
            }
            return Ok(PreparedBackend {
                bindings,
                transport: Some(Box::new(
                    VirtualPreparedTransport::new(
                        virtual_sinks,
                        virtual_sessions,
                        stop,
                        gate,
                        clock,
                    )
                    .with_recovery_cap(recovery_cap_periods)?,
                )),
            });
        }
        Ok(PreparedBackend {
            bindings,
            transport: Some(transport),
        })
    }
}
fn virtual_extra_bytes(
    buffer: crate::virtual_owner::VirtualBufferOptions,
    reserved_frames: usize,
    channels: usize,
) -> Result<usize, BackendError> {
    let crate::latency::transport::TransportInputPolicy::Target {
        capacity_frames, ..
    } = buffer.input
    else {
        return Ok(0);
    };
    // The base estimate already covers the default carrier. Larger explicit
    // rings must also fit the aggregate project budget before any owner opens.
    (capacity_frames as usize)
        .saturating_sub(reserved_frames)
        .checked_mul(channels)
        .and_then(|samples| samples.checked_mul(size_of::<f32>()))
        .ok_or_else(|| unsupported("virtual carrier budget overflow"))
}

fn observation(
    direction: &Direction,
    input: bool,
    master: bool,
    actual: NativeStreamInfo,
) -> ProfessionalStreamObservation {
    ProfessionalStreamObservation {
        node: direction.node,
        input,
        master,
        requested: direction.device,
        actual,
        dropped_frames: 0,
        silent_frames: 0,
        native_callbacks: 0,
        native_wait_wakes: 0,
        native_written_frames: 0,
        native_wait_timeouts: 0,
        native_dropped_frames: 0,
        native_silent_frames: 0,
        native_capture_quality: None,
        driver_latency: None,
    }
}

fn process_selector(executable: &str) -> Result<(u32, u64), BackendError> {
    let (pid, creation) = executable
        .strip_prefix("moiren:process:")
        .and_then(|s| s.split_once(':'))
        .ok_or_else(|| unsupported("process input requires a PID and creation time"))?;
    pid.parse::<u32>()
        .ok()
        .filter(|&p| p > 0)
        .zip(creation.parse::<u64>().ok().filter(|&c| c > 0))
        .ok_or_else(|| unsupported("invalid process identity"))
}
struct ProcessOwner {
    capture: Option<crate::capture::CaptureSession>,
    gate: crate::ActivationGate,
    stop: crate::StopSignal,
}
impl AuxiliaryOwner for ProcessOwner {
    fn activate(&self) -> Result<(), BackendError> {
        self.gate.activate().map_err(unsupported)
    }
    fn cancel(&self) {
        let _ = self.stop.request_stop();
    }
    fn running(&self) -> bool {
        self.capture.as_ref().is_some_and(|c| c.has_started())
    }
    fn finished(&self) -> bool {
        self.capture.as_ref().is_some_and(|c| c.is_finished())
    }
    fn join(&mut self) -> Result<(), BackendError> {
        if let Some(capture) = self.capture.take() {
            let report = capture.join().map_err(unsupported)?;
            if matches!(
                report.status,
                crate::capture::CaptureStatus::Failed | crate::capture::CaptureStatus::TargetExited
            ) {
                return Err(BackendError::ResourceUnavailable(
                    report
                        .failure
                        .unwrap_or_else(|| "process capture failed".into()),
                ));
            }
        }
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
fn prepare_process<S: ProcessingSample>(
    node: NodeId,
    device: DeviceOptions,
    identity: (u32, u64),
    bridge: crate::process_loopback::ProcessBridgeOptions,
    measured_source_generation: Option<u64>,
    telemetry: TelemetryMode,
    bindings: &mut NodeBindings<S>,
    transport: &mut Transport<S>,
    cancel: &CancellationToken,
) -> Result<(), BackendError> {
    let target = crate::process_loopback::inspect_process(identity.0).map_err(unsupported)?;
    if target.creation_time_100ns != identity.1 {
        return Err(BackendError::ResourceUnavailable(
            "selected process identity changed".into(),
        ));
    }
    let stop = crate::StopSignal::new().map_err(unsupported)?;
    let gate = crate::ActivationGate::new(stop.clone()).map_err(unsupported)?;
    let cancel_stop = stop.clone();
    cancel.on_cancel(move || {
        let _ = cancel_stop.request_stop();
    })?;
    let prepared = crate::process_loopback::prepare_process_capture_with_gate(
        crate::process_loopback::ProcessLoopbackOptions {
            artifact: None,
            bridge,
            target: target.clone(),
            duration: crate::SessionDuration::UntilStopped,
            latency_telemetry: telemetry,
        },
        stop.clone(),
        gate.clone(),
    )
    .map_err(unsupported)?;
    let channels = prepared.channels;
    let sample_rate = prepared.sample_rate;
    transport
        .native_process_delivery
        .push(native_observations::ProcessDeliverySource {
            node,
            process: target,
            input_sample_rate: sample_rate,
            consumer_sample_rate: bridge.engine_sample_rate,
            prepared_consumer_demand_frames: bridge.max_demand_frames,
            admitted_bridge: bridge,
            measured_source_generation,
            bridge: prepared.observer.clone(),
            consumer: transport
                .streams
                .iter()
                .find(|(stream, _)| stream.master)
                .and_then(|(_, counters)| counters.consumer_events.get().cloned()),
        });
    transport.native_streams.push(NativeStreamObservation {
        node,
        kind: NativeStreamKind::ProcessInput,
        timing: None,
        shared_period: None,
        render_queue: None,
        input_queue: Some(bridge.prepare().map_err(unsupported)?),
        virtual_budget: None,
    });
    transport
        .native_bridges
        .push((node, prepared.observer.clone()));
    transport.auxiliaries.push(Box::new(ProcessOwner {
        capture: Some(prepared.session),
        gate,
        stop,
    }));
    let source = ProcessSource::<S> {
        source: prepared.source,
        scratch: vec![0.0; 512].into_boxed_slice(),
        marker: PhantomData,
    };
    bindings.bind_source(node, source).map_err(unsupported)?;
    transport.streams.push((
        ProfessionalStreamObservation {
            node,
            input: true,
            master: false,
            requested: device,
            // Virtual loopback has no measured hardware device buffer/period.
            actual: NativeStreamInfo {
                sample_rate,
                format: SampleFormat::Float32,
                buffer_frames: 0,
                channels,
            },
            dropped_frames: 0,
            silent_frames: 0,
            native_callbacks: 0,
            native_wait_wakes: 0,
            native_written_frames: 0,
            native_wait_timeouts: 0,
            native_dropped_frames: 0,
            native_silent_frames: 0,
            native_capture_quality: None,
            driver_latency: None,
        },
        Arc::default(),
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asio_defaults_use_driver_preferred_and_one_explicit_side_wins() {
        let defaults = DeviceOptions {
            backend: AudioBackend::Asio,
            ..Default::default()
        };
        assert_eq!(
            asio_session_options(Some(defaults), Some(defaults), 96000).unwrap(),
            (96000, 0)
        );
        let exact = DeviceOptions {
            period_frames: Some(128),
            ..defaults
        };
        assert_eq!(
            asio_session_options(Some(defaults), Some(exact), 44100).unwrap(),
            (44100, 128)
        );
        assert_eq!(
            asio_session_options(Some(exact), Some(defaults), 44100).unwrap(),
            (44100, 128)
        );
        let conflict = DeviceOptions {
            period_frames: Some(256),
            ..defaults
        };
        assert!(asio_session_options(Some(exact), Some(conflict), 48000).is_err());
        assert!(
            asio_session_options(
                Some(exact),
                Some(DeviceOptions {
                    hardware_sample_rate: Some(96000),
                    ..exact
                }),
                48000
            )
            .is_err()
        );
    }
}
