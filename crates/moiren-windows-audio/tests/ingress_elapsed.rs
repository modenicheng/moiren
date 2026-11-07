//! Host publication residence is deliberately not Process source sample age.
use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    process_loopback::ProcessBridgeOptions,
    stats::{DISCONTINUITY, SILENT},
};
use serde_json::json;
use std::cell::Cell;

fn config(limit: Option<u64>) -> ClockBridgeConfig {
    ClockBridgeConfig {
        capacity_frames: 64,
        target_fill_frames: 4,
        trim_on_prime: true,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        max_ingress_elapsed_100ns: limit,
        ..Default::default()
    }
}

fn packet(frames: usize) -> CapturePacket {
    CapturePacket {
        frames,
        flags: SILENT,
        device_position_frames: 0,
        qpc_100ns: 0,
    }
}

fn push(
    ingress: &mut moiren_windows_audio::clock_bridge::CaptureIngress,
    frames: usize,
    time: Option<u64>,
) {
    assert_eq!(
        ingress
            .push_packet_with_publication_clock(&[], packet(frames), || time)
            .unwrap(),
        frames
    );
}

fn read(
    source: &mut moiren_windows_audio::clock_bridge::ClockSource,
    frames: usize,
    time: Option<u64>,
) -> moiren_engine::boundary::BoundaryReport {
    source
        .read_interleaved_with_publication_clock(&mut vec![0.0; frames * 2], || time)
        .unwrap()
}

fn read_with_endpoints(
    source: &mut moiren_windows_audio::clock_bridge::ClockSource,
    times: [u64; 3],
) -> moiren_engine::boundary::BoundaryReport {
    let mut times = times.into_iter();
    let report = source
        .read_interleaved_with_publication_clock(&mut [0.0; 2], || {
            Some(times.next().expect("only three read observations"))
        })
        .unwrap();
    assert!(times.next().is_none());
    report
}

#[test]
fn process_ingress_limit_roundtrips_without_relaxing_sample_age_refusal() {
    let baseline = serde_json::to_value(ProcessBridgeOptions::default()).unwrap();
    assert!(baseline.get("max_ingress_elapsed_100ns").is_none());
    for limit in [1, 100_000, u64::MAX] {
        let mut value = baseline.clone();
        value["max_ingress_elapsed_100ns"] = json!(limit);
        let options: ProcessBridgeOptions = serde_json::from_value(value).unwrap();
        options.prepare().unwrap();
        assert_eq!(options.max_ingress_elapsed_100ns, Some(limit));
        assert_eq!(
            options
                .for_consumer(48_000, 256)
                .unwrap()
                .max_ingress_elapsed_100ns,
            Some(limit)
        );
        let requested = ProcessBridgeOptions {
            age_limit_100ns: Some(1),
            ..options
        };
        assert!(
            requested
                .prepare()
                .unwrap_err()
                .to_string()
                .contains("sample-age enforcement unsupported")
        );
    }
    assert!(
        ProcessBridgeOptions {
            max_ingress_elapsed_100ns: Some(0),
            ..Default::default()
        }
        .prepare()
        .is_err()
    );
    assert!(capture_bridge(config(Some(0))).is_err());
}

#[test]
fn ingress_elapsed_discards_below_high_water_and_does_not_preserve_stale_suffix() {
    let (mut ingress, mut source, observer) = capture_bridge(config(Some(100))).unwrap();
    push(&mut ingress, 4, Some(1000));
    assert_eq!(read(&mut source, 1, Some(1050)).transferred_frames, 1);
    let report = read(&mut source, 1, Some(1200));
    assert_eq!(
        report.transferred_frames, 0,
        "old held input must not resume"
    );
    assert!(report.discontinuity);
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 4);
    assert_eq!(source.snapshot().fill_frames, 0);
    assert!(source.oldest_frame_age(1200).is_none());
    let queue = observer.queue_history().unwrap();
    assert!(!queue.saturated);
    assert_eq!(queue.events[1].sample.ingress_elapsed_discarded_frames, 4);
    assert_eq!(queue.events[1].sample.reset_discarded_frames, 0);
    assert_eq!(queue.events[1].sample.after.retired_frames(), Some(4));
    push(&mut ingress, 3, Some(1210));
    assert_eq!(
        read(&mut source, 1, Some(1220)).transferred_frames,
        0,
        "reprime, not stale two-frame retention"
    );
    push(&mut ingress, 1, Some(1220));
    assert_eq!(read(&mut source, 1, Some(1230)).transferred_frames, 1);
    assert_eq!(observer.snapshot().ingress_elapsed_recoveries, 1);
}

