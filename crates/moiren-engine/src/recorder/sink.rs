//! Format-agnostic control-side destination for a recording session.
use std::io;

/// A recording destination owned by the dedicated `"moiren-recording"`
/// worker. All methods run on that thread only, never RT, and implementations
/// buffer internally. `channels` is fixed for the whole session and always
/// divides `samples.len()`.
pub trait RecordingSink: Send + 'static {
    /// Append interleaved frames. A partial write must surface as an error:
    /// the worker stops the session on the first failure and never calls this
    /// again.
    fn write_interleaved(&mut self, samples: &[f32], channels: usize) -> io::Result<()>;
    /// Frames successfully handed to the destination so far.
    fn written_frames(&self) -> u64;
    /// Finalize the destination; called once after the last write.
    fn finish(&mut self) -> io::Result<()>;
}
