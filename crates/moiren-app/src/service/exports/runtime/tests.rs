//! A fake at the native-IO boundary; the service, Engine host and job queues
//! remain real in managed/tests.rs.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(crate) struct FakeAdapter {
    pub revision: AtomicU64,
    ticket: AtomicU64,
    pub fail_remove: AtomicBool,
    pub uncertain_create: AtomicBool,
    pub transport_unavailable: AtomicBool,
    pub defer_create: AtomicBool,
    pub hold_open: AtomicBool,
    pub open_entered: AtomicBool,
    pub calls: Mutex<Vec<(String, Option<u64>, String)>>,
    records: Mutex<BTreeMap<VirtualEndpointId, Record>>,
}
struct Record {
    intent: ExportIntent,
    stamp: DriverStamp,
    resource: ExportResourceState,
    gate: bool,
    allow_default: bool,
}
impl Default for FakeAdapter {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            ticket: AtomicU64::new(1),
            fail_remove: AtomicBool::new(false),
            uncertain_create: AtomicBool::new(false),
            transport_unavailable: AtomicBool::new(false),
            defer_create: AtomicBool::new(false),
            hold_open: AtomicBool::new(false),
            open_entered: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            records: Mutex::new(BTreeMap::new()),
        }
    }
}
impl Adapter for FakeAdapter {
    fn authorize(&self, revision: Option<u64>) -> u64 {
        self.revision
            .store(revision.unwrap_or(0), Ordering::Release);
        self.ticket.fetch_add(1, Ordering::AcqRel) + 1
    }
    fn execute(&self, work: Work) -> Batch {
        let mut records = self.records.lock().unwrap();
        for record in records.values_mut() {
            if record.gate && self.transport_unavailable.load(Ordering::Acquire) {
                record.gate = false;
                record.stamp[4] += 1;
            }
            if record.resource == ExportResourceState::Removing
                && !self.fail_remove.load(Ordering::Acquire)
            {
                record.resource = ExportResourceState::Absent;
            }
        }
        let mut errors = Vec::new();
        let mut calls = self.calls.lock().unwrap();
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        if work.shutdown {
            assert!(work.drained);
            for record in records.values_mut() {
                if self.uncertain_create.load(Ordering::Acquire) {
                    continue;
                }
                if record.resource != ExportResourceState::Absent {
                    record.gate = false;
                    record.resource = ExportResourceState::Removing;
                    calls.push(("remove".into(), None, thread.clone()));
                    if self.fail_remove.load(Ordering::Acquire) {
                        errors.push("shutdown REMOVE timeout".into());
                    }
                }
            }
        }
        for dispatch in work.actions {
            match dispatch.action {
                ExportAction::Create {
                    mut intent,
                    allow_default,
                } => {
                    let old = records.get(&intent.endpoint_id);
                    assert!(old.is_none_or(|r| r.resource == ExportResourceState::Absent));
                    let generation = old.map_or(1, |r| r.stamp[2] + 1);
                    calls.push(("create".into(), None, thread.clone()));
                    if self.defer_create.load(Ordering::Acquire) {
                        // A validated deferral owns no resource in this session.
                        records.remove(&intent.endpoint_id);
                        errors.push("CREATE deferred: no resource created".into());
                        continue;
                    }
                    intent.binding = None;
                    records.insert(
                        intent.endpoint_id,
                        Record {
                            intent,
                            stamp: [generation, 7, generation, 3, 5],
                            resource: if self.uncertain_create.load(Ordering::Acquire) {
                                ExportResourceState::Creating
                            } else {
                                ExportResourceState::Present
                            },
                            gate: false,
                            allow_default,
                        },
                    );
                }
                ExportAction::SetGate { endpoint_id, open } => {
                    if open {
                        self.open_entered.store(true, Ordering::Release);
                        let deadline = Instant::now() + Duration::from_secs(3);
                        while self.hold_open.load(Ordering::Acquire) && Instant::now() < deadline {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                    let record = records.get_mut(&endpoint_id).unwrap();
                    if dispatch.expected != Some(record.stamp) {
                        continue;
                    }
                    if open
                        && (self.transport_unavailable.load(Ordering::Acquire)
                            || work.ticket != self.ticket.load(Ordering::Acquire)
                            || work.revision.unwrap_or(0) == 0
                            || work.revision.unwrap_or(0) != self.revision.load(Ordering::Acquire))
                    {
                        continue;
                    }
                    if record.gate && !open {
                        record.stamp[4] += 1;
                    }
                    record.gate = open;
                    calls.push((
                        if open { "open" } else { "close" }.into(),
                        work.revision,
                        thread.clone(),
                    ));
                }
                ExportAction::Remove { endpoint_id } => {
                    assert!(work.drained, "native retirement must wait for host reap");
                    let record = records.get_mut(&endpoint_id).unwrap();
                    assert!(!record.gate);
                    assert_eq!(dispatch.expected, Some(record.stamp));
                    record.resource = ExportResourceState::Removing;
                    calls.push(("remove".into(), None, thread.clone()));
                    if self.fail_remove.load(Ordering::Acquire) {
                        errors.push("REMOVE timeout".into());
                    }
                }
                ExportAction::SetDefaultEligibility { .. } => panic!("eligibility must recreate"),
            }
        }
        let observations = records
            .values()
            .map(|record| NativeObservation {
                intent: record.intent.clone(),
                snapshot: (!self.uncertain_create.load(Ordering::Acquire)).then_some(record.stamp),
                resource: record.resource,
                gate_open: record.gate,
                allow_default: record.allow_default,
                can_open: !self.transport_unavailable.load(Ordering::Acquire),
                error: None,
            })
            .collect();
        Batch {
            observations,
            errors,
        }
    }
}

fn runtime_graph() -> (crate::service::GraphSnapshot, ExportIntent) {
    let mut graph = crate::service::ServiceGraph::default();
    let intent = super::super::tests::intent(&graph);
    graph
        .edit(crate::service::GraphEdit::SetExports(vec![intent.clone()]))
        .unwrap();
    let mut graph = graph.snapshot();
    graph.active_revision = Some(graph.candidate_revision);
    (graph, intent)
}

#[test]
fn explicit_create_deferral_retries_until_one_creation_is_accepted() {
    let (graph, intent) = runtime_graph();
    let adapter = Arc::new(FakeAdapter::default());
    adapter.defer_create.store(true, Ordering::Release);
    let mut runtime = Runtime {
        adapter: Some(adapter.clone()),
        ..Default::default()
    };
    let mut service = ExportService::default();
    for attempt in 1..=3 {
        runtime.project(&mut service, &graph, &[], None);
        let work = runtime.work(&service, &graph, true).unwrap();
        assert_eq!(work.actions.len(), 1);
        assert!(matches!(&work.actions[0].action,
            ExportAction::Create { intent: requested, .. } if requested == &intent
        ));
        let batch = adapter.execute(work);
        assert_eq!(batch.errors.len(), 1);
        assert!(batch.observations.is_empty());
        assert_eq!(runtime.complete(&mut service, Ok(batch)).len(), 1);
        assert!(service.observations.is_empty());
        assert!(runtime.stamps.is_empty());
        assert!(!runtime.ready_for(&service, std::slice::from_ref(&intent)));
        let status = &service.statuses(&graph)[0];
        assert_eq!(status.resource, ExportResourceState::Absent);
        assert!(!status.gate_open);
        assert!(status.active_revision.is_none());
        assert!(adapter.records.lock().unwrap().is_empty());
        assert_eq!(adapter.calls.lock().unwrap().len(), attempt);
    }
    adapter.defer_create.store(false, Ordering::Release);
    let work = runtime.work(&service, &graph, true).unwrap();
    assert_eq!(work.actions.len(), 1);
    assert!(
        runtime
            .complete(&mut service, Ok(adapter.execute(work)))
            .is_empty()
    );
    assert_eq!(
        service.statuses(&graph)[0].resource,
        ExportResourceState::Present
    );
    for _ in 0..3 {
        runtime.project(&mut service, &graph, &[], None);
        let work = runtime.work(&service, &graph, true).unwrap();
        assert!(work.actions.is_empty(), "accepted creation must not repeat");
        assert!(
            runtime
                .complete(&mut service, Ok(adapter.execute(work)))
                .is_empty()
        );
    }
    assert_eq!(adapter.records.lock().unwrap().len(), 1);
    let calls = adapter.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(calls.iter().all(|(kind, _, _)| kind == "create"));
}

#[test]
fn disabling_intent_during_explicit_create_deferral_stops_retry_and_retirement() {
    let (mut graph, _) = runtime_graph();
    let adapter = Arc::new(FakeAdapter::default());
    adapter.defer_create.store(true, Ordering::Release);
    let mut runtime = Runtime {
        adapter: Some(adapter.clone()),
        ..Default::default()
    };
    let mut service = ExportService::default();
    runtime.project(&mut service, &graph, &[], None);
    let work = runtime.work(&service, &graph, true).unwrap();
    let batch = adapter.execute(work);
    assert!(batch.observations.is_empty());
    assert_eq!(batch.errors.len(), 1);
    // Desired intent can change before the already dispatched result arrives.
    graph.exports[0].enabled = false;
    assert_eq!(runtime.complete(&mut service, Ok(batch)).len(), 1);
    adapter.defer_create.store(false, Ordering::Release);
    for _ in 0..3 {
        runtime.project(&mut service, &graph, &[], None);
        let work = runtime.work(&service, &graph, true).unwrap();
        assert!(work.actions.is_empty());
        assert!(
            runtime
                .complete(&mut service, Ok(adapter.execute(work)))
                .is_empty()
        );
    }
    assert!(service.observations.is_empty());
    assert!(adapter.records.lock().unwrap().is_empty());
    let calls = adapter.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "no CREATE retry or REMOVE after disable");
    assert_eq!(calls[0].0, "create");
    let status = &service.statuses(&graph)[0];
    assert!(!status.intent.enabled);
    assert_eq!(status.resource, ExportResourceState::Absent);
    assert!(!status.gate_open);
}

#[test]
fn uncertain_create_stays_creating_without_retry_or_admission() {
    let (graph, intent) = runtime_graph();
    let adapter = Arc::new(FakeAdapter::default());
    adapter.uncertain_create.store(true, Ordering::Release);
    let mut runtime = Runtime {
        adapter: Some(adapter.clone()),
        ..Default::default()
    };
    let mut service = ExportService::default();
    for round in 0..4 {
        runtime.project(&mut service, &graph, &[], None);
        let work = runtime.work(&service, &graph, true).unwrap();
        if round == 0 {
            assert_eq!(work.actions.len(), 1);
            assert!(matches!(
                work.actions[0].action,
                ExportAction::Create { .. }
            ));
        } else {
            assert!(
                work.actions.is_empty(),
                "unknown creation must stay tracked"
            );
        }
        let batch = adapter.execute(work);
        assert_eq!(batch.observations.len(), 1);
        assert_eq!(
            batch.observations[0].resource,
            ExportResourceState::Creating
        );
        assert!(batch.observations[0].snapshot.is_none());
        runtime.complete(&mut service, Ok(batch));
        let status = &service.statuses(&graph)[0];
        assert_eq!(status.resource, ExportResourceState::Creating);
        assert!(!status.gate_open);
        assert!(status.active_revision.is_none());
        assert!(!runtime.ready_for(&service, std::slice::from_ref(&intent)));
    }
    assert_eq!(service.observations.len(), 1);
    assert_eq!(adapter.records.lock().unwrap().len(), 1);
    let calls = adapter.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "create");
}

