use super::state::{MeasuredDraft, MeasuredEditor};
use crate::service::SessionPhase;
use crate::service::{InputSelection, NativeSessionObservations, SessionGeneration};
use moiren_core::graph::NodeKind;
use moiren_windows_audio::engine_backend::NativeObservationSnapshot;

pub(super) fn fixture() -> (crate::service::AppSnapshot, MeasuredDraft) {
    let mut context = crate::service::ServiceContext::default();
    let spec = crate::service::SessionSpec {
        source: InputSelection::Process {
            pid: 42,
            creation_time_100ns: 123,
        },
        output_endpoint_id: "fixture-output".into(),
        limit: crate::service::RunLimit::UntilStopped,
        gain: 1.0,
        pan: 0.0,
        max_block_frames: 256,
    };
    context.graph.configure_session(&spec).unwrap();
    let generation = context.core.start(spec).unwrap();
    context.core.activated(generation);
    context.core.owner_started(generation);
    let revision = context.graph.snapshot().candidate_revision;
    context.graph.preparing(generation, revision);
    context.graph.running(generation, revision);
    context.active_audio_settings = Some(context.audio_settings.clone());
    context.active_transport_latency = Some(context.transport_latency);
    context.active_latency_profile = Some(context.latency_profile);
    let node = context
        .graph
        .snapshot()
        .node_of_kind(NodeKind::Source)
        .unwrap()
        .0
        .id();
    context.native_observations = Some(NativeSessionObservations {
        generation,
        observation: NativeObservationSnapshot {
            owner_generation: 7,
            running: true,
            streams: vec![],
            bridges: vec![],
            boundary_probes: vec![],
            render_padding: vec![],
            process_delivery: vec![crate::service::measured_process::tests::observation(node)],
        },
    });
    (
        crate::service::snapshot(&context),
        MeasuredDraft {
            node: Some(node),
            capacity_frames: 2048,
            target_frames: 256,
            jitter_frames: 0,
        },
    )
}

#[test]
fn retained_apply_confirmation_waits_in_idle_after_consumption_but_never_confirms_without_new_owner()
 {
    let (mut snapshot, draft) = fixture();
    let request = draft.request(&snapshot).unwrap();
    let selected = crate::service::measured_process::MeasuredProcessProfile::from_observation(
        1,
        request,
        &snapshot
            .native_observations
            .as_ref()
            .unwrap()
            .observation
            .process_delivery[0],
    )
    .unwrap()
    .summary()
    .clone();
    snapshot.phase = SessionPhase::Idle;
    snapshot.native_observations = None;
    snapshot.active_measured_process = None;
    let mut editor = MeasuredEditor::default();
    editor.begin_retained(crate::service::RequestId(8), request, &snapshot);
    snapshot.consumed_request_id = crate::service::RequestId(8);
    snapshot.measured_process = Some(selected);
    editor.observe(&snapshot, None);
    assert!(
        editor.busy(),
        "accepted stopped Apply may still wait for resources"
    );
    snapshot.graph = std::sync::Arc::new(crate::service::GraphSnapshot {
        last_error: Some("late candidate refused".into()),
        ..(*snapshot.graph).clone()
    });
    editor.observe(&snapshot, None);
    assert!(!editor.busy());
    assert_eq!(editor.error.as_deref(), Some("late candidate refused"));
}

#[test]
fn desktop_measured_preflight_uses_only_matching_live_origin_and_three_independent_values() {
    let (snapshot, draft) = fixture();
    let request = draft.request(&snapshot).unwrap();
    assert_eq!(request.node, draft.node.unwrap());
    assert_eq!(request.session_generation, snapshot.generation);
    assert_eq!(request.native_owner_generation, 7);
    assert_eq!(request.graph_revision, snapshot.graph.candidate_revision);
    assert_eq!(
        (
            request.capacity_frames,
            request.target_frames,
            request.jitter_frames
        ),
        (2048, 256, 0)
    );
    assert!(snapshot.transport_latency.process.is_none());
    for fault in 0..8 {
        let mut changed = snapshot.clone();
        match fault {
            0 => changed.phase = SessionPhase::Idle,
            1 => changed.native_observations = None,
            2 => changed.native_observations.as_mut().unwrap().generation = SessionGeneration(0),
            3 => {
                changed
                    .native_observations
                    .as_mut()
                    .unwrap()
                    .observation
                    .running = false
            }
            4 => {
                changed
                    .native_observations
                    .as_mut()
                    .unwrap()
                    .observation
                    .process_delivery[0]
                    .process
                    .creation_time_100ns += 1
            }
            5 => std::sync::Arc::make_mut(&mut changed.graph).pending_revision = Some(99),
            6 => changed.active_audio_settings = None,
            7 => {
                changed
                    .native_observations
                    .as_mut()
                    .unwrap()
                    .observation
                    .process_delivery[0]
                    .consumer = None
            }
            _ => unreachable!(),
        }
        assert!(draft.request(&changed).is_err(), "fault={fault}");
    }
    assert!(
        MeasuredDraft {
            target_frames: 128,
            ..draft
        }
        .request(&snapshot)
        .unwrap_err()
        .contains("target")
    );
}

