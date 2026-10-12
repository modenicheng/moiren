use moiren_app::service::{
    ActiveSession, AppHandle, AppRequest, AppSnapshot, BackendAdapter, BackendDriver, BackendError,
    Cancellation, DeviceCatalog, DispatchError, InputSelection, PreparedSession, RunLimit,
    ServiceMailbox, ServiceRuntime, SessionPhase, SessionReport, SessionSpec, SignalPreparation,
    StartedState, SubscriptionDeliveryError, TimelineSnapshot,
};
use moiren_app::tone::{ToneConfig, ToneSession, prepare_tone_with_signals};
use moiren_core::{graph::NodeId, protocol::ReplyCode};
use moiren_engine::{compiler::CompiledBindings, control::ControlPort};
use moiren_signal::{PublisherStatus, SubscriptionOptions};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

#[test]
fn typed_subscription_is_admitted_by_the_running_service() {
    let runtime = ServiceRuntime::spawn().unwrap();
    let handle = runtime.handle();
    let topic = handle
        .try_snapshot()
        .unwrap()
        .signal_topics
        .tone_output
        .clone();
    let mut ticket = handle
        .subscribe_topic(topic, SubscriptionOptions::default())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let result = loop {
        match ticket.try_take() {
            Ok(None) if Instant::now() < deadline => std::thread::yield_now(),
            result => break result,
        }
    };
    handle.request_exit();
    runtime.join().unwrap();
    let subscriber = result.unwrap().expect("service delivered a subscriber");
    assert_eq!(subscriber.status().publisher, PublisherStatus::NoPublisher);
}

#[test]
fn typed_subscriptions_share_the_ordinary_queue_capacity_and_exit_gate() {
    let (handle, _mailbox) = ServiceMailbox::new(std::thread::current());
    let topic = handle
        .try_snapshot()
        .unwrap()
        .signal_topics
        .tone_output
        .clone();
    for _ in 0..64 {
        handle.try_request(AppRequest::RefreshCatalog).unwrap();
    }
    assert!(matches!(
        handle.subscribe_topic(topic.clone(), Default::default()),
        Err(DispatchError::Busy)
    ));
    handle.request_exit();
    assert!(matches!(
        handle.subscribe_topic(topic, Default::default()),
        Err(DispatchError::Exiting)
    ));
}

#[test]
fn exiting_service_disconnects_pending_tickets_without_hiding_shutdown() {
    let (handle, mailbox) = ServiceMailbox::new(std::thread::current());
    let topic = handle
        .try_snapshot()
        .unwrap()
        .signal_topics
        .tone_output
        .clone();
    let mut ticket = handle.subscribe_topic(topic, Default::default()).unwrap();
    handle.request_exit();
    assert!(mailbox.exit_requested());
    drop(mailbox);
    assert!(matches!(
        ticket.try_take(),
        Err(SubscriptionDeliveryError::Disconnected)
    ));
}

struct OfflineSignalBackend {
    retired: mpsc::Sender<(String, usize)>,
}

struct OfflineSignalSession {
    tone: Option<ToneSession>,
    stopped: AtomicBool,
    retired: mpsc::Sender<(String, usize)>,
}

impl OfflineSignalSession {
    fn finish(mut self: Box<Self>) -> Result<SessionReport, BackendError> {
        let mut tone = self.tone.take().unwrap();
        let remaining = tone.compiled.engine.retire_controls();
        drop(tone);
        self.retired
            .send((std::thread::current().name().unwrap().to_owned(), remaining))
            .unwrap();
        Ok(SessionReport::stopped())
    }
}

impl BackendAdapter for OfflineSignalBackend {
    fn prepare(
        &self,
        _: SessionSpec,
        _: Cancellation,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        Err(BackendError::new(
            "fixture",
            "requires real signal preparation",
        ))
    }

    fn prepare_with_signals(
        &self,
        spec: SessionSpec,
        _: Cancellation,
        signals: &mut SignalPreparation,
    ) -> Result<Box<dyn PreparedSession>, BackendError> {
        let tone = prepare_tone_with_signals(
            ToneConfig {
                gain: spec.gain,
                pan: spec.pan,
                ..Default::default()
            },
            signals,
        )
        .unwrap();
        Ok(Box::new(OfflineSignalSession {
            tone: Some(tone),
            stopped: AtomicBool::new(false),
            retired: self.retired.clone(),
        }))
    }

    fn catalog(&self) -> Result<DeviceCatalog, BackendError> {
        Ok(DeviceCatalog::default())
    }
}

