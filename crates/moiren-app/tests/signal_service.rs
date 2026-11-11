mod common;

use moiren_app::service::LevelSource;

/// The level lease is the whole demand chain (decision D-4 route B):
/// subscribing through the running service arms the registry's consumer
/// count immediately, and dropping the lease releases it. The registry's
/// round then binds and releases the engine subscription, so the realtime
/// meters capture exactly while a reader exists.
#[test]
fn level_source_demand_follows_subscriptions_through_the_running_service() {
    let runtime = common::service();
    let handle = runtime.handle();
    let source: LevelSource = common::wait_snapshot(&handle)
        .signal_topics
        .tone_output
        .clone();
    assert_eq!(source.business_subscribers(), 0);
    let lease = source.subscribe();
    assert_eq!(source.business_subscribers(), 1);
    // A second lease of the same tap counts independently.
    let second = source.subscribe();
    assert_eq!(source.business_subscribers(), 2);
    drop(second);
    assert_eq!(source.business_subscribers(), 1);
    drop(lease);
    assert_eq!(source.business_subscribers(), 0);
    common::finish(runtime);
}
