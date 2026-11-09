//! Shared placement math for anchored popovers (design 21 §3.1).
//!
//! Every shell-root popover hangs beside something on screen — an anchor
//! rect a canvas hit test reported, or the point a trigger was clicked at —
//! and must stay inside the window no matter where that anchor lands. The
//! formulas live here once so the popovers cannot drift apart; the values
//! (8px viewport margin, 4px anchor gap) are the ones the fault popover
//! established and are kept verbatim — moving them to theme tokens is a
//! later PR.

use gpui_kit::{Bounds, Pixels, Size};

/// Space a popover keeps from the window edges.
const VIEWPORT_MARGIN: f32 = 8.;
/// Space between the anchor's edge and the popover placed beside it.
const ANCHOR_GAP: f32 = 4.;

/// The window-pixel position a popover lands at.
pub(crate) struct Anchored {
    pub left: f32,
    pub top: f32,
}

/// Clamps a horizontal origin so a `width` box stays inside the window,
/// keeping the viewport margin on both sides. Shared by every anchored
/// popover's left edge, rect-hung or point-hung.
pub(crate) fn clamped_left(x: f32, width: f32, viewport: Size<Pixels>) -> f32 {
    x.min(f32::from(viewport.width) - width - VIEWPORT_MARGIN)
        .max(VIEWPORT_MARGIN)
}

/// Hangs a `width` × `height` box under `anchor`, flipping above it when
/// the window bottom is too close for the box to fit below, and keeping it
/// inside the window sideways. `width` must already include any widening
/// over the anchor itself. This is the fault popover's and binding menu's
/// placement.
pub(crate) fn under_anchor(
    anchor: Bounds<Pixels>,
    width: f32,
    height: f32,
    viewport: Size<Pixels>,
) -> Anchored {
    let left = clamped_left(f32::from(anchor.origin.x), width, viewport);
    let below = f32::from(anchor.origin.y + anchor.size.height) + ANCHOR_GAP;
    let top = if below + height > f32::from(viewport.height) - VIEWPORT_MARGIN {
        (f32::from(anchor.origin.y) - ANCHOR_GAP - height).max(VIEWPORT_MARGIN)
    } else {
        below
    };
    Anchored { left, top }
}
