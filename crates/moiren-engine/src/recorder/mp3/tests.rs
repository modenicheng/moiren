use super::*;
use crate::recorder::{RecordingState, sink::RecordingSink};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "moiren-mp3-recording-{}-{}.mp3",
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

/// LAME emits an in-band ID3v2 header when tagging is enabled, otherwise the
/// stream starts with an MPEG frame sync.
fn assert_mp3(bytes: &[u8]) {
    let starts_with_id3 = bytes.len() >= 3 && &bytes[..3] == b"ID3";
    let starts_with_sync = bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0;
    assert!(
        starts_with_id3 || starts_with_sync,
        "not an MP3 stream: {:02x?}",
        &bytes[..bytes.len().min(4)]
    );
}

#[test]
fn mp3_sink_rejects_unsupported_formats_before_touching_disk() {
    let file = TemporaryFile::new();
    let error = Mp3Recording::create(&file.0, 3, 48_000).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("mono or stereo"));
    // LAME would resample silently to a nearby output rate, so non-native
    // rates must fail loudly instead of producing a wrong-duration file.
    let error = Mp3Recording::create(&file.0, 2, 96_000).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("native sample rates"));
    assert!(!file.0.exists());
}

#[test]
fn mp3_sink_encodes_stereo_and_mono_and_counts_frames() {
    for channels in [1, 2] {
        let file = TemporaryFile::new();
        let mut mp3 = Mp3Recording::create(&file.0, channels, 48_000).unwrap();
        // Roughly a quarter second per feed, split across two writes to
        // exercise repeated interleaved appends.
        let block: Vec<f32> = (0..12_000 * channels)
            .map(|i| i as f32 % 64. / 64.)
            .collect();
        let (head, tail) = block.split_at(block.len() / 2);
        mp3.write_interleaved(head, channels).unwrap();
        assert_eq!(mp3.written_frames(), (head.len() / channels) as u64);
        mp3.write_interleaved(tail, channels).unwrap();
        assert_eq!(mp3.written_frames(), 12_000);
        mp3.finish().unwrap();
        let bytes = fs::read(&file.0).unwrap();
        assert_mp3(&bytes);
        // CBR 256 kbps yields 32 kB/s: a quarter second must land well above
        // the ID3 header alone, and both channel layouts encode real payload.
        assert!(
            bytes.len() > 2_000,
            "suspiciously small MP3 for {} channels: {} bytes",
            channels,
            bytes.len()
        );
    }
}

#[test]
fn mp3_sink_rejects_channel_mismatch_mid_session() {
    let file = TemporaryFile::new();
    let mut mp3 = Mp3Recording::create(&file.0, 2, 48_000).unwrap();
    let error = mp3.write_interleaved(&[0.0], 1).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
#[cfg_attr(miri, ignore = "requires filesystem access outside Miri isolation")]
fn record_mp3_now_discards_preroll_and_finishes_a_playable_file() {
    let file = TemporaryFile::new();
    let (mut recorder, reader) =
        crate::recorder::recorder(2, crate::recorder::tests::settings(64)).unwrap();
    crate::recorder::tests::observe::<f32>(&mut recorder, &[&[9.0, 9.0, 9.0], &[9.0, 9.0, 9.0]]);
    let session = reader.record_mp3_now(&file.0, 48_000).unwrap();
    assert_eq!(
        session.status().state,
        RecordingState::Recording,
        "preroll discard must not finalize the session"
    );
    let summary = session.finish().unwrap();
    // The queued preroll was skipped, so nothing further was observed: the
    // file carries only encoder priming, but must still finalize cleanly.
    assert_eq!(summary.written_frames, 0);
    let bytes = fs::read(&file.0).unwrap();
    assert!(!bytes.is_empty());
}
