use super::*;
use crate::service::{
    AppRequest, BuiltinState, GraphEdit, InputSelection, ManagedDriver, RunLimit, ServiceRuntime,
    SessionSpec,
    test_backend::{FakeBackend, State},
};
use moiren_engine::spectrum::SpectrumSettings;
use std::sync::atomic::Ordering;

struct Rig {
    runtime: Option<ServiceRuntime>,
    handle: AppHandle,
    backend: Arc<State>,
    node: NodeId,
}

impl Rig {
    fn new() -> Self {
        let backend = FakeBackend::default();
        let state = backend.0.clone();
        state.timed_meter_frames.store(64, Ordering::Release);
        let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
            Ok(Box::new(backend.clone()))
        }))
        .unwrap();
        let handle = runtime.handle();
        let mut rig = Self {
            runtime: Some(runtime),
            handle,
            backend: state,
            node: crate::service::ServiceGraph::default().snapshot().nodes[0].id,
        };
        rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
            kind: NodeKind::SpectrumAnalyzer,
            channels: 2,
        }));
        let graph = rig.snapshot().graph;
        let node = graph
            .node_of_kind(NodeKind::SpectrumAnalyzer)
            .unwrap()
            .0
            .id();
        let source = graph.node_of_kind(NodeKind::Source).unwrap().0.outputs()[0].id();
        let destination = graph.graph.get_node(node).unwrap().inputs()[0].id();
        rig.node = node;
        rig.request(AppRequest::EditGraph(GraphEdit::Connect {
            source,
            destination,
        }));
        rig.request(AppRequest::EditGraph(GraphEdit::SetBuiltin {
            node,
            state: BuiltinState::SpectrumAnalyzer(SpectrumSettings {
                fft_size: 64,
                hop_frames: 32,
            }),
        }));
        rig
    }

    fn snapshot(&self) -> AppSnapshot {
        (*crate::ui::wait_for(Duration::from_secs(2), || self.handle.try_snapshot())).clone()
    }

    fn request(&mut self, request: AppRequest) {
        let request = self.handle.try_request_tracked(request).unwrap();
        self.wait(|rig| rig.snapshot().consumed_request_id >= request);
    }

    fn tick(&mut self) {
        std::thread::sleep(Duration::from_millis(2));
    }

    fn wait(&mut self, mut predicate: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !predicate(self) {
            assert!(
                Instant::now() < deadline,
                "spectrum condition unmet: {:?}",
                self.snapshot()
            );
            self.tick();
        }
    }

    fn start(&mut self) {
        self.request(AppRequest::Start(SessionSpec {
            source: InputSelection::Tone { frequency_hz: 440. },
            output_endpoint_id: "output".into(),
            limit: RunLimit::UntilStopped,
            gain: 1.,
            pan: 0.,
            max_block_frames: 256,
        }));
        self.wait(|rig| {
            rig.snapshot().phase == SessionPhase::Running
                && rig
                    .snapshot()
                    .signal_topics
                    .node_spectra
                    .contains_key(&rig.node)
        });
    }

    fn live(&mut self, spectra: &mut NodeSpectra) {
        self.wait(|rig| {
            let snapshot = rig.snapshot();
            spectra.update(&snapshot, &BTreeSet::from([rig.node]));
            spectra.status(rig.node, &snapshot) == SpectrumStatus::Live
        });
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.handle.request_exit();
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.join();
        }
    }
}

#[test]
fn spectrum_visible_subscription_delivers_real_channel_major_fft() {
    let mut rig = Rig::new();
    let mut spectra = NodeSpectra::new(rig.handle.clone());
    let visible = BTreeSet::from([rig.node]);
    spectra.update(&rig.snapshot(), &visible);
    assert_eq!(
        spectra.status(rig.node, &rig.snapshot()),
        SpectrumStatus::Stopped
    );
    rig.start();
    let binding = rig.snapshot().signal_topics.node_spectra[&rig.node].clone();
    assert_eq!(binding.topic.business_subscribers(), 0);
    rig.live(&mut spectra);
    assert_eq!(binding.topic.business_subscribers(), 1);
    let observation = spectra.observation(rig.node).expect("actual FFT block");
    assert_eq!(observation.window.channels, 2);
    assert_eq!(observation.window.fft_size, 64);
    assert_eq!(observation.window.sample_rate, 48_000.);
    assert_eq!(
        observation.window.frame_end - observation.window.frame_start,
        64
    );
    assert_eq!(observation.magnitudes.len(), 66);
    for channel in observation.magnitudes.as_chunks::<33>().0 {
        assert!(
            (channel[0] - 1.).abs() < 0.00001,
            "real ConstantSource DC, not UI synthesis"
        );
        assert!(channel[2..].iter().all(|value| value.abs() < 0.00001));
    }
}

