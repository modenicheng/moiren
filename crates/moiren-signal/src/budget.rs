use std::{
    alloc::Layout,
    cell::UnsafeCell,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
};

use crossbeam_utils::CachePadded;

use crate::{
    RegisterError,
    storage::{BlockSlot, LatestStorage, StreamStorage, ValueSlot},
};

#[derive(Debug)]
pub(crate) struct Budget {
    used: Arc<AtomicUsize>,
    limit: usize,
}

impl Budget {
    pub(crate) fn new(limit: usize, baseline: usize) -> Result<Self, RegisterError> {
        if baseline > limit {
            return Err(RegisterError::BudgetExceeded);
        }
        Ok(Self {
            used: Arc::new(AtomicUsize::new(baseline)),
            limit,
        })
    }

    pub(crate) fn check(&self, additional: usize) -> Result<usize, RegisterError> {
        let next = add(self.used(), additional)?;
        if next > self.limit {
            return Err(RegisterError::BudgetExceeded);
        }
        Ok(next)
    }
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
    pub(crate) fn reserve(&self, bytes: usize) -> Charge {
        let charge = Charge(Some(Arc::new(Reservation {
            ledger: Arc::clone(&self.used),
            bytes,
        })));
        self.used.fetch_add(bytes, Ordering::AcqRel);
        charge
    }
}

struct Reservation {
    ledger: Arc<AtomicUsize>,
    bytes: usize,
}

#[derive(Clone)]
/// Shared credit is released after resource owners and this Arc allocation.
/// Every handle uses into_inner, so concurrent final drops elect one refund.
pub(crate) struct Charge(Option<Arc<Reservation>>);
impl Drop for Charge {
    fn drop(&mut self) {
        if let Some(reservation) = Arc::into_inner(self.0.take().expect("live reservation")) {
            reservation
                .ledger
                .fetch_sub(reservation.bytes, Ordering::AcqRel);
        }
    }
}

pub(crate) fn reservation_bytes() -> Result<usize, RegisterError> {
    Ok(arc_layout::<Reservation>()?.size())
}
pub(crate) fn state_bytes() -> Result<usize, RegisterError> {
    add(
        arc_layout::<crate::subscriber::TopicState>()?.size(),
        reservation_bytes()?,
    )
}

pub(crate) fn add(a: usize, b: usize) -> Result<usize, RegisterError> {
    a.checked_add(b).ok_or(RegisterError::SizeOverflow)
}

pub(crate) fn array<T>(count: usize) -> Result<Layout, RegisterError> {
    Layout::array::<T>(count).map_err(|_| RegisterError::SizeOverflow)
}

fn extend(a: Layout, b: Layout) -> Result<Layout, RegisterError> {
    a.extend(b)
        .map(|(layout, _)| layout)
        .map_err(|_| RegisterError::SizeOverflow)
}

pub(crate) fn arc_layout<T>() -> Result<Layout, RegisterError> {
    Ok(extend(array::<AtomicUsize>(2)?, Layout::new::<T>())?.pad_to_align())
}

pub(crate) fn latest_subscriber<S: Clone + Send>(
    element_bytes: usize,
) -> Result<usize, RegisterError> {
    add(
        add(
            add(
                Layout::new::<crate::subscriber::LatestReceiver<S>>().size(),
                latest_shared::<S>()?.size(),
            )?,
            arc_layout::<crate::subscriber::SubscriptionLease>()?.size(),
        )?,
        add(
            element_bytes
                .checked_mul(3)
                .ok_or(RegisterError::SizeOverflow)?,
            reservation_bytes()?,
        )?,
    )
}

