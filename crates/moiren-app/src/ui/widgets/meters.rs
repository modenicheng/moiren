//! Real source/sink observations; missing windows retain unknown readouts.
//! Bars are dual-layer: the bright fill is the moving RMS body, the dark
//! zone above it extends to the peak bar, and the thin marker holds the
//! recent peak maximum. The digital scale uses green below −6 dBFS, yellow
//! from −6 to 0 dBFS, and red above digital zero.
use crate::ui::monitoring::DisplayLevel;
use crate::ui::theme;
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

/// The clip lamp at the loud end of one meter track: above a vertical
/// track, right of a horizontal one. Lit red while a clip latches, dim
/// otherwise; the surrounding element owns click-to-reset. The single clip
/// indicator for every meter surface — the fill never changes background
/// for clipping.
pub(crate) fn paint_clip_lamp(
    track: Bounds<Pixels>,
    axis: Axis,
    clipped: bool,
    window: &mut Window,
) {
    let lamp = match axis {
        Axis::Vertical => Bounds::new(
            point(track.left(), track.top() - px(10.)),
            size(track.size.width, px(3.)),
        ),
        Axis::Horizontal => Bounds::new(
            point(track.right() + px(3.), track.top()),
            size(px(3.), track.size.height),
        ),
    };
    window.paint_quad(fill(
        lamp,
        if clipped {
            theme::DANGER
        } else {
            theme::BORDER
        },
    ));
}

pub(crate) fn paint_channel_meter(
    bounds: Bounds<Pixels>,
    levels: Option<&[DisplayLevel]>,
    channels: usize,
    clips: &[bool],
    window: &mut Window,
    cx: &mut App,
) {
    use super::fader::paint_text;
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
        let level = levels.and_then(|levels| levels.get(index));
        if let Some(level) = level {
            paint_meter_fill(track, *level, theme::BORDER, window);
        }
        // A held display marker is not a new overload. Trust the
        // raw-window latch so clicking reset clears it immediately.
        let clip = clips.get(index).copied().unwrap_or(false);
        paint_clip_lamp(track, Axis::Vertical, clip, window);
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
    let (floor_db, floor_position) = scale_floor();
    let floor = 10f64.powf(floor_db / 20.);
    let value = f64::from(value);
    if value >= floor {
        use super::fader::FaderScale;
        1. - FaderScale::position(value)
    } else {
        // `FaderScale::position` holds finite gains at the last tick so the
        // fader itself stays visible; a meter bar must keep falling, so
        // invert the amplitude-linear tail of `FaderScale::gain` that owns
        // the `floor_position`..1 segment reaching exact digital zero.
        (value / floor) as f32 * (1. - floor_position)
    }
}

/// The last scale tick. The fader holds finite gains at it on purpose, so
/// meter readings must never reuse `FaderScale::position` below it.
fn scale_floor() -> (f64, f32) {
    use super::fader::FaderScale;
    let (&floor, _) = FaderScale::ANCHORS.split_last().expect("non-empty");
    floor
}

/// Track position of a held peak marker, downwards from the top in 0..1,
/// sharing [`meter_height`]'s inverted tail: below the last tick the fader
/// holds position, but a falling marker keeps sliding through the −∞ tail
/// and rests at the bottom edge.
fn meter_marker_position(marker: f32) -> f32 {
    1. - meter_height(marker)
}

/// Fill one vertical meter track from the shared `meter_spans` math: the
/// zone colors rise from the quiet bottom, the dark RMS tail covers the
/// transient zone, and the held marker stands at its reading. Every meter
/// surface paints these spans; only the track geometry and the surrounding
/// decorations differ.
pub(crate) fn paint_meter_fill(
    track: Bounds<Pixels>,
    level: DisplayLevel,
    background: Hsla,
    window: &mut Window,
) {
    window.paint_quad(fill(track, background));
    for span in meter_spans(level) {
        window.paint_quad(fill(
            Bounds::new(
                point(
                    track.left(),
                    track.bottom() - track.size.height * (span.bottom + span.height),
                ),
                size(track.size.width, track.size.height * span.height),
            ),
            span.color,
        ));
    }
    let y = track.top() + track.size.height * meter_marker_position(level.marker);
    window.paint_quad(fill(
        Bounds::new(
            point(track.left(), y - px(1.)),
            size(track.size.width, px(2.)),
        ),
        theme::TEXT,
    ));
}

/// [`paint_meter_fill`] rotated for horizontal tracks: `bottom` becomes the
/// quiet left edge and `height` the extent from there.
pub(crate) fn paint_meter_fill_horizontal(
    track: Bounds<Pixels>,
    level: DisplayLevel,
    background: Hsla,
    window: &mut Window,
) {
    window.paint_quad(fill(track, background));
    for span in meter_spans(level) {
        window.paint_quad(fill(
            Bounds::new(
                point(track.left() + track.size.width * span.bottom, track.top()),
                size(track.size.width * span.height, track.size.height),
            ),
            span.color,
        ));
    }
    let x = track.left() + track.size.width * meter_height(level.marker);
    window.paint_quad(fill(
        Bounds::new(point(x, track.top()), size(px(2.), track.size.height)),
        theme::TEXT,
    ));
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
/// segmentation strategy: every meter surface paints these spans through the
/// [`paint_meter_fill`] painters in its own orientation.
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

#[cfg(test)]
mod tests {
    use super::{MeterSpan, meter_height, meter_marker_position, meter_spans};
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
    fn peak_markers_slide_through_the_silence_tail_to_the_bottom_edge() {
        let floor = 10f32.powf(-72. / 20.);
        // Mid-scale markers keep their interpolated position.
        assert!((meter_marker_position(1.) - 0.24).abs() < 0.001);
        // Below the fader's −72 dB hold the marker keeps sliding instead of
        // pinning at the tick, converging on the bottom edge as it fades.
        let at_floor = meter_marker_position(floor);
        assert!((at_floor - 0.97).abs() < 0.00001);
        assert!(meter_marker_position(floor * 0.5) > at_floor);
        let faded = meter_marker_position(floor * 0.01);
        assert!(faded > 0.999 && faded < 1., "faded = {faded}");
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
}