#[test]
fn spectrum_hide_is_immediate_and_grace_releases_only_ui_demand() {
    let mut rig = Rig::new();
    rig.start();
    let mut spectra = NodeSpectra::new(rig.handle.clone());
    rig.live(&mut spectra);
    let snapshot = rig.snapshot();
    let topic = snapshot.signal_topics.node_spectra[&rig.node].topic.clone();
    let mut ticket = rig
        .handle
        .subscribe_topic(topic.clone(), Default::default())
        .unwrap();
    let mut analysis = None;
    rig.wait(|_| {
        analysis = ticket.try_take().unwrap();
        analysis.is_some()
    });
    assert_eq!(topic.business_subscribers(), 2);
    let now = Instant::now();
    assert!(spectra.update_at(&snapshot, &BTreeSet::new(), now));
    assert_eq!(spectra.status(rig.node, &snapshot), SpectrumStatus::Hidden);
    assert!(spectra.observation(rig.node).is_none());
    assert_eq!(
        topic.business_subscribers(),
        2,
        "brief viewport changes keep the subscription"
    );
    spectra.update_at(
        &snapshot,
        &BTreeSet::new(),
        now + SUBSCRIPTION_GRACE + Duration::from_millis(1),
    );
    assert_eq!(
        topic.business_subscribers(),
        1,
        "only the hidden UI demand is released"
    );
    drop(spectra);
    drop(analysis);
    assert_eq!(topic.business_subscribers(), 0);
}

#[test]
fn spectrum_retained_window_is_not_live_after_hide_show_or_owner_change() {
    let mut rig = Rig::new();
    rig.start();
    let mut spectra = NodeSpectra::new(rig.handle.clone());
    rig.live(&mut spectra);
    rig.backend.pause_audio.store(true, Ordering::Release);
    let visible = BTreeSet::from([rig.node]);
    for _ in 0..8 {
        rig.tick();
        spectra.update(&rig.snapshot(), &visible);
    }
    let snapshot = rig.snapshot();
    let now = Instant::now();
    spectra.update_at(&snapshot, &BTreeSet::new(), now);
    spectra.update_at(&snapshot, &visible, now + Duration::from_millis(1));
    assert_eq!(spectra.status(rig.node, &snapshot), SpectrumStatus::Waiting);
    assert!(spectra.observation(rig.node).is_none());
    rig.backend.pause_audio.store(false, Ordering::Release);
    rig.live(&mut spectra);
    rig.backend.pause_audio.store(true, Ordering::Release);
    for _ in 0..8 {
        rig.tick();
        spectra.update(&rig.snapshot(), &visible);
    }
    let snapshot = rig.snapshot();
    for identity in 0..3 {
        let mut changed = snapshot.clone();
        match identity {
            0 => {
                changed
                    .signal_topics
                    .node_spectra
                    .get_mut(&rig.node)
                    .unwrap()
                    .owner_generation += 1
            }
            1 => {
                changed
                    .signal_topics
                    .node_spectra
                    .get_mut(&rig.node)
                    .unwrap()
                    .settings
                    .hop_frames = 16
            }
            _ => changed.generation.0 += 1,
        }
        assert_ne!(
            spectra.status(rig.node, &changed),
            SpectrumStatus::Live,
            "status must fence snapshot identity before the next poll"
        );
        spectra.update(&changed, &visible);
        assert!(spectra.observation(rig.node).is_none());
        assert_ne!(spectra.status(rig.node, &changed), SpectrumStatus::Live);
    }
}

#[test]
fn spectrum_stale_stop_and_removal_clear_observations() {
    let mut rig = Rig::new();
    rig.start();
    let mut spectra = NodeSpectra::new(rig.handle.clone());
    rig.live(&mut spectra);
    rig.backend.pause_audio.store(true, Ordering::Release);
    let visible = BTreeSet::from([rig.node]);
    for _ in 0..8 {
        rig.tick();
        spectra.update(&rig.snapshot(), &visible);
    }
    let snapshot = rig.snapshot();
    let now = Instant::now();
    assert!(spectra.update_at(
        &snapshot,
        &visible,
        now + STALE_AFTER + Duration::from_millis(1)
    ));
    assert_eq!(spectra.status(rig.node, &snapshot), SpectrumStatus::Stale);
    assert!(spectra.observation(rig.node).is_none());
    let topic = snapshot.signal_topics.node_spectra[&rig.node].topic.clone();
    let mut stopped = snapshot.clone();
    stopped.phase = SessionPhase::Idle;
    spectra.update(&stopped, &visible);
    assert_eq!(spectra.status(rig.node, &stopped), SpectrumStatus::Stopped);
    assert_eq!(topic.business_subscribers(), 0);
    let mut removed = snapshot;
    removed.signal_topics.node_spectra.remove(&rig.node);
    spectra.update(&removed, &visible);
    assert_eq!(
        spectra.status(rig.node, &removed),
        SpectrumStatus::Unavailable
    );
    assert!(spectra.observation(rig.node).is_none());
}

