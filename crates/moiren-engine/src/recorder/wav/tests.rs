use super::*;
use crate::recorder::{
    recorder,
    tests::{observe, settings},
};
use std::io::{self, Cursor, Seek, SeekFrom, Write};

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[test]
fn wav_finalizes_ieee_float_header_fact_and_exact_pcm() {
    let (mut recorder, mut reader) = recorder(2, settings(4)).unwrap();
    observe::<f64>(&mut recorder, &[&[0.25, -0.5], &[1.0, -2.0]]);
    let mut wav = WavRecording::from_writer(Cursor::new(Vec::new()), 2, 48_000).unwrap();
    assert_eq!(wav.drain(&mut reader).unwrap(), 2);
    assert_eq!(wav.written_frames(), 2);
    let bytes = wav.finish().unwrap().into_inner();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(u32_at(&bytes, 4) as usize, bytes.len() - 8);
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(u32_at(&bytes, 16), 18);
    assert_eq!(u16_at(&bytes, 20), 3);
    assert_eq!(u16_at(&bytes, 22), 2);
    assert_eq!(u32_at(&bytes, 24), 48_000);
    assert_eq!(u32_at(&bytes, 28), 48_000 * 8);
    assert_eq!(u16_at(&bytes, 32), 8);
    assert_eq!(u16_at(&bytes, 34), 32);
    assert_eq!(u16_at(&bytes, 36), 0);
    assert_eq!(&bytes[38..42], b"fact");
    assert_eq!(u32_at(&bytes, 42), 4);
    assert_eq!(u32_at(&bytes, 46), 2);
    assert_eq!(&bytes[50..54], b"data");
    assert_eq!(u32_at(&bytes, 54), 16);
    let expected: Vec<u8> = [0.25f32, 1.0, -0.5, -2.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    assert_eq!(&bytes[58..], expected);
}

#[test]
fn wav_drain_honors_frame_budget_and_checks_channel_count_before_reading() {
    let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
    observe::<f32>(&mut recorder, &[&[1.0, 2.0, 3.0, 4.0]]);
    let mut mismatch = WavRecording::from_writer(Cursor::new(Vec::new()), 2, 48_000).unwrap();
    assert_eq!(
        mismatch.drain(&mut reader).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(reader.queued_frames(), 4);
    let mut wav = WavRecording::from_writer(Cursor::new(Vec::new()), 1, 48_000).unwrap();
    assert_eq!(wav.drain_frames(&mut reader, 0).unwrap(), 0);
    assert_eq!(wav.drain_frames(&mut reader, 2).unwrap(), 2);
    assert_eq!(reader.queued_frames(), 2);
    assert_eq!(wav.drain(&mut reader).unwrap(), 2);
    assert_eq!(wav.drain(&mut reader).unwrap(), 0);
    assert_eq!(wav.written_frames(), 4);
    wav.finish().unwrap();
}

#[test]
fn wav_rejects_invalid_format_before_writing_header() {
    for (channels, sample_rate) in [(0, 48_000), (257, 48_000), (1, 0), (256, u32::MAX)] {
        assert!(matches!(
            WavRecording::from_writer(Cursor::new(Vec::new()), channels, sample_rate),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput
        ));
    }
}

#[derive(Default)]
struct FailingWriter {
    bytes: Cursor<Vec<u8>>,
    fail_write: bool,
    fail_seek: bool,
    fail_flush: bool,
}

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_write {
            return Err(io::Error::other("injected write failure"));
        }
        self.bytes.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::other("injected flush failure"));
        }
        Ok(())
    }
}

impl Seek for FailingWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if self.fail_seek {
            return Err(io::Error::other("injected seek failure"));
        }
        self.bytes.seek(position)
    }
}

#[test]
fn wav_reports_header_creation_data_seek_and_flush_failures() {
    assert!(
        WavRecording::from_writer(
            FailingWriter {
                fail_write: true,
                ..Default::default()
            },
            1,
            48_000
        )
        .is_err()
    );
    let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
    observe::<f32>(&mut recorder, &[&[1.0, 2.0]]);
    let mut wav = WavRecording::from_writer(FailingWriter::default(), 1, 48_000).unwrap();
    wav.writer.fail_write = true;
    assert!(wav.drain(&mut reader).is_err());
    assert_eq!(wav.written_frames(), 0);
    assert!(wav.is_failed());
    wav.writer.fail_write = false;
    observe::<f32>(&mut recorder, &[&[3.0]]);
    assert!(wav.drain(&mut reader).is_err());
    assert_eq!(reader.queued_frames(), 1);
    assert!(wav.finish().is_err());
    for seek_failure in [false, true] {
        let mut wav = WavRecording::from_writer(FailingWriter::default(), 1, 48_000).unwrap();
        wav.writer.fail_seek = seek_failure;
        wav.writer.fail_flush = !seek_failure;
        assert!(wav.finish().is_err());
    }
}

#[test]
fn wav_preflights_riff_size_before_consuming_any_frames() {
    let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
    observe::<f32>(&mut recorder, &[&[1.0, 2.0]]);
    let mut wav = WavRecording::from_writer(Cursor::new(Vec::new()), 1, 48_000).unwrap();
    // Pretend a small in-memory writer already contains a nearly full RIFF.
    wav.data_bytes = (u64::from(u32::MAX) - 50) / 4 * 4;
    assert_eq!(
        wav.drain(&mut reader).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(reader.queued_frames(), 2);
    assert_eq!(reader.stats().read_frames, 0);
}
