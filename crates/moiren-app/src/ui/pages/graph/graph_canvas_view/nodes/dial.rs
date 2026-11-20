use gpui_kit::{App, FontWeight};

use crate::ui::pages::graph::graph_canvas::{CanvasFrame, CardKnob, KnobLayout};
use crate::ui::theme;

use super::geometry::{arc_capsule_path, capsule_path};
use super::shared::{Label, Paint, paint_label};

/// One knob dial, parameterized by the sweep it turns through and where
/// its value arc anchors: a neutral pill-ended track over the whole sweep,
/// the value portion filled in `TRACK_FILL` from the control's origin to
/// its position (unipolar knobs fill from the floor, bipolar ones from
/// their neutral point, in whichever direction the value lies), and a
/// slim pill pointer at the value angle. Screen angles run from
/// `DIAL_SWEEP_START` with y pointing down, so the sweep starts bottom-
/// left, passes the top, and ends bottom-right.
pub(super) fn paint_knob(
    frame: &CanvasFrame,
    layout: &KnobLayout,
    knob: &CardKnob,
    scale: f32,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let dial = frame.window_rect(layout.dial);
    let center = (
        f32::from(dial.origin.x) + f32::from(dial.size.width) / 2.,
        f32::from(dial.origin.y) + f32::from(dial.size.height) / 2.,
    );
    let track = theme::card::DIAL_TRACK_W * scale;
    let radius = f32::from(dial.size.width) / 2. - track;
    let angle_at = |fraction: f32| {
        theme::card::DIAL_SWEEP_START + theme::card::DIAL_SWEEP_DEG * fraction.clamp(0., 1.)
    };
    if let Ok(path) = arc_capsule_path(center, radius, track / 2., angle_at(0.), angle_at(1.)) {
        window.paint_path(path, theme::WINDOW_BG);
    }
    let (from, to) = if knob.position >= knob.origin {
        (knob.origin, knob.position)
    } else {
        (knob.position, knob.origin)
    };
    if to - from > 0.002
        && let Ok(path) = arc_capsule_path(center, radius, track / 2., angle_at(from), angle_at(to))
    {
        window.paint_path(path, theme::TRACK_FILL);
    }
    // Pointer from the hub to the track's inner edge: two thirds of the
    // old 1.6× stroke, with pill ends.
    let angle = angle_at(knob.position).to_radians();
    let direction = (angle.cos(), angle.sin());
    let (inner, outer) = (radius * 0.35, radius - track);
    if let Ok(path) = capsule_path(
        (
            center.0 + direction.0 * inner,
            center.1 + direction.1 * inner,
        ),
        (
            center.0 + direction.0 * outer,
            center.1 + direction.1 * outer,
        ),
        track * 0.8 / 2.,
    ) {
        window.paint_path(path, theme::card::THUMB);
    }
    paint_label(
        frame,
        &layout.label,
        Label {
            text: &knob.label,
            family: theme::FONT_UI,
            color: theme::TEXT_FAINT,
            right_aligned: false,
            weight: FontWeight::NORMAL,
        },
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
    paint_label(
        frame,
        &layout.value,
        Label {
            text: &knob.value_text,
            family: theme::FONT_MONO,
            color: theme::TEXT,
            right_aligned: false,
            weight: FontWeight::NORMAL,
        },
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
}