#[test]
fn ingress_elapsed_checks_priming_inventory_and_unknown_or_future_clocks() {
    for (published, observed) in [
        (Some(1000), Some(1101)),
        (None, Some(1050)),
        (Some(0), Some(1050)),
        (Some(1100), Some(1050)),
        (Some(1000), None),
        (Some(1000), Some(0)),
    ] {
        let (mut ingress, mut source, observer) = capture_bridge(config(Some(100))).unwrap();
        push(&mut ingress, 4, published);
        let report = read(&mut source, 1, observed);
        assert_eq!(report.transferred_frames, 0, "{published:?}/{observed:?}");
        assert!(report.discontinuity);
        assert_eq!(source.snapshot().fill_frames, 0);
        assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 4);
        assert!(!observer.queue_history().unwrap().saturated);
    }
}

#[test]
fn ingress_elapsed_at_exact_limit_is_fresh_but_none_preserves_legacy() {
    for (limit, now) in [
        (Some(100), Some(1100)),
        (None, None),
        (None, Some(u64::MAX)),
    ] {
        let (mut ingress, mut source, observer) = capture_bridge(config(limit)).unwrap();
        push(&mut ingress, 4, Some(1000));
        assert_eq!(read(&mut source, 1, now).transferred_frames, 1);
        assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 0);
    }
}

#[test]
fn ingress_elapsed_never_admits_publications_after_the_acquired_read_prefix() {
    let (mut ingress, mut source, observer) = capture_bridge(config(Some(100))).unwrap();
    push(&mut ingress, 4, Some(1000));
    let calls = Cell::new(0);
    let report = source
        .read_interleaved_with_publication_clock(&mut [0.0; 16], || {
            let call = calls.get();
            calls.set(call + 1);
            if call == 1 {
                push(&mut ingress, 4, Some(1050));
            }
            Some(1050)
        })
        .unwrap();
    assert_eq!(
        calls.get(),
        3,
        "one pre, one post-acquisition, one post-read observation"
    );
    assert_eq!(
        report.transferred_frames, 3,
        "new tail belongs to the next read"
    );
    assert_eq!(source.snapshot().fill_frames, 4);
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 0);
    assert!(!observer.queue_history().unwrap().saturated);
}

#[test]
fn ingress_elapsed_stale_recovery_does_not_discard_concurrent_new_publication() {
    let (mut ingress, mut source, observer) = capture_bridge(config(Some(100))).unwrap();
    push(&mut ingress, 4, Some(1000));
    let calls = Cell::new(0);
    let report = source
        .read_interleaved_with_publication_clock(&mut [0.0; 2], || {
            let call = calls.get();
            calls.set(call + 1);
            if call == 1 {
                push(&mut ingress, 4, Some(1200));
            }
            Some(1200)
        })
        .unwrap();
    assert_eq!(report.transferred_frames, 0);
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 4);
    assert_eq!(source.snapshot().fill_frames, 4);
    assert_eq!(read(&mut source, 1, Some(1201)).transferred_frames, 1);
    assert!(!observer.queue_history().unwrap().saturated);
}