#[cfg(windows)]
fn native_snapshot(
    state: moiren_windows_audio::vad::PdoState,
    stream: moiren_windows_audio::vad::StreamState,
    policy: bool,
    audio: bool,
) -> moiren_windows_audio::vad::EndpointSnapshot {
    use moiren_windows_audio::vad::{Capabilities, EndpointSnapshot, StreamState};
    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    let mut caps = [0; 64];
    for (offset, value) in [
        (0, 64),
        (4, 2),
        (8, 31),
        (12, 64),
        (16, 1),
        (20, 48_000),
        (24, 3),
        (28, 6),
        (32, 6),
        (36, 4),
        (48, 4800),
    ] {
        put32(&mut caps, offset, value);
    }
    put64(&mut caps, 40, 7);
    let caps = Capabilities::decode(&caps).unwrap();
    let mut bytes = [0; 128];
    for (offset, value) in [
        (0, 128),
        (4, 2),
        (16, state as u32),
        (20, 1),
        (24, 2),
        (32, u32::from(policy)),
        (36, u32::from(audio)),
        (100, stream as u32),
    ] {
        put32(&mut bytes, offset, value);
    }
    for (offset, value) in [
        (8, 1),
        (40, 7),
        (48, 1),
        (56, u64::from(stream != StreamState::Closed)),
        (64, 5),
    ] {
        put64(&mut bytes, offset, value);
    }
    bytes[72..88].copy_from_slice(&[0x31; 16]);
    EndpointSnapshot::decode(&bytes, caps).unwrap()
}

