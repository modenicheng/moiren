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

pub fn fader(state: &Entity<base::slider::SliderState>, gain: f64) -> impl IntoElement {
    base::slider::Slider::new(state)
        .vertical()
        .relative()
        .flex_none()
        .w(px(32.))
        .h_full()
        // Keep the indicator: gpui uses it to record pointer mapping bounds.
        .child(
            base::slider::SliderIndicator::new(state)
                .absolute()
                .left(px(15.))
                .top_0()
                .w(px(2.))
                .h_full(),
        )
        .child(
            base::slider::SliderTrack::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(15.))
                .top_0()
                .w(px(2.))
                .h_full()
                .bg(theme::BORDER_STRONG),
        )
        .child(
            base::slider::SliderThumb::new(state)
                .axis(Axis::Vertical)
                .absolute()
                .left(px(5.))
                .top(relative(FaderScale::position(gain)))
                .mt(px(-16.))
                .w(px(22.))
                .h(px(32.))
                .bg(theme::TEXT_MUTED)
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .child(
                    div()
                        .absolute()
                        .left(px(2.))
                        .right(px(2.))
                        .top(px(14.))
                        .h(px(2.))
                        .bg(theme::TEXT),
                ),
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
}
