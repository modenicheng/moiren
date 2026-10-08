//! Scoped port views keep raw pointer conversion behind the arena's validation.
//! Their lifetimes follow the callback borrow, so a processor cannot retain a
//! sample reference into a later render or create overlapping mutable views.
use std::marker::PhantomData;

use super::{Binding, SlotMeta};
use crate::sample::ProcessingSample;

#[derive(Clone, Copy)]
pub(super) struct RawWindow<S> {
    pub(super) base: *mut S,
    pub(super) start: usize,
    pub(super) frames: usize,
}

impl<S: ProcessingSample> RawWindow<S> {
    // Caller proves: same live arena, valid initialized range, no mutable alias,
    // and 'a bounded by the access scope / reborrow of the owning port set.
    unsafe fn read<'a>(self, meta: SlotMeta) -> AudioBlock<'a, S> {
        // SAFETY: the caller supplies the range and exclusivity proof above;
        // prepare validates checked allocation sizes and the active window.
        let data = unsafe { std::slice::from_raw_parts(self.base.add(meta.offset), meta.len) };
        AudioBlock {
            data,
            channels: meta.channels,
            stride: meta.stride,
            start: self.start,
            frames: self.frames,
        }
    }
    // As above, but the entire slot must be exclusively borrowed for 'a.
    unsafe fn write<'a>(self, meta: SlotMeta) -> AudioBlockMut<'a, S> {
        // SAFETY: no whole-arena slice or overlapping slot view coexists.
        let data = unsafe { std::slice::from_raw_parts_mut(self.base.add(meta.offset), meta.len) };
        AudioBlockMut {
            data,
            channels: meta.channels,
            stride: meta.stride,
            start: self.start,
            frames: self.frames,
        }
    }
}

pub enum ProcessIo<'a, S: ProcessingSample> {
    ReadOnly {
        inputs: ReadPorts<'a, S>,
    },
    Separate {
        inputs: ReadPorts<'a, S>,
        outputs: WritePorts<'a, S>,
    },
    InPlace {
        inputs: ReadPorts<'a, S>,
        outputs: WritePorts<'a, S>,
        pairs: InPlacePorts<'a, S>,
    },
}

impl<S: ProcessingSample> ProcessIo<'_, S> {
    /// Separate outputs start each invocation defined and silent. In-place
    /// storage preserves its input. Write-only is a semantic promise, not &mut
    /// access to uninitialized samples.
    pub(crate) fn clear_outputs(&mut self) {
        match self {
            Self::Separate { outputs, .. } | Self::InPlace { outputs, .. } => {
                for (_, mut block) in outputs.iter_mut() {
                    block.clear();
                }
            }
            Self::ReadOnly { .. } => {}
        }
    }
}

pub struct ReadPorts<'a, S: ProcessingSample> {
    pub(super) raw: RawWindow<S>,
    pub(super) bindings: &'a [Binding],
    pub(super) borrow: PhantomData<&'a [S]>,
}
pub struct WritePorts<'a, S: ProcessingSample> {
    pub(super) raw: RawWindow<S>,
    pub(super) bindings: &'a [Binding],
    pub(super) borrow: PhantomData<&'a mut [S]>,
}
pub struct InPlacePorts<'a, S: ProcessingSample> {
    pub(super) raw: RawWindow<S>,
    pub(super) bindings: &'a [Binding],
    pub(super) borrow: PhantomData<&'a mut [S]>,
}

impl<S: ProcessingSample> ReadPorts<'_, S> {
    pub fn get(&self, port: u16) -> Option<AudioBlock<'_, S>> {
        let binding = self.bindings.iter().find(|b| b.input == Some(port))?;
        // SAFETY: reads may alias each other, never any writable slot in this scope.
        Some(unsafe { self.raw.read(binding.meta) })
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (u16, AudioBlock<'_, S>)> {
        self.bindings.iter().map(|b| {
            // SAFETY: same validated read ranges; lifetime bound to &self.
            (b.input.unwrap(), unsafe { self.raw.read(b.meta) })
        })
    }
}

impl<S: ProcessingSample> WritePorts<'_, S> {
    pub fn get_mut(&mut self, port: u16) -> Option<AudioBlockMut<'_, S>> {
        let binding = self.bindings.iter().find(|b| b.output == Some(port))?;
        // SAFETY: exclusive reborrow prevents a second get/iterator on this set.
        Some(unsafe { self.raw.write(binding.meta) })
    }
    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = (u16, AudioBlockMut<'_, S>)> {
        let raw = self.raw;
        self.bindings.iter().map(move |b| {
            // SAFETY: each validated disjoint slot is yielded once; the iterator
            // borrows this set exclusively until all yielded views expire.
            (b.output.unwrap(), unsafe { raw.write(b.meta) })
        })
    }
}

impl<S: ProcessingSample> InPlacePorts<'_, S> {
    pub fn get_mut(&mut self, input: u16, output: u16) -> Option<AudioBlockMut<'_, S>> {
        let binding = self
            .bindings
            .iter()
            .find(|b| b.input == Some(input) && b.output == Some(output))?;
        // SAFETY: pair has one exclusive view, never simultaneous input/output slices.
        Some(unsafe { self.raw.write(binding.meta) })
    }
    pub fn iter_mut(
        &mut self,
    ) -> impl ExactSizeIterator<Item = ((u16, u16), AudioBlockMut<'_, S>)> {
        let raw = self.raw;
        self.bindings.iter().map(move |b| {
            // SAFETY: validated unique ranges, yielded once under exclusive borrow.
            ((b.input.unwrap(), b.output.unwrap()), unsafe {
                raw.write(b.meta)
            })
        })
    }
}

#[derive(Clone, Copy)]
pub struct AudioBlock<'a, S: ProcessingSample> {
    data: &'a [S],
    channels: usize,
    stride: usize,
    start: usize,
    frames: usize,
}
pub struct AudioBlockMut<'a, S: ProcessingSample> {
    data: &'a mut [S],
    channels: usize,
    stride: usize,
    start: usize,
    frames: usize,
}
impl<S: ProcessingSample> AudioBlock<'_, S> {
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn channel_count(&self) -> usize {
        self.channels
    }
    pub fn channel(&self, channel: usize) -> &[S] {
        assert!(channel < self.channels);
        let start = channel * self.stride + self.start;
        &self.data[start..start + self.frames]
    }
    pub fn channels(&self) -> impl ExactSizeIterator<Item = &[S]> {
        self.data
            .chunks_exact(self.stride)
            .map(|ch| &ch[self.start..self.start + self.frames])
    }
}
impl<S: ProcessingSample> AudioBlockMut<'_, S> {
    pub fn frames(&self) -> usize {
        self.frames
    }
    pub fn channel_count(&self) -> usize {
        self.channels
    }
    pub fn channel(&self, channel: usize) -> &[S] {
        assert!(channel < self.channels);
        let start = channel * self.stride + self.start;
        &self.data[start..start + self.frames]
    }
    pub fn channel_mut(&mut self, channel: usize) -> &mut [S] {
        assert!(channel < self.channels);
        let start = channel * self.stride + self.start;
        &mut self.data[start..start + self.frames]
    }
    pub fn channels_mut(&mut self) -> impl ExactSizeIterator<Item = &mut [S]> {
        let start = self.start;
        let end = start + self.frames;
        self.data
            .chunks_exact_mut(self.stride)
            .map(move |ch| &mut ch[start..end])
    }
    pub fn clear(&mut self) {
        for channel in self.channels_mut() {
            channel.fill(S::ZERO);
        }
    }
}
