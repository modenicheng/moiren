use moiren_engine::meter::signal::{AudioWindow, ChannelLevel};
use moiren_signal::{
    BlockInit, Bus, BusConfig, LatestBlock, PumpBudget, PumpReport, RegisterError, Shape,
    SubscribeError, Subscriber, SubscriptionOptions, Topic,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TryRecvError},
};
use std::time::{Duration, Instant};

pub type LevelTopic = LatestBlock<AudioWindow, ChannelLevel>;

#[derive(Debug, Clone)]
pub struct SignalTopics {
    pub node_levels: std::collections::BTreeMap<moiren_core::graph::NodeId, Topic<LevelTopic>>,
    pub monitor_input: Topic<LevelTopic>,
    pub monitor_output: Topic<LevelTopic>,
    pub tone_output: Topic<LevelTopic>,
}

pub(crate) struct SignalService {
    bus: Bus,
    pub topics: SignalTopics,
    next_poll: Instant,
    copy_budget: usize,
    poll_timer: moiren_engine::meter::MeterPollingTimer,
}

impl SignalService {
    pub(super) fn create_bridge_host(
        &mut self,
    ) -> Result<(moiren_signal::HostControl, moiren_signal::RtHost), moiren_signal::HostError> {
        self.bus.create_host(131)
    }
    pub fn prepare_session(
        &mut self,
        source: &super::InputSelection,
    ) -> Result<super::SignalPreparation, moiren_signal::HostError> {
        let (input, output) = match source {
            super::InputSelection::Tone { .. } => (None, self.topics.tone_output.clone()),
            _ => (
                Some(self.topics.monitor_input.clone()),
                self.topics.monitor_output.clone(),
            ),
        };
        super::SignalPreparation::new(&mut self.bus, input, output)
    }

    pub fn new() -> Result<Self, RegisterError> {
        let mut bus = Bus::new(BusConfig {
            max_topics: 131,
            ..BusConfig::default()
        })?;
        let topics = SignalTopics::register(&mut bus)?;
        Ok(Self {
            bus,
            topics,
            next_poll: Instant::now(),
            copy_budget: 256 * 1024,
            poll_timer: Default::default(),
        })
    }

    pub fn execute(&mut self, request: SubscriptionRequest) {
        request(&mut self.bus);
    }

    pub fn pump_if_due(&mut self, now: Instant) -> Option<PumpReport> {
        let demand = self.bus.has_business_demand();
        self.poll_timer.set_demand(demand);
        if !demand || now < self.next_poll {
            return None;
        }
        let report = self.bus.pump(PumpBudget {
            topic_checks: 524,
            deliveries: 256,
            copy_bytes: self.copy_budget,
        });
        self.copy_budget = (256 * 1024)
            .max(report.required_copy_bytes)
            .min(1024 * 1024);
        // The service loop uses this deadline for both the compatibility
        // bridge relay and this bus pump; no demand leaves idle waits intact.
        self.next_poll = now + Duration::from_millis(1);
        Some(report)
    }

    pub fn wait_duration(&self, now: Instant) -> Option<Duration> {
        self.bus
            .has_business_demand()
            .then(|| self.next_poll.saturating_duration_since(now))
    }
}

impl SignalTopics {
    fn register(bus: &mut Bus) -> Result<Self, RegisterError> {
        let initial = BlockInit {
            header: AudioWindow {
                timeline_epoch: 0,
                frame_start: 0,
                frame_end: 0,
                sample_rate: 0.0,
            },
            element: ChannelLevel {
                peak: 0.0,
                rms: 0.0,
            },
            capacity: 2,
        };
        Ok(Self {
            node_levels: Default::default(),
            monitor_input: bus.register_latest_block("monitor/input/output/level", initial)?,
            monitor_output: bus.register_latest_block("monitor/output/level", initial)?,
            tone_output: bus.register_latest_block("tone/output/level", initial)?,
        })
    }
}

