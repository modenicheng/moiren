//! Explicit, single-session evidence. The PCM worker alone mutates the buffers;
//! the mutex is used only during reservation, worker exit and actual join.
use super::*;
use serde::Serialize;

const MAX_FRAMES: u32 = 480_000;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VadInputCaptureIdentity {
    pub stable_id: [u8; 16],
    /// PDO token, control session, endpoint generation, stream generation,
    /// gate epoch. These belong to the actual owner, never a later QUERY.
    pub native_stamp: [u64; 5],
    pub reset_epoch: u64,
    pub admission_ticket: u64,
    pub active_revision: u64,
    pub format: u32,
    pub channels: u32,
    pub packet_frames: u32,
}

#[derive(Debug, Serialize)]
pub struct VadInputCapturePacket {
    pub sequence: u64,
    pub driver_qpc: u64,
    pub byte_offset: usize,
    /// Complete frames in the returned bytes. A malformed residual tail is
    /// retained in byte_count/raw and marked short_or_wrong_packet, not hidden.
    pub actual_frames: u32,
    pub byte_count: usize,
    pub publication: &'static str,
}

#[derive(Debug, Serialize)]
pub struct VadInputCaptureArtifact {
    pub requested_frames: u32,
    pub stored_frames: u32,
    pub identity: Option<VadInputCaptureIdentity>,
    pub seal_reason: &'static str,
    pub excluded_stale_packets: u64,
    pub excluded_epoch_packets: u64,
    pub worker_join: &'static str,
    pub worker_outcome: &'static str,
    pub packets: Vec<VadInputCapturePacket>,
    #[serde(skip)]
    pub raw: Vec<u8>,
}

struct SharedState {
    artifact: Option<VadInputCaptureArtifact>,
    claimed: bool,
    joined: bool,
    spawn_failed: bool,
    taken: bool,
}

pub(super) struct CaptureShared {
    endpoint: VirtualEndpointId,
    inner: Mutex<SharedState>,
}

/// Detached evidence only: does not retain the provider, native handle or session.
pub struct VadInputCaptureHandle(Arc<CaptureShared>);

impl VadInputCaptureHandle {
    /// Cancellation/drained/worker exit are deliberately insufficient. A panic
    /// result from JoinHandle::join still proves that the worker has terminated.
    pub fn take_after_join(&self) -> Result<VadInputCaptureArtifact, &'static str> {
        let mut state = self.0.inner.lock().unwrap_or_else(|p| p.into_inner());
        if state.taken {
            return Err("already_taken");
        }
        if state.spawn_failed {
            return Err("spawn_failed_no_worker_unknown");
        }
        if !state.joined {
            return Err(if state.claimed {
                "not_joined"
            } else {
                "not_started_unknown"
            });
        }
        state.taken = true;
        state.artifact.take().ok_or("artifact_unavailable_unknown")
    }
}

impl CaptureShared {
    pub(super) fn new(
        endpoint: VirtualEndpointId,
        frames: u32,
    ) -> Result<(Arc<Self>, VadInputCaptureHandle), BackendError> {
        if !(1..=MAX_FRAMES).contains(&frames) {
            return Err(BackendError::Unsupported(
                "input capture frames must be 1..=480000".into(),
            ));
        }
        let mut raw = Vec::new();
        raw.try_reserve_exact((frames as usize * 8).min(MAX_BYTES))
            .map_err(|_| BackendError::Failed("input capture allocation refused".into()))?;
        let mut packets = Vec::new();
        packets
            .try_reserve_exact(MAX_RECORDS)
            .map_err(|_| BackendError::Failed("input capture records allocation refused".into()))?;
        let shared = Arc::new(Self {
            endpoint,
            inner: Mutex::new(SharedState {
                artifact: Some(VadInputCaptureArtifact {
                    requested_frames: frames,
                    stored_frames: 0,
                    identity: None,
                    seal_reason: "open",
                    excluded_stale_packets: 0,
                    excluded_epoch_packets: 0,
                    worker_join: "unknown",
                    worker_outcome: "unknown",
                    packets,
                    raw,
                }),
                claimed: false,
                joined: false,
                spawn_failed: false,
                taken: false,
            }),
        });
        Ok((shared.clone(), VadInputCaptureHandle(shared)))
    }

