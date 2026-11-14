//! Bounded recording transport. Construct and reclaim both endpoints on the
//! control side; the observer only converts and publishes complete audio blocks.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

use crate::{
    buffer::{IoMode, PreparedIo, ReadPorts},
    processor::{ProcessContext, ProcessorError, RtObserver, builtin::MAX_CHANNELS},
    sample::ProcessingSample,
};

#[cfg(feature = "flac")]
mod flac;
#[cfg(feature = "mp3")]
mod mp3;
mod session;
mod sink;
mod wav;
#[cfg(feature = "flac")]
pub use flac::{FLAC_MAX_LEVEL, FlacEncoding, FlacRecording};
#[cfg(feature = "mp3")]
pub use mp3::{MP3_BITRATE_KBPS, Mp3Encoding, Mp3Mode, Mp3Recording};
pub use session::{
    ReattachError, RecordingError, RecordingSession, RecordingState, RecordingStatus,
    RecordingSummary,
};
pub(crate) use session::{RecordingSource, spawn_recording};
pub use sink::RecordingSink;
pub use wav::WavRecording;

#[cfg(test)]
mod tests;

/// Same built-in channel budget as the processing kernels; see
/// `builtin::MAX_CHANNELS` for why builtin state is width-bounded.
const MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecorderSettings {
    pub capacity_frames: usize,
}

impl RecorderSettings {
    /// Preparation is capped independently of channel count; the factory also
    /// limits total queue storage to 64 MiB.
    pub const MAX_CAPACITY_FRAMES: usize = 1_000_000;

    pub fn valid(self) -> bool {
        (1..=Self::MAX_CAPACITY_FRAMES).contains(&self.capacity_frames)
    }
}

