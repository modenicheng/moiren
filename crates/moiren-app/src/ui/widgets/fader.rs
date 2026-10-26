//! One numerical scale for the main Mixer's fader, ruler and meters.
use crate::ui::theme;
use gpui_kit::prelude::*;
use gpui_kit::*;

pub struct FaderScale;

impl FaderScale {
    pub const ANCHORS: [(f64, f32); 10] = [
        (12., 0.),
        (6., 0.10),
        (0., 0.24),
        (-6., 0.38),
        (-12., 0.51),
        (-24., 0.67),
        (-36., 0.78),
        (-48., 0.86),
        (-60., 0.92),
        (-72., 0.97),
    ];

    /// Downwards from the track top. Silence has its own endpoint; finite
    /// gains below the last tick keep their value when snapshots synchronize.
    pub fn position(gain: f64) -> f32 {
        if gain <= 0. {
            return 1.;
        }
        let db = 20. * gain.log10();
        for pair in Self::ANCHORS.windows(2) {
            let [(high, top), (low, bottom)] = pair else {
                unreachable!()
            };
            if db >= *low {
                return top + ((high - db) / (high - low)).clamp(0., 1.) as f32 * (bottom - top);
            }
        }
        0.97
    }

    pub fn gain(position: f32) -> f64 {
        let position = position.clamp(0., 1.);
        if position >= 1. {
            return 0.;
        }
        for pair in Self::ANCHORS.windows(2) {
            let [(high, top), (low, bottom)] = pair else {
                unreachable!()
            };
            if position <= *bottom {
                let db = high + f64::from((position - top) / (bottom - top)) * (low - high);
                return 10f64.powf(db / 20.);
            }
        }
        // A short amplitude-linear segment reaches exact digital zero.
        10f64.powf(-72. / 20.) * f64::from((1. - position) / 0.03)
    }
}

/// These children all fill the same inset track. Relative positions therefore
/// share geometry even when the window or display scale changes.
pub fn ruler() -> Div {
    // A ruler is one painted surface, rather than eleven independently laid
    // out labels on every animation frame. It shares the track's exact bounds.
    div().w(px(26.)).h_full().flex_none().child(
        canvas(
            |_, _, _| {},
            |bounds, _, window, cx| {
                for (db, y) in FaderScale::ANCHORS {
                    // Keep the finite anchor, but avoid colliding with infinity
                    // at the compact track's bottom.
                    if db == -72. {
                        continue;
                    }
                    paint_text(
                        &format!("{db:+.0}"),
                        point(
                            bounds.left(),
                            bounds.top() + bounds.size.height * y - px(5.),
                        ),
                        9.,
                        TextAlign::Right,
                        Some(bounds.size.width - px(4.)),
                        window,
                        cx,
                    );
                }
                paint_text(
                    "−∞",
                    point(bounds.left(), bounds.bottom() - px(5.)),
                    9.,
                    TextAlign::Right,
                    Some(bounds.size.width - px(4.)),
                    window,
                    cx,
                );
            },
        )
        .size_full(),
    )
}

pub(super) fn paint_text(
    text: &str,
    origin: Point<Pixels>,
    font_size: f32,
    align: TextAlign,
    width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let line = window.text_system().shape_line(
        text.to_owned().into(),
        px(font_size),
        &[TextRun {
            len: text.len(),
            font: font(theme::FONT_MONO),
            color: theme::TEXT_FAINT,
            ..Default::default()
        }],
        None,
    );
    let _ = line.paint(origin, px(10.), align, width, window, cx);
}

/// Linear position of `value` inside the slider's own value range, 0..1.
/// Pure so the geometry tests run without a rendered slider.
fn normalized_value(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span <= 0. {
        return 0.;
    }
    ((value - min) / span).clamp(0., 1.)
}

/// Vertical thumb position from the track top. The fader's slider value space
/// is inverted (1 = top), so the thumb rides at `1 - normalized`.
fn thumb_top(value: f32, min: f32, max: f32) -> f32 {
    1. - normalized_value(value, min, max)
}

/// Vertical channel fader for the main mixer and the tray flyout: a flat
/// capsule track, a fill from the thumb down to the quiet end, and a 0 dB
/// detent tick (the flyout has no ruler or unity line of its own). The
/// widget reads its value from the slider state, so callers never duplicate
/// the gain mapping.
pub fn fader(state: &Entity<base::slider::SliderState>, cx: &App) -> impl IntoElement {
    let slider = state.read(cx);
    let thumb = thumb_top(slider.value().end(), slider.min_value(), slider.max_value());
    base::slider::Slider::new(state)
        .vertical()
        .relative()
        .flex_none()
        .w(px(32.))
        .h_full()
        // Keep the indicator: gpui uses it to record pointer mapping bounds.
        // Without it the state keeps zero bounds and drags map to NaN.
        .child(
            base::slider::SliderIndicator::new(state)
                .absolute()
                .left(px(13.))
                .top_0()
                .w(px(6.))
                .h_full(),
        )
        .child(
            base::slider::SliderTrack::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(13.))
                .top_0()
                .w(px(6.))
                .h_full()
                .rounded(px(theme::RADIUS_SM))
                .bg(theme::WINDOW_BG)
                .border_1()
                .border_color(theme::BORDER),
        )
        .child(
            // The value space is inverted: everything louder than the thumb
            // lights up between it and the bottom (quiet) end.
            div()
                .absolute()
                .left(px(13.))
                .top(relative(thumb))
                .bottom_0()
                .w(px(6.))
                .rounded(px(theme::RADIUS_SM))
                .bg(theme::SIGNAL_DEVICE.opacity(0.35)),
        )
        .child(
            div()
                .absolute()
                .left(px(11.))
                .top(relative(FaderScale::position(1.)))
                .w(px(10.))
                .h(px(1.))
                .bg(theme::SIGNAL_DEVICE.opacity(0.45)),
        )
        .child(
            base::slider::SliderThumb::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(5.))
                .top(relative(thumb))
                .mt(px(-7.))
                .w(px(22.))
                .h(px(14.))
                .rounded(px(3.))
                .bg(theme::SURFACE_3)
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .child(
                    div()
                        .absolute()
                        .left(px(2.))
                        .right(px(2.))
                        .top(px(6.))
                        .h(px(2.))
                        .bg(theme::TEXT),
                ),
        )
}

