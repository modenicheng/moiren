use super::*;

fn rename(r: &mut Rig, name: &str) -> ExportIntent {
    let mut intent = r.c.graph.snapshot().exports[0].clone();
    intent.name = name.into();
    r.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![
        intent.clone(),
    ])));
    r.request(AppRequest::ApplyGraph);
    intent
}

fn creates(adapter: &super::super::super::exports::runtime::tests::FakeAdapter) -> usize {
    adapter
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.0 == "create")
        .count()
}

#[test]
fn rename_waits_for_held_client_removal_after_owner_drain_and_gate_close() {
    let (mut r, adapter) = export_rig();
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let original = r.c.graph.snapshot().exports[0].clone();
    adapter.fail_remove.store(true, Ordering::Release);
    let requested = rename(&mut r, "Renamed while held");
    r.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Removing
    });
    for _ in 0..8 {
        r.tick();
    }
    let status = r.c.exports.statuses(&r.c.graph.snapshot()).remove(0);
    assert_eq!(status.intent, requested);
    assert_eq!(status.actual_name.as_deref(), Some(original.name.as_str()));
    assert!(status.rename_pending);
    assert!(!status.gate_open);
    assert!(r.driver.owner.is_none());
    assert_eq!(r.driver.cleaning, 0);
    assert_eq!(r.state.stops.load(Ordering::SeqCst), 1);
    assert_eq!(creates(&adapter), 1, "held PDO cannot be recreated");
    adapter.fail_remove.store(false, Ordering::Release);
    r.wait(|r| {
        let status = &r.c.exports.statuses(&r.c.graph.snapshot())[0];
        status.gate_open && status.actual_name.as_deref() == Some(requested.name.as_str())
    });
    let status = r.c.exports.statuses(&r.c.graph.snapshot()).remove(0);
    assert!(!status.rename_pending);
    assert_eq!(status.intent.endpoint_id, original.endpoint_id);
    assert_eq!(status.intent.binding, original.binding);
    assert_eq!(r.c.graph.snapshot().exports, vec![requested]);
    assert_eq!(creates(&adapter), 2);
    let calls = adapter.calls.lock().unwrap();
    let close = calls.iter().position(|c| c.0 == "close").unwrap();
    let remove = calls.iter().position(|c| c.0 == "remove").unwrap();
    let recreate = calls.iter().rposition(|c| c.0 == "create").unwrap();
    assert!(close < remove && remove < recreate);
}

#[test]
fn rename_create_failure_retains_desired_name_without_claiming_native_presence() {
    let (mut r, adapter) = export_rig();
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    adapter.defer_create.store(true, Ordering::Release);
    let requested = rename(&mut r, "Retained after create failure");
    r.wait(|r| {
        r.c.graph
            .snapshot()
            .last_error
            .as_deref()
            .is_some_and(|s| s.contains("CREATE deferred"))
    });
    let status = r.c.exports.statuses(&r.c.graph.snapshot()).remove(0);
    assert_eq!(status.intent, requested);
    assert_eq!(status.resource, ExportResourceState::Absent);
    assert_eq!(status.actual_name, None);
    assert!(status.rename_pending);
    assert!(!status.gate_open);
    assert_eq!(status.active_revision, None);
    assert_eq!(r.c.graph.snapshot().exports, vec![requested.clone()]);
    adapter.defer_create.store(false, Ordering::Release);
    r.wait(|r| {
        let status = &r.c.exports.statuses(&r.c.graph.snapshot())[0];
        status.resource == ExportResourceState::Present
            && status.actual_name.as_deref() == Some(requested.name.as_str())
    });
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    assert_eq!(r.c.graph.snapshot().exports, vec![requested]);
}

#[test]
fn repeated_rename_while_removing_recreates_only_the_latest_name_and_binding() {
    let (mut r, adapter) = export_rig();
    r.request(AppRequest::ApplyGraph);
    r.wait(|r| r.c.exports.statuses(&r.c.graph.snapshot())[0].gate_open);
    let original = r.c.graph.snapshot().exports[0].clone();
    adapter.fail_remove.store(true, Ordering::Release);
    rename(&mut r, "Intermediate rename");
    r.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Removing
    });
    let latest = rename(&mut r, "Final rename");
    for _ in 0..8 {
        r.tick();
    }
    assert_eq!(creates(&adapter), 1);
    assert_eq!(r.c.graph.snapshot().exports, vec![latest.clone()]);
    assert_eq!(
        r.c.exports.observations[&original.endpoint_id].intent.name,
        original.name
    );
    adapter.fail_remove.store(false, Ordering::Release);
    r.wait(|r| {
        let status = &r.c.exports.statuses(&r.c.graph.snapshot())[0];
        status.gate_open && status.actual_name.as_deref() == Some(latest.name.as_str())
    });
    assert_eq!(creates(&adapter), 2);
    let status = r.c.exports.statuses(&r.c.graph.snapshot()).remove(0);
    assert_eq!(status.intent.endpoint_id, original.endpoint_id);
    assert_eq!(status.intent.binding, original.binding);
    assert_eq!(status.intent, latest);
    assert!(!status.rename_pending);
}