impl SignalService {
    pub(super) fn prepare_node_levels(
        &mut self,
        graph: &moiren_core::graph::LogicalGraph,
    ) -> Result<(), String> {
        let removed: Vec<_> = self
            .topics
            .node_levels
            .keys()
            .filter(|id| graph.get_node(**id).is_err())
            .copied()
            .collect();
        for id in removed {
            let topic = self
                .topics
                .node_levels
                .remove(&id)
                .expect("existing level topic");
            self.bus
                .unregister(&topic)
                .map_err(|error| error.to_string())?;
        }
        for node in graph.nodes() {
            if self.topics.node_levels.contains_key(&node.id()) {
                continue;
            }
            let topic = self
                .bus
                .register_latest_block(
                    &format!("node/{:?}/level", node.id()),
                    BlockInit {
                        header: AudioWindow {
                            timeline_epoch: 0,
                            frame_start: 0,
                            frame_end: 0,
                            sample_rate: 0.,
                        },
                        element: ChannelLevel { peak: 0., rms: 0. },
                        capacity: node.channels(),
                    },
                )
                .map_err(|error| error.to_string())?;
            self.topics.node_levels.insert(node.id(), topic);
        }
        Ok(())
    }

    pub(super) fn clear_node_levels(&mut self) {
        for topic in std::mem::take(&mut self.topics.node_levels).into_values() {
            let _ = self.bus.unregister(&topic);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubscriptionDeliveryError {
    #[error(transparent)]
    Subscribe(#[from] SubscribeError),
    #[error("signal subscription service disconnected")]
    Disconnected,
}

pub struct SubscriptionTicket<P: Shape> {
    receiver: Receiver<Result<Subscriber<P>, SubscribeError>>,
    cancelled: Arc<AtomicBool>,
}

impl<P: Shape> SubscriptionTicket<P> {
    pub fn try_take(&mut self) -> Result<Option<Subscriber<P>>, SubscriptionDeliveryError> {
        match self.receiver.try_recv() {
            Ok(Ok(subscriber)) => Ok(Some(subscriber)),
            Ok(Err(error)) => Err(error.into()),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(SubscriptionDeliveryError::Disconnected),
        }
    }
}

impl<P: Shape> Drop for SubscriptionTicket<P> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(crate) type SubscriptionRequest = Box<dyn FnOnce(&mut Bus) + Send>;

pub(crate) fn subscription_request<P: Shape>(
    topic: Topic<P>,
    options: SubscriptionOptions,
) -> (SubscriptionTicket<P>, SubscriptionRequest) {
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let request_cancelled = cancelled.clone();
    (
        SubscriptionTicket {
            receiver,
            cancelled,
        },
        Box::new(move |bus| {
            deliver_subscription(bus, &topic, options, sender, &request_cancelled, || {});
        }),
    )
}

fn deliver_subscription<P: Shape>(
    bus: &mut Bus,
    topic: &Topic<P>,
    options: SubscriptionOptions,
    sender: SyncSender<Result<Subscriber<P>, SubscribeError>>,
    cancelled: &AtomicBool,
    after_construction: impl FnOnce(),
) {
    if cancelled.load(Ordering::Acquire) {
        return;
    }
    let result = bus.subscribe(topic, options);
    after_construction();
    if !cancelled.load(Ordering::Acquire) {
        let _ = sender.try_send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Bus, SignalTopics) {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topics = SignalTopics::register(&mut bus).unwrap();
        (bus, topics)
    }

    #[test]
    fn pump_and_service_wait_follow_business_demand_and_the_one_ms_deadline() {
        let mut service = SignalService::new().unwrap();
        let now = Instant::now();
        assert!(service.pump_if_due(now).is_none());
        assert!(service.wait_duration(now).is_none());
        let (mut ticket, request) =
            subscription_request(service.topics.tone_output.clone(), Default::default());
        service.execute(request);
        let subscriber = ticket.try_take().unwrap().unwrap();
        let report = service.pump_if_due(now).unwrap();
        assert!(report.topic_checks <= 524);
        assert!(report.deliveries <= 256);
        assert!(report.copy_bytes <= 256 * 1024);
        assert_eq!(service.wait_duration(now), Some(Duration::from_millis(1)));
        assert!(
            service
                .pump_if_due(now + Duration::from_micros(500))
                .is_none()
        );
        assert!(
            service
                .pump_if_due(now + Duration::from_millis(1))
                .is_some()
        );
        drop(subscriber);
        assert!(service.wait_duration(now).is_none());
        assert!(
            service
                .pump_if_due(now + Duration::from_millis(10))
                .is_none()
        );
    }

    #[test]
    fn cancellation_before_service_execution_skips_receiver_preparation() {
        let (mut bus, topics) = fixture();
        let analysis = bus
            .subscribe(&topics.tone_output, Default::default())
            .unwrap();
        let generation = topics.tone_output.demand_snapshot().generation;
        let baseline = bus.budget_used();
        let (ticket, request) =
            subscription_request(topics.tone_output.clone(), Default::default());
        drop(ticket);
        request(&mut bus);
        assert_eq!(topics.tone_output.business_subscribers(), 1);
        assert_eq!(topics.tone_output.demand_snapshot().generation, generation);
        assert_eq!(bus.budget_used(), baseline);
        drop(analysis);
        assert_eq!(topics.tone_output.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn cancellation_during_construction_discards_subscriber_before_response() {
        let (mut bus, topics) = fixture();
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = AtomicBool::new(false);
        deliver_subscription(
            &mut bus,
            &topics.tone_output,
            Default::default(),
            sender,
            &cancelled,
            || cancelled.store(true, Ordering::Release),
        );
        assert!(!bus.has_business_demand());
        assert!(matches!(
            receiver.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn dropping_delivered_ticket_releases_pending_response_demand() {
        let (mut bus, topics) = fixture();
        let analysis = bus
            .subscribe(&topics.tone_output, Default::default())
            .unwrap();
        let generation = topics.tone_output.demand_snapshot().generation;
        let (ticket, request) =
            subscription_request(topics.tone_output.clone(), Default::default());
        request(&mut bus);
        assert_eq!(topics.tone_output.business_subscribers(), 2);
        drop(ticket);
        assert_eq!(topics.tone_output.business_subscribers(), 1);
        assert_eq!(topics.tone_output.demand_snapshot().generation, generation);
        drop(analysis);
        assert_eq!(topics.tone_output.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn taking_subscriber_transfers_ownership_out_of_ticket() {
        let (mut bus, topics) = fixture();
        let (mut ticket, request) =
            subscription_request(topics.tone_output.clone(), Default::default());
        assert!(ticket.try_take().unwrap().is_none());
        request(&mut bus);
        let subscriber = ticket.try_take().unwrap().unwrap();
        drop(ticket);
        assert_eq!(topics.tone_output.business_subscribers(), 1);
        drop(subscriber);
        assert_eq!(topics.tone_output.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn ticket_cancelled_after_construction_releases_only_its_own_demand() {
        let (mut bus, topics) = fixture();
        let topic = &topics.tone_output;
        let analysis = bus.subscribe(topic, Default::default()).unwrap();
        let generation = topic.demand_snapshot().generation;
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let ticket = SubscriptionTicket {
            receiver,
            cancelled: cancelled.clone(),
        };
        let (constructed_tx, constructed_rx) = mpsc::sync_channel(1);
        let (resume_tx, resume_rx) = mpsc::sync_channel(1);
        std::thread::scope(|threads| {
            let worker_bus = &mut bus;
            let worker = threads.spawn(move || {
                deliver_subscription(
                    worker_bus,
                    topic,
                    Default::default(),
                    sender,
                    &cancelled,
                    || {
                        constructed_tx.send(topic.business_subscribers()).unwrap();
                        resume_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    },
                );
            });
            assert_eq!(
                constructed_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
                2
            );
            drop(ticket);
            assert_eq!(topic.business_subscribers(), 2);
            resume_tx.send(()).unwrap();
            worker.join().unwrap();
        });
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(topic.demand_snapshot().generation, generation);
        drop(analysis);
        assert_eq!(topic.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn disconnected_response_send_releases_constructed_demand_once() {
        let (mut bus, topics) = fixture();
        let topic = &topics.tone_output;
        let analysis = bus.subscribe(topic, Default::default()).unwrap();
        let generation = topic.demand_snapshot().generation;
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = AtomicBool::new(false);
        deliver_subscription(
            &mut bus,
            topic,
            Default::default(),
            sender,
            &cancelled,
            || {
                assert_eq!(topic.business_subscribers(), 2);
                drop(receiver);
            },
        );
        assert!(!cancelled.load(Ordering::Acquire));
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(topic.demand_snapshot().generation, generation);
        drop(analysis);
        assert_eq!(topic.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn zero_demand_does_not_drain_full_ingress_or_advance_pump_deadline() {
        use moiren_signal::PublishOutcome;

        let mut bus = Bus::new(BusConfig {
            max_topics: 4,
            ..Default::default()
        })
        .unwrap();
        let topics = SignalTopics::register(&mut bus).unwrap();
        let now = Instant::now();
        let mut service = SignalService {
            bus,
            topics,
            next_poll: now,
            copy_budget: 256 * 1024,
            poll_timer: Default::default(),
        };
        let topic = service
            .bus
            .register_stream_value("test/ingress", 0_u32, 2)
            .unwrap();
        let subscriber = service.bus.subscribe(&topic, Default::default()).unwrap();
        let (control, mut host) = service.bus.create_host(1).unwrap();
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&topic, key).unwrap();
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        let operation = update.op_bindings(key).unwrap();
        drop(host.activate(update).unwrap());
        host.begin_block();
        for value in 1..=2 {
            assert!(matches!(
                host.scope(&operation)
                    .publish(&publisher.handle(), value)
                    .unwrap(),
                PublishOutcome::Published(_)
            ));
        }
        drop(subscriber);
        for elapsed in [0, 5, 10, 100] {
            let time = now + Duration::from_millis(elapsed);
            assert!(service.pump_if_due(time).is_none());
            assert!(service.wait_duration(time).is_none());
            assert_eq!(service.next_poll, now);
        }
        assert_eq!(topic.stats().attempts, 2);
        let mut subscriber = service.bus.subscribe(&topic, Default::default()).unwrap();
        host.begin_block();
        assert!(matches!(
            host.scope(&operation)
                .publish(&publisher.handle(), 3)
                .unwrap(),
            PublishOutcome::DroppedFull { .. }
        ));
        assert!(
            service
                .pump_if_due(now + Duration::from_millis(100))
                .is_some()
        );
        assert!(
            subscriber.next().is_none(),
            "old generation ingress is discarded"
        );
        assert!(matches!(
            host.scope(&operation)
                .publish(&publisher.handle(), 4)
                .unwrap(),
            PublishOutcome::Published(_)
        ));
        service
            .pump_if_due(now + Duration::from_millis(105))
            .unwrap();
        assert_eq!(subscriber.next().unwrap().value, 4);
    }

    #[test]
    fn analysis_receives_real_audio_windows_after_ui_hide_without_generation_reset() {
        use crate::ui::signals::{PresentationDemand, VisibleSignal};

        let mut service = SignalService::new().unwrap();
        let topic = service.topics.tone_output.clone();
        let mut snapshot = super::super::snapshot(&super::super::ServiceContext::default());
        snapshot.signal_topics = service.topics.clone();
        let (handle, mailbox) =
            super::super::ServiceMailbox::with_snapshot(std::thread::current(), snapshot);
        let mut ui = VisibleSignal::new(handle, topic.clone(), Default::default());
        let mut demand = PresentationDemand {
            window_visible: true,
            minimized: false,
            page_active: true,
            component_visible: true,
            intersects_viewport: true,
        };
        ui.update_demand(demand).unwrap();
        assert!(matches!(mailbox.try_dispatch(&mut service), Some(None)));
        assert!(ui.subscriber().unwrap().is_some());
        let mut analysis = service.bus.subscribe(&topic, Default::default()).unwrap();
        let generation = topic.demand_snapshot().generation;
        let mut preparation = service
            .prepare_session(&super::super::InputSelection::Tone {
                frequency_hz: 440.0,
            })
            .unwrap();
        let mut tone = crate::tone::prepare_tone_with_signals(
            crate::tone::ToneConfig {
                gain: 0.01,
                ..Default::default()
            },
            &mut preparation,
        )
        .unwrap();
        let now = Instant::now();
        for _ in 0..6 {
            tone.compiled.engine.render(256).unwrap();
            tone.output.read_interleaved(&mut [0.0; 512]).unwrap();
        }
        service.pump_if_due(now).unwrap();
        let first_sequence = analysis.read_latest().unwrap().meta().sequence;
        assert_eq!(
            ui.subscriber()
                .unwrap()
                .unwrap()
                .read_latest()
                .unwrap()
                .meta()
                .sequence,
            first_sequence
        );
        demand.window_visible = false;
        ui.update_demand(demand).unwrap();
        ui.update_demand(demand).unwrap();
        assert!(ui.subscriber().unwrap().is_none());
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(topic.demand_snapshot().generation, generation);
        // Two 256-frame blocks complete three more 200-frame windows. Latest
        // keeps the newest window; skipped windows cannot contribute peaks.
        for _ in 0..2 {
            tone.compiled.engine.render(256).unwrap();
            tone.output.read_interleaved(&mut [0.0; 512]).unwrap();
        }
        service.pump_if_due(now + Duration::from_millis(5)).unwrap();
        {
            let window = analysis.read_latest().unwrap();
            assert_eq!(window.meta().sequence, first_sequence + 3);
            assert_eq!(window.meta().activation_generation, generation);
            assert_eq!(window.header().frame_start, 1800);
            assert_eq!(window.header().frame_end, 2000);
            let expected_rms = ((1800..2000)
                .map(|frame| {
                    (std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0)
                        .sin()
                        .powi(2)
                })
                .sum::<f64>()
                / 200.0)
                .sqrt()
                * 0.01;
            for level in window.elements() {
                assert!((level.peak - 0.01).abs() < 0.00001);
                assert!((f64::from(level.rms) - expected_rms).abs() < 0.00001);
            }
        }
        drop(ui);
        assert_eq!(topic.business_subscribers(), 1);
        drop(analysis);
        assert_eq!(topic.business_subscribers(), 0);
        let attempts = topic.stats().attempts;
        for _ in 0..6 {
            tone.compiled.engine.render(256).unwrap();
            tone.output.read_interleaved(&mut [0.0; 512]).unwrap();
        }
        assert_eq!(topic.stats().attempts, attempts);
        assert!(
            service
                .pump_if_due(now + Duration::from_millis(10))
                .is_none()
        );
    }

    #[test]
    fn subscribe_error_and_service_disconnection_are_distinct() {
        let (mut bus, topics) = fixture();
        let (mut ticket, request) = subscription_request(
            topics.tone_output.clone(),
            SubscriptionOptions {
                stream_depth: Some(1),
            },
        );
        request(&mut bus);
        assert!(matches!(
            ticket.try_take(),
            Err(SubscriptionDeliveryError::Subscribe(_))
        ));
        let (mut ticket, request) = subscription_request(topics.tone_output, Default::default());
        drop(request);
        assert!(matches!(
            ticket.try_take(),
            Err(SubscriptionDeliveryError::Disconnected)
        ));
        assert!(!bus.has_business_demand());
    }
}
