//! Actual host lifecycle with explicit software-only candidate construction.
use super::*;
use crate::service::{
    EvaluatedProcessCandidate,
    process_trace::{candidate::software_candidate, tests::evidence},
};

fn stopped(r: &mut Rig) -> EvaluatedProcessCandidate {
    r.request(AppRequest::Start(SessionSpec {
        source: InputSelection::Process {
            pid: 42,
            creation_time_100ns: 123,
        },
        output_endpoint_id: "output".into(),
        limit: RunLimit::UntilStopped,
        gain: 1.,
        pan: 0.,
        max_block_frames: 256,
    }));
    r.wait(|r| r.c.core.phase() == SessionPhase::Running && r.driver.confirm_parameters.is_none());
    let owner = r.driver.owner.as_ref().unwrap();
    let generation = owner.generation;
    let native = owner.observer.read().state.owner_generation;
    let revision = owner.active_revision.1;
    let document = owner.document;
    let node = r.node(NodeKind::Source);
    r.request(AppRequest::Stop);
    r.wait(|r| r.driver.cleaning == 0 && r.c.core.phase() == SessionPhase::Idle);
    let mut evidence = evidence(node);
    evidence.origin.generation = generation;
    evidence.origin.document = document;
    evidence.origin.native_owner_generation = native;
    evidence.origin.graph_revision = revision;
    evidence.origin.audio_settings = r.c.audio_settings.clone();
    // Software host generations start at1; rebuild structural test labels only.
    evidence.artifact.owner_generation = native;
    evidence.trace = Some(
        evidence
            .artifact
            .trace(&evidence.origin.process, native)
            .unwrap(),
    );
    let mut observation = crate::service::measured_process::tests::observation(node);
    observation.owner_generation = native;
    evidence.measurement = observation
        .measured(&observation.process, native)
        .map_err(|error| error.to_string());
    let evidence = Arc::new(evidence);
    r.c.last_cleanup.as_mut().unwrap().process_trace = Some(evidence.clone());
    software_candidate(evidence)
}

#[test]
fn retained_apply_reopens_stopped_host_in_both_precisions_and_preserves_original_request() {
    for precision in [ProcessingPrecision::F32, ProcessingPrecision::F64] {
        let mut r = Rig::new();
        r.driver
            .set_measured_test_factory(FakeBackend(r.state.clone()));
        let mut audio = AudioSettings::default();
        audio.processing.precision = precision;
        r.request(AppRequest::SetAudioSettings(audio));
        let candidate = stopped(&mut r);
        let original = candidate.request();
        r.request(AppRequest::ApplyEvaluatedProcessBridge(candidate));
        let selected = r.c.measured_process.as_ref().unwrap().summary().clone();
        assert_eq!(selected.request, original);
        r.wait(|r| r.c.active_measured_process.as_ref() == Some(&selected));
        assert_ne!(r.c.core.generation(), original.session_generation);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn retained_apply_rechecks_settings_before_open_and_refuses_stale_document_or_predecessor() {
    for fault in 0..5 {
        let mut r = Rig::new();
        let backend = FakeBackend(r.state.clone());
        r.driver.factory = Arc::new(move |_, _, _, _| Ok(Box::new(backend.clone())));
        let candidate = stopped(&mut r);
        match fault {
            0 => {
                r.driver.document += 1;
            }
            1 => {
                r.c.last_cleanup.as_mut().unwrap().process_trace = None;
            }
            2 => {
                r.c.audio_settings.processing.block_frames = 512;
            }
            _ => {}
        }
        r.request(AppRequest::ApplyEvaluatedProcessBridge(candidate));
        if fault >= 3 {
            r.c.audio_settings.processing.block_frames = 512;
        }
        if fault == 4 {
            r.request(AppRequest::ApplyGraph);
        }
        for _ in 0..5 {
            r.tick();
        }
        assert_eq!(
            r.state.starts.load(Ordering::SeqCst),
            1,
            "fault {fault} must not open a new owner"
        );
        assert!(r.c.active_measured_process.is_none());
        assert!(r.c.graph.snapshot().last_error.is_some());
    }
}

#[test]
fn retained_apply_waits_for_cleanup_and_completion_credits_without_losing_idle_admission() {
    for cleanup_credit in [true, false] {
        let mut r = Rig::new();
        r.driver
            .set_measured_test_factory(FakeBackend(r.state.clone()));
        let candidate = stopped(&mut r);
        let mut cleanups = Vec::new();
        let mut completions = Vec::new();
        if cleanup_credit {
            while let Ok(permit) = r.c.try_reserve_cleanup() {
                cleanups.push(permit);
            }
        } else {
            let port = r.driver.queue().port();
            while let Ok(permit) = port.try_reserve() {
                completions.push(permit);
            }
        }
        r.request(AppRequest::ApplyEvaluatedProcessBridge(candidate));
        r.tick();
        assert_eq!(
            r.c.core.phase(),
            SessionPhase::Idle,
            "no intent transition before cleanup/completion credits exist"
        );
        assert!(r.driver.apply);
        drop(cleanups);
        drop(completions);
        r.wait(|r| r.c.core.phase() == SessionPhase::Running);
        assert_eq!(r.state.starts.load(Ordering::SeqCst), 2);
        r.request(AppRequest::Stop);
        r.wait(|r| r.driver.is_drained());
    }
}

#[test]
fn retained_apply_refuses_captured_settings_even_after_current_settings_mutate_back() {
    for kind in 0..3 {
        let mut r = Rig::new();
        let backend = FakeBackend(r.state.clone());
        r.driver.factory = Arc::new(move |_, _, _, _| Ok(Box::new(backend.clone())));
        let candidate = stopped(&mut r);
        r.request(AppRequest::ApplyEvaluatedProcessBridge(candidate));
        let audio = r.c.audio_settings.clone();
        let transport = r.c.transport_latency;
        let profile = r.c.latency_profile;
        match kind {
            0 => {
                r.c.audio_settings.processing.precision = match audio.processing.precision {
                    ProcessingPrecision::F32 => ProcessingPrecision::F64,
                    ProcessingPrecision::F64 => ProcessingPrecision::F32,
                }
            }
            1 => r.c.transport_latency = TransportLatencySettings::default(),
            _ => r.c.latency_profile = AudioLatencyProfile::Low,
        }
        // Transport starts at default, so change one editable bound explicitly.
        if kind == 1 {
            r.c.transport_latency.process = Some(crate::service::ExpertInputBuffer {
                capacity_frames: 4096,
                target_frames: 512,
                jitter_frames: 96,
                max_packet_frames: 96,
                max_gap_frames: 96,
                age_limit_100ns: None,
                provenance: crate::service::DeliveryProvenance::Declared,
            });
        }
        r.request(AppRequest::ApplyGraph);
        r.c.audio_settings = audio;
        r.c.transport_latency = transport;
        r.c.latency_profile = profile;
        for _ in 0..5 {
            r.tick();
        }
        assert_eq!(
            r.state.starts.load(Ordering::SeqCst),
            1,
            "captured mismatch {kind}"
        );
        assert!(r.c.graph.snapshot().last_error.is_some());
    }
}
