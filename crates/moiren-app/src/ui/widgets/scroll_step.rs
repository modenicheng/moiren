//! Wheel classification for slider gesture surfaces: a discrete wheel notch
//! steps the value, a continuous touchpad pan tracks the fingers along the
//! track.

use gpui_kit::*;

/// One scroll event translated into slider intent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollGesture {
    /// A discrete wheel notch: one signed behavior step, +1 up/right.
    Notch(i32),
    /// Continuous travel (precision touchpad pan) as a fraction of the
    /// gesture surface's extent along the slider axis, positive toward the
    /// slider's max. Already sign-oriented so the thumb follows the fingers.
    Pan(f32),
}

/// Classify one wheel event. `track_length` is the gesture surface's extent
/// along `axis` in logical pixels; a zero length (before the first layout)
/// swallows the event.
///
/// Line deltas keep wheel semantics — one notch per event regardless of
/// magnitude, so a real mouse wheel behaves exactly as before. Pixel deltas
/// become proportional travel: the thumb follows the fingers, which for the
/// vertical axis means inverting the raw delta (the system's
/// natural-scrolling convention reports content-space travel, so an upward
/// finger motion arrives negative; a horizontal pan is untouched by that
/// setting and follows the finger directly).
pub fn scroll_gesture(delta: &ScrollDelta, axis: Axis, track_length: f32) -> Option<ScrollGesture> {
    let (x, y) = match delta {
        ScrollDelta::Lines(lines) => {
            return lines_direction(lines).map(ScrollGesture::Notch);
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

/// Wheel direction as a step count: wheel up (or tilt right) is +1. The
/// secondary axis covers shift+wheel and tilt wheels.
fn lines_direction(lines: &Point<f32>) -> Option<i32> {
    if lines.y != 0. {
        Some(lines.y.signum() as i32)
    } else if lines.x != 0. {
        Some(lines.x.signum() as i32)
    } else {
        None
    }
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

    #[test]
    fn line_notches_keep_wheel_direction_and_discard_magnitude() {
        assert_eq!(
            scroll_gesture(&ScrollDelta::Lines(point(0., 3.)), Axis::Vertical, 300.),
            Some(ScrollGesture::Notch(1))
        );
        assert_eq!(
            scroll_gesture(&ScrollDelta::Lines(point(0., -0.25)), Axis::Vertical, 300.),
            Some(ScrollGesture::Notch(-1))
        );
        assert_eq!(
            scroll_gesture(&ScrollDelta::Lines(point(2., 0.)), Axis::Horizontal, 100.),
            Some(ScrollGesture::Notch(1))
        );
        assert_eq!(
            scroll_gesture(&ScrollDelta::Lines(point(0., 0.)), Axis::Vertical, 300.),
            None
        );
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
