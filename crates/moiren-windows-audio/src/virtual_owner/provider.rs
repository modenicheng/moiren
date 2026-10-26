use super::boundary::{BoundaryProbe, VirtualMicrophoneSink, VirtualPlaybackSource};
use moiren_engine::{
    boundary::{AudioReader, AudioWriter},
    host::{BackendError, CancellationToken, OwnerStatus, ResourceEvent},
    node::VirtualEndpointId,
};
use std::sync::{Arc, atomic::AtomicU64};

/// Persistent desired resource identity is resolved here, never on render.
#[derive(Debug, Clone, Copy)]
pub struct VirtualBoundaryRequest {
    pub endpoint_id: VirtualEndpointId,
    pub channels: usize,
    /// Engine rate for SRC at the user/kernel bridge; driver PCM stays at 48 kHz.
    pub engine_sample_rate: u32,
    pub max_block_frames: usize,
    /// Complete consumption burst across all graph segments of one owner wake.
    pub max_demand_frames: u32,
    pub buffer: super::VirtualBufferOptions,
    pub audio_byte_budget: usize,
    pub owner_generation: u64,
}

/// A prepared transport worker's control-side lifetime, independent of clients.
///
/// `Running` means the transport is alive, including when zero external clients
/// are connected. These methods are never called by graph processing. Activate
/// must be bounded/cancellable; any IOCTL belongs to the transport worker.
pub trait VirtualResourceSession: Send {
    fn activate(&self) -> Result<(), BackendError>;
    fn cancel(&self);
    fn status(&self) -> OwnerStatus;
    fn stop_and_join(self: Box<Self>) -> Result<(), BackendError>;
}

pub struct PreparedVirtualPlayback {
    pub carrier: AudioReader<f32>,
    pub reset_epoch: Arc<AtomicU64>,
    /// Producer acknowledges only after old PCM publication is impossible.
    pub producer_quiesced_epoch: Arc<AtomicU64>,
    /// Consumer acknowledges after discarding its old carrier/FIR contents.
    pub consumer_quiesced_epoch: Arc<AtomicU64>,
    /// Control-side carrier water-level probe for this boundary.
    pub probe: Arc<BoundaryProbe>,
    pub session: Box<dyn VirtualResourceSession>,
}

pub struct PreparedVirtualMicrophone {
    pub carrier: AudioWriter<f32>,
    pub reset_epoch: Arc<AtomicU64>,
    /// Engine producer pauses publication until the PCM consumer has discarded.
    pub producer_quiesced_epoch: Arc<AtomicU64>,
    pub consumer_quiesced_epoch: Arc<AtomicU64>,
    /// Control-side carrier water-level probe for this boundary.
    pub probe: Arc<BoundaryProbe>,
    pub session: Box<dyn VirtualResourceSession>,
}

/// Opt-in transport factory. Returning a session and stereo carrier is an
/// assertion that actual resources were prepared; this library has no silent
/// default implementation. Providers must honor channels and cancellation.
pub trait VirtualBoundaryProvider: Send + Sync + 'static {
    fn prepare_playback(
        &self,
        request: VirtualBoundaryRequest,
        cancel: &CancellationToken,
    ) -> Result<PreparedVirtualPlayback, BackendError>;
    fn prepare_microphone(
        &self,
        request: VirtualBoundaryRequest,
        cancel: &CancellationToken,
    ) -> Result<PreparedVirtualMicrophone, BackendError>;
    fn poll_resource(&self) -> Option<ResourceEvent> {
        None
    }
}

/// Prepare a playback boundary and bind its carrier in one step: the session
/// joins `sessions` under its reset epoch, and the returned source is fully
/// fenced, probed and drift-tracked. Both native backends must route virtual
/// IO through these so a boundary behaves identically wherever it prepares.
pub fn bind_playback_source(
    provider: &dyn VirtualBoundaryProvider,
    request: VirtualBoundaryRequest,
    cancel: &CancellationToken,
    sessions: &mut VirtualSessions,
) -> Result<(VirtualPlaybackSource, Arc<BoundaryProbe>), BackendError> {
    let queue = request
        .buffer
        .prepare(request.engine_sample_rate, request.max_demand_frames)
        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
    let prepared = provider.prepare_playback(request, cancel)?;
    sessions.push_with_reset_epoch(prepared.session, prepared.reset_epoch.clone());
    if queue.is_some_and(|q| prepared.carrier.capacity_frames() != q.capacity_frames as usize) {
        return Err(BackendError::Unsupported(
            "virtual playback provider did not prepare the requested carrier capacity".into(),
        ));
    }
    let source = VirtualPlaybackSource::new(
        prepared.carrier,
        request.channels,
        request.max_block_frames,
        request.engine_sample_rate,
        request.audio_byte_budget,
    )?
    .with_reset_fence(
        prepared.reset_epoch,
        prepared.producer_quiesced_epoch,
        prepared.consumer_quiesced_epoch,
    )
    .with_probe(prepared.probe.clone());
    // Native packet publication and Engine block wakes keep independent
    // phases even at one nominal rate; the boundary carries the drift cost.
    let source = if let Some(queue) = queue {
        source.with_clock_drift_target(queue.servo_reference_frames as usize)?
    } else {
        source.with_clock_drift()?
    };
    Ok((source, prepared.probe))
}

