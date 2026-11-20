use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    App, BorderStyle, Bounds, Font, FontWeight, Hsla, PaintQuad, Pixels, ShapedLine, TextAlign,
    TextRun, Window, font, point, px,
};

use crate::ui::pages::graph::graph_canvas::{CanvasFrame, TextLayout};
use crate::ui::theme;

/// The canvas painters' shaping style: everything [`shape_line`] reads. The
/// cache keys hash this verbatim, so the fields are stored bit-exact.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub(super) struct ShapeStyle {
    size_bits: u32,
    family: &'static str,
    weight_bits: u32,
    color: [u32; 4],
}

impl ShapeStyle {
    pub(super) fn new(
        font_size: f32,
        family: &'static str,
        weight: FontWeight,
        color: Hsla,
    ) -> Self {
        Self {
            size_bits: font_size.to_bits(),
            family,
            weight_bits: weight.0.to_bits(),
            color: [
                color.h.to_bits(),
                color.s.to_bits(),
                color.l.to_bits(),
                color.a.to_bits(),
            ],
        }
    }

    fn font(&self) -> Font {
        Font {
            weight: FontWeight(f32::from_bits(self.weight_bits)),
            ..font(self.family)
        }
    }

    fn color(&self) -> Hsla {
        let [h, s, l, a] = self.color.map(f32::from_bits);
        Hsla { h, s, l, a }
    }
}

/// One text's shaped variants: the full line plus, per available width that
/// actually forced a truncation, the fitted line. Most labels never
/// overflow, so the truncation list stays empty; an overflowing label
/// reflows only when its width really changed (audit A-1: "宽度没变就不重排").
struct ShapeVariants {
    full: ShapedLine,
    truncated: Vec<(u32, ShapedLine)>,
}

/// The canvas text shaping cache (audit A-1). Card paints shape 2-6 labels
/// each, and the cached canvas re-paints on every rebuild while the hot
/// layer paints every frame — without a cache the same (text, style) lines
/// re-shape, and overflowing titles re-run the truncation binary search, on
/// every display frame. Keyed by the exact text, then by the style it is
/// shaped at, so the same label at two zooms or two colors costs two
/// entries and one lookup forever after — a line shaped at one size must
/// never serve another, or a zoomed canvas would paint the old zoom's
/// glyphs inside the new bounds.
#[derive(Default)]
pub(in crate::ui) struct CanvasShapes {
    lines: HashMap<Arc<str>, HashMap<ShapeStyle, ShapeVariants>>,
    entries: usize,
    /// Test census: real shaper runs (full lines and truncation probes).
    #[cfg(feature = "gpui-test")]
    pub(in crate::ui) shapes: usize,
}

/// Far above the honest (text, style) population — every card's label at
/// every zoom the session actually visits; a pathological feed clears the
/// cache and pays reshapes again instead of growing without bound.
const CANVAS_SHAPE_CACHE_CAP: usize = 4096;

/// Shell-owned handle shared by the cached canvas paint and the hot layer's
/// paints, so a label shaped once is reused by both.
pub(in crate::ui) type CanvasShapesHandle = Rc<RefCell<CanvasShapes>>;

/// One paint session: the shaping cache shared with the sibling hot layer
/// plus the window the painter draws into. Painters take it as one bundle —
/// the argument set otherwise outgrows the lint budget — and destructure it
/// once; interior painters re-borrow per sub-paint.
pub(in crate::ui) struct Paint<'a> {
    pub(in crate::ui) shapes: &'a mut CanvasShapes,
    pub(in crate::ui) window: &'a mut Window,
}

/// Far above the honest truncation widths per text (zoom steps); same
/// cap-and-clear policy as the outer map.
const TRUNCATED_VARIANTS_CAP: usize = 32;

impl CanvasShapes {
    fn shape(&mut self, window: &Window, text: &str, style: ShapeStyle) -> ShapedLine {
        #[cfg(feature = "gpui-test")]
        {
            self.shapes += 1;
        }
        window.text_system().shape_line(
            text.to_owned().into(),
            px(f32::from_bits(style.size_bits)),
            &[TextRun {
                len: text.len(),
                font: style.font(),
                color: style.color(),
                ..TextRun::default()
            }],
            None,
        )
    }

    /// The shaped line for `text` at `style`, running the shaper only on a
    /// cache miss. The returned line borrows the cache; paint it before the
    /// next lookup.
    pub(super) fn full(&mut self, window: &Window, text: &str, style: ShapeStyle) -> &ShapedLine {
        if self.variants(text, style).is_none() {
            if self.entries >= CANVAS_SHAPE_CACHE_CAP {
                self.lines.clear();
                self.entries = 0;
            }
            let line = self.shape(window, text, style);
            self.lines
                .entry(Arc::from(text))
                .or_default()
                .entry(style)
                .or_insert_with(|| ShapeVariants {
                    full: line,
                    truncated: Vec::new(),
                });
            self.entries += 1;
        }
        &self.lines[text][&style].full
    }

    /// The cached variants for `text` at `style`, if a line was shaped for
    /// this exact pair before.
    fn variants(&self, text: &str, style: ShapeStyle) -> Option<&ShapeVariants> {
        self.lines.get(text).and_then(|styles| styles.get(&style))
    }