/// Minimal horizontal slider for inspector parameters, the channel strip and
/// sends: a thin capsule track, a fill from the quiet (left) end and a dot
/// thumb. No detents, ticks or labels; the surrounding row keeps the value.
pub fn hslider(state: &Entity<base::slider::SliderState>, cx: &App) -> impl IntoElement {
    let slider = state.read(cx);
    let fill = normalized_value(slider.value().end(), slider.min_value(), slider.max_value());
    base::slider::Slider::new(state)
        .horizontal()
        .relative()
        .flex_none()
        .w_full()
        .h(px(16.))
        .child(
            base::slider::SliderIndicator::new(state)
                .absolute()
                .left_0()
                .top(px(5.))
                .w_full()
                .h(px(6.)),
        )
        .child(
            base::slider::SliderTrack::new(state)
                .axis(Axis::Horizontal)
                .absolute()
                .left_0()
                .top(px(5.))
                .w_full()
                .h(px(6.))
                .rounded_full()
                .bg(theme::WINDOW_BG)
                .border_1()
                .border_color(theme::BORDER),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .top(px(5.))
                .w(relative(fill))
                .h(px(6.))
                .rounded_full()
                .bg(theme::SIGNAL_DEVICE.opacity(0.3)),
        )
        .child(
            base::slider::SliderThumb::new(state)
                .axis(Axis::Horizontal)
                .absolute()
                .left(relative(fill))
                .ml(px(-6.))
                .top(px(2.))
                .w(px(12.))
                .h(px(12.))
                .rounded_full()
                .bg(theme::TEXT_MUTED)
                .hover(|style| style.bg(theme::TEXT))
                .border_1()
                .border_color(theme::BORDER_STRONG),
        )
}

#[cfg(test)]
mod tests {
    use super::FaderScale;

    #[test]
    fn scale_aligns_working_levels_and_round_trips() {
        for (db, y) in [
            (12., 0.),
            (6., 0.10),
            (0., 0.24),
            (-6., 0.38),
            (-12., 0.51),
            (-24., 0.67),
            (-36., 0.78),
            (-48., 0.86),
            (-60., 0.92),
            (-72., 0.97),
        ] {
            let gain = 10f64.powf(db / 20.);
            assert!((FaderScale::position(gain) - y).abs() < 0.00001);
            assert!((20. * FaderScale::gain(y).log10() - db).abs() < 0.0001);
        }
        for db in -71..13 {
            let gain = 10f64.powf(f64::from(db) / 20.);
            assert!(
                (20. * FaderScale::gain(FaderScale::position(gain)).log10() - f64::from(db)).abs()
                    < 0.001
            );
        }
    }

    #[test]
    fn silence_is_distinct_from_finite_values_below_the_last_tick() {
        assert_eq!(FaderScale::position(0.), 1.);
        assert_eq!(FaderScale::gain(1.), 0.);
        assert_eq!(FaderScale::position(1e-5), 0.97);
        assert!(FaderScale::gain(0.99) > 0.);
    }

    #[test]
    fn normalized_maps_values_inside_the_slider_range() {
        use super::{normalized_value, thumb_top};
        assert_eq!(normalized_value(0., 0., 1.), 0.);
        assert_eq!(normalized_value(0.5, 0., 1.), 0.5);
        assert_eq!(normalized_value(12., -60., 24.), 72. / 84.);
        assert_eq!(normalized_value(-70., -60., 24.), 0.);
        assert_eq!(normalized_value(30., -60., 24.), 1.);
        // A degenerate span keeps the thumb at the quiet end instead of NaN.
        assert_eq!(normalized_value(5., 5., 5.), 0.);
        assert_eq!(thumb_top(0., 0., 1.), 1.);
    }

    #[test]
    fn vertical_thumb_positions_mirror_the_fader_scale() {
        use super::thumb_top;
        // The slider's inverted value space round-trips through the scale: a
        // value derived from a gain puts the thumb at the gain's position.
        for (db, position) in FaderScale::ANCHORS {
            let gain = 10f64.powf(db / 20.);
            let value = 1. - FaderScale::position(gain);
            assert!((thumb_top(value, 0., 1.) - position).abs() < 0.00001);
        }
    }
}