#[test]
fn spectrum_window_validation_accepts_overlap_but_rejects_corruption_and_rewinds() {
    let settings = SpectrumSettings {
        fft_size: 64,
        hop_frames: 32,
    };
    let header = SpectrumWindow {
        timeline_epoch: 3,
        frame_start: 32,
        frame_end: 96,
        sample_rate: 48_000.,
        fft_size: 64,
        channels: 2,
    };
    let data = vec![0.; 66];
    let previous = SpectrumWindow {
        frame_start: 0,
        frame_end: 64,
        ..header
    };
    assert!(
        valid_window(settings, 2, header, &data, Some(previous)),
        "FFT windows intentionally overlap"
    );
    assert!(!valid_window(settings, 2, header, &data[..65], None));
    assert!(
        !valid_window(settings, 2, header, &data, Some(header)),
        "a new publication sequence alone cannot refresh a repeated audio window"
    );
    for invalid in [
        SpectrumWindow {
            timeline_epoch: 0,
            ..header
        },
        SpectrumWindow {
            timeline_epoch: 2,
            ..header
        },
        SpectrumWindow {
            frame_end: 95,
            ..header
        },
        SpectrumWindow {
            frame_start: 97,
            ..header
        },
        SpectrumWindow {
            sample_rate: f64::NAN,
            ..header
        },
        SpectrumWindow {
            sample_rate: 0.,
            ..header
        },
        SpectrumWindow {
            sample_rate: 44_100.,
            ..header
        },
        SpectrumWindow {
            channels: 1,
            ..header
        },
        SpectrumWindow {
            fft_size: 128,
            ..header
        },
    ] {
        assert!(
            !valid_window(settings, 2, invalid, &data, Some(previous)),
            "{invalid:?}"
        );
    }
    for value in [f32::NAN, f32::INFINITY, -0.001] {
        let mut invalid = data.clone();
        invalid[65] = value;
        assert!(!valid_window(settings, 2, header, &invalid, None));
    }
    let restart = SpectrumWindow {
        timeline_epoch: 4,
        frame_start: 0,
        frame_end: 64,
        sample_rate: 44_100.,
        ..header
    };
    assert!(
        valid_window(settings, 2, restart, &data, Some(header)),
        "a newer timeline epoch may restart frame numbering and sample rate"
    );
}

#[test]
fn spectrum_low_rate_large_hop_stays_fresh_between_slow_windows() {
    let settings = SpectrumSettings {
        fft_size: 8192,
        hop_frames: 8192,
    };
    let timeout = freshness_timeout(settings, 8000.).expect("valid low processing rate");
    assert!(timeout >= Duration::from_secs_f64(2. * 8192. / 8000. + 0.1));
    assert!(
        timeout <= Duration::from_millis(2500),
        "scheduling slack must stay modest"
    );
    let data = vec![0.; settings.bins_per_channel()];
    let start = Instant::now();
    let mut previous = None;
    let mut last_delivery = start;
    for frame in 0_u32..3 {
        let now = start + Duration::from_secs_f64(f64::from(frame) * 1.024);
        assert!(
            now.saturating_duration_since(last_delivery) < timeout,
            "a healthy 8 kHz/8192-hop analyzer must not flicker stale at 500 ms"
        );
        let window = SpectrumWindow {
            timeline_epoch: 1,
            frame_start: u64::from(frame) * 8192,
            frame_end: u64::from(frame + 1) * 8192,
            sample_rate: 8000.,
            fft_size: 8192,
            channels: 1,
        };
        assert!(valid_window(settings, 1, window, &data, previous));
        last_delivery = now;
        previous = Some(window);
    }
    for rate in [0., -1., f64::NAN, f64::INFINITY, f64::MIN_POSITIVE] {
        assert!(
            freshness_timeout(settings, rate).is_none(),
            "invalid/unrepresentable rate {rate}"
        );
    }
    assert_eq!(
        freshness_timeout(
            SpectrumSettings {
                fft_size: 64,
                hop_frames: 32
            },
            48_000.
        ),
        Some(STALE_AFTER)
    );
}
