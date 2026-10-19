mod common;

#[cfg(windows)]
#[test]
#[ignore = "requires live default capture/output devices; automatic startup must be muted"]
fn desktop_service_starts_without_a_ui_start_request() {
    use moiren_app::service::{SessionPhase, start_service};
    use std::time::{Duration, Instant};

    let runtime = start_service().unwrap();
    let handle = runtime.handle();
    // Deadline-tolerant poll: on timeout it yields the latest snapshot so the
    // phase assertion below reports why startup stalled, which a panicking
    // common::wait_until would discard.
    let deadline = Instant::now() + Duration::from_secs(10);
    let snapshot = loop {
        if let Some(snapshot) = handle.try_snapshot()
            && (matches!(snapshot.phase, SessionPhase::Running | SessionPhase::Failed)
                || Instant::now() >= deadline)
        {
            break snapshot;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    common::finish(runtime);
    assert_eq!(
        snapshot.phase,
        SessionPhase::Running,
        "desktop service did not automatically start: {snapshot:?}"
    );
    assert_eq!(snapshot.desired.as_ref().unwrap().gain, 0.0);
    assert_eq!(handle.try_snapshot().unwrap().phase, SessionPhase::Exited);
}