#[test]
fn desktop_measured_confirmation_requires_exact_new_selection_and_owner_not_only_consumption() {
    let (mut snapshot, draft) = fixture();
    let request = draft.request(&snapshot).unwrap();
    let mut editor = MeasuredEditor::default();
    editor.draft = draft;
    editor.begin(crate::service::RequestId(5), request, &snapshot);
    assert!(editor.busy());
    snapshot.consumed_request_id = crate::service::RequestId(5);
    let observation = &snapshot
        .native_observations
        .as_ref()
        .unwrap()
        .observation
        .process_delivery[0];
    let summary = crate::service::measured_process::MeasuredProcessProfile::from_observation(
        1,
        request,
        observation,
    )
    .unwrap()
    .summary()
    .clone();
    snapshot.measured_process = Some(summary.clone());
    editor.observe(&snapshot, None);
    assert!(
        editor.busy(),
        "consumption and desired profile are not activation"
    );
    snapshot.generation.0 += 1;
    snapshot.native_observations.as_mut().unwrap().generation = snapshot.generation;
    snapshot.active_measured_process = Some(crate::service::MeasuredProcessSummary {
        selection_id: 2,
        ..summary.clone()
    });
    editor.observe(&snapshot, None);
    assert!(!editor.busy());
    assert!(
        editor.error.is_some(),
        "another selection cannot confirm this transaction"
    );

    let (mut snapshot, draft) = fixture();
    let mut editor = MeasuredEditor::default();
    editor.draft = draft;
    editor.begin(crate::service::RequestId(5), request, &snapshot);
    snapshot.consumed_request_id = crate::service::RequestId(5);
    snapshot.measured_process = Some(summary.clone());
    snapshot.active_measured_process = Some(summary);
    editor.observe(&snapshot, None);
    assert!(editor.busy(), "same originating owner is not a replacement");
    snapshot.generation.0 += 1;
    snapshot.native_observations.as_mut().unwrap().generation = snapshot.generation;
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .owner_generation += 1;
    editor.observe(&snapshot, None);
    assert!(!editor.busy());
    assert!(editor.error.is_none());
    assert!(editor.message.as_deref().unwrap().contains("active"));
}

#[test]
fn desktop_measured_refusal_and_stopped_owner_release_busy_without_erasing_draft() {
    for stopped in [false, true] {
        let (mut snapshot, draft) = fixture();
        let request = draft.request(&snapshot).unwrap();
        let mut editor = MeasuredEditor::default();
        editor.draft = draft;
        editor.begin(crate::service::RequestId(9), request, &snapshot);
        snapshot.consumed_request_id = crate::service::RequestId(9);
        if stopped {
            snapshot.phase = SessionPhase::Idle;
        } else {
            std::sync::Arc::make_mut(&mut snapshot.graph).last_error =
                Some("injected measured refusal".into());
        }
        editor.observe(&snapshot, None);
        assert!(!editor.busy());
        assert!(editor.error.is_some());
        assert_eq!(editor.draft, draft);
        assert!(snapshot.active_measured_process.is_none());
    }
}

#[test]
fn desktop_measured_old_error_does_not_hide_success_and_success_expires_with_active_owner() {
    let (mut snapshot, draft) = fixture();
    std::sync::Arc::make_mut(&mut snapshot.graph).last_error =
        Some("previous unrelated refusal".into());
    let request = draft.request(&snapshot).unwrap();
    let summary = crate::service::measured_process::MeasuredProcessProfile::from_observation(
        1,
        request,
        &snapshot
            .native_observations
            .as_ref()
            .unwrap()
            .observation
            .process_delivery[0],
    )
    .unwrap()
    .summary()
    .clone();
    let mut editor = MeasuredEditor::default();
    editor.begin(crate::service::RequestId(4), request, &snapshot);
    snapshot.consumed_request_id = crate::service::RequestId(4);
    snapshot.measured_process = Some(summary.clone());
    editor.observe(&snapshot, None);
    assert!(
        editor.busy(),
        "an unchanged previous error is not this request's refusal"
    );
    snapshot.generation.0 += 1;
    snapshot.native_observations.as_mut().unwrap().generation = snapshot.generation;
    snapshot.active_measured_process = Some(summary);
    std::sync::Arc::make_mut(&mut snapshot.graph).last_error = None;
    editor.observe(&snapshot, None);
    assert!(!editor.busy());
    assert!(editor.error.is_none());
    assert!(editor.message.is_some());
    snapshot.phase = SessionPhase::Idle;
    snapshot.active_measured_process = None;
    editor.observe(&snapshot, None);
    assert!(
        editor.message.is_none(),
        "a stopped owner is not still active"
    );
}

#[test]
fn desktop_measured_old_deadline_cannot_expire_a_newer_captured_request() {
    let (snapshot, draft) = fixture();
    let request = draft.request(&snapshot).unwrap();
    let mut editor = MeasuredEditor::default();
    editor.begin(crate::service::RequestId(4), request, &snapshot);
    assert!(editor.expire(crate::service::RequestId(4)));
    assert!(!editor.busy());
    editor.begin(crate::service::RequestId(5), request, &snapshot);
    assert!(!editor.expire(crate::service::RequestId(4)));
    assert!(editor.busy());
    assert!(editor.error.is_none());
}
