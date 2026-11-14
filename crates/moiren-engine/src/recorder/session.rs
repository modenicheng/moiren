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
        mpsc,
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
    /// Session-level pause intent. The live gate travels with the current
    /// reader, so the worker re-applies this flag whenever a rebind installs
    /// a replacement reader.
    pause_requested: AtomicBool,
    current_pause: Mutex<Option<RecorderPause>>,
    status: Mutex<RecordingStatus>,
}

/// Owns a recording worker, never an audio host. `stop` is nonblocking; `finish`
/// waits for in-flight capture, drains the tail and reports finalization errors.
/// Drop also stops and joins, so release this handle off RT. Use `finish` when
/// errors must be observed. A slow or blocked destination can delay joining.
pub struct RecordingSession {
    shared: Arc<Shared>,
    channels: usize,
    rebind: Option<RebindHandle>,
    worker: Option<JoinHandle<()>>,
}

struct RebindHandle {
    incoming: mpsc::Sender<RecorderReader>,
    closed: Arc<AtomicBool>,
}

/// Why [`RecordingSession::reattach`] refused a replacement reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReattachError {
    /// The session was created without the rebindable source.
    NotRebindable,
    /// The replacement carries a different channel count than the file.
    ChannelMismatch { expected: usize, got: usize },
    /// The worker is gone; the session already finalized.
    Closed,
}

impl RecordingSession {
    /// Wire a replacement channel into a freshly spawned session;
    /// `RecordingSession` implements `Drop`, so the struct-update syntax is
    /// unavailable.
    fn with_rebind(mut self, handle: RebindHandle) -> Self {
        self.rebind = Some(handle);
        self
    }

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
        self.shared.pause_requested.store(true, Ordering::SeqCst);
        if let Some(handle) = self
            .shared
            .current_pause
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            handle.pause();
        }
    }

    /// Resume after [`RecordingSession::pause`].
    pub fn resume(&self) {
        self.shared.pause_requested.store(false, Ordering::SeqCst);
        if let Some(handle) = self
            .shared
            .current_pause
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            handle.resume();
        }
    }

    /// Hand the session a fresh reader from a replacement engine owner. The
    /// queued preroll is discarded, so the file continues with audio observed
    /// after the swap. Fails when the channel count changed; the caller then
    /// stops the session so the file finalizes.
    pub fn reattach(&self, reader: RecorderReader) -> Result<(), ReattachError> {
        let Some(handle) = &self.rebind else {
            return Err(ReattachError::NotRebindable);
        };
        if reader.channel_count() != self.channels {
            return Err(ReattachError::ChannelMismatch {
                expected: self.channels,
                got: reader.channel_count(),
            });
        }
        handle
            .incoming
            .send(reader)
            .map_err(|_| ReattachError::Closed)
    }

    /// No further reattach will arrive: once the current reader drains, the
    /// session finalizes instead of waiting for a replacement.
    pub fn close_rebind(&self) {
        if let Some(handle) = &self.rebind {
            handle.closed.store(true, Ordering::SeqCst);
        }
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
    /// The live reader, when one is attached. A rebindable source returns
    /// `None` while it waits for its replacement reader.
    fn reader(&mut self) -> Option<&mut RecorderReader>;
    /// Whether the worker should finalize independently of an explicit stop:
    /// a plain reader that stopped or disconnected, or a rebindable source
    /// whose slot closed and whose reader drained.
    fn finished(&mut self) -> bool;
}

impl RecordingSource for RecorderReader {
    fn reader(&mut self) -> Option<&mut RecorderReader> {
        Some(self)
    }

    fn finished(&mut self) -> bool {
        self.state() != RecorderState::Recording
    }
}

/// A source that survives engine owner swaps: the worker drains the current
/// reader's tail after a disconnect, then parks until the control side hands
/// over the replacement. Paused spans and dropped-reader tails stay out of
/// the file exactly like a plain session's.
struct RebindSource {
    current: Option<RecorderReader>,
    incoming: mpsc::Receiver<RecorderReader>,
    closed: Arc<AtomicBool>,
    shared: Arc<Shared>,
}

impl RebindSource {
    fn install(&mut self, mut next: RecorderReader) {
        // A replacement starts clean: discard its preroll so the file
        // continues with live audio observed after the swap.
        next.skip_queued();
        next.start();
        let handle = next.pause_handle();
        if self.shared.pause_requested.load(Ordering::SeqCst) {
            handle.pause();
        }
        *self
            .shared
            .current_pause
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(handle);
        self.current = Some(next);
    }
}

impl RecordingSource for RebindSource {
    fn reader(&mut self) -> Option<&mut RecorderReader> {
        let drained_dead = self.current.as_ref().is_some_and(|reader| {
            reader.state() == RecorderState::Disconnected && reader.queued_frames() == 0
        });
        if drained_dead || self.current.is_none() {
            match self.incoming.try_recv() {
                Ok(next) => self.install(next),
                Err(_) => self.current = None,
            }
        }
        self.current.as_mut()
    }

