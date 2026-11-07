//! Firefox-style overlay scrollbars.
//!
//! The bar never takes layout space: the thumb strip paints on top of the
//! content over a transparent track, appears while scrolling and fades away
//! once the scroll goes idle (`ScrollbarMode::Scrolling`), and widens under
//! the cursor. A container whose content fits shows no bar at all.
//!
//! Layering rule (load-bearing): the overlay must be a *sibling* of the
//! scroll container under a shared `relative` wrapper — never a child of the
//! container. A scroll container translates every child by the current
//! offset during prepaint, so an overlay placed inside it drifts away with
//! the content; as the later sibling it stays pinned (gpui has no z-index,
//! DOM order decides who paints above). For the same reason an overlay never
//! goes inside a cached panel: the cached splice replays paint but not
//! hitboxes, so the thumb would freeze and lose its drag target. Wrap the
//! cached subtree from outside.

use gpui_kit::InteractiveElement;
use gpui_kit::base::{Scrollbar, ScrollbarAxis, ScrollbarMode, ScrollbarStyles};
use gpui_kit::*;

use crate::ui::observe::TestObserve;
use crate::ui::theme;

/// Hit strip across the content edge. Narrower than the library default
/// (16px) so the bar reads as Firefox's thin overlay, still wide enough to
/// grab on the first try.
const TRACK_WIDTH: f32 = 10.;
/// Thumb width at rest; hover and drag widen it to [`THUMB_ACTIVE_WIDTH`].
const THUMB_WIDTH: f32 = 5.;
const THUMB_ACTIVE_WIDTH: f32 = 8.;
/// Resting gap between the thumb and the content edge.
const THUMB_INSET: f32 = 2.;
const THUMB_RADIUS: f32 = 3.;
/// Shortest thumb, so a long list never collapses the grab target away.
const MIN_THUMB_LENGTH: f32 = 32.;

/// Thumb colors: the neutral grip gray, translucent like Firefox's overlay
/// thumbs. Hover and drag raise alpha instead of shifting hue.
const THUMB: Hsla = Hsla {
    a: 0.4,
    ..theme::GRIP
};
const THUMB_HOVER: Hsla = Hsla {
    a: 0.62,
    ..theme::GRIP
};
const THUMB_DRAG: Hsla = Hsla {
    a: 0.85,
    ..theme::GRIP
};

/// The moiren bar look: transparent tracks, a thin translucent neutral thumb
/// that widens under the cursor and while dragged. Built as the closure the
/// library's [`Scrollbar::styles`] expects so the caller passes it verbatim.
fn scrollbar_styles() -> impl FnOnce(ScrollbarStyles) -> ScrollbarStyles {
    // `s` arrives as the library default; the tracks never paint (only the
    // thumb shows, Firefox-style) — the width still shapes the hit strip.
    |s: ScrollbarStyles| {
        s.track(|t| t.width(px(TRACK_WIDTH)))
            .track_hover(|t| t.width(px(TRACK_WIDTH)))
            .track_active(|t| t.width(px(TRACK_WIDTH)))
            .thumb(|t| {
                t.bg(THUMB)
                    .width(px(THUMB_WIDTH))
                    .inset(px(THUMB_INSET))
                    .radius(px(THUMB_RADIUS))
                    .min_length(px(MIN_THUMB_LENGTH))
            })
            .thumb_hover(|t| {
                t.bg(THUMB_HOVER)
                    .width(px(THUMB_ACTIVE_WIDTH))
                    .inset(px(THUMB_INSET))
                    .radius(px(THUMB_RADIUS))
                    .min_length(px(MIN_THUMB_LENGTH))
            })
            .thumb_active(|t| {
                t.bg(THUMB_DRAG)
                    .width(px(THUMB_ACTIVE_WIDTH))
                    .inset(px(THUMB_INSET))
                    .radius(px(THUMB_RADIUS))
                    .min_length(px(MIN_THUMB_LENGTH))
            })
    }
}

/// The pinned scrollbar overlay for one scroll container: an `inset_0`
/// absolute element carrying the bar. Append it as the *last* child of the
/// container's `relative` wrapper — the wrapper takes over the container's
/// outer sizing (`flex_1`/`min_h_0`/…), the scroll container fills it with
/// `w_full().h_full()`. The bar hides itself while the content fits, so a
/// container with both axes enabled can carry a both-axis overlay and only
/// ever show the axis that actually overflows.
pub fn overlay_scrollbar(
    handle: &ScrollHandle,
    id: impl Into<ElementId>,
    axis: ScrollbarAxis,
) -> impl IntoElement {
    div().id(id).gpui_test_observe().absolute().inset_0().child(
        // The bar's own element state keys off the full ancestry path,
        // which the overlay's unique id above already makes distinct per
        // container.
        Scrollbar::new(handle)
            .axis(axis)
            .mode(ScrollbarMode::Scrolling)
            .viewport_from_layout()
            .styles(scrollbar_styles()),
    )
}