    pub(super) fn reserve(
        self: &Arc<Self>,
        endpoint: VirtualEndpointId,
    ) -> Result<Option<CaptureWorker>, BackendError> {
        if endpoint != self.endpoint {
            return Ok(None);
        }
        let mut state = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if state.claimed {
            // The diagnostic is single-shot, not an audio admission policy.
            // Subsequent sessions run normally without replacing the artifact.
            return Ok(None);
        }
        state.claimed = true;
        Ok(Some(CaptureWorker {
            shared: self.clone(),
            artifact: state.artifact.take().expect("unclaimed capture"),
            owner: None,
        }))
    }

    pub(super) fn joined(&self, panicked: bool) {
        let mut state = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(artifact) = state.artifact.as_mut() {
            artifact.worker_join = if panicked {
                "joined_panicked"
            } else {
                "joined"
            };
        }
        state.joined = true;
    }

    pub(super) fn spawn_failed(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn_failed = true;
    }
}

pub(super) struct CaptureWorker {
    shared: Arc<CaptureShared>,
    artifact: VadInputCaptureArtifact,
    owner: Option<VadInputCaptureIdentity>,
}

impl CaptureWorker {
    pub(super) fn shared(&self) -> Arc<CaptureShared> {
        self.shared.clone()
    }
    pub(super) fn observe(
        &mut self,
        snapshot: super::super::EndpointSnapshot,
        state: &SessionState,
    ) {
        if self.artifact.identity.is_some_and(|identity| {
            identity.native_stamp != lifecycle::stamp(snapshot)
                || identity.reset_epoch != state.reset_epoch.load(Ordering::Acquire)
                || !state
                    .admission
                    .matches(identity.admission_ticket, identity.active_revision)
                || snapshot.sample_format().map(|f| f as u32) != Some(identity.format)
                || snapshot.packet_frames() != identity.packet_frames
                || snapshot.channels() as u32 != identity.channels
        }) {
            self.seal("identity_changed");
        }
    }
    pub(super) fn bind(&mut self, identity: VadInputCaptureIdentity) {
        self.owner = Some(identity);
    }
    pub(super) fn seal(&mut self, reason: &'static str) {
        if self.artifact.identity.is_some() && self.artifact.seal_reason == "open" {
            self.artifact.seal_reason = reason;
        }
    }
    pub(super) fn before_decode(
        &mut self,
        state: &SessionState,
        epoch: u64,
        stamp: super::super::PcmPacketStamp,
        pcm: &[u8],
    ) -> Option<usize> {
        let identity = self.owner?;
        if self.artifact.seal_reason != "open" {
            return None;
        }
        // The identity belongs to the actual PcmOwner, not a later QUERY or
        // admission. Never relabel borrowed old-owner bytes with fresh atoms.
        if identity.reset_epoch != epoch
            || !state.admitted()
            || state.reset_epoch.load(Ordering::Acquire) != epoch
            || state.admitted_ticket.load(Ordering::Acquire) != identity.admission_ticket
            || state.admitted_revision.load(Ordering::Acquire) != identity.active_revision
            || !state
                .admission
                .matches(identity.admission_ticket, identity.active_revision)
        {
            self.artifact.excluded_epoch_packets =
                self.artifact.excluded_epoch_packets.saturating_add(1);
            self.seal("identity_changed");
            return None;
        }
        if self
            .artifact
            .identity
            .is_some_and(|first| first != identity)
        {
            self.seal("identity_changed");
            return None;
        }
        let sample_bytes = match identity.format {
            1 => 2,
            2 => 4,
            _ => {
                self.artifact.seal_reason = "invalid_shape";
                return None;
            }
        };
        let frame_bytes = identity.channels as usize * sample_bytes;
        if frame_bytes == 0 {
            self.artifact.seal_reason = "invalid_packet_length";
            return None;
        }
        let frames = (pcm.len() / frame_bytes) as u32;
        if self.artifact.stored_frames.saturating_add(frames) > self.artifact.requested_frames
            || self.artifact.raw.len() + pcm.len() > self.artifact.raw.capacity()
            || self.artifact.raw.len() + pcm.len() > MAX_BYTES
            || self.artifact.packets.len() >= MAX_RECORDS
        {
            self.artifact.seal_reason = "capacity_whole_packet";
            return None;
        }
        self.artifact.identity = Some(identity);
        let index = self.artifact.packets.len();
        self.artifact.packets.push(VadInputCapturePacket {
            sequence: stamp.sequence,
            driver_qpc: stamp.qpc,
            byte_offset: self.artifact.raw.len(),
            actual_frames: frames,
            byte_count: pcm.len(),
            publication: "pending",
        });
        self.artifact.raw.extend_from_slice(pcm);
        self.artifact.stored_frames += frames;
        if !pcm.len().is_multiple_of(frame_bytes) || frames != identity.packet_frames {
            self.artifact.seal_reason = "short_or_wrong_packet";
        } else if self.artifact.stored_frames == self.artifact.requested_frames {
            self.artifact.seal_reason = "frame_capacity";
        }
        Some(index)
    }
    pub(super) fn stale(&mut self) {
        self.artifact.excluded_stale_packets =
            self.artifact.excluded_stale_packets.saturating_add(1);
    }
    pub(super) fn epoch_excluded(&mut self) {
        self.artifact.excluded_epoch_packets =
            self.artifact.excluded_epoch_packets.saturating_add(1);
        self.seal("identity_changed");
    }
    pub(super) fn finished(&mut self, result: &Result<(), BackendError>) {
        self.artifact.worker_outcome = match result {
            Ok(()) => "completed",
            Err(BackendError::WorkerPanicked) => "panicked",
            Err(_) => "failed",
        };
        self.seal(if result.is_err() {
            "worker_error"
        } else {
            "worker_exit"
        });
        if self.artifact.identity.is_none() && self.artifact.seal_reason == "open" {
            self.artifact.seal_reason = "no_accepted_data_unknown";
        }
    }
    pub(super) fn published(&mut self, index: Option<usize>, result: &'static str) {
        if let Some(index) = index {
            self.artifact.packets[index].publication = result;
        }
    }
}

