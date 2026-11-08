use super::*;
use crate::service::exports::runtime::tests::FakeAdapter;

fn fresh_rig() -> (Rig, Arc<FakeAdapter>) {
    let mut rig = Rig::new();
    let adapter = Arc::new(FakeAdapter::default());
    rig.driver.exports_runtime.adapter = Some(adapter.clone());
    (rig, adapter)
}

fn create(rig: &mut Rig) -> ExportIntent {
    rig.request(AppRequest::CreateExport {
        direction: ExportDirection::Playback,
        name: "Fresh playback".into(),
        channels: 2,
    });
    rig.c.graph.snapshot().exports[0].clone()
}

#[test]
fn fresh_export_retries_retain_allocation_then_rename_reopens_the_same_identity() {
    let (mut rig, adapter) = fresh_rig();
    adapter.defer_create.store(true, Ordering::Release);
    let mut intent = create(&mut rig);
    rig.wait(|_| adapter.creations.lock().unwrap().len() >= 2);
    let token = {
        let attempts = adapter.creations.lock().unwrap();
        let token = attempts[0].1.clone().expect("fresh allocation authority");
        for (id, attempt) in attempts.iter() {
            assert_eq!(*id, intent.endpoint_id);
            assert!(Arc::ptr_eq(&token, attempt.as_ref().unwrap()));
        }
        token
    };
    assert_eq!(token.endpoint_id(), intent.endpoint_id);
    assert!(!rig.c.exports.allow_default(intent.endpoint_id));
    assert_eq!(intent.binding, None);
    adapter.defer_create.store(false, Ordering::Release);
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Present
    });
    assert!(rig.c.exports.new_identities.is_empty());
    intent.name = "Renamed playback".into();
    rig.request(AppRequest::EditGraph(GraphEdit::SetExports(vec![
        intent.clone(),
    ])));
    rig.wait(|r| {
        let status = &r.c.exports.statuses(&r.c.graph.snapshot())[0];
        status.resource == ExportResourceState::Present
            && status.actual_name.as_deref() == Some("Renamed playback")
    });
    let attempts = adapter.creations.lock().unwrap();
    assert_eq!(attempts.last().unwrap().0, intent.endpoint_id);
    assert!(attempts.last().unwrap().1.is_none());
}

#[test]
fn project_load_cannot_inherit_fresh_authority_for_its_saved_id() {
    let (mut rig, adapter) = fresh_rig();
    let intent = create(&mut rig);
    let document =
        crate::project::ProjectDocument::from_graph("Loaded", &rig.c.graph.snapshot()).unwrap();
    rig.request(AppRequest::LoadProject(document));
    assert!(rig.c.exports.new_identities.is_empty());
    rig.wait(|_| !adapter.creations.lock().unwrap().is_empty());
    let attempts = adapter.creations.lock().unwrap();
    assert_eq!(attempts[0].0, intent.endpoint_id);
    assert!(attempts[0].1.is_none(), "saved IDs only reopen or adopt");
}

#[test]
fn uncertain_create_keeps_its_original_allocation_without_a_second_create() {
    let (mut rig, adapter) = fresh_rig();
    adapter.uncertain_create.store(true, Ordering::Release);
    let intent = create(&mut rig);
    rig.wait(|r| {
        r.c.exports.statuses(&r.c.graph.snapshot())[0].resource == ExportResourceState::Creating
    });
    for _ in 0..10 {
        rig.tick();
    }
    assert_eq!(adapter.creations.lock().unwrap().len(), 1);
    assert!(
        rig.c
            .exports
            .new_identities
            .contains_key(&intent.endpoint_id)
    );
}

#[test]
fn replacing_the_document_clears_unestablished_allocations() {
    let (mut rig, _) = fresh_rig();
    let intent = create(&mut rig);
    let token = rig.c.exports.new_identities[&intent.endpoint_id].clone();
    let graph = ServiceGraph::default().snapshot();
    rig.request(AppRequest::ReplaceGraph {
        graph: graph.graph,
        nodes: graph.nodes,
    });
    assert!(rig.c.exports.new_identities.is_empty());
    assert!(!token.valid_for(intent.endpoint_id));
}

