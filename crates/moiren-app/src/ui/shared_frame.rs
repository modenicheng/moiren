//! Geometry shared between a cached panel's prepaint and a hot surface's
//! paint (design 17, Tier H). The cached subtree cannot hand per-frame
//! geometry to its sibling — gpui-fast replays a cached panel's paint without
//! re-running its closures — so the geometry is published out of band and
//! read by the hot layer at paint time. Stale by design: between cache
//! rebuilds, the geometry the cached panel painted stands.
use std::cell::RefCell;
use std::rc::Rc;

enum Slot<F> {
    Empty,
    Current(Rc<F>),
}

/// A shared geometry frame. Handles clone by sharing — every holder sees the
/// same slot, like the `Rc<RefCell<...>>` it replaces. Single-writer
/// surfaces publish a whole frame ([`SharedFrame::publish`]); accumulating
/// surfaces reset it on rebuild and mutate it from each registering canvas
/// ([`SharedFrame::reset`]/[`SharedFrame::update`]). Readers clone the
/// current `Rc`, so painting never borrows the cell across a yield point.
pub(in crate::ui) struct SharedFrame<F>(Rc<RefCell<Slot<F>>>);

impl<F> Default for SharedFrame<F> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(Slot::Empty)))
    }
}

impl<F> Clone for SharedFrame<F> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<F> SharedFrame<F> {
    /// Publish a freshly built frame (single-writer surfaces).
    pub(in crate::ui) fn publish(&self, frame: Rc<F>) {
        *self.0.borrow_mut() = Slot::Current(frame);
    }

    /// The frame the sibling cached panel last painted with.
    pub(in crate::ui) fn read(&self) -> Option<Rc<F>> {
        match &*self.0.borrow() {
            Slot::Empty => None,
            Slot::Current(frame) => Some(Rc::clone(frame)),
        }
    }
}

impl<F: Default + Clone> SharedFrame<F> {
    /// Start a rebuild from a blank frame (accumulating surfaces). Each
    /// registering canvas then extends it with [`SharedFrame::update`].
    pub(in crate::ui) fn reset(&self) {
        self.publish(Rc::new(F::default()));
    }

    /// Mutate the current frame in place. If a reader still holds the frame
    /// the previous paint ran against, the mutation applies to a fresh copy
    /// instead of the shared one.
    pub(in crate::ui) fn update(&self, f: impl FnOnce(&mut F)) {
        let mut slot = self.0.borrow_mut();
        match &mut *slot {
            Slot::Empty => {}
            Slot::Current(frame) => {
                if let Some(unique) = Rc::get_mut(frame) {
                    f(unique);
                    return;
                }
                let mut next = (**frame).clone();
                f(&mut next);
                *frame = Rc::new(next);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_and_read_round_trip_through_shared_handles() {
        let frame: SharedFrame<u32> = SharedFrame::default();
        let handle = frame.clone();
        assert!(handle.read().is_none(), "nothing published yet");
        // A handle taken before the first publish still sees it: the slot is
        // shared, not snapshotted.
        frame.publish(Rc::new(11));
        assert_eq!(handle.read().map(|value| *value), Some(11));
    }

    #[test]
    fn update_mutates_in_place_until_a_reader_holds_the_frame() {
        #[derive(Default, Clone, PartialEq)]
        struct Geometry {
            marks: Vec<u32>,
        }
        let frame: SharedFrame<Geometry> = SharedFrame::default();
        frame.reset();
        frame.update(|geometry| geometry.marks.push(1));
        let reader = frame.read().expect("frame after reset");
        frame.update(|geometry| geometry.marks.push(2));
        // The reader kept the frame it saw; the next reader gets both marks.
        assert_eq!(reader.marks, vec![1]);
        let latest = frame.read().expect("frame after update");
        assert_eq!(latest.marks, vec![1, 2]);
    }

    #[test]
    fn update_before_any_reset_is_a_no_op() {
        let frame: SharedFrame<u32> = SharedFrame::default();
        frame.update(|value| *value = 5);
        assert!(frame.read().is_none(), "no frame to mutate");
    }
}
