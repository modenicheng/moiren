//! Bounded recording transport. Construct and reclaim both endpoints on the
//! control side; the observer only converts and publishes complete audio blocks.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};

use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

use crate::{
    buffer::{IoMode, PreparedIo, ReadPorts},
    processor::{ProcessContext, ProcessorError, RtObserver},
    sample::ProcessingSample,
};

mod wav;
pub use wav::{
    RecordingError, RecordingSession, RecordingState, RecordingStatus, RecordingSummary,
    WavRecording,
};
pub(crate) use wav::{RecordingSource, spawn_recording};

#[cfg(test)]
mod tests;

const MAX_CHANNELS: usize = 256;
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
