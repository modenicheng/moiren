//! Lossless FLAC sink over the pure-Rust flacenc encoder. Frames encode
//! incrementally as blocks fill; the STREAMINFO block is written as a
//! placeholder up front and patched with the real totals and MD5 when the
//! session finishes, the same seek-back trick the WAV sink uses for its
//! lengths. Requires the `flac` feature.
use std::{
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::Path,
};

use flacenc::{
    bitsink::ByteSink,
    component::{BitRepr, StreamInfo},
    config::Encoder,
    encode_fixed_size_frame,
    error::Verify,
    source::{Context, Fill, FrameBuf},
};
use serde::{Deserialize, Serialize};

use super::sink::RecordingSink;

/// FLAC packs the sample rate into 20 STREAMINFO bits.
const MAX_SAMPLE_RATE: u32 = 655_350;
/// Recording captures at 24-bit depth: the float bus carries well under 24
/// significant bits, and 24-bit FLAC is universally decodable.
const BITS_PER_SAMPLE: usize = 24;
/// `fLaC` stream marker plus the last-metadata-block header for the 34-byte
/// STREAMINFO block that follows it.
const STREAMINFO_PREFIX: [u8; 8] = [0x66, 0x4c, 0x61, 0x43, 0x80, 0x00, 0x00, 0x22];
/// STREAMINFO is always 34 bytes on the wire; the placeholder reserves them
/// so the finish-time patch never moves frame data.
const STREAMINFO_BYTES: usize = 34;
/// Byte offset of the STREAMINFO block inside the file.
const STREAMINFO_OFFSET: u64 = 8;
/// The FLAC format's smallest legal block. A final capture shorter than
/// this pads with silence rather than emitting a frame decoders reject.
const SPEC_MIN_BLOCK_FRAMES: usize = 16;

/// The top compression level. Levels 0..=8 follow libFLAC's convention of
/// "bigger number, slower and smaller", mapped onto our own block-size and
/// LPC-order presets below.
pub const FLAC_MAX_LEVEL: u8 = 8;

/// Our level presets: (block size, quantized LPC order). libFLAC-inspired,
/// tuned to flacenc's qlpc encoder rather than copied, since the two encoders
/// expose different knobs.
const LEVEL_PRESETS: [(usize, usize); 9] = [
    (1_152, 2),  // 0: fastest
    (1_152, 4),  // 1
    (2_048, 4),  // 2
    (3_072, 6),  // 3
    (4_096, 8),  // 4
    (4_096, 12), // 5: default
    (4_096, 16), // 6
    (8_192, 24), // 7
    (8_192, 32), // 8: best compression
];

/// User-selectable encoding knobs. The single level stands in for the whole
/// preset; invalid levels are rejected at `FlacRecording::create`, before the
/// queue lease is claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FlacEncoding {
    /// Compression preset, 0 (fastest) through [`FLAC_MAX_LEVEL`] (best).
    pub level: u8,
}

impl Default for FlacEncoding {
    fn default() -> Self {
        Self { level: 5 }
    }
}

impl FlacEncoding {
    pub fn valid(self) -> bool {
        self.level <= FLAC_MAX_LEVEL
    }
}

/// Convert a bus sample to signed 24-bit. Full-scale negative stays exactly
/// representable; positive full-scale clamps to `2^23 - 1`.
fn quantize(sample: f32) -> i32 {
    (sample * 8_388_608.0)
        .round()
        .clamp(-8_388_608.0, 8_388_607.0) as i32
}

pub struct FlacRecording<W: Write + Seek = BufWriter<File>> {
    writer: W,
    encoder: flacenc::error::Verified<Encoder>,
    stream_info: StreamInfo,
    framebuf: FrameBuf,
    /// Side-car md5 and sample accounting kept in lockstep with the frame
    /// buffer by the tuple `Fill` implementation.
    context: Context,
    sink: ByteSink,
    /// Interleaved 24-bit samples waiting for a full block; audio callbacks
    /// and drain chunks rarely align with the encoder's block size.
    pending: Vec<i32>,
    channels: usize,
    block_size: usize,
    frame_number: usize,
    written_frames: u64,
}

impl std::fmt::Debug for FlacRecording {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FlacRecording")
            .field("channels", &self.channels)
            .field("block_size", &self.block_size)
            .field("written_frames", &self.written_frames)
            .finish_non_exhaustive()
    }
}

/// Reject unsupported formats before any destination is touched. Returns the
/// level's (block size, LPC order) preset.
fn validate_format(
    channels: usize,
    sample_rate: u32,
    encoding: FlacEncoding,
) -> io::Result<(usize, usize)> {
    if !(1..=8).contains(&channels) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FLAC supports one through eight channels",
        ));
    }
    if sample_rate == 0 || sample_rate > MAX_SAMPLE_RATE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FLAC requires a sample rate that fits STREAMINFO's 20 bits",
        ));
    }
    if !encoding.valid() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FLAC compression level must be 0 through 8",
        ));
    }
    Ok(LEVEL_PRESETS[encoding.level as usize])
}

impl FlacRecording<BufWriter<File>> {
    /// The encoder and placeholder header are created eagerly so unsupported
    /// formats and settings fail before the queue lease is claimed.
    pub fn create(
        path: impl AsRef<Path>,
        channels: usize,
        sample_rate: u32,
        encoding: FlacEncoding,
    ) -> io::Result<Self> {
        validate_format(channels, sample_rate, encoding)?;
        let writer = BufWriter::new(File::create(path)?);
        Self::from_writer(writer, channels, sample_rate, encoding)
    }
}

