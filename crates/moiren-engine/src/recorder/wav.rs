use std::{
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::Path,
};

use super::{MAX_CHANNELS, RecorderReader, session::DRAIN_CHUNK_FRAMES, sink::RecordingSink};

const RIFF_PREFIX_BYTES: u64 = 50;
const MAX_DATA_BYTES: u64 = u32::MAX as u64 - RIFF_PREFIX_BYTES;

/// Control-side IEEE float32 WAV output. Stop capture, wait for the reader's
/// `is_quiescent`, drain the tail, then call `finish` to patch lengths and report
/// flush errors. `RecorderReader::record_wav` manages these steps. Dropping
/// this helper does not finalize a file or conceal a failed finalization.
pub struct WavRecording<W: Write + Seek = BufWriter<File>> {
    writer: W,
    channels: usize,
    data_bytes: u64,
    failed: bool,
    scratch: Vec<f32>,
    byte_scratch: Vec<u8>,
}

impl WavRecording<BufWriter<File>> {
    pub fn create(path: impl AsRef<Path>, channels: usize, sample_rate: u32) -> io::Result<Self> {
        validate_format(channels, sample_rate)?;
        let writer = BufWriter::new(File::create(path)?);
        Self::from_writer(writer, channels, sample_rate)
    }
}

impl<W: Write + Seek> WavRecording<W> {
    /// The writer must be empty. This also permits in-memory destinations for
    /// offline recording and deterministic IO failure tests.
    pub fn from_writer(mut writer: W, channels: usize, sample_rate: u32) -> io::Result<Self> {
        validate_format(channels, sample_rate)?;
        if writer.seek(SeekFrom::End(0))? != 0 {
            return Err(invalid_input("WAV destination must be empty"));
        }
        // Non-PCM WAVEFORMATEX includes cbSize=0; fact stores the frame count.
        writer.write_all(b"RIFF")?;
        writer.write_all(&0u32.to_le_bytes())?;
        writer.write_all(b"WAVE")?;
        writer.write_all(b"fmt ")?;
        writer.write_all(&18u32.to_le_bytes())?;
        writer.write_all(&3u16.to_le_bytes())?;
        writer.write_all(&(channels as u16).to_le_bytes())?;
        writer.write_all(&sample_rate.to_le_bytes())?;
        writer.write_all(&(sample_rate * channels as u32 * 4).to_le_bytes())?;
        writer.write_all(&((channels * 4) as u16).to_le_bytes())?;
        writer.write_all(&32u16.to_le_bytes())?;
        writer.write_all(&0u16.to_le_bytes())?;
        writer.write_all(b"fact")?;
        writer.write_all(&4u32.to_le_bytes())?;
        writer.write_all(&0u32.to_le_bytes())?;
        writer.write_all(b"data")?;
        writer.write_all(&0u32.to_le_bytes())?;
        let scratch_samples = channels
            .checked_mul(DRAIN_CHUNK_FRAMES)
            .ok_or_else(|| invalid_input("WAV scratch buffer size overflow"))?;
        let scratch_bytes = scratch_samples
            .checked_mul(4)
            .ok_or_else(|| invalid_input("WAV byte scratch buffer size overflow"))?;
        Ok(Self {
            writer,
            channels,
            data_bytes: 0,
            failed: false,
            scratch: vec![0.0; scratch_samples],
            byte_scratch: vec![0; scratch_bytes],
        })
    }

