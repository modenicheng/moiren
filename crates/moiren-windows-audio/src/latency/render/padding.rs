//! Fixed-size live render padding observations, independent of native IO.

use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

/// Raw native observations only: neither end-to-end latency nor XRUN evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderPaddingSample {
    pub padding_frames: u32,
    pub capacity_frames: u32,
    /// Observed client rate; `None` means unavailable, not a default rate.
    pub sample_rate: Option<u32>,
    /// Observed client-rate period; unknown is distinct from zero.
    pub period_frames: Option<u32>,
}

/// One coherent sample from the currently active owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderPaddingSnapshot {
    pub padding_frames: u32,
    pub capacity_frames: u32,
    pub sample_rate: Option<u32>,
    pub period_frames: Option<u32>,
    /// Strictly increases on publication, including across owner replacement.
    pub sequence: u64,
}

#[derive(Debug, Default)]
struct PublishedPadding {
    next_owner: AtomicU64,
    active_owner: AtomicU64,
    /// Even when stable, odd while a writer owns the scalar tuple.
    version: AtomicU64,
    sample_owner: AtomicU64,
    padding_frames: AtomicU32,
    capacity_frames: AtomicU32,
    /// Zero encodes None; u32 values are widened and offset by one.
    sample_rate: AtomicU64,
    period_frames: AtomicU64,
}

/// Cloneable read-side handle. Creation and owner preparation are non-RT;
/// scalar publication and one-attempt snapshots neither allocate nor wait.
#[derive(Debug, Default, Clone)]
pub struct RenderPaddingObserver {
    published: Arc<PublishedPadding>,
}

/// Unique publication capability for one native owner. Return this handle to
/// the joining control thread for destruction, so the last Arc is never freed
/// on the audio thread. Dropping it invalidates only its own generation.
#[derive(Debug)]
pub struct RenderPaddingOwner {
    published: Arc<PublishedPadding>,
    generation: u64,
}

impl RenderPaddingObserver {
    /// Allocate the fixed-size scalar storage before starting native IO.
    pub fn new() -> Self {
        Self::default()
    }

    /// Non-RT preparation. Immediately replaces the previous owner and makes
    /// its last sample unknown, even if that owner is still inside publish.
    ///
    /// Panics rather than reusing an owner identity after u64 exhaustion.
    pub fn new_owner(&self) -> RenderPaddingOwner {
        let generation = self
            .published
            .next_owner
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |previous| {
                previous.checked_add(1)
            })
            .expect("render padding owner identities exhausted")
            + 1;
        self.published
            .active_owner
            .store(generation, Ordering::SeqCst);
        RenderPaddingOwner {
            published: self.published.clone(),
            generation,
        }
    }

    /// One bounded attempt, with no stale fallback: unknown, a stopped owner,
    /// or an overlapping publication/replacement returns None. Retry on the
    /// next control poll, not in a loop on the audio thread.
    pub fn try_snapshot(&self) -> Option<RenderPaddingSnapshot> {
        let published = &self.published;
        let before = published.version.load(Ordering::SeqCst);
        if !before.is_multiple_of(2) {
            return None;
        }
        let active = published.active_owner.load(Ordering::SeqCst);
        let sample_owner = published.sample_owner.load(Ordering::SeqCst);
        let snapshot = RenderPaddingSnapshot {
            padding_frames: published.padding_frames.load(Ordering::SeqCst),
            capacity_frames: published.capacity_frames.load(Ordering::SeqCst),
            sample_rate: decode_optional(published.sample_rate.load(Ordering::SeqCst)),
            period_frames: decode_optional(published.period_frames.load(Ordering::SeqCst)),
            sequence: before / 2,
        };
        // As in TimelineObserver, all tuple/version accesses share one total
        // order. An unchanged even version proves no scalar writes overlapped;
        // the owner checks also fence reset/replacement independently of IO.
        (active != 0
            && active == sample_owner
            && active == published.active_owner.load(Ordering::SeqCst)
            && before == published.version.load(Ordering::SeqCst))
        .then_some(snapshot)
    }
}

impl RenderPaddingOwner {
    /// Publish one complete tuple with a single writer-acquisition attempt.
    /// False means stale/stopped owner, an in-flight predecessor, or exhausted
    /// sequence space. There is no spinning, allocation, destruction, or lock.
    /// A concurrent replacement may invalidate even a successful publication.
    pub fn publish(&mut self, sample: RenderPaddingSample) -> bool {
        self.publish_with(sample, || {})
    }

    // The empty production closure is optimized away. Unit tests pause the
    // real publication here to exercise contention without scheduler guesses.
    fn publish_with(&mut self, sample: RenderPaddingSample, in_flight: impl FnOnce()) -> bool {
        let published = &self.published;
        if self.generation == 0 || published.active_owner.load(Ordering::SeqCst) != self.generation
        {
            return false;
        }
        let before = published.version.load(Ordering::SeqCst);
        let Some(after) = before.checked_add(2) else {
            // Never wrap: that could accept a mixed tuple via version ABA.
            return false;
        };
        if !before.is_multiple_of(2)
            || published
                .version
                .compare_exchange(before, before + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return false;
        }
        if published.active_owner.load(Ordering::SeqCst) != self.generation {
            // Replacement may have published between our first owner check
            // and acquisition. No payload was touched, so keep its version.
            published.version.store(before, Ordering::SeqCst);
            return false;
        }
        in_flight();
        published
            .padding_frames
            .store(sample.padding_frames, Ordering::SeqCst);
        published
            .capacity_frames
            .store(sample.capacity_frames, Ordering::SeqCst);
        published
            .sample_rate
            .store(encode_optional(sample.sample_rate), Ordering::SeqCst);
        published
            .period_frames
            .store(encode_optional(sample.period_frames), Ordering::SeqCst);
        published
            .sample_owner
            .store(self.generation, Ordering::SeqCst);
        published.version.store(after, Ordering::SeqCst);
        true
    }

    /// Stop/reset this capability without waiting for readers or freeing its
    /// storage. A retired owner cannot invalidate a replacement owner's data.
    pub fn invalidate(&mut self) {
        let _ = self.published.active_owner.compare_exchange(
            self.generation,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        self.generation = 0;
    }
}

impl Drop for RenderPaddingOwner {
    fn drop(&mut self) {
        self.invalidate();
    }
}

fn encode_optional(value: Option<u32>) -> u64 {
    value.map_or(0, |value| u64::from(value) + 1)
}

fn decode_optional(value: u64) -> Option<u32> {
    value.checked_sub(1).map(|value| value as u32)
}

#[cfg(test)]
#[path = "padding_tests.rs"]
mod tests;
