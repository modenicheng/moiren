use super::{MeterStatus, MeterSubscription, STALE_AFTER, valid_level_window, window_timing};
use crate::service::{ServiceContext, ServiceMailbox};
use moiren_engine::meter::signal::{AudioWindow, ChannelLevel};
use std::time::{Duration, Instant};

fn header(frames: u64, rate: f64) -> AudioWindow {
    AudioWindow {
        timeline_epoch: 1,
        frame_start: 0,
        frame_end: frames,
        sample_rate: rate,
    }
}

#[test]
fn meter_freshness_short_windows_keep_the_half_second_floor() {
    for frames in [1, 200, 480, 9600] {
        assert_eq!(
            window_timing(header(frames, 48_000.)).unwrap().1,
            STALE_AFTER
        );
    }
}

#[test]
fn meter_freshness_tracks_actual_one_second_and_maximum_low_rate_windows() {
    assert_eq!(
        window_timing(header(48_000, 48_000.)),
        Some((Duration::from_secs(1), Duration::from_millis(2100)))
    );
    assert_eq!(
        window_timing(header(1_048_576, 8000.)),
        Some((
            Duration::from_micros(131_072_000),
            Duration::from_micros(262_244_000)
        ))
    );
    assert_eq!(
        window_timing(header(1_048_576, 1.)).unwrap().1,
        Duration::from_millis(2_097_152_100)
    );
}

#[test]
fn meter_freshness_rejects_invalid_unrepresentable_and_overflowing_headers() {
    let levels = [ChannelLevel {
        peak: 0.5,
        rms: 0.25,
    }; 2];
    for bad in [
        header(0, 48_000.),
        AudioWindow {
            frame_start: 2,
            ..header(1, 48_000.)
        },
        AudioWindow {
            timeline_epoch: 0,
            ..header(480, 48_000.)
        },
        header(480, 0.),
        header(480, -1.),
        header(480, f64::NAN),
        header(480, f64::INFINITY),
        header(480, f64::NEG_INFINITY),
        header(u64::MAX, f64::MIN_POSITIVE),
        header(1, f64::MAX),
        header(u64::MAX, 1.),
        header(u64::MAX, 2.),
    ] {
        assert!(window_timing(bad).is_none(), "{bad:?}");
        assert!(!valid_level_window(2, 2, 1, bad, &levels, None), "{bad:?}");
    }
    let near_end = AudioWindow {
        frame_start: u64::MAX - 480,
        frame_end: u64::MAX,
        ..header(480, 48_000.)
    };
    assert_eq!(window_timing(near_end).unwrap().1, STALE_AFTER);
}

