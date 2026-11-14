//! MP3 sink over the bundled LAME encoder: CBR 256 kbps, mono or stereo.
//! The LAME/VBR tag is disabled and the ID3v2 tag is emitted in-band by the
//! encoder, so encoded bytes stream straight to the file and finalization
//! never needs to seek back. Requires the `mp3` feature.
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
};

use mp3lame_encoder::{Bitrate, Builder, FlushGap, InterleavedPcm, MonoPcm, Quality};
use serde::{Deserialize, Serialize};

use super::{session::DRAIN_CHUNK_FRAMES, sink::RecordingSink};

/// LAME's interleaved entry points assume stereo; mono sessions must go
/// through the single-channel buffer API.
enum Pcm<'a> {
    Mono(MonoPcm<'a, f32>),
    Stereo(InterleavedPcm<'a, f32>),
}

impl mp3lame_encoder::EncoderInput for Pcm<'_> {
    fn encode(
        self,
        encoder: &mut mp3lame_encoder::Encoder,
        output_buf: *mut u8,
        output_len: usize,
    ) -> std::ffi::c_int {
        match self {
            Pcm::Mono(pcm) => pcm.encode(encoder, output_buf, output_len),
            Pcm::Stereo(pcm) => pcm.encode(encoder, output_buf, output_len),
        }
    }
}

/// The discrete input rates LAME encodes natively. Outside this set it would
/// resample to a nearby output rate, silently changing the file's duration
/// relative to the session's frame accounting, so reject them up front.
const LAME_SAMPLE_RATES: [u32; 9] = [
    8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000,
];

/// The flush tail can emit up to 7200 bytes; the encode buffer sized for a
/// full drain chunk covers every ordinary write.
const FLUSH_TAIL_BYTES: usize = 7_200;

/// The discrete CBR bitrates LAME supports, ascending. Shared with the UI so
/// preference steppers walk exactly this ladder.
pub const MP3_BITRATE_KBPS: [u32; 16] = [
    8, 16, 24, 32, 40, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];

/// User-selectable encoding knobs. Invalid combinations are rejected at
/// `Mp3Recording::create`, before the queue lease is claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mp3Encoding {
    /// CBR target; must sit on the ladder above.
    pub bitrate_kbps: u32,
    pub mode: Mp3Mode,
}

/// Rate-control strategy. CBR fixes the bitrate; VBR fixes a quality level
/// and lets the bitrate float with the material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mp3Mode {
    Cbr,
    /// `quality` runs 0 (best) through 9 (worst), LAME's own scale.
    Vbr {
        quality: u8,
    },
}

impl Default for Mp3Encoding {
    fn default() -> Self {
        Self {
            bitrate_kbps: 256,
            mode: Mp3Mode::Cbr,
        }
    }
}

impl Mp3Encoding {
    pub fn valid(self) -> bool {
        MP3_BITRATE_KBPS.contains(&self.bitrate_kbps)
            && match self.mode {
                Mp3Mode::Cbr => true,
                Mp3Mode::Vbr { quality } => quality <= 9,
            }
    }
}

fn bitrate_from_kbps(kbps: u32) -> Option<Bitrate> {
    Some(match kbps {
        8 => Bitrate::Kbps8,
        16 => Bitrate::Kbps16,
        24 => Bitrate::Kbps24,
        32 => Bitrate::Kbps32,
        40 => Bitrate::Kbps40,
        48 => Bitrate::Kbps48,
        64 => Bitrate::Kbps64,
        80 => Bitrate::Kbps80,
        96 => Bitrate::Kbps96,
        112 => Bitrate::Kbps112,
        128 => Bitrate::Kbps128,
        160 => Bitrate::Kbps160,
        192 => Bitrate::Kbps192,
        224 => Bitrate::Kbps224,
        256 => Bitrate::Kbps256,
        320 => Bitrate::Kbps320,
        _ => return None,
    })
}

fn quality_from_level(level: u8) -> Option<Quality> {
    Some(match level {
        0 => Quality::Best,
        1 => Quality::SecondBest,
        2 => Quality::NearBest,
        3 => Quality::VeryNice,
        4 => Quality::Nice,
        5 => Quality::Good,
        6 => Quality::Decent,
        7 => Quality::Ok,
        8 => Quality::SecondWorst,
        9 => Quality::Worst,
        _ => return None,
    })
}

