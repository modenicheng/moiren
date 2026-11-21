use gpui_kit::{Hsla, Window, point, px};

use crate::ui::pages::graph::graph_canvas::{CanvasFrame, CurveSpec, curves};
use crate::ui::theme;

/// A static curve thumbnail: baseline, the sampled curve, and a
/// center/threshold tick. The same math the detail window paints, at
/// card scale.
pub(super) fn paint_curve_thumb(
    frame: &CanvasFrame,
    bounds: gpui_kit::Bounds<f32>,
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
