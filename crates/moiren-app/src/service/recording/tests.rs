use super::*;

#[test]
fn float_wav_estimate_accounts_for_riff_headers_and_whole_frames() {
    let estimate = RecordingEstimate::float_wav(48_000, 2, 48_000, None);
    let frames = (u64::from(u32::MAX) - 50) / 8 - 48_000;
    assert_eq!(estimate.bytes_per_second, Some(384_000));
    assert_eq!(estimate.riff_remaining_frames, Some(frames));
    assert_eq!(
        estimate.riff_remaining_seconds,
        Some(frames as f64 / 48_000.0)
    );
    assert_eq!(estimate.free_disk_bytes, None);
    assert_eq!(estimate.disk_remaining_seconds, None);
    assert_eq!(estimate.remaining_seconds, estimate.riff_remaining_seconds);
    assert_eq!(estimate.basis, RecordingEstimateBasis::RiffOnly);
    assert_eq!(estimate.basis.label(), "RIFF only (disk space unknown)");
}

#[test]
fn disk_estimate_is_the_smaller_bound_and_saturates_at_capacity() {
    let reserve = MIN_DISK_RESERVE_BYTES + 384_000 * 2;
    let estimate = RecordingEstimate::float_wav(48_000, 2, 0, Some(reserve + 384_000));
    assert_eq!(estimate.disk_reserve_bytes, reserve);
    assert_eq!(estimate.disk_remaining_seconds, Some(1.0));
    assert_eq!(estimate.remaining_seconds, Some(1.0));
    let full = RecordingEstimate::float_wav(48_000, 2, u64::MAX, Some(u64::MAX));
    assert_eq!(full.riff_remaining_frames, Some(0));
    assert_eq!(full.remaining_seconds, Some(0.0));
    let low = RecordingEstimate::float_wav(48_000, 2, 0, Some(MIN_DISK_RESERVE_BYTES));
    assert_eq!(low.disk_remaining_seconds, Some(0.0));
}

#[test]
fn disk_estimate_reserves_margin_for_aggregate_same_volume_rate() {
    let aggregate = 384_000 * 2;
    let reserve = MIN_DISK_RESERVE_BYTES + aggregate * 2;
    let mut estimate = RecordingEstimate::float_wav(48_000, 2, 0, Some(reserve + aggregate * 10));
    estimate.apply_disk_budget(aggregate);
    assert_eq!(estimate.aggregate_bytes_per_second, Some(aggregate));
    assert_eq!(estimate.disk_reserve_bytes, reserve);
    assert_eq!(estimate.disk_remaining_seconds, Some(10.0));
    assert_eq!(estimate.basis, RecordingEstimateBasis::DiskAndRiff);
}

#[test]
fn invalid_or_unknown_format_never_invents_a_remaining_duration() {
    for (rate, channels) in [(0, 2), (48_000, 0), (u32::MAX, usize::MAX)] {
        let estimate = RecordingEstimate::float_wav(rate, channels, 0, None);
        assert_eq!(estimate.bytes_per_second, None);
        assert_eq!(estimate.remaining_seconds, None);
    }
    assert_eq!(RecordingFormat::WavFloat32.label(), "WAV (RIFF, float32)");
}

#[test]
fn destination_requires_a_wav_file_without_touching_the_filesystem() {
    for path in [
        "",
        "capture.flac",
        "capture.mp3",
        "directory/",
        "capture\0.wav",
    ] {
        assert!(validate_destination(Path::new(path)).is_err(), "{path:?}");
    }
    assert!(validate_destination(Path::new("capture.WAV")).is_ok());
}

#[test]
fn selected_wav_estimate_matches_writer_width_header_and_padding() {
    for (format, width, header) in [
        (RecordingFormat::WavFloat32, 4, 58),
        (RecordingFormat::WavPcm24, 3, 68),
        (RecordingFormat::WavPcm16, 2, 68),
    ] {
        assert_eq!(format.bytes_per_sample(), width);
        assert_eq!(format.header_bytes(), header);
        for channels in [1, 2, 3, 256] {
            let estimate = RecordingEstimate::wav(format, 48_000, channels, 0, None);
            let frames = estimate.riff_remaining_frames.unwrap();
            let frame_bytes = channels as u64 * u64::from(width);
            assert_eq!(estimate.bytes_per_second, Some(48_000 * frame_bytes));
            let bytes = frames * frame_bytes;
            assert!(u64::from(header) - 8 + bytes + (bytes & 1) <= u64::from(u32::MAX));
            let next = bytes + frame_bytes;
            assert!(u64::from(header) - 8 + next + (next & 1) > u64::from(u32::MAX));
            assert_eq!(
                estimate.riff_remaining_frames,
                format.wav_format().max_frames(channels)
            );
            let full = RecordingEstimate::wav(format, 48_000, channels, frames, None);
            assert_eq!(full.remaining_seconds, Some(0.0));
        }
    }
    assert_eq!(RecordingFormat::default(), RecordingFormat::WavFloat32);
    assert_eq!(
        RecordingEstimate::float_wav(48_000, 2, 10, None),
        RecordingEstimate::wav(RecordingFormat::WavFloat32, 48_000, 2, 10, None)
    );
}

#[test]
fn recording_format_intent_survives_path_and_replaces_saved_statistics() {
    let mut graph = super::super::graph::ServiceGraph::default();
    graph
        .edit(super::super::GraphEdit::AddNode {
            kind: NodeKind::Recorder,
            channels: 2,
        })
        .unwrap();
    let graph = graph.snapshot();
    let node = graph
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Recorder)
        .unwrap()
        .id();
    let mut service = RecordingService::default();
    service
        .set_format(&graph, node, RecordingFormat::WavPcm24)
        .unwrap();
    service
        .configure(&graph, node, PathBuf::from("selected.wav"))
        .unwrap();
    assert_eq!(service.entries[&node].format, RecordingFormat::WavPcm24);
    for phase in [
        RecordingPhase::Starting,
        RecordingPhase::Recording,
        RecordingPhase::Stopping,
    ] {
        service.entries.get_mut(&node).unwrap().phase = phase;
        assert!(
            service
                .set_format(&graph, node, RecordingFormat::WavPcm16)
                .is_err()
        );
        assert_eq!(service.entries[&node].format, RecordingFormat::WavPcm24);
    }
    let entry = service.entries.get_mut(&node).unwrap();
    entry.phase = RecordingPhase::Finished;
    entry.written_frames = 123;
    entry.sample_rate = Some(48_000);
    entry.channels = Some(2);
    let saved = entry.clone();
    service
        .set_format(&graph, node, RecordingFormat::WavPcm24)
        .unwrap();
    assert_eq!(
        service.entries[&node], saved,
        "unchanged selection preserves saved status"
    );
    service
        .set_format(&graph, node, RecordingFormat::WavPcm16)
        .unwrap();
    let next = &service.entries[&node];
    assert_eq!(next.phase, RecordingPhase::Idle);
    assert_eq!(next.path, saved.path);
    assert_eq!(next.written_frames, 0);
    assert_eq!(next.sample_rate, None);
    assert_eq!(next.channels, None);
    assert_eq!(next.stats, None);
}
