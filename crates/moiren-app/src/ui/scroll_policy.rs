//! Scroll policy for the graph canvas, ported from the Slint-era
//! graph_input.rs. Touchpad two-finger scroll pans; wheel or Ctrl+scroll
//! zooms; both responses are multiplicative and reversible.

#[derive(Debug, PartialEq)]
pub enum ScrollAction {
    Pan(f32, f32),
    Zoom(f32),
}

// Keep wheel zoom smooth for both integer wheel notches and high-resolution
// pixel deltas. A 60 logical-pixel step is about 2%, while smaller deltas are
// still preserved instead of being rounded to a direction-only step.
const WHEEL_ZOOM_STEP: f32 = 60.0;
const WHEEL_ZOOM_FACTOR: f32 = 1.02;
const MAX_ZOOM_LOG_STEP: f32 = 0.5;

fn exponential_zoom_factor(log_delta: f32) -> f32 {
    log_delta.clamp(-MAX_ZOOM_LOG_STEP, MAX_ZOOM_LOG_STEP).exp()
}

fn wheel_zoom_factor(delta_y: f32) -> f32 {
    exponential_zoom_factor(delta_y / WHEEL_ZOOM_STEP * WHEEL_ZOOM_FACTOR.ln())
}

pub fn pinch_zoom_factor(delta: f32) -> f32 {
    // gpui reports pinch delta as a fractional scale change, same as the
    // original winit mapping: 0.1 == +10%, multiplicative and reversible.
    exponential_zoom_factor(delta / 0.1 * 1.1_f32.ln())
}

/// Classify one scroll event. `continuous` marks pixel-precise deltas
/// (precision touchpads); discrete notches from mice always zoom.
pub fn scroll_action(delta_x: f32, delta_y: f32, continuous: bool, control: bool) -> ScrollAction {
    if continuous && !control {
        ScrollAction::Pan(delta_x, delta_y)
    } else {
        ScrollAction::Zoom(wheel_zoom_factor(delta_y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discrete_wheel_notches_zoom_symmetrically() {
        let up = scroll_action(0., 60., false, false);
        let down = scroll_action(0., -60., false, false);
        match (up, down) {
            (ScrollAction::Zoom(up), ScrollAction::Zoom(down)) => {
                assert!((up * down - 1.0).abs() < 1e-6, "exact inverse");
                assert!((up - 1.02).abs() < 1e-6);
            }
            other => panic!("expected zoom, got {other:?}"),
        }
    }

    #[test]
    fn continuous_touchpad_two_finger_scrolls_pan() {
        assert_eq!(
            scroll_action(3., -7., true, false),
            ScrollAction::Pan(3., -7.)
        );
    }

    #[test]
    fn pinch_control_zooms_even_when_continuous() {
        assert_eq!(
            scroll_action(0., -60., true, true),
            ScrollAction::Zoom(wheel_zoom_factor(-60.))
        );
    }

    #[test]
    fn pinch_factor_is_multiplicative_and_reversible() {
        // gpui pinch delta is fractional: 0.1 == +10%.
        let f = pinch_zoom_factor(0.1);
        assert!((f - 1.1).abs() < 1e-5);
        assert!((f * pinch_zoom_factor(-0.1) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn large_deltas_are_clamped() {
        let huge = scroll_action(0., 1e6, false, false);
        let tiny = scroll_action(0., -1e6, false, false);
        let e = MAX_ZOOM_LOG_STEP.exp();
        match (huge, tiny) {
            (ScrollAction::Zoom(up), ScrollAction::Zoom(down)) => {
                assert!((up - e).abs() < 1e-6 && (down - 1. / e).abs() < 1e-6);
            }
            other => panic!("expected zoom, got {other:?}"),
        }
    }
}
