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