    pub fn written_frames(&self) -> u64 {
        self.frame_count()
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
            .checked_mul(self.channels as u64 * 4)
            .ok_or_else(|| invalid_input("WAV data size overflow"))?;
        if self
            .data_bytes
            .checked_add(additional_bytes)
            .is_none_or(|next| next > MAX_DATA_BYTES)
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
            if let Err(error) = Self::write_samples(
                &mut self.writer,
                &mut self.byte_scratch,
                &self.scratch[..read * self.channels],
            ) {
                // write_all may have written a partial frame. The reader has
                // advanced, so resuming could silently produce corrupt audio.
                self.failed = true;
                return Err(error);
            }
            self.data_bytes += (read * self.channels * 4) as u64;
            drained += read;
            remaining -= read;
            if read < frames {
                break;
            }
        }
        Ok(drained)
    }

    /// Converts interleaved samples to little-endian bytes and writes them.
    /// Split borrows let `drain_frames` feed `self.scratch` without copying.
    fn write_samples(
        writer: &mut W,
        byte_scratch: &mut Vec<u8>,
        samples: &[f32],
    ) -> io::Result<()> {
        let used_bytes = samples.len() * 4;
        if byte_scratch.len() < used_bytes {
            byte_scratch.resize(used_bytes, 0);
        }
        for (destination, &sample) in byte_scratch[..used_bytes]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(samples)
        {
            destination.copy_from_slice(&sample.to_le_bytes());
        }
        writer.write_all(&byte_scratch[..used_bytes])
    }

    /// Patches the RIFF/fact/data lengths and flushes, without consuming the
    /// writer so the sink trait can finalize in place.
    fn finalize(&mut self) -> io::Result<()> {
        self.check_healthy()?;
        let riff_size = RIFF_PREFIX_BYTES
            .checked_add(self.data_bytes)
            .ok_or_else(|| invalid_input("WAV RIFF size overflow"))?;
        if riff_size > u32::MAX as u64 || self.frame_count() > u32::MAX as u64 {
            return Err(invalid_input("WAV RIFF size exceeds the 32-bit limit"));
        }
        let data_size = u32::try_from(self.data_bytes)
            .map_err(|_| invalid_input("WAV data size exceeds the 32-bit limit"))?;
        let frames = u32::try_from(self.frame_count())
            .map_err(|_| invalid_input("WAV frame count exceeds the 32-bit limit"))?;
        self.writer.seek(SeekFrom::Start(4))?;
        self.writer.write_all(&(riff_size as u32).to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(46))?;
        self.writer.write_all(&frames.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(54))?;
        self.writer.write_all(&data_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::End(0))?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.finalize()?;
        Ok(self.writer)
    }

    /// Frame count without borrowing conflicts with the same-named sink
    /// method; both delegate here.
    fn frame_count(&self) -> u64 {
        self.data_bytes / (self.channels as u64 * 4)
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

impl<W: Write + Seek + Send + 'static> RecordingSink for WavRecording<W> {
    fn write_interleaved(&mut self, samples: &[f32], channels: usize) -> io::Result<()> {
        self.check_healthy()?;
        if channels != self.channels {
            return Err(invalid_input("WAV and recorder channel counts differ"));
        }
        let additional_bytes = (samples.len() as u64)
            .checked_mul(4)
            .ok_or_else(|| invalid_input("WAV data size overflow"))?;
        if self
            .data_bytes
            .checked_add(additional_bytes)
            .is_none_or(|next| next > MAX_DATA_BYTES)
        {
            return Err(invalid_input("WAV RIFF size exceeds the 32-bit limit"));
        }
        if let Err(error) = Self::write_samples(&mut self.writer, &mut self.byte_scratch, samples) {
            self.failed = true;
            return Err(error);
        }
        self.data_bytes += additional_bytes;
        Ok(())
    }

    fn written_frames(&self) -> u64 {
        self.frame_count()
    }

    fn finish(&mut self) -> io::Result<()> {
        self.finalize()
    }
}

fn validate_format(channels: usize, sample_rate: u32) -> io::Result<()> {
    if !(1..=MAX_CHANNELS).contains(&channels)
        || sample_rate == 0
        || channels
            .checked_mul(4)
            .is_none_or(|align| align > u16::MAX as usize)
        || (sample_rate as u64)
            .checked_mul(channels as u64)
            .and_then(|rate| rate.checked_mul(4))
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
