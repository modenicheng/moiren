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

/// A shared geometry frame. Single-writer surfaces replace the frame
/// wholesale ([`SharedFrame::replace`]); accumulating surfaces reset it on
/// rebuild and mutate it from each registering canvas
/// ([`SharedFrame::reset`]/[`SharedFrame::update`]). Readers clone the current
/// `Rc`, so painting never borrows the cell across a yield point.
pub(in crate::ui) struct SharedFrame<F>(RefCell<Slot<F>>);

impl<F> Default for SharedFrame<F> {
    fn default() -> Self {
        Self(RefCell::new(Slot::Empty))
    }
}

impl<F> Clone for SharedFrame<F> {
    fn clone(&self) -> Self {
        Self(RefCell::new(match &*self.0.borrow() {
            Slot::Empty => Slot::Empty,
            Slot::Current(frame) => Slot::Current(Rc::clone(frame)),
        }))
    }
}

impl<F> SharedFrame<F> {
    /// Publish a freshly built frame (single-writer surfaces).
    pub(in crate::ui) fn replace(&self, frame: F) {
        *self.0.borrow_mut() = Slot::Current(Rc::new(frame));
    }

    /// Drop the current frame (nothing was painted this rebuild).
    pub(in crate::ui) fn clear(&self) {
        *self.0.borrow_mut() = Slot::Empty;
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
        *self.0.borrow_mut() = Slot::Current(Rc::new(F::default()));
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
    fn replace_and_read_round_trip_the_published_frame() {
        let frame: SharedFrame<u32> = SharedFrame::default();
        assert!(frame.read().is_none(), "nothing published yet");
        frame.replace(11);
        assert_eq!(frame.read().map(|value| *value), Some(11));
        frame.clear();
        assert!(frame.read().is_none());
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
