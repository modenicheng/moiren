use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    App, Axis, BorderStyle, Bounds, ContentMask, Font, FontWeight, Hsla, PaintQuad, Path,
    PathBuilder, Pixels, ShapedLine, TextAlign, TextRun, TransformationMatrix, Window, font, point,
    px, size,
};
use rust_i18n::t;

use super::super::graph_canvas::{
    BindingChip, CanvasFrame, CanvasNode, CardFault, CardKnob, ChipLayout, CurveSpec, KnobLayout,
    MeterLayout, NodeLayout, TextLayout, curves,
};
use crate::ui::monitoring::{REST_DB, SPECTRUM_NOMINAL_SPAN_HZ, SpectrumCurve};
use crate::ui::theme;

/// The canvas painters' shaping style: everything [`shape_line`] reads. The
/// cache keys hash this verbatim, so the fields are stored bit-exact.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct ShapeStyle {
    size_bits: u32,
    family: &'static str,
    weight_bits: u32,
    color: [u32; 4],
}

impl ShapeStyle {
    fn new(font_size: f32, family: &'static str, weight: FontWeight, color: Hsla) -> Self {
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
/// every display frame. Keyed first by the exact text, then by style, so
/// the same label at two zooms or two colors costs two entries and one
/// lookup forever after. Far above the honest population of card labels;
/// exceeding the cap clears the cache rather than growing without bound.
#[derive(Default)]
pub(in crate::ui) struct CanvasShapes {
    lines: HashMap<Arc<str>, ShapeVariants>,
    /// Test census: real shaper runs (full lines and truncation probes).
    #[cfg(feature = "gpui-test")]
    pub(in crate::ui) shapes: usize,
}

/// Far above the honest text population (every card's labels at the zooms
/// the session actually uses); a pathological feed clears the cache and
/// pays reshapes again instead of growing without bound.
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
    fn full(&mut self, window: &Window, text: &str, style: ShapeStyle) -> &ShapedLine {
        if !self.lines.contains_key(text) {
            if self.lines.len() >= CANVAS_SHAPE_CACHE_CAP {
                self.lines.clear();
            }
            let line = self.shape(window, text, style);
            self.lines.insert(
                Arc::from(text),
                ShapeVariants {
                    full: line,
                    truncated: Vec::new(),
                },
            );
        }
        &self.lines[text].full
    }

    /// The widest line for `text` at `style` that fits `available`, falling
    /// back to the cached full line when it already fits. A truncation is
    /// cached per available width, so an overflowing label re-runs the
    /// binary search only when its width changed.
    fn fitted(
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
            .lines
            .get_mut(text)
            .and_then(|variants| variants.truncated.iter().position(|(bits, _)| *bits == key));
        if let Some(position) = cached {
            return &self.lines[text].truncated[position].1;
        }
        if let Some(variants) = self.lines.get_mut(text)
            && variants.truncated.len() >= TRUNCATED_VARIANTS_CAP
        {
            variants.truncated.clear();
        }
        let line = self.truncate(window, text, style, available);
        let variants = self.lines.get_mut(text).expect("full() inserted the text");
        variants.truncated.push((key, line));
        &variants.truncated.last().expect("just pushed").1
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

struct Label<'a> {
    text: &'a str,
    family: &'static str,
    color: Hsla,
    right_aligned: bool,
    weight: FontWeight,
}

fn paint_label(
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

fn rounded(bounds: Bounds<Pixels>, color: Hsla, radius: f32, window: &mut Window) {
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: radius.into(),
        background: color.into(),
        border_widths: 0f32.into(),
        border_color: theme::controls::TRANSPARENT,
        border_style: BorderStyle::default(),
    });
}

/// One node card's meter tracks and live bars, in window space. The zone
/// segmentation, dark RMS tail, held marker, and clip latch are the same
/// `meter_spans` math every other meter surface paints (design 17 D1); only
/// the surface differs. Shared by the cached canvas paint and the hot
/// layer's overlay paint so both draw identical geometry; callers own the
/// card content mask. Curve nodes run their meters vertically along the
/// body's right edge, everything else as a horizontal strip under the
/// body (design 18 B2).
pub(in crate::ui) fn paint_meter_bars(
    frame: &CanvasFrame,
    layout: &NodeLayout,
    reading: Option<&super::NodeMeterReading>,
    window: &mut Window,
) {
    let stride = (theme::card::METER_H + theme::card::METER_CHANNEL_GAP) * layout.scale;
    for channel in 0..theme::card::METER_CHANNELS {
        // The clip lamp reserves the last 3 world units of the horizontal
        // row's loud end — the same loud-end indicator every meter surface
        // uses; the fill never changes background for clipping. Vertical
        // tracks run the body's full height instead, so their bottom edge
        // aligns with the plot below it.
        let (track, axis) = match layout.meter {
            MeterLayout::Horizontal { bounds } => {
                let mut meter = bounds;
                meter.origin.y += channel as f32 * stride;
                meter.size.width -= 3. * layout.scale;
                (meter, Axis::Horizontal)
            }
            MeterLayout::Vertical { bounds } => {
                let mut meter = bounds;
                meter.origin.x -= channel as f32 * stride;
                (meter, Axis::Vertical)
            }
        };
        let track = frame.window_rect(track);
        let clip = reading.is_some_and(|reading| reading.clips[channel]);
        rounded(track, theme::WINDOW_BG, layout.scale, window);
        if let Some(level) = reading.and_then(|reading| reading.levels.get(channel)) {
            match axis {
                Axis::Horizontal => {
                    crate::ui::widgets::meters::paint_meter_fill_horizontal(
                        track,
                        *level,
                        theme::WINDOW_BG,
                        window,
                    );
                }
                _ => {
                    crate::ui::widgets::meters::paint_meter_fill(
                        track,
                        *level,
                        theme::WINDOW_BG,
                        window,
                    );
                }
            }
        }
        crate::ui::widgets::meters::paint_clip_lamp(track, axis, clip, window);
    }
}

/// A filled pill between two points: straight sides plus semicircular
/// caps. gpui's stroke builder has no reachable line-cap option, so pill
/// ends are built as geometry.
fn capsule_path(a: (f32, f32), b: (f32, f32), half: f32) -> anyhow::Result<Path<Pixels>> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx.hypot(dy);
    if length < 1e-3 {
        anyhow::bail!("degenerate capsule");
    }
    let (ux, uy) = (dx / length, dy / length);
    // Walk the +normal side a -> b, round the far end through the forward
    // direction (normal - 90°), come back along the -normal side, and
    // round the near end through the backward direction.
    let normal = ux.atan2(-uy).to_degrees();
    let mut builder = PathBuilder::fill();
    builder.move_to(point(px(a.0 - uy * half), px(a.1 + ux * half)));
    builder.line_to(point(px(b.0 - uy * half), px(b.1 + ux * half)));
    cap(&mut builder, b, half, normal, normal - 180.);
    builder.line_to(point(px(a.0 + uy * half), px(a.1 - ux * half)));
    cap(&mut builder, a, half, normal + 180., normal);
    builder.build()
}