/// Allocation layout model for the *pinned* triple_buffer 9.0.0 shared state.
///
/// Upstream's private SharedState is three CachePadded<UnsafeCell<S>> followed
/// by CachePadded<AtomicU8>, owned through std::sync::Arc. Every field size is
/// a multiple of its cache/slot alignment, so Rust field ordering does not
/// change this total size/alignment. The Arc allocation has two AtomicUsize
/// counters before its aligned data. This is a layout model, never a cast to
/// upstream's private struct. Allocator probes cover ZST and over-aligned
/// values/blocks; any dependency/toolchain layout change must rerun them.
pub(crate) fn latest_shared<S: Send>() -> Result<Layout, RegisterError> {
    let buffers = array::<CachePadded<UnsafeCell<S>>>(3)?;
    let shared = extend(buffers, Layout::new::<CachePadded<AtomicU8>>())?.pad_to_align();
    Ok(extend(array::<AtomicUsize>(2)?, shared)?.pad_to_align())
}

fn owner_ring<S>(depth: usize) -> Result<usize, RegisterError> {
    // rtrb uses indices in 0..2*capacity, even for zero-sized queue items.
    depth.checked_mul(2).ok_or(RegisterError::SizeOverflow)?;
    add(
        Layout::new::<rtrb::RingBuffer<S>>().size(),
        array::<S>(depth)?.size(),
    )
}

pub(crate) fn latest<S: Clone + Send>(element_bytes: usize) -> Result<usize, RegisterError> {
    add(
        add(
            Layout::new::<LatestStorage<S>>().size(),
            add(latest_shared::<S>()?.size(), state_bytes()?)?,
        )?,
        add(element_bytes, reservation_bytes()?)?,
    )
}

pub(crate) fn stream<S: Clone + Send>(
    depth: usize,
    element_bytes: usize,
) -> Result<usize, RegisterError> {
    let slots = add(depth, 1)?;
    let queues = add(owner_ring::<S>(slots)?, owner_ring::<S>(depth)?)?;
    let base = add(Layout::new::<StreamStorage<S>>().size(), state_bytes()?)?;
    add(
        add(base, queues)?,
        add(element_bytes, reservation_bytes()?)?,
    )
}

pub(crate) fn stream_subscriber<S>(
    depth: usize,
    element_bytes: usize,
) -> Result<usize, RegisterError> {
    let slots = add(depth, 1)?;
    add(
        add(
            add(
                Layout::new::<crate::subscriber::StreamReceiver<S>>().size(),
                arc_layout::<crate::subscriber::SubscriptionLease>()?.size(),
            )?,
            add(
                add(owner_ring::<S>(slots)?, owner_ring::<S>(depth)?)?,
                arc_layout::<std::sync::Mutex<Option<crate::storage::PoolReader<S>>>>()?.size(),
            )?,
        )?,
        add(
            element_bytes
                .checked_mul(slots)
                .ok_or(RegisterError::SizeOverflow)?,
            reservation_bytes()?,
        )?,
    )
}

pub(crate) fn value_copy<T>() -> Layout {
    Layout::new::<ValueSlot<T>>()
}

pub(crate) fn block_copy<H, E>(capacity: usize) -> Result<Layout, RegisterError> {
    // Box pointers are not copied to readers: the actual copy is metadata,
    // business header, and the complete fixed-capacity element region.
    Ok(extend(
        extend(Layout::new::<crate::PublicationMeta>(), Layout::new::<H>())?,
        array::<E>(capacity)?,
    )?
    .pad_to_align())
}

pub(crate) fn block_elements<E>(capacity: usize, slots: usize) -> Result<usize, RegisterError> {
    // Check logical element counts even for ZSTs: multiplying byte sizes first
    // would hide a capacity overflow and allow factories to run for that input.
    capacity
        .checked_mul(slots)
        .ok_or(RegisterError::SizeOverflow)?;
    array::<E>(capacity)?
        .size()
        .checked_mul(slots)
        .ok_or(RegisterError::SizeOverflow)
}

pub(crate) fn latest_block<H: Copy + Send, E: Copy + Send>(
    capacity: usize,
) -> Result<usize, RegisterError> {
    latest::<BlockSlot<H, E>>(block_elements::<E>(capacity, 5)?)
}

pub(crate) fn stream_block<H: Copy + Send, E: Copy + Send>(
    capacity: usize,
    depth: usize,
) -> Result<usize, RegisterError> {
    stream::<BlockSlot<H, E>>(depth, block_elements::<E>(capacity, add(depth, 3)?)?)
}