#[cfg(windows)]
#[test]
fn native_admission_readiness_does_not_require_external_clients_or_pcm_shape() {
    use moiren_windows_audio::vad::{PdoState, StreamState};
    for stream in [
        StreamState::Closed,
        StreamState::Paused,
        StreamState::Running,
    ] {
        let snapshot = native_snapshot(PdoState::Present, stream, true, true);
        assert!(
            snapshot.sample_format().is_none(),
            "no PCM stream was prepared"
        );
        assert!(
            native_can_open(true, Some(snapshot)),
            "{stream:?} is admission-ready"
        );
        assert!(
            !native_can_open(false, Some(snapshot)),
            "an Engine owner is still required"
        );
    }
    assert!(!native_can_open(true, None));
    for (state, policy) in [
        (PdoState::Present, false),
        (PdoState::Present, true),
        (PdoState::Creating, true),
        (PdoState::Removing, true),
    ] {
        assert!(!native_can_open(
            true,
            Some(native_snapshot(state, StreamState::Closed, policy, false,))
        ));
    }
}

#[cfg(windows)]
#[test]
fn idle_native_mapping_plans_gate_open_only_after_engine_confirmation() {
    use moiren_windows_audio::vad::{PdoState, StreamState};
    let (graph, intent) = runtime_graph();
    let mut runtime = Runtime {
        adapter: Some(Arc::new(FakeAdapter::default())),
        ..Default::default()
    };
    let mut service = ExportService::default();
    service.set_available(true);
    let snapshot = native_snapshot(PdoState::Present, StreamState::Closed, true, true);
    assert!(
        runtime
            .complete(
                &mut service,
                Ok(Batch {
                    observations: vec![NativeObservation {
                        intent: intent.clone(),
                        snapshot: Some([1, 7, 1, 0, 5]),
                        resource: ExportResourceState::Present,
                        gate_open: false,
                        allow_default: false,
                        can_open: native_can_open(true, Some(snapshot)),
                        error: None,
                    }],
                    errors: vec![],
                })
            )
            .is_empty()
    );
    runtime.project(&mut service, &graph, std::slice::from_ref(&intent), None);
    assert!(
        runtime
            .work(&service, &graph, false)
            .unwrap()
            .actions
            .is_empty()
    );
    runtime.project(
        &mut service,
        &graph,
        std::slice::from_ref(&intent),
        Some([1, 1, 1, graph.candidate_revision, 1]),
    );
    let work = runtime.work(&service, &graph, false).unwrap();
    assert!(work.actions.iter().any(|dispatch| matches!(dispatch.action,
        ExportAction::SetGate { endpoint_id, open: true } if endpoint_id == intent.endpoint_id
    )));
    assert_eq!(work.revision, graph.active_revision);
    runtime.revoke();
    runtime.project(&mut service, &graph, std::slice::from_ref(&intent), None);
    assert!(
        runtime
            .work(&service, &graph, false)
            .unwrap()
            .actions
            .is_empty()
    );
}

