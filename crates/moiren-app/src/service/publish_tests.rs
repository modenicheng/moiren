//! Tests for the token-gated snapshot publisher.
use super::measured_process::MeasuredDeliveryBound;
use super::*;

fn running_context() -> ServiceContext {
    let mut c = ServiceContext::default();
    let spec = SessionSpec {
        source: InputSelection::Physical {
            endpoint_id: "probe-input".into(),
        },
        output_endpoint_id: "probe-output".into(),
        gain: 1.0,
        pan: 0.0,
        limit: RunLimit::UntilStopped,
        max_block_frames: 64,
    };
    let generation = c.core.start(spec).expect("probe spec validates");
    c.core.managed_execution(generation, true, None);
    c
}

#[test]
fn snapshot_elapsed_is_quantized_to_whole_seconds() {
    let c = running_context();
    let first = snapshot(&c);
    assert_eq!(first.phase, SessionPhase::Running);
    assert_eq!(
        first.elapsed.subsec_nanos(),
        0,
        "sub-second precision would churn every publish while running"
    );
}

#[test]
fn same_second_snapshots_are_equal_for_publish_dedup() {
    let c = running_context();
    // Fresh graph Arcs must compare by content so an unchanged topology
    // keeps equality across snapshot builds.
    let first = snapshot(&c);
    let second = snapshot(&c);
    assert_eq!(first, second);
}

fn assert_published_change(before: AppSnapshot, after: AppSnapshot) {
    let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
    assert_ne!(before, after, "diagnostic changes must not be deduplicated");
    mailbox.publish(before);
    let first = handle.try_snapshot().unwrap();
    mailbox.publish(after.clone());
    let changed = handle.try_snapshot().unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    mailbox.publish(after);
    assert!(Arc::ptr_eq(&changed, &handle.try_snapshot().unwrap()));
}

#[test]
fn transport_and_measured_selection_changes_are_never_idle_deduplicated() {
    let initial = snapshot(&ServiceContext::default());
    let transport = TransportLatencySettings {
        process: Some(ExpertInputBuffer::default()),
        ..Default::default()
    };
    let measured = MeasuredProcessSummary {
        selection_id: 1,
        request: MeasuredProcessRequest {
            node: initial.graph.graph.nodes()[0].id(),
            session_generation: SessionGeneration(1),
            native_owner_generation: 1,
            graph_revision: 1,
            capacity_frames: 8192,
            target_frames: 2048,
            jitter_frames: 96,
        },
        process_pid: 42,
        process_creation_time_100ns: 123,
        delivery: MeasuredDeliveryBound {
            sample_rate: 48_000,
            max_packet_frames: 480,
            max_gap_frames: 480,
        },
        observed_window: None,
    };
    for field in 0..4 {
        let mut changed = initial.clone();
        match field {
            0 => changed.transport_latency = transport,
            1 => changed.active_transport_latency = Some(transport),
            2 => changed.measured_process = Some(measured.clone()),
            3 => changed.active_measured_process = Some(measured.clone()),
            _ => unreachable!(),
        }
        assert_published_change(initial.clone(), changed);
    }
}

#[test]
fn joined_cleanup_state_and_generation_changes_are_published() {
    let initial = snapshot(&ServiceContext::default());
    let mut previous = initial.clone();
    previous.last_cleanup = Some(SessionCleanupObservation {
        generation: SessionGeneration(1),
        state: moiren_engine::host::HostSnapshot {
            host: moiren_engine::host::HostState::Closed,
            execution: moiren_engine::host::ExecutionState::Dormant,
            desired_revision: 1,
            active_revision: None,
            plan_revision: None,
            owner_generation: 1,
            resource_generation: 1,
            timeline: None,
            update: moiren_engine::host::UpdateState::Idle,
            fault: None,
            cleanup_error: None,
        },
        #[cfg(windows)]
        process_trace: None,
        #[cfg(windows)]
        final_observations: None,
    });
    assert_published_change(initial, previous.clone());
    for field in 0..2 {
        let mut changed = previous.clone();
        let cleanup = changed.last_cleanup.as_mut().unwrap();
        match field {
            0 => cleanup.generation.0 += 1,
            1 => cleanup.state.host = moiren_engine::host::HostState::Closing,
            _ => unreachable!(),
        }
        assert_published_change(previous.clone(), changed);
    }
}

