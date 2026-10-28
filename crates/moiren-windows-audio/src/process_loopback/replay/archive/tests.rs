use crate::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    process_loopback::{
        ProcessBridgeOptions, ProcessIdentity, ProcessReplayError, ProcessTraceArchive,
        ProcessTraceCollectionRefusal, ProcessTraceCollector, ProcessTraceDocument,
        ProcessTraceOrigin,
    },
    stats::SILENT,
};

fn process() -> ProcessIdentity {
    ProcessIdentity {
        pid: 42,
        creation_time_100ns: 123,
        executable_name: "source.exe".into(),
    }
}

fn collected(count: usize, poll_every: usize) -> ProcessTraceArchive {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 32,
        target_fill_frames: 4,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    let mut collector = ProcessTraceCollector::new(process(), 7, 48_000, 48_000, 4);
    collector.poll(&observer);
    for n in 0..count {
        let stamp = 1000 + n as u64 * 100;
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 4,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(stamp),
            )
            .unwrap();
        source
            .read_interleaved_with_publication_clock(&mut [0.0; 8], || Some(stamp + 10))
            .unwrap();
        if (n + 1).is_multiple_of(poll_every) {
            collector.poll(&observer);
        }
    }
    collector.poll(&observer);
    assert!(!collector.artifact().packet_history.writer_finished);
    assert!(!collector.artifact().queue_history.writer_finished);
    drop(ingress);
    drop(source);
    collector.poll(&observer);
    collector.artifact()
}

#[test]
fn archive_retains_exact_sequence_one_across_overlapping_suffixes_and_final_closure() {
    let artifact = collected(600, 64);
    assert_eq!(artifact.collection_refusal, None);
    assert_eq!(artifact.packet_history.events.len(), 600);
    assert_eq!(artifact.queue_history.events.len(), 600);
    assert_eq!(artifact.packet_history.overwritten_events, 0);
    assert_eq!(artifact.packet_history.offered_frames, 2400);
    assert!(artifact.packet_history.writer_finished && artifact.queue_history.writer_finished);
    let report = artifact
        .trace(&process(), 7)
        .unwrap()
        .replay(
            &process(),
            7,
            ProcessBridgeOptions {
                max_demand_frames: 4,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!((report.packet_events, report.read_events), (600, 600));
    assert_eq!(report.baseline.requested_frames, 2400);
    assert!(!report.qualified);
    assert_eq!(report.trace_origin, ProcessTraceOrigin::CallerSupplied);
}

#[test]
fn archive_latches_missed_poll_without_losing_final_closure_diagnostics() {
    let artifact = collected(600, 300);
    assert_eq!(
        artifact.collection_refusal,
        Some(ProcessTraceCollectionRefusal::MissedEvents)
    );
    assert!(artifact.packet_history.writer_finished && artifact.queue_history.writer_finished);
    assert_eq!(artifact.packet_history.total_events, 600);
    assert!(artifact.trace(&process(), 7).is_err());
}

#[test]
fn archive_limit_refuses_instead_of_silently_restarting_an_empty_candidate() {
    let artifact = collected(8320, 64);
    assert_eq!(
        artifact.collection_refusal,
        Some(ProcessTraceCollectionRefusal::EventLimit)
    );
    assert!(artifact.packet_history.events.len() <= 8192);
    assert!(artifact.queue_history.events.len() <= 8192);
    assert_eq!(artifact.packet_history.total_events, 8320);
    assert!(artifact.packet_history.writer_finished && artifact.queue_history.writer_finished);
    assert!(artifact.trace(&process(), 7).is_err());
}

#[test]
fn archive_deserialization_is_strict_bounded_and_keeps_imported_origin() {
    let artifact = collected(600, 64);
    let value = serde_json::to_value(&artifact).unwrap();
    let decoded: ProcessTraceDocument = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded.process(), &process());
    assert_eq!(decoded.owner_generation(), 7);
    assert!(decoded.trace(&process(), 7).is_ok());
    for field in [
        "collection_refusal",
        "schema_version",
        "packet_history",
        "owner_generation",
    ] {
        let mut invalid = value.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<ProcessTraceArchive>(invalid).is_err(),
            "{field}"
        );
    }
    for history in ["packet_history", "queue_history"] {
        let mut invalid = value.clone();
        invalid[history]["events"] =
            serde_json::json!(vec![value[history]["events"][0].clone(); 8193]);
        assert!(serde_json::from_value::<ProcessTraceArchive>(invalid).is_err());
    }
    let mut invalid = value;
    invalid["trace_origin"] = "retained_native_owner".into();
    assert!(serde_json::from_value::<ProcessTraceDocument>(invalid).is_err());
}