#[test]
fn unhealthy_native_observation_cannot_confirm_or_prepare_and_plans_gate_close() {
    let (graph, intent) = runtime_graph();
    let mut runtime = Runtime {
        adapter: Some(Arc::new(FakeAdapter::default())),
        ..Default::default()
    };
    let mut service = ExportService::default();
    service.set_available(true);
    runtime.complete(
        &mut service,
        Ok(Batch {
            observations: vec![NativeObservation {
                intent: intent.clone(),
                snapshot: Some([1, 7, 1, 3, 5]),
                resource: ExportResourceState::Present,
                gate_open: true,
                allow_default: false,
                can_open: true,
                error: Some("QUERY timed out; snapshot is last known".into()),
            }],
            errors: vec![],
        }),
    );
    runtime.project(
        &mut service,
        &graph,
        std::slice::from_ref(&intent),
        Some([1, 1, 1, graph.candidate_revision, 1]),
    );
    assert!(!runtime.lifecycle_ready_for(&service, std::slice::from_ref(&intent)));
    assert!(!runtime.ready_for(&service, std::slice::from_ref(&intent)));
    assert_eq!(service.statuses(&graph)[0].active_revision, None);
    assert!(
        runtime
            .work(&service, &graph, false)
            .unwrap()
            .actions
            .iter()
            .any(|dispatch| matches!(dispatch.action, ExportAction::SetGate { open: false, .. }))
    );
}

