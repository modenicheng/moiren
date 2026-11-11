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
    /// CBR 256 kbps at near-best quality. The encoder is created eagerly so
    /// unsupported formats fail before the queue lease is claimed.
    pub fn create(path: impl AsRef<Path>, channels: usize, sample_rate: u32) -> io::Result<Self> {
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
        let encoder = Builder::new()
            .ok_or_else(|| io::Error::other("LAME encoder allocation failed"))?
            .with_num_channels(channels as u8)
            .and_then(|builder| builder.with_sample_rate(sample_rate))
            .and_then(|builder| builder.with_brate(Bitrate::Kbps256))
            .and_then(|builder| builder.with_quality(Quality::NearBest))
            .and_then(|builder| builder.with_to_write_vbr_tag(false))
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
