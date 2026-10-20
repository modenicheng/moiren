//! Same-file endpoint ownership. All methods except admission invalidation run
//! on control workers; a failed IO never removes the record needed for recovery.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSpec {
    pub id: VirtualEndpointId,
    pub intent: Intent,
    pub channels: Channels,
    pub name: String,
    pub allow_default: bool,
}

#[derive(Debug, Clone)]
pub struct ExportView {
    pub spec: ExportSpec,
    pub snapshot: Option<super::super::EndpointSnapshot>,
    pub removing: bool,
    pub error: Option<String>,
    pub transport_prepared: bool,
}

pub(super) struct EndpointRecord {
    pub spec: ExportSpec,
    pub snapshot: Option<super::super::EndpointSnapshot>,
    pub removing: bool,
    // The driver revokes before the ACX call. A queried Removing state cannot
    // replace the successful REMOVE acknowledgment needed to set this flag.
    removal_accepted: bool,
    pub sessions: Vec<Arc<SessionState>>,
    pub error: Option<String>,
}

pub(super) trait EndpointControl {
    /// None is a validated no-resource deferral, never an uncertain IO failure.
    fn create(
        &mut self,
        spec: &ExportSpec,
    ) -> Result<Option<super::super::EndpointSnapshot>, BackendError>;
    fn query(
        &mut self,
        previous: super::super::EndpointSnapshot,
    ) -> Result<super::super::EndpointSnapshot, BackendError>;
    fn gate(
        &mut self,
        previous: super::super::EndpointSnapshot,
        open: bool,
    ) -> Result<super::super::EndpointSnapshot, BackendError>;
    fn remove(
        &mut self,
        previous: super::super::EndpointSnapshot,
    ) -> Result<super::super::EndpointSnapshot, BackendError>;
}

impl EndpointControl for ControlClient {
    fn create(
        &mut self,
        spec: &ExportSpec,
    ) -> Result<Option<super::super::EndpointSnapshot>, BackendError> {
        match self.create(
            StableEndpointId::from_virtual_endpoint_id(spec.id),
            spec.intent,
            spec.channels,
            &spec.name,
            spec.allow_default,
        ) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(ClientError::CreateDeferred) => Ok(None),
            Err(error) => Err(client_error(error)),
        }
    }
    fn query(
        &mut self,
        previous: super::super::EndpointSnapshot,
    ) -> Result<super::super::EndpointSnapshot, BackendError> {
        self.query(previous.key())
            .map_err(client_error)?
            .require_progress_after(previous)
            .map_err(|e| client_error(e.into()))
    }
    fn gate(
        &mut self,
        previous: super::super::EndpointSnapshot,
        open: bool,
    ) -> Result<super::super::EndpointSnapshot, BackendError> {
        if open {
            self.open_gate(previous)
        } else {
            self.close_gate(previous)
        }
        .map_err(client_error)
    }
    fn remove(
        &mut self,
        previous: super::super::EndpointSnapshot,
    ) -> Result<super::super::EndpointSnapshot, BackendError> {
        self.remove(previous.key()).map_err(client_error)
    }
}

pub(super) struct Lifecycle<C> {
    pub control: C,
    pub endpoints: BTreeMap<VirtualEndpointId, EndpointRecord>,
}

pub(super) fn stamp(snapshot: super::super::EndpointSnapshot) -> [u64; 5] {
    [
        snapshot.pdo_token().get(),
        snapshot.session_epoch(),
        snapshot.endpoint_generation(),
        snapshot.stream_generation(),
        snapshot.gate_epoch(),
    ]
}

fn unavailable(message: &str) -> BackendError {
    BackendError::ResourceUnavailable(message.into())
}

impl<C: EndpointControl> Lifecycle<C> {
    pub fn new(control: C) -> Self {
        Self {
            control,
            endpoints: BTreeMap::new(),
        }
    }

