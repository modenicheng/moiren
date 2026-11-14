use moiren_windows_audio::{
    clock_bridge::{CaptureIngress, CapturePacket, ClockBridgeConfig, capture_bridge},
    latency::interpolation::{
        InterpolationHistorySnapshot, InterpolationTraceCollector, InterpolationTraceRefusal,
        MAX_TRACE_EVENTS,
    },
};

fn bridge_config() -> ClockBridgeConfig {
    ClockBridgeConfig {
        capacity_frames: 2048,
        target_fill_frames: 4,
        trim_on_prime: false,
        max_correction_ppm: 0.0,
        ..Default::default()
    }
}

fn push_silence(ingress: &mut CaptureIngress) {
    ingress
        .push_packet(
            &[0; 8192],
            CapturePacket {
                frames: 1024,
                flags: 0,
                device_position_frames: 0,
                qpc_100ns: 1,
            },
        )
        .unwrap();
}

fn closed_history(reads: usize) -> InterpolationHistorySnapshot {
    let (mut ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    push_silence(&mut ingress);
    for _ in 0..reads {
        assert_eq!(
            source
                .read_interleaved(&mut [0.0; 2])
                .unwrap()
                .transferred_frames,
            1
        );
    }
    drop(source);
    let history = observer.interpolation_history().unwrap();
    assert_eq!(history.total_events, reads as u64);
    assert_eq!(history.started_reads, reads as u64);
    assert_eq!(history.completed_reads, reads as u64);
    assert!(!history.saturated);
    assert!(history.writer_finished);
    history
}

fn assert_refused(archive: &InterpolationTraceCollector, refusal: InterpolationTraceRefusal) {
    let report = archive.artifact();
    assert_eq!(report.refusal, Some(refusal), "{report:?}");
    assert!(!report.history_complete, "{report:?}");
    assert!(!report.qualified, "{report:?}");
}

#[test]
fn completed_read_cannot_gain_late_spans_in_a_later_snapshot() {
    let mut initial = closed_history(1);
    initial.writer_finished = false;
    initial.requested_frames = 32;
    let mut later = initial.clone();
    let mut appended = later.events[0];
    appended.sequence = 2;
    appended.span.output_start_frame = 1;
    appended.span.input_start_frame = 1;
    later.events.push(appended);
    later.total_events = 2;
    later.writer_finished = true;
    let mut archive = InterpolationTraceCollector::new(32).unwrap();
    archive.observe(initial);
    archive.drain_chunk().unwrap();
    archive.observe(later);
    archive.finish();
    assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
}

#[test]
fn request_frontier_can_grow_only_with_enough_new_nonempty_reads() {
    let mut initial = closed_history(1);
    initial.writer_finished = false;
    initial.requested_frames = 10;
    for (requested, reads) in [(11, 1), (11, 3), (10, 2)] {
        let mut later = initial.clone();
        later.requested_frames = requested;
        later.started_reads = reads;
        later.completed_reads = reads;
        later.writer_finished = true;
        let mut archive = InterpolationTraceCollector::new(32).unwrap();
        archive.observe(initial.clone());
        archive.observe(later);
        archive.finish();
        assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
    }
}

#[test]
fn draining_actual_cursor_chunks_retains_sequence_one_beyond_the_pending_limit() {
    let (mut ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    push_silence(&mut ingress);
    let mut archive = InterpolationTraceCollector::new(128).unwrap();
    let mut chunks = Vec::new();
    for _ in 0..6 {
        for _ in 0..100 {
            source.read_interleaved(&mut [0.0; 2]).unwrap();
        }
        archive.observe(observer.interpolation_history().unwrap());
        chunks.push(archive.drain_chunk().unwrap());
        assert_eq!(archive.artifact().retained_events, 0);
    }
    drop(source);
    archive.observe(observer.interpolation_history().unwrap());
    archive.finish();
    let report = archive.artifact();
    assert!(report.history_complete, "{report:?}");
    assert!(report.refusal.is_none());
    assert_eq!(report.drained_events, 600);
    assert_eq!(report.total_events, 600);
    assert_eq!(report.requested_frames, 600);
    assert_eq!(report.started_reads, 600);
    assert_eq!(report.completed_reads, 600);
    assert_eq!(report.drained_chunks, 6);
    assert_eq!(report.missing_events, 0);
    assert_eq!(report.unretained_events, 0);
    assert_eq!(chunks.len(), 6);
    for (index, event) in chunks.iter().flat_map(|chunk| &chunk.events).enumerate() {
        assert_eq!(event.sequence, index as u64 + 1);
        assert_eq!(event.span.output_start_frame, index as u64);
    }
    assert!(!report.qualified);
}

#[test]
fn first_observation_after_300_actual_events_keeps_44_missing_events_sticky_after_drain() {
    let (mut ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    push_silence(&mut ingress);
    for _ in 0..300 {
        assert_eq!(
            source
                .read_interleaved(&mut [0.0; 2])
                .unwrap()
                .transferred_frames,
            1
        );
    }
    let history = observer.interpolation_history().unwrap();
    assert_eq!(history.total_events, 300);
    assert_eq!(history.overwritten_events, 44);
    assert_eq!(history.events.len(), 256);
    assert_eq!(history.events.first().unwrap().sequence, 45);
    assert_eq!(history.events.last().unwrap().sequence, 300);

    let mut archive = InterpolationTraceCollector::new(300).unwrap();
    archive.observe(history.clone());
    archive.observe(history);
    assert_eq!(archive.artifact().missing_events, 44);
    assert_eq!(archive.artifact().total_events, 300);
    assert_eq!(archive.artifact().retained_events, 256);
    assert_refused(&archive, InterpolationTraceRefusal::MissingEvents);
    let first = archive.drain_chunk().unwrap();
    assert_eq!((first.schema_version, first.sequence), (1, 1));
    assert_eq!(first.events.len(), 256);

    for _ in 0..100 {
        assert_eq!(
            source
                .read_interleaved(&mut [0.0; 2])
                .unwrap()
                .transferred_frames,
            1
        );
    }
    drop(source);
    archive.observe(observer.interpolation_history().unwrap());
    let second = archive.drain_chunk().unwrap();
    assert_eq!(second.sequence, 2);
    assert_eq!(second.events.len(), 100);
    assert_eq!(second.events.first().unwrap().sequence, 301);
    assert_eq!(second.events.last().unwrap().sequence, 400);
    assert!(archive.drain_chunk().is_none());
    archive.finish();
    let report = archive.artifact();
    assert_eq!(report.missing_events, 44);
    assert_eq!(report.unretained_events, 0);
    assert_eq!(report.total_events, 400);
    assert_eq!(report.drained_events, 356);
    assert_eq!(report.drained_chunks, 2);
    assert_eq!(report.retained_events, 0);
    assert_eq!(report.requested_frames, 400);
    assert_eq!((report.started_reads, report.completed_reads), (400, 400));
    assert!(report.writer_finished);
    assert!(report.finalized);
    assert_refused(&archive, InterpolationTraceRefusal::MissingEvents);
}

#[test]
fn limit_eight_counts_unretained_events_once_and_drain_does_not_recover_them() {
    let (mut ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    push_silence(&mut ingress);
    let mut archive = InterpolationTraceCollector::new(8).unwrap();
    for _ in 0..8 {
        source.read_interleaved(&mut [0.0; 2]).unwrap();
    }
    archive.observe(observer.interpolation_history().unwrap());
    assert_eq!(archive.artifact().retained_events, 8);
    assert_eq!(archive.artifact().unretained_events, 0);
    assert!(archive.artifact().refusal.is_none());
    for _ in 0..4 {
        source.read_interleaved(&mut [0.0; 2]).unwrap();
    }
    let history = observer.interpolation_history().unwrap();
    archive.observe(history.clone());
    archive.observe(history);
    assert_eq!(archive.artifact().retained_events, 8);
    assert_eq!(archive.artifact().unretained_events, 4);
    assert_eq!(archive.artifact().missing_events, 0);
    assert_refused(&archive, InterpolationTraceRefusal::ArchiveLimit);
    let first = archive.drain_chunk().unwrap();
    assert_eq!(first.sequence, 1);
    assert_eq!(
        first
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );

    // A drain frees pending capacity, but cannot recreate events 9 through 12.
    archive.observe(observer.interpolation_history().unwrap());
    assert!(archive.drain_chunk().is_none());
    for _ in 0..10 {
        source.read_interleaved(&mut [0.0; 2]).unwrap();
    }
    drop(source);
    archive.observe(observer.interpolation_history().unwrap());
    assert_eq!(archive.artifact().unretained_events, 6);
    let second = archive.drain_chunk().unwrap();
    assert_eq!(second.sequence, 2);
    assert_eq!(
        second
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (13..=20).collect::<Vec<_>>()
    );
    archive.observe(observer.interpolation_history().unwrap());
    archive.finish();
    let report = archive.artifact();
    assert_eq!(report.event_limit, 8);
    assert_eq!(report.total_events, 22);
    assert_eq!(report.drained_events, 16);
    assert_eq!(report.drained_chunks, 2);
    assert_eq!(report.retained_events, 0);
    assert_eq!(report.unretained_events, 6);
    assert_eq!(report.missing_events, 0);
    assert!(report.finalized);
    assert_refused(&archive, InterpolationTraceRefusal::ArchiveLimit);
}

#[test]
fn changed_drained_overlap_refuses_before_appending_new_events() {
    let (mut ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    push_silence(&mut ingress);
    let mut archive = InterpolationTraceCollector::new(8).unwrap();
    for _ in 0..4 {
        source.read_interleaved(&mut [0.0; 2]).unwrap();
    }
    archive.observe(observer.interpolation_history().unwrap());
    let chunk = archive.drain_chunk().unwrap();
    for _ in 0..4 {
        source.read_interleaved(&mut [0.0; 2]).unwrap();
    }
    drop(source);
    let valid = observer.interpolation_history().unwrap();
    let mut changed = valid.clone();
    changed.events[0].span.phase = 0.5;
    archive.observe(changed);
    let report = archive.artifact();
    assert_eq!(report.total_events, 4);
    assert_eq!(report.requested_frames, 4);
    assert_eq!(report.drained_events, 4);
    assert_eq!(report.retained_events, 0);
    assert!(report.events.is_empty());
    assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);

    archive.observe(valid.clone());
    archive.observe(valid);
    assert_eq!(archive.artifact().total_events, 8);
    assert_eq!(archive.artifact().retained_events, 4);
    assert_eq!(archive.artifact().events[0].sequence, 5);
    assert_eq!(chunk.events[0].span.phase, 0.0);
    archive.finish();
    assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
}

#[test]
fn changed_pending_overlap_does_not_partially_append_a_valid_suffix() {
    let mut prefix = closed_history(2);
    prefix.writer_finished = false;
    let mut changed = closed_history(4);
    changed.events[1].span.step = 1.25;
    let mut archive = InterpolationTraceCollector::new(8).unwrap();
    archive.observe(prefix.clone());
    archive.observe(changed);
    assert_eq!(archive.artifact().events, prefix.events);
    assert_eq!(archive.artifact().retained_events, 2);
    assert_eq!(archive.artifact().total_events, 2);
    assert_eq!(archive.artifact().requested_frames, 2);
    archive.finish();
    assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
}

#[test]
fn invalid_limits_are_rejected_and_both_inclusive_bounds_are_accepted() {
    for limit in [0, MAX_TRACE_EVENTS + 1, usize::MAX] {
        assert!(InterpolationTraceCollector::new(limit).is_err(), "{limit}");
    }
    for limit in [1, MAX_TRACE_EVENTS] {
        let mut archive = InterpolationTraceCollector::new(limit).unwrap();
        let report = archive.artifact();
        assert_eq!(report.schema_version, 1);
        assert_eq!(report.event_limit, limit);
        assert_eq!(report.retained_events, 0);
        assert_eq!(report.drained_events, 0);
        assert_eq!(report.drained_chunks, 0);
        assert_eq!(report.total_events, 0);
        assert!(report.events.is_empty());
        assert!(report.refusal.is_none());
        assert!(!report.finalized);
        assert!(!report.history_complete);
        assert!(!report.qualified);
        assert!(archive.drain_chunk().is_none());
        archive.finish();
        assert!(archive.artifact().finalized);
        assert_refused(&archive, InterpolationTraceRefusal::OpenWriter);
    }
}

#[test]
fn empty_and_invalid_actual_reads_do_not_manufacture_archive_reads() {
    let (_ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    assert_eq!(
        source.read_interleaved(&mut []).unwrap().transferred_frames,
        0
    );
    assert!(source.read_interleaved(&mut [0.0]).is_err());
    drop(source);
    let history = observer.interpolation_history().unwrap();
    assert_eq!(history.requested_frames, 0);
    assert_eq!((history.started_reads, history.completed_reads), (0, 0));
    assert!(history.events.is_empty());
    let mut archive = InterpolationTraceCollector::new(1).unwrap();
    archive.observe(history);
    archive.finish();
    assert!(archive.artifact().writer_finished);
    assert!(archive.artifact().finalized);
    assert!(archive.drain_chunk().is_none());
    assert_refused(&archive, InterpolationTraceRefusal::NoReads);
}

#[test]
fn open_writer_cannot_be_closed_by_finish_and_finalization_is_immutable() {
    let (mut ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    push_silence(&mut ingress);
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    let mut archive = InterpolationTraceCollector::new(4).unwrap();
    archive.observe(observer.interpolation_history().unwrap());
    archive.finish();
    assert_refused(&archive, InterpolationTraceRefusal::OpenWriter);
    let finalized = serde_json::to_value(archive.artifact()).unwrap();
    source.read_interleaved(&mut [0.0; 2]).unwrap();
    drop(source);
    archive.observe(observer.interpolation_history().unwrap());
    archive.finish();
    assert_eq!(serde_json::to_value(archive.artifact()).unwrap(), finalized);
    let chunk = archive.drain_chunk().unwrap();
    assert_eq!(chunk.events.len(), 1);
    assert!(archive.artifact().finalized);
    assert_refused(&archive, InterpolationTraceRefusal::OpenWriter);
}

#[test]
fn closed_writer_completes_structural_history_and_can_be_drained_after_finish() {
    let history = closed_history(8);
    let mut archive = InterpolationTraceCollector::new(8).unwrap();
    archive.observe(history.clone());
    archive.observe(history.clone());
    assert!(archive.artifact().refusal.is_none());
    assert!(!archive.artifact().history_complete);
    archive.finish();
    let report = archive.artifact();
    assert!(report.writer_finished);
    assert!(report.finalized);
    assert!(report.history_complete, "{report:?}");
    assert!(!report.qualified);
    assert_eq!(report.total_events, 8);
    assert_eq!(report.retained_events, 8);
    assert_eq!(report.events, history.events);
    let chunk = archive.drain_chunk().unwrap();
    assert_eq!((chunk.schema_version, chunk.sequence), (1, 1));
    assert_eq!(chunk.events, history.events);
    archive.finish();
    let report = archive.artifact();
    assert!(report.history_complete, "{report:?}");
    assert_eq!(report.drained_events, 8);
    assert_eq!(report.retained_events, 0);
    assert_eq!(report.drained_chunks, 1);
    assert!(archive.drain_chunk().is_none());
}

#[test]
fn terminal_priming_read_closes_without_fabricated_emitted_events() {
    let (_ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 66])
            .unwrap()
            .transferred_frames,
        0
    );
    let mut archive = InterpolationTraceCollector::new(1).unwrap();
    archive.observe(observer.interpolation_history().unwrap());
    assert!(archive.artifact().events.is_empty());
    drop(source);
    archive.observe(observer.interpolation_history().unwrap());
    archive.finish();
    let report = archive.artifact();
    assert_eq!(report.requested_frames, 33);
    assert_eq!((report.started_reads, report.completed_reads), (1, 1));
    assert_eq!(report.total_events, 0);
    assert_eq!(report.retained_events, 0);
    assert_eq!(report.missing_events, 0);
    assert_eq!(report.unretained_events, 0);
    assert!(report.history_complete, "{report:?}");
    assert!(report.refusal.is_none());
    assert!(!report.qualified);
    assert!(archive.drain_chunk().is_none());
}

#[test]
fn incomplete_read_is_refused_even_when_all_emitted_events_are_present() {
    let mut history = closed_history(2);
    history.completed_reads = 1;
    let mut archive = InterpolationTraceCollector::new(2).unwrap();
    archive.observe(history);
    assert!(archive.artifact().refusal.is_none());
    assert_eq!(archive.artifact().retained_events, 2);
    archive.finish();
    assert_refused(&archive, InterpolationTraceRefusal::IncompleteRead);
}

#[test]
fn malformed_event_payloads_and_history_frontiers_are_refused_without_retention() {
    type Mutation = fn(&mut InterpolationHistorySnapshot);
    let cases: &[(&str, Mutation)] = &[
        ("short suffix", |h| {
            h.events.pop();
        }),
        ("long suffix", |h| h.events.push(h.events[1])),
        ("wrong total", |h| h.total_events += 1),
        ("wrong overwritten count", |h| h.overwritten_events = 1),
        ("completed beyond started", |h| h.completed_reads += 1),
        ("multiple unfinished reads", |h| h.completed_reads = 0),
        ("requested below started", |h| h.requested_frames = 1),
        ("zero event sequence", |h| h.events[0].sequence = 0),
        ("duplicate event sequence", |h| h.events[1].sequence = 1),
        ("reordered events", |h| h.events.swap(0, 1)),
        ("zero read sequence", |h| h.events[0].span.read_sequence = 0),
        ("read beyond frontier", |h| {
            h.events[1].span.read_sequence = 3
        }),
        ("zero span", |h| h.events[0].span.frames = 0),
        ("oversized span", |h| h.events[0].span.frames = 33),
        ("nan phase", |h| h.events[0].span.phase = f64::NAN),
        ("infinite phase", |h| h.events[0].span.phase = f64::INFINITY),
        ("negative phase", |h| h.events[0].span.phase = -0.25),
        ("phase at upper bound", |h| h.events[0].span.phase = 1.0),
        ("nan step", |h| h.events[0].span.step = f64::NAN),
        ("infinite step", |h| h.events[0].span.step = f64::INFINITY),
        ("zero step", |h| h.events[0].span.step = 0.0),
        ("negative step", |h| h.events[0].span.step = -1.0),
        ("oversized fade", |h| h.events[0].span.fade_remaining = 33),
        ("span beyond requested frontier", |h| {
            h.events[1].span.output_start_frame = 2
        }),
        ("span end overflow", |h| {
            h.requested_frames = u64::MAX;
            h.events[1].span.output_start_frame = u64::MAX;
        }),
        ("overlapping output", |h| {
            h.events[1].span.output_start_frame = 0
        }),
        ("decreasing read order", |h| {
            h.events[0].span.read_sequence = 2;
            h.events[1].span.read_sequence = 1;
        }),
        ("same-read output gap", |h| {
            h.requested_frames = 3;
            h.events[1].span.read_sequence = 1;
            h.events[1].span.output_start_frame = 2;
        }),
    ];
    let valid = closed_history(2);
    for &(name, mutate) in cases {
        let mut malformed = valid.clone();
        mutate(&mut malformed);
        let mut archive = InterpolationTraceCollector::new(8).unwrap();
        archive.observe(malformed);
        let report = archive.artifact();
        assert_eq!(
            report.refusal,
            Some(InterpolationTraceRefusal::InvalidHistory),
            "{name}: {report:?}"
        );
        assert_eq!(report.total_events, 0, "{name}");
        assert_eq!(report.retained_events, 0, "{name}");
        assert_eq!(report.requested_frames, 0, "{name}");
        assert!(report.events.is_empty(), "{name}");
        archive.observe(valid.clone());
        assert_eq!(archive.artifact().total_events, 2, "{name}");
        archive.finish();
        assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
    }
}

#[test]
fn regressing_frontiers_preserve_the_last_valid_snapshot_and_refusal_is_sticky() {
    type Mutation = fn(&mut InterpolationHistorySnapshot);
    let cases: &[(&str, u64, Mutation)] = &[
        ("event total", 2, |h| {
            h.events.pop();
            h.total_events = 1;
        }),
        ("requested frames", 2, |h| h.requested_frames = 4),
        ("started reads", 2, |h| h.started_reads = 2),
        ("completed reads", 3, |h| h.completed_reads = 2),
    ];
    // Silent reads can advance request/read frontiers without emitting a span.
    let mut previous = closed_history(2);
    previous.writer_finished = false;
    previous.requested_frames = 5;
    previous.started_reads = 3;
    for &(name, completed, mutate) in cases {
        previous.completed_reads = completed;
        let mut archive = InterpolationTraceCollector::new(8).unwrap();
        archive.observe(previous.clone());
        assert!(archive.artifact().refusal.is_none(), "{name}");
        let mut regressed = previous.clone();
        mutate(&mut regressed);
        // Each input is valid in isolation: only comparison with the previous
        // observation establishes that its frontier moved backward.
        let mut standalone = InterpolationTraceCollector::new(8).unwrap();
        standalone.observe(regressed.clone());
        assert!(standalone.artifact().refusal.is_none(), "{name}");
        archive.observe(regressed);
        let report = archive.artifact();
        assert_eq!(
            report.refusal,
            Some(InterpolationTraceRefusal::InvalidHistory),
            "{name}: {report:?}"
        );
        assert_eq!(report.total_events, 2, "{name}");
        assert_eq!(report.requested_frames, 5, "{name}");
        assert_eq!(
            (report.started_reads, report.completed_reads),
            (3, completed),
            "{name}"
        );
        assert_eq!(report.events, previous.events, "{name}");
        let mut closed = previous.clone();
        closed.completed_reads = 3;
        closed.writer_finished = true;
        archive.observe(closed);
        archive.finish();
        assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
    }
}

#[test]
fn changed_closed_writer_snapshot_is_invalid_but_identical_reobservation_is_allowed() {
    let history = closed_history(2);
    for reopen in [false, true] {
        let mut archive = InterpolationTraceCollector::new(8).unwrap();
        archive.observe(history.clone());
        archive.observe(history.clone());
        assert!(archive.artifact().refusal.is_none());
        let mut changed = history.clone();
        if reopen {
            changed.writer_finished = false;
        } else {
            changed.requested_frames += 1;
        }
        archive.observe(changed);
        assert_eq!(archive.artifact().requested_frames, 2);
        assert!(archive.artifact().writer_finished);
        assert_eq!(archive.artifact().events, history.events);
        archive.finish();
        assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
    }
}

#[test]
fn saturated_history_stays_refused_after_drain_and_a_clear_marker_is_rejected() {
    let mut history = closed_history(2);
    history.writer_finished = false;
    history.saturated = true;
    let mut archive = InterpolationTraceCollector::new(2).unwrap();
    archive.observe(history.clone());
    assert_eq!(archive.artifact().retained_events, 2);
    assert_refused(&archive, InterpolationTraceRefusal::Saturated);
    archive.drain_chunk().unwrap();
    history.saturated = false;
    history.requested_frames += 1;
    archive.observe(history);
    assert_eq!(archive.artifact().total_events, 2);
    assert_eq!(archive.artifact().drained_events, 2);
    assert_eq!(archive.artifact().requested_frames, 2);
    assert!(archive.artifact().events.is_empty());
    archive.finish();
    assert_refused(&archive, InterpolationTraceRefusal::Saturated);
}

#[test]
fn adjacent_nonoverlapping_actual_windows_close_across_a_drained_boundary() {
    let mut first = closed_history(256);
    first.writer_finished = false;
    let second = closed_history(512);
    assert_eq!(first.events.last().unwrap().sequence, 256);
    assert_eq!(second.events.first().unwrap().sequence, 257);
    let mut archive = InterpolationTraceCollector::new(256).unwrap();
    archive.observe(first.clone());
    let first_chunk = archive.drain_chunk().unwrap();
    archive.observe(second.clone());
    let second_chunk = archive.drain_chunk().unwrap();
    archive.finish();
    assert_eq!(first_chunk.events, first.events);
    assert_eq!(second_chunk.events, second.events);
    let report = archive.artifact();
    assert_eq!(report.total_events, 512);
    assert_eq!(report.drained_events, 512);
    assert_eq!(report.drained_chunks, 2);
    assert_eq!(report.retained_events, 0);
    assert_eq!(report.missing_events, 0);
    assert_eq!(report.unretained_events, 0);
    assert!(report.history_complete, "{report:?}");
    assert!(report.refusal.is_none());
    assert!(!report.qualified);
}

#[test]
fn adjacent_window_edge_order_is_validated_without_a_shared_event() {
    type Mutation = fn(&mut InterpolationHistorySnapshot);
    let cases: &[(&str, Mutation)] = &[
        ("backward read sequence", |h| {
            h.events[0].span.read_sequence = 255;
        }),
        ("output overlaps previous last span", |h| {
            h.events[0].span.output_start_frame = 255;
        }),
        ("gap within previous last read", |h| {
            h.events[0].span.read_sequence = 256;
            h.requested_frames += 1;
            for event in &mut h.events {
                event.span.output_start_frame += 1;
            }
        }),
    ];
    let mut first = closed_history(256);
    first.writer_finished = false;
    let second = closed_history(512);
    for &(name, mutate) in cases {
        let mut archive = InterpolationTraceCollector::new(256).unwrap();
        archive.observe(first.clone());
        archive.drain_chunk().unwrap();
        let mut malformed = second.clone();
        mutate(&mut malformed);
        archive.observe(malformed);
        let report = archive.artifact();
        assert_eq!(
            report.refusal,
            Some(InterpolationTraceRefusal::InvalidHistory),
            "{name}: {report:?}"
        );
        assert_eq!(report.total_events, 256, "{name}");
        assert_eq!(report.requested_frames, 256, "{name}");
        assert_eq!(report.drained_events, 256, "{name}");
        assert_eq!(report.retained_events, 0, "{name}");
        assert_eq!(report.missing_events, 0, "{name}");
        assert!(archive.drain_chunk().is_none(), "{name}");
        archive.observe(second.clone());
        archive.finish();
        assert_eq!(archive.artifact().total_events, 512, "{name}");
        assert_refused(&archive, InterpolationTraceRefusal::InvalidHistory);
    }
}

#[test]
fn adjacent_window_accepts_contiguous_same_read_or_silent_gap_between_reads() {
    let mut first = closed_history(256);
    first.writer_finished = false;
    for same_read in [false, true] {
        let mut first = first.clone();
        let mut second = closed_history(512);
        if same_read {
            first.completed_reads = 255;
            first.requested_frames = 257;
            second.events[0].span.read_sequence = 256;
            for event in &mut second.events[1..] {
                event.span.read_sequence -= 1;
            }
            second.started_reads -= 1;
            second.completed_reads -= 1;
        } else {
            // Different reads may leave requested but unfilled output gaps.
            second.requested_frames += 3;
            for event in &mut second.events {
                event.span.output_start_frame += 3;
            }
        }
        let requested_frames = second.requested_frames;
        let mut archive = InterpolationTraceCollector::new(256).unwrap();
        archive.observe(first.clone());
        archive.drain_chunk().unwrap();
        archive.observe(second);
        archive.finish();
        let report = archive.artifact();
        assert!(report.history_complete, "same_read={same_read}: {report:?}");
        assert!(report.refusal.is_none());
        assert_eq!(report.total_events, 512);
        assert_eq!(report.requested_frames, requested_frames);
        assert_eq!(report.drained_events, 256);
        assert_eq!(report.retained_events, 256);
        assert!(!report.qualified);
    }
}

#[test]
fn final_actual_priming_read_advances_frontiers_without_repeating_drained_spans() {
    let (mut ingress, mut source, observer) = capture_bridge(bridge_config()).unwrap();
    push_silence(&mut ingress);
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 2048])
            .unwrap()
            .transferred_frames,
        1023
    );
    let history = observer.interpolation_history().unwrap();
    assert_eq!(history.total_events, 32);
    let mut archive = InterpolationTraceCollector::new(32).unwrap();
    archive.observe(history.clone());
    let chunk = archive.drain_chunk().unwrap();
    assert_eq!(chunk.events, history.events);
    assert_eq!(
        source
            .read_interleaved(&mut [0.0; 10])
            .unwrap()
            .transferred_frames,
        0
    );
    drop(source);
    let terminal = observer.interpolation_history().unwrap();
    assert_eq!(terminal.events, history.events);
    assert_eq!(terminal.requested_frames, 1029);
    assert_eq!((terminal.started_reads, terminal.completed_reads), (2, 2));
    archive.observe(terminal);
    archive.finish();
    let report = archive.artifact();
    assert_eq!(report.requested_frames, 1029);
    assert_eq!((report.started_reads, report.completed_reads), (2, 2));
    assert_eq!(report.total_events, 32);
    assert_eq!(report.drained_events, 32);
    assert_eq!(report.retained_events, 0);
    assert!(report.history_complete, "{report:?}");
    assert!(report.refusal.is_none());
    assert!(!report.qualified);
    assert!(archive.drain_chunk().is_none());
}