impl PreparedSession for OfflineSignalSession {
    fn activate(&self) -> Result<(), BackendError> {
        Ok(())
    }
    fn into_active(self: Box<Self>) -> Box<dyn ActiveSession> {
        self
    }
    fn cancel(&self) -> Result<(), BackendError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
        self.finish()
    }
}

impl ActiveSession for OfflineSignalSession {
    fn request_stop(&self) -> Result<(), BackendError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
    fn is_finished(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    fn poll_started(&self) -> StartedState {
        StartedState::Running
    }
    fn control(&mut self) -> Option<(&mut ControlPort, &CompiledBindings)> {
        let tone = self.tone.as_mut().unwrap();
        tone.compiled.engine.render(256).unwrap();
        tone.output.read_interleaved(&mut [0.0; 512]).unwrap();
        Some((&mut tone.compiled.control, &tone.compiled.bindings))
    }
    fn timeline(&self) -> Option<TimelineSnapshot> {
        let tone = self.tone.as_ref().unwrap();
        Some(TimelineSnapshot {
            frame: tone.compiled.engine.timeline(),
            epoch: 1,
            revision: 1,
        })
    }
    fn gain_pan_nodes(&self) -> Option<(NodeId, NodeId)> {
        let tone = self.tone.as_ref().unwrap();
        Some((tone.gain_node, tone.pan_node))
    }
    fn join(self: Box<Self>) -> Result<SessionReport, BackendError> {
        self.finish()
    }
}

#[track_caller]
fn wait_snapshot(handle: &AppHandle, predicate: impl Fn(&AppSnapshot) -> bool) -> Arc<AppSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(snapshot) = handle.try_snapshot()
            && predicate(&snapshot)
        {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "service snapshot deadline: {:?}",
            handle.try_snapshot()
        );
        std::thread::yield_now();
    }
}

#[test]
fn zero_signal_demand_still_applies_controls_retires_owner_and_exits() {
    let (retired_tx, retired_rx) = mpsc::channel();
    let runtime =
        ServiceRuntime::spawn_with_driver(BackendDriver::new(Arc::new(OfflineSignalBackend {
            retired: retired_tx,
        })))
        .unwrap();
    let handle = runtime.handle();
    let topic = handle
        .try_snapshot()
        .unwrap()
        .signal_topics
        .tone_output
        .clone();
    let spec = SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
        output_endpoint_id: "offline-fixture".into(),
        limit: RunLimit::UntilStopped,
        gain: 0.01,
        pan: 0.0,
        max_block_frames: 256,
    };
    handle.try_request(AppRequest::Start(spec.clone())).unwrap();
    wait_snapshot(&handle, |snapshot| snapshot.phase == SessionPhase::Running);
    assert_eq!(topic.status().publisher, PublisherStatus::Bound);
    assert_eq!(topic.business_subscribers(), 0);
    let control_id = handle
        .try_request_tracked(AppRequest::SetGainPan {
            gain: 0.008,
            pan: 0.5,
        })
        .unwrap();
    let applied = wait_snapshot(&handle, |snapshot| {
        snapshot.consumed_request_id >= control_id
            && snapshot.control.accepted_pending == 0
            && snapshot.control.last_result.as_ref().is_some_and(|reply| {
                matches!(reply.code, ReplyCode::Applied | ReplyCode::AppliedLate)
            })
    });
    assert_eq!(applied.phase, SessionPhase::Running);
    assert_eq!(topic.stats().attempts, 0);
    handle.try_request(AppRequest::Stop).unwrap();
    assert_eq!(
        retired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        ("app-cleanup".into(), 0)
    );
    wait_snapshot(&handle, |snapshot| snapshot.phase == SessionPhase::Idle);
    assert_eq!(topic.status().publisher, PublisherStatus::NoPublisher);
    handle.try_request(AppRequest::Start(spec)).unwrap();
    wait_snapshot(&handle, |snapshot| snapshot.phase == SessionPhase::Running);
    handle.request_exit();
    assert_eq!(
        retired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        ("app-cleanup".into(), 0)
    );
    wait_snapshot(&handle, |snapshot| snapshot.phase == SessionPhase::Exited);
    runtime.join().unwrap();
    assert_eq!(topic.business_subscribers(), 0);
    assert_eq!(topic.stats().attempts, 0);
    assert_eq!(topic.status().publisher, PublisherStatus::NoPublisher);
    assert!(matches!(
        retired_rx.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}
