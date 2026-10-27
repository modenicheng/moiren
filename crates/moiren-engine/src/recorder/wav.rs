use std::{
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::Path,
};

use super::{MAX_CHANNELS, RecorderReader};

mod format;
pub use format::WavFormat;
mod worker;
pub use worker::{
    RecordingError, RecordingSession, RecordingState, RecordingStatus, RecordingSummary,
};
pub(crate) use worker::{RecordingSource, spawn_recording};

const DRAIN_CHUNK_FRAMES: usize = 4096;

/// Control-side WAV output, defaulting to IEEE float32. Stop capture, wait for the reader's
/// `is_quiescent`, drain the tail, then call `finish` to patch lengths and report
/// flush errors. `RecorderReader::record_wav` manages these steps. Dropping
/// this helper does not finalize a file or conceal a failed finalization.
pub struct WavRecording<W: Write + Seek = BufWriter<File>> {
    writer: W,
    channels: usize,
    format: WavFormat,
    data_bytes: u64,
    failed: bool,
    scratch: Vec<f32>,
    byte_scratch: Vec<u8>,
}

impl WavRecording<BufWriter<File>> {
    pub fn create(path: impl AsRef<Path>, channels: usize, sample_rate: u32) -> io::Result<Self> {
        Self::create_with_format(path, channels, sample_rate, WavFormat::Float32)
    }

    pub fn create_with_format(
        path: impl AsRef<Path>,
        channels: usize,
        sample_rate: u32,
        format: WavFormat,
    ) -> io::Result<Self> {
        validate_format(channels, sample_rate, format)?;
        let writer = BufWriter::new(File::create(path)?);
        Self::from_writer_with_format(writer, channels, sample_rate, format)
    }
}

impl<W: Write + Seek> WavRecording<W> {
    /// The writer must be empty. This also permits in-memory destinations for
    /// offline recording and deterministic IO failure tests.
    pub fn from_writer(writer: W, channels: usize, sample_rate: u32) -> io::Result<Self> {
        Self::from_writer_with_format(writer, channels, sample_rate, WavFormat::Float32)
    }

    /// Select encoding before writing any bytes; see [`WavFormat`] for the PCM
    /// saturation, non-finite and no-dither policy. The destination must be empty.
    pub fn from_writer_with_format(
        mut writer: W,
        channels: usize,
        sample_rate: u32,
        format: WavFormat,
    ) -> io::Result<Self> {
        validate_format(channels, sample_rate, format)?;
        if writer.seek(SeekFrom::End(0))? != 0 {
            return Err(invalid_input("WAV destination must be empty"));
        }
        let float = format == WavFormat::Float32;
        let sample_bytes = format.bytes_per_sample();
        // Preserve the original float WAVEFORMATEX/fact layout. Both integer
        // formats use WAVEFORMATEXTENSIBLE, including widths beyond stereo.
        writer.write_all(b"RIFF")?;
        writer.write_all(&0u32.to_le_bytes())?;
        writer.write_all(b"WAVE")?;
        writer.write_all(b"fmt ")?;
        writer.write_all(&(if float { 18u32 } else { 40 }).to_le_bytes())?;
        writer.write_all(&(if float { 3u16 } else { 0xfffe }).to_le_bytes())?;
        writer.write_all(&(channels as u16).to_le_bytes())?;
        writer.write_all(&sample_rate.to_le_bytes())?;
        writer
            .write_all(&(sample_rate * channels as u32 * u32::from(sample_bytes)).to_le_bytes())?;
        writer.write_all(&(channels as u16 * sample_bytes).to_le_bytes())?;
        writer.write_all(&(sample_bytes * 8).to_le_bytes())?;
        if float {
            writer.write_all(&0u16.to_le_bytes())?;
            writer.write_all(b"fact")?;
            writer.write_all(&4u32.to_le_bytes())?;
            writer.write_all(&0u32.to_le_bytes())?;
        } else {
            writer.write_all(&22u16.to_le_bytes())?;
            writer.write_all(&(sample_bytes * 8).to_le_bytes())?;
            // Graph channels carry no speaker assignment: do not invent one.
            writer.write_all(&0u32.to_le_bytes())?;
            // KSDATAFORMAT_SUBTYPE_PCM: 00000001-0000-0010-8000-00aa00389b71.
            writer.write_all(&[1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113])?;
        }
        writer.write_all(b"data")?;
        writer.write_all(&0u32.to_le_bytes())?;
        let scratch_samples = channels
            .checked_mul(DRAIN_CHUNK_FRAMES)
            .ok_or_else(|| invalid_input("WAV scratch buffer size overflow"))?;
        let scratch_bytes = scratch_samples
            .checked_mul(sample_bytes as usize)
            .ok_or_else(|| invalid_input("WAV byte scratch buffer size overflow"))?;
        Ok(Self {
            writer,
            channels,
            format,
            data_bytes: 0,
            failed: false,
            scratch: vec![0.0; scratch_samples],
            byte_scratch: vec![0; scratch_bytes],
        })
    }