/// A filled ring segment with pill ends, `half` thick either side of
/// `radius`, swept between two screen angles in degrees (y down, so 90°
/// points at the bottom).
fn arc_capsule_path(
    center: (f32, f32),
    radius: f32,
    half: f32,
    from_deg: f32,
    to_deg: f32,
) -> anyhow::Result<Path<Pixels>> {
    let (from, to) = if to_deg >= from_deg {
        (from_deg, to_deg)
    } else {
        (to_deg, from_deg)
    };
    if to - from < 0.5 {
        anyhow::bail!("degenerate arc");
    }
    let steps = ((to - from) / 6.).ceil().max(2.) as usize;
    let on_ring = |angle_deg: f32, ring: f32| {
        let angle = angle_deg.to_radians();
        (center.0 + angle.cos() * ring, center.1 + angle.sin() * ring)
    };
    let mut builder = PathBuilder::fill();
    for step in 0..=steps {
        let angle = from + (to - from) * step as f32 / steps as f32;
        let (x, y) = on_ring(angle, radius + half);
        if step == 0 {
            builder.move_to(point(px(x), px(y)));
        } else {
            builder.line_to(point(px(x), px(y)));
        }
    }
    cap(&mut builder, on_ring(to, radius), half, to, to + 180.);
    for step in 0..=steps {
        let angle = to - (to - from) * step as f32 / steps as f32;
        let (x, y) = on_ring(angle, radius - half);
        builder.line_to(point(px(x), px(y)));
    }
    cap(
        &mut builder,
        on_ring(from, radius),
        half,
        from + 180.,
        from + 360.,
    );
    builder.build()
}

/// Half-circle cap around `center` from one screen angle to another, as
/// line segments appended to a path under construction.
fn cap(builder: &mut PathBuilder, center: (f32, f32), half: f32, from_deg: f32, to_deg: f32) {
    const STEPS: usize = 8;
    for step in 1..=STEPS {
        let angle = (from_deg + (to_deg - from_deg) * step as f32 / STEPS as f32).to_radians();
        builder.line_to(point(
            px(center.0 + angle.cos() * half),
            px(center.1 + angle.sin() * half),
        ));
    }
}

