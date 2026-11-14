//! Resolve intent on control, then negotiate every native stream on its owner.
mod absence;
pub(crate) mod observations;
#[cfg(windows)]
mod owner;
#[cfg(any(windows, test))]
mod recovery;

#[cfg(windows)]
pub use observations::{
    ProfessionalObservationSnapshot, ProfessionalObservations, ProfessionalPrepareProgress,
    ProfessionalStreamObservation, StreamEndpoint,
};

use crate::asio::{AsioOptions, AsioStream};
use crate::bridge::{CaptureWriteClock, ProcessSource, RingSink, RingSource, Tone, unsupported};
use crate::native::{NativeStreamInfo, canonical_asio_driver_id};
use crate::resample::StereoResampler;
use crate::virtual_owner::{
    ProjectClock, QpcClockFactory, VirtualBoundaryProvider, VirtualClockFactory,
    VirtualPreparedTransport,
};
use crate::wasapi::{WasapiOptions, WasapiStream, resolve_endpoint};
use absence::{
    AbsenceMonitor, AbsentDevice, AbsentEntry, RuntimeAbsenceReport, RuntimeAbsent,
    RuntimeAbsentMark, StuckOpens,
};
use moiren_core::graph::NodeId;
use moiren_engine::{
    audio_config::{AudioBackend, SAMPLE_RATES, SampleFormat},
    boundary::{CaptureIntent, ClockRole, ConstantSource, DiscardSink},
    compiler::NodeBindings,
    host::{
        BackendError, CancellationToken, EngineBackend, NodeConfig, PrepareMode, PrepareRequest,
        PreparedBackend,
    },
    node::{DeviceMode, DeviceOptions, EndpointSelector, InputSource, OutputTarget},
    sample::ProcessingSample,
};
use observations::Counters;
use owner::{AuxiliaryOwner, Opening, StreamFactory, Transport, WorkerRequest};
use recovery::RecoveryMonitor;
use rtrb::RingBuffer;
use std::{
    marker::PhantomData,
    sync::Arc,
    time::{Duration, Instant},
};
use validation::select_clock;

/// Preparation budget charged per native IO node: 32 bytes per capacity
/// frame — two stereo-f64 (`StereoFrame`) spans, covering the node's queue
/// ring plus the boundary resampler's frame buffer on the render side.
const NATIVE_IO_BUDGET_BYTES_PER_FRAME: usize = 32;
/// Fixed per-IO-node margin on top of the per-frame budget: 128 KiB of
/// headroom above the capacity-scaled estimate.
const NATIVE_IO_BUDGET_FIXED_MARGIN_BYTES: usize = 131_072;

/// Deadline for one native group open (issue #105 decision D1): healthy
/// devices negotiate in well under a second per stage, while a wedged
/// endpoint can block for a minute — this bound converts that hang into a
/// null-stream degradation instead of a frozen session start.
const OPEN_DEADLINE: Duration = Duration::from_secs(10);
/// How often the parallel handshake collector re-checks cancellation while
/// waiting for open settlements.
const OPEN_POLL_SLICE: Duration = Duration::from_millis(20);

#[cfg(test)]
#[path = "engine_backend/tests.rs"]
mod virtual_tests;

mod validation;
mod virtual_io;
use validation::validate;

/// Re-resolves one absent device to its native id (`None` = missing from
/// enumeration right now); production asks the enumerator/registry.
type AbsenceProbe = Arc<dyn Fn(&AbsentDevice) -> Option<String> + Send + Sync>;

