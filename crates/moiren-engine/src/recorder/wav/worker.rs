//! File IO and finalization run on a dedicated control-side worker. The source
//! owns the only consumer; no host directory lock is held while writing.
use std::{
    io::{self, Seek, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use super::{DRAIN_CHUNK_FRAMES, WavFormat, WavRecording};
use crate::recorder::{RecorderReader, RecorderState, RecorderStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    Recording,
    Stopping,
    Finished,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingError {
    pub kind: io::ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingStatus {
    pub state: RecordingState,
    pub written_frames: u64,
    pub stats: RecorderStats,
    pub error: Option<RecordingError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingSummary {
    pub written_frames: u64,
    pub stats: RecorderStats,
}

struct Shared {
    stop: AtomicBool,
    status: Mutex<RecordingStatus>,
}

/// Owns a file worker, never an audio host. `stop` is nonblocking; `finish`
/// waits for in-flight capture, drains the tail and reports finalization errors.
/// Drop also stops and joins, so release this handle off RT. Use `finish` when
/// errors must be observed. A slow or blocked destination can delay joining.
pub struct RecordingSession {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl RecordingSession {
    pub fn status(&self) -> RecordingStatus {
        self.shared
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }

    pub fn finish(mut self) -> io::Result<RecordingSummary> {
        self.stop();
        self.join()?;
        let status = self.status();
        if let Some(error) = status.error {
            return Err(io::Error::new(error.kind, error.message));
        }
        Ok(RecordingSummary {
            written_frames: status.written_frames,
            stats: status.stats,
        })
    }

    fn join(&mut self) -> io::Result<()> {
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("recording worker panicked"))?;
        }
        Ok(())
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.stop();
        let _ = self.join();
    }
}

pub(crate) trait RecordingSource: Send + 'static {
    fn reader(&mut self) -> &mut RecorderReader;
    fn is_active(&self) -> bool {
        true
    }
}

impl RecordingSource for RecorderReader {
    fn reader(&mut self) -> &mut RecorderReader {
        self
    }
}

impl RecorderReader {
    /// Consume the sole queue reader and start an automatic WAV worker.
    /// Existing queued frames are included. `sample_rate` is the graph's
    /// processing rate, which can differ from the physical device's mix rate.
    pub fn record_wav(
        self,
        path: impl AsRef<Path>,
        sample_rate: u32,
    ) -> io::Result<RecordingSession> {
        self.record_wav_format(path, sample_rate, WavFormat::Float32)
    }

    /// Explicit encoding; capture transport remains float32 and conversion is
    /// performed off RT by the file worker. See [`WavFormat`] for PCM policy.
    pub fn record_wav_format(
        self,
        path: impl AsRef<Path>,
        sample_rate: u32,
        format: WavFormat,
    ) -> io::Result<RecordingSession> {
        let wav =
            WavRecording::create_with_format(path, self.channel_count(), sample_rate, format)?;
        spawn_recording(self, wav)
    }

    /// Control-side variant for custom destinations. All subsequent writes,
    /// seeks and flushes belong exclusively to the recording worker.
    pub fn record_with_writer<W: Write + Seek + Send + 'static>(
        self,
        writer: W,
        sample_rate: u32,
    ) -> io::Result<RecordingSession> {
        self.record_with_writer_format(writer, sample_rate, WavFormat::Float32)
    }

    /// Explicit-encoding variant; caller-owned writer must be empty.
    pub fn record_with_writer_format<W: Write + Seek + Send + 'static>(
        self,
        writer: W,
        sample_rate: u32,
        format: WavFormat,
    ) -> io::Result<RecordingSession> {
        let wav = WavRecording::from_writer_with_format(
            writer,
            self.channel_count(),
            sample_rate,
            format,
        )?;
        spawn_recording(self, wav)
    }
}

// This guard also stops capture if spawning fails or a custom writer panics.
struct SourceGuard<T: RecordingSource>(T);
impl<T: RecordingSource> Drop for SourceGuard<T> {
    fn drop(&mut self) {
        self.0.reader().stop();
        // Builder::spawn can fail before the worker body runs. Settle that
        // path too, before a managed source returns its exclusive reader.
        while !self.0.reader().is_quiescent() {
            thread::park_timeout(Duration::from_millis(2));
        }
    }
}

pub(crate) fn spawn_recording<T: RecordingSource, W: Write + Seek + Send + 'static>(
    mut source: T,
    wav: WavRecording<W>,
) -> io::Result<RecordingSession> {
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        status: Mutex::new(RecordingStatus {
            state: RecordingState::Recording,
            written_frames: 0,
            stats: source.reader().stats(),
            error: None,
        }),
    });
    let mut source = SourceGuard(source);
    let worker_shared = Arc::clone(&shared);
    let worker = thread::Builder::new()
        .name("moiren-recording".into())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| run(&mut source.0, wav, &worker_shared)))
                .unwrap_or_else(|_| Err(io::Error::other("recording destination panicked")));
            source.0.reader().stop();
            while !source.0.reader().is_quiescent() {
                thread::park_timeout(Duration::from_millis(2));
            }
            let stats = source.0.reader().stats();
            // Return a leased reader before publishing terminal status. Observers
            // can rely on Finished/Failed to mean the queue is no longer claimed.
            drop(source);
            let mut status = worker_shared
                .status
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            status.stats = stats;
            match result {
                Ok(written_frames) => {
                    status.written_frames = written_frames;
                    status.state = RecordingState::Finished;
                }
                Err(error) => {
                    status.state = RecordingState::Failed;
                    status.error = Some(RecordingError {
                        kind: error.kind(),
                        message: error.to_string(),
                    });
                }
            }
        })?;
    Ok(RecordingSession {
        shared,
        worker: Some(worker),
    })
}

fn run<T: RecordingSource, W: Write + Seek>(
    source: &mut T,
    mut wav: WavRecording<W>,
    shared: &Shared,
) -> io::Result<u64> {
    if source.is_active() && !shared.stop.load(Ordering::Acquire) {
        source.reader().start();
    }
    loop {
        let stopping = shared.stop.load(Ordering::Acquire)
            || !source.is_active()
            || source.reader().state() != RecorderState::Recording;
        if stopping {
            source.reader().stop();
        }
        let drained = wav.drain_frames(source.reader(), DRAIN_CHUNK_FRAMES);
        {
            let mut status = shared.status.lock().unwrap_or_else(|p| p.into_inner());
            status.state = if stopping {
                RecordingState::Stopping
            } else {
                RecordingState::Recording
            };
            status.written_frames = wav.written_frames();
            status.stats = source.reader().stats();
        }
        let drained = drained?;
        // Checking queue emptiness alone could lose a block already entering
        // observe(). The stop handshake proves that no later publication can
        // follow this final queue snapshot.
        if stopping && source.reader().is_quiescent() && source.reader().queued_frames() == 0 {
            break;
        }
        if drained == 0 {
            thread::park_timeout(Duration::from_millis(2));
        }
    }
    let written_frames = wav.written_frames();
    wav.finish()?;
    Ok(written_frames)
}

#[cfg(test)]
mod tests;
