use super::*;
use crate::recorder::{RecordingState, sink::RecordingSink};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "moiren-flac-recording-{}-{}.flac",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Decode a finished file with claxon and cross-check the stream facts and
/// every sample. FLAC is lossless, so decoded samples must equal the
/// quantized inputs exactly; this catches frame-boundary and tail bugs that
/// byte-level structure checks cannot.
fn round_trip(file: &Path, expected: &[f32], channels: usize, sample_rate: u32) {
    let mut reader = claxon::FlacReader::open(file).unwrap();
    let info = reader.streaminfo();
    assert_eq!(info.sample_rate, sample_rate);
    assert_eq!(info.channels as usize, channels);
    assert_eq!(info.bits_per_sample, BITS_PER_SAMPLE as u32);
    assert_eq!(info.samples, Some((expected.len() / channels) as u64));
    let decoded: Vec<i32> = reader.samples().map(|sample| sample.unwrap()).collect();
    let expected: Vec<i32> = expected.iter().map(|&sample| quantize(sample)).collect();
    assert_eq!(decoded, expected);
}

/// The STREAMINFO MD5 field sits at bytes 26..42 of every FLAC file and must
/// carry the digest of the unencoded samples once a recording finishes.
fn md5_field(bytes: &[u8]) -> [u8; 16] {
    bytes[26..42].try_into().expect("flac header present")
}

#[test]
fn flac_sink_rejects_unsupported_formats_before_touching_disk() {
    let file = TemporaryFile::new();
    let error = FlacRecording::create(&file.0, 9, 48_000, Default::default()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("eight channels"));
    // STREAMINFO packs the rate into 20 bits; wider rates must fail loudly
    // instead of truncating the field into a wrong-rate file.
    let error = FlacRecording::create(&file.0, 2, 700_000, Default::default()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("20 bits"));
    let error = FlacRecording::create(
        &file.0,
        2,
        48_000,
        FlacEncoding {
            level: FLAC_MAX_LEVEL + 1,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!file.0.exists());
}

#[test]
fn flac_sink_round_trips_stereo_and_mono_across_a_short_tail() {
    for channels in [1, 2] {
        let file = TemporaryFile::new();
        let mut flac =
            FlacRecording::create(&file.0, channels, 48_000, Default::default()).unwrap();
        // Default level 5 encodes 4096-frame blocks, so 10_000 frames means
        // two full blocks plus a 1808-frame tail split across several writes
        // that deliberately misalign with the block size.
        let block: Vec<f32> = (0..10_000 * channels)
            .map(|i| (i as f32 % 64.) / 64. - 0.5)
            .collect();
        let (head, tail) = block.split_at(block.len() / 3);
        flac.write_interleaved(head, channels).unwrap();
        assert_eq!(flac.written_frames(), (head.len() / channels) as u64);
        flac.write_interleaved(tail, channels).unwrap();
        assert_eq!(flac.written_frames(), 10_000);
        flac.finish().unwrap();
        round_trip(&file.0, &block, channels, 48_000);
        let bytes = fs::read(&file.0).unwrap();
        assert_ne!(
            md5_field(&bytes),
            [0u8; 16],
            "finish must patch the real MD5 into STREAMINFO"
        );
    }
}

#[test]
fn flac_sink_level_presets_reach_the_stream() {
    // Level 0 uses 1152-frame blocks; 4608 frames are exactly four blocks,
    // so the stream's block-size range pins the preset that was used.
    let file = TemporaryFile::new();
    let mut flac = FlacRecording::create(&file.0, 2, 48_000, FlacEncoding { level: 0 }).unwrap();
    let block: Vec<f32> = (0..4_608 * 2).map(|i| (i as f32 % 32.) / 32.).collect();
    flac.write_interleaved(&block, 2).unwrap();
    flac.finish().unwrap();
    let reader = claxon::FlacReader::open(&file.0).unwrap();
    let info = reader.streaminfo();
    assert_eq!(info.min_block_size, 1_152);
    assert_eq!(info.max_block_size, 1_152);
    round_trip(&file.0, &block, 2, 48_000);
}

#[test]
fn flac_sink_rejects_channel_mismatch_mid_session() {
    let file = TemporaryFile::new();
    let mut flac = FlacRecording::create(&file.0, 2, 48_000, Default::default()).unwrap();
    let error = flac.write_interleaved(&[0.0], 1).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
#[cfg_attr(miri, ignore = "requires filesystem access outside Miri isolation")]
fn record_flac_now_discards_preroll_and_finishes_a_playable_file() {
    let file = TemporaryFile::new();
    let (mut recorder, reader) =
        crate::recorder::recorder(2, crate::recorder::tests::settings(64)).unwrap();
    crate::recorder::tests::observe::<f32>(&mut recorder, &[&[9.0, 9.0, 9.0], &[9.0, 9.0, 9.0]]);
    let session = reader
        .record_flac_now(&file.0, 48_000, Default::default())
        .unwrap();
    assert_eq!(
        session.status().state,
        RecordingState::Recording,
        "preroll discard must not finalize the session"
    );
    let summary = session.finish().unwrap();
    // The queued preroll was skipped and nothing further was observed: the
    // file stays header-only but must still be a valid FLAC stream.
    assert_eq!(summary.written_frames, 0);
    let reader = claxon::FlacReader::open(&file.0).unwrap();
    // FLAC cannot express "known zero" in the total-samples field; zero means
    // unknown, so an empty recording decodes with no sample count either way.
    assert_eq!(reader.streaminfo().samples.unwrap_or(0), 0);
}