#[cfg(windows)]
#[test]
fn native_telemetry_and_trace_refusals_remain_visible_with_idle_dedup() {
    use moiren_windows_audio::virtual_owner::{
        VirtualCallbackOutcome, VirtualCallbackSample, VirtualCallbackSnapshot,
    };

    let initial = snapshot(&ServiceContext::default());
    let mut previous = initial.clone();
    previous.native_observations = Some(NativeSessionObservations {
        generation: SessionGeneration(1),
        observation: moiren_windows_audio::engine_backend::NativeObservationSnapshot {
            owner_generation: 1,
            running: true,
            streams: Vec::new(),
            bridges: Vec::new(),
            boundary_probes: Vec::new(),
            render_padding: Vec::new(),
            process_delivery: Vec::new(),
            virtual_clock: None,
            virtual_callback: None,
        },
    });
    assert_published_change(initial.clone(), previous.clone());

    let mut context = running_context();
    context.native_observations = previous.native_observations.clone();
    let before_callback = snapshot(&context);
    context
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .virtual_callback = Some(VirtualCallbackSnapshot::default());
    let mut enabled = snapshot(&context);
    // A quantized clock tick must not mask callback-only dedup regressions.
    enabled.elapsed = before_callback.elapsed;
    assert_published_change(before_callback, enabled.clone());

    // Only callback evidence changes: an in-flight attempt must remain
    // visible without inventing completion clocks or physical timing.
    context
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .virtual_callback = Some(VirtualCallbackSnapshot {
        attempts: 1,
        latest: Some(VirtualCallbackSample {
            attempt_sequence: 1,
            epoch_before: 1,
            frame_before: 256,
            epoch_after: None,
            frame_after: None,
            requested_frames: 1280,
            outcome: VirtualCallbackOutcome::Unfinished,
            qpc_before_100ns: None,
            qpc_after_100ns: None,
            duration_100ns: None,
        }),
        ..VirtualCallbackSnapshot::default()
    });
    let mut unfinished = snapshot(&context);
    unfinished.elapsed = enabled.elapsed;
    assert_published_change(enabled, unfinished.clone());

    let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
    mailbox.publish(unfinished);
    let published = handle.try_snapshot().unwrap();
    let json =
        serde_json::to_value(&published.native_observations.as_ref().unwrap().observation).unwrap();
    assert_eq!(
        json["virtual_callback"],
        serde_json::json!({
            "attempts": 1,
            "completed": 0,
            "errors": 0,
            "valid_timings": 0,
            "missing_clocks": 0,
            "regressed_clocks": 0,
            "max_duration_100ns": null,
            "saturated": false,
            "latest": {
                "attempt_sequence": 1,
                "epoch_before": 1,
                "frame_before": 256,
                "epoch_after": null,
                "frame_after": null,
                "requested_frames": 1280,
                "outcome": "unfinished",
                "qpc_before_100ns": null,
                "qpc_after_100ns": null,
                "duration_100ns": null,
            },
        })
    );

    let mut changed = previous.clone();
    changed
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .running = false;
    assert_published_change(previous, changed);
    let mut changed = initial.clone();
    changed.process_trace_status = Some(ProcessTraceCollectionStatus {
        origin: None,
        refusal: Some("history overwritten".into()),
    });
    assert_published_change(initial, changed);
}

fn probe_session() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "render".into(),
        gain: 1.0,
        pan: 0.0,
        limit: RunLimit::UntilStopped,
        max_block_frames: 256,
    }
}

/// Reports the context's snapshot-build counter every service round.
/// `is_drained == false` keeps the loop cycling on its 10 ms cadence
/// without any signal demand, so the test observes pure idle rounds.
struct BuildProbeDriver {
    builds: std::sync::mpsc::SyncSender<u64>,
    exiting: bool,
}
impl ServiceDriver for BuildProbeDriver {
    fn request(&mut self, c: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        // The probe applies exactly one domain change so the test can
        // watch a real content change propagate to the publication.
        if let AppRequest::SetLatencyProfile(profile) = request {
            c.latency_profile = profile;
        }
    }
    fn poll(&mut self, c: &mut ServiceContext, _: &mut JobPool, _: RoundBudget) {
        let _ = self
            .builds
            .try_send(c.snapshot_builds.load(std::sync::atomic::Ordering::Relaxed));
    }
    fn begin_exit(&mut self, _: &mut ServiceContext, _: &mut JobPool) {
        self.exiting = true;
    }
    fn is_drained(&self) -> bool {
        // Undrained keeps the loop cycling on its 10 ms cadence without
        // demand; exit must still be able to retire the service.
        self.exiting
    }
}

