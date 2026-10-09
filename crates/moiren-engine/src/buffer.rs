//! One owned planar slab, with IO validation before scoped views are constructed.
use std::{marker::PhantomData, mem::size_of, ops::Range, sync::Arc};

use thiserror::Error;

use crate::sample::ProcessingSample;

mod view;
use view::RawWindow;
pub use view::{AudioBlock, AudioBlockMut, InPlacePorts, ProcessIo, ReadPorts, WritePorts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferSlotId(u32);

#[derive(Debug, Clone, Copy)]
pub struct BufferSlotLayout {
    pub channels: usize,
    pub capacity_frames: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BufferError {
    #[error("slot layout has zero channels or zero capacity")]
    InvalidLayout,
    #[error("slot size in samples exceeds addressable memory")]
    SizeOverflow,
    #[error("arena allocation exceeds the configured byte budget")]
    BudgetExceeded,
    #[error("the system allocator refused a non-RT reservation")]
    AllocationFailed,
    #[error("port references a slot outside this arena")]
    InvalidSlot,
    #[error("an input or output port number is declared twice")]
    DuplicatePort,
    #[error("two writable views would alias the same slot range")]
    AliasedWrite,
    #[error("prepared IO belongs to a different arena")]
    ForeignAccess,
    #[error("window is empty or exceeds the prepared maximum")]
    InvalidFrames,
}

#[derive(Debug, Clone, Copy)]
struct SlotMeta {
    offset: usize,
    len: usize,
    channels: usize,
    stride: usize,
}

#[derive(Debug)]
struct Layout {
    slots: Box<[SlotMeta]>,
}

/// IDs are local to a prepared arena, never wire-protocol handles.
pub struct BufferArena<S: ProcessingSample> {
    data: Box<[S]>,
    layout: Arc<Layout>,
}

/// Logical input/output port numbers survive physical slot assignment.
#[derive(Debug, Clone, Copy)]
pub enum PortAccess {
    Read {
        port: u16,
        slot: BufferSlotId,
    },
    Write {
        port: u16,
        slot: BufferSlotId,
    },
    InPlace {
        input: u16,
        output: u16,
        slot: BufferSlotId,
    },
}

#[derive(Debug, Clone, Copy)]
struct Binding {
    input: Option<u16>,
    output: Option<u16>,
    meta: SlotMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    ReadOnly,
    Separate,
    InPlace,
}

/// Constructed only by `prepare_io`; retains the originating layout identity.
pub struct PreparedIo {
    layout: Arc<Layout>,
    reads: Box<[Binding]>,
    writes: Box<[Binding]>,
    pairs: Box<[Binding]>,
    max_frames: usize,
    mode: IoMode,
}

impl PreparedIo {
    pub fn mode(&self) -> IoMode {
        self.mode
    }
    pub fn max_frames(&self) -> usize {
        self.max_frames
    }
    pub fn input_count(&self) -> usize {
        self.reads.len() + self.pairs.len()
    }
    pub fn output_count(&self) -> usize {
        self.writes.len() + self.pairs.len()
    }
    pub fn input_ports(&self) -> impl Iterator<Item = (u16, usize)> {
        self.reads
            .iter()
            .chain(self.pairs.iter())
            .map(|b| (b.input.unwrap(), b.meta.channels))
    }
    pub fn input_channels(&self, port: u16) -> Option<usize> {
        self.reads
            .iter()
            .chain(self.pairs.iter())
            .find(|b| b.input == Some(port))
            .map(|b| b.meta.channels)
    }
    pub fn output_ports(&self) -> impl Iterator<Item = (u16, usize)> {
        self.writes
            .iter()
            .chain(self.pairs.iter())
            .map(|b| (b.output.unwrap(), b.meta.channels))
    }
    pub fn output_channels(&self, port: u16) -> Option<usize> {
        self.writes
            .iter()
            .chain(self.pairs.iter())
            .find(|b| b.output == Some(port))
            .map(|b| b.meta.channels)
    }
}

impl<S: ProcessingSample> BufferArena<S> {
    /// Allocation/zero-initialization happens on the control thread only.
    pub fn new(layouts: &[BufferSlotLayout], byte_budget: usize) -> Result<Self, BufferError> {
        if size_of::<S>() == 0 || layouts.len() > u32::MAX as usize {
            return Err(BufferError::InvalidLayout);
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(layouts.len())
            .map_err(|_| BufferError::AllocationFailed)?;
        let mut total = 0usize;
        for layout in layouts {
            if layout.channels == 0 || layout.capacity_frames == 0 {
                return Err(BufferError::InvalidLayout);
            }
            let len = layout
                .channels
                .checked_mul(layout.capacity_frames)
                .ok_or(BufferError::SizeOverflow)?;
            let next = total.checked_add(len).ok_or(BufferError::SizeOverflow)?;
            slots.push(SlotMeta {
                offset: total,
                len,
                channels: layout.channels,
                stride: layout.capacity_frames,
            });
            total = next;
        }
        let bytes = total
            .checked_mul(size_of::<S>())
            .ok_or(BufferError::SizeOverflow)?;
        if bytes > isize::MAX as usize {
            return Err(BufferError::SizeOverflow);
        }
        if bytes > byte_budget {
            return Err(BufferError::BudgetExceeded);
        }
        let mut data = Vec::new();
        data.try_reserve_exact(total)
            .map_err(|_| BufferError::AllocationFailed)?;
        data.resize(total, S::ZERO);
        Ok(Self {
            data: data.into_boxed_slice(),
            layout: Arc::new(Layout {
                slots: slots.into_boxed_slice(),
            }),
        })
    }

    pub fn slot(&self, index: usize) -> Option<BufferSlotId> {
        (index < self.layout.slots.len()).then_some(BufferSlotId(index as u32))
    }
    pub fn allocated_samples(&self) -> usize {
        self.data.len()
    }
    pub(crate) fn owns(&self, access: &PreparedIo) -> bool {
        Arc::ptr_eq(&self.layout, &access.layout)
    }

    pub fn prepare_io(&self, ports: &[PortAccess]) -> Result<PreparedIo, BufferError> {
        let mut all = Vec::<Binding>::with_capacity(ports.len());
        let mut reads = Vec::new();
        let mut writes = Vec::new();
        let mut pairs = Vec::new();
        let mut max_frames = usize::MAX;
        for access in ports {
            let (input, output, id) = match *access {
                PortAccess::Read { port, slot } => (Some(port), None, slot),
                PortAccess::Write { port, slot } => (None, Some(port), slot),
                PortAccess::InPlace {
                    input,
                    output,
                    slot,
                } => (Some(input), Some(output), slot),
            };
            let meta = *self
                .layout
                .slots
                .get(id.0 as usize)
                .ok_or(BufferError::InvalidSlot)?;
            for previous in &all {
                if input.is_some() && input == previous.input
                    || output.is_some() && output == previous.output
                {
                    return Err(BufferError::DuplicatePort);
                }
                // Compare actual allocation ranges, not just symbolic IDs.
                let overlap = meta.offset < previous.meta.offset + previous.meta.len
                    && previous.meta.offset < meta.offset + meta.len;
                if overlap && (output.is_some() || previous.output.is_some()) {
                    return Err(BufferError::AliasedWrite);
                }
            }
            let binding = Binding {
                input,
                output,
                meta,
            };
            all.push(binding);
            max_frames = max_frames.min(meta.stride);
            match (input, output) {
                (Some(_), Some(_)) => pairs.push(binding),
                (Some(_), None) => reads.push(binding),
                (None, Some(_)) => writes.push(binding),
                _ => unreachable!(),
            }
        }
        let mode = if !pairs.is_empty() {
            IoMode::InPlace
        } else if !writes.is_empty() {
            IoMode::Separate
        } else {
            IoMode::ReadOnly
        };
        Ok(PreparedIo {
            layout: Arc::clone(&self.layout),
            reads: reads.into_boxed_slice(),
            writes: writes.into_boxed_slice(),
            pairs: pairs.into_boxed_slice(),
            max_frames,
            mode,
        })
    }

    /// The callback's result cannot contain a view from this invocation.
    /// No safety property depends on `Drop` running for a view or iterator.
    ///
    /// ```compile_fail
    /// use moiren_engine::buffer::*;
    /// let mut arena = BufferArena::<f32>::new(&[BufferSlotLayout {
    ///     channels: 1, capacity_frames: 8,
    /// }], 1024).unwrap();
    /// let io = arena.prepare_io(&[PortAccess::Write {
    ///     port: 0, slot: arena.slot(0).unwrap(),
    /// }]).unwrap();
    /// let escaped = arena.with_io(&io, 0..8, |io| match io {
    ///     ProcessIo::Separate { mut outputs, .. } => outputs.get_mut(0).unwrap(),
    ///     _ => unreachable!(),
    /// });
    /// ```
    pub fn with_io<R>(
        &mut self,
        access: &PreparedIo,
        window: Range<usize>,
        f: impl for<'scope> FnOnce(ProcessIo<'scope, S>) -> R,
    ) -> Result<R, BufferError> {
        if !Arc::ptr_eq(&self.layout, &access.layout) {
            return Err(BufferError::ForeignAccess);
        }
        if window.start >= window.end || window.end > access.max_frames {
            return Err(BufferError::InvalidFrames);
        }
        // Take the slab pointer once, BEFORE any sample references exist. Do not
        // recreate a whole-slab reference while the scoped views are live.
        let base = self.data.as_mut_ptr();
        let raw = RawWindow {
            base,
            start: window.start,
            frames: window.end - window.start,
        };
        let inputs = ReadPorts {
            raw,
            bindings: &access.reads,
            borrow: PhantomData,
        };
        let outputs = WritePorts {
            raw,
            bindings: &access.writes,
            borrow: PhantomData,
        };
        let io = match access.mode {
            IoMode::ReadOnly => ProcessIo::ReadOnly { inputs },
            IoMode::Separate => ProcessIo::Separate { inputs, outputs },
            IoMode::InPlace => ProcessIo::InPlace {
                inputs,
                outputs,
                pairs: InPlacePorts {
                    raw,
                    bindings: &access.pairs,
                    borrow: PhantomData,
                },
            },
        };
        Ok(f(io))
    }
}
