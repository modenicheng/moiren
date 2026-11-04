//! Separate streaming evidence; never changes the historical held-client Case.
use super::{NativeName, RetirementEvidence, RuntimeEvidence};
use crate::validation::Observation;
use anyhow::{Context, ensure};
use serde::Serialize;

#[cfg(windows)]
#[path = "stream/target.rs"]
pub(crate) mod target;

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PcmPhase {
    pub initialized: bool,
    pub started: u32,
    pub released: bool,
    pub release_errors: Vec<String>,
    pub pair_mmdevice_id: Option<String>,
    pub gate_transitions: Vec<crate::validation::GateProgress>,
    pub name: Option<NativeName>,
    pub admission: Option<RuntimeEvidence>,
    pub confirmed: Option<RuntimeEvidence>,
    pub pcm: Observation,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Case {
    pub direction: String,
    pub requested_name: String,
    pub before: PcmPhase,
    pub after: PcmPhase,
    pub old_removed: Option<RetirementEvidence>,
    pub retirement_first: Option<RuntimeEvidence>,
    pub retirement_last: Option<RuntimeEvidence>,
    pub edit_request: Option<u64>,
    /// Actual samples/states after edit; not part of either qualified PCM window.
    pub interruption: Observation,
    pub interruption_first: Option<RuntimeEvidence>,
    pub interruption_runtime: Option<RuntimeEvidence>,
    pub native_interruptions: Vec<NativeFailure>,
    pub rename_while_started: bool,
    pub diagnostic_completed: bool,
    pub completed: bool,
    pub failures: Vec<String>,
}

impl Case {
    pub fn new(direction: String, requested_name: String) -> Self {
        Self {
            direction,
            requested_name,
            before: PcmPhase::default(),
            after: PcmPhase::default(),
            old_removed: None,
            retirement_first: None,
            retirement_last: None,
            edit_request: None,
            interruption: Observation::default(),
            interruption_first: None,
            interruption_runtime: None,
            native_interruptions: Vec::new(),
            rename_while_started: false,
            diagnostic_completed: false,
            completed: false,
            failures: Vec::new(),
        }
    }

    pub fn fail(&mut self, error: String) {
        self.failures.push(error);
    }

    fn retain_runtime_errors(&mut self, stage: &str, runtime: &RuntimeEvidence) {
        // A later Apply/poll may clear these fields. Keep observed failures
        // sticky without growing the report on identical snapshot polls.
        for (source, error) in [
            ("service", &runtime.service_error),
            ("graph", &runtime.graph_error),
            ("provider", &runtime.provider_error),
        ] {
            if let Some(error) = error {
                let failure = format!("{stage} {source} error: {error}");
                if !self.failures.contains(&failure) {
                    self.fail(failure);
                }
            }
        }
    }

    pub(super) fn observe_retirement(&mut self, runtime: RuntimeEvidence) -> anyhow::Result<()> {
        self.retain_runtime_errors("retirement", &runtime);
        self.retirement_first.get_or_insert_with(|| runtime.clone());
        self.retirement_last = Some(runtime.clone());
        let baseline = self
            .before
            .confirmed
            .as_ref()
            .context("missing old PCM confirmation")?;
        super::direct_removed(baseline, runtime.native_retirement.as_ref())?;
        self.old_removed = runtime.native_retirement;
        Ok(())
    }

    pub(super) fn observe_transition(&mut self, runtime: RuntimeEvidence, edit: u64) -> bool {
        self.retain_runtime_errors("transition", &runtime);
        self.interruption_first
            .get_or_insert_with(|| runtime.clone());
        let identity_changed = self
            .before
            .confirmed
            .as_ref()
            .and_then(|e| e.native_stamp)
            .zip(runtime.native_stamp)
            .is_some_and(|(old, new)| old[..3] != new[..3]);
        let transitioned = runtime.consumed_request_id >= edit
            && (!runtime.gate_open || runtime.resource != "present" || identity_changed);
        if transitioned {
            self.interruption_runtime = Some(runtime);
        }
        transitioned
    }

    pub fn finish(&mut self) {
        if let Err(error) = self.require_complete() {
            self.fail(format!("{error:#}"));
        }
        self.completed = self.failures.is_empty();
    }

    fn require_complete(&self) -> anyhow::Result<()> {
        ensure!(
            self.diagnostic_completed && self.rename_while_started,
            "streaming diagnostic sequence or edit while RUN missing"
        );
        self.before.require_complete()?;
        self.after.require_complete()?;
        let old = self
            .before
            .confirmed
            .as_ref()
            .context("missing old PCM confirmation")?;
        let new = self
            .after
            .confirmed
            .as_ref()
            .context("missing replacement PCM confirmation")?;
        super::direct_removed(old, self.old_removed.as_ref())?;
        replacement(
            old,
            new,
            &self.requested_name,
            self.edit_request.context("missing rename request")?,
        )?;
        super::windows_name(
            self.before.name.as_ref().context("missing before name")?,
            self.after
                .name
                .as_ref()
                .context("missing replacement name")?,
            &self.requested_name,
        )
    }
}

impl PcmPhase {
    pub(super) fn observe_pcm(
        &mut self,
        samples: &[f32],
        discontinuity: bool,
        second: bool,
        settling: bool,
        elapsed_ms: u128,
    ) -> anyhow::Result<()> {
        ensure!(
            self.started == 2,
            "PCM callback without both successful Starts"
        );
        if settling {
            ensure!(
                samples.iter().all(|v| v.is_finite()),
                "nonfinite PCM during fixed settling"
            );
            return Ok(());
        }
        self.pcm.first_frame_ms.get_or_insert(elapsed_ms);
        self.pcm.last_frame_ms = Some(elapsed_ms);
        let expected = if second {
            crate::validation::SECOND_EXPECTED
        } else {
            crate::validation::EXPECTED
        };
        self.pcm.observe_packet(samples, expected, discontinuity)
    }

    fn require_complete(&self) -> anyhow::Result<()> {
        ensure!(
            self.initialized
                && self.started == 2
                && self.released
                && self.release_errors.is_empty(),
            "PCM phase lacks both Starts or complete clean owner release"
        );
        self.pcm.require_signal()?;
        let admission = self.admission.as_ref().context("missing PCM admission")?;
        let confirmed = self
            .confirmed
            .as_ref()
            .context("missing fresh PCM confirmation")?;
        running(admission)?;
        running(confirmed)?;
        let old = admission
            .native_stamp
            .context("missing PCM admission identity")?;
        let new = confirmed
            .native_stamp
            .context("missing PCM confirmation identity")?;
        ensure!(
            old[..3] == new[..3]
                && new[3] >= old[3]
                && new[4] >= old[4]
                && admission.intent == confirmed.intent
                && admission.requested_name == confirmed.requested_name
                && admission.active_revision == confirmed.active_revision
                && confirmed.observation_generation >= admission.observation_generation,
            "PCM evidence crossed native identity, binding, revision or observation"
        );
        let name = self.name.as_ref().context("missing actual MMDevice name")?;
        ensure!(
            self.pair_mmdevice_id.as_deref() == Some(name.mmdevice_id.as_str())
                && name.stable_id == admission.intent.endpoint_id
                && name.direction == admission.intent.direction,
            "PCM pair does not match the observed MMDevice/project identity"
        );
        Ok(())
    }
}

pub(super) fn running(current: &RuntimeEvidence) -> anyhow::Result<()> {
    owner_running(current)?;
    ensure!(current.gate_open, "streaming admission gate is closed");
    Ok(())
}

pub(super) fn owner_running(current: &RuntimeEvidence) -> anyhow::Result<()> {
    ensure!(
        current.resource == "present"
            && !current.rename_pending
            && current.phase == "running"
            && current.active_revision == Some(current.candidate_revision)
            && current.export_active_revision == current.active_revision
            && current.pending_revision.is_none()
            && current.intent.enabled
            && current.intent.binding.is_some()
            && current.intent.channels == 2
            && !current.allow_default
            && current.provider_name.as_deref() == Some(current.requested_name.as_str())
            && current.service_error.is_none()
            && current.graph_error.is_none()
            && current.provider_error.is_none()
            && current.observation_generation.is_some()
            && current
                .native_stamp
                .is_some_and(|s| s.iter().all(|v| *v > 0)),
        "missing clean Engine-confirmed streaming admission"
    );
    Ok(())
}

pub(super) fn replacement(
    old: &RuntimeEvidence,
    new: &RuntimeEvidence,
    requested: &str,
    request: u64,
) -> anyhow::Result<()> {
    running(old)?;
    running(new)?;
    ensure!(
        old.intent == new.intent
            && old.requested_name != requested
            && new.requested_name == requested
            && new.candidate_revision > old.candidate_revision
            && new.consumed_request_id >= request
            && new.observation_generation > old.observation_generation,
        "replacement has stale request/revision/observation or changed project binding"
    );
    let previous = old.native_stamp.context("missing old native stamp")?;
    let current = new
        .native_stamp
        .context("missing replacement native stamp")?;
    ensure!(
        current[0] != previous[0] && current[1] == previous[1] && current[2] > previous[2],
        "replacement requires new PDO/newer endpoint in the same control session"
    );
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeFailure {
    pub stage: &'static str,
    pub hresult: Option<i32>,
    pub message: String,
    pub invalidated: bool,
}

pub(super) fn release_owner<T>(
    owner: &mut Option<T>,
    phase: &mut PcmPhase,
    stop: impl FnOnce(&mut T) -> Vec<String>,
) {
    if let Some(mut owner) = owner.take() {
        phase.release_errors.extend(stop(&mut owner));
        drop(owner);
        phase.released = true;
    }
}

#[cfg(test)]
#[path = "stream/tests.rs"]
mod tests;
