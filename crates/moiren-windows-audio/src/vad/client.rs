use super::{
    CAPABILITIES_SIZE, Capabilities, Channels, DeviceInterface, EndpointKey, EndpointSnapshot,
    IOCTL_CAPABILITIES, IOCTL_CREATE, IOCTL_QUERY, IOCTL_REMOVE, IOCTL_SET_GATE, Intent, PcmOwner,
    PdoState, ProtocolError, SNAPSHOT_SIZE, StableEndpointId, capabilities_request,
    control_io::ControlHandle, create_request, endpoint_request, gate_request,
};
use std::{
    fmt,
    time::{Duration, Instant},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Debug)]
pub enum ClientError {
    Windows {
        operation: &'static str,
        source: windows::core::Error,
    },
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
    Protocol(ProtocolError),
    DeadlineExceeded,
    Cancelled,
    InvalidDeadline,
    SessionUncertain,
    CreateDeferred,
    PreparationFailed {
        status: u32,
    },
    PcmOwnerExists,
    NoFreePcmSlot,
    InvalidPcmTicket,
    SequenceExhausted,
    /// The driver rejected a PCM request because this endpoint's kernel
    /// request quota is still occupied — typically cancelled-but-unprocessed
    /// requests of a just-retired owner. Transient by contract; the pump
    /// retries on a later pass instead of failing the session.
    KernelPcmQuotaFull,
}

impl ClientError {
    pub(super) fn windows(operation: &'static str, source: windows::core::Error) -> Self {
        Self::Windows { operation, source }
    }
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows { operation, source } => write!(f, "{operation}: {source}"),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Protocol(error) => error.fmt(f),
            Self::DeadlineExceeded => f.write_str("VAD request deadline expired"),
            Self::Cancelled => f.write_str("VAD operation cancelled"),
            Self::InvalidDeadline => f.write_str("VAD deadline must be positive and finite"),
            Self::SessionUncertain => f.write_str("VAD session has uncertain IO completion"),
            Self::CreateDeferred => {
                f.write_str("VAD creation deferred until the prior endpoint is destroyed")
            }
            Self::PreparationFailed { status } => {
                write!(f, "VAD preparation failed NTSTATUS={status:#010x}")
            }
            Self::PcmOwnerExists => {
                f.write_str("VAD endpoint already has a PCM owner or owner quota is full")
            }
            Self::NoFreePcmSlot => f.write_str("all four VAD PCM slots are occupied"),
            Self::InvalidPcmTicket => f.write_str("invalid or retired VAD PCM ticket"),
            Self::SequenceExhausted => f.write_str("VAD PCM sequence or ticket counter exhausted"),
            Self::KernelPcmQuotaFull => {
                f.write_str("VAD kernel PCM request quota is temporarily full")
            }
        }
    }
}
impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Windows { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::Protocol(error) => Some(error),
            _ => None,
        }
    }
}
impl From<ProtocolError> for ClientError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

fn created_reply(
    bytes: &[u8],
    capabilities: Capabilities,
    id: StableEndpointId,
    intent: Intent,
    channels: Channels,
) -> Result<EndpointSnapshot, ClientError> {
    match super::protocol::decode_create_reply(bytes, capabilities, id, intent, channels)? {
        super::protocol::CreateReply::Created(snapshot) => Ok(snapshot),
        super::protocol::CreateReply::Deferred => Err(ClientError::CreateDeferred),
    }
}

/// One owning file object, not a process identity. No methods belong on render.
///
/// Opens and same-file PCM duplicates are non-inheritable. A PCM owner retains
/// the original handle so dropping this client cannot close it during direct IO.
/// Cleanup runs after the final same-file handle closes and is asynchronous.
pub struct ControlClient {
    io: ControlHandle,
    capabilities: Capabilities,
}

impl ControlClient {
    pub fn connect(interface: &DeviceInterface) -> Result<Self, ClientError> {
        let mut io = ControlHandle::open(interface)?;
        let bytes = io.exchange::<CAPABILITIES_SIZE>(
            IOCTL_CAPABILITIES,
            &capabilities_request(),
            deadline_after(REQUEST_TIMEOUT)?,
        )?;
        Ok(Self {
            io,
            capabilities: Capabilities::decode(&bytes)?,
        })
    }
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Duplicate the control view without opening a second driver session.
    /// Endpoint workers use this so each worker can serialize its own control
    /// operations while all workers retain the one owning file object.
    pub fn duplicate(&self) -> Self {
        Self {
            io: self.io.duplicate(),
            capabilities: self.capabilities,
        }
    }

