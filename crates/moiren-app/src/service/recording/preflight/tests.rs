use super::*;

fn fixture() -> (Preflights, RecordingService, NodeId, Request) {
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let node = graph.create_node(NodeKind::Recorder, 2).unwrap();
    let mut service = RecordingService::default();
    let mut entry = RecordingSnapshot::idle(node);
    entry.path = Some(PathBuf::from("preflight.wav"));
    service.entries.insert(node, entry);
    let mut preflight = Preflights::default();
    preflight.request(&mut service, node, 48_000, 2).unwrap();
    let request = preflight.requests[&node].clone();
    (preflight, service, node, request)
}

#[test]
fn stale_preflight_cannot_overwrite_newer_destination_or_format() {
    let (mut preflight, mut service, node, stale) = fixture();
    service.entries.get_mut(&node).unwrap().path = Some(PathBuf::from("new.wav"));
    preflight.request(&mut service, node, 44_100, 1).unwrap();
    preflight.complete(
        &mut service,
        &stale,
        Some(disk::Observation {
            available: 1,
            volume: None,
        }),
    );
    let estimate = &service.entries[&node].estimate;
    assert_eq!(estimate.proposed_sample_rate, Some(44_100));
    assert_eq!(estimate.proposed_channels, Some(1));
    assert_eq!(estimate.free_disk_bytes, None);
    assert!(estimate.preflight_pending);
}

#[test]
fn reset_preflight_cannot_adopt_same_node_path_and_format_in_new_document() {
    let (mut preflight, mut service, node, stale) = fixture();
    preflight.cancel_all(&mut service);
    preflight.request(&mut service, node, 48_000, 2).unwrap();
    let current = preflight.requests[&node].clone();
    assert_ne!(stale.ticket, current.ticket);
    preflight.complete(
        &mut service,
        &stale,
        Some(disk::Observation {
            available: 1,
            volume: None,
        }),
    );
    assert_eq!(service.entries[&node].estimate.free_disk_bytes, None);
    preflight.complete(&mut service, &current, None);
    assert!(!service.entries[&node].estimate.preflight_pending);
    assert_eq!(service.entries[&node].sample_rate, None);
    assert_eq!(
        service.entries[&node].estimate.basis,
        RecordingEstimateBasis::RiffOnly
    );
}

#[test]
fn starting_recording_invalidates_inflight_preflight_without_waiting() {
    let (mut preflight, mut service, node, stale) = fixture();
    preflight.cancel(&mut service, node);
    let entry = service.entries.get_mut(&node).unwrap();
    entry.phase = RecordingPhase::Starting;
    entry.sample_rate = Some(48_000);
    entry.channels = Some(2);
    entry.estimate = RecordingEstimate::float_wav(48_000, 2, 0, None);
    preflight.complete(
        &mut service,
        &stale,
        Some(disk::Observation {
            available: 1,
            volume: None,
        }),
    );
    assert!(!preflight.has_pending());
    assert_eq!(service.entries[&node].estimate.proposed_sample_rate, None);
    assert_eq!(service.entries[&node].estimate.free_disk_bytes, None);
}

#[test]
fn blocked_disk_probe_does_not_block_poll_or_exit_cancellation() {
    let (mut preflight, mut service, node, request) = fixture();
    let (release, wait) = std::sync::mpsc::channel();
    preflight.worker = Some(Worker {
        request,
        thread: thread::spawn(move || {
            let _ = wait.recv();
            None
        }),
    });
    preflight.poll(&mut service);
    assert!(service.entries[&node].estimate.preflight_pending);
    preflight.cancel_all(&mut service);
    assert!(!preflight.has_pending());
    assert!(!service.entries[&node].estimate.preflight_pending);
    assert!(!preflight.worker.as_ref().unwrap().thread.is_finished());
    release.send(()).unwrap();
    while !preflight.worker.as_ref().unwrap().thread.is_finished() {
        thread::yield_now();
    }
    preflight.poll(&mut service);
    assert!(preflight.worker.is_none());
    assert_eq!(service.entries[&node].estimate.free_disk_bytes, None);
}

#[test]
fn rejected_start_cancels_blocked_preflight_without_replacing_failure() {
    let (mut preflight, mut service, node, request) = fixture();
    let (release, wait) = std::sync::mpsc::channel();
    preflight.worker = Some(Worker {
        request,
        thread: thread::spawn(move || {
            let _ = wait.recv();
            Some(disk::Observation {
                available: 1,
                volume: None,
            })
        }),
    });
    preflight.cancel(&mut service, node);
    service.fail(node, "audio is stopped");
    let failure = service.entries[&node].clone();
    assert!(!preflight.has_pending());
    assert!(!failure.estimate.preflight_pending);
    assert!(!preflight.worker.as_ref().unwrap().thread.is_finished());
    release.send(()).unwrap();
    while !preflight.worker.as_ref().unwrap().thread.is_finished() {
        thread::yield_now();
    }
    preflight.poll(&mut service);
    assert!(preflight.worker.is_none());
    assert_eq!(service.entries[&node], failure);
}

#[test]
fn stale_encoding_preflight_cannot_publish_into_new_format() {
    let (mut preflight, mut service, node, stale) = fixture();
    service.entries.get_mut(&node).unwrap().format = RecordingFormat::WavPcm24;
    preflight.request(&mut service, node, 48_000, 2).unwrap();
    let current = preflight.requests[&node].clone();
    assert_ne!(current.ticket, stale.ticket);
    preflight.complete(
        &mut service,
        &stale,
        Some(disk::Observation {
            available: 1,
            volume: None,
        }),
    );
    assert!(service.entries[&node].estimate.preflight_pending);
    assert_eq!(
        service.entries[&node].estimate.bytes_per_second,
        Some(288_000)
    );
    assert_eq!(service.entries[&node].estimate.free_disk_bytes, None);
    preflight.complete(&mut service, &current, None);
    assert_eq!(service.entries[&node].format, RecordingFormat::WavPcm24);
    assert_eq!(
        service.entries[&node].estimate.bytes_per_second,
        Some(288_000)
    );
    assert!(!service.entries[&node].estimate.preflight_pending);
    assert_eq!(service.entries[&node].sample_rate, None);
}
