//! Independent service, native lifecycle and Windows property evidence.
use anyhow::{Context, ensure};
use serde::Serialize;

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

#[derive(Debug, Clone, Serialize)]
pub(super) struct RuntimeEvidence {
    pub intent: IntentEvidence,
    pub requested_name: String,
    pub provider_name: Option<String>,
    pub rename_pending: bool,
    pub resource: String,
    pub observation_generation: Option<u64>,
    /// Fresh provider QUERY [pdo, session, endpoint, stream, gate].
    pub native_stamp: Option<[u64; 5]>,
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
pub(super) struct NativeName {
    pub root_instance: String,
    pub stable_id: String,
    pub direction: String,
    pub mmdevice_id: String,
    pub adapter_instance: String,
    /// Exact PKEY_Device_FriendlyName; never the provider's spec_name.
    pub windows_friendly_name: String,
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
    pub replacement: Option<RuntimeEvidence>,
    pub after: Option<NativeName>,
    pub held_completion_refused: bool,
    pub fresh_replacement_confirmed: bool,
    pub windows_name_verified: bool,
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
            replacement: None,
            after: None,
            held_completion_refused: false,
            fresh_replacement_confirmed: false,
            windows_name_verified: false,
            completed: false,
            error: None,
            cleanup_errors: Vec::new(),
        }
    }

    pub fn finish(&mut self) {
        let lifecycle_complete = self.held_client_initialized
            && self.held_client_released
            && self.replacement_client_initialized
            && self.replacement_client_released
            && self.held_completion_refused
            && self.fresh_replacement_confirmed
            && self.windows_name_verified;
        if self.error.is_none() {
            if !self.cleanup_errors.is_empty() {
                self.error = Some("native cleanup failed; see cleanup_errors".into());
            } else if !lifecycle_complete {
                self.error = Some("native rename lifecycle/evidence incomplete".into());
            }
        }
        self.completed =
            self.error.is_none() && self.cleanup_errors.is_empty() && lifecycle_complete;
    }
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
    ensure!(
        before == held,
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
    let new = current
        .native_stamp
        .context("missing fresh current native QUERY stamp")?;
    ensure!(
        old[..3].iter().all(|value| *value > 0) && new[..3].iter().all(|value| *value > 0),
        "invalid zero native identity"
    );
    Ok((old, new))
}

#[cfg(test)]
#[path = "rename/tests.rs"]
mod tests;