impl Default for RecorderSettings {
    fn default() -> Self {
        Self {
            capacity_frames: 48_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecorderState {
    Recording = 0,
    Stopped = 1,
    Disconnected = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecorderStats {
    pub accepted_frames: u64,
    pub read_frames: u64,
    pub dropped_frames: u64,
    pub state: RecorderState,
}

// Each counter has exactly one writer: the observer or the owning reader.
struct RecorderControl {
    state: AtomicU8,
    active: AtomicBool,
    // Independent of `state`: a paused recorder keeps its Recording state so
    // a worker session keeps running, while the observer skips frames without
    // counting them as drops. Paused spans are simply not recorded.
    paused: AtomicBool,
    accepted_frames: AtomicU64,
    read_frames: AtomicU64,
    dropped_frames: AtomicU64,
}

// Borrow the existing atomic; the audio thread never clones/drops an Arc here.
// Clear on every return and unwind, after any queue commit and counter updates.
struct ObserverActivity<'a>(&'a AtomicBool);

impl Drop for ObserverActivity<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub struct Recorder {
    tx: Producer<f32>,
    channels: usize,
    control: Arc<RecorderControl>,
}

pub struct RecorderReader {
    rx: Consumer<f32>,
    channels: usize,
    capacity_frames: usize,
    control: Arc<RecorderControl>,
}

/// Pause control that stays usable while a recording worker owns the reader:
/// the gate lives in the shared control block, so it flips atomics only and
/// never touches the leased queue. See [`RecorderReader::pause`].
pub struct RecorderPause {
    control: Arc<RecorderControl>,
}

impl RecorderPause {
    pub fn pause(&self) {
        self.control.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.control.paused.store(false, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.control.paused.load(Ordering::Acquire)
    }
}

/// Allocate a float32 SPSC queue off the audio thread. Both endpoints must also
/// be dropped off that thread, since the final endpoint reclaims the ring.
pub fn recorder(
    channels: usize,
    settings: RecorderSettings,
) -> Result<(Recorder, RecorderReader), ProcessorError> {
    if !(1..=MAX_CHANNELS).contains(&channels) || !settings.valid() {
        return Err(ProcessorError::InvalidIo);
    }
    let samples = channels
        .checked_mul(settings.capacity_frames)
        .ok_or(ProcessorError::InvalidIo)?;
    let bytes = samples
        .checked_mul(std::mem::size_of::<f32>())
        .ok_or(ProcessorError::InvalidIo)?;
    if bytes > MAX_BUFFER_BYTES || bytes > isize::MAX as usize {
        return Err(ProcessorError::InvalidIo);
    }

    let (tx, rx) = RingBuffer::new(samples);
    let control = Arc::new(RecorderControl {
        state: AtomicU8::new(RecorderState::Recording as u8),
        active: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        accepted_frames: AtomicU64::new(0),
        read_frames: AtomicU64::new(0),
        dropped_frames: AtomicU64::new(0),
    });
    Ok((
        Recorder {
            tx,
            channels,
            control: Arc::clone(&control),
        },
        RecorderReader {
            rx,
            channels,
            capacity_frames: settings.capacity_frames,
            control,
        },
    ))
}

impl RecorderReader {
    pub fn channel_count(&self) -> usize {
        self.channels
    }

    pub fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Enable capture starting with an observer call that sees this command.
    pub fn start(&self) {
        if !self.rx.is_abandoned() {
            self.control
                .state
                .store(RecorderState::Recording as u8, Ordering::SeqCst);
        }
    }

    /// Pause capture without clearing queued frames. An in-flight observer call
    /// may still publish its complete block; this command does not wait for it.
    /// Poll `is_quiescent()` before the final drain and file finalization.
    pub fn stop(&self) {
        if !self.rx.is_abandoned() {
            self.control
                .state
                .store(RecorderState::Stopped as u8, Ordering::SeqCst);
        }
    }

    /// Gate capture while a recording session keeps running: the state stays
    /// `Recording`, but the observer skips frames without counting them as
    /// drops. An in-flight observer call may still publish one complete block.
    pub fn pause(&self) {
        if !self.rx.is_abandoned() {
            self.control.paused.store(true, Ordering::SeqCst);
        }
    }

    /// Lift a [`RecorderReader::pause`] gate. Capture resumes with the next
    /// observer call that sees this command.
    pub fn resume(&self) {
        self.control.paused.store(false, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.control.paused.load(Ordering::Acquire)
    }

    /// A control-side gate that outlives a reader lease: while a recording
    /// worker owns this reader, pause commands travel through the shared
    /// control block instead of the reader itself.
    pub fn pause_handle(&self) -> RecorderPause {
        RecorderPause {
            control: Arc::clone(&self.control),
        }
    }

    /// Discard everything queued so a file starts with audio observed after
    /// this call: stop acceptance, wait out any in-flight observer block, then
    /// drain. A later `start()` resumes capture, so nothing between the drain
    /// and the resume can reach a destination.
    pub fn skip_queued(&mut self) {
        self.stop();
        while !self.is_quiescent() {
            std::thread::park_timeout(Duration::from_millis(2));
        }
        self.clear();
    }

    /// Drain queued frames on the consumer side. Only complete frames are in
    /// the queue, so this commits whole blocks and keeps `read_frames` honest.
    fn clear(&mut self) {
        let sample_count = self.rx.slots();
        if sample_count == 0 {
            return;
        }
        let chunk = self
            .rx
            .read_chunk(sample_count)
            .expect("reserved consumer data");
        chunk.commit_all();
        add_frames(&self.control.read_frames, sample_count / self.channels);
    }

    /// Nonblocking query: capture is disabled and no observer can still publish
    /// a block accepted before stop. Disconnection also satisfies this condition.
    /// Queued frames may remain: after this returns true, drain them before
    /// finalizing a recording. Calling `start()` invalidates quiescence.
    pub fn is_quiescent(&self) -> bool {
        if self.rx.is_abandoned() {
            return true;
        }
        // In the SeqCst order, an observer that saw Recording before stop must
        // have set active before both our Stopped read and active read. False
        // therefore observes its exit (and publishes its queue/counter writes).
        // An observer entering later sees Stopped and cannot publish. Acquire/
        // Release alone on two separate atomics would allow both sides to miss
        // each other's store. No start may intervene before finalization.
        self.control.state.load(Ordering::SeqCst) != RecorderState::Recording as u8
            && !self.control.active.load(Ordering::SeqCst)
    }

    pub fn is_recording(&self) -> bool {
        self.state() == RecorderState::Recording
    }

    pub fn state(&self) -> RecorderState {
        if self.rx.is_abandoned() {
            return RecorderState::Disconnected;
        }
        state_from_u8(self.control.state.load(Ordering::Acquire))
    }

    /// Independently sampled atomic counters, not a transactional snapshot.
    /// Stopped frames are ignored; overflow drops count whole source blocks.
    pub fn stats(&self) -> RecorderStats {
        RecorderStats {
            accepted_frames: self.control.accepted_frames.load(Ordering::Relaxed),
            read_frames: self.control.read_frames.load(Ordering::Relaxed),
            dropped_frames: self.control.dropped_frames.load(Ordering::Relaxed),
            state: self.state(),
        }
    }

    pub fn queued_frames(&self) -> usize {
        self.rx.slots() / self.channels
    }

    /// Return the number of complete frames read, bounded by the initial queue
    /// snapshot and destination length. The unused suffix stays untouched.
    pub fn read_interleaved(&mut self, samples: &mut [f32]) -> usize {
        let requested = (samples.len() / self.channels).min(self.queued_frames());
        if requested == 0 {
            return 0;
        }
        let sample_count = requested * self.channels;
        let chunk = self
            .rx
            .read_chunk(sample_count)
            .expect("reserved consumer data");
        let (first, second) = chunk.as_slices();
        samples[..first.len()].copy_from_slice(first);
        samples[first.len()..sample_count].copy_from_slice(second);
        chunk.commit_all();
        add_frames(&self.control.read_frames, requested);
        requested
    }
}

impl<S: ProcessingSample> RtObserver<S> for Recorder {
    fn validate_inputs(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        if io.mode() == IoMode::ReadOnly
            && io.input_count() == 1
            && io.input_channels(0) == Some(self.channels)
        {
            Ok(())
        } else {
            Err(ProcessorError::InvalidIo)
        }
    }

    fn observe(
        &mut self,
        _ctx: &ProcessContext,
        inputs: ReadPorts<'_, S>,
        _signals: crate::signal::ProcessSignals<'_>,
    ) {
        self.control.active.store(true, Ordering::SeqCst);
        let _activity = ObserverActivity(&self.control.active);
        if self.control.state.load(Ordering::SeqCst) != RecorderState::Recording as u8 {
            return;
        }
        if self.control.paused.load(Ordering::SeqCst) {
            // Paused frames are neither recorded nor overflow loss; skipping
            // them here keeps the pause gap out of the file and the counters.
            return;
        }
        let input = inputs.get(0).expect("validated recorder input");
        debug_assert_eq!(input.channel_count(), self.channels);
        let frames = input.frames();
        if frames == 0 {
            return;
        }
        if self.tx.is_abandoned() || frames > self.tx.slots() / self.channels {
            add_frames(&self.control.dropped_frames, frames);
            return;
        }
        // Only this endpoint consumes capacity, so the reader can only make a
        // successful reservation larger. One commit publishes the whole block.
        let sample_count = frames * self.channels;
        let channels = self.channels;
        let chunk = self
            .tx
            .write_chunk_uninit(sample_count)
            .expect("reserved producer capacity");
        let written = chunk.fill_from_iter((0..frames).flat_map(|frame| {
            (0..channels).map(move |channel| input.channel(channel)[frame].to_f64() as f32)
        }));
        debug_assert_eq!(written, sample_count);
        add_frames(&self.control.accepted_frames, frames);
    }
}

fn add_frames(counter: &AtomicU64, frames: usize) {
    // Single-writer counters need no compare-exchange retry loop on the audio
    // thread. Saturation preserves monotonic statistics during long sessions.
    let current = counter.load(Ordering::Relaxed);
    counter.store(current.saturating_add(frames as u64), Ordering::Relaxed);
}

fn state_from_u8(value: u8) -> RecorderState {
    match value {
        0 => RecorderState::Recording,
        1 => RecorderState::Stopped,
        _ => RecorderState::Disconnected,
    }
}
