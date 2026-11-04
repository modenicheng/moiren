use super::*;
use std::time::Duration;

fn pcm_route(current: &RuntimeEvidence) -> crate::report::Evidence {
    use crate::report::{Evidence, ExportEvidence};
    use moiren_app::service::ExportDirection;

    Evidence {
        phase: current.phase.clone(),
        candidate_revision: current.candidate_revision,
        active_revision: current.active_revision,
        pending_revision: current.pending_revision,
        consumed_request_id: current.consumed_request_id,
        exports: [
            (
                "stable",
                ExportDirection::Playback,
                current.native_stamp,
                current.gate_open,
            ),
            (
                "sibling",
                ExportDirection::Microphone,
                Some([20, 7, 21, 3, 5]),
                true,
            ),
        ]
        .into_iter()
        .map(|(id, direction, native_stamp, gate_open)| ExportEvidence {
            endpoint_id: id.into(),
            direction,
            present: true,
            enabled: true,
            bound: true,
            channels: 2,
            allow_default: false,
            gate_open,
            active_revision: current.active_revision,
            native_stamp,
            windows_name: None,
            resource: "present".into(),
        })
        .collect(),
        error: current.service_error.clone(),
        graph_error: current.graph_error.clone(),
    }
}

fn pcm_end_phase() -> PcmPhase {
    let mut phase = phase(false);
    phase.pcm = Observation::default();
    phase
        .observe_pcm(
            &validation::EXPECTED.repeat(validation::REQUIRED_FRAMES as usize),
            false,
            false,
            false,
            1_000,
        )
        .unwrap();
    phase.gate_transitions = pcm_route(&runtime(false))
        .exports
        .iter()
        .map(|export| {
            validation::GateProgress::new(export.endpoint_id.clone(), export.native_stamp, 0)
                .unwrap()
        })
        .collect();
    phase
}

#[test]
fn pcm_end_waits_for_a_nonzero_stream_after_pre_client_admission() {
    let ids = ["stable".into(), "sibling".into()];
    let mut phase = pcm_end_phase();
    let mut idle = runtime(false);
    idle.native_stamp.as_mut().unwrap()[3] = 0;
    phase.admission = Some(idle.clone());
    phase.gate_transitions[0] =
        validation::GateProgress::new("stable".into(), idle.native_stamp, 0).unwrap();
    let readiness = phase.end_ready(
        &pcm_route(&idle),
        idle.clone(),
        &ids,
        1_100,
        12_000,
        Duration::ZERO,
    );
    assert!(
        matches!(readiness, Ok(false)),
        "pre-client state must wait: {readiness:?}"
    );
    idle.native_stamp.as_mut().unwrap()[3] = 1;
    assert!(
        phase
            .end_ready(&pcm_route(&idle), idle, &ids, 1_200, 12_000, Duration::ZERO)
            .unwrap()
    );
    assert!(phase.require_complete().is_ok());
}

#[test]
fn pcm_end_waits_for_legal_readmission_then_requires_both_open_gates() {
    let ids = ["stable".into(), "sibling".into()];
    for sibling in [false, true] {
        let mut phase = pcm_end_phase();
        let mut current = runtime(false);
        current.native_stamp = Some([10, 7, 11, 4, 6]);
        let mut route = pcm_route(&current);
        let index = usize::from(sibling);
        // Match the actual coordinator: its pre-pump observation records a
        // newer open generation, then its first end check sees the closure.
        let open_stamp = if sibling {
            [20, 7, 21, 4, 6]
        } else {
            [10, 7, 11, 4, 6]
        };
        phase.gate_transitions[index]
            .observe(Some(open_stamp), true, 1_100, 12_000)
            .unwrap();
        if sibling {
            route.exports[index].native_stamp = Some([20, 7, 21, 4, 7]);
        } else {
            current.native_stamp = Some([10, 7, 11, 4, 7]);
            route.exports[index].native_stamp = current.native_stamp;
            current.gate_open = false;
        }
        route.exports[index].gate_open = false;
        let readiness =
            phase.end_ready(&route, current.clone(), &ids, 1_200, 12_000, Duration::ZERO);
        assert!(
            matches!(readiness, Ok(false)),
            "legal closure must wait: {readiness:?}"
        );
        assert_eq!(phase.gate_transitions[index].closures, 1);
        assert!(!phase.confirmed.as_ref().unwrap().gate_open || sibling);
        route.exports[index].gate_open = true;
        current.gate_open = true;
        assert!(
            phase
                .end_ready(&route, current, &ids, 1_300, 12_000, Duration::ZERO)
                .unwrap()
        );
        assert_eq!(phase.gate_transitions[index].reopens, 1);
    }
}

