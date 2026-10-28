use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    stats::{DISCONTINUITY, SILENT},
};

mod common;

fn config() -> ClockBridgeConfig {
    ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        trim_on_prime: false,
        detect_position_gaps: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    }
}

fn packet(frames: usize, flags: u32) -> CapturePacket {
    CapturePacket {
        frames,
        flags: flags | SILENT,
        device_position_frames: 0,
        qpc_100ns: 0,
    }
}

#[test]
fn queue_inventory_records_priming_and_exact_publication_retirement_frontiers() {
    let (mut ingress, mut source, observer) = capture_bridge(config()).unwrap();
    let empty = observer.queue_history().unwrap();
    assert!(empty.events.is_empty());
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(100))
        .unwrap();
    ingress.push_packet(&[], packet(8, 0)).unwrap();
    source
        .read_interleaved_at_qpc(&mut [0.0; 6], Some(200))
        .unwrap();
    let history = observer.queue_history().unwrap();
    assert_eq!(history.total_events, 2);
    let prime = history.events[0].sample;
    assert_eq!(prime.requested_frames, 1);
    assert_eq!(prime.output_frames, 0);
    assert_eq!(prime.before.pending_frames(), Some(0));
    let read = history.events[1].sample;
    assert_eq!(read.before.published_frames(), Some(8));
    assert_eq!(read.before.retired_frames(), Some(0));
    assert_eq!(read.after.published_frames(), Some(8));
    assert_eq!(read.after.retired_frames(), Some(2));
    assert_eq!(read.after.pending_frames(), Some(6));
    assert_eq!(read.after.removed_frames, 4);
    assert_eq!(read.after.held_frames, 2);
    assert_eq!(read.output_frames, 3);
    assert_eq!(read.qpc_before_100ns, Some(200));
    assert_eq!(history.unavailable_clocks, 0);
    assert!(!history.saturated);
}

#[test]
fn queue_inventory_accounts_prime_trim_and_reset_held_losses_separately() {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        trim_on_prime: true,
        ..config()
    })
    .unwrap();
    ingress.push_packet(&[], packet(10, 0)).unwrap();
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(100))
        .unwrap();
    ingress.push_packet(&[], packet(4, DISCONTINUITY)).unwrap();
    source
        .read_interleaved_at_qpc(&mut [0.0; 32], Some(200))
        .unwrap();
    let history = observer.queue_history().unwrap();
    let prime = history.events[0].sample;
    assert_eq!(prime.prime_discarded_frames, 6);
    assert_eq!(prime.after.removed_frames, 8);
    assert_eq!(prime.after.retired_frames(), Some(6));
    let reset = history.events[1].sample;
    assert_eq!(reset.resets, 1);
    assert_eq!(reset.reset_discarded_frames, 2);
    assert_eq!(reset.prime_discarded_frames, 0);
    assert_eq!(reset.after.published_frames(), Some(14));
    assert_eq!(reset.after.held_frames, 0);
}

#[test]
fn queue_inventory_publication_never_allocates_or_deallocates_during_audio() {
    let (mut ingress, mut source, observer) = capture_bridge(config()).unwrap();
    let output = &mut [0.0; 6];
    common::assert_allocates_nothing(|| {
        for stamp in 1..=300 {
            ingress.push_packet(&[], packet(3, 0)).unwrap();
            source.read_interleaved_at_qpc(output, Some(stamp)).unwrap();
        }
    });
    drop(ingress);
    drop(source);
    let history = observer.queue_history().unwrap();
    assert_eq!(history.total_events, 300);
    assert_eq!(history.overwritten_events, 44);
    assert_eq!(history.events.len(), 256);
    assert!(!history.saturated);
}

#[test]
fn queue_inventory_removals_follow_native_rate_not_output_frame_count() {
    for (input, output, removed) in [(96_000, 48_000, 6), (48_000, 96_000, 3)] {
        let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
            input_sample_rate: input,
            output_sample_rate: output,
            ..config()
        })
        .unwrap();
        ingress.push_packet(&[], packet(16, 0)).unwrap();
        source
            .read_interleaved_at_qpc(&mut [0.0; 6], Some(100))
            .unwrap();
        let history = observer.queue_history().unwrap();
        let sample = history.events[0].sample;
        assert_eq!(sample.output_frames, 3);
        assert_eq!(sample.after.removed_frames, removed);
        assert_eq!(sample.after.retired_frames(), Some(removed - 2));
        assert_eq!(sample.after.published_frames(), Some(16));
    }
}

#[test]
fn queue_inventory_does_not_count_dropped_new_tail_as_publication() {
    let (mut ingress, mut source, observer) = capture_bridge(config()).unwrap();
    assert_eq!(ingress.push_packet(&[], packet(40, 0)).unwrap(), 32);
    drop(ingress);
    let report = source
        .read_interleaved_at_qpc(&mut [0.0; 80], Some(100))
        .unwrap();
    assert_eq!(report.transferred_frames, 31);
    let history = observer.queue_history().unwrap();
    let sample = history.events[0].sample;
    assert_eq!(sample.before.published_frames(), Some(32));
    assert_eq!(sample.after.published_frames(), Some(32));
    assert_eq!(sample.after.retired_frames(), Some(32));
    assert_eq!(sample.reset_discarded_frames, 1);
    assert_eq!(sample.underrun_frames, 9);
    assert_eq!(observer.snapshot().dropped_frames, 8);
}