/// Prepare a microphone boundary and bind its carrier in one step, with the
/// same session/fence/probe/drift semantics as [`bind_playback_source`].
pub fn bind_microphone_sink(
    provider: &dyn VirtualBoundaryProvider,
    request: VirtualBoundaryRequest,
    cancel: &CancellationToken,
    sessions: &mut VirtualSessions,
) -> Result<(VirtualMicrophoneSink, Arc<BoundaryProbe>), BackendError> {
    let queue = request
        .buffer
        .prepare(request.engine_sample_rate, request.max_demand_frames)
        .map_err(|error| BackendError::Unsupported(error.to_string()))?;
    let prepared = provider.prepare_microphone(request, cancel)?;
    sessions.push_with_reset_epoch(prepared.session, prepared.reset_epoch.clone());
    if queue.is_some_and(|q| prepared.carrier.capacity_frames() != q.capacity_frames as usize) {
        return Err(BackendError::Unsupported(
            "virtual microphone provider did not prepare the requested carrier capacity".into(),
        ));
    }
    let sink = VirtualMicrophoneSink::new(
        prepared.carrier,
        request.channels,
        request.max_block_frames,
        request.engine_sample_rate,
        request.audio_byte_budget,
    )?
    .with_reset_fence(
        prepared.reset_epoch,
        prepared.producer_quiesced_epoch,
        prepared.consumer_quiesced_epoch,
    )
    .with_probe(prepared.probe.clone());
    let sink = if let Some(queue) = queue {
        sink.with_clock_drift_target(queue.servo_reference_frames as usize)?
    } else {
        sink.with_clock_drift()?
    };
    Ok((sink, prepared.probe))
}

/// Keeps resources on the control side through compiler/attachment failures.
#[derive(Default)]
pub struct VirtualSessions {
    sessions: Vec<Box<dyn VirtualResourceSession>>,
    timeline_resets: Vec<Arc<AtomicU64>>,
}

impl VirtualSessions {
    pub fn push(&mut self, session: Box<dyn VirtualResourceSession>) {
        self.sessions.push(session);
    }
    /// Register the prepared boundary's reset fence for virtual-clock stalls.
    /// Handles are copied before starting the owner; render only bumps atomics.
    pub fn push_with_reset_epoch(
        &mut self,
        session: Box<dyn VirtualResourceSession>,
        reset_epoch: Arc<AtomicU64>,
    ) {
        self.sessions.push(session);
        self.timeline_resets.push(reset_epoch);
    }
    #[cfg(windows)]
    pub(super) fn timeline_resets(&self) -> Vec<Arc<AtomicU64>> {
        self.timeline_resets.clone()
    }
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
    pub fn activate(&self) -> Result<(), BackendError> {
        for session in &self.sessions {
            if let Err(error) = session.activate() {
                self.cancel();
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn cancel(&self) {
        for session in &self.sessions {
            session.cancel();
        }
    }
    pub fn status(&self) -> OwnerStatus {
        let mut starting = false;
        for session in &self.sessions {
            match session.status() {
                OwnerStatus::Failed(error) => return OwnerStatus::Failed(error),
                OwnerStatus::Starting => starting = true,
                OwnerStatus::Running => {}
            }
        }
        if starting {
            OwnerStatus::Starting
        } else {
            OwnerStatus::Running
        }
    }
    pub fn stop_and_join(&mut self) -> Result<(), BackendError> {
        self.cancel();
        let mut failure = None;
        for session in self.sessions.drain(..) {
            if let Err(error) = session.stop_and_join() {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for VirtualSessions {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}
