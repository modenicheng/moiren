use moiren_signal::{Bus, BusConfig, SubscribeError, SubscriptionOptions};

#[test]
fn initialization_is_not_a_message_and_latest_rejects_stream_options() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus.register_latest_value("latest", 99_u32).unwrap();
    let mut subscriber = bus
        .subscribe(&topic, SubscriptionOptions::default())
        .unwrap();
    assert_eq!(subscriber.read_latest(), None);
    assert!(matches!(
        bus.subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(8)
            }
        ),
        Err(SubscribeError::InvalidOptions)
    ));
}

#[test]
fn subscriptions_validate_identity_and_limits_without_spending_budget() {
    let config = BusConfig {
        max_subscribers_per_topic: 1,
        ..Default::default()
    };
    let mut bus = Bus::new(config).unwrap();
    let mut foreign = Bus::new(config).unwrap();
    let topic = bus.register_latest_value("latest", 99_u32).unwrap();
    let foreign_topic = foreign.register_latest_value("latest", 99_u32).unwrap();
    let before = bus.budget_used();
    assert!(matches!(
        bus.subscribe(&foreign_topic, Default::default()),
        Err(SubscribeError::ForeignTopic)
    ));
    assert_eq!(bus.budget_used(), before);
    let subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let before = bus.budget_used();
    assert!(matches!(
        bus.subscribe(&topic, Default::default()),
        Err(SubscribeError::SubscriberLimit)
    ));
    assert_eq!(bus.budget_used(), before);
    drop(subscriber);
    bus.subscribe(&topic, Default::default()).unwrap();
    assert_eq!(bus.budget_used(), before);
}
