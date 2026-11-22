//! Explicit gate closure must not leave local admission authorizing new IO.
use super::*;

#[test]
fn explicit_gate_close_revokes_local_admission_before_native_close_even_on_failure() {
    for fail_close in [false, true] {
        let admission = Arc::new(Admission::new());
        let mut lifecycle = Lifecycle::new(Fake::default());
        lifecycle.create(spec()).unwrap();
        let session = prepared(&mut lifecycle, admission.clone());
        let ticket = admission.set(Some(9));
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
            .unwrap();
        assert!(session.admitted());
        let sibling = SessionState::new(admission.clone());
        sibling.active.store(true, Ordering::Release);
        sibling.admitted_ticket.store(ticket, Ordering::Release);
        sibling.admitted_revision.store(9, Ordering::Release);
        assert!(sibling.admitted());
        lifecycle.control.observe_close = Some(session.clone());
        lifecycle.control.fail_close = fail_close;
        let result = lifecycle.gate(id(), [11, 7, 11, 3, 5], false, 9, ticket, &admission);
        if fail_close {
            assert_eq!(result, Err(unavailable("SET_GATE close failure")));
            assert!(lifecycle.control.current.gate_open());
        } else {
            result.unwrap();
            assert!(!lifecycle.control.current.gate_open());
        }
        assert!(!session.admitted());
        assert!(
            session.prepared(),
            "closing is not cancellation or draining"
        );
        assert!(!session.cancel.load(Ordering::Acquire));
        assert!(!session.drained.load(Ordering::Acquire));
        assert!(
            admission.matches(ticket, 9),
            "do not revoke sibling endpoints"
        );
        assert!(sibling.admitted());
        assert_eq!(session.reset_epoch.load(Ordering::Acquire), 0);
        assert_eq!(session.telemetry_snapshot().recovery_events, 0);
        assert_eq!(lifecycle.control.gates, [true, false]);
    }
}

#[test]
fn stale_explicit_gate_close_does_not_revoke_current_local_admission() {
    for changed_by_query in [false, true] {
        let admission = Arc::new(Admission::new());
        let mut lifecycle = Lifecycle::new(Fake::default());
        lifecycle.create(spec()).unwrap();
        let session = prepared(&mut lifecycle, admission.clone());
        let ticket = admission.set(Some(9));
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
            .unwrap();
        let expected = if changed_by_query {
            lifecycle.control.current = snapshot(PdoState::Present, 3, 6, true);
            [11, 7, 11, 3, 5]
        } else {
            [11, 7, 11, 3, 4]
        };
        assert_eq!(
            lifecycle.gate(id(), expected, false, 9, ticket, &admission),
            Err(BackendError::Cancelled)
        );
        assert!(
            session.admitted(),
            "stale dispatch cannot revoke a newer owner"
        );
        assert!(admission.matches(ticket, 9));
        assert!(lifecycle.control.current.gate_open());
        assert_eq!(lifecycle.control.gates, [true]);
    }
}

#[test]
fn preparation_fault_poll_revokes_before_native_close_even_on_failure() {
    for fail_close in [false, true] {
        let admission = Arc::new(Admission::new());
        let mut lifecycle = Lifecycle::new(Fake::default());
        lifecycle.create(spec()).unwrap();
        let session = prepared(&mut lifecycle, admission.clone());
        let ticket = admission.set(Some(9));
        lifecycle
            .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
            .unwrap();
        lifecycle.control.current = snapshot_native_with_status(
            PdoState::Present,
            3,
            5,
            true,
            11,
            (StreamState::Running, true, true),
            0xc000_0001,
        );
        lifecycle.control.observe_close = Some(session.clone());
        lifecycle.control.fail_close = fail_close;
        let views = lifecycle.poll();
        assert!(views[0].error.is_some());
        assert!(!session.admitted());
        assert!(session.prepared());
        assert_eq!(lifecycle.control.current.gate_open(), fail_close);
        assert!(!session.cancel.load(Ordering::Acquire));
        assert!(!session.drained.load(Ordering::Acquire));
        assert!(admission.matches(ticket, 9));
        assert_eq!(session.reset_epoch.load(Ordering::Acquire), 0);
        assert_eq!(session.telemetry_snapshot().recovery_events, 0);
        assert_eq!(lifecycle.control.gates, [true, false]);
    }
}

#[test]
fn closed_session_requires_fresh_explicit_gate_admission() {
    let admission = Arc::new(Admission::new());
    let mut lifecycle = Lifecycle::new(Fake::default());
    lifecycle.create(spec()).unwrap();
    let session = prepared(&mut lifecycle, admission.clone());
    let ticket = admission.set(Some(9));
    lifecycle
        .gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission)
        .unwrap();
    lifecycle
        .gate(id(), [11, 7, 11, 3, 5], false, 9, ticket, &admission)
        .unwrap();
    assert!(!session.admitted());
    assert!(!lifecycle.poll()[0].snapshot.unwrap().gate_open());
    assert!(!session.admitted(), "QUERY cannot restore local admission");
    assert_eq!(
        lifecycle.gate(id(), [11, 7, 11, 3, 5], true, 9, ticket, &admission),
        Err(BackendError::Cancelled)
    );
    assert!(!session.admitted());
    lifecycle
        .gate(id(), [11, 7, 11, 3, 6], true, 9, ticket, &admission)
        .unwrap();
    assert!(session.admitted());
    assert!(lifecycle.control.current.gate_open());
    assert_eq!(lifecycle.control.gates, [true, false, true]);
}