    pub fn create(&mut self, spec: ExportSpec) -> Result<(), BackendError> {
        // A local encoding failure never dispatched CREATE and must not strand
        // an unknown ownership record. Once IO starts, failures stay tracked.
        super::super::create_request(
            StableEndpointId::from_virtual_endpoint_id(spec.id),
            spec.intent,
            spec.channels,
            &spec.name,
            spec.allow_default,
        )
        .map_err(|error| client_error(error.into()))?;
        if let Some(record) = self.endpoints.get(&spec.id)
            && record
                .snapshot
                .is_none_or(|s| s.state() != PdoState::Removed)
        {
            return if record.spec == spec && !record.removing && record.error.is_none() {
                Ok(())
            } else {
                Err(unavailable(
                    "export recreation requires confirmed retirement",
                ))
            };
        }
        if !self.endpoints.contains_key(&spec.id)
            && self.endpoints.len() >= super::super::MAX_ENDPOINTS as usize
        {
            self.endpoints.retain(|_, record| {
                record
                    .snapshot
                    .is_none_or(|s| s.state() != PdoState::Removed)
            });
            if self.endpoints.len() >= super::super::MAX_ENDPOINTS as usize {
                return Err(unavailable("VAD endpoint tracking capacity reached"));
            }
        }
        // Publish ownership intent before CREATE. Even a timed out CREATE may
        // have reached the driver; never retry it as if no endpoint existed.
        self.endpoints.insert(
            spec.id,
            EndpointRecord {
                spec: spec.clone(),
                snapshot: None,
                removing: false,
                removal_accepted: false,
                sessions: Vec::new(),
                error: None,
            },
        );
        let result = self.control.create(&spec);
        let record = self.endpoints.get_mut(&spec.id).expect("reserved export");
        match result {
            Ok(Some(snapshot)) => {
                record.snapshot = Some(snapshot);
                Ok(())
            }
            Ok(None) => {
                // The driver explicitly did no work. Keep the desired export
                // absent so normal reconciliation can retry or disable it.
                self.endpoints.remove(&spec.id);
                Err(unavailable("prior endpoint destruction is still pending"))
            }
            Err(error) => {
                record.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub fn poll(&mut self) -> Vec<ExportView> {
        for record in self.endpoints.values_mut() {
            record
                .sessions
                .retain(|session| !session.drained.load(Ordering::Acquire));
            if let Some(previous) = record.snapshot
                && previous.state() != PdoState::Removed
            {
                match self.control.query(previous) {
                    Ok(snapshot) => {
                        record.snapshot = Some(snapshot);
                        record.error = ((snapshot.last_status() as i32) < 0).then(|| {
                            format!(
                                "VAD endpoint preparation failed: {:#x}",
                                snapshot.last_status()
                            )
                        });
                        if snapshot.gate_open()
                            && (record.removing
                                || record.error.is_some()
                                || !record.sessions.iter().any(|s| s.admitted()))
                        {
                            match self.control.gate(snapshot, false) {
                                Ok(snapshot) => record.snapshot = Some(snapshot),
                                Err(error) => record.error = Some(error.to_string()),
                            }
                        }
                        let snapshot = record.snapshot.expect("queried export");
                        if record.removing
                            && !record.removal_accepted
                            && snapshot.state() != PdoState::Removed
                            && !snapshot.gate_open()
                            && record.sessions.is_empty()
                        {
                            let _ = Self::submit_removal(&mut self.control, record, snapshot);
                        }
                    }
                    Err(error) => {
                        record.error = Some(error.to_string());
                        for session in &record.sessions {
                            session.revoke();
                        }
                    }
                }
            }
        }
        self.endpoints
            .values()
            .map(|record| ExportView {
                spec: record.spec.clone(),
                snapshot: record.snapshot,
                removing: record.removing,
                error: record.error.clone(),
                transport_prepared: record.sessions.iter().any(|s| s.prepared()),
            })
            .collect()
    }

    pub fn gate(
        &mut self,
        id: VirtualEndpointId,
        expected: [u64; 5],
        open: bool,
        revision: u64,
        ticket: u64,
        authority: &Admission,
    ) -> Result<(), BackendError> {
        let record = self
            .endpoints
            .get_mut(&id)
            .ok_or_else(|| unavailable("unknown export"))?;
        let previous = record
            .snapshot
            .ok_or_else(|| unavailable("uncertain CREATE"))?;
        if stamp(previous) != expected {
            return Err(BackendError::Cancelled);
        }
        let current = match self.control.query(previous) {
            Ok(current) => current,
            Err(error) => {
                record.error = Some(error.to_string());
                for session in &record.sessions {
                    session.revoke();
                }
                return Err(error);
            }
        };
        record.snapshot = Some(current);
        if stamp(current) != expected {
            return Err(BackendError::Cancelled);
        }
        if open
            && (record.removing
                || record.error.is_some()
                || !authority.matches(ticket, revision)
                || current.state() != PdoState::Present
                || !current.policy_prepared()
                || !current.audio_ready()
                || !record.sessions.iter().any(|s| s.prepared()))
        {
            return Err(BackendError::Cancelled);
        }
        if open {
            for session in &record.sessions {
                session.admitted_ticket.store(ticket, Ordering::Release);
                session.admitted_revision.store(revision, Ordering::Release);
            }
        }
        match self.control.gate(current, open) {
            Ok(snapshot) => record.snapshot = Some(snapshot),
            Err(error) => {
                record.error = Some(error.to_string());
                for session in &record.sessions {
                    session.revoke();
                }
                return Err(error);
            }
        }
        // Stop or a revision switch can race an overlapped SET_GATE. Revoke
        // that result on the same worker rather than publish stale admission.
        if open && !authority.matches(ticket, revision) {
            for session in &record.sessions {
                session.revoke();
            }
            let snapshot = record.snapshot.expect("SET_GATE response");
            match self.control.gate(snapshot, false) {
                Ok(snapshot) => record.snapshot = Some(snapshot),
                Err(error) => {
                    record.error = Some(error.to_string());
                    return Err(error);
                }
            }
            return Err(BackendError::Cancelled);
        }
        Ok(())
    }

    pub fn retire(
        &mut self,
        id: VirtualEndpointId,
        expected: [u64; 5],
    ) -> Result<(), BackendError> {
        let record = self
            .endpoints
            .get_mut(&id)
            .ok_or_else(|| unavailable("unknown export"))?;
        if record
            .sessions
            .iter()
            .any(|s| !s.drained.load(Ordering::Acquire))
        {
            return Err(BackendError::Cancelled);
        }
        let previous = record
            .snapshot
            .ok_or_else(|| unavailable("uncertain CREATE cannot be retired by key"))?;
        if stamp(previous) != expected {
            return Err(BackendError::Cancelled);
        }
        if previous.state() == PdoState::Removed {
            return Ok(());
        }
        let current = match self.control.query(previous) {
            Ok(current) => current,
            Err(error) => {
                record.error = Some(error.to_string());
                for session in &record.sessions {
                    session.revoke();
                }
                return Err(error);
            }
        };
        record.snapshot = Some(current);
        if stamp(current) != expected {
            return Err(BackendError::Cancelled);
        }
        if current.gate_open() {
            return Err(unavailable("close export gate before retirement"));
        }
        record.removing = true;
        if record.removal_accepted || current.state() == PdoState::Removed {
            return Ok(());
        }
        Self::submit_removal(&mut self.control, record, current)
    }

    fn submit_removal(
        control: &mut C,
        record: &mut EndpointRecord,
        previous: super::super::EndpointSnapshot,
    ) -> Result<(), BackendError> {
        // Keep the key on both failure and Removing. Only a real Removed
        // observation permits recreation, including eligibility/name changes.
        let result = control.remove(previous).and_then(|snapshot| {
            snapshot
                .removal_complete_after(previous)
                .map_err(|error| client_error(error.into()))?;
            Ok(snapshot)
        });
        match result {
            Ok(snapshot) => {
                record.snapshot = Some(snapshot);
                record.removal_accepted = true;
                record.error = None;
                Ok(())
            }
            Err(error) => {
                record.error = Some(error.to_string());
                Err(error)
            }
        }
    }
}

pub(super) struct Admission {
    ticket: AtomicU64,
    revision: AtomicU64,
}
impl Admission {
    pub fn new() -> Self {
        Self {
            ticket: AtomicU64::new(1),
            revision: AtomicU64::new(0),
        }
    }
    pub fn set(&self, revision: Option<u64>) -> u64 {
        self.revision.store(0, Ordering::Release);
        let Ok(previous) = self
            .ticket
            .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
        else {
            return 0;
        };
        let ticket = previous + 1;
        self.revision
            .store(revision.unwrap_or(0), Ordering::Release);
        ticket
    }
    pub fn matches(&self, ticket: u64, revision: u64) -> bool {
        revision != 0
            && self.ticket.load(Ordering::Acquire) == ticket
            && self.revision.load(Ordering::Acquire) == revision
            && self.ticket.load(Ordering::Acquire) == ticket
    }
}

#[cfg(test)]
mod tests;