#[test]
fn lifecycle_readiness_does_not_require_transport_admission() {
    let (graph, intent) = runtime_graph();
    let mut runtime = Runtime {
        adapter: Some(Arc::new(FakeAdapter::default())),
        ..Default::default()
    };
    let mut service = ExportService::default();
    service.set_available(true);

    // The PDO is still present after its PCM worker drained, so the provider
    // can report no query error while transport admission is already lost.
    runtime.complete(
        &mut service,
        Ok(Batch {
            observations: vec![NativeObservation {
                intent: intent.clone(),
                snapshot: Some([1, 7, 1, 3, 5]),
                resource: ExportResourceState::Present,
                gate_open: false,
                allow_default: false,
                can_open: false,
                error: None,
            }],
            errors: vec![],
        }),
    );

    runtime.project(
        &mut service,
        &graph,
        std::slice::from_ref(&intent),
        Some([1, 1, 1, graph.candidate_revision, 1]),
    );

    assert!(runtime.lifecycle_ready_for(&service, std::slice::from_ref(&intent)));
    assert!(!runtime.ready_for(&service, std::slice::from_ref(&intent)));
    assert_eq!(service.statuses(&graph)[0].active_revision, None);
    assert!(
        runtime
            .work(&service, &graph, false)
            .unwrap()
            .actions
            .is_empty()
    );
}

#[test]
fn confirmed_absence_releases_app_tracking_across_documents() {
    let (_, mut intent) = runtime_graph();
    let mut runtime = Runtime::default();
    let mut service = ExportService::default();
    service.set_available(true);
    for value in 1..=128 {
        intent.endpoint_id = VirtualEndpointId::from_bytes([value; 16]);
        runtime.complete(
            &mut service,
            Ok(Batch {
                observations: vec![NativeObservation {
                    intent: intent.clone(),
                    snapshot: Some([value as u64, 7, value as u64, 3, 5]),
                    resource: ExportResourceState::Absent,
                    gate_open: false,
                    allow_default: false,
                    can_open: false,
                    error: None,
                }],
                errors: vec![],
            }),
        );
    }
    assert!(runtime.is_drained());
    assert!(service.observations.is_empty());
    assert!(runtime.stamps.is_empty());
}

#[cfg(feature = "vad-integration-probe")]
mod native_stamp {
    use super::*;

    fn observed(intent: &ExportIntent, snapshot: Option<DriverStamp>) -> Batch {
        Batch {
            observations: vec![NativeObservation {
                intent: intent.clone(),
                snapshot,
                resource: ExportResourceState::Present,
                gate_open: false,
                allow_default: false,
                can_open: true,
                error: None,
            }],
            errors: vec![],
        }
    }