impl Drop for CaptureWorker {
    fn drop(&mut self) {
        // Move, never copy, the preallocated buffers on worker exit. Actual
        // join remains a separate control-side event.
        self.seal("worker_exit");
        if self.artifact.seal_reason == "open" {
            self.artifact.seal_reason = "no_accepted_data_unknown";
        }
        let mut state = self.shared.inner.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::swap(
            state
                .artifact
                .get_or_insert_with(|| VadInputCaptureArtifact {
                    requested_frames: 0,
                    stored_frames: 0,
                    identity: None,
                    seal_reason: "unknown",
                    excluded_stale_packets: 0,
                    excluded_epoch_packets: 0,
                    worker_join: "unknown",
                    worker_outcome: "unknown",
                    packets: Vec::new(),
                    raw: Vec::new(),
                }),
            &mut self.artifact,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_capture_preallocated_hook_never_grows_allocates_or_frees_at_record_bound() {
        let id = VirtualEndpointId::from_bytes([2; 16]);
        let (shared, _) = CaptureShared::new(id, 480_000).unwrap();
        let mut worker = shared.reserve(id).unwrap().unwrap();
        let state = SessionState::new(Arc::new(Admission::new()));
        let ticket = state.admission.set(Some(1));
        state.active.store(true, Ordering::Release);
        state.admitted_ticket.store(ticket, Ordering::Release);
        state.admitted_revision.store(1, Ordering::Release);
        worker.bind(VadInputCaptureIdentity {
            stable_id: id.to_bytes(),
            native_stamp: [1; 5],
            reset_epoch: 0,
            admission_ticket: ticket,
            active_revision: 1,
            format: 2,
            channels: 2,
            packet_frames: 1,
        });
        let raw_pointer = worker.artifact.raw.as_ptr();
        let raw_capacity = worker.artifact.raw.capacity();
        let packet_pointer = worker.artifact.packets.as_ptr();
        let packet_capacity = worker.artifact.packets.capacity();
        crate::test_allocation::assert_allocates_nothing(|| {
            for sequence in 1..=4097 {
                let index = worker.before_decode(
                    &state,
                    0,
                    super::super::super::PcmPacketStamp {
                        sequence,
                        qpc: sequence,
                    },
                    &[0; 8],
                );
                worker.published(index, "published");
            }
        });
        assert_eq!(worker.artifact.packets.len(), 4096);
        assert_eq!(worker.artifact.seal_reason, "capacity_whole_packet");
        assert_eq!(worker.artifact.raw.as_ptr(), raw_pointer);
        assert_eq!(worker.artifact.raw.capacity(), raw_capacity);
        assert_eq!(worker.artifact.packets.as_ptr(), packet_pointer);
        assert_eq!(worker.artifact.packets.capacity(), packet_capacity);
        assert!(worker.artifact.raw.len() <= MAX_BYTES);
    }
}