    /// Returns the accepted identity in Creating or Present; this is not a ready
    /// audio admission result. `allow_default` is a separate local preference.
    /// `CreateDeferred` acknowledges that no resource was created while a prior
    /// session's same-ID child retires. Only that reply authorizes a fresh retry.
    pub fn create(
        &mut self,
        id: StableEndpointId,
        intent: Intent,
        channels: Channels,
        name: &str,
        allow_default: bool,
    ) -> Result<EndpointSnapshot, ClientError> {
        self.capabilities.validate_create(intent, channels)?;
        let bytes = self.io.exchange::<SNAPSHOT_SIZE>(
            IOCTL_CREATE,
            &create_request(id, intent, channels, name, allow_default)?,
            deadline_after(REQUEST_TIMEOUT)?,
        )?;
        created_reply(&bytes, self.capabilities, id, intent, channels)
    }

    /// Wait for asynchronous preparation with one deadline, including CREATE.
    pub fn create_and_wait(
        &mut self,
        id: StableEndpointId,
        intent: Intent,
        channels: Channels,
        name: &str,
        allow_default: bool,
        timeout: Duration,
    ) -> Result<EndpointSnapshot, ClientError> {
        let deadline = deadline_after(timeout)?;
        self.capabilities.validate_create(intent, channels)?;
        let bytes = self.io.exchange::<SNAPSHOT_SIZE>(
            IOCTL_CREATE,
            &create_request(id, intent, channels, name, allow_default)?,
            deadline,
        )?;
        let snapshot = created_reply(&bytes, self.capabilities, id, intent, channels)?;
        let result = self.wait_present_at(snapshot, deadline);
        finish_created_endpoint(snapshot.key(), result, |key| {
            self.remove_and_wait(key, REQUEST_TIMEOUT).map(|_| ())
        })
    }

    /// Cancellation-aware creation for backend preparation. The underlying
    /// overlapped CREATE remains bounded by the control deadline; the async
    /// PnP wait checks cancellation between each query and never waits for a
    /// native stream or client.
    #[allow(clippy::too_many_arguments)]
    pub fn create_and_wait_cancellable(
        &mut self,
        id: StableEndpointId,
        intent: Intent,
        channels: Channels,
        name: &str,
        allow_default: bool,
        timeout: Duration,
        cancelled: impl Fn() -> bool,
    ) -> Result<EndpointSnapshot, ClientError> {
        let deadline = deadline_after(timeout)?;
        if cancelled() {
            return Err(ClientError::Cancelled);
        }
        self.capabilities.validate_create(intent, channels)?;
        let bytes = self.io.exchange::<SNAPSHOT_SIZE>(
            IOCTL_CREATE,
            &create_request(id, intent, channels, name, allow_default)?,
            deadline,
        )?;
        let snapshot = created_reply(&bytes, self.capabilities, id, intent, channels)?;
        let result = self.wait_present_at_cancellable(snapshot, deadline, &cancelled);
        finish_created_endpoint(snapshot.key(), result, |key| {
            self.remove_and_wait(key, REQUEST_TIMEOUT).map(|_| ())
        })
    }

    /// Resume a preparation wait after CREATE returned its runtime identity.
    pub fn wait_present(
        &mut self,
        snapshot: EndpointSnapshot,
        timeout: Duration,
    ) -> Result<EndpointSnapshot, ClientError> {
        self.validate_key(snapshot.key())?;
        self.wait_present_at(snapshot, deadline_after(timeout)?)
    }

    fn wait_present_at(
        &mut self,
        previous: EndpointSnapshot,
        deadline: Instant,
    ) -> Result<EndpointSnapshot, ClientError> {
        self.wait_present_at_cancellable(previous, deadline, &|| false)
    }