#[test]
fn pcm_end_rejects_unexplained_closure_and_keeps_readmission_deadline() {
    let ids = ["stable".into(), "sibling".into()];
    let mut phase = pcm_end_phase();
    let mut current = runtime(false);
    current.gate_open = false;
    assert!(
        phase
            .end_ready(
                &pcm_route(&current),
                current,
                &ids,
                1_100,
                12_000,
                Duration::ZERO
            )
            .is_err()
    );

    let mut phase = pcm_end_phase();
    let mut current = runtime(false);
    current.native_stamp = Some([10, 7, 11, 4, 6]);
    current.gate_open = false;
    assert!(
        !phase
            .end_ready(
                &pcm_route(&current),
                current.clone(),
                &ids,
                1_100,
                12_000,
                Duration::ZERO
            )
            .unwrap()
    );
    current.native_stamp = Some([10, 7, 11, 5, 7]);
    assert!(
        !phase
            .end_ready(
                &pcm_route(&current),
                current.clone(),
                &ids,
                2_000,
                12_000,
                Duration::ZERO
            )
            .unwrap()
    );
    assert_eq!(
        phase.gate_transitions[0]
            .pending
            .as_ref()
            .unwrap()
            .deadline_ms,
        6_100
    );
    current.gate_open = true;
    assert!(
        phase
            .end_ready(
                &pcm_route(&current),
                current,
                &ids,
                6_100,
                12_000,
                Duration::ZERO
            )
            .is_err()
    );
}

#[test]
fn pcm_end_without_fresh_strict_signal_cannot_finish() {
    let ids = ["stable".into(), "sibling".into()];
    let mut phase = pcm_end_phase();
    phase.pcm = Observation::default();
    let current = runtime(false);
    assert!(
        !phase
            .end_ready(
                &pcm_route(&current),
                current,
                &ids,
                1_100,
                12_000,
                Duration::ZERO
            )
            .unwrap()
    );
}

#[test]
fn pcm_end_keeps_identity_revision_error_and_phase_deadline_gates() {
    let ids = ["stable".into(), "sibling".into()];
    let changes: &[fn(&mut RuntimeEvidence)] = &[
        |current| current.native_stamp = Some([99, 7, 11, 3, 5]),
        |current| current.active_revision = Some(7),
        |current| current.intent.binding = Some("other-node".into()),
        |current| current.observation_generation = Some(39),
        |current| current.service_error = Some("service failed".into()),
        |current| current.graph_error = Some("graph failed".into()),
        |current| current.provider_error = Some("provider failed".into()),
    ];
    for change in changes {
        let mut phase = pcm_end_phase();
        let mut current = runtime(false);
        change(&mut current);
        assert!(
            phase
                .end_ready(
                    &pcm_route(&current),
                    current,
                    &ids,
                    1_100,
                    12_000,
                    Duration::ZERO
                )
                .is_err()
        );
    }
    let mut phase = pcm_end_phase();
    let current = runtime(false);
    assert!(
        phase
            .end_ready(
                &pcm_route(&current),
                current,
                &ids,
                12_000,
                12_000,
                Duration::ZERO
            )
            .is_err()
    );
    let mut phase = pcm_end_phase();
    let current = runtime(false);
    let mut route = pcm_route(&current);
    route.exports[1].native_stamp = Some([99, 7, 21, 3, 5]);
    assert!(
        phase
            .end_ready(&route, current, &ids, 1_100, 12_000, Duration::ZERO)
            .is_err()
    );
}

#[test]
fn pcm_end_rechecks_publication_age_after_pumping() {
    let ids = ["stable".into(), "sibling".into()];
    let current = runtime(false);
    let mut phase = pcm_end_phase();
    assert!(
        phase
            .end_ready(
                &pcm_route(&current),
                current.clone(),
                &ids,
                3_000,
                12_000,
                Duration::from_secs(2),
            )
            .unwrap()
    );
    assert!(
        phase
            .end_ready(
                &pcm_route(&current),
                current,
                &ids,
                3_001,
                12_000,
                Duration::from_millis(2_001),
            )
            .is_err(),
        "a stale final snapshot must not pass after a fresh loop-entry check"
    );
}