#[test]
fn ingress_elapsed_checks_later_support_and_rejects_publication_clock_regression() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        trim_on_prime: false,
        ..config(Some(1000))
    })
    .unwrap();
    push(&mut ingress, 4, Some(1000));
    push(&mut ingress, 4, Some(999));
    let report = read(&mut source, 8, Some(1010));
    assert_eq!(
        report.transferred_frames, 3,
        "never interpolate into unknown/regressed publication"
    );
    assert!(report.discontinuity);
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 5);
    assert_eq!(source.snapshot().fill_frames, 0);
    let history = observer.queue_history().unwrap();
    assert!(!history.saturated);
    let sample = history.events[0].sample;
    assert_eq!(sample.after.retired_frames(), Some(8));
    assert_eq!(sample.ingress_elapsed_discarded_frames, 5);
    assert_eq!(sample.reset_discarded_frames, 0);
}

#[test]
fn ingress_elapsed_clock_regression_across_consumer_reads_cannot_look_fresh() {
    let (mut ingress, mut source, observer) = capture_bridge(config(Some(1000))).unwrap();
    push(&mut ingress, 4, Some(1000));
    assert_eq!(read(&mut source, 1, Some(1100)).transferred_frames, 1);
    assert_eq!(read(&mut source, 1, Some(1099)).transferred_frames, 0);
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 4);
    assert_eq!(source.snapshot().fill_frames, 0);
    push(&mut ingress, 4, Some(1101));
    assert_eq!(read(&mut source, 1, Some(1102)).transferred_frames, 1);
}

#[test]
fn ingress_elapsed_consumer_retains_post_read_clock_high_water() {
    let (mut ingress, mut source, observer) = capture_bridge(config(Some(100))).unwrap();
    push(&mut ingress, 4, Some(1000));
    assert_eq!(
        read_with_endpoints(&mut source, [1050, 1050, 2000]).transferred_frames,
        1
    );
    assert_eq!(
        read_with_endpoints(&mut source, [1100, 1100, 1100]).transferred_frames,
        0,
        "a later known post-read time cannot be forgotten"
    );
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 4);
    assert_eq!(source.snapshot().fill_frames, 0);
}

#[test]
fn ingress_elapsed_consumer_retains_high_pre_clock_from_rejected_bracket() {
    let (mut ingress, mut source, observer) = capture_bridge(config(Some(1000))).unwrap();
    push(&mut ingress, 4, Some(1000));
    assert_eq!(
        read_with_endpoints(&mut source, [2000, 1100, 1100]).transferred_frames,
        0
    );
    push(&mut ingress, 4, Some(1101));
    assert_eq!(
        read_with_endpoints(&mut source, [1200, 1200, 1200]).transferred_frames,
        0,
        "even an invalid bracket retains its highest known observation"
    );
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 8);
    assert_eq!(source.snapshot().fill_frames, 0);
}

#[test]
fn ingress_elapsed_consumer_rejects_regressed_pre_even_if_acquire_recovers() {
    let (mut ingress, mut source, observer) = capture_bridge(config(Some(2000))).unwrap();
    push(&mut ingress, 4, Some(1000));
    assert_eq!(
        read_with_endpoints(&mut source, [1050, 1050, 2000]).transferred_frames,
        1
    );
    assert_eq!(
        read_with_endpoints(&mut source, [1100, 2001, 2002]).transferred_frames,
        0
    );
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 4);
}

#[test]
fn ingress_elapsed_producer_retains_previous_publication_end_clock() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        trim_on_prime: false,
        ..config(Some(2000))
    })
    .unwrap();
    let mut endpoints = [1000, 2000].into_iter();
    ingress
        .push_packet_with_publication_clock(&[], packet(4), || endpoints.next())
        .unwrap();
    assert!(endpoints.next().is_none());
    push(&mut ingress, 4, Some(1100));
    let report = read(&mut source, 8, Some(2100));
    assert_eq!(
        report.transferred_frames, 3,
        "a publication regressed behind the previous completion has unknown residence"
    );
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 5);
    assert_eq!(source.snapshot().fill_frames, 0);
}