pub struct ProfessionalBackend<S: ProcessingSample> {
    observations: ProfessionalObservations,
    recovery: RecoveryMonitor,
    absence: AbsenceMonitor,
    absence_probe: AbsenceProbe,
    /// Endpoints whose open was abandoned past the deadline in this backend
    /// instance's session (D3): re-prepares keep them as null streams until
    /// the device is observed missing or the user applies again.
    stuck: Arc<StuckOpens>,
    /// Endpoints whose RUNNING stream failed with an absence-class error (the
    /// runtime unplug): re-prepares keep them as null streams until the device
    /// is observed missing once — the re-plug signal — or the user applies
    /// again with a fresh backend.
    runtime_absent: Arc<RuntimeAbsent>,
    /// Per-group open deadline; production uses [`OPEN_DEADLINE`].
    open_timeout: Duration,
    virtual_provider: Option<Arc<dyn VirtualBoundaryProvider>>,
    virtual_clock: Arc<dyn VirtualClockFactory>,
    marker: PhantomData<S>,
}
impl<S: ProcessingSample> ProfessionalBackend<S> {
    pub fn new() -> Result<Self, BackendError> {
        Ok(Self {
            observations: ProfessionalObservations::default(),
            recovery: RecoveryMonitor::default(),
            absence: AbsenceMonitor::default(),
            absence_probe: Arc::new(system_endpoint_resolve),
            stuck: Arc::default(),
            runtime_absent: Arc::default(),
            open_timeout: OPEN_DEADLINE,
            virtual_provider: None,
            virtual_clock: Arc::new(QpcClockFactory),
            marker: PhantomData,
        })
    }
    pub fn observations(&self) -> ProfessionalObservations {
        self.observations.clone()
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
    /// Shorten the native open deadline (tests bound abandoned-open
    /// behavior without sleeping for the production bound).
    #[cfg(test)]
    pub(crate) fn with_open_timeout(mut self, timeout: Duration) -> Self {
        self.open_timeout = timeout;
        self
    }
    /// Replace the absent-device return probe (tests flip device presence
    /// without touching the enumerator or the registry). The probe resolves
    /// the selector to its native id — `None` means the device is missing
    /// from enumeration right now.
    #[cfg(test)]
    pub(crate) fn with_absence_probe(mut self, probe: AbsenceProbe) -> Self {
        self.absence_probe = probe;
        self
    }
}

#[derive(Clone)]
struct Direction {
    node: NodeId,
    device: DeviceOptions,
    /// The service-side endpoint identity the graph bound — recorded into
    /// the stream observation so the UI can attribute the stream to a
    /// catalog row. Distinct from the native open id an ASIO group
    /// canonicalizes to.
    endpoint: String,
    /// The selector the graph bound; absence and stuck watches re-resolve
    /// it to detect the device's return.
    selector: EndpointSelector,
}

/// One endpoint that prepares without a native stream: the device is missing
/// from enumeration, or its previous open is still marked stuck (D3).
struct AbsentPending {
    direction: Direction,
    input: bool,
    /// The native id a stuck endpoint last resolved to; `None` for endpoints
    /// that were absent at resolve time.
    native_id: Option<String>,
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

/// Endpoint resolution outcome: a native device id, or absence — the device
/// is missing from the system right now and prepares as a null stream.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolved {
    Native(String),
    Absent,
}

trait NativeDevices {
    fn resolve(
        &self,
        selector: &EndpointSelector,
        device: DeviceOptions,
        input: bool,
    ) -> Result<Resolved, BackendError>;
    fn factory(&self, group: &Group, rate: u32) -> Result<StreamFactory, BackendError>;
}

/// The production return probe: re-resolve one absent device to its native
/// id without any side effects. WASAPI asks the enumerator; ASIO checks the
/// registry. `None` reports the device missing from enumeration right now.
fn system_endpoint_resolve(device: &AbsentDevice) -> Option<String> {
    match device.backend {
        AudioBackend::Wasapi => resolve_endpoint(&device.selector, device.input)
            .ok()
            .flatten(),
        AudioBackend::Asio => match &device.selector {
            EndpointSelector::PinnedEndpoint { endpoint_id, .. } => (|| {
                let id = canonical_asio_driver_id(endpoint_id.as_str()).ok()?;
                crate::asio::list_drivers()
                    .ok()?
                    .iter()
                    .any(|driver| driver.id == id)
                    .then_some(id)
            })(),
            EndpointSelector::FollowDefault(_) => None,
        },
    }
}

struct SystemNativeDevices;
impl NativeDevices for SystemNativeDevices {
    fn resolve(
        &self,
        selector: &EndpointSelector,
        device: DeviceOptions,
        input: bool,
    ) -> Result<Resolved, BackendError> {
        match device.backend {
            AudioBackend::Wasapi => match resolve_endpoint(selector, input)? {
                Some(id) => Ok(Resolved::Native(id)),
                None => Ok(Resolved::Absent),
            },
            AudioBackend::Asio => match selector {
                EndpointSelector::PinnedEndpoint { endpoint_id, .. } => {
                    let id = canonical_asio_driver_id(endpoint_id)?;
                    if crate::asio::list_drivers()?
                        .iter()
                        .any(|driver| driver.id == id)
                    {
                        Ok(Resolved::Native(id))
                    } else {
                        Ok(Resolved::Absent)
                    }
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
        self.recovery
            .poll()
            .or_else(|| {
                self.virtual_provider
                    .as_ref()
                    .and_then(|p| p.poll_resource())
            })
            .or_else(|| {
                let probe = self.absence_probe.clone();
                let stuck = self.stuck.clone();
                let runtime_absent = self.runtime_absent.clone();
                // A device that enumerates has "returned" — unless its open
                // was abandoned past the deadline and it has not been seen
                // missing since (D3), or its running stream died of device
                // removal and no re-plug (a missing sighting) has cleared the
                // verdict yet. The first observation of a missing endpoint
                // clears both verdicts: a re-plug re-created the device
                // instance, so the next sighting may open promptly.
                self.absence.poll(move |device| match probe(device) {
                    Some(id) => {
                        !stuck.is_stuck(device.backend, &id)
                            && !runtime_absent.is_marked(device.backend, &device.selector, &id)
                    }
                    None => {
                        stuck.clear(device.backend, device.native_id.as_deref());
                        runtime_absent.clear_id(device.backend, device.native_id.as_deref());
                        false
                    }
                })
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
        let rate = validate(&request, self.virtual_provider.is_some())?;
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
        let prepared_bytes = io_count
            .checked_mul(
                capacity * NATIVE_IO_BUDGET_BYTES_PER_FRAME + NATIVE_IO_BUDGET_FIXED_MARGIN_BYTES,
            )
            .ok_or_else(|| unsupported("native queue budget overflow"))?;
        if prepared_bytes > request.project.processing.audio_byte_budget {
            return Err(unsupported(
                "professional native queues/SRC exceed the audio preparation budget",
            ));
        }
        let mut transport = Box::new(Transport::<S>::new(
            request.owner_generation,
            self.observations.clone(),
        ));
        let mut groups: Vec<Group> = Vec::new();
        let mut absent: Vec<AbsentPending> = Vec::new();
        let mut processes = Vec::new();
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
                        prepare_process(
                            node,
                            input.device,
                            identity,
                            rate,
                            capacity,
                            &mut bindings,
                            &mut transport,
                            cancel,
                        )?;
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
            let resolved = native.resolve(selector, device, input)?;
            let (id, endpoint) = match resolved {
                Resolved::Absent => {
                    // The device is gone right now: the node keeps its binding
                    // as a silent null stream so the rest of the graph runs,
                    // and the absence watch revives the device when it
                    // returns.
                    let endpoint = match selector {
                        EndpointSelector::PinnedEndpoint { endpoint_id, .. } => endpoint_id.clone(),
                        EndpointSelector::FollowDefault(role) => format!("default ({role:?})"),
                    };
                    log::info!(
                        target: "moiren::audio::professional",
                        "absent native device prepares as a null stream: node={:?} backend={} endpoint={endpoint:?} direction={}",
                        node,
                        device.backend.title(),
                        if input { "input" } else { "output" },
                    );
                    // A device removed mid-run confirms its own absence: the
                    // missing sighting consumes the runtime-unplug verdict and
                    // hands its last native id to the watch for the stale
                    // clears. Plain absent endpoints have no verdict to take.
                    let native_id = self.runtime_absent.take(device.backend, selector);
                    absent.push(AbsentPending {
                        direction: Direction {
                            node,
                            device,
                            endpoint,
                            selector: selector.clone(),
                        },
                        input,
                        native_id,
                    });
                    continue;
                }
                Resolved::Native(id) => {
                    // A present endpoint whose previous open never settled:
                    // keep the stuck verdict (D3) instead of hanging another
                    // bounded open on every re-prepare.
                    if self.stuck.is_stuck(device.backend, &id) {
                        let endpoint = match selector {
                            EndpointSelector::PinnedEndpoint { endpoint_id, .. } => {
                                endpoint_id.clone()
                            }
                            EndpointSelector::FollowDefault(_) => id.clone(),
                        };
                        log::info!(
                            target: "moiren::audio::professional",
                            "stuck native device prepares as a null stream: node={:?} backend={} endpoint={endpoint:?} direction={}",
                            node,
                            device.backend.title(),
                            if input { "input" } else { "output" },
                        );
                        absent.push(AbsentPending {
                            direction: Direction {
                                node,
                                device,
                                endpoint,
                                selector: selector.clone(),
                            },
                            input,
                            native_id: Some(id),
                        });
                        continue;
                    }
                    // A present endpoint whose running stream died of device
                    // removal: keep suppressing re-opens until the device is
                    // observed missing once (the re-plug) or the session is
                    // re-applied — reopening a just-invalidated device would
                    // only re-fail and flap the transport.
                    if self.runtime_absent.is_marked(device.backend, selector, &id) {
                        let endpoint = match selector {
                            EndpointSelector::PinnedEndpoint { endpoint_id, .. } => {
                                endpoint_id.clone()
                            }
                            EndpointSelector::FollowDefault(_) => id.clone(),
                        };
                        log::info!(
                            target: "moiren::audio::professional",
                            "runtime-unplugged native device prepares as a null stream: node={:?} backend={} endpoint={endpoint:?} direction={}",
                            node,
                            device.backend.title(),
                            if input { "input" } else { "output" },
                        );
                        absent.push(AbsentPending {
                            direction: Direction {
                                node,
                                device,
                                endpoint,
                                selector: selector.clone(),
                            },
                            input,
                            native_id: Some(id),
                        });
                        continue;
                    }
                    // The endpoint identity moved on (for example a followed
                    // default that switched devices): a stale verdict pinned
                    // to another native id no longer applies to this selector.
                    self.runtime_absent.take(device.backend, selector);
                    let endpoint = match selector {
                        EndpointSelector::PinnedEndpoint { endpoint_id, .. } => endpoint_id.clone(),
                        // A followed default resolves to a concrete native id, which
                        // is also the catalog row identity.
                        EndpointSelector::FollowDefault(_) => id.clone(),
                    };
                    (id, endpoint)
                }
            };
            let endpoint_id_for_log = id.clone();
            let direction = Direction {
                node,
                device,
                endpoint,
                selector: selector.clone(),
            };
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
        let (mut virtual_sessions, virtual_sinks, virtual_observations) = virtual_io::prepare(
            &request,
            self.virtual_provider.as_deref(),
            rate,
            &mut bindings,
            cancel,
        )?;
        transport.streams.extend(virtual_observations);
        // Open every native group concurrently (issue #105): all workers
        // park before activation and each owns its COM apartment, so the
        // serialization was never load-bearing. Total startup latency is the
        // maximum of the open deadlines instead of their sum, and one slow
        // device no longer delays every other device's start.
        struct PendingOpen {
            group: Group,
            counters: Arc<Counters>,
            capture_rx: Option<rtrb::Consumer<crate::native::StereoFrame>>,
            capture_clock: Option<Arc<CaptureWriteClock>>,
            render_tx: Option<rtrb::Producer<crate::native::StereoFrame>>,
            opening: Opening,
        }
        let mut pending: Vec<PendingOpen> = Vec::with_capacity(groups.len());
        for group in groups {
            let counters = Arc::new(Counters::default());
            let (capture_tx, capture_rx, capture_clock) = if group.input.is_some() {
                let (tx, rx) = RingBuffer::new(capacity);
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
            let worker_request = WorkerRequest {
                factory: native.factory(&group, rate)?,
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
                absence: {
                    // Every direction of the group reports device removal under
                    // its own bound selector; a duplex group marks both.
                    let marks: Vec<RuntimeAbsentMark> = group
                        .input
                        .iter()
                        .chain(&group.output)
                        .map(|direction| RuntimeAbsentMark {
                            backend: direction.device.backend,
                            selector: direction.selector.clone(),
                            native_id: group.id.clone(),
                        })
                        .collect();
                    (!marks.is_empty())
                        .then(|| RuntimeAbsenceReport::new(self.runtime_absent.clone(), marks))
                },
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
            pending.push(PendingOpen {
                group,
                counters,
                capture_rx,
                capture_clock,
                render_tx,
                opening: owner::start_worker(worker_request)?,
            });
        }
        let deadline = Instant::now() + self.open_timeout;
        /// One group whose handshake settled: the negotiated metadata plus
        /// the ring ends the bindings will consume.
        struct OpenedGroup {
            group: Group,
            counters: Arc<Counters>,
            capture_rx: Option<rtrb::Consumer<crate::native::StereoFrame>>,
            capture_clock: Option<Arc<CaptureWriteClock>>,
            render_tx: Option<rtrb::Producer<crate::native::StereoFrame>>,
            info: NativeStreamInfo,
        }
        let mut settled: Vec<OpenedGroup> = Vec::with_capacity(pending.len());
        let groups_total = pending.len();
        if groups_total > 0 {
            self.observations
                .publish_progress(request.owner_generation, groups_total, 0, None);
        }
        while !pending.is_empty() {
            if cancel.is_cancelled() {
                for open in pending {
                    open.opening.abandon();
                }
                return Err(BackendError::Cancelled);
            }
            let now = Instant::now();
            if now >= deadline {
                // Deadline: every still-unsettled group degrades to a null
                // stream (D2) and is marked stuck for this session (D3).
                for open in pending {
                    log::warn!(
                        target: "moiren::audio::professional",
                        "native open timed out after {:?}: backend={} endpoint={:?} node(s)={:?} — preparing a null stream; the endpoint is skipped until it is seen missing or the session is re-applied",
                        self.open_timeout,
                        open.group.backend.title(),
                        open.group.id,
                        open.group
                            .input
                            .iter()
                            .chain(&open.group.output)
                            .map(|direction| direction.node)
                            .collect::<Vec<_>>(),
                    );
                    self.stuck.mark(open.group.backend, &open.group.id);
                    let native_id = open.group.id.clone();
                    let input = open.group.input.clone();
                    let output = open.group.output.clone();
                    // Every node of the group keeps its binding as a null
                    // stream — a timed-out duplex group degrades both sides.
                    for (direction, input) in input
                        .into_iter()
                        .map(|direction| (direction, true))
                        .chain(output.into_iter().map(|direction| (direction, false)))
                    {
                        absent.push(AbsentPending {
                            direction,
                            input,
                            native_id: Some(native_id.clone()),
                        });
                    }
                    open.opening.abandon();
                }
                break;
            }
            let slice = OPEN_POLL_SLICE.min(deadline - now);
            let open = &mut pending[0];
            match open.opening.settle(slice) {
                Some(Ok(info)) => {
                    let open = pending.remove(0);
                    log::info!(
                        target: "moiren::audio::professional",
                        "native device group opened: backend={} endpoint={:?} actual_rate={} actual_format={:?} actual_channels={} actual_buffer_frames={}",
                        open.group.backend.title(),
                        open.group.id,
                        info.sample_rate,
                        info.format,
                        info.channels,
                        info.buffer_frames,
                    );
                    let PendingOpen {
                        group,
                        counters,
                        capture_rx,
                        capture_clock,
                        render_tx,
                        opening,
                    } = open;
                    transport.workers.push(opening.into_worker());
                    settled.push(OpenedGroup {
                        group,
                        counters,
                        capture_rx,
                        capture_clock,
                        render_tx,
                        info,
                    });
                    self.observations.publish_progress(
                        request.owner_generation,
                        groups_total,
                        settled.len(),
                        pending.first().map(|next| next.group.id.clone()),
                    );
                }
                Some(Err(error)) => {
                    // A hard open failure fails the whole preparation exactly
                    // as before: settled workers unwind through the dropped
                    // transport, unsettled ones are abandoned.
                    for open in pending.drain(..) {
                        open.opening.abandon();
                    }
                    return Err(error);
                }
                None => {
                    // Fair rotation: poll the next unsettled group.
                    pending.rotate_left(1);
                }
            }
        }
        self.observations.clear_progress(request.owner_generation);
        let clock = select_clock(
            &request,
            &absent
                .iter()
                .map(|pending| pending.direction.node)
                .collect::<Vec<_>>(),
        )?;
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
        for OpenedGroup {
            group,
            counters,
            capture_rx,
            capture_clock,
            render_tx,
            info,
        } in settled
        {
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
                bindings
                    .bind_source(
                        input.node,
                        RingSource::<S>::new(
                            capture_rx.unwrap(),
                            info.sample_rate,
                            rate,
                            info.buffer_frames as usize,
                            capacity,
                            counters.clone(),
                            capture_clock.unwrap(),
                        )?,
                    )
                    .map_err(unsupported)?;
                transport
                    .streams
                    .push((observation(input, true, false, info), counters.clone()));
            }
            if let Some(output) = &group.output {
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
        // Absent endpoints keep their graph bindings as null streams: a
        // constant silence source for capture, a discard sink for render.
        // No worker, ring, or native session backs them — the engine runs
        // the compiled graph unchanged and these boundaries transfer
        // nothing. Under a physical clock an absent render node joins the
        // transport's sink bookkeeping like a virtual sink; the engine
        // itself writes it during every render.
        let mut absent_entries = Vec::with_capacity(absent.len());
        let mut absent_virtual_sinks = Vec::new();
        for pending in absent {
            let AbsentPending {
                direction,
                input,
                native_id,
            } = pending;
            if input {
                bindings
                    .bind_source(
                        direction.node,
                        ConstantSource {
                            channels: 2,
                            value: 0.0,
                        },
                    )
                    .map_err(unsupported)?;
            } else {
                bindings
                    .bind_sink(direction.node, DiscardSink { channels: 2 })
                    .map_err(unsupported)?;
                if matches!(clock, ProjectClock::Physical(_)) {
                    transport.sinks.push(direction.node);
                } else {
                    // The virtual owner must protect every graph sink; an
                    // absent discard sink is bookkept like a virtual one.
                    absent_virtual_sinks.push(direction.node);
                }
            }
            transport.streams.push((
                absent_observation(&direction, input, rate),
                Arc::new(Counters::default()),
            ));
            absent_entries.push(AbsentEntry {
                node: direction.node,
                configuration: request.project.nodes[&direction.node].clone(),
                device: AbsentDevice {
                    selector: direction.selector,
                    backend: direction.device.backend,
                    input,
                    native_id,
                },
            });
        }
        self.absence
            .register(request.owner_generation, absent_entries);
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
            let mut virtual_sinks = virtual_sinks;
            virtual_sinks.extend(absent_virtual_sinks);
            return Ok(PreparedBackend {
                bindings,
                transport: Some(Box::new(VirtualPreparedTransport::new(
                    virtual_sinks,
                    virtual_sessions,
                    stop,
                    gate,
                    clock,
                ))),
            });
        }
        Ok(PreparedBackend {
            bindings,
            transport: Some(transport),
        })
    }
}
fn observation(
    direction: &Direction,
    input: bool,
    master: bool,
    actual: NativeStreamInfo,
) -> ProfessionalStreamObservation {
    ProfessionalStreamObservation {
        node: direction.node,
        endpoint: StreamEndpoint::Physical(direction.endpoint.clone()),
        input,
        master,
        requested: direction.device,
        actual,
        dropped_frames: 0,
        silent_frames: 0,
        xruns: 0,
        native_callbacks: 0,
        native_dropped_frames: 0,
        native_silent_frames: 0,
        native_xruns: 0,
        engine_blocks: 0,
        engine_busy_100ns: 0,
        engine_peak_block_100ns: 0,
        driver_latency: None,
        absent: false,
    }
}

/// Null-stream observation: the requested settings with no device behind
/// them, `buffer_frames = 0` like a virtual boundary.
fn absent_observation(
    direction: &Direction,
    input: bool,
    rate: u32,
) -> ProfessionalStreamObservation {
    ProfessionalStreamObservation {
        node: direction.node,
        endpoint: StreamEndpoint::Physical(direction.endpoint.clone()),
        input,
        master: false,
        requested: direction.device,
        actual: NativeStreamInfo {
            sample_rate: direction.device.hardware_sample_rate.unwrap_or(rate),
            format: direction.device.sample_format,
            buffer_frames: 0,
            channels: 2,
        },
        dropped_frames: 0,
        silent_frames: 0,
        xruns: 0,
        native_callbacks: 0,
        native_dropped_frames: 0,
        native_silent_frames: 0,
        native_xruns: 0,
        engine_blocks: 0,
        engine_busy_100ns: 0,
        engine_peak_block_100ns: 0,
        driver_latency: None,
        absent: true,
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
    fn join(&mut self, deadline: Instant) -> Result<(), BackendError> {
        if let Some(capture) = self.capture.take() {
            // The loopback worker's COM wind-down gets the same teardown
            // bound as the native streams: a wedged release detaches the
            // worker instead of parking the cleanup lane (issue #105).
            let report = capture.join_within(deadline).map_err(|error| {
                if matches!(error, crate::capture::CaptureError::TeardownTimeout) {
                    BackendError::ResourceUnavailable(
                        "process capture worker did not finish teardown within its deadline".into(),
                    )
                } else {
                    unsupported(error)
                }
            })?;
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
    engine_rate: u32,
    capacity: usize,
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
            target,
            duration: crate::SessionDuration::UntilStopped,
            latency_telemetry: crate::latency::TelemetryMode::Summary,
        },
        stop.clone(),
        gate.clone(),
    )
    .map_err(unsupported)?;
    let channels = prepared.channels;
    let sample_rate = prepared.sample_rate;
    transport.auxiliaries.push(Box::new(ProcessOwner {
        capture: Some(prepared.session),
        gate,
        stop,
    }));
    let source = ProcessSource::<S> {
        source: prepared.source,
        src: StereoResampler::new(48000, engine_rate, capacity).map_err(unsupported)?,
        scratch: vec![0.0; 512].into_boxed_slice(),
        marker: PhantomData,
    };
    bindings.bind_source(node, source).map_err(unsupported)?;
    transport.streams.push((
        ProfessionalStreamObservation {
            node,
            endpoint: StreamEndpoint::Application {
                pid: identity.0,
                creation_time_100ns: identity.1,
            },
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
            xruns: 0,
            native_callbacks: 0,
            native_dropped_frames: 0,
            native_silent_frames: 0,
            native_xruns: 0,
            engine_blocks: 0,
            engine_busy_100ns: 0,
            engine_peak_block_100ns: 0,
            driver_latency: None,
            absent: false,
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
