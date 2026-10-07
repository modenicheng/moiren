use crate::{
    buffer::{AudioBlock, AudioBlockMut},
    sample::ProcessingSample,
};

pub type FrameTime = u64;

#[derive(Debug, Clone, Copy)]
pub struct ProcessContext {
    pub timeline_start: FrameTime,
    pub frames: u32,
    pub processing_sr: f64,
}

pub struct ProcessIo<'a, S: ProcessingSample> {
    input: AudioBlock<'a, S>,
    output: AudioBlockMut<'a, S>,
}

impl<'a, S: ProcessingSample> ProcessIo<'a, S> {
    pub(crate) fn separate(
        input: AudioBlock<'a, S>,
        output: AudioBlockMut<'a, S>,
    ) -> Self {
        Self { input, output }
    }

    pub fn split(
        &mut self,
    ) -> (&AudioBlock<'a, S>, &mut AudioBlockMut<'a, S>) {
        (&self.input, &mut self.output)
    }
}

pub trait RtProcessor<S: ProcessingSample> {
    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
    );
}

#[derive(Debug, Clone, Copy)]
pub struct Gain<S: ProcessingSample> {
    gain: S,
}

impl<S: ProcessingSample> Gain<S> {
    pub const fn new(gain: S) -> Self {
        Self { gain }
    }

    pub const fn gain(&self) -> S {
        self.gain
    }

    pub fn set_gain(&mut self, gain: S) {
        self.gain = gain;
    }
}

impl<S: ProcessingSample> RtProcessor<S> for Gain<S> {
    fn process(
        &mut self,
        ctx: &ProcessContext,
        mut io: ProcessIo<'_, S>,
    ) {
        let (input, output) = io.split();
        let frames = ctx.frames as usize;

        assert_eq!(input.frame_count(), frames);
        assert_eq!(output.frame_count(), frames);
        assert_eq!(input.channel_count(), output.channel_count());

        for (src, dst) in input.channels().zip(output.channels_mut()) {
            for (src, dst) in src.iter().copied().zip(dst.iter_mut()) {
                *dst = src;
                *dst *= self.gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{BufferArena, BufferSlotId, BufferSlotLayout};

    #[test]
    fn separate_gain_processes_variable_blocks_without_touching_input() {
        for frames in [1usize, 5, 16] {
            let layouts = [
                BufferSlotLayout {
                    channels: 2,
                    capacity_frames: 16,
                },
                BufferSlotLayout {
                    channels: 2,
                    capacity_frames: 16,
                },
            ];
            let mut arena = BufferArena::<f32>::new(&layouts);
            let output_id = BufferSlotId::new(0);
            let input_id = BufferSlotId::new(1);

            {
                let mut input = arena.block_mut(input_id, 16);
                for (channel_index, channel) in input.channels_mut().enumerate() {
                    for (frame_index, sample) in channel.iter_mut().enumerate() {
                        *sample = (channel_index * 100 + frame_index + 1) as f32;
                    }
                }
            }

            let input_before = arena
                .block(input_id, 16)
                .channels()
                .map(<[f32]>::to_vec)
                .collect::<Vec<_>>();

            let ctx = ProcessContext {
                timeline_start: 0,
                frames: frames as u32,
                processing_sr: 48_000.0,
            };
            let (input, output) =
                arena.resolve_separate_pair(input_id, output_id, frames);
            let io = ProcessIo::separate(input, output);
            let mut gain = Gain::new(0.5f32);
            gain.process(&ctx, io);

            let input_after = arena
                .block(input_id, 16)
                .channels()
                .map(<[f32]>::to_vec)
                .collect::<Vec<_>>();
            assert_eq!(input_after, input_before);

            let output = arena.block(output_id, 16);
            for channel_index in 0..2 {
                let channel = output.channel(channel_index);
                for frame_index in 0..frames {
                    let source =
                        (channel_index * 100 + frame_index + 1) as f32;
                    assert_eq!(channel[frame_index], source * 0.5);
                }
                assert!(channel[frames..].iter().all(|&sample| sample == 0.0));
            }
        }
    }

    #[test]
    fn gain_is_generic_over_f64_processing_samples() {
        let layouts = [
            BufferSlotLayout {
                channels: 1,
                capacity_frames: 4,
            },
            BufferSlotLayout {
                channels: 1,
                capacity_frames: 4,
            },
        ];
        let mut arena = BufferArena::<f64>::new(&layouts);
        let input_id = BufferSlotId::new(0);
        let output_id = BufferSlotId::new(1);

        arena.block_mut(input_id, 4).channel_mut(0).copy_from_slice(&[
            0.25, -0.5, 1.0, 2.0,
        ]);

        let ctx = ProcessContext {
            timeline_start: 32,
            frames: 4,
            processing_sr: 96_000.0,
        };
        let (input, output) =
            arena.resolve_separate_pair(input_id, output_id, 4);
        let io = ProcessIo::separate(input, output);
        Gain::new(2.0f64).process(&ctx, io);

        assert_eq!(
            arena.block(output_id, 4).channel(0),
            &[0.5, -1.0, 2.0, 4.0]
        );
    }
}