/// Offset clamped against this frame's content size and the handle's
/// measured viewport — the same clamp a scroll container applies at its own
/// prepaint. Render-side consumers that position content from the offset
/// (the matrix page's frozen headers) must read the clamped value: the
/// container clamps only at prepaint, *after* render, so an unclamped read
/// overshoots past the scroll edge and snaps back one frame later — the
/// header rebound this function exists to kill.
pub fn clamped_offset(handle: &ScrollHandle, content: Size<Pixels>) -> Point<Pixels> {
    let viewport = handle.bounds().size;
    let offset = handle.offset();
    point(
        offset
            .x
            .clamp(-(content.width - viewport.width).max(px(0.)), px(0.)),
        offset
            .y
            .clamp(-(content.height - viewport.height).max(px(0.)), px(0.)),
    )
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests {
    use super::overlay_scrollbar;
    use gpui_kit::InteractiveElement;
    use gpui_kit::StatefulInteractiveElement;
    use gpui_kit::base::ScrollbarAxis;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        Context, IntoElement, ParentElement, Render, ScrollDelta, ScrollHandle, SharedString,
        Styled, Window, div, point, px,
    };

    use crate::ui::observe::TestObserve;

    /// Ten 50px rows in a 100px viewport: 400px of vertical overflow.
    struct SpikeView {
        handle: ScrollHandle,
        rows: usize,
    }

    impl Render for SpikeView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().p(px(10.)).child(
                // The load-bearing structure under test: the scroll area
                // and the bar overlay are siblings under one relative
                // wrapper.
                div()
                    .relative()
                    .w(px(100.))
                    .h(px(100.))
                    .child(
                        div()
                            .id("spike-area")
                            .gpui_test_observe()
                            .w_full()
                            .h_full()
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&self.handle)
                            .children((0..self.rows).map(|ix| {
                                div()
                                    .id(SharedString::from(format!("spike-row-{ix}")))
                                    .gpui_test_observe()
                                    .flex_none()
                                    .w_full()
                                    .h(px(50.))
                            })),
                    )
                    .child(overlay_scrollbar(
                        &self.handle,
                        "scrollbar-spike-area",
                        ScrollbarAxis::Vertical,
                    )),
            )
        }
    }

    #[gpui_kit::test]
    fn bar_overlay_stays_pinned_while_content_scrolls(cx: &mut gpui_kit::TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| SpikeView {
            handle: ScrollHandle::new(),
            rows: 10,
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            let area = window.find("spike-area").bounds();
            assert_eq!(
                window.find("scrollbar-spike-area").bounds(),
                area,
                "the overlay must pin to the container exactly"
            );

            // Wheel down 300px of the 400px overflow: the content moves, the
            // overlay does not.
            window.scroll(
                "spike-area",
                ScrollDelta::Pixels(point(px(0.), px(-300.))),
                cx,
            );
            window.render_frame(cx);
            // During development an observed overlay could lose its frame
            // registration across a scroll; wait a few frames for the
            // observation before measuring so the test reports drift, not a
            // registration gap.
            for _ in 0..4 {
                if window.try_find("scrollbar-spike-area").is_some() {
                    break;
                }
                window.render_frame(cx);
            }
            let area = window.find("spike-area").bounds();
            let last = window.find("spike-row-9").bounds();
            assert_eq!(last.top(), area.top() + px(450. - 300.));
            assert_eq!(window.find("scrollbar-spike-area").bounds(), area);

            // Overscroll: the container clamps at its prepaint, so the last
            // row's bottom lands on the container's bottom edge. Render-side
            // offset readers must clamp the same way (`clamped_offset`).
            window.scroll(
                "spike-area",
                ScrollDelta::Pixels(point(px(0.), px(-700.))),
                cx,
            );
            window.render_frame(cx);
            let area = window.find("spike-area").bounds();
            let last = window.find("spike-row-9").bounds();
            assert_eq!(last.bottom(), area.bottom());
            assert_eq!(window.find("scrollbar-spike-area").bounds(), area);
        });
    }

    #[gpui_kit::test]
    fn bar_overlay_leaves_short_content_untouched(cx: &mut gpui_kit::TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| SpikeView {
            handle: ScrollHandle::new(),
            rows: 1,
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            let first = window.find("spike-row-0").bounds();
            window.scroll(
                "spike-area",
                ScrollDelta::Pixels(point(px(0.), px(-300.))),
                cx,
            );
            window.render_frame(cx);
            assert_eq!(window.find("spike-row-0").bounds(), first);
        });
    }
}