fn collect_rounds(
    receiver: &std::sync::mpsc::Receiver<u64>,
    count: usize,
    timeout: Duration,
) -> Vec<u64> {
    let deadline = std::time::Instant::now() + timeout;
    let mut rounds = Vec::with_capacity(count);
    while rounds.len() < count {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .unwrap_or_default();
        assert!(
            !remaining.is_zero(),
            "service loop stopped reporting rounds: {rounds:?}"
        );
        match receiver.recv_timeout(remaining) {
            Ok(builds) => rounds.push(builds),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("no round report within budget: {rounds:?}")
            }
            Err(error) => panic!("service loop disconnected: {error}: {rounds:?}"),
        }
    }
    rounds
}

/// B-1 regression: rounds whose inputs did not change must not rebuild
/// (and republish) the snapshot at all; a real change must rebuild and
/// reach the published snapshot.
#[test]
fn service_loop_skips_snapshot_rebuilds_while_inputs_unchanged() {
    let (builds_tx, builds_rx) = std::sync::mpsc::sync_channel(256);
    let runtime = ServiceRuntime::spawn_with_driver(BuildProbeDriver {
        builds: builds_tx,
        exiting: false,
    })
    .unwrap();
    let handle = runtime.handle();
    // Steady run: every report carries the same build count. The old
    // build-and-compare loop incremented it on every round.
    let steady = collect_rounds(&builds_rx, 10, Duration::from_secs(5));
    let baseline = steady[0];
    assert!(
        steady.iter().all(|&builds| builds == baseline),
        "unchanged rounds must skip snapshot rebuilds: {steady:?}"
    );
    // A real change: the request round rebuilds exactly once and the
    // published snapshot carries it to the UI.
    handle
        .try_request(AppRequest::SetLatencyProfile(AudioLatencyProfile::Low))
        .unwrap();
    let mut rebuilt = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rebuilt.is_none() {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .unwrap_or_default();
        assert!(!remaining.is_zero(), "the edit never rebuilt the snapshot");
        match builds_rx.recv_timeout(remaining) {
            Ok(builds) if builds > baseline => rebuilt = Some(builds),
            Ok(_) => continue,
            Err(error) => panic!("service loop disconnected: {error}"),
        }
    }
    let rebuilt = rebuilt.expect("edit rebuild observed");
    // The round's publish lands right after the poll report above, so
    // wait for it instead of racing the snapshot slot read.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the rebuilt snapshot never reached the UI"
        );
        if handle.try_snapshot().unwrap().latency_profile == AudioLatencyProfile::Low {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // And the loop falls silent again until the next real change.
    let steady = collect_rounds(&builds_rx, 10, Duration::from_secs(5));
    assert!(
        steady.iter().all(|&builds| builds == rebuilt),
        "rounds after the change must skip snapshot rebuilds again: {steady:?}"
    );
    crate::ui::finish_service(runtime);
}