    pub fn written_frames(&self) -> u64 {
        self.data_bytes / (self.channels as u64 * u64::from(self.format.bytes_per_sample()))
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Drains only the queue contents observed at the start of this call.
    pub fn drain(&mut self, reader: &mut RecorderReader) -> io::Result<usize> {
        self.drain_frames(reader, reader.queued_frames())
    }

    /// Drains at most `max_frames`, making it suitable for bounded control work.
    pub fn drain_frames(
        &mut self,
        reader: &mut RecorderReader,
        max_frames: usize,
    ) -> io::Result<usize> {
        self.check_healthy()?;
        if reader.channel_count() != self.channels {
            return Err(invalid_input("WAV and recorder channel counts differ"));
        }
        let mut remaining = max_frames.min(reader.queued_frames());
        // Check the entire snapshot before consuming anything. RIFF includes
        // its headers in the u32 length, so checking data length alone is wrong.
        let additional_bytes = (remaining as u64)
            .checked_mul(self.channels as u64 * u64::from(self.format.bytes_per_sample()))
            .ok_or_else(|| invalid_input("WAV data size overflow"))?;
        if self
            .data_bytes
            .checked_add(additional_bytes)
            .is_none_or(|next| next > self.format.max_data_bytes())
        {
            return Err(invalid_input("WAV RIFF size exceeds the 32-bit limit"));
        }
        let mut drained = 0usize;
        while remaining > 0 {
            let frames = remaining.min(DRAIN_CHUNK_FRAMES);
            let sample_count = frames * self.channels;
            let read = reader.read_interleaved(&mut self.scratch[..sample_count]);
            if read == 0 {
                break;
            }
            let sample_bytes = self.format.bytes_per_sample() as usize;
            let used_bytes = read * self.channels * sample_bytes;
            for (destination, &sample) in self.byte_scratch[..used_bytes]
                .chunks_exact_mut(sample_bytes)
                .zip(&self.scratch[..read * self.channels])
            {
                self.format.encode(sample, destination);
            }
            if let Err(error) = self.writer.write_all(&self.byte_scratch[..used_bytes]) {
                // write_all may have written a partial frame. The reader has
                // advanced, so resuming could silently produce corrupt audio.
                self.failed = true;
                return Err(error);
            }
            self.data_bytes += used_bytes as u64;
            drained += read;
            remaining -= read;
            if read < frames {
                break;
            }
        }
        Ok(drained)
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.check_healthy()?;
        let padding = self.data_bytes & 1;
        let riff_size = (u64::from(self.format.header_bytes()) - 8)
            .checked_add(self.data_bytes)
            .and_then(|size| size.checked_add(padding))
            .ok_or_else(|| invalid_input("WAV RIFF size overflow"))?;
        if riff_size > u32::MAX as u64 || self.written_frames() > u32::MAX as u64 {
            return Err(invalid_input("WAV RIFF size exceeds the 32-bit limit"));
        }
        let data_size = u32::try_from(self.data_bytes)
            .map_err(|_| invalid_input("WAV data size exceeds the 32-bit limit"))?;
        let frames = u32::try_from(self.written_frames())
            .map_err(|_| invalid_input("WAV frame count exceeds the 32-bit limit"))?;
        // Only finalization adds the RIFF word pad, never an intermediate drain.
        // The data chunk length excludes this byte; the RIFF size includes it.
        if padding != 0 {
            self.writer.seek(SeekFrom::End(0))?;
            self.writer.write_all(&[0])?;
        }
        self.writer.seek(SeekFrom::Start(4))?;
        self.writer.write_all(&(riff_size as u32).to_le_bytes())?;
        if self.format == WavFormat::Float32 {
            self.writer.seek(SeekFrom::Start(46))?;
            self.writer.write_all(&frames.to_le_bytes())?;
        }
        self.writer
            .seek(SeekFrom::Start(u64::from(self.format.header_bytes()) - 4))?;
        self.writer.write_all(&data_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::End(0))?;
        self.writer.flush()?;
        Ok(self.writer)
    }

    fn check_healthy(&self) -> io::Result<()> {
        if self.failed {
            Err(io::Error::other(
                "WAV recording previously failed during IO",
            ))
        } else {
            Ok(())
        }
    }
}

fn validate_format(channels: usize, sample_rate: u32, format: WavFormat) -> io::Result<()> {
    if !(1..=MAX_CHANNELS).contains(&channels)
        || sample_rate == 0
        || channels
            .checked_mul(format.bytes_per_sample() as usize)
            .is_none_or(|align| align > u16::MAX as usize)
        || (sample_rate as u64)
            .checked_mul(channels as u64)
            .and_then(|rate| rate.checked_mul(u64::from(format.bytes_per_sample())))
            .is_none_or(|rate| rate > u32::MAX as u64)
    {
        return Err(invalid_input("unsupported WAV format"));
    }
    Ok(())
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests;
