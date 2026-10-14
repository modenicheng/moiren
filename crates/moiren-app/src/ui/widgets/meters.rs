//! Level meters and parameter knobs, ported from studio-controls.slint
//! (ChannelMeter, StereoLevelMeters) and dev-ready.slint (DevReadyKnob).

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::theme;

/// One channel bar: RMS fill (0..1) + peak line (0..1).
/// Vertical: fill grows from the bottom, like a tape machine meter.
pub fn channel_meter(level: f32, peak: f32) -> impl IntoElement {
    let level = level.clamp(0., 1.);
    let peak = peak.clamp(0., 1.);
    div()
        .relative()
        .w(px(7.))
        .h_full()
        .rounded(px(2.))
        .bg(theme::BORDER)
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(pct(level))
                .bg(theme::PRIMARY),
        )
        .when(peak > 0., |s| {
            s.child(
                div()
                    .absolute()
                    .left_0()
                    .w_full()
                    .h(px(2.))
                    .bottom(pct(peak))
                    .bg(theme::DANGER),
            )
        })
}

/// L/R pair with labels, for inspector panels and the mixer.
pub fn stereo_level_meters(
    left_rms: f32,
    right_rms: f32,
    left_peak: f32,
    right_peak: f32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .min_w(px(38.))
        .child(
            div()
                .id("stereo-tracks")
                .flex()
                .flex_1()
                .gap(px(6.))
                .items_stretch()
                .justify_center()
                .child(channel_meter(left_rms, left_peak))
                .child(channel_meter(right_rms, right_peak)),
        )
        .child(
            div()
                .flex()
                .h(px(14.))
                .gap(px(6.))
                .justify_center()
                .child(meter_label("L"))
                .child(meter_label("R")),
        )
}

fn meter_label(text: &str) -> impl IntoElement {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(9.))
        .text_color(theme::TEXT_FAINT)
        .child(text.to_string())
}

/// Percentage height helper (relative units).
fn pct(fraction: f32) -> Pixels {
    // Heights inside meters are driven by absolute pixel math in the parent;
    // fractional heights map through the `relative()` style. This helper is
    // only used where a parent supplies a concrete track height, which gpui
    // expresses via percentage styles. We convert to a Percentage length.
    // The rounding at extremes keeps 0.0 and 1.0 exact.
    px(fraction * 100.).clamp(px(0.), px(100.))
}

/// Parameter knob (DevReadyKnob): arc track + value arc + pointer dot.
/// The knob is display-only for now; drag-to-set lands with gesture work.
pub fn knob(label: &str, value: f32, tint: Hsla) -> impl IntoElement {
    let value = value.clamp(0., 1.);
    div()
        .id("knob")
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .w(px(56.))
        .child(knob_dial(value, tint))
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(px(theme::TEXT_XS))
                .text_color(theme::TEXT_MUTED)
                .child(label.to_string()),
        )
}

fn knob_dial(value: f32, tint: Hsla) -> impl IntoElement {
    // 270° sweep from 225° (bottom-left) to 315° (bottom-right).
    let size = 44.;
    let center = size / 2.;
    let radius = 16.;
    let start_deg = 225.;
    let sweep_deg = 270.;
    let end_deg = start_deg + sweep_deg * value;
    let to_point = move |deg: f32| -> (f32, f32) {
        let rad = deg.to_radians();
        (
            center + radius * rad.cos(),
            center - radius * rad.sin(), // screen y is flipped
        )
    };
    let (sx, sy) = to_point(start_deg);
    let (ex, ey) = to_point(end_deg);
    let (tx, ty) = to_point(start_deg + sweep_deg);
    // Pointer sits on the value arc.
    let (px_, py_) = to_point(end_deg);

    canvas(
        move |bounds, _w, _cx| bounds,
        move |bounds, _state, window, _cx| {
            let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let mut track = Path::new(point(px(ox + sx), px(oy + sy)));
            // Track arc: approximate the 270° sweep with line segments through
            // a few sample points; fill-only path needs a closed polygon.
            let mut lower = vec![];
            let steps = 24;
            for i in 0..=steps {
                let deg = start_deg + sweep_deg * (i as f32 / steps as f32);
                let (x, y) = to_point(deg);
                let rad = deg.to_radians();
                // inner offset toward center for a ribbon of width 3
                let ix = center + (radius - 3.) * rad.cos();
                let iy = center - (radius - 3.) * rad.sin();
                lower.push((ix, iy));
                if i == 0 {
                    track = Path::new(point(px(ox + x), px(oy + y)));
                } else {
                    track.line_to(point(px(ox + x), px(oy + y)));
                }
                let _ = (x, y);
            }
            for (x, y) in lower.iter().rev() {
                track.line_to(point(px(ox + x), px(oy + y)));
            }
            window.paint_path(track, theme::BORDER_STRONG);

            // Value arc (tint), same ribbon technique from start to end_deg.
            let mut value_path_points: Vec<(f32, f32)> = Vec::new();
            let mut value_inner: Vec<(f32, f32)> = Vec::new();
            let vsteps = ((end_deg - start_deg).max(0.) / 10.).ceil() as i32;
            for i in 0..=vsteps.max(1) {
                let deg = start_deg + (end_deg - start_deg) * (i as f32 / vsteps.max(1) as f32);
                let (x, y) = to_point(deg);
                let rad = deg.to_radians();
                value_path_points.push((ox + x, oy + y));
                value_inner.push((
                    ox + center + (radius - 3.) * rad.cos(),
                    oy + center - (radius - 3.) * rad.sin(),
                ));
            }
            if value_path_points.len() >= 2 {
                let mut vp = Path::new(point(
                    px(value_path_points[0].0),
                    px(value_path_points[0].1),
                ));
                for (x, y) in value_path_points.iter().skip(1) {
                    vp.line_to(point(px(*x), px(*y)));
                }
                for (x, y) in value_inner.iter().rev() {
                    vp.line_to(point(px(*x), px(*y)));
                }
                window.paint_path(vp, tint);
            }

            let _ = (ex, ey, tx, ty);
            // Pointer dot at the value angle.
            window.paint_quad(gpui_kit::fill(
                Bounds {
                    origin: point(px(ox + px_ - 2.), px(oy + py_ - 2.)),
                    size: gpui_kit::size(px(4.), px(4.)),
                },
                theme::TEXT,
            ));
        },
    )
    .size(px(size))
}