/// One knob dial, parameterized by the sweep it turns through and where
/// its value arc anchors: a neutral pill-ended track over the whole sweep,
/// the value portion filled in `TRACK_FILL` from the control's origin to
/// its position (unipolar knobs fill from the floor, bipolar ones from
/// their neutral point, in whichever direction the value lies), and a
/// slim pill pointer at the value angle. Screen angles run from
/// `DIAL_SWEEP_START` with y pointing down, so the sweep starts bottom-
/// left, passes the top, and ends bottom-right.
fn paint_knob(
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

/// A static curve thumbnail: baseline, the sampled curve, and a
/// center/threshold tick. The same math the detail window paints, at
/// card scale.
fn paint_curve_thumb(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    spec: &CurveSpec,
    scale: f32,
    window: &mut Window,
) {
    let plot = frame.window_rect(bounds);
    let width = f32::from(plot.size.width);
    let height = f32::from(plot.size.height);
    let origin_x = f32::from(plot.origin.x);
    let origin_y = f32::from(plot.origin.y);
    fn line(
        origin_x: f32,
        origin_y: f32,
        from: (f32, f32),
        to: (f32, f32),
        stroke: f32,
        color: Hsla,
        window: &mut Window,
    ) {
        let mut builder = gpui_kit::PathBuilder::stroke(px(stroke));
        builder.move_to(point(px(origin_x + from.0), px(origin_y + from.1)));
        builder.line_to(point(px(origin_x + to.0), px(origin_y + to.1)));
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }
    fn stroke_points(
        origin_x: f32,
        origin_y: f32,
        scale: f32,
        samples: &[(f64, f64)],
        x_of: &dyn Fn(f64) -> f32,
        y_of: &dyn Fn(f64) -> f32,
        window: &mut Window,
    ) {
        let mut builder = gpui_kit::PathBuilder::stroke(px(1.5 * scale));
        let mut first = true;
        for (x, y) in samples {
            let p = point(px(origin_x + x_of(*x)), px(origin_y + y_of(*y)));
            if first {
                builder.move_to(p);
                first = false;
            } else {
                builder.line_to(p);
            }
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, theme::TRACK_FILL);
        }
    }
    match spec {
        CurveSpec::Eq(settings) => {
            let x_of = |freq: f64| ((freq / 20.).ln() / 1000f64.ln()) as f32 * width;
            let y_of = |db: f64| height / 2. - (db as f32 / 24.) * height / 2.;
            line(
                origin_x,
                origin_y,
                (0., height / 2.),
                (width, height / 2.),
                scale,
                theme::HAIRLINE,
                window,
            );
            let center = x_of(settings.frequency_hz);
            line(
                origin_x,
                origin_y,
                (center, 0.),
                (center, height),
                scale,
                theme::BORDER_STRONG,
                window,
            );
            let samples = curves::sample_curve_log_x(
                |freq| curves::eq_magnitude_db(settings, 48_000., freq),
                20.,
                20_000.,
                64,
            );
            stroke_points(origin_x, origin_y, scale, &samples, &x_of, &y_of, window);
        }
        CurveSpec::Dynamics(curve) => {
            let span = 60.;
            let x_of = |db: f64| ((db + span) / (2. * span)) as f32 * width;
            let y_of = |db: f64| height - ((db + span) / (2. * span)) as f32 * height;
            line(
                origin_x,
                origin_y,
                (0., height),
                (width, 0.),
                scale,
                theme::HAIRLINE,
                window,
            );
            let threshold = x_of(curve.threshold_db);
            line(
                origin_x,
                origin_y,
                (threshold, 0.),
                (threshold, height),
                scale,
                theme::BORDER_STRONG,
                window,
            );
            let samples =
                curves::sample_curve_linear_x(|db| curve.transfer_db(db), -span, span, 64);
            stroke_points(origin_x, origin_y, scale, &samples, &x_of, &y_of, window);
        }
    }
}

