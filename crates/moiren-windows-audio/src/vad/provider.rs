//! Windows VAD transport adapter.
//!
//! The adapter owns the control session and keeps endpoint lifetimes separate
//! from native client lifetimes. A raw PDO may be Present while no client has
//! opened a stream; that is an idle, usable state, not a transport failure.

use super::{
    Channels, ClientError, ControlClient, DeviceInterface, Intent, PcmCompletion,
    PcmMicrophoneBridge, PcmOwner, PcmPlaybackBridge, PcmTicket, PdoState, ResidualPcm,
    StableEndpointId, StreamState, discover_present_interfaces,
};
use crate::virtual_owner::{
    BoundaryProbe, PreparedVirtualMicrophone, PreparedVirtualPlayback, VirtualBoundaryProvider,
    VirtualBoundaryRequest, VirtualResourceSession,
};
use moiren_engine::{
    boundary::{AudioReader, AudioWriter, audio_bridge},
    host::{BackendError, CancellationToken, OwnerStatus},
    node::VirtualEndpointId,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const REMOVE_TIMEOUT: Duration = Duration::from_secs(15);
const PCM_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
/// Worker-side retire bound: cancel, collect what a healthy driver returns
/// within two packet periods, then detach the rest. A driver holding
/// cancelled requests through its own gate cycle (the 2026-10-23 night1
/// outage) must not freeze the only thread pumping this endpoint.
const PCM_RETIRE_QUICK_DRAIN: Duration = Duration::from_millis(20);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

mod lifecycle;
mod pcm_worker;
mod telemetry;
mod transport;
use lifecycle::{Admission, ExportSpec, ExportView, Lifecycle};
use telemetry::PcmTelemetry;
pub use telemetry::VadPcmTelemetrySnapshot;

/// Control-plane state for one VAD export.
///
/// This is a detached snapshot: callers may retain it without holding the
/// provider lock, and a missing snapshot means that CREATE is still uncertain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VadExportObservation {
    pub endpoint_id: VirtualEndpointId,
    pub spec_name: String,
    pub intent: Intent,
    pub channels: Channels,
    pub allow_default: bool,
    pub snapshot: Option<super::EndpointSnapshot>,
    pub removing: bool,
    pub transport_prepared: bool,
    pub telemetry: Option<VadPcmTelemetrySnapshot>,
    pub error: Option<String>,
}

impl From<ExportView> for VadExportObservation {
    fn from(view: ExportView) -> Self {
        let ExportView {
            spec,
            snapshot,
            removing,
            transport_prepared,
            telemetry,
            error,
            ..
        } = view;
        Self {
            endpoint_id: spec.id,
            spec_name: spec.name,
            intent: spec.intent,
            channels: spec.channels,
            allow_default: spec.allow_default,
            snapshot,
            removing,
            transport_prepared,
            telemetry,
            error,
        }
    }
}

#[derive(Clone, Copy)]
struct TransportEndpoint {
    snapshot: super::EndpointSnapshot,
}

/// One driver file object shared by lifecycle jobs and Engine PCM workers.
/// Native operations belong on control workers. `set_active_revision` only
/// invalidates atomic admission and is safe on the app service pump.
pub struct WindowsVadProvider {
    state: Arc<Mutex<Lifecycle<ControlClient>>>,
    admission: Arc<Admission>,
}

