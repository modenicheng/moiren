use moiren_signal::{Bus, BusConfig, SubscribeError};

#[test]
fn failed_subscription_budget_is_atomic() {
    let mut measure = Bus::new(BusConfig::default()).unwrap();
    let topic = measure.register_latest_value("latest", 99_u32).unwrap();
    let initial = measure.budget_used();
    let _subscriber = measure.subscribe(&topic, Default::default()).unwrap();
    let required = measure.budget_used();
    let mut bus = Bus::new(BusConfig {
        byte_budget: required - 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_latest_value("latest", 99_u32).unwrap();
    assert!(matches!(
        bus.subscribe(&topic, Default::default()),
        Err(SubscribeError::BudgetExceeded)
    ));
    assert_eq!(bus.budget_used(), initial);
}

#[test]
fn staged_replacement_cannot_temporarily_exceed_the_byte_budget() {
    let mut measure = Bus::new(BusConfig::default()).unwrap();
    let topic = measure.register_latest_value("latest", 99_u32).unwrap();
    let subscriber = measure.subscribe(&topic, Default::default()).unwrap();
    let required = measure.budget_used();
    drop(subscriber);
    let mut bus = Bus::new(BusConfig {
        byte_budget: required,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_latest_value("latest", 99_u32).unwrap();
    let subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    drop(subscriber);
    let before = bus.budget_used();
    // Until the non-RT directory reclaims the retained sender, preparing a new
    // triple buffer would temporarily own both sets of resources.
    assert!(matches!(
        bus.subscribe(&topic, Default::default()),
        Err(SubscribeError::BudgetExceeded)
    ));
    assert_eq!(bus.budget_used(), before);
}