#[test]
fn ingress_elapsed_unknown_generation_support_flushes_prefix_and_fades_reprime() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        trim_on_prime: false,
        ..config(Some(1000))
    })
    .unwrap();
    push(&mut ingress, 4, Some(1000));
    ingress
        .push_packet_with_publication_clock(
            &[],
            CapturePacket {
                flags: SILENT | DISCONTINUITY,
                ..packet(4)
            },
            || Some(999),
        )
        .unwrap();
    let report = read(&mut source, 8, Some(1010));
    assert_eq!(report.transferred_frames, 3);
    assert!(report.discontinuity);
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 5);
    assert_eq!(observer.snapshot().ingress_elapsed_recoveries, 1);
    assert_eq!(source.snapshot().fill_frames, 0);
    let history = observer.queue_history().unwrap();
    assert!(!history.saturated);
    assert_eq!(history.events[0].sample.reset_discarded_frames, 0);
    assert_eq!(history.events[0].sample.ingress_elapsed_discarded_frames, 5);
    assert_eq!(history.events[0].sample.after.retired_frames(), Some(8));
    let bytes = (0..40)
        .flat_map(|index| {
            [10.0 + index as f32, -10.0 - index as f32]
                .into_iter()
                .flat_map(f32::to_le_bytes)
        })
        .collect::<Vec<_>>();
    ingress
        .push_packet_with_publication_clock(
            &bytes,
            CapturePacket {
                flags: 0,
                ..packet(40)
            },
            || Some(1020),
        )
        .unwrap();
    let mut output = [0.0; 64];
    assert_eq!(
        source
            .read_interleaved_with_publication_clock(&mut output, || Some(1021))
            .unwrap()
            .transferred_frames,
        32
    );
    for (index, frame) in output.as_chunks::<2>().0.iter().enumerate() {
        let expected = (10.0 + index as f32) * (index + 1) as f32 / 32.0;
        assert_eq!(*frame, [expected, -expected]);
    }
    assert_eq!(observer.snapshot().ingress_elapsed_recoveries, 1);
}

#[test]
fn ingress_elapsed_fresh_or_disabled_generation_change_preserves_legacy_reset() {
    for (limit, publication) in [(Some(1000), 1001), (None, 999)] {
        let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
            trim_on_prime: false,
            ..config(limit)
        })
        .unwrap();
        push(&mut ingress, 4, Some(1000));
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    flags: SILENT | DISCONTINUITY,
                    ..packet(4)
                },
                || Some(publication),
            )
            .unwrap();
        let report = read(&mut source, 8, Some(1010));
        assert_eq!(report.transferred_frames, 3);
        assert!(report.discontinuity);
        assert_eq!(source.snapshot().fill_frames, 3);
        assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 0);
        assert_eq!(observer.snapshot().ingress_elapsed_recoveries, 0);
        let history = observer.queue_history().unwrap();
        assert!(!history.saturated);
        assert_eq!(history.events[0].sample.reset_discarded_frames, 2);
    }
}

#[test]
fn ingress_elapsed_recovery_preserves_patterned_new_pcm_with_a_fade() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        target_fill_frames: 40,
        ..config(Some(100))
    })
    .unwrap();
    let bytes = |base: f32| {
        (0..40)
            .flat_map(|index| {
                [base + index as f32, -base - index as f32]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
            })
            .collect::<Vec<_>>()
    };
    let p = CapturePacket {
        flags: 0,
        ..packet(40)
    };
    ingress
        .push_packet_with_publication_clock(&bytes(1000.0), p, || Some(1000))
        .unwrap();
    assert_eq!(read(&mut source, 1, Some(1200)).transferred_frames, 0);
    ingress
        .push_packet_with_publication_clock(&bytes(10.0), p, || Some(1201))
        .unwrap();
    let mut output = [0.0; 64];
    assert_eq!(
        source
            .read_interleaved_with_publication_clock(&mut output, || Some(1202))
            .unwrap()
            .transferred_frames,
        32
    );
    for (index, frame) in output.as_chunks::<2>().0.iter().enumerate() {
        let expected = (10.0 + index as f32) * (index + 1) as f32 / 32.0;
        assert_eq!(*frame, [expected, -expected]);
    }
    assert_eq!(observer.snapshot().ingress_elapsed_discarded_frames, 40);
    assert!(source.oldest_frame_age(1202).is_none());
}