/// One straight hairline between two window points.
fn hairline(from: (f32, f32), to: (f32, f32), stroke: f32, color: Hsla, window: &mut Window) {
    let mut builder = PathBuilder::stroke(px(stroke));
    builder.move_to(point(px(from.0), px(from.1)));
    builder.line_to(point(px(to.0), px(to.1)));
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// A single unwrapped line of UI text, anchored at its center or its right
/// edge: the spectrum plot's tick labels sit on their grid lines.
fn paint_anchored_text(
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

/// The spectrum analyzer's body: a log-frequency / dBFS plot with its axis
/// furniture and the live curve, painted whole by the cached canvas (one
/// snapshot per rebuild) and by the hot layer (every frame), the way the
/// meter bars are. The axis follows the curve's own span — the ISO grid
/// clamped to the running Nyquist — and falls back to the nominal
/// 20 Hz .. 20 kHz band before the first window lands. Frequency grid
/// lines sit logarithmically, like graph paper — strong at each power of
/// ten, faint at every integer step inside the decade — and every 24 dB;
/// the curve fills down to the floor under a stroke in the card's tint.
pub(in crate::ui) fn paint_spectrum_body(
    frame: &CanvasFrame,
    layout: &NodeLayout,
    detail: &str,
    tint: Hsla,
    curve: Option<SpectrumCurve<'_>>,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let Some(bounds) = layout.spectrum else {
        return;
    };
    let scale = layout.scale;
    let plot = frame.window_rect(bounds);
    let left = f32::from(plot.origin.x);
    let top = f32::from(plot.origin.y);
    let width = f32::from(plot.size.width);
    let height = f32::from(plot.size.height);
    if width <= 0. || height <= 0. {
        return;
    }
    rounded(plot, theme::WINDOW_BG, 3. * scale, window);
    let (lo_hz, hi_hz) = curve
        .map(|curve| (curve.lo_hz, curve.hi_hz))
        .filter(|(lo, hi)| hi > lo)
        .unwrap_or(SPECTRUM_NOMINAL_SPAN_HZ);
    let span = hi_hz.log2() - lo_hz.log2();
    let x_of = |hz: f64| left + ((hz.log2() - lo_hz.log2()) / span) as f32 * width;
    // 0 dBFS sits a hair under the top edge; the floor is the plot bottom.
    let headroom = 2. * scale;
    let y_of = |db: f32| top + headroom + (db / REST_DB) * (height - headroom);
    let stroke = scale.max(0.75);
    for db in [-24., -48., -72.] {
        let y = y_of(db);
        hairline(
            (left, y),
            (left + width, y),
            stroke,
            theme::HAIRLINE,
            window,
        );
    }
    // The frequency grid: logarithmic graph paper — a strong line at every
    // power of ten, faint ones at each integer step inside the decade,
    // bunching toward the decade's end. The 1-2-5 series this replaces is
    // log-spaced too, but its repeating 1.32/1/1-octave pattern reads as
    // uniform — not the axis the reference analyzers background.
    let first_decade = 10f64.powi(lo_hz.log10().floor() as i32);
    let decades = ((hi_hz / first_decade).log10().floor() as i32 + 1).max(1);
    for decade in 0..decades {
        let base = first_decade * 10f64.powi(decade);
        for mantissa in 1..=9u32 {
            let hz = base * f64::from(mantissa);
            if hz < lo_hz || hz > hi_hz {
                continue;
            }
            let x = x_of(hz);
            let color = if mantissa == 1 {
                theme::BORDER_STRONG
            } else {
                theme::HAIRLINE
            };
            hairline((x, top), (x, top + height), stroke, color, window);
        }
    }
    // Decade labels along the floor, the FFT detail in the top-right corner.
    let label_size = theme::card::HINT_SIZE * scale;
    for (hz, text) in [(100., "100"), (1_000., "1k"), (10_000., "10k")] {
        if hz < lo_hz || hz > hi_hz {
            continue;
        }
        paint_anchored_text(
            text,
            label_size,
            theme::TEXT_FAINT,
            (x_of(hz), top + height - label_size - 4. * scale),
            false,
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
    }
    paint_anchored_text(
        detail,
        label_size,
        theme::TEXT_FAINT,
        (left + width - 4. * scale, top + 3. * scale),
        true,
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
    // The curve: a straight polyline through the grid points — the trace
    // the reference analyzers draw (REW, ARTA, SPAN), whose smoothness
    // comes from the band-power projection, never from the renderer:
    // spline handles overshoot at sharp peaks and would display levels
    // the spectrum does not contain. Masked to the plot so the stroke's
    // width stays inside the frame; nothing while parked at the floor.
    let Some(curve) = curve else {
        return;
    };
    let count = curve.points.len();
    if count < 2 || curve.points.iter().all(|point| *point <= REST_DB) {
        return;
    }
    let step = width / (count - 1) as f32;
    let floor = top + height;
    let at = |index: usize| {
        let index = index.min(count - 1);
        (
            left + index as f32 * step,
            y_of(curve.points[index].clamp(REST_DB, 0.)),
        )
    };
    let mut fill = PathBuilder::fill();
    let mut outline = PathBuilder::stroke(px(1.5 * scale));
    let start = at(0);
    fill.move_to(point(px(left), px(floor)));
    fill.line_to(point(px(start.0), px(start.1)));
    outline.move_to(point(px(start.0), px(start.1)));
    for index in 1..count {
        let next = at(index);
        let vertex = point(px(next.0), px(next.1));
        fill.line_to(vertex);
        outline.line_to(vertex);
    }
    fill.line_to(point(px(left + width), px(floor)));
    window.with_content_mask(Some(ContentMask { bounds: plot }), |window| {
        if let Ok(path) = fill.build() {
            window.paint_path(path, theme::halo(tint));
        }
        if let Ok(path) = outline.build() {
            window.paint_path(path, tint);
        }
    });
}

/// A binding chip: what this source/sink card is wired to. Unbound chips
/// sit quieter — surface fill, faint text — so the missing binding reads
/// as a prompt rather than an error.
fn paint_binding_chip(
    frame: &CanvasFrame,
    layout: &ChipLayout,
    chip: &BindingChip,
    tint: Hsla,
    scale: f32,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let bounds = frame.window_rect(layout.bounds);
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: (theme::RADIUS_SM * scale).into(),
        background: if chip.unbound {
            theme::SURFACE_4.into()
        } else {
            theme::SURFACE.into()
        },
        border_widths: 0f32.into(),
        border_color: theme::controls::TRANSPARENT,
        border_style: BorderStyle::default(),
    });
    let _ = window.paint_svg(
        frame.window_rect(layout.icon),
        format!("icons/{}.svg", chip.icon).into(),
        None,
        TransformationMatrix::unit(),
        if chip.unbound {
            theme::TEXT_FAINT
        } else {
            tint
        },
        cx,
    );
    paint_label(
        frame,
        &layout.label,
        Label {
            text: &chip.label,
            family: theme::FONT_UI,
            color: if chip.unbound {
                theme::TEXT_FAINT
            } else {
                theme::TEXT
            },
            right_aligned: false,
            weight: FontWeight::NORMAL,
        },
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
    let _ = window.paint_svg(
        frame.window_rect(layout.chevron),
        "icons/chevron-down.svg".into(),
        None,
        TransformationMatrix::unit(),
        theme::TEXT_FAINT,
        cx,
    );
}

/// The channel-mapper preview: one intensity cell per weight, tint-filled
/// over a dark grid so identity reads as a diagonal.
fn paint_matrix_preview(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    matrix: &[Vec<f64>],
    tint: Hsla,
    scale: f32,
    window: &mut Window,
) {
    let cell = (theme::card::MATRIX_CELL + theme::card::MATRIX_GAP) * scale;
    let grid = frame.window_rect(bounds);
    for (row, weights) in matrix.iter().enumerate().take(8) {
        for (col, weight) in weights.iter().enumerate().take(8) {
            let cell_bounds = Bounds {
                origin: point(
                    grid.origin.x + px(col as f32 * cell),
                    grid.origin.y + px(row as f32 * cell),
                ),
                size: size(
                    px(theme::card::MATRIX_CELL * scale),
                    px(theme::card::MATRIX_CELL * scale),
                ),
            };
            rounded(cell_bounds, theme::WINDOW_BG, 1.5 * scale, window);
            let intensity = (weight.abs().min(1.) as f32) * 0.85 + 0.15;
            rounded(cell_bounds, tint.opacity(intensity), 1.5 * scale, window);
        }
    }
}

/// The rack chain: one glyph per stage in signal order, with a count when
/// the chain outgrows the body.
fn paint_rack_chain(
    frame: &CanvasFrame,
    bounds: Bounds<f32>,
    stages: &[&'static str],
    tint: Hsla,
    scale: f32,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let grid = frame.window_rect(bounds);
    let icon = 14. * scale;
    let stride = 20. * scale;
    let top = grid.origin.y + px((f32::from(grid.size.height) - icon) / 2.);
    let shown = stages.len().min(12);
    for (index, stage) in stages.iter().take(shown).enumerate() {
        let _ = window.paint_svg(
            Bounds {
                origin: point(grid.origin.x + px(index as f32 * stride), top),
                size: size(px(icon), px(icon)),
            },
            format!("icons/{stage}.svg").into(),
            None,
            TransformationMatrix::unit(),
            tint,
            cx,
        );
    }
    if stages.len() > shown {
        let label = format!("+{}", stages.len() - shown);
        let line = shapes.fitted(
            window,
            &label,
            ShapeStyle::new(
                theme::TEXT_XS * scale,
                theme::FONT_MONO,
                FontWeight::NORMAL,
                theme::TEXT_FAINT,
            ),
            40. * scale,
        );
        let _ = line.paint(
            point(grid.origin.x + px(shown as f32 * stride), top),
            px(theme::TEXT_XS * scale + 2.),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

/// The fault tooltip under a hovered faulted card: every fault's cause
/// with its hint beneath, translated from the catalog keys the service
/// published exactly as the error strip does. Painted in window pixels —
/// a tooltip reads the same at every zoom — below the card, flipping
/// above it when the canvas bottom is too close, and above every card
/// because the hot layer paints it.
/// The faulted card's "!" badge in the reserved header slot: a danger-red
/// disc with a white glyph. The rect is camera-projected, so the badge
/// reads at every zoom like the outline does. Hit-tested as
/// `HitTarget::Badge`; clicking opens the fault detail popover.
fn paint_fault_badge(frame: &CanvasFrame, badge: &Bounds<f32>, paint: Paint<'_>, cx: &mut App) {
    let Paint { shapes, window } = paint;
    const GLYPH: f32 = theme::card::BADGE_SIZE * 0.72;
    let bounds = frame.window_rect(*badge);
    window.paint_quad(PaintQuad {
        bounds,
        corner_radii: (f32::from(bounds.size.width) / 2.).into(),
        background: theme::DANGER.into(),
        border_widths: 0f32.into(),
        border_color: theme::DANGER,
        border_style: BorderStyle::default(),
    });
    let glyph = shapes.fitted(
        window,
        "!",
        ShapeStyle::new(
            GLYPH,
            theme::FONT_UI,
            FontWeight::SEMIBOLD,
            theme::controls::ON_ACCENT,
        ),
        GLYPH,
    );
    let left =
        f32::from(bounds.origin.x) + (f32::from(bounds.size.width) - f32::from(glyph.width())) / 2.;
    let top = f32::from(bounds.origin.y) + (f32::from(bounds.size.height) - GLYPH) / 2.;
    let _ = glyph.paint(
        point(px(left), px(top)),
        px(GLYPH),
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// The hovered card's fault banner, hung below the card the tooltip is
/// for. The general painter below does the drawing; this only resolves
/// the canvas's card and clip.
pub(in crate::ui) fn paint_fault_tooltip(
    frame: &CanvasFrame,
    index: usize,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let node = &frame.world.nodes[index];
    if node.faults.is_empty() {
        return;
    }
    let card = frame.window_rect(frame.nodes[index].card);
    paint_fault_banner(
        &node.faults,
        card,
        frame.bounds,
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
}

/// The fault banner: a window-pixel panel hung below `anchor`, flipping
/// above it near `clip`'s bottom edge and clamped inside `clip`, listing
/// every fault's translated cause and hint. Shared by the graph page (the
/// hovered card) and the mixer page (the hovered strip), so both surfaces'
/// hover errors read identically.
pub(in crate::ui) fn paint_fault_banner(
    faults: &[CardFault],
    anchor: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    const PAD: f32 = 8.;
    const GAP: f32 = 6.;
    const ICON: f32 = 14.;
    const MAX_LINE: f32 = 360.;
    if faults.is_empty() {
        return;
    }
    let line_height = theme::TEXT_SM + 6.;
    let mut lines: Vec<(ShapedLine, bool)> = Vec::new();
    for fault in faults {
        lines.push((
            shapes
                .fitted(
                    window,
                    &t!(fault.cause.as_str()),
                    ShapeStyle::new(
                        theme::TEXT_SM,
                        theme::FONT_UI,
                        FontWeight::SEMIBOLD,
                        theme::DANGER,
                    ),
                    MAX_LINE,
                )
                .clone(),
            true,
        ));
        if let Some(hint) = &fault.hint {
            lines.push((
                shapes
                    .fitted(
                        window,
                        &t!(hint.as_str()),
                        ShapeStyle::new(
                            theme::TEXT_SM,
                            theme::FONT_UI,
                            FontWeight::NORMAL,
                            theme::TEXT,
                        ),
                        MAX_LINE,
                    )
                    .clone(),
                false,
            ));
        }
    }
    let text_width = lines
        .iter()
        .map(|(line, _)| f32::from(line.width()))
        .fold(0., f32::max);
    let width = PAD + ICON + 6. + text_width + PAD;
    let height = 2. * PAD + lines.len() as f32 * line_height;
    let clip_left = f32::from(clip.origin.x);
    let clip_right = clip_left + f32::from(clip.size.width);
    let clip_bottom = f32::from(clip.origin.y) + f32::from(clip.size.height);
    let left = f32::from(anchor.origin.x)
        .min(clip_right - width)
        .max(clip_left);
    let below = f32::from(anchor.origin.y) + f32::from(anchor.size.height) + GAP;
    let top = if below + height > clip_bottom {
        f32::from(anchor.origin.y) - GAP - height
    } else {
        below
    };
    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        window.paint_quad(PaintQuad {
            bounds: Bounds {
                origin: point(px(left), px(top)),
                size: size(px(width), px(height)),
            },
            corner_radii: theme::RADIUS_SM.into(),
            background: theme::SURFACE.into(),
            border_widths: 1f32.into(),
            border_color: theme::BORDER_STRONG,
            border_style: BorderStyle::default(),
        });
        let text_left = left + PAD + ICON + 6.;
        for (row, (line, cause)) in lines.iter().enumerate() {
            let row_top = top + PAD + row as f32 * line_height;
            if *cause {
                let _ = window.paint_svg(
                    Bounds {
                        origin: point(px(left + PAD), px(row_top + (line_height - ICON) / 2.)),
                        size: size(px(ICON), px(ICON)),
                    },
                    "icons/alert-triangle.svg".into(),
                    None,
                    TransformationMatrix::unit(),
                    theme::DANGER,
                    cx,
                );
            }
            let _ = line.paint(
                point(px(text_left), px(row_top)),
                px(line_height),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    });
}

/// One port anchor: the dot straddles the card edge (filled disc when
/// occupied, ring otherwise) with a channel-count hint inside the card.
struct PortPaint<'a> {
    anchor: (f32, f32),
    ring: &'static str,
    hint: Option<&'a TextLayout>,
    hint_text: String,
    hint_right: bool,
    occupied: bool,
}

fn paint_port(
    frame: &CanvasFrame,
    tint: Hsla,
    scale: f32,
    port: PortPaint<'_>,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let radius = 4. * scale;
    let bounds = frame.window_rect(Bounds {
        origin: point(port.anchor.0 - radius, port.anchor.1 - radius),
        size: size(radius * 2., radius * 2.),
    });
    if port.occupied {
        rounded(bounds, tint, radius, window);
    } else {
        let _ = window.paint_svg(
            bounds,
            format!("figma-nodes/{}.svg", port.ring).into(),
            None,
            TransformationMatrix::unit(),
            tint,
            cx,
        );
    }
    if let Some(hint) = port.hint {
        paint_label(
            frame,
            hint,
            Label {
                text: &port.hint_text,
                family: theme::FONT_MONO,
                color: theme::TEXT_FAINT,
                right_aligned: port.hint_right,
                weight: FontWeight::NORMAL,
            },
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
    }
}

/// The design 18 node card: header with icon, display name and the
/// expander glyph; full-height tint rail; per-kind body (the detail line
/// until the body widgets land); the meter strip; and port rails with
/// channel hints. Every card wears a gray outline so stacked cards keep
/// their edges, and the outline carries state — danger while a fault names
/// the node, warning while the engine re-prepares it, the primary highlight
/// while selected — so a card's body never changes with its state except
/// for a fault, which additionally tints the surface with the faint danger
/// wash and pins the "!" badge. Fault details surface in
/// [`paint_fault_tooltip`] on hover and in the fault popover via the badge.
pub(in crate::ui) fn paint_node(
    frame: &CanvasFrame,
    node: &CanvasNode,
    layout: &NodeLayout,
    selected: bool,
    live: super::NodeLive<'_>,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    let tint = node.tint();
    let card = frame.window_rect(layout.card);
    let radius = theme::card::RADIUS * layout.scale;
    let (outline, outline_width) = if node.faulted {
        (theme::DANGER, theme::card::OUTLINE_W)
    } else if node.preparing {
        (theme::WARNING, theme::card::OUTLINE_REST_W)
    } else if selected {
        (theme::controls::FOCUS, theme::card::OUTLINE_W)
    } else {
        (theme::GRIP, theme::card::OUTLINE_REST_W)
    };
    rounded(card, theme::controls::RAISED, radius, window);
    if node.faulted {
        // The faint red wash over the card's own surface, under the rail
        // and body that the masked paint lays on top of it.
        rounded(card, theme::DANGER_WASH, radius, window);
    }
    window.with_content_mask(Some(ContentMask { bounds: card }), |window| {
        // Tint rail: the card's own left edge just inside the outline. A
        // rail-wide quad cannot carry the card's corner radius (gpui clamps
        // radii to the quad's size and the corners poke out square), so
        // paint the inset card in tint and mask it down to the rail strip.
        let inset = px(theme::card::OUTLINE_W);
        let inner = Bounds {
            origin: card.origin + point(inset, inset),
            size: size(card.size.width - inset * 2., card.size.height - inset * 2.),
        };
        let rail = frame.window_rect(layout.rail);
        let strip = Bounds {
            origin: inner.origin,
            size: size(rail.size.width, inner.size.height),
        };
        window.with_content_mask(Some(ContentMask { bounds: strip }), |window| {
            rounded(
                inner,
                tint,
                (radius - theme::card::OUTLINE_W).max(0.),
                window,
            );
        });
        // Header.
        // Every kind icon is a Tabler glyph (24-unit box, 2-unit padding,
        // stroke 2), so all cards read at the same visual size.
        if let Some(name) = node.icon {
            let _ = window.paint_svg(
                frame.window_rect(layout.icon),
                format!("icons/{name}.svg").into(),
                None,
                TransformationMatrix::unit(),
                tint,
                cx,
            );
        }
        paint_label(
            frame,
            &layout.title,
            Label {
                text: &node.name,
                family: theme::FONT_UI,
                color: theme::TEXT,
                right_aligned: false,
                weight: FontWeight::SEMIBOLD,
            },
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
        let _ = window.paint_svg(
            frame.window_rect(layout.expand),
            "figma-nodes/more.svg".into(),
            None,
            TransformationMatrix::unit(),
            theme::TEXT_FAINT,
            cx,
        );
        if node.faulted {
            paint_fault_badge(
                frame,
                &layout.badge,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        }
        // Body: knobs where the kind exposes its main control, else the
        // detail line. States never touch the body — a faulted or
        // preparing card keeps every control usable under its outline.
        if !node.knobs.is_empty() {
            for (knob, knob_layout) in node.knobs.iter().zip(&layout.knobs) {
                paint_knob(
                    frame,
                    knob_layout,
                    knob,
                    layout.scale,
                    Paint {
                        shapes: &mut *shapes,
                        window: &mut *window,
                    },
                    cx,
                );
            }
        } else if let Some(chip) = node.binding.as_ref().zip(layout.chip.as_ref()) {
            let (chip, chip_layout) = chip;
            paint_binding_chip(
                frame,
                chip_layout,
                chip,
                tint,
                layout.scale,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        } else if let (Some(matrix), Some(bounds)) = (&node.matrix, layout.matrix) {
            paint_matrix_preview(frame, bounds, matrix, tint, layout.scale, window);
        } else if let (Some(bounds), false) = (layout.rack, node.rack_stages.is_empty()) {
            paint_rack_chain(
                frame,
                bounds,
                &node.rack_stages,
                tint,
                layout.scale,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        } else if layout.spectrum.is_some() {
            paint_spectrum_body(
                frame,
                layout,
                &node.detail,
                tint,
                live.spectrum,
                Paint {
                    shapes: &mut *shapes,
                    window: &mut *window,
                },
                cx,
            );
        } else if let (Some(spec), Some(bounds)) = (&node.curve, layout.curve) {
            paint_curve_thumb(frame, bounds, spec, layout.scale, window);
        } else {
            paint_label(
                frame,
                &layout.detail,
                Label {
                    text: &node.detail,
                    family: theme::FONT_UI,
                    color: theme::TEXT_MUTED,
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
        paint_meter_bars(frame, layout, live.meters, window);
    });
    // The outline last, above the rail and the body, so the ring stays
    // whole on every side whatever the state paints underneath.
    window.paint_quad(PaintQuad {
        bounds: card,
        corner_radii: radius.into(),
        background: theme::controls::TRANSPARENT.into(),
        border_widths: outline_width.into(),
        border_color: outline,
        border_style: BorderStyle::default(),
    });
    // Port dots straddle the card edge, outside the content mask; the
    // channel hints sit just inside. Occupied inputs render as filled
    // discs — the single-connection rule makes occupancy meaningful only
    // on the input side.
    let scale = layout.scale;
    for ((anchor, port), hint) in layout
        .inputs
        .iter()
        .copied()
        .zip(&node.input_ports)
        .zip(&layout.input_hints)
    {
        paint_port(
            frame,
            tint,
            scale,
            PortPaint {
                anchor,
                ring: "input-port",
                hint: Some(hint),
                hint_text: format!("{}ch", port.channels),
                hint_right: false,
                occupied: frame
                    .world
                    .wires
                    .iter()
                    .any(|wire| wire.dst_port == port.id),
            },
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
    }
    if let Some(anchor) = layout.output {
        paint_port(
            frame,
            tint,
            scale,
            PortPaint {
                anchor,
                ring: "output-port",
                hint: layout.output_hint.as_ref(),
                hint_text: format!(
                    "{}ch",
                    node.output_port
                        .map(|port| port.channels)
                        .unwrap_or(node.channels)
                ),
                hint_right: true,
                occupied: false,
            },
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Path, Pixels};

    use super::{arc_capsule_path, capsule_path};

    /// Total area of the tessellated triangles — a self-intersecting outline
    /// (a cap bulging the wrong way) would not add up to the analytic shape.
    fn area(path: &Path<Pixels>) -> f32 {
        path.vertices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|vertices| {
                let (a, b, c) = (
                    vertices[0].xy_position,
                    vertices[1].xy_position,
                    vertices[2].xy_position,
                );
                ((f32::from(b.x - a.x) * f32::from(c.y - a.y)
                    - f32::from(c.x - a.x) * f32::from(b.y - a.y))
                    / 2.)
                    .abs()
            })
            .sum()
    }

    #[test]
    fn capsule_covers_its_body_and_two_round_caps() {
        for (a, b, half) in [
            ((10., 10.), (60., 10.), 2.),
            ((0., 0.), (30., 40.), 1.6),
            ((50., 20.), (20., 50.), 3.),
        ] {
            let path = capsule_path(a, b, half).unwrap();
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let expected = 2. * half * length + std::f32::consts::PI * half * half;
            let actual = area(&path);
            assert!(
                (actual - expected).abs() / expected < 0.02,
                "{a:?}->{b:?}: {actual} vs {expected}"
            );
        }
        assert!(capsule_path((5., 5.), (5., 5.), 2.).is_err());
    }

    #[test]
    fn arc_capsule_covers_its_ring_segment_and_two_round_caps() {
        for (from, to) in [(135., 405.), (270., 300.), (300., 270.), (135., 270.)] {
            let (radius, half) = (17., 1.5);
            let path = arc_capsule_path((40., 40.), radius, half, from, to).unwrap();
            let sweep = (to - from).abs().to_radians();
            let expected = 2. * half * radius * sweep + std::f32::consts::PI * half * half;
            let actual = area(&path);
            assert!(
                (actual - expected).abs() / expected < 0.03,
                "{from}..{to}: {actual} vs {expected}"
            );
        }
        assert!(arc_capsule_path((0., 0.), 10., 1., 90., 90.2).is_err());
    }
}
