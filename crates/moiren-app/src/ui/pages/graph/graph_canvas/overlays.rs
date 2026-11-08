//! HUD overlays above the canvas, and the geometry that keeps the hot meter
//! layer from painting over them.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::{Bounds, IntoElement, Pixels, Styled, canvas, point, px, size};

/// Window rects of the overlays a graph page paints above its canvas: the
/// editing toolbar, the overview, the library's edge handle. They live
/// inside the cached panel, so the hot meter layer — a later sibling —
/// would otherwise paint live bars over them. The panel's bounds probes
/// register them during its prepaint (a replayed frame keeps the last
/// values, which stay valid until the next rebuild re-lays them out); the
/// hot layer reads them at paint time and carves its masks around them.
#[derive(Clone, Default)]
pub struct OverlayRects(Rc<RefCell<BTreeMap<&'static str, Bounds<Pixels>>>>);

impl OverlayRects {
    pub fn register(&self, slot: &'static str, bounds: Bounds<Pixels>) {
        self.0.borrow_mut().insert(slot, bounds);
    }

    /// Every registered overlay rect, grown past the overlay's hairline
    /// border (the probe measures the padding box) with a pixel to spare,
    /// so a bar never touches the overlay.
    pub fn rects(&self) -> Vec<Bounds<Pixels>> {
        self.0
            .borrow()
            .values()
            .map(|bounds| Bounds {
                origin: bounds.origin - point(px(2.), px(2.)),
                size: bounds.size + size(px(4.), px(4.)),
            })
            .collect()
    }

    /// A painting-free, hitbox-free probe that records its parent's rect
    /// under `slot`; mount it as a child of the overlay, which must be
    /// positioned so the probe's full size resolves against it.
    pub fn probe(&self, slot: &'static str) -> impl IntoElement {
        let rects = self.clone();
        canvas(
            move |bounds, _, _| rects.register(slot, bounds),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

/// `rect` minus every hole, as the up-to-four strips each subtraction
/// leaves (above, below, left of and right of the hole's overlap). Pieces
/// never overlap, so painting the same content once per piece under that
/// piece's mask covers every pixel outside the holes exactly once.
pub fn carve(rect: Bounds<Pixels>, holes: &[Bounds<Pixels>]) -> Vec<Bounds<Pixels>> {
    let mut pieces = vec![rect];
    for hole in holes {
        pieces = pieces
            .into_iter()
            .flat_map(|piece| subtract(piece, *hole))
            .collect();
    }
    pieces
}

fn subtract(piece: Bounds<Pixels>, hole: Bounds<Pixels>) -> Vec<Bounds<Pixels>> {
    let left = f32::from(piece.origin.x);
    let top = f32::from(piece.origin.y);
    let right = left + f32::from(piece.size.width);
    let bottom = top + f32::from(piece.size.height);
    let hole_left = f32::from(hole.origin.x).max(left);
    let hole_top = f32::from(hole.origin.y).max(top);
    let hole_right = (f32::from(hole.origin.x) + f32::from(hole.size.width)).min(right);
    let hole_bottom = (f32::from(hole.origin.y) + f32::from(hole.size.height)).min(bottom);
    if hole_left >= hole_right || hole_top >= hole_bottom {
        return vec![piece];
    }
    let strip = |l: f32, t: f32, r: f32, b: f32| Bounds {
        origin: point(px(l), px(t)),
        size: size(px(r - l), px(b - t)),
    };
    let mut pieces = Vec::with_capacity(4);
    if hole_top > top {
        pieces.push(strip(left, top, right, hole_top));
    }
    if hole_bottom < bottom {
        pieces.push(strip(left, hole_bottom, right, bottom));
    }
    if hole_left > left {
        pieces.push(strip(left, hole_top, hole_left, hole_bottom));
    }
    if hole_right < right {
        pieces.push(strip(hole_right, hole_top, right, hole_bottom));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Bounds, Pixels, point, px, size};

    use super::carve;

    fn rect(left: f32, top: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        }
    }

    fn area(pieces: &[Bounds<Pixels>]) -> f32 {
        pieces
            .iter()
            .map(|piece| f32::from(piece.size.width) * f32::from(piece.size.height))
            .sum()
    }

    /// Positive-area overlap; shared edges do not count.
    fn overlaps(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> bool {
        let (al, at) = (f32::from(a.origin.x), f32::from(a.origin.y));
        let (ar, ab) = (al + f32::from(a.size.width), at + f32::from(a.size.height));
        let (bl, bt) = (f32::from(b.origin.x), f32::from(b.origin.y));
        let (br, bb) = (bl + f32::from(b.size.width), bt + f32::from(b.size.height));
        al < br && bl < ar && at < bb && bt < ab
    }

    #[test]
    fn carve_leaves_the_rect_whole_when_no_hole_touches_it() {
        let card = rect(100., 100., 200., 80.);
        assert_eq!(carve(card, &[]), vec![card]);
        assert_eq!(carve(card, &[rect(0., 0., 50., 50.)]), vec![card]);
        // A hole sharing an edge is not an overlap either.
        assert_eq!(carve(card, &[rect(300., 100., 50., 80.)]), vec![card]);
    }

    #[test]
    fn carve_covers_everything_outside_the_holes_exactly_once() {
        let card = rect(100., 100., 200., 80.);
        // One hole straddling the top-left corner, one fully inside.
        let holes = [rect(80., 90., 60., 40.), rect(200., 120., 30., 20.)];
        let pieces = carve(card, &holes);
        let expected = 200. * 80. - 40. * 30. - 30. * 20.;
        assert!((area(&pieces) - expected).abs() < 1e-3, "{pieces:?}");
        for (index, piece) in pieces.iter().enumerate() {
            assert!(f32::from(piece.size.width) > 0. && f32::from(piece.size.height) > 0.);
            for hole in &holes {
                assert!(!overlaps(piece, hole), "{piece:?} overlaps {hole:?}");
            }
            for other in &pieces[index + 1..] {
                assert!(!overlaps(piece, other), "{piece:?} overlaps {other:?}");
            }
            assert!(
                f32::from(piece.origin.x) >= 100. && f32::from(piece.origin.y) >= 100.,
                "{piece:?} escapes the card"
            );
        }
    }

    #[test]
    fn carve_drops_a_rect_a_hole_swallows() {
        let card = rect(100., 100., 200., 80.);
        assert!(carve(card, &[rect(0., 0., 1000., 1000.)]).is_empty());
    }
}
