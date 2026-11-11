use moiren_core::graph::NodeId;
use moiren_engine::spectrum::SpectrumWindow;
use moiren_signal::{
    BlockInit, Bus, BusConfig, PumpBudget, PumpReport, RegisterError, Shape, StreamBlock,
    SubscribeError, Subscriber, SubscriptionOptions, Topic,
};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TryRecvError},
};
use std::time::{Duration, Instant};

use super::managed::LevelSource;

/// Spectrum windows stream too, so the relay never overwrites a window the
/// display has not drained; a display only needs the newest frame to look
/// right, so four slots (~40 ms at the default hop cadence) are plenty.
const SPECTRUM_STREAM_DEPTH: usize = 4;

pub type SpectrumTopic = StreamBlock<SpectrumWindow, f32>;

/// The shape a spectrum topic is registered for: one publication carries
/// `channels × (fft_size / 2 + 1)` one-sided magnitudes, so either change
/// re-registers the topic under a new identity the subscribers follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpectrumShape {
    pub channels: usize,
    pub fft_size: usize,
}

impl SpectrumShape {
    /// Magnitudes one application-topic publication carries.
    pub fn capacity(&self) -> usize {
        self.channels * (self.fft_size / 2 + 1)
    }

    /// Frames one engine window carries: the block the bridge transforms.
    pub fn window_capacity(&self) -> usize {
        self.channels * self.fft_size
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalTopics {
    /// One stable level source per node of the running project: the UI's
    /// subscription point, holding the engine subscription directly
    /// (decision D-4 route B — the relay's level topics are gone, only the
    /// stable identity survives).
    pub node_levels: BTreeMap<NodeId, LevelSource>,
    /// One topic per spectrum analyzer node of the running graph, at the
    /// shape its active configuration publishes.
    pub node_spectra: BTreeMap<NodeId, Topic<SpectrumTopic>>,
    pub monitor_input: LevelSource,
    pub monitor_output: LevelSource,
    pub tone_output: LevelSource,
}

pub(crate) struct SignalService {
    bus: Bus,
    pub topics: SignalTopics,
    spectrum_shapes: BTreeMap<NodeId, SpectrumShape>,
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
    pub fn new() -> Result<Self, RegisterError> {
        let bus = Bus::new(BusConfig {
            max_topics: 131,
            ..BusConfig::default()
        })?;
        // Level meters no longer register application topics (route B): the
        // fixed taps are plain subscription points without a bus behind
        // them, and per-node taps are minted by the registry on sync.
        let topics = SignalTopics::new();
        Ok(Self {
            bus,
            topics,
            spectrum_shapes: Default::default(),
            next_poll: Instant::now(),
            copy_budget: 256 * 1024,
            poll_timer: Default::default(),
        })
    }

    pub fn execute(&mut self, request: SubscriptionRequest) {
        request(&mut self.bus);
    }

    /// The service bus, for tests that need a real topic to exercise the
    /// typed subscription machinery with.
    #[cfg(test)]
    pub(crate) fn bus(&mut self) -> &mut Bus {
        &mut self.bus
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
        // The service loop uses this deadline for the spectrum relay and
        // this bus pump; no demand leaves idle waits intact.
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
    fn new() -> Self {
        Self {
            node_levels: Default::default(),
            node_spectra: Default::default(),
            monitor_input: LevelSource::new(),
            monitor_output: LevelSource::new(),
            tone_output: LevelSource::new(),
        }
    }
}

impl SignalService {
    /// Register one spectrum topic per analyzer node at the shape its
    /// running configuration publishes. A node whose shape changed (FFT
    /// size, channel count) or that left the graph is unregistered first:
    /// the fresh registration is a new identity, which is how subscribers
    /// learn to resubscribe at the right capacity.
    pub(super) fn prepare_node_spectra(
        &mut self,
        shapes: &BTreeMap<NodeId, SpectrumShape>,
    ) -> Result<(), String> {
        let stale: Vec<_> = self
            .topics
            .node_spectra
            .keys()
            .filter(|id| self.spectrum_shapes.get(*id) != shapes.get(*id))
            .copied()
            .collect();
        for id in stale {
            let topic = self.topics.node_spectra[&id].clone();
            self.bus
                .unregister(&topic)
                .map_err(|error| error.to_string())?;
            self.topics.node_spectra.remove(&id);
            self.spectrum_shapes.remove(&id);
        }
        for (&id, &shape) in shapes {
            if self.topics.node_spectra.contains_key(&id) {
                continue;
            }
            let topic = self
                .bus
                .register_stream_block(
                    &format!("node/{id:?}/spectrum"),
                    BlockInit {
                        header: SpectrumWindow {
                            timeline_epoch: 0,
                            frame_start: 0,
                            frame_end: 0,
                            sample_rate: 0.,
                            fft_size: shape.fft_size,
                            channels: shape.channels,
                        },
                        element: 0.,
                        capacity: shape.capacity(),
                    },
                    SPECTRUM_STREAM_DEPTH,
                )
                .map_err(|error| error.to_string())?;
            self.topics.node_spectra.insert(id, topic);
            self.spectrum_shapes.insert(id, shape);
        }
        Ok(())
    }

    pub(super) fn clear_node_spectra(&mut self) {
        self.spectrum_shapes.clear();
        for topic in std::mem::take(&mut self.topics.node_spectra).into_values() {
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

    /// A real stream topic on the test bus: the typed subscription
    /// machinery is payload-agnostic, so a plain u32 topic exercises it.
    fn fixture() -> (Bus, Topic<StreamBlock<u32, u32>>) {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus
            .register_stream_block(
                "test/ticket",
                BlockInit {
                    header: 0_u32,
                    element: 0_u32,
                    capacity: 2,
                },
                2,
            )
            .unwrap();
        (bus, topic)
    }

    #[test]
    fn pump_and_service_wait_follow_business_demand_and_the_one_ms_deadline() {
        let mut service = SignalService::new().unwrap();
        let now = Instant::now();
        assert!(service.pump_if_due(now).is_none());
        assert!(service.wait_duration(now).is_none());
        let topic = {
            let bus = service.bus();
            bus.register_stream_value("test/pump", 0_u32, 2).unwrap()
        };
        let (mut ticket, request) = subscription_request(topic.clone(), Default::default());
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
        let (mut bus, topic) = fixture();
        let analysis = bus.subscribe(&topic, Default::default()).unwrap();
        let generation = topic.demand_snapshot().generation;
        let baseline = bus.budget_used();
        let (ticket, request) = subscription_request(topic.clone(), Default::default());
        drop(ticket);
        request(&mut bus);
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(topic.demand_snapshot().generation, generation);
        assert_eq!(bus.budget_used(), baseline);
        drop(analysis);
        assert_eq!(topic.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn cancellation_during_construction_discards_subscriber_before_response() {
        let (mut bus, topic) = fixture();
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = AtomicBool::new(false);
        deliver_subscription(
            &mut bus,
            &topic,
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
        let (mut bus, topic) = fixture();
        let analysis = bus.subscribe(&topic, Default::default()).unwrap();
        let generation = topic.demand_snapshot().generation;
        let (ticket, request) = subscription_request(topic.clone(), Default::default());
        request(&mut bus);
        assert_eq!(topic.business_subscribers(), 2);
        drop(ticket);
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(topic.demand_snapshot().generation, generation);
        drop(analysis);
        assert_eq!(topic.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn taking_subscriber_transfers_ownership_out_of_ticket() {
        let (mut bus, topic) = fixture();
        let (mut ticket, request) = subscription_request(topic.clone(), Default::default());
        assert!(ticket.try_take().unwrap().is_none());
        request(&mut bus);
        let subscriber = ticket.try_take().unwrap().unwrap();
        drop(ticket);
        assert_eq!(topic.business_subscribers(), 1);
        drop(subscriber);
        assert_eq!(topic.business_subscribers(), 0);
        assert!(!bus.has_business_demand());
    }

    #[test]
    fn ticket_cancelled_after_construction_releases_only_its_own_demand() {
        let (mut bus, topic) = fixture();
        let analysis = bus.subscribe(&topic, Default::default()).unwrap();
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
            let worker_topic = topic.clone();
            let worker = threads.spawn(move || {
                deliver_subscription(
                    worker_bus,
                    &worker_topic,
                    Default::default(),
                    sender,
                    &cancelled,
                    || {
                        constructed_tx
                            .send(worker_topic.business_subscribers())
                            .unwrap();
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
        let (mut bus, topic) = fixture();
        let analysis = bus.subscribe(&topic, Default::default()).unwrap();
        let generation = topic.demand_snapshot().generation;
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = AtomicBool::new(false);
        deliver_subscription(
            &mut bus,
            &topic,
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
            ..BusConfig::default()
        })
        .unwrap();
        let now = Instant::now();
        let topic = bus.register_stream_value("test/ingress", 0_u32, 2).unwrap();
        let mut service = SignalService {
            bus,
            topics: SignalTopics::new(),
            spectrum_shapes: Default::default(),
            next_poll: now,
            copy_budget: 256 * 1024,
            poll_timer: Default::default(),
        };
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
}