    fn wait_present_at_cancellable(
        &mut self,
        mut previous: EndpointSnapshot,
        deadline: Instant,
        cancelled: &impl Fn() -> bool,
    ) -> Result<EndpointSnapshot, ClientError> {
        loop {
            if cancelled() {
                return Err(ClientError::Cancelled);
            }
            if (previous.last_status() as i32) < 0 {
                return Err(ClientError::PreparationFailed {
                    status: previous.last_status(),
                });
            }
            match previous.state() {
                PdoState::Present => {
                    if self.capabilities.flags() & super::CAP_POLICY_STAGED != 0
                        && !previous.policy_prepared()
                    {
                        return Err(ProtocolError::Inconsistent(
                            "present endpoint lacks staged policy",
                        )
                        .into());
                    }
                    return Ok(previous);
                }
                PdoState::Creating => {}
                _ => {
                    return Err(
                        ProtocolError::Inconsistent("endpoint removed during preparation").into(),
                    );
                }
            }
            poll_delay(deadline)?;
            previous = self
                .query_at(previous.key(), deadline)?
                .require_progress_after(previous)?;
        }
    }

    pub fn query(&mut self, key: EndpointKey) -> Result<EndpointSnapshot, ClientError> {
        self.query_at(key, deadline_after(REQUEST_TIMEOUT)?)
    }
    pub fn remove(&mut self, key: EndpointKey) -> Result<EndpointSnapshot, ClientError> {
        self.remove_at(key, deadline_after(REQUEST_TIMEOUT)?)
    }

    /// Compare the caller's exact gate epoch. A stale close can only confirm
    /// the same stream is now closed; it never retries SET_GATE.
    pub fn close_gate(
        &mut self,
        snapshot: EndpointSnapshot,
    ) -> Result<EndpointSnapshot, ClientError> {
        let result = self.set_gate(snapshot, false);
        finish_close(snapshot, result, || self.query(snapshot.key()))
    }
    pub fn open_gate(
        &mut self,
        snapshot: EndpointSnapshot,
    ) -> Result<EndpointSnapshot, ClientError> {
        self.capabilities.require_pcm()?;
        if snapshot.state() != PdoState::Present
            || !snapshot.policy_prepared()
            || !snapshot.audio_ready()
        {
            return Err(ProtocolError::NotReady.into());
        }
        self.set_gate(snapshot, true)
    }
    fn set_gate(
        &mut self,
        previous: EndpointSnapshot,
        open: bool,
    ) -> Result<EndpointSnapshot, ClientError> {
        self.validate_key(previous.key())?;
        let bytes = self.io.exchange::<SNAPSHOT_SIZE>(
            IOCTL_SET_GATE,
            &gate_request(previous, open),
            deadline_after(REQUEST_TIMEOUT)?,
        )?;
        let snapshot = EndpointSnapshot::decode(&bytes, self.capabilities)?
            .require_progress_after(previous)?;
        if snapshot.gate_open() != open
            || (open
                && (snapshot.state() != PdoState::Present
                    || snapshot.stream_generation() != previous.stream_generation()
                    || snapshot.gate_epoch() != previous.gate_epoch()))
            // Closing an already closed gate is idempotent in the driver. A
            // real close advances the epoch and invalidates queued PCM.
            || (!open
                && previous.gate_open()
                && snapshot.gate_epoch() <= previous.gate_epoch())
        {
            return Err(ProtocolError::Inconsistent("SET_GATE state or epoch mismatch").into());
        }
        Ok(snapshot)
    }

    pub fn remove_and_wait(
        &mut self,
        key: EndpointKey,
        timeout: Duration,
    ) -> Result<EndpointSnapshot, ClientError> {
        let deadline = deadline_after(timeout)?;
        let mut previous = self.remove_at(key, deadline)?;
        loop {
            if previous.removal_complete_after(previous)? {
                return Ok(previous);
            }
            poll_delay(deadline)?;
            let current = self.query_at(key, deadline)?;
            if current.removal_complete_after(previous)? {
                return Ok(current);
            }
            previous = current;
        }
    }

    /// Control/IO-owner side only: allocates four fixed slots and events.
    /// One helper per endpoint is enforced across this file object's clones.
    pub fn pcm_owner(&self, snapshot: EndpointSnapshot) -> Result<PcmOwner, ClientError> {
        self.validate_key(snapshot.key())?;
        let shape = snapshot.pcm_shape(self.capabilities)?;
        PcmOwner::new(self.io.pcm_lease(snapshot.key())?, self.capabilities, shape)
    }

