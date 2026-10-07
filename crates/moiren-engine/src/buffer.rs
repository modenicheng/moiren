use std::ops::Index;

/// Buffer storage and view module.
/// Provides AudioBlock and AudioBlockMut for accessing audio data,
/// hiding the underlying storage and memory layout.
use super::sample::ProcessingSample;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferSlotId(u32);

impl BufferSlotId {
    pub(crate) fn new(index: u32) -> Self {
        Self(index)
    }

    #[inline]
    pub(crate) fn idx(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BufferSlotLayout {
    pub channels: usize,
    pub capacity_frames: usize,
}

/// This struct is not for abstract API, it is only for store the data.
/// data field holds the actrual memory.
struct SlotStorage<S: ProcessingSample> {
    /// planar channel data
    data: Box<[S]>,
    /// numbers of channels in this slot
    channels: usize,
    /// physical size of a channel, may have padding samples/frames
    channel_stride: usize,
    /// max valid frames. **NOT** the physical size of the buffer
    capacity: usize,
}

pub struct BufferArena<S: ProcessingSample> {
    slots: Box<[SlotStorage<S>]>,
}

pub struct AudioBlock<'a, S: ProcessingSample> {
    /// a view of SlotStorage.data
    data: &'a [S],
    pub channels: usize,
    channel_stride: usize,
    /// indicates how many frames this block will process
    pub frames: usize, // this is not related to the capacity field of SlotStorage. Becareful with the difference.
}

pub struct AudioBlockMut<'a, S: ProcessingSample> {
    data: &'a mut [S],
    pub channels: usize,
    channel_stride: usize,
    pub frames: usize,
}

impl<S: ProcessingSample> SlotStorage<S> {
    fn new(channels: usize, capacity: usize) -> Self {
        assert!(channels > 0);
        assert!(capacity > 0);

        let channel_stride = capacity; // 需要做对齐

        let total_samples = channels
            .checked_mul(channel_stride)
            .expect("buffer slot size overflow");

        let data = vec![S::ZERO; total_samples].into_boxed_slice();

        Self {
            data,
            channels,
            channel_stride,
            capacity,
        }
    }
}

impl<S: ProcessingSample> AudioBlock<'_, S> {
    pub fn channel(&self, channel: usize) -> &[S] {
        assert!(channel < self.channels);
        let start = channel * self.channel_stride;
        &self.data[start..start + self.frames]
    }
    pub fn channels(&self) -> impl ExactSizeIterator<Item = &[S]> {
        let frames = self.frames;

        self.data
            .chunks_exact(self.channel_stride)
            .take(self.channels)
            .map(move |channel| &channel[..frames])
    }
}

impl<S: ProcessingSample> AudioBlockMut<'_, S> {
    pub fn channel(&self, channel: usize) -> &[S] {
        assert!(channel < self.channels);

        let start = channel * self.channel_stride;
        &self.data[start..start + self.frames]
    }

    pub fn channel_mut(&mut self, channel: usize) -> &mut [S] {
        assert!(channel < self.channels);
        let start = channel * self.channel_stride;

        &mut self.data[start..start + self.frames]
    }

    pub fn channels(&self) -> impl ExactSizeIterator<Item = &[S]> {
        let frames = self.frames;

        self.data
            .chunks_exact(self.channel_stride)
            .take(self.channels)
            .map(move |channel| &channel[..frames])
    }

    pub fn channels_mut(&mut self) -> impl ExactSizeIterator<Item = &mut [S]> {
        let frames = self.frames;

        self.data
            .chunks_exact_mut(self.channel_stride)
            .take(self.channels)
            .map(move |ch| &mut ch[..frames])
    }
}

impl<S: ProcessingSample> BufferArena<S> {
    pub fn new(layouts: &[BufferSlotLayout]) -> Self {
        let slots = layouts
            .iter()
            .map(|layout| SlotStorage::new(layout.channels, layout.capacity_frames))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self { slots }
    }

