mod common;

use moiren_app::service::{
    AppRequest, AppSnapshot, DispatchError, ServiceMailbox, SubscriptionDeliveryError,
};
use moiren_signal::{PublisherStatus, SubscriptionOptions};
use std::time::{Duration, Instant};

fn wait_snapshot(
    handle: &moiren_app::service::AppHandle,
    predicate: impl Fn(&AppSnapshot) -> bool,
) -> std::sync::Arc<AppSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(snapshot) = handle.try_snapshot().filter(|snapshot| predicate(snapshot)) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "snapshot predicate unmet");
        std::thread::yield_now();
    }
}

#[test]
fn typed_subscription_is_admitted_by_the_running_service() {
    let runtime = common::service();
    let handle = runtime.handle();
    let topic = wait_snapshot(&handle, |_| true)
        .signal_topics
        .tone_output
        .clone();
    let mut ticket = handle
        .subscribe_topic(topic, SubscriptionOptions::default())
        .unwrap();
    // Result-shaped wait: an Err delivery is a real outcome, not "keep
    // polling", so the Option-based common::wait_for cannot express it.
    let deadline = Instant::now() + Duration::from_secs(2);
    let result = loop {
        match ticket.try_take() {
            Ok(None) if Instant::now() < deadline => std::thread::yield_now(),
            result => break result,
        }
    };
    common::finish(runtime);
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
