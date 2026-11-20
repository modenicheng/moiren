use gpui_kit::{App, ContentMask, Hsla, PathBuilder, point, px};

use crate::ui::monitoring::{REST_DB, SPECTRUM_NOMINAL_SPAN_HZ, SpectrumCurve};
use crate::ui::pages::graph::graph_canvas::{CanvasFrame, NodeLayout};
use crate::ui::theme;

use super::shared::{Paint, hairline, paint_anchored_text, rounded};

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