#[test]
fn queued_fresh_dispatch_is_revoked_when_its_document_is_loaded_as_plain_ids() {
    let (mut rig, adapter) = fresh_rig();
    let intent = create(&mut rig);
    let graph = rig.c.graph.snapshot();
    rig.c.exports.set_available(true);
    let work = rig
        .driver
        .exports_runtime
        .work(&rig.c.exports, &graph, true)
        .unwrap();
    let token = work.actions[0].new_identity.as_ref().unwrap().clone();
    let document = crate::project::ProjectDocument::from_graph("Loaded", &graph).unwrap();
    rig.request(AppRequest::LoadProject(document));
    assert!(!token.valid_for(intent.endpoint_id));
    let batch = crate::service::exports::runtime::Adapter::execute(adapter.as_ref(), work);
    assert_eq!(batch.errors, ["fresh allocation revoked"]);
    assert!(adapter.creations.lock().unwrap().is_empty());
}

#[test]
fn present_without_confirmed_policy_keeps_allocation_until_publication_completes() {
    use crate::service::exports::runtime::{Adapter, Batch};
    let (mut rig, adapter) = fresh_rig();
    let intent = create(&mut rig);
    let graph = rig.c.graph.snapshot();
    rig.c.exports.set_available(true);
    let work = rig
        .driver
        .exports_runtime
        .work(&rig.c.exports, &graph, true)
        .unwrap();
    let mut batch = adapter.execute(work);
    batch.observations[0].publication_prepared = false;
    rig.driver
        .exports_runtime
        .complete(&mut rig.c.exports, Ok(batch));
    assert!(
        rig.c
            .exports
            .new_identities
            .contains_key(&intent.endpoint_id)
    );
    // Failed/unknown native readback cannot consume initial authority either.
    let work = rig
        .driver
        .exports_runtime
        .work(&rig.c.exports, &graph, true)
        .unwrap();
    assert!(
        work.actions.is_empty(),
        "Present must not dispatch a second CREATE"
    );
    let mut batch = adapter.execute(work);
    batch.observations[0].error = Some("QUERY failed".into());
    rig.driver
        .exports_runtime
        .complete(&mut rig.c.exports, Ok(batch));
    assert!(
        rig.c
            .exports
            .new_identities
            .contains_key(&intent.endpoint_id)
    );
    let work = rig
        .driver
        .exports_runtime
        .work(&rig.c.exports, &graph, true)
        .unwrap();
    let Batch {
        observations,
        errors,
    } = adapter.execute(work);
    rig.driver.exports_runtime.complete(
        &mut rig.c.exports,
        Ok(Batch {
            observations,
            errors,
        }),
    );
    assert!(rig.c.exports.new_identities.is_empty());
    assert_eq!(adapter.creations.lock().unwrap().len(), 1);
}

#[test]
fn rejected_creation_and_rejected_project_leave_no_partial_identity_change() {
    let (mut rig, _) = fresh_rig();
    rig.request(AppRequest::CreateExport {
        direction: ExportDirection::Playback,
        name: "\n".into(),
        channels: 2,
    });
    assert!(rig.c.graph.snapshot().exports.is_empty());
    assert!(rig.c.exports.new_identities.is_empty());
    let intent = create(&mut rig);
    let mut document =
        crate::project::ProjectDocument::from_graph("Invalid", &rig.c.graph.snapshot()).unwrap();
    document.exports[0].name = "\n".into();
    rig.request(AppRequest::LoadProject(document));
    assert!(
        rig.c
            .exports
            .new_identities
            .contains_key(&intent.endpoint_id)
    );
    assert_eq!(rig.c.graph.snapshot().exports[0], intent);
}
