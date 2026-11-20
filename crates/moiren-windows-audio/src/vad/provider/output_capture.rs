//! Opt-in encoded MIC attempts, not native sample time or recorder delivery.
use super::*;
use serde::Serialize;

const MAX_FRAMES: u32 = 480_000;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VadOutputCaptureIdentity {
    pub stable_id: [u8; 16],
    /// PDO token, control session, endpoint generation, stream generation, gate epoch.
    pub native_stamp: [u64; 5],
    pub reset_epoch: u64,
    pub admission_ticket: u64,
    pub active_revision: u64,
    pub format: u32,
    pub channels: u32,
    pub packet_frames: u32,
}

#[derive(Debug, Serialize)]
pub struct VadOutputCapturePacket {
    pub byte_offset: usize,
    pub byte_count: usize,
    pub actual_frames: u32,
    pub silence_frames: usize,
    pub submission: &'static str,
    pub identity_current_after_submit: Option<bool>,
    pub completion: &'static str,
    pub completion_bytes: Option<usize>,
    pub sequence: Option<u64>,
    /// Echoed host WRITE-header QPC, never a driver callback/sample timestamp.
    pub host_submission_qpc: Option<u64>,
    pub error_kind: Option<&'static str>,
    pub raw_hresult: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct VadOutputCaptureArtifact {
    pub requested_frames: u32,
    pub stored_frames: u32,
    pub identity: Option<VadOutputCaptureIdentity>,
    pub seal_reason: &'static str,
    /// Terminal completion observations without a recorded ticket, including
    /// normal unrecorded attempts after the bounded prefix. Not stale/loss proof.
    pub untracked_completion_observations: u64,
    pub excluded_epoch_packets: u64,
    pub worker_join: &'static str,
    pub worker_outcome: &'static str,
    pub packets: Vec<VadOutputCapturePacket>,
    #[serde(skip)]
    pub raw: Vec<u8>,
}

struct SharedState {
    artifact: Option<VadOutputCaptureArtifact>,
    claimed: bool,
    joined: bool,
    spawn_failed: bool,
    taken: bool,
}

pub(super) struct CaptureShared {
    endpoint: VirtualEndpointId,
    inner: Mutex<SharedState>,
}

/// Detached one-shot evidence: no provider, native handle or session ownership.
pub struct VadOutputCaptureHandle(Arc<CaptureShared>);

impl VadOutputCaptureHandle {
    pub fn take_after_join(&self) -> Result<VadOutputCaptureArtifact, &'static str> {
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
    ) -> Result<(Arc<Self>, VadOutputCaptureHandle), BackendError> {
        if !(1..=MAX_FRAMES).contains(&frames) {
            return Err(BackendError::Unsupported(
                "output capture frames must be 1..=480000".into(),
            ));
        }
        let mut raw = Vec::new();
        raw.try_reserve_exact((frames as usize * 8).min(MAX_BYTES))
            .map_err(|_| BackendError::Failed("output capture allocation refused".into()))?;
        let mut packets = Vec::new();
        packets.try_reserve_exact(MAX_RECORDS).map_err(|_| {
            BackendError::Failed("output capture records allocation refused".into())
        })?;
        let shared = Arc::new(Self {
            endpoint,
            inner: Mutex::new(SharedState {
                artifact: Some(VadOutputCaptureArtifact {
                    requested_frames: frames,
                    stored_frames: 0,
                    identity: None,
                    seal_reason: "open",
                    untracked_completion_observations: 0,
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
        Ok((shared.clone(), VadOutputCaptureHandle(shared)))
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
            return Ok(None);
        }
        state.claimed = true;
        Ok(Some(CaptureWorker {
            shared: self.clone(),
            artifact: state.artifact.take().expect("unclaimed capture"),
            owner: None,
            pending: [None; super::super::REQUEST_SLOTS],
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
    artifact: VadOutputCaptureArtifact,
    owner: Option<VadOutputCaptureIdentity>,
    pending: [Option<(PcmTicket, usize)>; super::super::REQUEST_SLOTS],
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
            identity.stable_id != snapshot.stable_id().to_le_bytes()
                || identity.native_stamp != lifecycle::stamp(snapshot)
                || !self.publication_current(state, identity.reset_epoch)
                || snapshot.sample_format().map(|format| format as u32) != Some(identity.format)
                || snapshot.packet_frames() != identity.packet_frames
                || snapshot.channels() as u32 != identity.channels
        }) {
            self.seal("identity_changed");
        }
    }

    pub(super) fn bind(&mut self, identity: VadOutputCaptureIdentity) {
        // Each call denotes a different actual PcmOwner, even with equal scalars.
        // Pending old-owner tickets can still be resolved without merging PCM.
        self.seal("owner_reopened");
        self.owner = Some(identity);
    }

    pub(super) fn publication_current(&self, state: &SessionState, epoch: u64) -> bool {
        self.owner.is_some_and(|identity| {
            identity.reset_epoch == epoch
                && state.admitted()
                && state.reset_epoch.load(Ordering::Acquire) == epoch
                && state.admitted_ticket.load(Ordering::Acquire) == identity.admission_ticket
                && state.admitted_revision.load(Ordering::Acquire) == identity.active_revision
                && state
                    .admission
                    .matches(identity.admission_ticket, identity.active_revision)
        })
    }

    pub(super) fn seal(&mut self, reason: &'static str) {
        if self.artifact.identity.is_some() && self.artifact.seal_reason == "open" {
            self.artifact.seal_reason = reason;
        }
    }

    pub(super) fn encoded(
        &mut self,
        state: &SessionState,
        epoch: u64,
        packet: &[u8],
        silence_frames: usize,
    ) -> Option<usize> {
        let identity = self.owner?;
        if self.artifact.seal_reason != "open" {
            return None;
        }
        if !self.publication_current(state, epoch) {
            self.artifact.excluded_epoch_packets =
                self.artifact.excluded_epoch_packets.saturating_add(1);
            self.seal("identity_changed");
            return None;
        }
        if identity.stable_id != self.shared.endpoint.to_bytes()
            || self
                .artifact
                .identity
                .is_some_and(|first| first != identity)
        {
            self.artifact.seal_reason = "identity_changed";
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
        if !(1..=2).contains(&identity.channels)
            || identity.packet_frames == 0
            || identity.packet_frames > super::super::MAX_PACKET_FRAMES
        {
            self.artifact.seal_reason = "invalid_shape";
            return None;
        }
        let frame_bytes = identity.channels as usize * sample_bytes;
        let frames = u32::try_from(packet.len() / frame_bytes).ok()?;
        let byte_count = self.artifact.raw.len().checked_add(packet.len())?;
        let stored_frames = self.artifact.stored_frames.checked_add(frames)?;
        if stored_frames > self.artifact.requested_frames
            || byte_count > MAX_BYTES
            || byte_count > self.artifact.raw.capacity()
            || self.artifact.packets.len() >= MAX_RECORDS
        {
            self.artifact.seal_reason = "capacity_whole_packet";
            return None;
        }
        self.artifact.identity = Some(identity);
        let index = self.artifact.packets.len();
        self.artifact.packets.push(VadOutputCapturePacket {
            byte_offset: self.artifact.raw.len(),
            byte_count: packet.len(),
            actual_frames: frames,
            silence_frames,
            submission: "unobserved",
            identity_current_after_submit: None,
            completion: "not_submitted",
            completion_bytes: None,
            sequence: None,
            host_submission_qpc: None,
            error_kind: None,
            raw_hresult: None,
        });
        self.artifact.raw.extend_from_slice(packet);
        self.artifact.stored_frames = stored_frames;
        if !packet.len().is_multiple_of(frame_bytes)
            || frames != identity.packet_frames
            || silence_frames > frames as usize
        {
            self.artifact.seal_reason = "short_or_wrong_packet";
        } else if stored_frames == self.artifact.requested_frames {
            self.artifact.seal_reason = "frame_capacity";
        }
        Some(index)
    }

    pub(super) fn submitted(
        &mut self,
        index: Option<usize>,
        result: &Result<PcmTicket, ClientError>,
        identity_current: bool,
    ) {
        let Some(index) = index else {
            return;
        };
        let Some(record) = self.artifact.packets.get_mut(index) else {
            return;
        };
        if record.submission != "unobserved" {
            return;
        }
        record.identity_current_after_submit = Some(identity_current);
        match result {
            Ok(ticket) => {
                record.submission = "accepted";
                record.completion = "pending";
                // Four actual IO slots bound this side table. Do not use slot
                // indices: opaque equality includes serial and owner identity.
                if self
                    .pending
                    .iter()
                    .flatten()
                    .any(|(pending, _)| pending == ticket)
                {
                    record.completion = "tracking_unavailable";
                    self.seal("ticket_tracking_unavailable");
                } else if let Some(slot) = self.pending.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some((*ticket, index));
                } else {
                    record.completion = "tracking_unavailable";
                    self.seal("ticket_tracking_unavailable");
                }
            }
            Err(error) => {
                record.submission = if matches!(error, ClientError::KernelPcmQuotaFull) {
                    "quota_rejected"
                } else {
                    "error"
                };
                record.completion = "not_submitted";
                (record.error_kind, record.raw_hresult) = error_fields(error);
            }
        }
        if !identity_current {
            self.seal("identity_changed_during_submission");
        }
    }

    pub(super) fn fenced(&mut self, index: Option<usize>) {
        let Some(record) = index.and_then(|index| self.artifact.packets.get_mut(index)) else {
            return;
        };
        if record.submission != "unobserved" {
            return;
        }
        record.submission = "fenced";
        record.completion = "not_submitted";
        self.artifact.excluded_epoch_packets =
            self.artifact.excluded_epoch_packets.saturating_add(1);
        self.seal("identity_changed_before_submission");
    }

    pub(super) fn completed(
        &mut self,
        ticket: PcmTicket,
        result: &Result<Option<PcmCompletion<'_>>, ClientError>,
    ) {
        if matches!(result, Ok(None)) {
            return;
        }
        let Some(slot) = self
            .pending
            .iter()
            .position(|slot| slot.is_some_and(|(pending, _)| pending == ticket))
        else {
            self.artifact.untracked_completion_observations = self
                .artifact
                .untracked_completion_observations
                .saturating_add(1);
            return;
        };
        let (_, index) = self.pending[slot].expect("matched pending ticket");
        let record = &mut self.artifact.packets[index];
        match result {
            Ok(Some(PcmCompletion::Capture { stamp, bytes })) => {
                record.sequence = Some(stamp.sequence);
                record.host_submission_qpc = Some(stamp.qpc);
                record.completion_bytes = Some(*bytes);
                record.completion = if *bytes == record.byte_count {
                    "completed"
                } else {
                    "wrong_shape"
                };
            }
            Ok(Some(PcmCompletion::CaptureDropped { stamp })) => {
                record.sequence = Some(stamp.sequence);
                record.host_submission_qpc = Some(stamp.qpc);
                record.completion = "dropped";
            }
            Ok(Some(PcmCompletion::Render { .. })) => record.completion = "wrong_shape",
            Err(error) => {
                record.completion = if matches!(error, ClientError::KernelPcmQuotaFull) {
                    "quota_rejected"
                } else {
                    "error"
                };
                (record.error_kind, record.raw_hresult) = error_fields(error);
            }
            Ok(None) => unreachable!("pending completion handled above"),
        }
        // Timeout/uncertain completion can leave kernel ownership outstanding;
        // retain its ticket so retirement cannot turn an observed error green.
        if !matches!(
            result,
            Err(ClientError::DeadlineExceeded | ClientError::SessionUncertain)
        ) {
            self.pending[slot] = None;
        }
    }

    pub(super) fn retired(&mut self) {
        // PcmOwner retirement collects internally. Join proves memory safety,
        // not which unresolved requests succeeded, expired or were cancelled.
        for slot in &mut self.pending {
            if let Some((_, index)) = slot.take() {
                let record = &mut self.artifact.packets[index];
                if record.completion == "pending" {
                    record.completion = "retired_completion_unobserved";
                }
            }
        }
        self.seal("owner_retired");
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
        self.retired();
        if self.artifact.identity.is_none() && self.artifact.seal_reason == "open" {
            self.artifact.seal_reason = "no_encoded_data_unknown";
        }
    }
}

fn error_fields(error: &ClientError) -> (Option<&'static str>, Option<i32>) {
    let kind = match error {
        ClientError::Windows { source, .. } => return (Some("windows"), Some(source.code().0)),
        ClientError::Io { .. } => "io",
        ClientError::Protocol(_) => "protocol",
        ClientError::DeadlineExceeded => "deadline_exceeded",
        ClientError::Cancelled => "cancelled",
        ClientError::InvalidDeadline => "invalid_deadline",
        ClientError::SessionUncertain => "session_uncertain",
        ClientError::CreateDeferred => "create_deferred",
        ClientError::PreparationFailed { .. } => "preparation_failed",
        ClientError::PcmOwnerExists => "pcm_owner_exists",
        ClientError::NoFreePcmSlot => "no_free_pcm_slot",
        ClientError::InvalidPcmTicket => "invalid_pcm_ticket",
        ClientError::SequenceExhausted => "sequence_exhausted",
        ClientError::KernelPcmQuotaFull => "kernel_pcm_quota_full",
    };
    (Some(kind), None)
}

impl Drop for CaptureWorker {
    fn drop(&mut self) {
        self.seal("worker_exit");
        self.retired();
        if self.artifact.seal_reason == "open" {
            self.artifact.seal_reason = "no_encoded_data_unknown";
        }
        let mut state = self.shared.inner.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::swap(
            state
                .artifact
                .get_or_insert_with(|| VadOutputCaptureArtifact {
                    requested_frames: 0,
                    stored_frames: 0,
                    identity: None,
                    seal_reason: "unknown",
                    untracked_completion_observations: 0,
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

    fn identity(state: &SessionState, format: u32, packet_frames: u32) -> VadOutputCaptureIdentity {
        let ticket = state.admission.set(Some(9));
        state.active.store(true, Ordering::Release);
        state.admitted_ticket.store(ticket, Ordering::Release);
        state.admitted_revision.store(9, Ordering::Release);
        VadOutputCaptureIdentity {
            stable_id: [2; 16],
            native_stamp: [11, 7, 11, 3, 5],
            reset_epoch: 0,
            admission_ticket: ticket,
            active_revision: 9,
            format,
            channels: 2,
            packet_frames,
        }
    }

    struct Fixture {
        state: SessionState,
        shared: Arc<CaptureShared>,
        handle: VadOutputCaptureHandle,
        worker: Option<CaptureWorker>,
    }

    impl Fixture {
        fn new(frames: u32) -> Self {
            let endpoint = VirtualEndpointId::from_bytes([2; 16]);
            let state = SessionState::new(Arc::new(Admission::new()));
            let identity = identity(&state, 2, 1);
            let (shared, handle) = CaptureShared::new(endpoint, frames).unwrap();
            let mut worker = shared.reserve(endpoint).unwrap().unwrap();
            worker.bind(identity);
            Self {
                state,
                shared,
                handle,
                worker: Some(worker),
            }
        }

        fn encoded(&mut self) -> Option<usize> {
            self.worker
                .as_mut()
                .unwrap()
                .encoded(&self.state, 0, &[0; 8], 1)
        }

        fn take(mut self) -> VadOutputCaptureArtifact {
            drop(self.worker.take());
            self.shared.joined(false);
            self.handle.take_after_join().unwrap()
        }
    }

    fn stamp() -> super::super::super::PcmPacketStamp {
        super::super::super::PcmPacketStamp {
            sequence: 17,
            qpc: 123_456,
        }
    }

    fn snapshot() -> super::super::super::EndpointSnapshot {
        use super::super::super::{Capabilities, EndpointSnapshot};
        let mut caps = [0; 64];
        for (offset, value) in [
            (0, 64_u32),
            (4, 2),
            (8, 31),
            (12, 64),
            (16, 1),
            (20, 48_000),
            (24, 3),
            (28, 6),
            (32, 6),
            (36, 4),
            (48, 4800),
        ] {
            caps[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        caps[40..48].copy_from_slice(&7_u64.to_le_bytes());
        let caps = Capabilities::decode(&caps).unwrap();
        let mut bytes = [0; 128];
        for (offset, value) in [
            (0, 128_u32),
            (4, 2),
            (16, 1),
            (20, 2),
            (24, 2),
            (28, 1),
            (32, 1),
            (36, 1),
            (88, 2),
            (92, 480),
            (100, 2),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [(8, 11_u64), (40, 7), (48, 11), (56, 3), (64, 5)] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[72..88].fill(2);
        EndpointSnapshot::decode(&bytes, caps).unwrap()
    }

    #[test]
    fn output_capture_retains_exact_encoded_pcm16_and_f32_bits_before_write() {
        for (format, raw) in [
            (1, vec![0, 128, 255, 127]),
            (2, vec![0, 0, 0, 128, 0x34, 0x12, 0xc0, 0x7f]),
        ] {
            let endpoint = VirtualEndpointId::from_bytes([2; 16]);
            let state = SessionState::new(Arc::new(Admission::new()));
            let identity = identity(&state, format, 1);
            let (shared, handle) = CaptureShared::new(endpoint, 4).unwrap();
            let mut worker = shared.reserve(endpoint).unwrap().unwrap();
            worker.bind(identity);
            assert_eq!(worker.encoded(&state, 0, &raw, 0), Some(0));
            drop(worker);
            shared.joined(false);
            let artifact = handle.take_after_join().unwrap();
            assert_eq!(artifact.raw, raw);
            assert_eq!(artifact.identity, Some(identity));
            assert_eq!(artifact.stored_frames, 1);
            assert_eq!(artifact.packets[0].submission, "unobserved");
            assert_eq!(artifact.packets[0].host_submission_qpc, None);
        }
    }

    #[test]
    fn output_capture_submission_errors_and_fence_never_become_delivery() {
        for (error, submission, kind) in [
            (
                ClientError::KernelPcmQuotaFull,
                "quota_rejected",
                "kernel_pcm_quota_full",
            ),
            (ClientError::Cancelled, "error", "cancelled"),
            (ClientError::InvalidPcmTicket, "error", "invalid_pcm_ticket"),
        ] {
            let mut f = Fixture::new(4);
            let index = f.encoded();
            f.worker
                .as_mut()
                .unwrap()
                .submitted(index, &Err(error), true);
            let a = f.take();
            assert_eq!(a.raw, [0; 8]);
            let record = &a.packets[0];
            assert_eq!(record.silence_frames, 1);
            assert_eq!(record.submission, submission);
            assert_eq!(record.completion, "not_submitted");
            assert_eq!(record.identity_current_after_submit, Some(true));
            assert_eq!(record.error_kind, Some(kind));
            assert_eq!(record.raw_hresult, None);
            assert_eq!(record.sequence, None);
        }
        let mut f = Fixture::new(4);
        let index = f.encoded();
        f.worker.as_mut().unwrap().fenced(index);
        let a = f.take();
        assert_eq!(a.packets[0].submission, "fenced");
        assert_eq!(a.packets[0].identity_current_after_submit, None);
        assert_eq!(a.packets[0].completion, "not_submitted");
        assert_eq!(a.excluded_epoch_packets, 1);
        assert_eq!(a.seal_reason, "identity_changed_before_submission");
    }

    #[test]
    fn output_capture_pending_observed_success_and_dropped_are_distinct() {
        use super::super::super::pcm_slots::SlotBook;
        let mut f = Fixture::new(8);
        let mut book = SlotBook::new().unwrap();
        let ticket = book.reserve().unwrap();
        let index = f.encoded();
        let worker = f.worker.as_mut().unwrap();
        worker.submitted(index, &Ok(ticket), true);
        worker.completed(ticket, &Ok(None));
        assert_eq!(worker.artifact.packets[0].completion, "pending");
        assert_eq!(worker.artifact.packets[0].sequence, None);
        worker.completed(
            ticket,
            &Ok(Some(PcmCompletion::Capture {
                stamp: stamp(),
                bytes: 8,
            })),
        );
        let next = worker.encoded(&f.state, 0, &[0; 8], 0);
        book.finish(ticket).unwrap();
        let dropped = book.reserve().unwrap();
        worker.submitted(next, &Ok(dropped), true);
        worker.completed(
            dropped,
            &Ok(Some(PcmCompletion::CaptureDropped { stamp: stamp() })),
        );
        let a = f.take();
        assert_eq!(a.packets[0].completion, "completed");
        assert_eq!(a.packets[0].completion_bytes, Some(8));
        assert_eq!(a.packets[0].sequence, Some(17));
        assert_eq!(a.packets[0].host_submission_qpc, Some(123_456));
        assert_eq!(a.packets[1].byte_offset, 8);
        assert_eq!(a.packets[1].completion, "dropped");
        assert_eq!(a.packets[1].completion_bytes, None);
        assert_eq!(a.packets[1].sequence, Some(17));
    }

    #[test]
    fn output_capture_wrong_direction_length_and_errors_preserve_observations() {
        use super::super::super::pcm_slots::SlotBook;
        let mut f = Fixture::new(8);
        let mut book = SlotBook::new().unwrap();
        let completions = [
            Ok(Some(PcmCompletion::Capture {
                stamp: stamp(),
                bytes: 7,
            })),
            Ok(Some(PcmCompletion::Render {
                stamp: stamp(),
                pcm: &[0; 8],
            })),
            Err(ClientError::KernelPcmQuotaFull),
            Err(ClientError::Cancelled),
        ];
        for result in completions {
            let index = f.encoded();
            let ticket = book.reserve().unwrap();
            let worker = f.worker.as_mut().unwrap();
            worker.submitted(index, &Ok(ticket), true);
            worker.completed(ticket, &result);
            book.finish(ticket).unwrap();
        }
        let a = f.take();
        assert_eq!(a.packets[0].completion, "wrong_shape");
        assert_eq!(a.packets[0].completion_bytes, Some(7));
        assert_eq!(a.packets[1].completion, "wrong_shape");
        assert_eq!(a.packets[1].host_submission_qpc, None);
        assert_eq!(a.packets[2].completion, "quota_rejected");
        assert_eq!(a.packets[3].error_kind, Some("cancelled"));
    }

    #[test]
    fn output_capture_retirement_and_join_do_not_invent_terminal_outcomes() {
        use super::super::super::pcm_slots::SlotBook;
        let mut f = Fixture::new(8);
        let ticket = SlotBook::new().unwrap().reserve().unwrap();
        let index = f.encoded();
        let worker = f.worker.as_mut().unwrap();
        worker.submitted(index, &Ok(ticket), true);
        worker.completed(ticket, &Err(ClientError::DeadlineExceeded));
        assert_eq!(worker.artifact.packets[0].completion, "error");
        worker.retired();
        worker.completed(
            ticket,
            &Ok(Some(PcmCompletion::Capture {
                stamp: stamp(),
                bytes: 8,
            })),
        );
        worker.finished(&Err(BackendError::Failed("retained worker failure".into())));
        let a = f.take();
        assert_eq!(a.packets[0].completion, "error");
        assert_eq!(a.packets[0].error_kind, Some("deadline_exceeded"));
        assert_eq!(a.packets[0].sequence, None);
        assert_eq!(a.worker_outcome, "failed");
        assert_eq!(a.worker_join, "joined");
        assert_eq!(a.untracked_completion_observations, 1);
    }

    #[test]
    fn output_capture_untracked_pending_polls_are_not_stale_completions() {
        use super::super::super::pcm_slots::SlotBook;
        let mut f = Fixture::new(1);
        let mut book = SlotBook::new().unwrap();
        let ticket = book.reserve().unwrap();
        assert_eq!(f.encoded(), Some(0));
        f.worker
            .as_mut()
            .unwrap()
            .submitted(Some(0), &Err(ClientError::KernelPcmQuotaFull), true);
        assert!(
            f.encoded().is_none(),
            "frame cap has stopped recording new attempts"
        );
        for _ in 0..20 {
            f.worker.as_mut().unwrap().completed(ticket, &Ok(None));
        }
        assert_eq!(
            f.worker
                .as_ref()
                .unwrap()
                .artifact
                .untracked_completion_observations,
            0
        );
        f.worker.as_mut().unwrap().completed(
            ticket,
            &Ok(Some(PcmCompletion::Capture {
                stamp: stamp(),
                bytes: 8,
            })),
        );
        assert_eq!(
            f.worker
                .as_ref()
                .unwrap()
                .artifact
                .untracked_completion_observations,
            1
        );
    }

    #[test]
    fn output_capture_unresolved_retirement_stays_unknown_after_clean_join() {
        use super::super::super::pcm_slots::SlotBook;
        let mut f = Fixture::new(8);
        let index = f.encoded();
        let ticket = SlotBook::new().unwrap().reserve().unwrap();
        let worker = f.worker.as_mut().unwrap();
        worker.submitted(index, &Ok(ticket), true);
        worker.finished(&Ok(()));
        let a = f.take();
        assert_eq!(a.worker_outcome, "completed");
        assert_eq!(a.worker_join, "joined");
        assert_eq!(a.packets[0].completion, "retired_completion_unobserved");
        assert_eq!(a.packets[0].completion_bytes, None);
        assert_eq!(a.packets[0].sequence, None);
        assert_eq!(a.packets[0].host_submission_qpc, None);
    }

    #[test]
    fn output_capture_opaque_ticket_reuse_cannot_complete_another_record() {
        use super::super::super::pcm_slots::SlotBook;
        let mut f = Fixture::new(16);
        let mut book = SlotBook::new().unwrap();
        let old = book.reserve().unwrap();
        let index = f.encoded();
        f.worker.as_mut().unwrap().submitted(index, &Ok(old), true);
        f.worker.as_mut().unwrap().completed(
            old,
            &Ok(Some(PcmCompletion::Capture {
                stamp: stamp(),
                bytes: 8,
            })),
        );
        book.finish(old).unwrap();
        let mut reused = None;
        for _ in 0..4 {
            let ticket = book.reserve().unwrap();
            if ticket.slot == old.slot {
                reused = Some(ticket);
                break;
            }
            book.finish(ticket).unwrap();
        }
        let reused = reused.unwrap();
        assert_ne!(reused, old);
        let index = f.encoded();
        let worker = f.worker.as_mut().unwrap();
        worker.submitted(index, &Ok(reused), true);
        worker.completed(
            old,
            &Ok(Some(PcmCompletion::Capture {
                stamp: stamp(),
                bytes: 8,
            })),
        );
        let foreign = SlotBook::new().unwrap().reserve().unwrap();
        worker.completed(foreign, &Err(ClientError::Cancelled));
        assert_eq!(worker.artifact.packets[1].completion, "pending");
        worker.completed(
            reused,
            &Ok(Some(PcmCompletion::CaptureDropped { stamp: stamp() })),
        );
        let a = f.take();
        assert_eq!(a.packets[0].completion, "completed");
        assert_eq!(a.packets[1].completion, "dropped");
        assert_eq!(a.untracked_completion_observations, 2);
    }

    #[test]
    fn output_capture_identity_change_seals_new_bytes_but_resolves_pending() {
        use super::super::super::pcm_slots::SlotBook;
        for race_after_submit in [false, true] {
            let mut f = Fixture::new(8);
            let index = f.encoded();
            let ticket = SlotBook::new().unwrap().reserve().unwrap();
            f.worker
                .as_mut()
                .unwrap()
                .submitted(index, &Ok(ticket), !race_after_submit);
            let identity = f.worker.as_ref().unwrap().owner.unwrap();
            f.worker.as_mut().unwrap().bind(identity);
            assert!(f.encoded().is_none());
            f.worker.as_mut().unwrap().completed(
                ticket,
                &Ok(Some(PcmCompletion::Capture {
                    stamp: stamp(),
                    bytes: 8,
                })),
            );
            let a = f.take();
            assert_eq!(a.stored_frames, 1);
            assert_eq!(a.packets[0].completion, "completed");
            assert_eq!(
                a.packets[0].identity_current_after_submit,
                Some(!race_after_submit)
            );
            assert_eq!(
                a.seal_reason,
                if race_after_submit {
                    "identity_changed_during_submission"
                } else {
                    "owner_reopened"
                }
            );
        }
        let mut f = Fixture::new(8);
        f.encoded();
        f.state.reset_epoch.store(1, Ordering::Release);
        assert!(f.encoded().is_none());
        assert_eq!(f.take().excluded_epoch_packets, 1);
    }

    #[test]
    fn output_capture_reservation_and_actual_join_are_one_shot() {
        let endpoint = VirtualEndpointId::from_bytes([2; 16]);
        for frames in [0, MAX_FRAMES + 1, u32::MAX] {
            assert!(CaptureShared::new(endpoint, frames).is_err());
        }
        let (shared, handle) = CaptureShared::new(endpoint, 1).unwrap();
        assert_eq!(handle.take_after_join().unwrap_err(), "not_started_unknown");
        assert!(
            shared
                .reserve(VirtualEndpointId::from_bytes([3; 16]))
                .unwrap()
                .is_none()
        );
        let worker = shared.reserve(endpoint).unwrap().unwrap();
        assert!(shared.reserve(endpoint).unwrap().is_none());
        assert_eq!(handle.take_after_join().unwrap_err(), "not_joined");
        drop(worker);
        assert_eq!(handle.take_after_join().unwrap_err(), "not_joined");
        shared.joined(true);
        assert_eq!(
            handle.take_after_join().unwrap().worker_join,
            "joined_panicked"
        );
        assert_eq!(handle.take_after_join().unwrap_err(), "already_taken");
        let (shared, handle) = CaptureShared::new(endpoint, 1).unwrap();
        shared.spawn_failed();
        assert_eq!(
            handle.take_after_join().unwrap_err(),
            "spawn_failed_no_worker_unknown"
        );
    }

    #[test]
    fn output_capture_whole_packet_bounds_keep_raw_and_record_storage_fixed() {
        let mut f = Fixture::new(1);
        assert!(
            f.worker
                .as_mut()
                .unwrap()
                .encoded(&f.state, 0, &[0; 16], 0)
                .is_none()
        );
        let a = f.take();
        assert_eq!(a.stored_frames, 0);
        assert!(a.raw.is_empty());
        assert_eq!(a.seal_reason, "capacity_whole_packet");

        let mut f = Fixture::new(MAX_FRAMES);
        let worker = f.worker.as_mut().unwrap();
        let pointers = (
            worker.artifact.raw.as_ptr(),
            worker.artifact.packets.as_ptr(),
        );
        crate::test_allocation::assert_allocates_nothing(|| {
            for _ in 0..=MAX_RECORDS {
                let index = worker.encoded(&f.state, 0, &[0; 8], 0);
                worker.submitted(index, &Err(ClientError::KernelPcmQuotaFull), true);
            }
        });
        assert_eq!(worker.artifact.raw.as_ptr(), pointers.0);
        assert_eq!(worker.artifact.packets.as_ptr(), pointers.1);
        assert_eq!(worker.artifact.packets.len(), MAX_RECORDS);
        assert!(worker.artifact.raw.len() <= MAX_BYTES);
        assert_eq!(worker.artifact.seal_reason, "capacity_whole_packet");
    }

    #[test]
    fn output_capture_max_frame_boundary_and_all_success_hooks_allocate_nothing() {
        use super::super::super::pcm_slots::SlotBook;
        let mut f = Fixture::new(MAX_FRAMES);
        let mut owner = f.worker.as_ref().unwrap().owner.unwrap();
        owner.packet_frames = 4800;
        f.worker.as_mut().unwrap().bind(owner);
        let packet = vec![0xa5; 4800 * 8];
        let mut book = SlotBook::new().unwrap();
        let worker = f.worker.as_mut().unwrap();
        let raw_capacity = worker.artifact.raw.capacity();
        let record_capacity = worker.artifact.packets.capacity();
        crate::test_allocation::assert_allocates_nothing(|| {
            for sequence in 1..=100 {
                let index = worker.encoded(&f.state, 0, &packet, 0).unwrap();
                let ticket = book.reserve().unwrap();
                worker.submitted(Some(index), &Ok(ticket), true);
                worker.completed(ticket, &Ok(None));
                worker.completed(
                    ticket,
                    &Ok(Some(PcmCompletion::Capture {
                        stamp: super::super::super::PcmPacketStamp {
                            sequence,
                            qpc: sequence + 100,
                        },
                        bytes: 38_400,
                    })),
                );
                book.finish(ticket).unwrap();
            }
            assert!(worker.encoded(&f.state, 0, &packet, 0).is_none());
        });
        assert_eq!(worker.artifact.stored_frames, 480_000);
        assert_eq!(worker.artifact.raw.len(), 3_840_000);
        assert_eq!(worker.artifact.raw.capacity(), raw_capacity);
        assert_eq!(worker.artifact.packets.capacity(), record_capacity);
        assert_eq!(worker.artifact.packets.len(), 100);
        assert_eq!(worker.artifact.packets[99].byte_offset, 3_801_600);
        assert_eq!(worker.artifact.seal_reason, "frame_capacity");
        assert!(
            worker
                .artifact
                .packets
                .iter()
                .all(|record| record.completion == "completed")
        );
    }

    #[test]
    fn output_capture_malformed_or_invalid_identity_is_never_repaired() {
        for (format, channels, frames) in [(3, 2, 1), (2, 0, 1), (2, 3, 1), (2, 2, 0), (2, 2, 4801)]
        {
            let mut f = Fixture::new(8);
            let mut owner = f.worker.as_ref().unwrap().owner.unwrap();
            owner.format = format;
            owner.channels = channels;
            owner.packet_frames = frames;
            f.worker.as_mut().unwrap().bind(owner);
            assert!(f.encoded().is_none());
            assert_eq!(f.take().seal_reason, "invalid_shape");
        }
        let mut f = Fixture::new(8);
        assert_eq!(
            f.worker.as_mut().unwrap().encoded(&f.state, 0, &[0; 7], 1),
            Some(0)
        );
        let a = f.take();
        assert_eq!(a.raw, [0; 7]);
        assert_eq!(a.packets[0].byte_count, 7);
        assert_eq!(a.packets[0].actual_frames, 0);
        assert_eq!(a.seal_reason, "short_or_wrong_packet");
    }

    #[test]
    fn output_capture_observed_owner_or_admission_change_seals_without_touching_pending() {
        use super::super::super::pcm_slots::SlotBook;
        let snapshot = snapshot();
        let endpoint = snapshot.stable_id().to_virtual_endpoint_id();
        let state = SessionState::new(Arc::new(Admission::new()));
        let mut owner = identity(
            &state,
            snapshot.sample_format().unwrap() as u32,
            snapshot.packet_frames(),
        );
        owner.stable_id = endpoint.to_bytes();
        owner.native_stamp = lifecycle::stamp(snapshot);
        let (shared, handle) = CaptureShared::new(endpoint, 960).unwrap();
        let mut worker = shared.reserve(endpoint).unwrap().unwrap();
        worker.bind(owner);
        let raw = vec![0; 480 * 8];
        let index = worker.encoded(&state, 0, &raw, 0);
        assert_eq!(index, Some(0));
        let ticket = SlotBook::new().unwrap().reserve().unwrap();
        worker.submitted(index, &Ok(ticket), true);
        worker.observe(snapshot, &state);
        assert_eq!(worker.artifact.seal_reason, "open");
        state.revoke();
        worker.observe(snapshot, &state);
        assert_eq!(worker.artifact.seal_reason, "identity_changed");
        worker.completed(
            ticket,
            &Ok(Some(PcmCompletion::Capture {
                stamp: stamp(),
                bytes: raw.len(),
            })),
        );
        assert_eq!(worker.artifact.packets[0].completion, "completed");
        drop(worker);
        shared.joined(false);
        assert_eq!(handle.take_after_join().unwrap().stored_frames, 480);
    }

    #[test]
    fn output_capture_windows_error_keeps_signed_hresult_without_formatting() {
        let mut f = Fixture::new(4);
        let index = f.encoded();
        let error = ClientError::Windows {
            operation: "test",
            source: windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8007_051a_u32 as i32,
            )),
        };
        crate::test_allocation::assert_allocates_nothing(|| {
            f.worker
                .as_mut()
                .unwrap()
                .submitted(index, &Err(error), true);
        });
        let a = f.take();
        assert_eq!(a.packets[0].error_kind, Some("windows"));
        assert_eq!(a.packets[0].raw_hresult, Some(0x8007_051a_u32 as i32));
    }
}
