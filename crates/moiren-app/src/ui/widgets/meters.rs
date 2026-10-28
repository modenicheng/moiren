//! Real source/sink observations; missing windows retain unknown readouts.
//! Bars are dual-layer: the bright fill is the moving RMS body, the dark
//! zone above it extends to the peak bar, and the thin marker holds the
//! recent peak maximum. The digital scale uses green below −6 dBFS, yellow
//! from −6 to 0 dBFS, and red above digital zero.
use crate::ui::monitoring::DisplayLevel;
use crate::ui::theme;
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Main Mixer bars fill exactly the shared fader track; there is no second
/// ruler or independent dB interpolation here. Channel count is never inferred
/// from a stereo projection of mono/multichannel data.
pub(crate) fn channel_meter_width(channels: usize) -> f32 {
    let count = channels.max(1) as f32;
    let gap = (8. / count).min(3.);
    let width = (28. / count)
        .clamp(2., 10.)
        .min((40. - gap * (count - 1.)) / count);
    width * count + gap * (count - 1.)
}

pub(crate) fn paint_channel_meter(
    bounds: Bounds<Pixels>,
    levels: Option<&[DisplayLevel]>,
    channels: usize,
    clips: &[bool],
    window: &mut Window,
    cx: &mut App,
) {
    use super::fader::{FaderScale, paint_text};
    let count = channels.max(1) as f32;
    let gap = (8. / count).min(3.);
    let width = (28. / count)
        .clamp(2., 10.)
        .min((40. - gap * (count - 1.)) / count);
    for index in 0..channels {
        let left = bounds.left() + px(index as f32 * (width + gap));
        let track = Bounds::new(
            point(left, bounds.top()),
            size(px(width), bounds.size.height),
        );
        window.paint_quad(fill(track, theme::BORDER));
        let level = levels.and_then(|levels| levels.get(index));
        if let Some(level) = level {
            // The zone segmentation, the dark peak tail and their
            // colors are the DOM bars' [`meter_spans`]; only the
            // painting surface differs here.
            for span in meter_spans(*level) {
                window.paint_quad(fill(
                    Bounds::new(
                        point(
                            left,
                            bounds.bottom() - bounds.size.height * (span.bottom + span.height),
                        ),
                        size(px(width), bounds.size.height * span.height),
                    ),
                    span.color,
                ));
            }
            let y =
                bounds.top() + bounds.size.height * FaderScale::position(f64::from(level.marker));
            window.paint_quad(fill(
                Bounds::new(point(left, y - px(1.)), size(px(width), px(2.))).intersect(&track),
                theme::TEXT,
            ));
        }
        // A held display marker is not a new overload. Trust the
        // raw-window latch so clicking reset clears it immediately.
        let clip = clips.get(index).copied().unwrap_or(false);
        window.paint_quad(fill(
            Bounds::new(point(left, bounds.top() - px(10.)), size(px(width), px(3.))),
            if clip { theme::DANGER } else { theme::BORDER },
        ));
        if width + gap >= 5. {
            let label = if channels == 2 {
                ["L", "R"][index].to_owned()
            } else {
                (index + 1).to_string()
            };
            paint_text(
                &label,
                point(left, bounds.bottom() + px(6.)),
                8.,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
    if width + gap < 5. {
        paint_text(
            &format!("{channels} ch"),
            point(bounds.left(), bounds.bottom() + px(6.)),
            8.,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

pub(crate) fn meter_height(value: f32) -> f32 {
    if value <= 0. {
        return 0.;
    }
    use super::fader::FaderScale;
    let (&(floor_db, floor_position), _) = FaderScale::ANCHORS.split_last().expect("non-empty");
    let floor = 10f64.powf(floor_db / 20.);
    let value = f64::from(value);
    if value >= floor {
        1. - FaderScale::position(value)
    } else {
        // `FaderScale::position` holds finite gains at the last tick so the
        // fader itself stays visible; a meter bar must keep falling, so
        // invert the amplitude-linear tail of `FaderScale::gain` that owns
        // the `floor_position`..1 segment reaching exact digital zero.
        (value / floor) as f32 * (1. - floor_position)
    }
}

/// One colored layer of a meter bar, spanning `bottom`..`bottom + height` in
/// the 0..1 track space.
struct MeterSpan {
    bottom: f32,
    height: f32,
    color: Hsla,
}

/// Layers for one channel: the bright segment fill covers the RMS body, and
/// a translucent dark extension above it reaches the peak bar so the
/// sustained reading stays light and transients show as the dark tail.
/// Zone boundaries use the compact digital reference: −6 dBFS
/// (green→yellow) and 0 dBFS (yellow→red). This is the single source for the
/// segmentation strategy: the canvas meter paints the same spans with quads.
fn meter_spans(level: DisplayLevel) -> Vec<MeterSpan> {
    let bar_amount = meter_height(level.bar);
    let mut spans = Vec::with_capacity(4);
    let green_top = meter_height(10f32.powf(-6. / 20.));
    let red_top = meter_height(1.);
    for (bottom, top, color) in [
        (0., green_top, theme::SUCCESS),
        (green_top, red_top, theme::WARNING),
        (red_top, 1., theme::DANGER),
    ] {
        let height = (bar_amount.min(top) - bottom).max(0.);
        if height > 0. {
            spans.push(MeterSpan {
                bottom,
                height,
                color,
            });
        }
    }
    // The display ballistics can leave the smoothed RMS above a falling bar;
    // the dark tail must stay above the lit fill.
    let rms_amount = meter_height(level.rms).min(bar_amount);
    let tail = bar_amount - rms_amount;
    if tail > 0. {
        spans.push(MeterSpan {
            bottom: rms_amount,
            height: tail,
            color: theme::METER_PEAK_ZONE,
        });
    }
    spans
}

/// Draw the dual-layer fill and the held peak marker onto a prepared track.
fn meter_bar(track: Div, channel: Option<DisplayLevel>) -> Div {
    let Some(level) = channel else {
        return track;
    };
    let mut track = track;
    for span in meter_spans(level) {
        track = track.child(
            div()
                .absolute()
                .bottom(relative(span.bottom))
                .w_full()
                .h(relative(span.height))
                .bg(span.color),
        );
    }
    track.child(
        div()
            .absolute()
            .bottom(relative(meter_height(level.marker)))
            .w_full()
            .h(px(2.))
            .bg(theme::TEXT),
    )
}

/// Flyout-scale stereo meter: two slim bars and a clip dot, no scale readouts.
pub fn compact_stereo_meter(levels: Option<[DisplayLevel; 2]>, clips: [bool; 2]) -> Div {
    let mut pair = div()
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .gap(px(3.))
        .line_height(px(10.));
    let mut bars = div().flex().flex_none().gap(px(2.));
    for index in 0..2 {
        let channel = levels.map(|l| l[index]);
        bars = bars.child(meter_bar(
            div()
                .relative()
                .w(px(7.))
                .h(px(176.))
                .flex_none()
                .overflow_hidden()
                .rounded(px(1.))
                .bg(theme::BORDER),
            channel,
        ));
    }
    pair = pair
        .child(bars)
        .child(div().w(px(10.)).h(px(3.)).flex_none().rounded(px(1.)).bg(
            if clips[0] || clips[1] {
                theme::DANGER
            } else {
                theme::BORDER
            },
        ));
    pair
}

/// A horizontal bar reuses the vertical track space: `bottom` becomes the
/// distance from the left (quiet) edge and `height` the extent from there.
fn horizontal_span(span: &MeterSpan) -> (f32, f32) {
    (span.bottom, span.height)
}

/// Inspector-scale horizontal meter: one slim bar per channel (up to a stereo
/// pair), the quiet end at the left, reusing `meter_spans` so zone colors and
/// the peak tail stay identical to the vertical bars. The clip latch is a
/// single dot below the bars, like the flyout meter.
pub fn compact_horizontal_meter(
    levels: Option<&[DisplayLevel]>,
    channels: usize,
    clips: &[bool],
) -> Div {
    let levels = levels.map(<[DisplayLevel]>::to_vec);
    let clips = clips.to_vec();
    let count = channels.clamp(1, 2);
    let mut bars = div().flex().flex_none().flex_col().gap(px(2.)).w_full();
    for index in 0..count {
        let mut bar = div()
            .relative()
            .w_full()
            .h(px(4.))
            .flex_none()
            .overflow_hidden()
            .rounded(px(1.))
            .bg(theme::BORDER);
        if let Some(level) = levels.as_deref().and_then(|levels| levels.get(index)) {
            for span in meter_spans(*level) {
                let (left, width) = horizontal_span(&span);
                bar = bar.child(
                    div()
                        .absolute()
                        .left(relative(left))
                        .top_0()
                        .bottom_0()
                        .w(relative(width))
                        .bg(span.color),
                );
            }
            bar = bar.child(
                div()
                    .absolute()
                    .left(relative(meter_height(level.marker)))
                    .top_0()
                    .bottom_0()
                    .w(px(2.))
                    .bg(theme::TEXT),
            );
        }
        bars = bars.child(bar);
    }
    let clip = (0..count).any(|index| clips.get(index).copied().unwrap_or(false));
    bars.child(
        div()
            .w(px(6.))
            .h(px(3.))
            .flex_none()
            .rounded(px(1.))
            .bg(if clip { theme::DANGER } else { theme::BORDER }),
    )
}

#[cfg(test)]
mod tests {
    use super::{MeterSpan, horizontal_span, meter_height, meter_spans};
    use crate::ui::monitoring::DisplayLevel;
    use crate::ui::theme;

    fn level(peak_db: f32, rms_db: f32) -> DisplayLevel {
        let peak = 10f32.powf(peak_db / 20.);
        DisplayLevel {
            bar: peak,
            marker: peak,
            rms: 10f32.powf(rms_db / 20.),
        }
    }

    fn same_color(span: &MeterSpan, expected: gpui_kit::Hsla) -> bool {
        span.color.h == expected.h
            && span.color.s == expected.s
            && span.color.l == expected.l
            && span.color.a == expected.a
    }

    #[test]
    fn meter_regions_follow_dbfs_and_clamp_clipping() {
        assert_eq!(meter_height(0.), 0.);
        assert!((meter_height(1.) - 0.76).abs() < 0.00001);
        assert!(meter_height(2.) > meter_height(1.));
        assert_eq!(meter_height(10f32.powf(12. / 20.)), 1.);
        assert_eq!(meter_height(2. * 10f32.powf(12. / 20.)), 1.);
        assert!((meter_height(10f32.powf(-6. / 20.)) - 0.62).abs() < 0.00001);
    }

    #[test]
    fn silence_below_the_scale_floor_keeps_falling_to_zero_height() {
        let floor = 10f32.powf(-72. / 20.);
        // The tail meets the interpolated scale exactly at the last tick.
        assert!((meter_height(floor) - 0.03).abs() < 0.00001);
        // A decaying bar keeps shrinking through the tail instead of
        // saturating at the fader's −72 dB hold.
        let fading = meter_height(floor * 0.01);
        assert!(fading > 0. && fading < 0.0004, "fading = {fading}");
        assert!(meter_height(floor * 0.5) < meter_height(floor));
        // And exact digital zero stays fully invisible.
        assert_eq!(meter_height(0.), 0.);
    }

    #[test]
    fn meter_spans_light_the_rms_body_and_darken_the_peak_tail() {
        let spans = meter_spans(level(3., -30.));
        // The bar at +3 dB crosses all three zones: green through −6 dB,
        // yellow through 0 dB, red above digital zero.
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].bottom, 0.);
        let green_top = meter_height(10f32.powf(-6. / 20.));
        let red_top = meter_height(1.);
        let peak = meter_height(10f32.powf(3. / 20.));
        let rms = meter_height(10f32.powf(-30. / 20.));
        assert!((spans[0].height - green_top).abs() < 0.00001);
        assert!((spans[1].bottom - green_top).abs() < 0.00001);
        assert!((spans[1].height - (red_top - green_top)).abs() < 0.00001);
        assert!((spans[2].bottom - red_top).abs() < 0.00001);
        assert!((spans[2].height - (peak - red_top)).abs() < 0.00001);
        // The dark tail covers the transient zone from the −30 dB RMS body
        // up to the peak reading.
        assert!((spans[3].bottom - rms).abs() < 0.00001);
        assert!((spans[3].height - (peak - rms)).abs() < 0.00001);
        assert!(same_color(&spans[3], theme::METER_PEAK_ZONE));
    }

    #[test]
    fn meter_spans_skip_the_dark_tail_when_rms_reaches_the_bar() {
        // The display ballistics can leave the smoothed RMS above a falling
        // bar; the whole fill then reads as the bright RMS body.
        let spans = meter_spans(DisplayLevel {
            bar: 0.1,
            marker: 0.1,
            rms: 0.5,
        });
        assert_eq!(spans.len(), 1);
        assert!((spans[0].height - meter_height(0.1)).abs() < 0.00001);
        assert!(!same_color(&spans[0], theme::METER_PEAK_ZONE));
    }

    #[test]
    fn horizontal_spans_map_distance_from_the_quiet_left_edge() {
        // Mapping bottom->left and height->width rotates the vertical track:
        // the zone segments tile from the left edge up to the bar height, and
        // the translucent tail overlays the top of that run exactly as it
        // overlays the zone tops in the vertical bars.
        let spans = meter_spans(level(3., -30.));
        assert_eq!(spans.len(), 4);
        let mut reach = 0.;
        for span in &spans[..3] {
            let (left, width) = horizontal_span(span);
            assert!((left - reach).abs() < 0.00001);
            reach = left + width;
        }
        let peak = meter_height(10f32.powf(3. / 20.));
        assert!((reach - peak).abs() < 0.00001);
        let rms = meter_height(10f32.powf(-30. / 20.));
        let (left, width) = horizontal_span(&spans[3]);
        assert!((left - rms).abs() < 0.00001);
        assert!((width - (peak - rms)).abs() < 0.00001);
        let (left, width) = horizontal_span(&spans[0]);
        assert_eq!(left, 0.);
        assert!((width - meter_height(10f32.powf(-6. / 20.))).abs() < 0.00001);
    }
}