#[test]
fn archive_canonical_validation_catches_early_events_outside_the_final_native_suffix() {
    let original = collected(600, 64);
    for edit in 0..5 {
        let mut a = original.clone();
        match edit {
            0 => a.packet_history.events[0].published_before_frames += 1,
            1 => a.queue_history.events[10].sample.after.removed_frames += 1,
            2 => a.packet_history.events[280].sequence += 1,
            3 => a.queue_history.events[280].sample.before.held_frames += 1,
            _ => a.packet_history.published_frames += 1,
        }
        assert!(a.trace(&process(), 7).is_err(), "edit={edit}");
    }
}

#[test]
fn archive_conflicting_overlap_and_reopened_or_regressed_writer_latch_refusal() {
    let original = collected(8, 4);
    for edit in 0..6 {
        let mut collector = ProcessTraceCollector::new(process(), 7, 48_000, 48_000, 4);
        let mut first = original.packet_history.clone();
        first.writer_finished = edit == 2;
        collector.packets(first.clone());
        let mut next = first;
        match edit {
            0 => next.events[0].sample.accepted_frames -= 1,
            1 => next.total_events -= 1,
            2 => next.writer_finished = false,
            3 => next.offered_frames -= 1,
            4 => next.published_frames -= 1,
            _ => next.events[0].sequence = 0,
        }
        collector.packets(next);
        assert_eq!(
            collector.artifact.collection_refusal,
            Some(ProcessTraceCollectionRefusal::ConflictingHistory),
            "edit={edit}"
        );
        collector.packets(original.packet_history.clone());
        assert!(collector.artifact.collection_refusal.is_some());
    }
    let mut collector = ProcessTraceCollector::new(process(), 7, 48_000, 48_000, 4);
    collector.reads(original.queue_history.clone());
    let mut changed = original.queue_history;
    changed.events[0].sample.requested_frames += 1;
    collector.reads(changed);
    assert_eq!(
        collector.artifact.collection_refusal,
        Some(ProcessTraceCollectionRefusal::ConflictingHistory)
    );
}

#[test]
fn document_preserves_legacy_limits_and_requires_explicit_archive_collection_marker() {
    let a = collected(8, 4);
    let mut value = serde_json::to_value(a).unwrap();
    value["schema_version"] = 1.into();
    assert!(serde_json::from_value::<ProcessTraceDocument>(value.clone()).is_err());
    value.as_object_mut().unwrap().remove("collection_refusal");
    assert!(
        serde_json::from_value::<ProcessTraceDocument>(value.clone())
            .unwrap()
            .trace(&process(), 7)
            .is_ok()
    );
    value["packet_history"]["events"] =
        serde_json::json!(vec![value["packet_history"]["events"][0].clone(); 257]);
    assert!(serde_json::from_value::<ProcessTraceDocument>(value).is_err());
    let mut value = serde_json::to_value(collected(8, 4)).unwrap();
    value.as_object_mut().unwrap().remove("collection_refusal");
    assert!(serde_json::from_value::<ProcessTraceDocument>(value).is_err());
}