    fn validate_key(&self, key: EndpointKey) -> Result<(), ClientError> {
        if key.session_epoch() != self.capabilities.session_epoch() {
            return Err(ProtocolError::Inconsistent("request belongs to another session").into());
        }
        Ok(())
    }
    fn query_at(
        &mut self,
        key: EndpointKey,
        deadline: Instant,
    ) -> Result<EndpointSnapshot, ClientError> {
        self.validate_key(key)?;
        let bytes =
            self.io
                .exchange::<SNAPSHOT_SIZE>(IOCTL_QUERY, &endpoint_request(key), deadline)?;
        Ok(EndpointSnapshot::decode(&bytes, self.capabilities)?.require_key(key)?)
    }
    fn remove_at(
        &mut self,
        key: EndpointKey,
        deadline: Instant,
    ) -> Result<EndpointSnapshot, ClientError> {
        self.validate_key(key)?;
        let bytes =
            self.io
                .exchange::<SNAPSHOT_SIZE>(IOCTL_REMOVE, &endpoint_request(key), deadline)?;
        let snapshot = EndpointSnapshot::decode(&bytes, self.capabilities)?.require_key(key)?;
        snapshot.removal_complete_after(snapshot)?;
        Ok(snapshot)
    }
}

pub(super) fn deadline_after(timeout: Duration) -> Result<Instant, ClientError> {
    if timeout.is_zero() {
        return Err(ClientError::InvalidDeadline);
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or(ClientError::InvalidDeadline)
}

fn finish_created_endpoint<T>(
    key: EndpointKey,
    result: Result<T, ClientError>,
    cleanup: impl FnOnce(EndpointKey) -> Result<(), ClientError>,
) -> Result<T, ClientError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match cleanup(key) {
            Ok(()) => Err(error),
            Err(_) => Err(ClientError::SessionUncertain),
        },
    }
}

fn finish_close(
    previous: EndpointSnapshot,
    result: Result<EndpointSnapshot, ClientError>,
    query: impl FnOnce() -> Result<EndpointSnapshot, ClientError>,
) -> Result<EndpointSnapshot, ClientError> {
    let Err(error) = result else { return result };
    if !matches!(
        &error,
        ClientError::Windows { source, .. }
            if source.code() == windows::Win32::Foundation::ERROR_REVISION_MISMATCH.to_hresult()
    ) {
        return Err(error);
    }
    // Lifecycle and PCM workers share the file but can race to close the same
    // gate. Confirm only the same stream is now closed. Never retry SET_GATE
    // using a refreshed epoch: that could close a newly admitted stream/revision.
    let current = match query() {
        Ok(snapshot) => match snapshot.require_progress_after(previous) {
            Ok(current) => current,
            Err(progress_error) => {
                log::debug!(
                    target: "moiren::audio::vad",
                    "event=gate_close_confirmation decision=invalid_progress previous={previous:?} confirmation={snapshot:?} close_error={error:?} progress_error={progress_error:?}"
                );
                return Err(progress_error.into());
            }
        },
        Err(query_error) => {
            log::debug!(
                target: "moiren::audio::vad",
                "event=gate_close_confirmation decision=query_failed previous={previous:?} confirmation={query_error:?} close_error={error:?}"
            );
            return Err(query_error);
        }
    };
    let decision = if current.stream_generation() != previous.stream_generation() {
        "rejected_changed_stream"
    } else if current.gate_open() {
        "rejected_still_open"
    } else if current.gate_epoch() <= previous.gate_epoch() {
        "rejected_epoch_not_advanced"
    } else {
        "accepted_closed_same_stream"
    };
    log::debug!(
        target: "moiren::audio::vad",
        "event=gate_close_confirmation decision={decision} previous={previous:?} confirmation={current:?} close_error={error:?}"
    );
    if !current.gate_open()
        && current.stream_generation() == previous.stream_generation()
        && current.gate_epoch() > previous.gate_epoch()
    {
        Ok(current)
    } else {
        Err(error)
    }
}

