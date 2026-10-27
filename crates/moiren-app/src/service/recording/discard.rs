//! Establish a fresh capture boundary using the engine's existing stop barrier.
use std::io::{self, Seek, SeekFrom, Write};

use moiren_engine::host::ManagedRecorderReader;

pub(super) fn before_start(reader: &ManagedRecorderReader, sample_rate: u32) -> io::Result<()> {
    if !reader.stop() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Recorder endpoint is no longer available",
        ));
    }
    // ManagedRecorderReader exposes read_interleaved but not is_quiescent.
    // An empty read, or a sleep, cannot rule out a final in-flight RT publish.
    // A temporary lease to a seekable sink reuses RecordingSession::finish's
    // stop/quiescence/tail-drain handshake. It returns the queue stopped and
    // empty before the real file worker may arm it again. All waits, allocations
    // and discarded sample conversion stay on the private control-side worker.
    reader
        .record_with_writer(DiscardWriter::default(), sample_rate)?
        .finish()?;
    Ok(())
}

#[derive(Default)]
struct DiscardWriter {
    position: u64,
    length: u64,
}

impl Write for DiscardWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.position = self
            .position
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("Discard writer length overflow"))?;
        self.length = self.length.max(self.position);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for DiscardWriter {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.length) + i128::from(offset),
        };
        self.position = u64::try_from(position).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid discard writer seek")
        })?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests;
