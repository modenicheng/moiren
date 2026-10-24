//! Real source/sink observations; missing windows retain unknown readouts.
//! Bars are dual-layer: the bright fill is the moving RMS body, the dark
//! zone above it extends to the peak bar, and the thin marker holds the
//! recent peak maximum. Zones follow EBU R68/R97 levels: green through the
//! −18 dBFS alignment level, yellow through the −9 dBFS permitted maximum,
//! red above.
use crate::ui::monitoring::DisplayLevel;
use crate::ui::theme;
use gpui_kit::prelude::*;
use gpui_kit::*;
fn label(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(10.))
        .text_color(theme::TEXT_MUTED)
        .child(text.into())
}
fn db(value: f64) -> String {
    if value <= 0. {
        "−∞".into()
    } else {
        format!("{:.1}", 20. * value.log10())
    }
}
pub fn stereo_meter_with_clip(levels: Option<[DisplayLevel; 2]>, clips: [bool; 2]) -> Div {
    let mut pair = div()
        .flex()
        .flex_none()
        .h(px(260.))
        .gap(px(2.))
        .line_height(px(12.));
    let mut ticks = div()
        .relative()
        .mt(px(16.))
        .h(px(190.))
        .flex_none()
        .w(px(20.))
        .text_size(px(8.))
        .text_color(theme::TEXT_FAINT);
    for db in [0, -9, -18, -36, -60] {
        ticks = ticks.child(
            div()
                .absolute()
                .top(px((-db as f32 / 60.) * 184.))
                .child(db.to_string()),
        );
    }
    for (top, name) in [(210., "PK"), (226., "RMS")] {
        ticks = ticks.child(div().absolute().top(px(top)).child(name));
    }
    pair = pair.child(ticks);
    for (index, name) in ["L", "R"].into_iter().enumerate() {
        let channel = levels.map(|l| l[index]);
        pair = pair.child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(px(24.))
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .h(px(12.))
                        .flex_none()
                        .px(px(3.))
                        .rounded(px(2.))
                        .text_size(px(8.))
                        .text_color(
                            if clips[index] || channel.is_some_and(|level| level.marker >= 1.) {
                                theme::DANGER
                            } else {
                                theme::TEXT_FAINT
                            },
                        )
                        .bg(
                            if clips[index] || channel.is_some_and(|level| level.marker >= 1.) {
                                theme::soft(theme::DANGER)
                            } else {
                                theme::SURFACE
                            },
                        )
                        .child(if channel.is_some() || clips[index] {
                            "CLIP"
                        } else {
                            "—"
                        }),
                )
                .child(meter_bar(
                    div()
                        .relative()
                        .w(px(7.))
                        .h(px(190.))
                        .flex_none()
                        .overflow_hidden()
                        .bg(theme::BORDER),
                    channel,
                ))
                .child(label(name))
                .child(
                    div().font_family(theme::FONT_MONO).text_size(px(8.)).child(
                        channel
                            .map(|l| db(f64::from(l.marker)))
                            .unwrap_or_else(|| "—".into()),
                    ),
                )
                .child(
                    div().font_family(theme::FONT_MONO).text_size(px(8.)).child(
                        channel
                            .map(|level| db(f64::from(level.rms)))
                            .unwrap_or_else(|| "—".into()),
                    ),
                ),
        );
    }
    pair
}

pub(crate) fn meter_height(value: f32) -> f32 {
    if value <= 0. {
        0.
    } else {
        ((20. * value.log10() + 60.) / 60.).clamp(0., 1.)
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
/// Zone boundaries anchor to EBU R68/R97 levels: −18 dBFS alignment level
/// (green→yellow) and −9 dBFS permitted maximum (yellow→red).
fn meter_spans(level: DisplayLevel) -> Vec<MeterSpan> {
    let bar_amount = meter_height(level.bar);
    let mut spans = Vec::new();
    for (bottom, top, color) in [
        (0., 0.7, theme::SUCCESS),
        (0.7, 0.85, theme::WARNING),
        (0.85, 1., theme::DANGER),
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

#[cfg(test)]
mod tests {
    use super::{MeterSpan, meter_height, meter_spans};
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
        assert_eq!(meter_height(1.), 1.);
        assert_eq!(meter_height(2.), 1.);
        assert!((meter_height(10f32.powf(-12. / 20.)) - 0.8).abs() < 0.00001);
        assert!((meter_height(10f32.powf(-3. / 20.)) - 0.95).abs() < 0.00001);
    }

    #[test]
    fn meter_spans_light_the_rms_body_and_darken_the_peak_tail() {
        let spans = meter_spans(level(-6., -30.));
        // The bar at −6 dB crosses all three zones: green through −18 dB,
        // yellow through −9 dB, red above (EBU R68/R97 anchors).
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].bottom, 0.);
        assert!((spans[0].height - 0.7).abs() < 0.00001);
        assert!((spans[1].bottom - 0.7).abs() < 0.00001);
        assert!((spans[1].height - 0.15).abs() < 0.00001);
        assert!((spans[2].bottom - 0.85).abs() < 0.00001);
        assert!((spans[2].height - 0.05).abs() < 0.00001);
        // The dark tail covers the transient zone from the −30 dB RMS body
        // up to the peak reading.
        assert!((spans[3].bottom - 0.5).abs() < 0.00001);
        assert!((spans[3].height - 0.4).abs() < 0.00001);
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
}