    fn finished(&mut self) -> bool {
        self.closed.load(Ordering::SeqCst) && self.reader().is_none()
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

    /// Managed variant for recordings that must survive engine owner swaps:
    /// the queued preroll is discarded like the `_now` entry points, and the
    /// returned session accepts replacement readers via
    /// [`RecordingSession::reattach`] instead of finalizing when its reader
    /// disconnects.
    pub fn record_rebindable<S: RecordingSink>(mut self, sink: S) -> io::Result<RecordingSession> {
        self.skip_queued();
        // Restart acceptance immediately so capture observed right after this
        // call is recorded even before the worker's own start handshake.
        self.start();
        let channels = self.channel_count();
        let stats = self.stats();
        let (sender, incoming) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let handle = RebindHandle {
            incoming: sender,
            closed: Arc::clone(&closed),
        };
        build_session(sink, channels, stats, None, move |shared| {
            Box::new(RebindSource {
                current: Some(self),
                incoming,
                closed,
                shared,
            })
        })
        .map(|session| session.with_rebind(handle))
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
struct SourceGuard(Box<dyn RecordingSource>);
impl Drop for SourceGuard {
    fn drop(&mut self) {
        settle_source(self.0.as_mut());
    }
}

pub(crate) fn spawn_recording<T: RecordingSource, S: RecordingSink>(
    mut source: T,
    sink: S,
) -> io::Result<RecordingSession> {
    let channels = source
        .reader()
        .expect("a fresh source owns a reader")
        .channel_count();
    let stats = source
        .reader()
        .expect("a fresh source owns a reader")
        .stats();
    build_session(sink, channels, stats, None, move |_| Box::new(source))
}

/// Shared spawn body: the source factory receives the freshly built
/// [`Shared`] block so a rebindable source can wire its swap-time pause
/// propagation before the worker starts.
fn build_session<S: RecordingSink>(
    sink: S,
    channels: usize,
    initial_stats: RecorderStats,
    rebind: Option<RebindHandle>,
    source: impl FnOnce(Arc<Shared>) -> Box<dyn RecordingSource>,
) -> io::Result<RecordingSession> {
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        pause_requested: AtomicBool::new(false),
        current_pause: Mutex::new(None),
        status: Mutex::new(RecordingStatus {
            state: RecordingState::Recording,
            paused: false,
            written_frames: 0,
            stats: initial_stats,
            error: None,
        }),
    });
    let mut source = source(Arc::clone(&shared));
    *shared
        .current_pause
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = source.reader().map(|reader| reader.pause_handle());
    let worker_shared = Arc::clone(&shared);
    let source = SourceGuard(source);
    let worker = thread::Builder::new()
        .name("moiren-recording".into())
        .spawn(move || {
            let mut source = source;
            let result = catch_unwind(AssertUnwindSafe(|| {
                let mut sink = sink;
                // Channel counts are bounded by `builtin::MAX_CHANNELS`, so
                // the interleaved scratch cannot overflow.
                let mut scratch = vec![0.0; channels * DRAIN_CHUNK_FRAMES];
                run(
                    source.0.as_mut(),
                    &mut sink,
                    channels,
                    &mut scratch,
                    &worker_shared,
                )
            }))
            .unwrap_or_else(|_| Err(io::Error::other("recording destination panicked")));
            settle_source(source.0.as_mut());
            let stats = source.0.as_mut().reader().map(|reader| reader.stats());
            // Return a leased reader before publishing terminal status. Observers
            // can rely on Finished/Failed to mean the queue is no longer claimed.
            drop(source);
            let mut status = worker_shared
                .status
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(stats) = stats {
                status.stats = stats;
            }
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
        channels,
        rebind,
        worker: Some(worker),
    })
}

fn settle_source(source: &mut dyn RecordingSource) {
    let Some(reader) = source.reader() else {
        return;
    };
    reader.stop();
    while !reader.is_quiescent() {
        thread::park_timeout(Duration::from_millis(2));
    }
}

fn run<S: RecordingSink>(
    source: &mut dyn RecordingSource,
    sink: &mut S,
    channels: usize,
    scratch: &mut [f32],
    shared: &Shared,
) -> io::Result<u64> {
    if !shared.stop.load(Ordering::Acquire) {
        // A fresh session records from now: clear any pause gate a previous
        // session on this reader left behind.
        if let Some(reader) = source.reader() {
            reader.start();
            reader.resume();
        }
    }
    loop {
        let stopping = shared.stop.load(Ordering::Acquire) || source.finished();
        if stopping && let Some(reader) = source.reader() {
            reader.stop();
        }
        let drained = drain_once(source, sink, channels, scratch);
        {
            let mut status = shared.status.lock().unwrap_or_else(|p| p.into_inner());
            status.state = if stopping {
                RecordingState::Stopping
            } else {
                RecordingState::Recording
            };
            // The session-level gate is authoritative: a rebindable source
            // waiting for its replacement reports the intent, not the
            // missing reader.
            status.paused = shared.pause_requested.load(Ordering::SeqCst)
                || source.reader().is_some_and(|reader| reader.is_paused());
            if let Some(reader) = source.reader() {
                status.stats = reader.stats();
            }
            status.written_frames = sink.written_frames();
        }
        let drained = drained?;
        // Checking queue emptiness alone could lose a block already entering
        // observe(). The stop handshake proves that no later publication can
        // follow this final queue snapshot.
        if stopping {
            match source.reader() {
                Some(reader) => {
                    if reader.is_quiescent() && reader.queued_frames() == 0 {
                        break;
                    }
                }
                // No reader attached: nothing is in flight, finalize now.
                None => break,
            }
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
fn drain_once<S: RecordingSink>(
    source: &mut dyn RecordingSource,
    sink: &mut S,
    channels: usize,
    scratch: &mut [f32],
) -> io::Result<usize> {
    let Some(reader) = source.reader() else {
        return Ok(0);
    };
    let want = reader.queued_frames().min(DRAIN_CHUNK_FRAMES);
    if want == 0 {
        return Ok(0);
    }
    let read = reader.read_interleaved(&mut scratch[..want * channels]);
    if read == 0 {
        return Ok(0);
    }
    sink.write_interleaved(&scratch[..read * channels], channels)?;
    Ok(read)
}

#[cfg(test)]
mod tests;
