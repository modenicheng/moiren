use super::*;
use moiren_windows_audio::vad::{
    Channels, Intent, PdoState, StreamState, VadExportObservation, VadPcmTelemetrySnapshot,
};

fn telemetry() -> VadPcmTelemetrySnapshot {
    VadPcmTelemetrySnapshot {
        schema_version: 1,
        packet_frames: 128,
        max_packet_frames: 256,
        submission_attempts: u64::MAX,
        submitted_packets: (1 << 53) + 1,
        completed_packets: (1 << 32) + 3,
        completion_gaps: 7,
        max_completion_gap_100ns: 123_456,
        completion_clock_regressions: 11,
        dropped_packets: 13,
        driver_dropped_packets: 17,
        quota_dropped_packets: 19,
        carrier_dropped_packets: 23,
        stale_packets: 29,
        silence_packets: 31,
        silence_frames: (1 << 53) + 37,
        epoch_resets: 41,
        recovery_events: 43,
    }
}

fn provider_view(intent: &ExportIntent) -> VadExportObservation {
    VadExportObservation {
        endpoint_id: intent.endpoint_id,
        spec_name: intent.name.clone(),
        intent: Intent::Playback,
        channels: Channels::try_from(intent.channels as u32).unwrap(),
        allow_default: false,
        snapshot: Some(native_snapshot(
            PdoState::Present,
            StreamState::Closed,
            true,
            true,
        )),
        removing: false,
        transport_prepared: true,
        telemetry: Some(telemetry()),
        error: None,
    }
}

fn complete_native(
    runtime: &mut Runtime,
    service: &mut ExportService,
    native: NativeObservation,
) -> Vec<String> {
    runtime.complete(
        service,
        Ok(Batch {
            observations: vec![native],
            errors: vec![],
        }),
    )
}

#[test]
fn provider_mapping_preserves_telemetry_transport_state_and_endpoint_error() {
    let (_, intent) = runtime_graph();
    for prepared in [false, true] {
        let mut view = provider_view(&intent);
        view.transport_prepared = prepared;
        view.error = Some("endpoint refused PCM preparation: packet quota".into());
        let native = NativeObservation::from(view);
        assert_eq!(native.resource, ExportResourceState::Present);
        assert_eq!(
            native.error.as_deref(),
            Some("endpoint refused PCM preparation: packet quota")
        );
        let diagnostics = native
            .diagnostics
            .expect("provider diagnostics were discarded");
        assert_eq!(diagnostics.telemetry, Some(telemetry()));
        assert_eq!(diagnostics.transport_prepared, prepared);
        assert_eq!(diagnostics.error, native.error);
    }
}

#[test]
fn complete_binds_real_diagnostics_to_accepted_generation_and_exact_stamp() {
    let (graph, intent) = runtime_graph();
    let adapter = Arc::new(FakeAdapter::default());
    let mut runtime = Runtime {
        adapter: Some(adapter.clone()),
        sequence: 40,
        ..Default::default()
    };
    let mut service = ExportService::default();
    service.set_available(true);
    for (index, stamp) in [
        [1, 7, 1, 0, 5],
        [u64::MAX, (1 << 53) + 1, 3, 4, 6],
        [2, 8, 4, 5, 7],
    ]
    .into_iter()
    .enumerate()
    {
        let mut native = NativeObservation::from(provider_view(&intent));
        native.snapshot = Some(stamp);
        native.can_open = false;
        if let Some(diagnostics) = native.diagnostics.as_mut() {
            // The adapter cannot supply lifecycle authority or another stamp.
            diagnostics.generation = u64::MAX;
            diagnostics.native_stamp = Some([99; 5]);
        }
        assert!(complete_native(&mut runtime, &mut service, native).is_empty());
        runtime.project(
            &mut service,
            &graph,
            std::slice::from_ref(&intent),
            Some([1, 1, 1, graph.candidate_revision, 1]),
        );
        for _ in 0..2 {
            let status = &service.statuses(&graph)[0];
            let diagnostics = status.diagnostics.as_ref().expect("missing diagnostics");
            assert_eq!(diagnostics.telemetry, Some(telemetry()));
            assert!(diagnostics.transport_prepared);
            assert_eq!(diagnostics.native_stamp, Some(stamp));
            assert_eq!(diagnostics.generation, 41 + index as u64);
            assert_eq!(
                diagnostics.generation,
                service.observations[&intent.endpoint_id].generation
            );
            assert_eq!(diagnostics.error, None);
            assert!(!status.gate_open);
            assert_eq!(status.active_revision, None);
        }
        assert!(!runtime.ready_for(&service, std::slice::from_ref(&intent)));
        assert!(
            runtime
                .work(&service, &graph, false)
                .unwrap()
                .actions
                .is_empty()
        );
        assert_eq!(runtime.stamps[&intent.endpoint_id], stamp);
    }
    assert!(adapter.calls.lock().unwrap().is_empty());
}