impl<W: Write + Seek> FlacRecording<W> {
    /// The writer must be empty. This also permits in-memory destinations for
    /// offline recording and deterministic IO failure tests.
    pub fn from_writer(
        mut writer: W,
        channels: usize,
        sample_rate: u32,
        encoding: FlacEncoding,
    ) -> io::Result<Self> {
        let (block_size, lpc_order) = validate_format(channels, sample_rate, encoding)?;
        if writer.seek(SeekFrom::End(0))? != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "FLAC destination must be empty",
            ));
        }
        let mut config = Encoder::default();
        config.block_size = block_size;
        // The recording worker thread is the only encoder user; flacenc's
        // thread pool would only add contention with the audio graph.
        config.multithread = false;
        config.subframe_coding.qlpc.lpc_order = lpc_order;
        let encoder = config.into_verified().map_err(|(_, error)| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("flacenc rejected the encoder config: {error:?}"),
            )
        })?;
        let framebuf = FrameBuf::with_size(channels, block_size).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("flacenc rejected the frame layout: {error:?}"),
            )
        })?;
        let mut stream_info = StreamInfo::new(sample_rate as usize, channels, BITS_PER_SAMPLE)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("flacenc rejected the stream format: {error:?}"),
                )
            })?;
        // The placeholder must already satisfy the spec's non-optional fields;
        // frame sizes stay 0 (unknown) until frames exist.
        stream_info
            .set_block_sizes(block_size, block_size)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, format!("{error:?}")))?;
        stream_info
            .set_frame_sizes(0, 0)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, format!("{error:?}")))?;
        let mut sink = ByteSink::new();
        stream_info
            .write(&mut sink)
            .map_err(|error| io::Error::other(format!("FLAC header write failed: {error:?}")))?;
        debug_assert_eq!(sink.len(), STREAMINFO_BYTES * 8);
        writer.write_all(&STREAMINFO_PREFIX)?;
        writer.write_all(sink.as_slice())?;
        Ok(Self {
            writer,
            encoder,
            stream_info,
            framebuf,
            context: Context::new(BITS_PER_SAMPLE, channels),
            sink,
            pending: Vec::with_capacity(block_size * channels),
            channels,
            block_size,
            frame_number: 0,
            written_frames: 0,
        })
    }

    /// Encode every complete pending block. `flush_tail` also encodes the
    /// short final block; FLAC allows only the last frame to be smaller than
    /// the stream's other blocks. Blocks are copied out before encoding:
    /// `fill_and_encode` needs all of `self` mutably, and this runs on the
    /// recording worker where a block-sized copy is noise next to encoding.
    fn drain_blocks(&mut self, flush_tail: bool) -> io::Result<()> {
        let block_samples = self.block_size * self.channels;
        while self.pending.len() >= block_samples {
            let block: Vec<i32> = self.pending.drain(..block_samples).collect();
            self.fill_and_encode(&block)?;
        }
        if flush_tail && !self.pending.is_empty() {
            // FLAC's smallest legal block is 16 frames, and flacenc encodes
            // whatever is filled without checking that floor, so a shorter
            // final capture would emit a frame decoders reject. Pad with
            // silence: STREAMINFO, MD5, and the frame data all describe the
            // padded stream, and no captured audio is lost.
            let frames = self.pending.len() / self.channels;
            if frames < SPEC_MIN_BLOCK_FRAMES {
                self.pending
                    .resize(SPEC_MIN_BLOCK_FRAMES * self.channels, 0);
            }
            let tail = std::mem::take(&mut self.pending);
            self.fill_and_encode(&tail)?;
        }
        Ok(())
    }

    fn fill_and_encode(&mut self, block: &[i32]) -> io::Result<()> {
        (&mut self.framebuf, &mut self.context)
            .fill_interleaved(block)
            .map_err(|error| io::Error::other(format!("FLAC frame fill failed: {error:?}")))?;
        let frame = encode_fixed_size_frame(
            &self.encoder,
            &self.framebuf,
            self.frame_number,
            &self.stream_info,
        )
        .map_err(|error| io::Error::other(format!("FLAC frame encoding failed: {error:?}")))?;
        self.sink.clear();
        frame
            .write(&mut self.sink)
            .map_err(|error| io::Error::other(format!("FLAC frame write failed: {error:?}")))?;
        self.writer.write_all(self.sink.as_slice())?;
        self.stream_info.update_frame_info(&frame);
        self.frame_number += 1;
        Ok(())
    }
}

impl<W: Write + Seek + Send + 'static> RecordingSink for FlacRecording<W> {
    fn write_interleaved(&mut self, samples: &[f32], channels: usize) -> io::Result<()> {
        if channels != self.channels {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "FLAC and recorder channel counts differ",
            ));
        }
        let frames = samples.len() / channels;
        if frames == 0 {
            return Ok(());
        }
        self.pending.extend(
            samples[..frames * channels]
                .iter()
                .map(|&sample| quantize(sample)),
        );
        self.drain_blocks(false)?;
        self.written_frames += frames as u64;
        Ok(())
    }

    fn written_frames(&self) -> u64 {
        self.written_frames
    }

    fn finish(&mut self) -> io::Result<()> {
        self.drain_blocks(true)?;
        self.stream_info.set_md5_digest(&self.context.md5_digest());
        self.sink.clear();
        self.stream_info
            .write(&mut self.sink)
            .map_err(|error| io::Error::other(format!("FLAC header write failed: {error:?}")))?;
        debug_assert_eq!(self.sink.len(), STREAMINFO_BYTES * 8);
        self.writer.seek(SeekFrom::Start(STREAMINFO_OFFSET))?;
        self.writer.write_all(self.sink.as_slice())?;
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests;
