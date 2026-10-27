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

#[test]
fn pcm_formats_saturate_round_and_silence_non_finite_without_dither() {
    for (format, scale, width) in [
        (WavFormat::Pcm16, 32_768i32, 2),
        (WavFormat::Pcm24, 8_388_608, 3),
    ] {
        let samples = [
            -2.0,
            -1.0,
            -0.5,
            0.0,
            0.5,
            1.0,
            2.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let (mut recorder, mut reader) = recorder(1, settings(16)).unwrap();
        observe::<f32>(&mut recorder, &[&samples]);
        let mut wav =
            WavRecording::from_writer_with_format(Cursor::new(Vec::new()), 1, 48_000, format)
                .unwrap();
        assert_eq!(wav.drain(&mut reader).unwrap(), samples.len());
        assert_eq!(wav.written_frames(), samples.len() as u64);
        let bytes = wav.finish().unwrap().into_inner();
        assert_eq!(u32_at(&bytes, 16), 40);
        assert_eq!(u16_at(&bytes, 20), 0xfffe);
        assert_eq!(u16_at(&bytes, 36), 22);
        assert_eq!(u16_at(&bytes, 38), width as u16 * 8);
        assert_eq!(u32_at(&bytes, 40), 0);
        assert_eq!(
            &bytes[44..60],
            &[1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]
        );
        assert_eq!(u16_at(&bytes, 32), width as u16);
        assert_eq!(u16_at(&bytes, 34), width as u16 * 8);
        assert_eq!(u32_at(&bytes, 28), 48_000 * width as u32);
        assert_eq!(&bytes[60..64], b"data");
        assert_eq!(u32_at(&bytes, 64), samples.len() as u32 * width as u32);
        let expected: Vec<u8> = [
            -scale,
            -scale,
            -scale / 2,
            0,
            scale / 2,
            scale - 1,
            scale - 1,
            0,
            0,
            0,
        ]
        .into_iter()
        .flat_map(|sample| sample.to_le_bytes()[..width].to_vec())
        .collect();
        assert_eq!(&bytes[68..], expected);
        assert_eq!(u32_at(&bytes, 4) as usize, bytes.len() - 8);
    }
}

#[test]
fn pcm24_odd_data_is_padded_once_at_finish_not_between_drains() {
    for frames in [0, 1, 2, 3] {
        let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
        let mut wav = WavRecording::from_writer_with_format(
            Cursor::new(Vec::new()),
            1,
            48_000,
            WavFormat::Pcm24,
        )
        .unwrap();
        for _ in 0..frames {
            observe::<f32>(&mut recorder, &[&[0.5]]);
            assert_eq!(wav.drain(&mut reader).unwrap(), 1);
        }
        assert_eq!(wav.written_frames(), frames);
        let bytes = wav.finish().unwrap().into_inner();
        let data = frames as usize * 3;
        assert_eq!(bytes.len(), 68 + data + (data & 1));
        assert_eq!(u32_at(&bytes, 64) as usize, data);
        assert_eq!(u32_at(&bytes, 4) as usize, bytes.len() - 8);
        assert_eq!(&bytes[68..68 + data], [0, 0, 0x40].repeat(frames as usize));
        if data & 1 != 0 {
            assert_eq!(bytes.last(), Some(&0));
        }
    }
}

#[test]
fn each_format_reserves_riff_padding_before_consuming_frames() {
    for format in [WavFormat::Float32, WavFormat::Pcm24, WavFormat::Pcm16] {
        for channels in [1, 2, 3, 256] {
            let limit = format.max_frames(channels).unwrap();
            let data = limit * channels as u64 * u64::from(format.bytes_per_sample());
            let prefix = u64::from(format.header_bytes()) - 8;
            assert!(prefix + data + (data & 1) <= u64::from(u32::MAX));
            let next = data + channels as u64 * u64::from(format.bytes_per_sample());
            assert!(prefix + next + (next & 1) > u64::from(u32::MAX));
        }
        let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
        observe::<f32>(&mut recorder, &[&[0.5]]);
        let mut wav =
            WavRecording::from_writer_with_format(Cursor::new(Vec::new()), 1, 48_000, format)
                .unwrap();
        wav.data_bytes = format.max_frames(1).unwrap() * u64::from(format.bytes_per_sample());
        assert!(wav.drain(&mut reader).is_err());
        assert_eq!(reader.queued_frames(), 1);
        assert_eq!(reader.stats().read_frames, 0);
    }
}

#[test]
fn pcm_rounds_half_lsb_away_from_zero() {
    for (format, scale, width) in [
        (WavFormat::Pcm16, 32_768.0, 2),
        (WavFormat::Pcm24, 8_388_608.0, 3),
    ] {
        let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
        observe::<f32>(&mut recorder, &[&[0.5 / scale, -0.5 / scale, 0.0, 0.0]]);
        let mut wav =
            WavRecording::from_writer_with_format(Cursor::new(Vec::new()), 1, 48_000, format)
                .unwrap();
        wav.drain(&mut reader).unwrap();
        let bytes = wav.finish().unwrap().into_inner();
        let expected: Vec<u8> = [1i32, -1, 0, 0]
            .into_iter()
            .flat_map(|n| n.to_le_bytes()[..width].to_vec())
            .collect();
        assert_eq!(&bytes[68..], expected);
    }
}

#[test]
fn pcm_channels_stay_interleaved_across_drain_boundaries() {
    for (format, width) in [(WavFormat::Pcm16, 2), (WavFormat::Pcm24, 3)] {
        let (mut recorder, mut reader) = recorder(2, settings(4)).unwrap();
        observe::<f64>(&mut recorder, &[&[-1.0, 0.5], &[0.0, -0.5]]);
        let mut wav =
            WavRecording::from_writer_with_format(Cursor::new(Vec::new()), 2, 44_100, format)
                .unwrap();
        assert_eq!(wav.drain_frames(&mut reader, 1).unwrap(), 1);
        assert_eq!(wav.drain(&mut reader).unwrap(), 1);
        assert_eq!(wav.written_frames(), 2);
        let bytes = wav.finish().unwrap().into_inner();
        let scale = 1i32 << (width * 8 - 1);
        let expected: Vec<u8> = [-scale, 0, scale / 2, -scale / 2]
            .into_iter()
            .flat_map(|sample| sample.to_le_bytes()[..width].to_vec())
            .collect();
        assert_eq!(&bytes[68..], expected);
        assert_eq!(u16_at(&bytes, 22), 2);
        assert_eq!(u16_at(&bytes, 32), 2 * width as u16);
        assert_eq!(u32_at(&bytes, 28), 44_100 * 2 * width as u32);
    }
}

#[test]
fn pcm24_padding_failure_is_not_reported_as_a_finalized_file() {
    let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
    observe::<f32>(&mut recorder, &[&[0.0]]);
    let mut wav = WavRecording::from_writer_with_format(
        FailingWriter::default(),
        1,
        48_000,
        WavFormat::Pcm24,
    )
    .unwrap();
    wav.drain(&mut reader).unwrap();
    wav.writer.fail_write = true;
    assert!(wav.finish().is_err());
}

#[test]
fn float32_keeps_non_finite_values_and_headroom_for_compatibility() {
    let (mut recorder, mut reader) = recorder(1, settings(8)).unwrap();
    observe::<f64>(
        &mut recorder,
        &[&[2.0, -2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY]],
    );
    let mut wav = WavRecording::from_writer(Cursor::new(Vec::new()), 1, 48_000).unwrap();
    wav.drain(&mut reader).unwrap();
    let bytes = wav.finish().unwrap().into_inner();
    let samples: Vec<f32> = bytes[58..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    assert_eq!(&samples[..2], &[2.0, -2.0]);
    assert!(samples[2].is_nan());
    assert_eq!(samples[3], f32::INFINITY);
    assert_eq!(samples[4], f32::NEG_INFINITY);
}

#[test]
fn pcm_formats_validate_before_header_and_report_every_io_failure() {
    for format in [WavFormat::Pcm16, WavFormat::Pcm24] {
        for (channels, rate) in [(0, 48_000), (257, 48_000), (1, 0), (256, u32::MAX)] {
            let mut writer = Cursor::new(Vec::new());
            assert!(
                WavRecording::from_writer_with_format(&mut writer, channels, rate, format).is_err()
            );
            assert!(writer.get_ref().is_empty());
        }
        assert!(
            WavRecording::from_writer_with_format(Cursor::new(vec![1]), 1, 48_000, format).is_err()
        );
        assert!(
            WavRecording::from_writer_with_format(
                FailingWriter {
                    fail_write: true,
                    ..Default::default()
                },
                1,
                48_000,
                format
            )
            .is_err()
        );
        let (mut recorder, mut reader) = recorder(1, settings(4)).unwrap();
        observe::<f32>(&mut recorder, &[&[0.5]]);
        let mut wav =
            WavRecording::from_writer_with_format(FailingWriter::default(), 1, 48_000, format)
                .unwrap();
        wav.writer.fail_write = true;
        assert!(wav.drain(&mut reader).is_err());
        assert!(wav.is_failed());
        assert_eq!(wav.written_frames(), 0);
        wav.writer.fail_write = false;
        assert!(wav.finish().is_err());
        for seek_failure in [false, true] {
            let mut wav =
                WavRecording::from_writer_with_format(FailingWriter::default(), 1, 48_000, format)
                    .unwrap();
            wav.writer.fail_seek = seek_failure;
            wav.writer.fail_flush = !seek_failure;
            assert!(wav.finish().is_err());
        }
    }
}