#[test]
fn endpoint_error_survives_present_status_without_granting_readiness() {
    let (mut graph, intent) = runtime_graph();
    let mut other = intent.clone();
    other.endpoint_id = VirtualEndpointId::from_bytes([0x72; 16]);
    other.name = "Other endpoint".into();
    other.binding = None;
    graph.exports.push(other.clone());
    let mut runtime = Runtime {
        adapter: Some(Arc::new(FakeAdapter::default())),
        ..Default::default()
    };
    let mut service = ExportService::default();
    service.set_available(true);
    let refusal = "playback endpoint refused preparation: packet quota";
    let mut view = provider_view(&intent);
    view.error = Some(refusal.into());
    let mut failed = NativeObservation::from(view);
    failed.can_open = true;
    let errors = runtime.complete(
        &mut service,
        Ok(Batch {
            observations: vec![failed, NativeObservation::from(provider_view(&other))],
            errors: vec!["existing global failure".into()],
        }),
    );
    assert_eq!(errors, ["existing global failure", refusal]);
    runtime.project(
        &mut service,
        &graph,
        std::slice::from_ref(&intent),
        Some([1, 1, 1, graph.candidate_revision, 1]),
    );
    let statuses = service.statuses(&graph);
    let status = statuses
        .iter()
        .find(|s| s.intent.endpoint_id == intent.endpoint_id)
        .unwrap();
    assert_eq!(status.resource, ExportResourceState::Present);
    let diagnostics = status.diagnostics.as_ref().expect("missing refusal reason");
    assert_eq!(diagnostics.error.as_deref(), Some(refusal));
    assert_eq!(diagnostics.telemetry, Some(telemetry()));
    assert!(!status.gate_open);
    assert_eq!(status.active_revision, None);
    assert!(!runtime.lifecycle_ready_for(&service, std::slice::from_ref(&intent)));
    assert!(!runtime.ready_for(&service, std::slice::from_ref(&intent)));
    let other_status = statuses
        .iter()
        .find(|s| s.intent.endpoint_id == other.endpoint_id)
        .unwrap();
    assert_eq!(other_status.diagnostics.as_ref().unwrap().error, None);
}

