use moiren_signal::{BlockInit, Bus, BusConfig, SubscribeError, SubscriptionOptions};

#[test]
fn stream_subscriptions_prepare_independent_depths_before_demand() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus.register_stream_value("stream", 99_u32, 2).unwrap();
    let before = bus.budget_used();
    let subscribers: Vec<_> = [1, 2, 5]
        .into_iter()
        .map(|depth| {
            bus.subscribe(
                &topic,
                SubscriptionOptions {
                    stream_depth: Some(depth),
                },
            )
            .expect("Stream has independently prepared receivers")
        })
        .collect();
    assert!(bus.budget_used() > before);
    drop(subscribers);
    let block = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 1_u32,
                element: 2_u32,
                capacity: 4,
            },
            2,
        )
        .unwrap();
    bus.subscribe(&block, Default::default())
        .expect("Stream block receiver");
}

#[test]
fn invalid_stream_depths_preserve_budget() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus.register_stream_value("stream", (), 2).unwrap();
    let before = bus.budget_used();
    for depth in [0, usize::MAX] {
        assert!(matches!(
            bus.subscribe(
                &topic,
                SubscriptionOptions {
                    stream_depth: Some(depth)
                }
            ),
            Err(SubscribeError::InvalidCapacity)
        ));
        assert_eq!(bus.budget_used(), before);
    }
}

#[test]
fn status_distinguishes_registered_without_publisher_from_no_message() {
    use moiren_signal::{PublisherStatus, TopicLifecycle, TopicStatus};
    let mut bus = Bus::new(Default::default()).unwrap();
    let value = bus.register_stream_value("value", 99_u32, 2).unwrap();
    let mut subscriber = bus.subscribe(&value, Default::default()).unwrap();
    assert_eq!(
        subscriber.status(),
        TopicStatus {
            lifecycle: TopicLifecycle::Registered,
            publisher: PublisherStatus::NoPublisher
        }
    );
    assert_eq!(subscriber.next(), None);
    let block = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 7_u32,
                element: 8_u32,
                capacity: 3,
            },
            2,
        )
        .unwrap();
    let mut subscriber = bus.subscribe(&block, Default::default()).unwrap();
    assert!(subscriber.next().is_none());
    assert_eq!(subscriber.copy_next(&mut []), Ok(None));
    let latest = bus.register_latest_value("latest", 99_u32).unwrap();
    assert_eq!(
        bus.subscribe(&latest, Default::default())
            .unwrap()
            .status()
            .publisher,
        PublisherStatus::NoPublisher
    );
}

#[test]
fn default_depth_rejects_free_ring_index_overflow_before_preparation() {
    assert!(matches!(
        Bus::new(BusConfig {
            default_stream_depth: usize::MAX / 2,
            ..Default::default()
        }),
        Err(moiren_signal::RegisterError::SizeOverflow)
    ));
}