#[test]
fn archived_replay_work_limit_is_independent_of_event_capacity() {
    use crate::latency::{
        packets::{PacketPublicationOrigin, PacketPublicationSample, packet_history},
        queue::{QueueReadSample, queue_history},
    };
    for count in [2048_usize, 2049] {
        let (mut packets, packet_observer) = packet_history();
        let (mut reads, read_observer) = queue_history();
        let mut collector = ProcessTraceCollector::new(process(), 7, 48_000, 48_000, 4096);
        for n in 0..count {
            let stamp = 1000 + n as u64 * 100;
            packets.record(PacketPublicationSample {
                offered_frames: 4096,
                accepted_frames: 0,
                qpc_before_100ns: Some(stamp),
                qpc_after_100ns: Some(stamp),
                origin: PacketPublicationOrigin::NativeCapture,
                buffer_generation: n as u64 + 1,
                flags: SILENT,
            });
            reads.record(QueueReadSample {
                requested_frames: 4096,
                qpc_before_100ns: Some(stamp + 10),
                qpc_after_100ns: Some(stamp + 10),
                ..Default::default()
            });
            if (n + 1).is_multiple_of(64) {
                collector.packets(packet_observer.snapshot().unwrap());
                collector.reads(read_observer.snapshot().unwrap());
            }
        }
        drop(packets);
        drop(reads);
        collector.packets(packet_observer.snapshot().unwrap());
        collector.reads(read_observer.snapshot().unwrap());
        assert_eq!(collector.artifact.collection_refusal, None);
        let result = collector.artifact.trace(&process(), 7);
        if count == 2048 {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err(), ProcessReplayError::UnsupportedShape);
        }
    }
}

#[test]
fn complete_six_second_prefix_exercises_targets_without_expanding_native_history_or_capacity() {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        capacity_frames: 8192,
        target_fill_frames: 2048,
        detect_position_gaps: false,
        ..Default::default()
    })
    .unwrap();
    let mut collector = ProcessTraceCollector::new(process(), 7, 48_000, 48_000, 48);
    collector.poll(&observer);
    for n in 0..6000_usize {
        let stamp = 1_000_000 + n as u64 * 10_000;
        ingress
            .push_packet_with_publication_clock(
                &[],
                CapturePacket {
                    frames: 48,
                    flags: SILENT,
                    device_position_frames: 0,
                    qpc_100ns: 0,
                },
                || Some(stamp),
            )
            .unwrap();
        source
            .read_interleaved_with_publication_clock(&mut [0.0; 96], || Some(stamp + 100))
            .unwrap();
        if (n + 1).is_multiple_of(64) {
            collector.poll(&observer);
        }
    }
    drop(ingress);
    drop(source);
    collector.poll(&observer);
    assert_eq!(observer.packet_history().unwrap().events.len(), 256);
    let trace = collector.artifact().trace(&process(), 7).unwrap();
    for target in [256, 512, 1024] {
        let mut first = None;
        for capacity in [2048, 4096] {
            let report = trace
                .replay(
                    &process(),
                    7,
                    ProcessBridgeOptions {
                        input: TransportInputPolicy::Target {
                            capacity_frames: capacity,
                            target_frames: target,
                            jitter_frames: 0,
                        },
                        delivery: Some(InputDeliveryTiming {
                            sample_rate: 48_000,
                            max_packet_frames: 48,
                            max_gap_frames: 48,
                            source: DeliveryTimingSource::Declared,
                        }),
                        engine_sample_rate: 48_000,
                        max_demand_frames: 48,
                        age_limit_100ns: None,
                    },
                )
                .unwrap();
            assert_eq!((report.packet_events, report.read_events), (6000, 6000));
            assert_eq!(report.candidate.captured_frames, 288_000);
            assert_eq!(report.candidate.dropped_frames, 0);
            assert_eq!(report.candidate.underrun_frames, 0);
            assert_eq!(report.unserved_frames, report.startup_unserved_frames);
            assert!(!report.qualified);
            let values = (
                report.candidate.output_frames,
                report.startup_unserved_frames,
                report.candidate.prime_discarded_frames,
                report.terminal_inventory_frames,
            );
            if let Some(first) = first {
                assert_eq!(values, first);
            }
            first = Some(values);
        }
    }
}
