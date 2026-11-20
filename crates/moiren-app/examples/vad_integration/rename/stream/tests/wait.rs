use super::*;
use std::cell::RefCell;

const PHASES: [WaitPhase; 3] = [
    WaitPhase {
        stage: "old_apply",
        request: 6,
    },
    WaitPhase {
        stage: "replacement_present",
        request: 6,
    },
    WaitPhase {
        stage: "replacement_apply",
        request: 6,
    },
];

fn failed_runtime(error_kind: usize) -> RuntimeEvidence {
    let mut observed = runtime(true);
    match error_kind {
        0 => observed.service_error = Some("service wait failed".into()),
        1 => observed.graph_error = Some("graph wait failed".into()),
        _ => observed.provider_error = Some("provider wait QUERY failed".into()),
    }
    observed
}

#[test]
fn current_wait_error_survives_later_ready_and_deduplicates_polls() {
    for phase in PHASES {
        for error_kind in 0..3 {
            let mut case = complete();
            let failed = failed_runtime(error_kind);
            assert!(!case.observe_wait(phase, &failed, || false));
            assert!(!case.observe_wait(phase, &failed, || false));
            assert!(case.observe_wait(phase, &runtime(true), || true));
            case.finish();
            assert!(
                !case.completed,
                "{} forgot current error {error_kind}",
                phase.stage
            );
            assert_eq!(case.failures.len(), 1, "duplicate wait error");
            assert!(case.failures[0].starts_with(phase.stage));
        }
    }
}

#[test]
fn current_wait_error_on_ready_snapshot_still_prevents_acceptance() {
    for phase in PHASES {
        for error_kind in 0..3 {
            let mut case = complete();
            assert!(case.observe_wait(phase, &failed_runtime(error_kind), || true));
            case.finish();
            assert!(
                !case.completed,
                "{} ignored ready snapshot error {error_kind}",
                phase.stage
            );
            assert_eq!(case.failures.len(), 1);
        }
    }
}

#[test]
fn stale_wait_snapshot_neither_completes_nor_attributes_its_error() {
    for phase in PHASES {
        for error_kind in 0..3 {
            let mut case = complete();
            let mut stale = failed_runtime(error_kind);
            stale.consumed_request_id = phase.request - 1;
            assert!(!case.observe_wait(phase, &stale, || {
                panic!("pre-request snapshot reached readiness")
            }));
            assert!(case.failures.is_empty());
            assert!(case.observe_wait(phase, &runtime(true), || true));
            case.finish();
            assert!(case.completed, "{:?}", case.failures);
        }
    }
}

#[test]
fn clean_pending_wait_remains_pending_without_failure() {
    for phase in PHASES {
        let mut case = complete();
        let mut pending = runtime(true);
        pending.rename_pending = true;
        pending.resource = "removing".into();
        assert!(!case.observe_wait(phase, &pending, || false));
        assert!(case.failures.is_empty());
        assert!(case.observe_wait(phase, &runtime(true), || true));
        case.finish();
        assert!(case.completed, "{:?}", case.failures);
    }
}

fn route() -> ([String; 2], [RuntimeEvidence; 2]) {
    let mut peer = runtime(true);
    peer.intent.endpoint_id = "peer".into();
    peer.intent.direction = "Microphone".into();
    (["stable".into(), "peer".into()], [runtime(true), peer])
}

fn project_route(route: &[RuntimeEvidence; 2], id: &str) -> anyhow::Result<RuntimeEvidence> {
    route
        .iter()
        .find(|runtime| runtime.intent.endpoint_id == id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing configured route export {id}"))
}

#[test]
fn wait_retains_either_route_provider_error_after_both_exports_recover() {
    for phase in PHASES {
        for failed_index in 0..2 {
            for ready_with_error in [false, true] {
                let mut case = complete();
                let (ids, clean) = route();
                let mut failed = clean.clone();
                failed[failed_index].provider_error = Some("route QUERY failed".into());
                failed[failed_index].export_active_revision = None;
                assert_eq!(
                    case.wait_ready(
                        phase,
                        &ids,
                        |id| project_route(&failed, id),
                        || ready_with_error,
                    )
                    .unwrap(),
                    ready_with_error
                );
                assert!(
                    !case
                        .wait_ready(phase, &ids, |id| project_route(&failed, id), || false)
                        .unwrap()
                );
                assert!(
                    case.wait_ready(phase, &ids, |id| project_route(&clean, id), || true)
                        .unwrap()
                );
                case.finish();
                assert!(
                    !case.completed,
                    "{} lost provider error on route export {failed_index}",
                    phase.stage
                );
                assert_eq!(case.failures.len(), 1, "duplicate route provider failure");
            }
        }
    }
}

#[test]
fn wait_projects_both_route_exports_before_evaluating_readiness() {
    for phase in PHASES {
        let mut case = complete();
        let (ids, clean) = route();
        let projected = RefCell::new(Vec::new());
        assert!(
            case.wait_ready(
                phase,
                &ids,
                |id| {
                    projected.borrow_mut().push(id.to_owned());
                    project_route(&clean, id)
                },
                || {
                    assert_eq!(projected.borrow().as_slice(), ids.as_slice());
                    true
                },
            )
            .unwrap()
        );
        case.finish();
        assert!(case.completed, "{:?}", case.failures);
    }
}

#[test]
fn stale_route_errors_do_not_reach_readiness_or_current_case_failures() {
    for phase in PHASES {
        let mut case = complete();
        let (ids, clean) = route();
        let mut stale = clean.clone();
        for runtime in &mut stale {
            runtime.consumed_request_id = phase.request - 1;
            runtime.provider_error = Some("earlier request QUERY failed".into());
        }
        assert!(
            !case
                .wait_ready(
                    phase,
                    &ids,
                    |id| project_route(&stale, id),
                    || panic!("stale route reached readiness"),
                )
                .unwrap()
        );
        assert!(case.failures.is_empty());
        assert!(
            case.wait_ready(phase, &ids, |id| project_route(&clean, id), || true)
                .unwrap()
        );
        case.finish();
        assert!(case.completed, "{:?}", case.failures);
    }
}