    #[test]
    fn raw_stamp_updates_without_granting_gate_admission_or_dispatching_io() {
        let (graph, intent) = runtime_graph();
        let adapter = Arc::new(FakeAdapter::default());
        let mut runtime = Runtime {
            adapter: Some(adapter.clone()),
            ..Default::default()
        };
        let mut service = ExportService::default();
        runtime.project(&mut service, &graph, &[], None);
        assert_eq!(service.statuses(&graph)[0].native_stamp, None);

        // A closed native stream can legitimately have generation zero.
        for stamp in [
            [11, 22, 33, 0, 55],
            [11, 22, 33, 44, 56],
            [12, 23, 34, 45, 57],
        ] {
            assert!(
                runtime
                    .complete(&mut service, Ok(observed(&intent, Some(stamp))))
                    .is_empty()
            );
            runtime.project(&mut service, &graph, std::slice::from_ref(&intent), None);
            for _ in 0..3 {
                let status = &service.statuses(&graph)[0];
                assert_eq!(status.native_stamp, Some(stamp));
                assert!(!status.gate_open);
                assert_eq!(status.active_revision, None);
            }
            assert!(
                runtime
                    .work(&service, &graph, false)
                    .unwrap()
                    .actions
                    .is_empty()
            );
        }
        assert!(adapter.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn missing_absent_unavailable_and_failed_observations_clear_raw_stamp() {
        let (graph, intent) = runtime_graph();
        let stamp = [11, 22, 33, 44, 55];
        let mut absent = observed(&intent, Some(stamp));
        absent.observations[0].resource = ExportResourceState::Absent;
        let mut unavailable = observed(&intent, Some(stamp));
        unavailable.observations[0].resource = ExportResourceState::Unavailable;
        let mut failed = observed(&intent, Some(stamp));
        failed.observations[0].error = Some("QUERY failed; snapshot is last known".into());
        for (case, completion) in [
            ("missing snapshot", Ok(observed(&intent, None))),
            (
                "missing endpoint",
                Ok(Batch {
                    observations: vec![],
                    errors: vec![],
                }),
            ),
            ("absent", Ok(absent)),
            ("unavailable", Ok(unavailable)),
            ("failed observation", Ok(failed)),
            ("failed completion", Err("lifecycle worker failed".into())),
        ] {
            let mut runtime = Runtime::default();
            let mut service = ExportService::default();
            service.set_available(true);
            assert!(
                runtime
                    .complete(&mut service, Ok(observed(&intent, Some(stamp))))
                    .is_empty()
            );
            assert_eq!(
                service.statuses(&graph)[0].native_stamp,
                Some(stamp),
                "{case}"
            );
            runtime.complete(&mut service, completion);
            let status = &service.statuses(&graph)[0];
            assert_eq!(status.native_stamp, None, "{case}");
            assert!(!status.gate_open, "{case}");

            let fresh = [12, 23, 34, 45, 56];
            assert!(
                runtime
                    .complete(&mut service, Ok(observed(&intent, Some(fresh))))
                    .is_empty()
            );
            assert_eq!(
                service.statuses(&graph)[0].native_stamp,
                Some(fresh),
                "{case}"
            );
        }
    }

    #[test]
    fn availability_loss_clears_raw_stamp_until_a_fresh_valid_completion() {
        let (graph, intent) = runtime_graph();
        let mut runtime = Runtime::default();
        let mut service = ExportService::default();
        let stamp = [11, 22, 33, 44, 55];
        service.set_available(true);
        assert!(
            runtime
                .complete(&mut service, Ok(observed(&intent, Some(stamp))))
                .is_empty()
        );
        assert_eq!(service.statuses(&graph)[0].native_stamp, Some(stamp));
        service.set_available(false);
        assert_eq!(service.statuses(&graph)[0].native_stamp, None);
        assert_eq!(
            runtime
                .complete(&mut service, Ok(observed(&intent, Some(stamp))))
                .len(),
            1
        );
        service.set_available(true);
        assert_eq!(service.statuses(&graph)[0].native_stamp, None);
        assert!(
            runtime
                .complete(&mut service, Ok(observed(&intent, Some(stamp))))
                .is_empty()
        );
        assert_eq!(service.statuses(&graph)[0].native_stamp, Some(stamp));
        assert!(!service.statuses(&graph)[0].gate_open);
    }

    #[test]
    fn rejected_completion_cannot_publish_raw_stamp() {
        let (graph, intent) = runtime_graph();
        for stale_generation in [false, true] {
            let mut runtime = Runtime::default();
            let mut service = ExportService::default();
            service.set_available(true);
            assert!(
                runtime
                    .complete(&mut service, Ok(observed(&intent, Some([1, 2, 3, 4, 5]))))
                    .is_empty()
            );
            let mut rejected = observed(&intent, Some([6, 7, 8, 9, 10]));
            if stale_generation {
                runtime.sequence = 0;
            } else {
                rejected.observations[0].intent.channels = 3;
            }
            assert_eq!(runtime.complete(&mut service, Ok(rejected)).len(), 1);
            assert_eq!(service.statuses(&graph)[0].native_stamp, None);
            assert!(!service.statuses(&graph)[0].gate_open);
        }
    }

    #[test]
    fn unstamped_service_observation_retires_previous_raw_stamp() {
        let (graph, intent) = runtime_graph();
        let mut runtime = Runtime::default();
        let mut service = ExportService::default();
        service.set_available(true);
        assert!(
            runtime
                .complete(&mut service, Ok(observed(&intent, Some([1, 2, 3, 4, 5]))))
                .is_empty()
        );
        let mut observation = service.observations[&intent.endpoint_id].clone();
        observation.generation += 1;
        service.observe(observation).unwrap();
        assert_eq!(service.statuses(&graph)[0].native_stamp, None);
        assert!(!service.statuses(&graph)[0].gate_open);
    }
}