#[test]
fn queue_inventory_backlog_recovery_retains_next_without_double_counting_reset() {
    use moiren_windows_audio::latency::PreparedInputQueue;
    let input = PreparedInputQueue {
        capacity_frames: 32,
        prime_frames: 4,
        servo_reference_frames: 4,
        low_water_frames: 2,
        high_water_frames: 5,
        modeled_minimum_frames: Some(4),
    };
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        max_correction_ppm: 0.0,
        ..ClockBridgeConfig::with_prepared_input(48_000, 2, input)
    })
    .unwrap();
    ingress.push_packet(&[], packet(4, 0)).unwrap();
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(100))
        .unwrap();
    ingress.push_packet(&[], packet(1, 0)).unwrap();
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(200))
        .unwrap();
    let history = observer.queue_history().unwrap();
    let sample = history.events[1].sample;
    assert_eq!(sample.before.pending_frames(), Some(5));
    assert_eq!(sample.after.pending_frames(), Some(4));
    assert_eq!(sample.backlog_discarded_frames, 1);
    assert_eq!(sample.reset_discarded_frames, 0);
    assert_eq!(sample.resets, 1);
    assert_eq!(sample.after.retired_frames(), Some(1));
    assert!(!history.saturated);
}

#[test]
fn queue_inventory_age_recovery_keeps_cause_and_does_not_invent_sample_age() {
    use moiren_windows_audio::latency::PreparedInputQueue;
    let input = PreparedInputQueue {
        capacity_frames: 256,
        prime_frames: 64,
        servo_reference_frames: 64,
        low_water_frames: 16,
        high_water_frames: 240,
        modeled_minimum_frames: Some(64),
    };
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        age_limit_100ns: Some(10_000),
        ..ClockBridgeConfig::with_prepared_input(48_000, 2, input)
    })
    .unwrap();
    for (position, stamp) in [(0, 1_000_000), (64, 1_013_333)] {
        ingress
            .push_packet(
                &[],
                CapturePacket {
                    frames: 64,
                    flags: SILENT,
                    device_position_frames: position,
                    qpc_100ns: stamp,
                },
            )
            .unwrap();
    }
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(1_020_000))
        .unwrap();
    source
        .read_interleaved_at_qpc(&mut [0.0; 2], Some(1_033_333))
        .unwrap();
    let history = observer.queue_history().unwrap();
    let sample = history.events[1].sample;
    assert!(sample.age_discarded_frames > 0);
    assert_eq!(sample.reset_discarded_frames, 0);
    assert_eq!(sample.backlog_discarded_frames, 0);
    assert_eq!(sample.resets, 1);
    assert_eq!(sample.after.published_frames(), Some(128));
    assert!(!history.saturated);
}

#[test]
fn queue_inventory_history_storage_is_included_in_preparation_limit() {
    let result = capture_bridge(ClockBridgeConfig {
        // The 32-frame PCM/metadata ring fits; the fixed history does not.
        byte_budget: 8192,
        ..config()
    });
    assert!(matches!(
        result,
        Err(moiren_windows_audio::clock_bridge::ClockBridgeError::BudgetExceeded)
    ));
}

#[test]
fn queue_inventory_conserves_frontiers_during_concurrent_producer_commits() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 256,
        ..config()
    })
    .unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let writer = std::thread::spawn(move || {
        for _ in 0..20_000 {
            ingress.push_packet(&[], packet(7, 0)).unwrap();
            std::thread::yield_now();
        }
        finished.store(true, Ordering::Release);
    });
    let mut stamp = 0;
    while !done.load(Ordering::Acquire) {
        stamp += 1;
        source
            .read_interleaved_at_qpc(&mut [0.0; 10], Some(stamp))
            .unwrap();
    }
    writer.join().unwrap();
    for _ in 0..60 {
        stamp += 1;
        source
            .read_interleaved_at_qpc(&mut [0.0; 10], Some(stamp))
            .unwrap();
    }
    let history = observer.queue_history().unwrap();
    assert!(!history.saturated, "{history:?}");
    let final_boundary = history.events.last().unwrap().sample.after;
    assert_eq!(
        final_boundary.published_frames(),
        Some(observer.snapshot().written_frames)
    );
    let mut previous = None;
    for event in history.events {
        let sample = event.sample;
        assert!(sample.after.retired_frames() >= sample.before.retired_frames());
        assert!(sample.after.published_frames() >= sample.before.published_frames());
        if let Some(before) = previous {
            assert!(sample.before.published_frames().unwrap() >= before);
        }
        previous = sample.after.published_frames();
    }
}
