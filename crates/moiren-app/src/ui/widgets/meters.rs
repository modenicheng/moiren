//! Real source/sink observations; missing windows retain unknown readouts.
//! Bars are dual-layer: the bright fill is the moving RMS body, the dark
//! zone above it extends to the peak bar, and the thin marker holds the
//! recent peak maximum. The digital scale uses green below −6 dBFS, yellow
//! from −6 to 0 dBFS, and red above digital zero.
use crate::ui::monitoring::DisplayLevel;
use crate::ui::theme;
use gpui_kit::prelude::*;
use gpui_kit::*;

mod canvas;
mod geometry;

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
        0.
    } else {
        use super::fader::FaderScale;
        1. - FaderScale::position(f64::from(value))
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

/// Node-card viewport: 100px for mono/stereo, growing to 328px at eight channels.
/// Further channels scroll horizontally without shrinking their 38px readout cells.
pub fn node_meter_width(channels: usize) -> f32 {
    geometry::node_viewport_width(channels)
}

/// A caller-height meter with an internal clip header and per-channel readouts.
/// The outer card can use [`node_meter_width`] for the same channel-aware width.
pub fn node_vertical_meter(
    levels: Option<&[DisplayLevel]>,
    channels: usize,
    clips: &[bool],
) -> Div {
    canvas::node(levels, channels, clips)
}

/// Flyout-scale stereo meter with independent clip latches above its tracks.
/// Its legacy 182px outer box contains 176px tracks; explicit height overrides
/// are honored by the canvas without changing the main Mixer's shared track.
pub fn compact_stereo_meter(levels: Option<[DisplayLevel; 2]>, clips: [bool; 2]) -> Div {
    canvas::compact_stereo(levels, clips)
}

/// A horizontal bar reuses the vertical track space: `bottom` becomes the
/// distance from the left (quiet) edge and `height` the extent from there.
fn horizontal_span(span: &MeterSpan) -> (f32, f32) {
    (span.bottom, span.height)
}

/// One horizontal row per channel, each with its own right-edge clip latch.
/// The marker and fill stay within the track rather than entering the clip cell.
pub fn compact_horizontal_meter(
    levels: Option<&[DisplayLevel]>,
    channels: usize,
    clips: &[bool],
) -> Div {
    canvas::compact_horizontal(levels, channels, clips)
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