    /// The widest line for `text` at `style` that fits `available`, falling
    /// back to the cached full line when it already fits. A truncation is
    /// cached per available width within the (text, style) entry, so an
    /// overflowing label re-runs the binary search only when its width
    /// changed at that style.
    pub(super) fn fitted(
        &mut self,
        window: &Window,
        text: &str,
        style: ShapeStyle,
        available: f32,
    ) -> &ShapedLine {
        // Sized first, without holding the cache borrow across the miss
        // path's mutations (a full-line lookup on the hit path is one map
        // probe, never a shaper run).
        let overflows = f32::from(self.full(window, text, style).width()) > available;
        if !overflows {
            return self.full(window, text, style);
        }
        let key = available.to_bits();
        let cached = self
            .variants_mut(text, style)
            .and_then(|variants| variants.truncated.iter().position(|(bits, _)| *bits == key));
        if let Some(position) = cached {
            return &self.lines[text][&style].truncated[position].1;
        }
        if let Some(variants) = self.variants_mut(text, style)
            && variants.truncated.len() >= TRUNCATED_VARIANTS_CAP
        {
            variants.truncated.clear();
        }
        let line = self.truncate(window, text, style, available);
        let variants = self
            .variants_mut(text, style)
            .expect("full() inserted the text");
        variants.truncated.push((key, line));
        &variants.truncated.last().expect("just pushed").1
    }

    fn variants_mut(&mut self, text: &str, style: ShapeStyle) -> Option<&mut ShapeVariants> {
        self.lines
            .get_mut(text)
            .and_then(|styles| styles.get_mut(&style))
    }

    /// The widest prefix of `text` — through the ellipsis — that fits
    /// `available`, by binary search over character boundaries. One shaper
    /// run per candidate; the census counts each so tests can watch the
    /// cache absorb them.
    fn truncate(
        &mut self,
        window: &Window,
        text: &str,
        style: ShapeStyle,
        available: f32,
    ) -> ShapedLine {
        #[cfg_attr(not(feature = "gpui-test"), allow(unused_mut))]
        let mut shape = |probe: &str| {
            #[cfg(feature = "gpui-test")]
            {
                self.shapes += 1;
            }
            window.text_system().shape_line(
                probe.to_owned().into(),
                px(f32::from_bits(style.size_bits)),
                &run(probe, style.font(), style.color()),
                None,
            )
        };
        let boundaries = text
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let mut lower = 0;
        let mut upper = boundaries.len();
        let mut best = shape("");
        while lower < upper {
            let middle = lower + (upper - lower) / 2;
            let candidate = shape(&format!("{}…", &text[..boundaries[middle]]));
            if f32::from(candidate.width()) <= available {
                best = candidate;
                lower = middle + 1;
            } else {
                upper = middle;
            }
        }
        best
    }
}

fn run(text: &str, font: Font, color: Hsla) -> [TextRun; 1] {
    [TextRun {
        len: text.len(),
        font,
        color,
        ..TextRun::default()
    }]
}

pub(super) struct Label<'a> {
    pub(super) text: &'a str,
    pub(super) family: &'static str,
    pub(super) color: Hsla,
    pub(super) right_aligned: bool,
    pub(super) weight: FontWeight,
}

pub(super) fn paint_label(
    frame: &CanvasFrame,
    layout: &TextLayout,
    label: Label<'_>,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let rect = frame.window_rect(layout.bounds);
    let line = shapes.fitted(
        window,
        label.text,
        ShapeStyle::new(layout.font_size, label.family, label.weight, label.color),
        layout.bounds.size.width,
    );
    let left = if label.right_aligned {
        rect.origin.x + rect.size.width - line.width()
    } else {
        rect.origin.x
    };
    let _ = line.paint(
        point(left, rect.origin.y),
        px(layout.line_height),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

pub(super) fn rounded(bounds: Bounds<Pixels>, color: Hsla, radius: f32, window: &mut Window) {
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: radius.into(),
        background: color.into(),
        border_widths: 0f32.into(),
        border_color: theme::controls::TRANSPARENT,
        border_style: BorderStyle::default(),
    });
}

/// A single unwrapped line of UI text, anchored at its center or its right
/// edge: the spectrum plot's tick labels sit on their grid lines.
pub(super) fn paint_anchored_text(
    text: &str,
    font_size: f32,
    color: Hsla,
    anchor: (f32, f32),
    right_aligned: bool,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let line = shapes.full(
        window,
        text,
        ShapeStyle::new(font_size, theme::FONT_UI, FontWeight::NORMAL, color),
    );
    let width = f32::from(line.width());
    let left = if right_aligned {
        anchor.0 - width
    } else {
        anchor.0 - width / 2.
    };
    let _ = line.paint(
        point(px(left), px(anchor.1)),
        px(font_size + 2.),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// One straight hairline between two window points.
pub(super) fn hairline(
    from: (f32, f32),
    to: (f32, f32),
    stroke: f32,
    color: Hsla,
    window: &mut Window,
) {
    let mut builder = gpui_kit::PathBuilder::stroke(px(stroke));
    builder.move_to(point(px(from.0), px(from.1)));
    builder.line_to(point(px(to.0), px(to.1)));
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}
