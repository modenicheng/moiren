//! Shared placement math for anchored popovers (design 21 §3.1).
//!
//! Every shell-root popover hangs beside something on screen — an anchor
//! rect a canvas hit test reported, or the point a trigger was clicked at —
//! and must stay inside the window no matter where that anchor lands. The
//! formulas live here once so the popovers cannot drift apart; the values
//! are the theme's popover placement tokens (design 21 §1.3), the ones the
//! fault popover established.

use crate::ui::theme;
use gpui_kit::{Bounds, Pixels, Size};

/// The window-pixel position a popover lands at.
pub(crate) struct Anchored {
    pub left: f32,
    pub top: f32,
}

/// Clamps a horizontal origin so a `width` box stays inside the window,
/// keeping the viewport margin on both sides. Shared by every anchored
/// popover's left edge, rect-hung or point-hung.
pub(crate) fn clamped_left(x: f32, width: f32, viewport: Size<Pixels>) -> f32 {
    x.min(f32::from(viewport.width) - width - theme::POPOVER_VIEWPORT_MARGIN)
        .max(theme::POPOVER_VIEWPORT_MARGIN)
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
    let below = f32::from(anchor.origin.y + anchor.size.height) + theme::POPOVER_ANCHOR_GAP;
    let top = if below + height > f32::from(viewport.height) - theme::POPOVER_VIEWPORT_MARGIN {
        (f32::from(anchor.origin.y) - theme::POPOVER_ANCHOR_GAP - height)
            .max(theme::POPOVER_VIEWPORT_MARGIN)
    } else {
        below
    };
    Anchored { left, top }
}