    pub fn block(&self, slot_id: BufferSlotId, frames: usize) -> AudioBlock<'_, S> {
        assert!(slot_id.idx() < self.slots.len());
        let slot = &self.slots[slot_id.idx()];
        assert!(frames > 0);
        assert!(frames <= slot.capacity);
        AudioBlock {
            data: slot.data.as_ref(),
            channels: slot.channels,
            channel_stride: slot.channel_stride,
            frames,
        }
    }

    pub fn block_mut(&mut self, slot_id: BufferSlotId, frames: usize) -> AudioBlockMut<'_, S> {
        assert!(slot_id.idx() < self.slots.len());
        let slot = &mut self.slots[slot_id.idx()];
        assert!(frames > 0);
        assert!(frames <= slot.capacity);
        AudioBlockMut {
            data: slot.data.as_mut(),
            channels: slot.channels,
            channel_stride: slot.channel_stride,
            frames,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_new_slot() {
        let slot = SlotStorage::<f32>::new(2, 128);
        assert_eq!(slot.channels, 2);
        assert_eq!(slot.capacity, 128);
        assert_eq!(slot.channel_stride, 128);
        assert_eq!(slot.data.len(), 256);
    }

    #[test]
    fn test_new_buffer_arena() {
        let layouts = &[
            BufferSlotLayout {
                channels: 2,
                capacity_frames: 128,
            },
            BufferSlotLayout {
                channels: 4,
                capacity_frames: 256,
            },
        ];
        let arena = BufferArena::<f32>::new(layouts);

        assert_eq!(arena.slots.len(), 2);

        let slot1 = &arena.slots[0];
        assert_eq!(slot1.capacity, 128);
        assert_eq!(slot1.channels, 2);

        let slot2 = &arena.slots[1];
        assert_eq!(slot2.capacity, 256);
        assert_eq!(slot2.channels, 4);
    }

    // ---------- helpers ----------

    /// Build a read-only block view over `data` with `channels` channels,
    /// `stride` samples per channel and `frames` valid frames.
    fn make_block(
        data: &[f32],
        channels: usize,
        stride: usize,
        frames: usize,
    ) -> AudioBlock<'_, f32> {
        AudioBlock {
            data,
            channels,
            channel_stride: stride,
            frames,
        }
    }

    /// Build a mutable block view over `data` with `channels` channels,
    /// `stride` samples per channel and `frames` valid frames.
    fn make_block_mut(
        data: &mut [f32],
        channels: usize,
        stride: usize,
        frames: usize,
    ) -> AudioBlockMut<'_, f32> {
        AudioBlockMut {
            data,
            channels,
            channel_stride: stride,
            frames,
        }
    }

    // ---------- BufferSlotId ----------

    #[test]
    fn test_buffer_slot_id_roundtrip() {
        for i in [0u32, 1, 42, u32::MAX] {
            assert_eq!(BufferSlotId::new(i).idx(), i as usize);
        }
    }

    #[test]
    fn test_buffer_slot_id_is_copy() {
        let id = BufferSlotId::new(7);
        let copied = id;
        // both copies stay usable
        assert_eq!(id.idx(), 7);
        assert_eq!(copied.idx(), 7);
    }

    // ---------- SlotStorage ----------

    #[test]
    fn test_new_slot_zero_filled() {
        let slot = SlotStorage::<f32>::new(3, 64);
        assert_eq!(slot.data.len(), 3 * 64);
        assert!(slot.data.iter().all(|&s| s == f32::ZERO));
    }

    #[test]
    #[should_panic(expected = "channels > 0")]
    fn test_new_slot_panics_on_zero_channels() {
        let _ = SlotStorage::<f32>::new(0, 128);
    }

    #[test]
    #[should_panic(expected = "capacity > 0")]
    fn test_new_slot_panics_on_zero_capacity() {
        let _ = SlotStorage::<f32>::new(2, 0);
    }

    #[test]
    #[should_panic(expected = "buffer slot size overflow")]
    fn test_new_slot_panics_on_size_overflow() {
        let _ = SlotStorage::<f32>::new(usize::MAX, usize::MAX);
    }

    // ---------- BufferArena ----------

    #[test]
    fn test_buffer_arena_zero_filled() {
        let layouts = &[
            BufferSlotLayout {
                channels: 2,
                capacity_frames: 32,
            },
            BufferSlotLayout {
                channels: 1,
                capacity_frames: 512,
            },
        ];
        let arena = BufferArena::<f64>::new(layouts);
        assert!(
            arena
                .slots
                .iter()
                .all(|slot| slot.data.iter().all(|&s| s == f64::ZERO))
        );
    }

    #[test]
    fn test_buffer_arena_empty_layouts() {
        let arena = BufferArena::<f32>::new(&[]);
        assert_eq!(arena.slots.len(), 0);
    }

    // ---------- AudioBlock ----------

    #[test]
    fn test_audio_block_channel_returns_valid_frames() {
        // 2 channels, stride 128, frames 64, each sample encodes its index
        let data: Vec<f32> = (0..2 * 128).map(|i| i as f32).collect();
        let block = make_block(&data, 2, 128, 64);

        assert_eq!(
            block.channel(0),
            (0..64).map(|i| i as f32).collect::<Vec<_>>()
        );
        assert_eq!(
            block.channel(1),
            (128..192).map(|i| i as f32).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_audio_block_channel_never_exposes_padding() {
        // frames < channel_stride: samples beyond `frames` must not leak out
        let data: Vec<f32> = (0..2 * 128).map(|i| i as f32).collect();
        let block = make_block(&data, 2, 128, 8);

        assert_eq!(block.channel(0).len(), 8);
        assert_eq!(block.channel(1).len(), 8);
        // sample right after the valid region of channel 0 is untouched
        assert_eq!(block.channel(0)[7], 7.0);
        assert_ne!(block.channel(0)[7], data[8]);
    }

    #[test]
    fn test_audio_block_channels_iterator() {
        let data: Vec<f32> = (0..2 * 128).map(|i| i as f32).collect();
        let block = make_block(&data, 2, 128, 64);

        let collected: Vec<&[f32]> = block.channels().collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], &data[..64]);
        assert_eq!(collected[1], &data[128..192]);
    }

    #[test]
    fn test_audio_block_channels_is_exact_size_iterator() {
        let data = vec![0.0f32; 4 * 16];
        let block = make_block(&data, 4, 16, 16);

        let mut iter = block.channels();
        assert_eq!(iter.len(), 4);
        assert_eq!(iter.next().unwrap().len(), 16);
        assert_eq!(iter.len(), 3);
        assert_eq!(iter.count(), 3);
    }

    #[test]
    fn test_audio_block_channels_never_exposes_padding() {
        let data: Vec<f32> = (0..3 * 32).map(|i| i as f32).collect();
        let block = make_block(&data, 3, 32, 4);

        for (ch, channel) in block.channels().enumerate() {
            assert_eq!(channel.len(), 4);
            let start = ch * 32;
            assert_eq!(channel, &data[start..start + 4]);
        }
    }

    #[test]
    #[should_panic(expected = "channel < self.channels")]
    fn test_audio_block_channel_out_of_bounds_panics() {
        let data = vec![0.0f32; 2 * 128];
        let block = make_block(&data, 2, 128, 64);
        let _ = block.channel(2);
    }

    // ---------- AudioBlockMut ----------

    #[test]
    fn test_audio_block_mut_channel_reads_valid_frames() {
        let mut data: Vec<f32> = (0..2 * 128).map(|i| i as f32).collect();
        let block = make_block_mut(&mut data, 2, 128, 64);

        assert_eq!(block.channel(0).len(), 64);
        assert_eq!(block.channel(0)[63], 63.0);
        assert_eq!(block.channel(1)[0], 128.0);
    }

    #[test]
    fn test_audio_block_mut_channel_mut_write() {
        let mut data = vec![0.0f32; 2 * 128];
        {
            let mut block = make_block_mut(&mut data, 2, 128, 64);
            block.channel_mut(0).fill(1.5);
            block.channel_mut(1).iter_mut().for_each(|s| *s = -2.0);
        }

        // only the first `frames` samples of each channel are written
        assert!(data[..64].iter().all(|&s| s == 1.5));
        assert!(data[64..128].iter().all(|&s| s == 0.0));
        assert!(data[128..192].iter().all(|&s| s == -2.0));
        assert!(data[192..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_audio_block_mut_channels_mut_write_isolated() {
        let mut data = vec![0.0f32; 3 * 32];
        {
            let mut block = make_block_mut(&mut data, 3, 32, 32);
            for (ch, channel) in block.channels_mut().enumerate() {
                channel.fill(ch as f32);
            }
        }

        for ch in 0..3 {
            let start = ch * 32;
            assert!(data[start..start + 32].iter().all(|&s| s == ch as f32));
        }
    }

    #[test]
    fn test_audio_block_mut_channels_reads_writes() {
        let mut data = vec![0.0f32; 2 * 16];
        {
            let mut block = make_block_mut(&mut data, 2, 16, 16);
            block.channel_mut(0).fill(7.0);

            let collected: Vec<&[f32]> = block.channels().collect();
            assert_eq!(collected[0], [7.0f32; 16]);
            assert_eq!(collected[1], [0.0f32; 16]);
        }
    }

    #[test]
    fn test_audio_block_mut_full_frames_equals_stride() {
        // frames == channel_stride: each channel view covers its whole stride
        let mut data = vec![0.0f32; 2 * 8];
        {
            let mut block = make_block_mut(&mut data, 2, 8, 8);
            for (ch, channel) in block.channels_mut().enumerate() {
                for (frame, sample) in channel.iter_mut().enumerate() {
                    *sample = (ch * 8 + frame) as f32;
                }
            }
        }
        assert_eq!(data, (0..16).map(|i| i as f32).collect::<Vec<_>>());
    }

    #[test]
    #[should_panic(expected = "channel < self.channels")]
    fn test_audio_block_mut_channel_out_of_bounds_panics() {
        let mut data = vec![0.0f32; 2 * 128];
        let block = make_block_mut(&mut data, 2, 128, 64);
        let _ = block.channel(2);
    }

    #[test]
    #[should_panic(expected = "channel < self.channels")]
    fn test_audio_block_mut_channel_mut_out_of_bounds_panics() {
        let mut data = vec![0.0f32; 2 * 128];
        let mut block = make_block_mut(&mut data, 2, 128, 64);
        let _ = block.channel_mut(2);
    }

    #[test]
    fn test_audio_block_mut_channels_mut_is_exact_size_iterator() {
        let mut data = vec![0.0f32; 4 * 16];
        let mut block = make_block_mut(&mut data, 4, 16, 16);

        let mut iter = block.channels_mut();
        assert_eq!(iter.len(), 4);
        assert_eq!(iter.next().unwrap().len(), 16);
        assert_eq!(iter.len(), 3);
        assert_eq!(iter.count(), 3);
    }
}
