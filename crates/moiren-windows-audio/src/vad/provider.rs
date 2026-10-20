//! Windows VAD transport adapter.
//!
//! The adapter owns the control session and keeps endpoint lifetimes separate
//! from native client lifetimes. A raw PDO may be Present while no client has
//! opened a stream; that is an idle, usable state, not a transport failure.

use super::{
    Channels, ClientError, ControlClient, DeviceInterface, Intent, PcmCompletion,
    PcmMicrophoneBridge, PcmOwner, PcmPlaybackBridge, PcmTicket, PdoState, StableEndpointId,
    StreamState, discover_present_interfaces,
};
use crate::virtual_owner::{
    PreparedVirtualMicrophone, PreparedVirtualPlayback, VirtualBoundaryProvider,
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
    time::Duration,
};

const REMOVE_TIMEOUT: Duration = Duration::from_secs(15);
const PCM_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

mod lifecycle;
mod pcm_worker;
mod transport;
use lifecycle::{Admission, ExportSpec, ExportView, Lifecycle};

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
    pub fn export_observations(&self) -> Vec<ExportView> {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).poll()
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
                    self.set_export_gate(view.spec.id, lifecycle::stamp(snapshot), false, 0, 0)?;
                } else if snapshot.state() != PdoState::Removing {
                    self.retire_export(view.spec.id, lifecycle::stamp(snapshot))?;
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
            },
            state,
        )?;
        Ok(PreparedVirtualPlayback {
            carrier: reader,
            reset_epoch: session.reset_epoch(),
            producer_quiesced_epoch: session.state.producer_quiesced_epoch.clone(),
            consumer_quiesced_epoch: session.state.consumer_quiesced_epoch.clone(),
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
            },
            state,
        )?;
        Ok(PreparedVirtualMicrophone {
            carrier: writer,
            reset_epoch: session.reset_epoch(),
            producer_quiesced_epoch: session.state.producer_quiesced_epoch.clone(),
            consumer_quiesced_epoch: session.state.consumer_quiesced_epoch.clone(),
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
    Ok((native_frames + 32).max(super::MAX_PACKET_FRAMES as usize))
}

enum WorkerConfig {
    Playback {
        control: ControlClient,
        endpoint: TransportEndpoint,
        channels: Channels,
        writer: AudioWriter<f32>,
        byte_budget: usize,
    },
    Microphone {
        control: ControlClient,
        endpoint: TransportEndpoint,
        channels: Channels,
        reader: AudioReader<f32>,
        byte_budget: usize,
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match config {
        WorkerConfig::Playback {
            control,
            endpoint,
            channels,
            writer,
            byte_budget,
        } => run_playback(
            control,
            endpoint,
            channels,
            Some(writer),
            byte_budget,
            &state,
        ),
        WorkerConfig::Microphone {
            control,
            endpoint,
            channels,
            reader,
            byte_budget,
        } => run_microphone(
            control,
            endpoint,
            channels,
            Some(reader),
            byte_budget,
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
    state: &SessionState,
) -> Result<(), BackendError> {
    let mut current = endpoint.snapshot;
    let mut owner: Option<PcmOwner> = None;
    let mut bridge = None;
    let mut tickets = Vec::with_capacity(super::REQUEST_SLOTS);
    let mut order = pcm_worker::PacketOrder::default();
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
                    teardown_playback(&mut owner, &mut bridge, &mut writer, &mut tickets)
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
                            teardown_playback(&mut owner, &mut bridge, &mut writer, &mut tickets);
                        }
                    }
                } else {
                    teardown_playback(&mut owner, &mut bridge, &mut writer, &mut tickets);
                }
            } else {
                teardown_playback(&mut owner, &mut bridge, &mut writer, &mut tickets);
            }
            thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    })();
    state.revoke();
    let close_result = close_worker_gate(&mut control, &mut current);
    teardown_playback(&mut owner, &mut bridge, &mut writer, &mut tickets);
    result.and(close_result)
}

fn teardown_playback(
    owner: &mut Option<PcmOwner>,
    bridge: &mut Option<PcmPlaybackBridge>,
    writer: &mut Option<AudioWriter<f32>>,
    tickets: &mut Vec<PcmTicket>,
) {
    if let Some(mut pcm) = owner.take() {
        let _ = pcm.shutdown(PCM_SHUTDOWN_TIMEOUT);
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
    state: &SessionState,
) -> Result<(), BackendError> {
    let mut current = endpoint.snapshot;
    let mut owner: Option<PcmOwner> = None;
    let mut bridge = None;
    let mut tickets = Vec::with_capacity(super::REQUEST_SLOTS);
    let mut order = pcm_worker::PacketOrder::default();
    let mut observed_reset = 0;
    let result = (|| {
        while !state.cancel.load(std::sync::atomic::Ordering::Acquire) {
            if state.active.load(std::sync::atomic::Ordering::Acquire) {
                state.status.store(1, std::sync::atomic::Ordering::Release);
                let (queried, _) = transport::poll(&mut control, current, state)?;
                current = queried;
                if state.reset_epoch.load(Ordering::Acquire) != observed_reset {
                    teardown_microphone(&mut owner, &mut bridge, &mut reader, &mut tickets);
                    if !pcm_worker::quiesce_microphone(
                        state,
                        &mut observed_reset,
                        reader.as_mut().expect("microphone carrier is free"),
                    ) {
                        thread::sleep(POLL_INTERVAL);
                        continue;
                    }
                }
                if current.stream_state() == StreamState::Running
                    && current.gate_open()
                    && state.admitted()
                {
                    if owner.is_none() {
                        let shape = current
                            .pcm_shape(control.capabilities())
                            .map_err(|e| client_error(e.into()))?;
                        bridge = Some(PcmMicrophoneBridge::new(
                            channels,
                            shape.format(),
                            shape.frames() as usize,
                            byte_budget,
                            reader.take().expect("microphone carrier is free"),
                        )?);
                        owner = Some(control.pcm_owner(current).map_err(client_error)?);
                        tickets.clear();
                        order = pcm_worker::PacketOrder::default();
                    }
                    match pcm_worker::microphone(
                        owner.as_mut().expect("microphone owner"),
                        bridge.as_mut().expect("microphone bridge"),
                        &mut tickets,
                        &mut order,
                        state,
                        observed_reset,
                    ) {
                        Ok(pcm_worker::PumpState::Current) => {
                            if let Err(error) = pcm_worker::wait_for_activity(
                                owner.as_mut().expect("microphone owner"),
                                &tickets,
                                state,
                                observed_reset,
                            ) {
                                current =
                                    transport::after_io_error(&mut control, current, state, error)?;
                                teardown_microphone(
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
                            teardown_microphone(&mut owner, &mut bridge, &mut reader, &mut tickets);
                        }
                    }
                } else {
                    teardown_microphone(&mut owner, &mut bridge, &mut reader, &mut tickets);
                }
            } else {
                teardown_microphone(&mut owner, &mut bridge, &mut reader, &mut tickets);
            }
            thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    })();
    state.revoke();
    let close_result = close_worker_gate(&mut control, &mut current);
    teardown_microphone(&mut owner, &mut bridge, &mut reader, &mut tickets);
    result.and(close_result)
}

fn teardown_microphone(
    owner: &mut Option<PcmOwner>,
    bridge: &mut Option<PcmMicrophoneBridge>,
    reader: &mut Option<AudioReader<f32>>,
    tickets: &mut Vec<PcmTicket>,
) {
    if let Some(mut pcm) = owner.take() {
        let _ = pcm.shutdown(PCM_SHUTDOWN_TIMEOUT);
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