impl WindowsVadProvider {
    pub fn discover() -> Result<Arc<Self>, BackendError> {
        let interfaces = discover_present_interfaces()
            .map_err(|error| BackendError::ResourceUnavailable(error.to_string()))?;
        let mut last_error = None;
        for interface in interfaces {
            match Self::from_interface(&interface) {
                Ok(provider) => return Ok(provider),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            BackendError::ResourceUnavailable("no present Moiren VAD control interface".into())
        }))
    }

    pub fn from_interface(interface: &DeviceInterface) -> Result<Arc<Self>, BackendError> {
        let control = ControlClient::connect(interface).map_err(client_error)?;
        control
            .capabilities()
            .require_pcm()
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        Ok(Arc::new(Self {
            state: Arc::new(Mutex::new(Lifecycle::new(control))),
            admission: Arc::new(Admission::new()),
        }))
    }

    /// Invalidate every outstanding gate dispatch before changing owner,
    /// configuration or confirmed revision. This never performs native IO.
    pub fn set_active_revision(&self, revision: Option<u64>) -> u64 {
        self.admission.set(revision)
    }

    pub fn create_export(
        &self,
        id: VirtualEndpointId,
        intent: Intent,
        channels: Channels,
        name: &str,
        allow_default: bool,
    ) -> Result<(), BackendError> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .create(ExportSpec {
                id,
                intent,
                channels,
                name: name.into(),
                allow_default,
            })
    }

    /// Poll actual driver state, retaining uncertain/removing records. Call
    /// only from the lifecycle worker, never the service pump or Engine render.
    pub fn export_observations(&self) -> Vec<VadExportObservation> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .poll()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    pub fn set_export_gate(
        &self,
        id: VirtualEndpointId,
        expected: [u64; 5],
        open: bool,
        revision: u64,
        ticket: u64,
    ) -> Result<(), BackendError> {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).gate(
            id,
            expected,
            open,
            revision,
            ticket,
            &self.admission,
        )
    }

    pub fn retire_export(
        &self,
        id: VirtualEndpointId,
        expected: [u64; 5],
    ) -> Result<(), BackendError> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retire(id, expected)
    }

    /// Compatibility helper for explicit shutdown callers. Failed removals
    /// retain their identities; Stop must not call this method.
    pub fn retire_all(&self) -> Result<(), BackendError> {
        self.set_active_revision(None);
        let deadline = std::time::Instant::now() + REMOVE_TIMEOUT;
        loop {
            let views = self.export_observations();
            if views
                .iter()
                .all(|v| v.snapshot.is_some_and(|s| s.state() == PdoState::Removed))
            {
                return Ok(());
            }
            for view in views {
                let snapshot = view.snapshot.ok_or_else(|| {
                    BackendError::ResourceUnavailable(
                        "uncertain CREATE still belongs to the control session".into(),
                    )
                })?;
                if snapshot.state() == PdoState::Removed {
                    continue;
                }
                if snapshot.gate_open() {
                    self.set_export_gate(
                        view.endpoint_id,
                        lifecycle::stamp(snapshot),
                        false,
                        0,
                        0,
                    )?;
                } else if snapshot.state() != PdoState::Removing {
                    self.retire_export(view.endpoint_id, lifecycle::stamp(snapshot))?;
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(BackendError::ResourceUnavailable(
                    "VAD retirement is still pending".into(),
                ));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn endpoint(
        &self,
        request: VirtualBoundaryRequest,
        intent: Intent,
        cancel: &CancellationToken,
    ) -> Result<(ControlClient, TransportEndpoint, Arc<SessionState>), BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let control = state.control.duplicate();
        let record = state
            .endpoints
            .get_mut(&request.endpoint_id)
            .ok_or_else(|| {
                BackendError::ResourceUnavailable(
                    "export has not been created by the lifecycle adapter".into(),
                )
            })?;
        record
            .sessions
            .retain(|s| !s.drained.load(Ordering::Acquire));
        let snapshot = record
            .snapshot
            .filter(|s| s.state() == PdoState::Present)
            .ok_or_else(|| BackendError::ResourceUnavailable("export is not Present".into()))?;
        if record.removing
            || record.error.is_some()
            || record.spec.intent != intent
            || record.spec.channels as usize != request.channels
            || !record.sessions.is_empty()
        {
            return Err(BackendError::ResourceUnavailable(
                "export transport is retiring, occupied or incompatible".into(),
            ));
        }
        let session = Arc::new(SessionState::new(self.admission.clone()));
        record.sessions.push(session.clone());
        Ok((control, TransportEndpoint { snapshot }, session))
    }
}

fn client_error(error: ClientError) -> BackendError {
    match error {
        ClientError::DeadlineExceeded => {
            BackendError::ResourceUnavailable("VAD control operation timed out".into())
        }
        ClientError::PreparationFailed { .. } => {
            BackendError::ResourceUnavailable(error.to_string())
        }
        ClientError::Windows { .. } | ClientError::Io { .. } => {
            BackendError::ResourceUnavailable(error.to_string())
        }
        ClientError::SessionUncertain => BackendError::Failed(error.to_string()),
        // The pump loops consume quota rejections in-pass; one reaching here
        // means a deferral site was missed, which is a fault, not load.
        ClientError::KernelPcmQuotaFull => BackendError::Failed(error.to_string()),
        _ => BackendError::Unsupported(error.to_string()),
    }
}

impl VirtualBoundaryProvider for WindowsVadProvider {
    fn prepare_playback(
        &self,
        request: VirtualBoundaryRequest,
        cancel: &CancellationToken,
    ) -> Result<PreparedVirtualPlayback, BackendError> {
        let capacity = carrier_capacity(request)?;
        let channels = Channels::try_from(request.channels as u32)
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;

        let (writer, reader) = audio_bridge(2, capacity, request.audio_byte_budget)
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        let (control, endpoint, state) = self.endpoint(request, Intent::Playback, cancel)?;
        let session = VadSession::spawn(
            WorkerConfig::Playback {
                control,
                endpoint,
                channels,
                writer,
                byte_budget: request.audio_byte_budget,
                buffer: request.buffer,
            },
            state,
        )?;
        Ok(PreparedVirtualPlayback {
            carrier: reader,
            reset_epoch: session.reset_epoch(),
            producer_quiesced_epoch: session.state.producer_quiesced_epoch.clone(),
            consumer_quiesced_epoch: session.state.consumer_quiesced_epoch.clone(),
            probe: Arc::new(BoundaryProbe::default()),
            session: Box::new(session),
        })
    }

    fn prepare_microphone(
        &self,
        request: VirtualBoundaryRequest,
        cancel: &CancellationToken,
    ) -> Result<PreparedVirtualMicrophone, BackendError> {
        let capacity = carrier_capacity(request)?;
        let channels = Channels::try_from(request.channels as u32)
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;

        let (writer, reader) = audio_bridge(2, capacity, request.audio_byte_budget)
            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
        let (control, endpoint, state) = self.endpoint(request, Intent::Microphone, cancel)?;
        let session = VadSession::spawn(
            WorkerConfig::Microphone {
                control,
                endpoint,
                channels,
                reader,
                byte_budget: request.audio_byte_budget,
                buffer: request.buffer,
            },
            state,
        )?;
        Ok(PreparedVirtualMicrophone {
            carrier: writer,
            reset_epoch: session.reset_epoch(),
            producer_quiesced_epoch: session.state.producer_quiesced_epoch.clone(),
            consumer_quiesced_epoch: session.state.consumer_quiesced_epoch.clone(),
            probe: Arc::new(BoundaryProbe::default()),
            session: Box::new(session),
        })
    }
}

fn carrier_capacity(request: VirtualBoundaryRequest) -> Result<usize, BackendError> {
    if !(1..=2).contains(&request.channels)
        || !(44_100..=96_000).contains(&request.engine_sample_rate)
        || !(1..=4096).contains(&request.max_block_frames)
        || request.audio_byte_budget == 0
    {
        return Err(BackendError::Unsupported(
            "invalid VAD transport preparation shape".into(),
        ));
    }
    let native_frames = (request.max_block_frames as u64 * 48_000)
        .div_ceil(u64::from(request.engine_sample_rate)) as usize;
    if let Some(queue) = request
        .buffer
        .prepare(request.engine_sample_rate, request.max_demand_frames)
        .map_err(|error| BackendError::Unsupported(error.to_string()))?
    {
        return Ok(queue.capacity_frames as usize);
    }
    Ok((native_frames + 32).max(super::MAX_PACKET_FRAMES as usize))
}

enum WorkerConfig {
    Playback {
        control: ControlClient,
        endpoint: TransportEndpoint,
        channels: Channels,
        writer: AudioWriter<f32>,
        byte_budget: usize,
        buffer: crate::virtual_owner::VirtualBufferOptions,
    },
    Microphone {
        control: ControlClient,
        endpoint: TransportEndpoint,
        channels: Channels,
        reader: AudioReader<f32>,
        byte_budget: usize,
        buffer: crate::virtual_owner::VirtualBufferOptions,
    },
}

struct SessionState {
    active: std::sync::atomic::AtomicBool,
    status: std::sync::atomic::AtomicU8,
    cancel: std::sync::atomic::AtomicBool,
    failure: Mutex<Option<BackendError>>,
    reset_epoch: Arc<AtomicU64>,
    producer_quiesced_epoch: Arc<AtomicU64>,
    consumer_quiesced_epoch: Arc<AtomicU64>,
    drained: std::sync::atomic::AtomicBool,
    admitted_ticket: AtomicU64,
    admitted_revision: AtomicU64,
    admission: Arc<Admission>,
    telemetry: Arc<PcmTelemetry>,
}

impl SessionState {
    fn new(admission: Arc<Admission>) -> Self {
        Self {
            active: std::sync::atomic::AtomicBool::new(false),
            status: std::sync::atomic::AtomicU8::new(0),
            cancel: std::sync::atomic::AtomicBool::new(false),
            failure: Mutex::new(None),
            reset_epoch: Arc::new(AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            drained: std::sync::atomic::AtomicBool::new(false),
            admitted_ticket: AtomicU64::new(0),
            admitted_revision: AtomicU64::new(0),
            admission,
            telemetry: Arc::new(PcmTelemetry::default()),
        }
    }
    fn prepared(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && !self.cancel.load(Ordering::Acquire)
            && !self.drained.load(Ordering::Acquire)
    }
    fn admitted(&self) -> bool {
        self.prepared()
            && self.admission.matches(
                self.admitted_ticket.load(Ordering::Acquire),
                self.admitted_revision.load(Ordering::Acquire),
            )
    }
    fn revoke(&self) {
        self.admitted_revision.store(0, Ordering::Release);
    }

    fn telemetry_snapshot(&self) -> VadPcmTelemetrySnapshot {
        self.telemetry.snapshot()
    }
}

struct VadSession {
    state: Arc<SessionState>,
    worker: Option<JoinHandle<()>>,
}

impl VadSession {
    fn spawn(config: WorkerConfig, state: Arc<SessionState>) -> Result<Self, BackendError> {
        let worker_state = state.clone();
        let worker = thread::Builder::new()
            .name("moiren-vad-pcm".into())
            .spawn(move || run_worker(config, worker_state))
            .map_err(|error| {
                state.drained.store(true, Ordering::Release);
                BackendError::Failed(format!("start VAD worker: {error}"))
            })?;
        Ok(Self {
            state,
            worker: Some(worker),
        })
    }

    fn reset_epoch(&self) -> Arc<AtomicU64> {
        self.state.reset_epoch.clone()
    }
}

impl VirtualResourceSession for VadSession {
    fn activate(&self) -> Result<(), BackendError> {
        if self.state.status.load(std::sync::atomic::Ordering::Acquire) == 2 {
            return Err(self
                .state
                .failure
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone()
                .unwrap_or_else(|| BackendError::ResourceUnavailable("VAD worker failed".into())));
        }
        self.state
            .active
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    fn cancel(&self) {
        self.state.revoke();
        self.state
            .cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn status(&self) -> OwnerStatus {
        match self.state.status.load(std::sync::atomic::Ordering::Acquire) {
            0 => OwnerStatus::Starting,
            1 => OwnerStatus::Running,
            2 => OwnerStatus::Failed(
                self.state
                    .failure
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .clone()
                    .unwrap_or_else(|| {
                        BackendError::ResourceUnavailable("VAD worker failed".into())
                    }),
            ),
            _ => OwnerStatus::Starting,
        }
    }

    fn stop_and_join(mut self: Box<Self>) -> Result<(), BackendError> {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| BackendError::WorkerPanicked)?;
        }
        match self.status() {
            OwnerStatus::Failed(error) => Err(error),
            _ => Ok(()),
        }
    }
}

impl Drop for VadSession {
    fn drop(&mut self) {
        // Preparation can fail before the session enters VirtualSessions.
        // Its control-side owner must still cancel and drain this worker.
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(config: WorkerConfig, state: Arc<SessionState>) {
    // Match the physical paths' scheduling class: this thread paces bounded
    // PCM IO against native deadlines. Registration failure is non-fatal.
    let _mmcss = crate::owner::Mmcss::audio();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match config {
        WorkerConfig::Playback {
            control,
            endpoint,
            channels,
            writer,
            byte_budget,
            buffer,
        } => run_playback(
            control,
            endpoint,
            channels,
            Some(writer),
            byte_budget,
            buffer,
            &state,
        ),
        WorkerConfig::Microphone {
            control,
            endpoint,
            channels,
            reader,
            byte_budget,
            buffer,
        } => run_microphone(
            control,
            endpoint,
            channels,
            Some(reader),
            byte_budget,
            buffer,
            &state,
        ),
    }))
    .unwrap_or(Err(BackendError::WorkerPanicked));
    state.revoke();
    if let Err(error) = result {
        *state
            .failure
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(error);
        state.status.store(2, std::sync::atomic::Ordering::Release);
    }
    // run_* (or unwinding) has dropped/drained every overlapped PCM slot.
    state.drained.store(true, Ordering::Release);
}

fn run_playback(
    mut control: ControlClient,
    endpoint: TransportEndpoint,
    channels: Channels,
    mut writer: Option<AudioWriter<f32>>,
    byte_budget: usize,
    buffer: crate::virtual_owner::VirtualBufferOptions,
    state: &SessionState,
) -> Result<(), BackendError> {
    let mut current = endpoint.snapshot;
    let mut owner: Option<PcmOwner> = None;
    let mut bridge = None;
    let mut tickets = Vec::with_capacity(super::REQUEST_SLOTS);
    let mut order = pcm_worker::PacketOrder::default();
    let mut retired = RetiredPcm::new();
    // A newly prepared carrier belongs to epoch zero, even when a reset was
    // requested before this worker started. Never skip that first handshake.
    let mut observed_reset = 0;
    let result = (|| {
        while !state.cancel.load(std::sync::atomic::Ordering::Acquire) {
            if state.active.load(std::sync::atomic::Ordering::Acquire) {
                state.status.store(1, std::sync::atomic::Ordering::Release);
                let (queried, _) = transport::poll(&mut control, current, state)?;
                current = queried;
                if !pcm_worker::quiesce_playback(state, &mut observed_reset, || {
                    teardown_playback(
                        &mut retired,
                        &mut owner,
                        &mut bridge,
                        &mut writer,
                        &mut tickets,
                    )
                }) {
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }
                if current.stream_state() == StreamState::Running
                    && current.gate_open()
                    && state.admitted()
                {
                    if owner.is_none() {
                        let shape = current
                            .pcm_shape(control.capabilities())
                            .map_err(|e| client_error(e.into()))?;
                        buffer
                            .admit_native_packet(shape.frames())
                            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
                        bridge = Some(PcmPlaybackBridge::new(
                            channels,
                            shape.format(),
                            shape.frames() as usize,
                            byte_budget,
                            writer.take().expect("playback carrier is free"),
                        )?);
                        owner = Some(control.pcm_owner(current).map_err(client_error)?);
                        tickets.clear();
                        order = pcm_worker::PacketOrder::default();
                    }
                    match pcm_worker::playback(
                        owner.as_mut().expect("playback owner"),
                        bridge.as_mut().expect("playback bridge"),
                        &mut tickets,
                        &mut order,
                        state,
                        observed_reset,
                    ) {
                        Ok(pcm_worker::PumpState::Current) => {
                            if let Err(error) = pcm_worker::wait_for_activity(
                                owner.as_mut().expect("playback owner"),
                                &tickets,
                                state,
                                observed_reset,
                            ) {
                                current =
                                    transport::after_io_error(&mut control, current, state, error)?;
                                teardown_playback(
                                    &mut retired,
                                    &mut owner,
                                    &mut bridge,
                                    &mut writer,
                                    &mut tickets,
                                );
                            }
                            continue;
                        }
                        result => {
                            if let Err(error) = result {
                                current =
                                    transport::after_io_error(&mut control, current, state, error)?;
                            }
                            teardown_playback(
                                &mut retired,
                                &mut owner,
                                &mut bridge,
                                &mut writer,
                                &mut tickets,
                            );
                        }
                    }
                } else {
                    teardown_playback(
                        &mut retired,
                        &mut owner,
                        &mut bridge,
                        &mut writer,
                        &mut tickets,
                    );
                }
            } else {
                teardown_playback(
                    &mut retired,
                    &mut owner,
                    &mut bridge,
                    &mut writer,
                    &mut tickets,
                );
            }
            retired.poll();
            thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    })();
    state.revoke();
    let close_result = close_worker_gate(&mut control, &mut current);
    teardown_playback(
        &mut retired,
        &mut owner,
        &mut bridge,
        &mut writer,
        &mut tickets,
    );
    retired.finish();
    result.and(close_result)
}

/// Parked retired PCM owners whose cancelled requests the driver has not yet
/// completed. Stack-allocated inside each transport worker; `poll` runs once
/// per pump iteration and is a no-op when empty. No thread, lock, or heap
/// allocation exists anywhere on this path.
///
/// Capacity two bounds the pathological case: a fresh retire while two
/// generations of requests are still stuck falls back to the bounded blocking
/// drain for the oldest (the pre-fix behavior), keeping the park finite.
/// Each entry carries the retire age so eviction always selects the true
/// oldest generation once `poll` starts recycling slots.
struct RetiredPcm {
    parked: [(Option<ResidualPcm>, u64); RETIRE_PARK_SLOTS],
    next_age: u64,
}

const RETIRE_PARK_SLOTS: usize = 2;

impl RetiredPcm {
    fn new() -> Self {
        Self {
            parked: std::array::from_fn(|_| (None, 0)),
            next_age: 0,
        }
    }

    /// Index of the oldest parked generation. Ages persist across `poll`
    /// recycling, so a freed-then-refilled slot never masquerades as oldest.
    fn oldest(&self) -> usize {
        if self.parked[0].1 <= self.parked[1].1 {
            0
        } else {
            1
        }
    }

    /// Retire `owner`, parking any residual. The only slow path is a full
    /// park: the oldest residual then gets the full bounded budget and its
    /// Drop's final safety drain, and the new residual takes its slot.
    fn retire(&mut self, owner: PcmOwner, endpoint: &'static str) {
        let age = self.next_age;
        self.next_age = self.next_age.wrapping_add(1);
        let mut residual = None;
        if owner.retire(PCM_RETIRE_QUICK_DRAIN, &mut residual) {
            return;
        }
        let Some(residual) = residual else {
            return;
        };
        if let Some(index) = self.parked.iter().position(|(slot, _)| slot.is_none()) {
            self.parked[index] = (Some(residual), age);
        } else {
            // Two residual generations already outstanding: the driver has
            // stalled every recovery for the quick bound twice over. Fall
            // back to the pre-fix bounded blocking drain for the oldest.
            let victim = self.oldest();
            let (mut oldest, _) = std::mem::take(&mut self.parked[victim]);
            if let Some(oldest) = oldest.as_mut() {
                log::warn!(
                    target: "moiren::audio::vad",
                    "event=pcm_retire_park_full endpoint={endpoint} oldest_pending={}",
                    oldest.pending()
                );
                if !oldest.drain(PCM_SHUTDOWN_TIMEOUT) {
                    log::warn!(
                        target: "moiren::audio::vad",
                        "event=pcm_retire_park_full_timeout endpoint={endpoint}"
                    );
                }
            }
            self.parked[victim] = (Some(residual), age);
            return;
        }
        log::debug!(
            target: "moiren::audio::vad",
            "event=pcm_retire_parked endpoint={endpoint} pending={}",
            self.parked
                .iter()
                .filter_map(|(residual, _)| residual.as_ref())
                .map(|residual| residual.pending())
                .sum::<usize>()
        );
    }

    /// Non-blocking collect pass over every parked residual.
    fn poll(&mut self) {
        for (slot, _) in &mut self.parked {
            if let Some(residual) = slot
                && residual.drain(Duration::ZERO)
            {
                *slot = None;
            }
        }
    }

    /// Worker-exit drain: bounded collect, then Drop's final safety drain.
    fn finish(&mut self) {
        for (slot, _) in &mut self.parked {
            if let Some(mut residual) = slot.take() {
                if !residual.drain(PCM_SHUTDOWN_TIMEOUT) {
                    log::warn!(
                        target: "moiren::audio::vad",
                        "event=pcm_retire_exit_timeout pending={}",
                        residual.pending()
                    );
                }
                let _ = residual;
            }
        }
    }
}

fn teardown_playback(
    retired: &mut RetiredPcm,
    owner: &mut Option<PcmOwner>,
    bridge: &mut Option<PcmPlaybackBridge>,
    writer: &mut Option<AudioWriter<f32>>,
    tickets: &mut Vec<PcmTicket>,
) {
    if let Some(pcm) = owner.take() {
        retired.retire(pcm, "playback");
    }
    tickets.clear();
    if let Some(bridge) = bridge.take() {
        *writer = Some(bridge.into_carrier());
    }
}

fn run_microphone(
    mut control: ControlClient,
    endpoint: TransportEndpoint,
    channels: Channels,
    mut reader: Option<AudioReader<f32>>,
    byte_budget: usize,
    buffer: crate::virtual_owner::VirtualBufferOptions,
    state: &SessionState,
) -> Result<(), BackendError> {
    let mut current = endpoint.snapshot;
    let mut owner: Option<PcmOwner> = None;
    let mut bridge = None;
    let mut tickets = Vec::with_capacity(super::REQUEST_SLOTS);
    let mut pump = None;
    let mut observed_reset = 0;
    let mut reprime_after_reset = false;
    let mut retired = RetiredPcm::new();
    let result = (|| {
        while !state.cancel.load(std::sync::atomic::Ordering::Acquire) {
            if state.active.load(std::sync::atomic::Ordering::Acquire) {
                state.status.store(1, std::sync::atomic::Ordering::Release);
                let (queried, _) = transport::poll(&mut control, current, state)?;
                current = queried;
                if state.reset_epoch.load(Ordering::Acquire) != observed_reset {
                    teardown_microphone(
                        &mut retired,
                        &mut owner,
                        &mut bridge,
                        &mut reader,
                        &mut tickets,
                    );
                    if !pcm_worker::quiesce_microphone(
                        state,
                        &mut observed_reset,
                        reader.as_mut().expect("microphone carrier is free"),
                    ) {
                        thread::sleep(POLL_INTERVAL);
                        continue;
                    }
                    // A completed fence re-primes the microphone carrier
                    // before the first fresh submission: a bounded native-
                    // side wait instead of ~20 s of partial silence (see
                    // MicrophonePump::arm_reprime).
                    reprime_after_reset = true;
                }
                if current.stream_state() == StreamState::Running
                    && current.gate_open()
                    && state.admitted()
                {
                    if owner.is_none() {
                        let shape = current
                            .pcm_shape(control.capabilities())
                            .map_err(|e| client_error(e.into()))?;
                        buffer
                            .admit_native_packet(shape.frames())
                            .map_err(|error| BackendError::Unsupported(error.to_string()))?;
                        bridge = Some(PcmMicrophoneBridge::new(
                            channels,
                            shape.format(),
                            shape.frames() as usize,
                            byte_budget,
                            reader.take().expect("microphone carrier is free"),
                        )?);
                        owner = Some(control.pcm_owner(current).map_err(client_error)?);
                        tickets.clear();
                        let mut fresh = pcm_worker::MicrophonePump::new(shape.frames());
                        if reprime_after_reset {
                            fresh.arm_reprime(Instant::now());
                            reprime_after_reset = false;
                        }
                        pump = Some(fresh);
                    }
                    match pcm_worker::microphone(
                        owner.as_mut().expect("microphone owner"),
                        bridge.as_mut().expect("microphone bridge"),
                        &mut tickets,
                        pump.as_mut().expect("microphone pump"),
                        state,
                        observed_reset,
                        Instant::now,
                    ) {
                        Ok(pcm_worker::PumpState::Current) => {
                            if let Err(error) = pcm_worker::wait_for_microphone(
                                owner.as_mut().expect("microphone owner"),
                                &tickets,
                                pump.as_mut().expect("microphone pump"),
                                state,
                                observed_reset,
                                Instant::now(),
                                thread::sleep,
                            ) {
                                current =
                                    transport::after_io_error(&mut control, current, state, error)?;
                                teardown_microphone(
                                    &mut retired,
                                    &mut owner,
                                    &mut bridge,
                                    &mut reader,
                                    &mut tickets,
                                );
                            }
                            continue;
                        }
                        result => {
                            if let Err(error) = result {
                                current =
                                    transport::after_io_error(&mut control, current, state, error)?;
                            }
                            teardown_microphone(
                                &mut retired,
                                &mut owner,
                                &mut bridge,
                                &mut reader,
                                &mut tickets,
                            );
                        }
                    }
                } else {
                    teardown_microphone(
                        &mut retired,
                        &mut owner,
                        &mut bridge,
                        &mut reader,
                        &mut tickets,
                    );
                }
            } else {
                teardown_microphone(
                    &mut retired,
                    &mut owner,
                    &mut bridge,
                    &mut reader,
                    &mut tickets,
                );
            }
            retired.poll();
            thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    })();
    state.revoke();
    let close_result = close_worker_gate(&mut control, &mut current);
    teardown_microphone(
        &mut retired,
        &mut owner,
        &mut bridge,
        &mut reader,
        &mut tickets,
    );
    retired.finish();
    result.and(close_result)
}

fn teardown_microphone(
    retired: &mut RetiredPcm,
    owner: &mut Option<PcmOwner>,
    bridge: &mut Option<PcmMicrophoneBridge>,
    reader: &mut Option<AudioReader<f32>>,
    tickets: &mut Vec<PcmTicket>,
) {
    if let Some(pcm) = owner.take() {
        retired.retire(pcm, "microphone");
    }
    tickets.clear();
    if let Some(bridge) = bridge.take() {
        *reader = Some(bridge.into_carrier());
    }
    // A concurrent graph write may still publish old PCM after this teardown.
    // Only quiesce_microphone may discard, after acquiring the producer fence.
}

fn close_worker_gate(
    control: &mut ControlClient,
    current: &mut super::EndpointSnapshot,
) -> Result<(), BackendError> {
    // Closing during shutdown is safe for the latest native stream, but opening
    // is never allowed to refresh its expected snapshot this way.
    *current = control.query(current.key()).map_err(client_error)?;
    if current.gate_open() {
        *current = control.close_gate(*current).map_err(client_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_observation_is_a_detached_public_control_snapshot() {
        let endpoint_id = VirtualEndpointId::from_bytes([0x42; 16]);
        let observation = VadExportObservation::from(ExportView {
            spec: ExportSpec {
                id: endpoint_id,
                intent: Intent::Playback,
                channels: Channels::Stereo,
                name: "Probe playback".into(),
                allow_default: false,
            },
            snapshot: None,
            removing: true,
            error: Some("CREATE timeout".into()),
            transport_prepared: false,
            telemetry: Some(VadPcmTelemetrySnapshot {
                schema_version: 1,
                submission_attempts: 3,
                ..VadPcmTelemetrySnapshot::default()
            }),
        });

        assert_eq!(observation.endpoint_id, endpoint_id);
        assert_eq!(observation.spec_name, "Probe playback");
        assert!(observation.snapshot.is_none());
        assert!(!observation.transport_prepared);
        assert_eq!(observation.telemetry.unwrap().submission_attempts, 3);
        assert_eq!(observation.error.as_deref(), Some("CREATE timeout"));
    }

    #[test]
    fn park_full_eviction_selects_the_oldest_generation_not_the_lowest_slot() {
        let mut retired = RetiredPcm::new();
        // Simulate retire order: slot 0 age 5, slot 1 age 3 — slot 1 parked
        // second but slot 0 is older only if ages say so. Pin the policy:
        // eviction picks the smallest age, and `poll` recycling a slot never
        // turns a younger generation into the victim by slot index alone.
        retired.parked[0].1 = 5;
        retired.parked[1].1 = 3;
        assert_eq!(retired.oldest(), 1);
        // A recycled slot keeps a stale age until refilled; the comparison
        // stays age-driven. Ages never compare equal in practice because
        // `next_age` is monotonic, but ties must resolve deterministically.
        retired.parked[1].1 = 5;
        assert_eq!(retired.oldest(), 0);
        retired.parked[0].1 = u64::MAX;
        retired.parked[1].1 = 0;
        assert_eq!(retired.oldest(), 1);
    }
}
