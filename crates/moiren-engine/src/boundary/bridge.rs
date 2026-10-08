//! Fixed-capacity audio copying at Processing SR. No PCM conversion or clock SRC.
use rtrb::{Consumer, Producer, RingBuffer};
use thiserror::Error;

use super::{BoundaryReport, RtAudioSink, RtAudioSource};
use crate::{
    buffer::{AudioBlock, AudioBlockMut},
    processor::ProcessContext,
    sample::ProcessingSample,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BridgeError {
    #[error("audio bridge channels or frame capacity is zero")]
    InvalidLayout,
    #[error("audio bridge sample storage exceeds addressable memory")]
    SizeOverflow,
    #[error("audio bridge sample storage exceeds its byte budget")]
    BudgetExceeded,
    #[error("interleaved sample count is not a whole number of frames")]
    InvalidInterleaved,
}

/// Single producer. Move to the capture worker or wrap in an OutputNode.
pub struct AudioWriter<S: ProcessingSample> {
    tx: Producer<S>,
    channels: usize,
    capacity_frames: usize,
}
/// Single consumer. Wrap in an InputNode or move to the render worker.
pub struct AudioReader<S: ProcessingSample> {
    rx: Consumer<S>,
    channels: usize,
    capacity_frames: usize,
}

/// Non-RT only. Budget covers sample storage, not ring metadata. The endpoints
/// must be stopped and dropped outside streaming; reprepare on format/epoch reset.
pub fn audio_bridge<S: ProcessingSample>(
    channels: usize,
    capacity_frames: usize,
    byte_budget: usize,
) -> Result<(AudioWriter<S>, AudioReader<S>), BridgeError> {
    if channels == 0 || capacity_frames == 0 || std::mem::size_of::<S>() == 0 {
        return Err(BridgeError::InvalidLayout);
    }
    let samples = channels
        .checked_mul(capacity_frames)
        .ok_or(BridgeError::SizeOverflow)?;
    let bytes = samples
        .checked_mul(std::mem::size_of::<S>())
        .ok_or(BridgeError::SizeOverflow)?;
    if bytes > isize::MAX as usize {
        return Err(BridgeError::SizeOverflow);
    }
    if bytes > byte_budget {
        return Err(BridgeError::BudgetExceeded);
    }
    let (tx, rx) = RingBuffer::new(samples);
    Ok((
        AudioWriter {
            tx,
            channels,
            capacity_frames,
        },
        AudioReader {
            rx,
            channels,
            capacity_frames,
        },
    ))
}

fn transfer_report(frames: usize, requested: usize, abandoned: bool) -> BoundaryReport {
    let short = frames < requested;
    BoundaryReport {
        transferred_frames: frames,
        discontinuity: short || (requested > 0 && abandoned),
        xruns: u64::from(short),
    }
}

impl<S: ProcessingSample> AudioWriter<S> {
    pub fn channel_count(&self) -> usize {
        self.channels
    }
    pub fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Accept the prefix that fits at call start; drop the new tail on overflow.
    /// Empty requests succeed. Malformed slices do not change the ring.
    pub fn write_interleaved(&mut self, samples: &[S]) -> Result<BoundaryReport, BridgeError> {
        if !samples.len().is_multiple_of(self.channels) {
            return Err(BridgeError::InvalidInterleaved);
        }
        Ok(self.push(samples.len() / self.channels, samples.iter().copied()))
    }

    fn push(&mut self, requested: usize, samples: impl Iterator<Item = S>) -> BoundaryReport {
        let abandoned = self.tx.is_abandoned();
        let frames = if abandoned {
            0
        } else {
            requested.min(self.tx.slots() / self.channels)
        };
        if frames > 0 {
            // A single chunk publishes only complete frames. The other endpoint
            // can only increase available capacity after this initial snapshot.
            let count = frames * self.channels;
            let chunk = self
                .tx
                .write_chunk_uninit(count)
                .expect("reserved producer capacity");
            let written = chunk.fill_from_iter(samples.take(count));
            debug_assert_eq!(written, count);
        }
        transfer_report(frames, requested, abandoned)
    }
}

impl<S: ProcessingSample> AudioReader<S> {
    pub fn channel_count(&self) -> usize {
        self.channels
    }
    pub fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Drain the initial available prefix, then initialize every missing sample
    /// to silence. Never wait for the producer, including after it exits.
    pub fn read_interleaved(&mut self, samples: &mut [S]) -> Result<BoundaryReport, BridgeError> {
        if !samples.len().is_multiple_of(self.channels) {
            return Err(BridgeError::InvalidInterleaved);
        }
        let channels = self.channels;
        Ok(self.read_to(samples.len() / channels, |frame, ch, value| {
            samples[frame * channels + ch] = value
        }))
    }

    fn read_to(
        &mut self,
        requested: usize,
        mut set: impl FnMut(usize, usize, S),
    ) -> BoundaryReport {
        let abandoned = self.rx.is_abandoned();
        let frames = requested.min(self.rx.slots() / self.channels);
        if frames > 0 {
            let chunk = self
                .rx
                .read_chunk(frames * self.channels)
                .expect("reserved consumer data");
            let (first, second) = chunk.as_slices();
            for (index, sample) in first.iter().chain(second).enumerate() {
                set(index / self.channels, index % self.channels, *sample);
            }
            chunk.commit_all();
        }
        for frame in frames..requested {
            for ch in 0..self.channels {
                set(frame, ch, S::ZERO);
            }
        }
        transfer_report(frames, requested, abandoned)
    }
}

impl<S: ProcessingSample> RtAudioSink<S> for AudioWriter<S> {
    fn channel_count(&self) -> usize {
        self.channels
    }
    fn write(&mut self, _: &ProcessContext, input: AudioBlock<'_, S>) -> BoundaryReport {
        if input.channel_count() != self.channels {
            return transfer_report(0, input.frames(), false);
        }
        let channels = self.channels;
        self.push(
            input.frames(),
            (0..input.frames())
                .flat_map(|frame| (0..channels).map(move |ch| input.channel(ch)[frame])),
        )
    }
}
impl<S: ProcessingSample> RtAudioSource<S> for AudioReader<S> {
    fn channel_count(&self) -> usize {
        self.channels
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        if output.channel_count() != self.channels {
            output.clear();
            return transfer_report(0, output.frames(), false);
        }
        self.read_to(output.frames(), |frame, ch, value| {
            output.channel_mut(ch)[frame] = value
        })
    }
}
