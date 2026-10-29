use super::{state::MeasuredDraft, window_state::WindowEditor};
use crate::service::{AppSnapshot, ProcessTraceSessionEvidence, SessionCleanupObservation};
use std::sync::Arc;

fn fixture() -> (AppSnapshot, MeasuredDraft, Arc<ProcessTraceSessionEvidence>) {
    let (mut snapshot, draft) = super::tests::fixture();
    let evidence = Arc::new(crate::service::process_trace::tests::evidence(
        draft.node.unwrap(),
    ));
    snapshot.last_cleanup = Some(SessionCleanupObservation {
        generation: evidence.origin().generation,
        state: cleanup_state(),
        process_trace: Some(evidence.clone()),
    });
    (snapshot, draft, evidence)
}

pub(in crate::ui::window::settings) fn cleanup_state() -> moiren_engine::host::HostSnapshot {
    use moiren_engine::host::*;
    HostSnapshot {
        host: HostState::Closed,
        execution: ExecutionState::Dormant,
        desired_revision: 5,
        active_revision: None,
        plan_revision: None,
        owner_generation: 7,
        resource_generation: 1,
        timeline: None,
        update: UpdateState::Idle,
        fault: None,
        cleanup_error: None,
    }
}

#[test]
fn process_window_editor_single_worker_and_exact_result_fences() {
    let (mut snapshot, draft, evidence) = fixture();
    let mut editor = WindowEditor::default();
    editor.policy = crate::service::process_trace::tests::policy();
    let job = editor.begin(&snapshot, draft).unwrap();
    assert!(editor.begin(&snapshot, draft).is_err());
    editor.changed();
    assert!(
        editor.busy(),
        "editing must not allow a second worker before completion"
    );
    let result = evidence.evaluate_candidate(
        draft.capacity_frames,
        draft.target_frames,
        draft.jitter_frames,
        editor.policy,
    );
    let changed = MeasuredDraft {
        target_frames: 512,
        ..draft
    };
    editor.finish(&job, &snapshot, changed, result);
    assert!(!editor.busy());
    assert!(editor.result(&snapshot, changed).is_none());
    assert!(editor.error.is_some());

    let job = editor.begin(&snapshot, draft).unwrap();
    let result = job.evaluate();
    editor.finish(&job, &snapshot, draft, result);
    assert!(editor.result(&snapshot, draft).is_some());
    assert!(editor.result(&snapshot, changed).is_none());
    editor.policy.min_output_reads += 1;
    assert!(editor.result(&snapshot, draft).is_none());
    editor.policy.min_output_reads -= 1;
    // An equal-looking predecessor is not the same retained session object.
    snapshot.last_cleanup.as_mut().unwrap().process_trace = Some(Arc::new(
        crate::service::process_trace::tests::evidence(draft.node.unwrap()),
    ));
    assert!(editor.result(&snapshot, draft).is_none());
}

#[test]
fn process_window_editor_policy_and_predecessor_change_refuse_late_completion() {
    for change_policy in [false, true] {
        let (mut snapshot, draft, _) = fixture();
        let mut editor = WindowEditor::default();
        let job = editor.begin(&snapshot, draft).unwrap();
        let result = job.evaluate();
        if change_policy {
            editor.policy.min_output_frames += 1;
        } else {
            snapshot.last_cleanup = None;
        }
        editor.finish(&job, &snapshot, draft, result);
        assert!(editor.result(&snapshot, draft).is_none());
        assert!(editor.error.is_some());
    }
    let (mut snapshot, draft, _) = fixture();
    snapshot.last_cleanup = None;
    assert!(WindowEditor::default().begin(&snapshot, draft).is_err());
}

#[test]
fn process_window_editor_invalidated_same_values_and_old_job_cannot_complete_new_worker() {
    let (snapshot, draft, _) = fixture();
    let mut editor = WindowEditor::default();
    let old = editor.begin(&snapshot, draft).unwrap();
    editor.changed();
    editor.finish(&old, &snapshot, draft, old.evaluate());
    assert!(editor.result(&snapshot, draft).is_none());
    let new = editor.begin(&snapshot, draft).unwrap();
    editor.finish(&old, &snapshot, draft, old.evaluate());
    assert!(editor.busy());
    editor.finish(&new, &snapshot, draft, new.evaluate());
    assert!(!editor.busy());
    assert!(editor.result(&snapshot, draft).is_some());
}

#[test]
fn process_window_candidate_failure_does_not_follow_a_replaced_predecessor() {
    let (mut snapshot, draft, _) = fixture();
    let mut editor = WindowEditor::default();
    let job = editor.begin(&snapshot, draft).unwrap();
    editor.finish(
        &job,
        &snapshot,
        draft,
        Err("original candidate preparation refused".into()),
    );
    assert_eq!(
        editor.feedback_error(&snapshot, draft),
        Some("original candidate preparation refused")
    );
    snapshot.last_cleanup.as_mut().unwrap().process_trace = Some(Arc::new(
        crate::service::process_trace::tests::evidence(draft.node.unwrap()),
    ));
    assert!(editor.feedback_error(&snapshot, draft).is_none());
    assert!(!editor.busy());
}

#[test]
fn retained_apply_candidate_is_fenced_separately_from_diagnostic_success() {
    let (snapshot, draft, evidence) = fixture();
    let mut editor = WindowEditor::default();
    let job = editor.begin(&snapshot, draft).unwrap();
    let diagnostic = job.evaluate();
    editor.finish(&job, &snapshot, draft, diagnostic);
    editor.finish_admission(
        &job,
        &snapshot,
        draft,
        Err("missing final native whole wake".into()),
    );
    assert!(editor.result(&snapshot, draft).is_some());
    assert!(editor.candidate(&snapshot, draft).is_none());
    assert!(
        editor
            .admission_feedback(&snapshot, draft)
            .unwrap()
            .contains("whole wake")
    );
    assert_eq!(evidence.origin().node, draft.node.unwrap());
}

#[test]
fn retained_apply_successful_candidate_cannot_follow_draft_policy_or_predecessor_changes() {
    for change in 0..4 {
        let (mut snapshot, mut draft, original) = fixture();
        // Preserve the exact object installed below; software tokens are only
        // control fixtures and their replay origin remains CallerSupplied.
        let node = original.origin().node;
        let mut evidence = crate::service::process_trace::tests::evidence(node);
        let observation = crate::service::measured_process::tests::observation(node);
        evidence.measurement = observation
            .measured(&observation.process, 7)
            .map_err(|e| e.to_string());
        let owned = Arc::new(evidence);
        snapshot.last_cleanup.as_mut().unwrap().process_trace = Some(owned.clone());
        let token = crate::service::process_trace::candidate::software_candidate(owned);
        let mut editor = WindowEditor::default();
        let job = editor.begin(&snapshot, draft).unwrap();
        editor.finish(&job, &snapshot, draft, job.evaluate());
        editor.finish_admission(&job, &snapshot, draft, Ok(token));
        assert!(editor.candidate(&snapshot, draft).is_some());
        match change {
            0 => draft.target_frames += 1,
            1 => editor.policy.min_output_frames += 1,
            2 => {
                snapshot.last_cleanup.as_mut().unwrap().process_trace = Some(Arc::new(
                    crate::service::process_trace::tests::evidence(node),
                ))
            }
            _ => editor.changed(),
        }
        assert!(
            editor.candidate(&snapshot, draft).is_none(),
            "change {change}"
        );
    }
}
