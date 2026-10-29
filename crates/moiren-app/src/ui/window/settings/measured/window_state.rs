//! One diagnostic worker, fenced independently of live measured Apply.
use super::state::MeasuredDraft;
use crate::service::{AppSnapshot, EvaluatedProcessCandidate, ProcessTraceSessionEvidence};
use moiren_windows_audio::process_loopback::{AssessedProcessReplay, ReplayWindowPolicy};
use rust_i18n::t;
use std::sync::Arc;

pub(super) struct WindowJob {
    pub evidence: Arc<ProcessTraceSessionEvidence>,
    pub draft: MeasuredDraft,
    pub policy: ReplayWindowPolicy,
}

impl WindowJob {
    pub fn admit(&self) -> Result<EvaluatedProcessCandidate, String> {
        self.evidence.clone().evaluate_for_apply(
            self.draft.capacity_frames,
            self.draft.target_frames,
            self.draft.jitter_frames,
            self.policy,
        )
    }
    pub fn evaluate(&self) -> Result<AssessedProcessReplay, String> {
        self.evidence.evaluate_candidate(
            self.draft.capacity_frames,
            self.draft.target_frames,
            self.draft.jitter_frames,
            self.policy,
        )
    }

    fn matches(
        &self,
        snapshot: &AppSnapshot,
        draft: MeasuredDraft,
        policy: ReplayWindowPolicy,
    ) -> bool {
        self.draft == draft
            && self.policy == policy
            && retained(snapshot).is_some_and(|current| Arc::ptr_eq(current, &self.evidence))
    }
}

pub(super) fn retained(snapshot: &AppSnapshot) -> Option<&Arc<ProcessTraceSessionEvidence>> {
    snapshot.last_cleanup.as_ref()?.process_trace.as_ref()
}

pub(super) struct WindowEditor {
    pub policy: ReplayWindowPolicy,
    pub error: Option<String>,
    pending: Option<(Arc<WindowJob>, bool)>,
    completed: Option<(Arc<WindowJob>, AssessedProcessReplay)>,
    failed: Option<(Arc<WindowJob>, String)>,
    admission: Option<(Arc<WindowJob>, Result<EvaluatedProcessCandidate, String>)>,
}

impl Default for WindowEditor {
    fn default() -> Self {
        Self {
            // Explicit diagnostic policy, never a production or VM grace setting.
            policy: ReplayWindowPolicy {
                max_initial_unserved_frames: 4096,
                max_initial_prime_discarded_frames: 4096,
                min_output_frames: 480,
                min_output_reads: 2,
                min_output_span_100ns: 100_000,
            },
            error: None,
            pending: None,
            completed: None,
            failed: None,
            admission: None,
        }
    }
}

impl WindowEditor {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn changed(&mut self) {
        if let Some((_, invalidated)) = &mut self.pending {
            *invalidated = true;
        }
        self.error = None;
        self.completed = None;
        self.failed = None;
        self.admission = None;
    }

    pub fn available(&self, snapshot: &AppSnapshot, draft: MeasuredDraft) -> Result<(), String> {
        self.policy.validate().map_err(|error| error.to_string())?;
        let evidence = retained(snapshot).ok_or_else(|| {
            t!("Stop a collected Process session to evaluate its closed prefix.").to_string()
        })?;
        if draft
            .node
            .is_some_and(|node| node != evidence.origin().node)
        {
            return Err(t!("The selected source is not the retained Process source.").into());
        }
        evidence.ordered_trace()?;
        Ok(())
    }

    pub fn begin(
        &mut self,
        snapshot: &AppSnapshot,
        draft: MeasuredDraft,
    ) -> Result<Arc<WindowJob>, String> {
        if self.busy() {
            return Err(t!("A finite-window evaluation is already running.").into());
        }
        self.available(snapshot, draft)?;
        let job = Arc::new(WindowJob {
            evidence: retained(snapshot).unwrap().clone(),
            draft,
            policy: self.policy,
        });
        self.error = None;
        self.completed = None;
        self.failed = None;
        self.admission = None;
        self.pending = Some((job.clone(), false));
        Ok(job)
    }

    pub fn finish(
        &mut self,
        job: &Arc<WindowJob>,
        snapshot: &AppSnapshot,
        draft: MeasuredDraft,
        result: Result<AssessedProcessReplay, String>,
    ) {
        let Some((pending, invalidated)) = &self.pending else {
            return;
        };
        if !Arc::ptr_eq(pending, job) {
            return;
        }
        let stale = *invalidated || !job.matches(snapshot, draft, self.policy);
        self.pending = None;
        if stale {
            self.error =
                Some(t!("Evaluation discarded: predecessor, candidate or policy changed.").into());
            return;
        }
        match result {
            Ok(result) => self.completed = Some((job.clone(), result)),
            Err(error) => self.failed = Some((job.clone(), error)),
        }
    }

    pub fn feedback_error(&self, snapshot: &AppSnapshot, draft: MeasuredDraft) -> Option<&str> {
        self.failed
            .as_ref()
            .filter(|(job, _)| job.matches(snapshot, draft, self.policy))
            .map(|(_, error)| error.as_str())
            .or(self.error.as_deref())
    }

    pub fn finish_admission(
        &mut self,
        job: &Arc<WindowJob>,
        snapshot: &AppSnapshot,
        draft: MeasuredDraft,
        result: Result<EvaluatedProcessCandidate, String>,
    ) {
        // Must follow the same accepted completion, never a stale discarded job.
        if !job.matches(snapshot, draft, self.policy)
            || self
                .completed
                .as_ref()
                .is_none_or(|(completed, _)| !Arc::ptr_eq(completed, job))
        {
            return;
        }
        self.admission = Some((job.clone(), result));
    }

    pub fn candidate(
        &self,
        snapshot: &AppSnapshot,
        draft: MeasuredDraft,
    ) -> Option<&EvaluatedProcessCandidate> {
        self.admission
            .as_ref()
            .filter(|(job, _)| job.matches(snapshot, draft, self.policy))?
            .1
            .as_ref()
            .ok()
    }

    pub fn admission_feedback(&self, snapshot: &AppSnapshot, draft: MeasuredDraft) -> Option<&str> {
        self.admission
            .as_ref()
            .filter(|(job, _)| job.matches(snapshot, draft, self.policy))?
            .1
            .as_ref()
            .err()
            .map(String::as_str)
    }

    pub fn result(
        &self,
        snapshot: &AppSnapshot,
        draft: MeasuredDraft,
    ) -> Option<&AssessedProcessReplay> {
        self.completed
            .as_ref()
            .filter(|(job, _)| job.matches(snapshot, draft, self.policy))
            .map(|(_, result)| result)
    }
}
