use moiren_signal::{
    Bus, BusConfig, LatestValue, LookupError, PublisherStatus, PumpBudget, SubscribeError,
    TopicLifecycle, UnregisterError,
};

#[test]
fn unregister_closes_subscribers_and_reuses_name_with_a_new_identity() {
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let old = bus.register_latest_value("name", 7_u32).unwrap();
    let mut subscriber = bus.subscribe(&old, Default::default()).unwrap();
    assert!(bus.has_business_demand());
    bus.unregister(&old).unwrap();
    assert!(!bus.has_business_demand());
    assert_eq!(old.status().lifecycle, TopicLifecycle::Closed);
    assert_eq!(subscriber.status().lifecycle, TopicLifecycle::Closed);
    assert_eq!(subscriber.status().publisher, PublisherStatus::NoPublisher);
    assert_eq!(subscriber.read_latest(), None);
    assert_eq!(bus.unregister(&old), Err(UnregisterError::Closed));
    assert!(matches!(
        bus.resolve::<LatestValue<u32>>("name"),
        Err(LookupError::NotFound)
    ));
    let new = bus.register_latest_value("name", 9_u32).unwrap();
    assert_ne!(old.id(), new.id());
    assert!(matches!(
        bus.subscribe(&old, Default::default()),
        Err(SubscribeError::Closed)
    ));
    assert_eq!(new.status().lifecycle, TopicLifecycle::Registered);
    drop(subscriber);
    assert_eq!(
        bus.resolve::<LatestValue<u32>>("name").unwrap().id(),
        new.id()
    );
}

#[test]
fn foreign_unregister_preserves_both_directories() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let mut foreign = Bus::new(Default::default()).unwrap();
    let topic = foreign.register_stream_value("stream", 7_u32, 2).unwrap();
    let before = bus.budget_used();
    assert_eq!(bus.unregister(&topic), Err(UnregisterError::ForeignTopic));
    assert_eq!(bus.budget_used(), before);
    assert_eq!(topic.status().lifecycle, TopicLifecycle::Registered);
    foreign.unregister(&topic).unwrap();
}

#[test]
fn zero_work_budget_is_a_noop_and_statistics_are_independent() {
    let mut bus = Bus::new(Default::default()).unwrap();
    let topic = bus.register_stream_value("stream", 7_u32, 2).unwrap();
    let subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let report = bus.pump(PumpBudget {
        topic_checks: 0,
        deliveries: 0,
        copy_bytes: 0,
    });
    assert_eq!(
        (report.topic_checks, report.deliveries, report.copy_bytes),
        (0, 0, 0)
    );
    assert_eq!(topic.stats().attempts, 0);
    assert_eq!(topic.stats().ingress_drops, 0);
    assert_eq!(subscriber.stats().delivered, 0);
    assert_eq!(subscriber.stats().overflow_drops, 0);
}