fn poll_delay(deadline: Instant) -> Result<(), ClientError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(ClientError::DeadlineExceeded)?;
    std::thread::sleep(remaining.min(POLL_INTERVAL));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> EndpointKey {
        EndpointKey::new(crate::vad::RuntimePdoToken::new(1).unwrap(), 2, 3).unwrap()
    }

    fn gate_snapshot(stream: u64, gate: u64, open: bool) -> EndpointSnapshot {
        gate_snapshot_for(7, 11, stream, gate, open)
    }

    fn gate_snapshot_for(
        session: u64,
        token: u64,
        stream: u64,
        gate: u64,
        open: bool,
    ) -> EndpointSnapshot {
        let mut caps = [0_u8; CAPABILITIES_SIZE];
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
        caps[40..48].copy_from_slice(&session.to_le_bytes());
        let mut bytes = [0_u8; SNAPSHOT_SIZE];
        for (offset, value) in [
            (0, 128_u32),
            (4, 2),
            (16, PdoState::Present as u32),
            (20, Intent::Microphone as u32),
            (24, 2),
            (28, u32::from(open)),
            (32, 1),
            (36, 1),
            (88, 2),
            (92, 480),
            (100, 2),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (8, token),
            (40, session),
            (48, token),
            (56, stream),
            (64, gate),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        EndpointSnapshot::decode(&bytes, Capabilities::decode(&caps).unwrap()).unwrap()
    }

    fn revision_mismatch() -> ClientError {
        ClientError::windows(
            "GetOverlappedResultEx(VAD)",
            windows::core::Error::from_hresult(
                windows::Win32::Foundation::ERROR_REVISION_MISMATCH.to_hresult(),
            ),
        )
    }

    #[test]
    fn concurrent_close_confirms_already_closed_same_stream_without_retry() {
        let previous = gate_snapshot(3, 5, true);
        let closed = gate_snapshot(3, 6, false);
        let calls = std::cell::Cell::new(0);
        let result = finish_close(previous, Err(revision_mismatch()), || {
            calls.set(calls.get() + 1);
            Ok(closed)
        });
        assert_eq!(result.unwrap(), closed);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn close_confirmation_never_accepts_reopened_or_changed_streams() {
        let previous = gate_snapshot(3, 5, true);
        for current in [
            gate_snapshot(3, 6, true),
            gate_snapshot(4, 6, false),
            gate_snapshot(3, 5, false),
        ] {
            let result = finish_close(previous, Err(revision_mismatch()), || Ok(current));
            assert!(matches!(result, Err(ClientError::Windows { .. })));
        }
    }

    #[test]
    fn close_confirmation_preserves_real_errors_without_querying() {
        let previous = gate_snapshot(3, 5, true);
        for error in [
            ClientError::DeadlineExceeded,
            ClientError::SessionUncertain,
            ClientError::windows(
                "GetOverlappedResultEx(VAD)",
                windows::core::Error::from_hresult(
                    windows::Win32::Foundation::ERROR_DEVICE_NOT_CONNECTED.to_hresult(),
                ),
            ),
        ] {
            assert!(finish_close(previous, Err(error), || panic!("not a close race")).is_err());
        }
        assert!(matches!(
            finish_close(previous, Err(revision_mismatch()), || Err(
                ClientError::DeadlineExceeded
            )),
            Err(ClientError::DeadlineExceeded)
        ));
        let regressed = gate_snapshot(2, 6, false);
        assert!(matches!(
            finish_close(previous, Err(revision_mismatch()), || Ok(regressed)),
            Err(ClientError::Protocol(_))
        ));
    }

    #[test]
    fn close_confirmation_rejects_foreign_identity_and_regressed_epochs() {
        let previous = gate_snapshot(3, 5, true);
        for current in [
            gate_snapshot_for(8, 11, 3, 6, false),
            gate_snapshot_for(7, 12, 3, 6, false),
            gate_snapshot(3, 4, false),
        ] {
            assert!(matches!(
                finish_close(previous, Err(revision_mismatch()), || Ok(current)),
                Err(ClientError::Protocol(_))
            ));
        }
    }

    #[test]
    fn successful_close_never_adds_a_query() {
        let previous = gate_snapshot(3, 5, true);
        let closed = gate_snapshot(3, 6, false);
        assert_eq!(
            finish_close(previous, Ok(closed), || panic!("already complete")).unwrap(),
            closed
        );
    }

    #[test]
    fn close_confirmation_log_child() {
        let Ok(filter) = std::env::var("MOIREN_CLOSE_CONFIRMATION_LOG_CHILD") else {
            return;
        };
        let _logger = if filter == "off" {
            None
        } else {
            Some(
                flexi_logger::Logger::try_with_str(filter)
                    .unwrap()
                    .log_to_stderr()
                    .start()
                    .unwrap(),
            )
        };
        let previous = gate_snapshot(3, 5, true);
        for (snapshot, accepted, protocol_error) in [
            (gate_snapshot(3, 6, false), true, false),
            (gate_snapshot(4, 6, false), false, false),
            (gate_snapshot(3, 6, true), false, false),
            (gate_snapshot(3, 5, false), false, false),
            (gate_snapshot_for(8, 11, 3, 6, false), false, true),
        ] {
            let calls = std::cell::Cell::new(0);
            let result = finish_close(previous, Err(revision_mismatch()), || {
                calls.set(calls.get() + 1);
                Ok(snapshot)
            });
            assert_eq!(calls.get(), 1);
            assert_eq!(result.is_ok(), accepted);
            if protocol_error {
                assert!(matches!(result, Err(ClientError::Protocol(_))));
            } else if !accepted {
                assert!(matches!(result, Err(ClientError::Windows { .. })));
            }
        }
        let calls = std::cell::Cell::new(0);
        assert!(matches!(
            finish_close(previous, Err(revision_mismatch()), || {
                calls.set(calls.get() + 1);
                Err(ClientError::DeadlineExceeded)
            }),
            Err(ClientError::DeadlineExceeded)
        ));
        assert_eq!(calls.get(), 1);
        finish_close(previous, Ok(gate_snapshot(3, 6, false)), || {
            panic!("successful close must not query")
        })
        .unwrap();
        assert!(matches!(
            finish_close(previous, Err(ClientError::Cancelled), || {
                panic!("unrelated error must not query")
            }),
            Err(ClientError::Cancelled)
        ));
    }

    fn close_confirmation_child(filter: &str) -> std::process::Output {
        // Isolate the global logger from other library logger tests. No native
        // handle or device is opened by the actual finish_close helper.
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "vad::client::tests::close_confirmation_log_child",
                "--nocapture",
            ])
            .env("MOIREN_CLOSE_CONFIRMATION_LOG_CHILD", filter)
            .output()
            .unwrap()
    }

    #[test]
    fn close_confirmation_debug_records_single_query_and_unchanged_decisions() {
        let output = close_confirmation_child("moiren::audio::vad=debug");
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(stderr.matches("event=gate_close_confirmation ").count(), 6);
        for decision in [
            "accepted_closed_same_stream",
            "rejected_changed_stream",
            "rejected_still_open",
            "rejected_epoch_not_advanced",
            "invalid_progress",
            "query_failed",
        ] {
            let line = stderr
                .lines()
                .find(|line| line.contains(&format!("decision={decision}")))
                .unwrap_or_else(|| panic!("missing {decision}: {stderr}"));
            assert!(line.contains("previous=EndpointSnapshot"), "{line}");
            assert!(line.contains("confirmation="), "{line}");
            assert!(line.contains("close_error="), "{line}");
        }
    }

    #[test]
    fn close_confirmation_logging_disabled_preserves_results_without_output() {
        let output = close_confirmation_child("off");
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn deadlines_reject_zero_and_instant_overflow() {
        assert!(matches!(
            deadline_after(Duration::ZERO),
            Err(ClientError::InvalidDeadline)
        ));
        assert!(matches!(
            deadline_after(Duration::MAX),
            Err(ClientError::InvalidDeadline)
        ));
        assert!(deadline_after(Duration::from_secs(1)).unwrap() > Instant::now());
    }

    #[test]
    fn cancellation_after_create_removes_the_accepted_endpoint() {
        let key = test_key();
        let mut removed = None;

        let result =
            finish_created_endpoint::<()>(key, Err(ClientError::Cancelled), |removed_key| {
                removed = Some(removed_key);
                Ok(())
            });

        assert!(matches!(result, Err(ClientError::Cancelled)));
        assert_eq!(removed, Some(key));
    }

    #[test]
    fn failed_cleanup_marks_the_control_session_uncertain() {
        let result = finish_created_endpoint::<()>(test_key(), Err(ClientError::Cancelled), |_| {
            Err(ClientError::DeadlineExceeded)
        });

        assert!(matches!(result, Err(ClientError::SessionUncertain)));
    }
}