#[test]
fn meter_freshness_state_stays_live_between_long_windows_then_expires() {
    let context = ServiceContext::default();
    let (handle, _mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let mut meter = MeterSubscription::new(handle, context.signals.topics.monitor_input.clone(), 2);
    let now = Instant::now();
    meter.expire_if_stale(now);
    assert_eq!(meter.status, MeterStatus::Waiting);
    assert!(meter.levels.is_none());
    let levels = [ChannelLevel {
        peak: 0.5,
        rms: 0.25,
    }; 2];
    let actual = header(48_000, 48_000.);
    assert!(valid_level_window(2, 1, 0, actual, &levels, None));
    meter.last_delivery = Some(now);
    meter.freshness_timeout = window_timing(actual).unwrap().1;
    meter.levels = Some(Box::new(levels));
    meter.status = MeterStatus::Live;
    for millis in [499, 500, 1000, 2099] {
        meter.expire_if_stale(now + Duration::from_millis(millis));
        assert_eq!(meter.status, MeterStatus::Live, "{millis} ms");
        assert!(meter.levels.is_some());
    }
    meter.expire_if_stale(now + Duration::from_millis(2100));
    meter.advance_display(now + Duration::from_millis(2100));
    assert_eq!(meter.status, MeterStatus::Stale);
    assert!(meter.levels.is_none() && meter.display.is_none());
}

#[test]
fn meter_freshness_subscription_uses_accepted_window_and_expires_after_silence() {
    let mut context = ServiceContext::default();
    let topic = context.signals.topics.monitor_input.clone();
    let (handle, mailbox) =
        ServiceMailbox::with_snapshot(std::thread::current(), crate::service::snapshot(&context));
    let (prepared_tx, prepared_rx) = std::sync::mpsc::sync_channel(1);
    let publication_topic = topic.clone();
    // Prepare on the service's actual bus without widening its production API.
    // Delivery still uses AppHandle -> mailbox -> VisibleSignal -> poll_windows.
    context.signals.execute(Box::new(move |bus| {
        let (control, mut host) = bus.create_host(1).unwrap();
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&publication_topic, key).unwrap();
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        let operation = update.op_bindings(key).unwrap();
        drop(host.activate(update).unwrap());
        assert!(
            prepared_tx
                .send((control, host, publisher, operation))
                .is_ok()
        );
    }));
    let (_control, mut host, publisher, operation) = prepared_rx.try_recv().unwrap();
    let mut meter = MeterSubscription::new(handle, topic.clone(), 2);
    let now = Instant::now();
    assert!(!meter.poll(now));
    assert_eq!(meter.status, MeterStatus::Waiting);
    assert!(matches!(
        mailbox.try_dispatch(&mut context.signals),
        Some(None)
    ));
    assert!(mailbox.try_dispatch(&mut context.signals).is_none());
    assert!(!meter.poll(now));
    assert_eq!(topic.business_subscribers(), 1);

    let levels = [ChannelLevel {
        peak: 0.5,
        rms: 0.25,
    }; 2];
    host.begin_block();
    assert!(matches!(
        host.scope(&operation)
            .publish_slice(&publisher.handle(), header(48_000, 48_000.), &levels)
            .unwrap(),
        moiren_signal::PublishOutcome::Published(_)
    ));
    assert!(context.signals.pump_if_due(now).unwrap().deliveries > 0);
    assert!(
        meter.poll(now),
        "poll_windows must accept the real publication"
    );
    assert_eq!(meter.levels.as_deref(), Some(levels.as_slice()));
    assert_eq!(meter.last_window, Some((1, 48_000)));
    assert_eq!(meter.last_delivery, Some(now));
    assert_eq!(meter.freshness_timeout, Duration::from_millis(2100));
    assert!(!meter.poll(now), "the publication queue must be drained");

    // No further publications: simulated UI polls must not refresh the receipt.
    for millis in [500, 1000, 2099] {
        assert!(!meter.poll(now + Duration::from_millis(millis)));
        assert_eq!(meter.status, MeterStatus::Live, "{millis} ms");
        assert_eq!(meter.last_delivery, Some(now));
        assert!(meter.levels.is_some() && meter.display.is_some());
    }
    assert!(!meter.poll(now + Duration::from_millis(2100)));
    assert_eq!(meter.status, MeterStatus::Stale);
    assert!(meter.levels.is_none() && meter.display.is_none());

    // A later short publication must shrink the allowance, not retain 2.1 s.
    let resumed = now + Duration::from_millis(2200);
    host.begin_block();
    assert!(matches!(
        host.scope(&operation)
            .publish_slice(
                &publisher.handle(),
                AudioWindow {
                    frame_start: 48_000,
                    frame_end: 48_480,
                    ..header(480, 48_000.)
                },
                &levels,
            )
            .unwrap(),
        moiren_signal::PublishOutcome::Published(_)
    ));
    assert!(context.signals.pump_if_due(resumed).unwrap().deliveries > 0);
    assert!(meter.poll(resumed));
    assert_eq!(meter.freshness_timeout, STALE_AFTER);
    assert!(!meter.poll(resumed + Duration::from_millis(499)));
    assert_eq!(meter.status, MeterStatus::Live);
    assert!(!meter.poll(resumed + STALE_AFTER));
    assert_eq!(meter.status, MeterStatus::Stale);
    assert!(meter.levels.is_none() && meter.display.is_none());
}