pub struct Mp3Recording {
    encoder: mp3lame_encoder::Encoder,
    file: BufWriter<File>,
    channels: usize,
    written_frames: u64,
    encoded: Vec<u8>,
}

impl std::fmt::Debug for Mp3Recording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The encoder handle is an opaque C pointer; report session facts.
        formatter
            .debug_struct("Mp3Recording")
            .field("channels", &self.channels)
            .field("written_frames", &self.written_frames)
            .finish_non_exhaustive()
    }
}

impl Mp3Recording {
    /// The encoder is created eagerly so unsupported formats and encoding
    /// settings fail before the queue lease is claimed.
    pub fn create(
        path: impl AsRef<Path>,
        channels: usize,
        sample_rate: u32,
        encoding: Mp3Encoding,
    ) -> io::Result<Self> {
        if !matches!(channels, 1 | 2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MP3 supports mono or stereo only",
            ));
        }
        if !LAME_SAMPLE_RATES.contains(&sample_rate) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MP3 requires one of LAME's native sample rates",
            ));
        }
        if !encoding.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MP3 encoding settings are off the LAME ladder",
            ));
        }
        let builder = Builder::new()
            .ok_or_else(|| io::Error::other("LAME encoder allocation failed"))?
            .with_num_channels(channels as u8)
            .and_then(|builder| builder.with_sample_rate(sample_rate))
            .and_then(|builder| builder.with_to_write_vbr_tag(false));
        // CBR keeps the algorithm quality internal (NearBest): LAME's -q
        // steps below 3 are inaudible and only slow encoding down. VBR
        // carries the user's quality level directly.
        let builder = match encoding.mode {
            Mp3Mode::Cbr => builder
                .and_then(|builder| {
                    builder.with_brate(
                        bitrate_from_kbps(encoding.bitrate_kbps)
                            .expect("validated against the ladder"),
                    )
                })
                .and_then(|builder| builder.with_quality(Quality::NearBest)),
            Mp3Mode::Vbr { quality } => builder
                .and_then(|builder| builder.with_vbr_mode(mp3lame_encoder::VbrMode::Mtrh))
                .and_then(|builder| {
                    builder.with_vbr_quality(quality_from_level(quality).expect("validated 0..=9"))
                }),
        };
        let encoder = builder
            .and_then(mp3lame_encoder::Builder::build)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("LAME rejected the format: {error:?}"),
                )
            })?;
        let capacity =
            mp3lame_encoder::max_required_buffer_size(DRAIN_CHUNK_FRAMES).max(FLUSH_TAIL_BYTES);
        let encoded = Vec::with_capacity(capacity);
        Ok(Self {
            encoder,
            file: BufWriter::new(File::create(path)?),
            channels,
            written_frames: 0,
            encoded,
        })
    }
}

impl RecordingSink for Mp3Recording {
    fn write_interleaved(&mut self, samples: &[f32], channels: usize) -> io::Result<()> {
        if channels != self.channels {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MP3 and recorder channel counts differ",
            ));
        }
        if samples.is_empty() {
            return Ok(());
        }
        let frames = samples.len() / channels;
        self.encoded.clear();
        let capacity = mp3lame_encoder::max_required_buffer_size(frames);
        if self.encoded.capacity() < capacity {
            self.encoded.reserve(capacity - self.encoded.capacity());
        }
        let input = if channels == 1 {
            Pcm::Mono(MonoPcm(samples))
        } else {
            Pcm::Stereo(InterleavedPcm(samples))
        };
        let written = self
            .encoder
            .encode_to_vec(input, &mut self.encoded)
            .map_err(|error| io::Error::other(format!("LAME encoding failed: {error:?}")))?;
        self.file.write_all(&self.encoded[..written])?;
        self.written_frames += frames as u64;
        Ok(())
    }

    fn written_frames(&self) -> u64 {
        self.written_frames
    }

    fn finish(&mut self) -> io::Result<()> {
        self.encoded.clear();
        if self.encoded.capacity() < FLUSH_TAIL_BYTES {
            self.encoded
                .reserve(FLUSH_TAIL_BYTES - self.encoded.capacity());
        }
        let written = self
            .encoder
            .flush_to_vec::<FlushGap>(&mut self.encoded)
            .map_err(|error| io::Error::other(format!("LAME flush failed: {error:?}")))?;
        self.file.write_all(&self.encoded[..written])?;
        self.file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
