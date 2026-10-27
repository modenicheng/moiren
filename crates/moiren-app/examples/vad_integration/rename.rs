//! Independent service, native lifecycle and Windows property evidence.
use anyhow::{Context, ensure};
use serde::Serialize;
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "rename/target.rs"]
pub(super) mod target;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct IntentEvidence {
    pub endpoint_id: String,
    pub direction: String,
    pub channels: usize,
    pub binding: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct RetirementEvidence {
    pub generation: u64,
    /// Only the production adapter's actual fresh Removed QUERY supplies this.
    pub native_stamp: [u64; 5],
}

impl From<moiren_app::service::ExportRetirementEvidence> for RetirementEvidence {
    fn from(value: moiren_app::service::ExportRetirementEvidence) -> Self {
        Self {
            generation: value.generation,
            native_stamp: value.native_stamp,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RuntimeEvidence {
    pub intent: IntentEvidence,
    pub requested_name: String,
    pub provider_name: Option<String>,
    pub windows_name: Option<String>,
    pub windows_name_identity: Option<[u64; 3]>,
    pub windows_name_mmdevice_id: Option<String>,
    pub windows_name_error: Option<String>,
    pub rename_pending: bool,
    pub resource: String,
    pub observation_generation: Option<u64>,
    /// Fresh provider QUERY [pdo, session, endpoint, stream, gate].
    pub native_stamp: Option<[u64; 5]>,
    pub native_retirement: Option<RetirementEvidence>,
    pub gate_open: bool,
    pub allow_default: bool,
    pub export_active_revision: Option<u64>,
    pub candidate_revision: u64,
    pub active_revision: Option<u64>,
    pub pending_revision: Option<u64>,
    pub consumed_request_id: u64,
    pub phase: String,
    pub service_error: Option<String>,
    pub graph_error: Option<String>,
    pub provider_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum PropertyReadError {
    OpenPropertyStore { hresult: i32, message: String },
    GetValue { hresult: i32, message: String },
    UnexpectedType { vartype: u16 },
    NullString,
    InvalidUtf16 { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum PropertyReadback {
    Value { value: String },
    Error { error: PropertyReadError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum TopologyReadError {
    Hresult {
        stage: &'static str,
        hresult: i32,
    },
    ConfigRet {
        stage: &'static str,
        code: u32,
    },
    ConnectorCount {
        count: u32,
    },
    NullString {
        field: &'static str,
    },
    UnterminatedString {
        field: &'static str,
        max_units: usize,
    },
    InvalidUtf16 {
        field: &'static str,
    },
    PropertyType {
        property_type: u32,
    },
    PropertyBytes {
        bytes: u32,
        capacity_bytes: usize,
    },
    PropertyTermination,
    UnexpectedDetailSuccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum TopologyRead<T> {
    Value { value: T },
    Error { error: TopologyReadError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TypedInterfaceName {
    /// Unavailable DDI output is unknown, never a fabricated type/byte count.
    pub property_type: Option<u32>,
    pub required_bytes: Option<u32>,
    pub readback: TopologyRead<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SetupInterfaceEvidence {
    /// The opaque topology ID is not guaranteed to be a SetupAPI path.
    /// This HRESULT comes only from SetupDiOpenDeviceInterfaceW(exact ID).
    Rejected {
        hresult: i32,
    },
    Error {
        error: TopologyReadError,
    },
    Opened {
        interface_class: String,
        pdo_instance: TopologyRead<String>,
        parent_instance: TopologyRead<String>,
        friendly_name: TypedInterfaceName,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct AdapterDeviceEvidence {
    pub device_id: TopologyRead<String>,
    pub device_desc: PropertyReadback,
    pub friendly_name: PropertyReadback,
    pub interface_friendly_name: PropertyReadback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TopologyEvidence {
    /// Exact GetDeviceIdConnectedTo output: no prefix removal/path guessing.
    pub adapter_id: TopologyRead<String>,
    /// Not attempted if the topology ID could not be read within the bound.
    pub setup_interface: Option<SetupInterfaceEvidence>,
    /// Resolve the opaque topology ID through MMDevice, independently of SetupAPI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_mmdevice: Option<TopologyRead<AdapterDeviceEvidence>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct NativeName {
    pub root_instance: String,
    pub stable_id: String,
    pub direction: String,
    pub mmdevice_id: String,
    pub adapter_instance: String,
    /// Exact PKEY_Device_FriendlyName; never the provider's spec_name.
    pub windows_friendly_name: String,
    /// Optional actual MMDevice diagnostics; never acceptance substitutes.
    pub device_desc: PropertyReadback,
    pub device_interface_friendly_name: PropertyReadback,
    /// Collected once per HeldClient, in baseline/fresh first observation only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topology: Option<TopologyEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Case {
    pub direction: String,
    pub requested_name: String,
    pub held_client_initialized: bool,
    pub held_client_released: bool,
    pub replacement_client_initialized: bool,
    pub replacement_client_released: bool,
    pub baseline: Option<RuntimeEvidence>,
    pub before: Option<NativeName>,
    pub held: Option<RuntimeEvidence>,
    pub while_held: Option<NativeName>,
    pub old_removed: Option<RetirementEvidence>,
    pub replacement: Option<RuntimeEvidence>,
    pub after: Option<NativeName>,
    pub held_completion_refused: bool,
    pub fresh_replacement_confirmed: bool,
    pub windows_name_verified: bool,
    pub direct_removed_verified: bool,
    /// The bounded diagnostic sequence reached its end, not an acceptance pass.
    pub diagnostic_completed: bool,
    pub qualified: bool,
    pub status: &'static str,
    pub progress: Vec<DiagnosticProgress>,
    pub acceptance_failures: Vec<AcceptanceFailure>,
    pub completed: bool,
    pub error: Option<String>,
    pub cleanup_errors: Vec<String>,
}

impl Case {
    pub fn new(direction: String, requested_name: String) -> Self {
        Self {
            direction,
            requested_name,
            held_client_initialized: false,
            held_client_released: false,
            replacement_client_initialized: false,
            replacement_client_released: false,
            baseline: None,
            before: None,
            held: None,
            while_held: None,
            old_removed: None,
            replacement: None,
            after: None,
            held_completion_refused: false,
            fresh_replacement_confirmed: false,
            windows_name_verified: false,
            direct_removed_verified: false,
            diagnostic_completed: false,
            qualified: false,
            status: "unqualified",
            progress: Vec::new(),
            acceptance_failures: Vec::new(),
            completed: false,
            error: None,
            cleanup_errors: Vec::new(),
        }
    }

    pub fn finish(&mut self) {
        if !self.direct_removed_verified || self.old_removed.is_none() {
            self.fail(
                Phase::DirectRemoved,
                "missing validated direct old-Removed evidence before fresh replacement".into(),
            );
        }
        let lifecycle_complete = self.held_client_initialized
            && self.held_client_released
            && self.replacement_client_initialized
            && self.replacement_client_released
            && self.held_completion_refused
            && self.fresh_replacement_confirmed
            && self.windows_name_verified
            && self.direct_removed_verified
            && self.old_removed.is_some()
            && self.diagnostic_completed;
        if self.error.is_none() {
            if !self.cleanup_errors.is_empty() {
                self.error = Some("native cleanup failed; see cleanup_errors".into());
            } else if !lifecycle_complete {
                self.error = Some("native rename lifecycle/evidence incomplete".into());
            }
        }
        self.completed = self.error.is_none()
            && self.acceptance_failures.is_empty()
            && self.cleanup_errors.is_empty()
            && lifecycle_complete;
        self.qualified = self.completed;
        self.status = if self.qualified {
            "passed"
        } else {
            "unqualified"
        };
    }

    fn progress(&mut self, phase: Phase, status: &'static str) {
        if let Some(entry) = self.progress.iter_mut().find(|entry| entry.phase == phase) {
            // A later retry/success cannot erase a previously failed checkpoint.
            if entry.status != "failed" {
                entry.status = status;
            }
        } else {
            self.progress.push(DiagnosticProgress { phase, status });
        }
    }

    fn fail(&mut self, phase: Phase, error: String) {
        self.progress(phase, "failed");
        self.error.get_or_insert_with(|| error.clone());
        // One checkpoint/failure per finite enum variant bounds report growth.
        if !self
            .acceptance_failures
            .iter()
            .any(|entry| entry.phase == phase)
        {
            self.acceptance_failures
                .push(AcceptanceFailure { phase, error });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Baseline,
    StrictHeld,
    HeldRelease,
    DirectRemoved,
    ReplacementPresent,
    ReplacementNativeInit,
    ExactWindowsName,
    EngineConfirmation,
    DiagnosticContinuation,
    ReplacementRelease,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DiagnosticProgress {
    pub phase: Phase,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AcceptanceFailure {
    pub phase: Phase,
    pub error: String,
}

fn step<T, E: FnMut(&Case)>(
    case: &mut Case,
    phase: Phase,
    emit: &mut E,
    action: impl FnOnce(&mut Case, &mut E) -> anyhow::Result<T>,
) -> Option<T> {
    case.progress(phase, "running");
    emit(case);
    let result = match action(case, emit) {
        Ok(value) => {
            case.progress(phase, "passed");
            Some(value)
        }
        Err(error) => {
            case.fail(phase, format!("{error:#}"));
            None
        }
    };
    emit(case);
    result
}

fn continue_after_held<E: FnMut(&Case)>(
    case: &mut Case,
    emit: &mut E,
    release: impl FnOnce(&mut Case, &mut E) -> anyhow::Result<()>,
    continuation: impl FnOnce(&mut Case, &mut E) -> anyhow::Result<()>,
) {
    step(case, Phase::HeldRelease, emit, release);
    // release consumes the native owner even if Stop fails. Continue only when
    // the owner was actually dropped, never merely because a DTO says Absent.
    if case.held_client_released {
        case.diagnostic_completed =
            step(case, Phase::DiagnosticContinuation, emit, continuation).is_some();
        emit(case);
    }
}

fn shared_deadline(now: Instant, deadline: Instant, shares: u32) -> Instant {
    // An expired parent stays expired. Splitting the remaining time, rather
    // than restarting a fixed phase timeout, leaves later diagnostics a turn.
    deadline.min(now + deadline.saturating_duration_since(now) / shares)
}

fn retirement_deadline(now: Instant, deadline: Instant) -> Instant {
    // This independent acceptance gate must not consume native/name readback
    // time when evidence is missing. Failure remains sticky after continuation.
    shared_deadline(now, deadline, 4).min(now + Duration::from_millis(500))
}

fn direct_removed(
    baseline: &RuntimeEvidence,
    retirement: Option<&RetirementEvidence>,
) -> anyhow::Result<()> {
    let retirement = retirement.context("missing direct old-Removed native QUERY evidence")?;
    let old = baseline
        .native_stamp
        .context("missing baseline native QUERY stamp")?;
    let generation = baseline
        .observation_generation
        .context("missing baseline observation generation")?;
    ensure!(
        old[..3].iter().all(|value| *value > 0) && retirement.native_stamp[..3] == old[..3],
        "direct old-Removed identity does not match baseline PDO/session/endpoint"
    );
    ensure!(
        retirement.generation > generation,
        "stale direct old-Removed generation"
    );
    ensure!(
        retirement.native_stamp[3] >= old[3] && retirement.native_stamp[4] >= old[4],
        "direct old-Removed stream/gate epoch regressed"
    );
    // The production adapter guarantees fresh QUERY Removed and a closed gate.
    // A historical gate epoch, Absent DTO or replacement token proves neither.
    Ok(())
}

fn held(
    baseline: &RuntimeEvidence,
    current: &RuntimeEvidence,
    requested: &str,
    request: u64,
) -> anyhow::Result<()> {
    let (old, new) = advance(baseline, current, requested, request)?;
    ensure!(
        old[..3] == new[..3] && new[3] >= old[3] && new[4] >= old[4],
        "native identity changed or generation regressed while client was held"
    );
    ensure!(
        current.resource == "removing"
            && current.rename_pending
            && current.provider_name == baseline.provider_name
            && !current.gate_open
            && current.phase == "idle"
            && current.active_revision.is_none()
            && current.export_active_revision.is_none()
            && current.pending_revision.is_none()
            && current.service_error.is_none(),
        "production service did not explicitly keep recreation pending with the old name and closed gate"
    );
    Ok(())
}

fn same_held_device(before: &NativeName, held: &NativeName) -> anyhow::Result<()> {
    // Optional metadata (including an unavailable property) is diagnostic only.
    // Keep precisely the original identity/FriendlyName acceptance surface.
    ensure!(
        before.root_instance == held.root_instance
            && before.stable_id == held.stable_id
            && before.direction == held.direction
            && before.mmdevice_id == held.mmdevice_id
            && before.adapter_instance == held.adapter_instance
            && before.windows_friendly_name == held.windows_friendly_name,
        "held MMDevice identity or Windows FriendlyName drifted"
    );
    Ok(())
}

fn replacement(
    baseline: &RuntimeEvidence,
    current: &RuntimeEvidence,
    requested: &str,
    request: u64,
) -> anyhow::Result<()> {
    let (old, new) = advance(baseline, current, requested, request)?;
    ensure!(
        new[0] != old[0] && new[1] == old[1] && new[2] > old[2] && new[3] > 0 && new[4] > 0,
        "replacement lacks a new PDO token and newer endpoint generation in the same control session"
    );
    ensure!(
        current.resource == "present"
            && !current.rename_pending
            && current.provider_name.as_deref() == Some(requested)
            && current.gate_open
            && current.phase == "running"
            && current.active_revision == Some(current.candidate_revision)
            && current.export_active_revision == current.active_revision
            && current.pending_revision.is_none()
            && current.service_error.is_none()
            && current.graph_error.is_none()
            && current.provider_error.is_none(),
        "replacement lacks clean Engine confirmation and native admission"
    );
    Ok(())
}

fn windows_name(before: &NativeName, after: &NativeName, requested: &str) -> anyhow::Result<()> {
    ensure!(
        before.root_instance == after.root_instance
            && before.stable_id == after.stable_id
            && before.direction == after.direction,
        "replacement MMDevice does not belong to the same root/stable ID/direction"
    );
    ensure!(
        after.windows_friendly_name == requested,
        "Windows FriendlyName mismatch: requested={requested:?}, observed={:?}",
        after.windows_friendly_name
    );
    Ok(())
}

fn advance(
    baseline: &RuntimeEvidence,
    current: &RuntimeEvidence,
    requested: &str,
    request: u64,
) -> anyhow::Result<([u64; 5], [u64; 5])> {
    ensure!(
        baseline.resource == "present"
            && !baseline.rename_pending
            && baseline.provider_name.as_deref() == Some(baseline.requested_name.as_str())
            && !baseline.gate_open
            && baseline.active_revision.is_none()
            && baseline.export_active_revision.is_none()
            && baseline.pending_revision.is_none()
            && baseline.phase == "idle"
            && baseline.intent.enabled
            && baseline.intent.binding.is_some()
            && baseline.intent.channels == 2
            && !baseline.allow_default
            && baseline.service_error.is_none()
            && baseline.graph_error.is_none()
            && baseline.provider_error.is_none(),
        "invalid stopped baseline"
    );
    ensure!(
        current.intent == baseline.intent
            && !current.allow_default
            && current.requested_name == requested
            && requested != baseline.requested_name
            && current.candidate_revision > baseline.candidate_revision
            && current.consumed_request_id >= request,
        "rename intent/binding changed or request/revision was not consumed"
    );
    let old_generation = baseline
        .observation_generation
        .context("missing baseline observation generation")?;
    let new_generation = current
        .observation_generation
        .context("missing current observation generation")?;
    ensure!(new_generation > old_generation, "stale service observation");
    let old = baseline
        .native_stamp
        .context("missing fresh baseline native QUERY stamp")?;
    let new = current.native_stamp.with_context(|| {
        if current.resource == "absent" {
            "missing fresh current native QUERY stamp; old provider resource is no longer held"
        } else {
            "missing fresh current native QUERY stamp"
        }
    })?;
    ensure!(
        old[..3].iter().all(|value| *value > 0) && new[..3].iter().all(|value| *value > 0),
        "invalid zero native identity"
    );
    Ok((old, new))
}

#[cfg(test)]
#[path = "rename/tests.rs"]
mod tests;
