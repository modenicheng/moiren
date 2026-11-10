//! Wheel classification for slider gesture surfaces: a discrete wheel notch
//! steps the value, a continuous touchpad pan tracks the fingers along the
//! track.

use gpui_kit::*;

/// One scroll event translated into slider intent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollGesture {
    /// Wheel travel in signed behavior steps, +1 up/right: exactly ±1 for a
    /// whole-line event, a share of one step for a fractional one.
    Notch(f32),
    /// Continuous travel (precision touchpad pan) as a fraction of the
    /// gesture surface's extent along the slider axis, positive toward the
    /// slider's max. Already sign-oriented so the thumb follows the fingers.
    Pan(f32),
}

/// Wheel lines one notch scrolls at the Windows default setting. A
/// fractional line delta counts as its share of a notch against this.
const LINES_PER_NOTCH: f32 = 3.;

/// Classify one wheel event. `track_length` is the gesture surface's extent
/// along `axis` in logical pixels; a zero length (before the first layout)
/// swallows the event.
///
/// Whole-line deltas keep wheel semantics — one notch per event regardless
/// of magnitude, so a real mouse wheel behaves exactly as before at any
/// "lines per notch" setting. Fractional line deltas come from devices that
/// report finer than a detent: high-resolution wheels, and touchpads whose
/// pans arrive as wheel messages rather than DirectManipulation pixels. They
/// step by their share of a notch (design 22 D3), so a gesture glides where
/// it used to climb a full ±1 dB step per event. Pixel deltas become
/// proportional travel: the thumb follows the fingers, which for the
/// vertical axis means inverting the raw delta (the system's
/// natural-scrolling convention reports content-space travel, so an upward
/// finger motion arrives negative; a horizontal pan is untouched by that
/// setting and follows the finger directly).
pub fn scroll_gesture(delta: &ScrollDelta, axis: Axis, track_length: f32) -> Option<ScrollGesture> {
    let (x, y) = match delta {
        ScrollDelta::Lines(lines) => {
            return line_notches(lines).map(ScrollGesture::Notch);
        }
        ScrollDelta::Pixels(pixels) => (f32::from(pixels.x), f32::from(pixels.y)),
    };
    if track_length <= 0. {
        return None;
    }
    let travel = match axis {
        Axis::Vertical => -y,
        Axis::Horizontal => x,
    };
    let normalized = travel / track_length;
    (normalized != 0.).then_some(ScrollGesture::Pan(normalized))
}

/// Wheel travel as steps: wheel up (or tilt right) is positive. The
/// secondary axis covers shift+wheel and tilt wheels.
fn line_notches(lines: &Point<f32>) -> Option<f32> {
    let travel = if lines.y != 0. {
        lines.y
    } else if lines.x != 0. {
        lines.x
    } else {
        return None;
    };
    Some(if travel.fract() == 0. {
        travel.signum()
    } else {
        travel / LINES_PER_NOTCH
    })
}

#[cfg(test)]
mod tests {
    // Explicit imports: `super::*` would re-export gpui-kit's `test` macro
    // and shadow the built-in `#[test]` attribute.
    use super::{ScrollGesture, scroll_gesture};
    use gpui_kit::{Axis, ScrollDelta, point, px};

    fn pixels(x: f32, y: f32) -> ScrollDelta {
        ScrollDelta::Pixels(point(px(x), px(y)))
    }

    fn lines(x: f32, y: f32) -> ScrollDelta {
        ScrollDelta::Lines(point(x, y))
    }

    #[test]
    fn whole_line_notches_keep_wheel_direction_and_discard_magnitude() {
        assert_eq!(
            scroll_gesture(&lines(0., 3.), Axis::Vertical, 300.),
            Some(ScrollGesture::Notch(1.))
        );
        assert_eq!(
            scroll_gesture(&lines(0., -6.), Axis::Vertical, 300.),
            Some(ScrollGesture::Notch(-1.))
        );
        // A one-line-per-notch setting still steps a whole notch.
        assert_eq!(
            scroll_gesture(&lines(0., 1.), Axis::Vertical, 300.),
            Some(ScrollGesture::Notch(1.))
        );
        assert_eq!(
            scroll_gesture(&lines(2., 0.), Axis::Horizontal, 100.),
            Some(ScrollGesture::Notch(1.))
        );
        assert_eq!(scroll_gesture(&lines(0., 0.), Axis::Vertical, 300.), None);
    }

    #[test]
    fn fractional_lines_step_by_their_share_of_a_notch() {
        // A quarter detent of a high-resolution wheel at the default three
        // lines per notch arrives as 0.75 lines: a quarter step.
        assert_eq!(
            scroll_gesture(&lines(0., 0.75), Axis::Vertical, 300.),
            Some(ScrollGesture::Notch(0.25))
        );
        assert_eq!(
            scroll_gesture(&lines(0., -0.375), Axis::Vertical, 300.),
            Some(ScrollGesture::Notch(-0.125))
        );
        // Shift+wheel and tilt wheels take the secondary axis the same way.
        assert_eq!(
            scroll_gesture(&lines(1.5, 0.), Axis::Horizontal, 100.),
            Some(ScrollGesture::Notch(0.5))
        );
        // Four fractional events add up to one detent's travel.
        let steps: f32 = (0..4)
            .map(
                |_| match scroll_gesture(&lines(0., 0.75), Axis::Vertical, 300.) {
                    Some(ScrollGesture::Notch(step)) => step,
                    other => panic!("expected a fractional notch, got {other:?}"),
                },
            )
            .sum();
        assert_eq!(steps, 1.);
    }

    #[test]
    fn vertical_pans_follow_the_fingers() {
        // An upward two-finger motion arrives as negative dy and must raise
        // the thumb (positive travel toward max).
        assert_eq!(
            scroll_gesture(&pixels(0., -12.), Axis::Vertical, 300.),
            Some(ScrollGesture::Pan(12. / 300.))
        );
        assert_eq!(
            scroll_gesture(&pixels(0., 6.), Axis::Vertical, 300.),
            Some(ScrollGesture::Pan(-6. / 300.))
        );
    }

    #[test]
    fn horizontal_pans_follow_the_fingers_directly() {
        assert_eq!(
            scroll_gesture(&pixels(9., -4.), Axis::Horizontal, 120.),
            Some(ScrollGesture::Pan(9. / 120.))
        );
    }

    #[test]
    fn cross_axis_travel_is_ignored() {
        assert_eq!(scroll_gesture(&pixels(7., 0.), Axis::Vertical, 300.), None);
        assert_eq!(
            scroll_gesture(&pixels(0., -5.), Axis::Horizontal, 300.),
            None
        );
    }

    #[test]
    fn zero_track_length_and_zero_travel_swallow_the_event() {
        assert_eq!(scroll_gesture(&pixels(0., -5.), Axis::Vertical, 0.), None);
        assert_eq!(scroll_gesture(&pixels(0., 0.), Axis::Vertical, 300.), None);
    }
}