#[test]
fn replacement_polls_clear_old_diagnostic_evidence_and_allow_fresh_recovery() {
    let (graph, intent) = runtime_graph();
    for case in [
        "missing endpoint",
        "missing diagnostics",
        "missing telemetry",
        "missing stamp",
        "confirmed removal",
        "unavailable endpoint",
        "failed poll",
        "service unavailable",
    ] {
        let mut runtime = Runtime::default();
        let mut service = ExportService::default();
        service.set_available(true);
        assert!(
            complete_native(
                &mut runtime,
                &mut service,
                NativeObservation::from(provider_view(&intent)),
            )
            .is_empty()
        );
        let previous = service.statuses(&graph)[0].diagnostics.clone().expect(case);
        let mut native = NativeObservation::from(provider_view(&intent));
        match case {
            "missing endpoint" => {
                runtime.complete(
                    &mut service,
                    Ok(Batch {
                        observations: vec![],
                        errors: vec![],
                    }),
                );
            }
            "failed poll" => {
                assert_eq!(
                    runtime.complete(&mut service, Err("lifecycle worker failed".into())),
                    ["lifecycle worker failed"]
                );
            }
            "service unavailable" => service.set_available(false),
            _ => {
                match case {
                    "missing diagnostics" => native.diagnostics = None,
                    "missing telemetry" => native.diagnostics.as_mut().unwrap().telemetry = None,
                    "missing stamp" => native.snapshot = None,
                    "confirmed removal" => native.resource = ExportResourceState::Absent,
                    "unavailable endpoint" => native.resource = ExportResourceState::Unavailable,
                    _ => unreachable!(),
                }
                complete_native(&mut runtime, &mut service, native);
            }
        }
        let statuses = service.statuses(&graph);
        let diagnostics = statuses[0].diagnostics.as_ref();
        if case == "missing telemetry" {
            let diagnostics = diagnostics.expect("fresh transport diagnostic is still available");
            assert_eq!(diagnostics.telemetry, None);
            assert!(diagnostics.generation > previous.generation);
        } else if case == "missing stamp" {
            let diagnostics = diagnostics.expect("uncertain CREATE can still carry a refusal");
            assert_eq!(diagnostics.native_stamp, None);
            assert!(diagnostics.generation > previous.generation);
        } else {
            assert_eq!(diagnostics, None, "{case}");
        }
        if case == "confirmed removal" {
            assert!(!service.observations.contains_key(&intent.endpoint_id));
            assert!(service.diagnostics.is_empty());
        }
        service.set_available(true);
        assert!(
            complete_native(
                &mut runtime,
                &mut service,
                NativeObservation::from(provider_view(&intent))
            )
            .is_empty()
        );
        let diagnostics = service.statuses(&graph)[0].diagnostics.clone().unwrap();
        assert_eq!(diagnostics.telemetry, Some(telemetry()));
        assert!(diagnostics.generation > previous.generation, "{case}");
    }
}

#[test]
fn rejected_runtime_observations_cannot_publish_diagnostics() {
    let (graph, intent) = runtime_graph();
    for case in ["stale generation", "invalid intent", "unavailable service"] {
        let mut runtime = Runtime::default();
        let mut service = ExportService::default();
        service.set_available(true);
        complete_native(
            &mut runtime,
            &mut service,
            NativeObservation::from(provider_view(&intent)),
        );
        assert!(service.statuses(&graph)[0].diagnostics.is_some());
        let mut rejected = NativeObservation::from(provider_view(&intent));
        rejected.snapshot = Some([99; 5]);
        match case {
            "stale generation" => runtime.sequence = 0,
            "invalid intent" => rejected.intent.channels = 3,
            "unavailable service" => service.set_available(false),
            _ => unreachable!(),
        }
        assert_eq!(
            complete_native(&mut runtime, &mut service, rejected).len(),
            1
        );
        assert_eq!(service.statuses(&graph)[0].diagnostics, None, "{case}");
    }
}

#[test]
fn service_observation_replacement_clears_diagnostics_only_after_acceptance() {
    let (graph, intent) = runtime_graph();
    let mut runtime = Runtime::default();
    let mut service = ExportService::default();
    service.set_available(true);
    complete_native(
        &mut runtime,
        &mut service,
        NativeObservation::from(provider_view(&intent)),
    );
    let previous = service.statuses(&graph)[0].diagnostics.clone().unwrap();
    let mut observation = service.observations[&intent.endpoint_id].clone();
    assert!(service.observe(observation.clone()).is_err());
    assert_eq!(
        service.statuses(&graph)[0].diagnostics.as_ref(),
        Some(&previous)
    );
    observation.generation += 1;
    service.observe(observation).unwrap();
    assert_eq!(service.statuses(&graph)[0].diagnostics, None);
}