/// Every tokened domain must move the revision exactly when its content
/// can change the published snapshot; idempotent re-projections must not
/// churn it (the hot per-round paths depend on that).
#[test]
fn snapshot_revision_tracks_every_tokened_domain() {
    let mut c = ServiceContext::default();
    let initial = snapshot_revision(&c);
    // A projection that changes nothing (wrong generation) is inert.
    c.core.managed_execution(SessionGeneration(7), true, None);
    assert_eq!(
        snapshot_revision(&c),
        initial,
        "an inert projection must not churn any token"
    );

    // Graph domain: an accepted edit advances the candidate revision.
    let before = snapshot_revision(&c);
    let gain = primary_node_of(
        &c.graph.snapshot_ref().graph,
        moiren_core::graph::NodeKind::Gain,
    )
    .expect("fixed chain gain")
    .id();
    c.graph
        .edit(GraphEdit::SetGain {
            node: gain,
            gain: 0.5,
        })
        .expect("gain edit validates");
    assert_ne!(snapshot_revision(&c), before, "graph edit must bump");

    // Core domain: desired intent and execution projection.
    let before = snapshot_revision(&c);
    c.core
        .managed_desired(probe_session())
        .expect("spec validates");
    assert_ne!(snapshot_revision(&c), before, "desired change must bump");
    let generation = c.core.generation();
    c.core.managed_execution(generation, false, None);
    let starting = snapshot_revision(&c);
    // The driver re-projects the same execution facts every round.
    c.core.managed_execution(generation, false, None);
    assert_eq!(
        snapshot_revision(&c),
        starting,
        "steady re-projection must not churn the core token"
    );
    c.core.managed_execution(generation, true, None);
    assert_ne!(
        snapshot_revision(&c),
        starting,
        "running transition must bump"
    );

    // Exports domain: availability drives every projected resource state.
    let before = snapshot_revision(&c);
    c.exports.set_available(true);
    assert_ne!(snapshot_revision(&c), before, "availability flip must bump");
    let available = snapshot_revision(&c);
    c.exports.set_available(true);
    assert_eq!(
        snapshot_revision(&c),
        available,
        "idempotent availability must not churn"
    );
    c.exports
        .set_authorizations(&[ExportAuthorization {
            endpoint_id: moiren_engine::node::VirtualEndpointId::from_bytes([0x11; 16]),
            allow_default: true,
        }])
        .expect("authorization validates");
    assert_ne!(
        snapshot_revision(&c),
        available,
        "authorization change must bump"
    );

    // Catalog domain: content-equal refresh keeps the token, a real
    // refresh moves it.
    let before = snapshot_revision(&c);
    c.replace_catalog(DeviceCatalog::default());
    assert_eq!(
        snapshot_revision(&c),
        before,
        "content-equal catalog refresh must not churn"
    );
    let catalog = DeviceCatalog {
        default_output_endpoint_id: Some("probe-output".into()),
        ..DeviceCatalog::default()
    };
    c.replace_catalog(catalog);
    assert_ne!(
        snapshot_revision(&c),
        before,
        "catalog content change must bump"
    );
}

/// The directly compared snapshot inputs must decide exactly like a full
/// rebuild followed by the publish dedup: `small_snapshot_inputs_match`
/// against the published snapshot must agree with `snapshot(&c) ==
/// published`, including the freshness filters applied at publication.
#[test]
fn small_snapshot_inputs_match_tracks_directly_compared_fields() {
    let mut c = ServiceContext::default();
    let published = snapshot(&c);
    let oracle = |c: &ServiceContext, published: &AppSnapshot| {
        small_snapshot_inputs_match(c, published) == (snapshot(c) == *published)
    };
    assert!(oracle(&c, &published), "unchanged context: both agree");

    c.latency_profile = AudioLatencyProfile::Low;
    assert!(!small_snapshot_inputs_match(&c, &published));
    assert!(oracle(&c, &published), "latency profile: both agree");
    let published = snapshot(&c);

    c.control.pending_gain_pan = Some((0.5, 0.0));
    assert!(!small_snapshot_inputs_match(&c, &published));
    assert!(oracle(&c, &published), "control: both agree");
    let published = snapshot(&c);

    c.audio_settings.processing.block_frames = 128;
    assert!(!small_snapshot_inputs_match(&c, &published));
    assert!(oracle(&c, &published), "audio settings: both agree");
    let published = snapshot(&c);

    c.catalog_error = Some("probe failure".into());
    assert!(!small_snapshot_inputs_match(&c, &published));
    assert!(oracle(&c, &published), "catalog error: both agree");
    let published = snapshot(&c);

    c.tone_session = true;
    assert!(!small_snapshot_inputs_match(&c, &published));
    assert!(oracle(&c, &published), "tone session: both agree");

    // The active-settings publication applies the same freshness filter
    // as `snapshot`, so both views must agree through the lifecycle.
    c.core
        .managed_desired(probe_session())
        .expect("spec validates");
    let generation = c.core.generation();
    c.core.managed_execution(generation, true, None);
    assert!(oracle(&c, &published), "running, no active settings: agree");
    c.active_audio_settings = Some(c.audio_settings.clone());
    assert!(!small_snapshot_inputs_match(&c, &published));
    assert!(oracle(&c, &published), "fresh active settings: agree");
    let published = snapshot(&c);
    assert!(small_snapshot_inputs_match(&c, &published));
    // Advancing the intent generation retires the active-settings slot:
    // the published snapshot filters it as stale.
    c.core.start(probe_session()).expect("spec validates");
    assert!(
        oracle(&c, &published),
        "stale active settings filtered: agree"
    );
}
