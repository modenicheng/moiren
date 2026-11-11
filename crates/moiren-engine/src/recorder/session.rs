//! Format-agnostic session worker. File IO and finalization run on a
//! dedicated control-side thread; the source owns the only queue consumer and
//! no host directory lock is held while writing. Sinks are chosen by the
//! caller ([`crate::recorder::WavRecording`], `Mp3Recording`, …).
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use super::sink::RecordingSink;
use crate::recorder::{RecorderPause, RecorderReader, RecorderState, RecorderStats};

/// Frames pulled from the queue per worker iteration, bounding the scratch
/// buffer and the time a custom destination can hold the loop.
pub(crate) const DRAIN_CHUNK_FRAMES: usize = 4096;

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
    /// Capture gate reported alongside the run state: a paused session keeps
    /// `Recording`, but `written_frames` stops advancing until it resumes.
    pub paused: bool,
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

/// Owns a recording worker, never an audio host. `stop` is nonblocking; `finish`
/// waits for in-flight capture, drains the tail and reports finalization errors.
/// Drop also stops and joins, so release this handle off RT. Use `finish` when
/// errors must be observed. A slow or blocked destination can delay joining.
pub struct RecordingSession {
    shared: Arc<Shared>,
    pause: RecorderPause,
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

    /// Skip capture without finalizing the file: paused spans stay out of the
    /// recording, so `written_frames` counts recorded audio only. The gate is
    /// shared with the observer, so it takes effect on the next audio block.
    pub fn pause(&self) {
        self.pause.pause();
    }

    /// Resume after [`RecordingSession::pause`].
    pub fn resume(&self) {
        self.pause.resume();
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
        let wav = super::WavRecording::create(path, self.channel_count(), sample_rate)?;
        spawn_recording(self, wav)
    }

    /// Like `record_wav`, but audio queued before this call is discarded, so
    /// the file starts with capture observed after this call. The discard
    /// stops acceptance and waits out any in-flight observer block first.
    pub fn record_wav_now(
        mut self,
        path: impl AsRef<Path>,
        sample_rate: u32,
    ) -> io::Result<RecordingSession> {
        let wav = super::WavRecording::create(path, self.channel_count(), sample_rate)?;
        self.skip_queued();
        spawn_recording(self, wav)
    }

    /// Like `record_wav_now`, encoded to MP3 with the caller's settings.
    /// Mono or stereo sessions only; the sample rate must be one LAME
    /// supports natively.
    #[cfg(feature = "mp3")]
    pub fn record_mp3_now(
        mut self,
        path: impl AsRef<Path>,
        sample_rate: u32,
        encoding: super::mp3::Mp3Encoding,
    ) -> io::Result<RecordingSession> {
        let mp3 =
            super::mp3::Mp3Recording::create(path, self.channel_count(), sample_rate, encoding)?;
        self.skip_queued();
        spawn_recording(self, mp3)
    }

    /// Like `record_wav_now`, encoded losslessly to FLAC with the caller's
    /// compression level. Up to eight channels; the sample rate must fit
    /// STREAMINFO's 20 bits.
    #[cfg(feature = "flac")]
    pub fn record_flac_now(
        mut self,
        path: impl AsRef<Path>,
        sample_rate: u32,
        encoding: super::flac::FlacEncoding,
    ) -> io::Result<RecordingSession> {
        let flac =
            super::flac::FlacRecording::create(path, self.channel_count(), sample_rate, encoding)?;
        self.skip_queued();
        spawn_recording(self, flac)
    }

    /// Control-side variant for custom destinations. All subsequent writes,
    /// seeks and flushes belong exclusively to the recording worker.
    pub fn record_with_writer<W: io::Write + io::Seek + Send + 'static>(
        self,
        writer: W,
        sample_rate: u32,
    ) -> io::Result<RecordingSession> {
        let wav = super::WavRecording::from_writer(writer, self.channel_count(), sample_rate)?;
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

pub(crate) fn spawn_recording<T: RecordingSource, S: RecordingSink>(
    mut source: T,
    sink: S,
) -> io::Result<RecordingSession> {
    // The handle gates capture through the shared control block, so it stays
    // usable while the worker owns the reader.
    let pause = source.reader().pause_handle();
    let channels = source.reader().channel_count();
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        status: Mutex::new(RecordingStatus {
            state: RecordingState::Recording,
            paused: pause.is_paused(),
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
            let result = catch_unwind(AssertUnwindSafe(|| {
                let mut sink = sink;
                // Channel counts are bounded by `builtin::MAX_CHANNELS`, so
                // the interleaved scratch cannot overflow.
                let mut scratch = vec![0.0; channels * DRAIN_CHUNK_FRAMES];
                run(
                    &mut source.0,
                    &mut sink,
                    channels,
                    &mut scratch,
                    &worker_shared,
                )
            }))
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
        pause,
        worker: Some(worker),
    })
}

fn run<T: RecordingSource, S: RecordingSink>(
    source: &mut T,
    sink: &mut S,
    channels: usize,
    scratch: &mut [f32],
    shared: &Shared,
) -> io::Result<u64> {
    if source.is_active() && !shared.stop.load(Ordering::Acquire) {
        // A fresh session records from now: clear any pause gate a previous
        // session on this reader left behind.
        source.reader().start();
        source.reader().resume();
    }
    loop {
        let stopping = shared.stop.load(Ordering::Acquire)
            || !source.is_active()
            || source.reader().state() != RecorderState::Recording;
        if stopping {
            source.reader().stop();
        }
        let drained = drain_once(source, sink, channels, scratch);
        {
            let mut status = shared.status.lock().unwrap_or_else(|p| p.into_inner());
            status.state = if stopping {
                RecordingState::Stopping
            } else {
                RecordingState::Recording
            };
            status.paused = source.reader().is_paused();
            status.written_frames = sink.written_frames();
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
    let written_frames = sink.written_frames();
    sink.finish()?;
    Ok(written_frames)
}

/// Pulls at most [`DRAIN_CHUNK_FRAMES`] queued frames into `scratch` and hands
/// them to the sink. The result is produced before the caller updates status,
/// mirroring the pre-sink drain error reporting order.
fn drain_once<T: RecordingSource, S: RecordingSink>(
    source: &mut T,
    sink: &mut S,
    channels: usize,
    scratch: &mut [f32],
) -> io::Result<usize> {
    let want = source.reader().queued_frames().min(DRAIN_CHUNK_FRAMES);
    if want == 0 {
        return Ok(0);
    }
    let read = source
        .reader()
        .read_interleaved(&mut scratch[..want * channels]);
    if read == 0 {
        return Ok(0);
    }
    sink.write_interleaved(&scratch[..read * channels], channels)?;
    Ok(read)
}

#[cfg(test)]
mod tests;
