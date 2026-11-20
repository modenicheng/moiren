//! Tests for the token-gated snapshot publisher.
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

fn probe_session() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "render".into(),
        gain: 1.0,
        pan: 0.0,
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
